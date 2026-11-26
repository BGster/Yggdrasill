//! archetype 迁移与 `swap_remove` 语义用例（sim-kernel §2「行序与删除语义」）。
//!
//! 三条要钉住的性质：
//! 1. `insert` 把实体迁到**更大的**位集桶，旧列值随之搬走（不是重建后丢值）。
//! 2. `swap_remove` 的填洞行 = 原末行，且 `entity_index` 被修好（否则下一次 `value_of`
//!    会读到一个错的行——这类错误只会在十万分之一的情况下显形，必须靠用例而不是靠运气）。
//! 3. **先校验后改**：任何拒绍都不留下半改状态。

use ygg_sim::archetype::{Column, ValueRef};
use ygg_sim::component::{ComponentDef, ComponentId, EntityId};
use ygg_sim::error::codes;
use ygg_sim::fixed::{self, ONE};
use ygg_sim::store::ArchetypeStore;

struct World {
    store: ArchetypeStore,
    pos: ComponentId,
    vel: ComponentId,
    hp: ComponentId,
    enemy: ComponentId,
}

fn world() -> World {
    let mut store = ArchetypeStore::new();
    let pos = store.register("Pos", ComponentDef::number()).unwrap();
    let vel = store.register("Vel", ComponentDef::number()).unwrap();
    let hp = store.register("Hp", ComponentDef::number()).unwrap();
    let enemy = store.register("Enemy", ComponentDef::tag()).unwrap();
    World {
        store,
        pos,
        vel,
        hp,
        enemy,
    }
}

fn i64_col(values: &[i64]) -> Column {
    Column::I64(values.to_vec())
}

fn as_i64(store: &ArchetypeStore, id: EntityId, comp: ComponentId) -> i64 {
    match store.value_of(id, comp).unwrap() {
        ValueRef::I64(v) => v,
        other => panic!("期望 i64 列，得到 {other:?}"),
    }
}

#[test]
fn spawn_assigns_dense_rows_and_monotonic_ids() {
    let mut w = world();
    let ids = w
        .store
        .spawn_columns(3, &[(w.pos, i64_col(&[ONE, 2 * ONE, 3 * ONE]))], &[])
        .unwrap();
    assert_eq!(ids, vec![1, 2, 3], "id 从 1 起、按分配序");
    assert_eq!(w.store.entity_count(), 3);
    assert_eq!(w.store.archetype_count(), 1);
    assert_eq!(w.store.locate(1).unwrap(), (0, 0));
    assert_eq!(w.store.locate(3).unwrap(), (0, 2));
    assert_eq!(as_i64(&w.store, 2, w.pos), 2 * ONE);
}

#[test]
fn insert_migrates_entity_to_superset_archetype_keeping_old_values() {
    let mut w = world();
    let ids = w
        .store
        .spawn_columns(3, &[(w.pos, i64_col(&[ONE, 2 * ONE, 3 * ONE]))], &[])
        .unwrap();
    let written = w
        .store
        .insert_columns(&[ids[1]], &[(w.vel, i64_col(&[10 * ONE]))], &[])
        .unwrap();
    assert_eq!(written, 1, "1 实体 × 1 组件 = 1 个值");
    assert_eq!(w.store.archetype_count(), 2, "新位集 ⇒ 新桶（创建序 = 1）");
    assert_eq!(w.store.entity_count(), 3, "迁移不改变实体数");

    let (src_idx, _) = w.store.locate(ids[0]).unwrap();
    let (dst_idx, dst_row) = w.store.locate(ids[1]).unwrap();
    assert_eq!((src_idx, dst_idx), (0, 1));
    assert_eq!(dst_row, 0, "目标桶的第一行");
    // 旧值随迁：pos 仍是 2.0，而不是被清零
    assert_eq!(as_i64(&w.store, ids[1], w.pos), 2 * ONE);
    assert_eq!(as_i64(&w.store, ids[1], w.vel), 10 * ONE);
    // 源桶被 swap_remove：它是三行，删中间那行 ⇒ 原末行（id 3）被搬到行 1
    assert_eq!(w.store.locate(ids[2]).unwrap(), (0, 1));
    assert_eq!(w.store.archetypes()[0].len(), 2);
    assert_eq!(as_i64(&w.store, ids[2], w.pos), 3 * ONE);
}

#[test]
fn insert_into_tag_component_adds_no_column() {
    let mut w = world();
    let ids = w
        .store
        .spawn_columns(1, &[(w.pos, i64_col(&[ONE]))], &[])
        .unwrap();
    let written = w.store.insert_columns(&[ids[0]], &[], &[w.enemy]).unwrap();
    assert_eq!(written, 0, "tag 没有列 ⇒ 零个值被写");
    let (arch_idx, _) = w.store.locate(ids[0]).unwrap();
    assert_eq!(w.store.archetypes()[arch_idx as usize].cols.len(), 1);
    // 取 tag 的「值」是 schema 错误（没有列可取）
    let err = w.store.value_of(ids[0], w.enemy).unwrap_err();
    assert_eq!(err.code(), codes::SCHEMA);
}

#[test]
fn despawn_swap_removes_and_repairs_entity_index() {
    let mut w = world();
    let ids = w
        .store
        .spawn_columns(
            4,
            &[(w.pos, i64_col(&[ONE, 2 * ONE, 3 * ONE, 4 * ONE]))],
            &[],
        )
        .unwrap();
    let removed = w.store.despawn_ids(&[ids[1]]).unwrap();
    assert_eq!(removed, 1);
    assert_eq!(w.store.entity_count(), 3);
    // 末行（id 4 / pos=4.0）被搬到行 1，索引必须跟着改
    assert_eq!(w.store.locate(ids[3]).unwrap(), (0, 1));
    assert_eq!(as_i64(&w.store, ids[3], w.pos), 4 * ONE);
    // 被删的 id 再查即 entity 错误（不复用）
    let err = w.store.locate(ids[1]).unwrap_err();
    assert_eq!(err.code(), codes::ENTITY);
}

#[test]
fn despawn_with_unknown_id_changes_nothing() {
    let mut w = world();
    let ids = w
        .store
        .spawn_columns(2, &[(w.pos, i64_col(&[ONE, 2 * ONE]))], &[])
        .unwrap();
    let err = w.store.despawn_ids(&[ids[0], 999]).unwrap_err();
    assert_eq!(err.code(), codes::ENTITY);
    // 先校验后改：第一个 id 也没被删
    assert_eq!(w.store.entity_count(), 2);
    assert_eq!(w.store.locate(ids[0]).unwrap(), (0, 0));
}

#[test]
fn insert_of_present_component_is_rejected_before_any_change() {
    let mut w = world();
    let ids = w
        .store
        .spawn_columns(2, &[(w.pos, i64_col(&[ONE, 2 * ONE]))], &[])
        .unwrap();
    let err = w
        .store
        .insert_columns(&[ids[0]], &[(w.pos, i64_col(&[9 * ONE]))], &[])
        .unwrap_err();
    assert_eq!(err.code(), codes::SCHEMA);
    assert_eq!(w.store.archetype_count(), 1, "拒绍不建桶");
    assert_eq!(as_i64(&w.store, ids[0], w.pos), ONE, "拒绍不改值");
}

#[test]
fn batch_shape_violations_are_typed() {
    let mut w = world();
    // 列行数与帧行数不符
    let err = w
        .store
        .spawn_columns(2, &[(w.pos, i64_col(&[ONE]))], &[])
        .unwrap_err();
    assert_eq!(err.code(), codes::SCHEMA);
    // 同一帧里重复组件
    let err = w
        .store
        .spawn_columns(
            1,
            &[(w.pos, i64_col(&[ONE])), (w.pos, i64_col(&[ONE]))],
            &[],
        )
        .unwrap_err();
    assert_eq!(err.code(), codes::SCHEMA);
    // 未知组件 id
    let err = w
        .store
        .spawn_columns(1, &[(99, i64_col(&[ONE]))], &[])
        .unwrap_err();
    assert_eq!(err.code(), codes::SCHEMA);
    // tag 带载荷
    let err = w
        .store
        .spawn_columns(1, &[(w.enemy, i64_col(&[ONE]))], &[])
        .unwrap_err();
    assert_eq!(err.code(), codes::SCHEMA);
    // 全部拒绍都不该留下状态
    assert_eq!(w.store.entity_count(), 0);
    assert_eq!(w.store.archetype_count(), 0);
    assert_eq!(w.store.next_id(), 1, "拒绍不消耗 id");
}

#[test]
fn parts_round_trip_rebuilds_index_and_rejects_duplicate_keys() {
    let mut w = world();
    let ids = w
        .store
        .spawn_columns(2, &[(w.pos, i64_col(&[ONE, 2 * ONE]))], &[])
        .unwrap();
    let (registry, archetypes, next_id) = w.store.clone().into_parts();
    let rebuilt = ArchetypeStore::from_parts(registry.clone(), archetypes, next_id).unwrap();
    assert_eq!(rebuilt.entity_count(), 2);
    assert_eq!(as_i64(&rebuilt, ids[1], w.pos), 2 * ONE);
    assert_eq!(rebuilt.next_id(), 3);

    // 两份同键桶 ⇒ 帧错误（快照里位集必须唯一）
    let duplicated = vec![
        ygg_sim::archetype::Archetype::new(
            1u128 << w.pos,
            vec![w.pos],
            vec![Column::I64(vec![ONE])],
        ),
        ygg_sim::archetype::Archetype::new(
            1u128 << w.pos,
            vec![w.pos],
            vec![Column::I64(vec![ONE])],
        ),
    ];
    let err = ArchetypeStore::from_parts(registry, duplicated, 3).unwrap_err();
    assert_eq!(err.code(), codes::FRAME);
}

#[test]
fn row_values_survive_fixed_point_math() {
    // 组件值是定点：在列里存/取不改变位模式（不做任何「友好化」转换）
    let mut w = world();
    let value = fixed::mul(3 * ONE, ONE + ONE / 2); // 4.5
    let ids = w
        .store
        .spawn_columns(1, &[(w.hp, i64_col(&[value]))], &[])
        .unwrap();
    assert_eq!(as_i64(&w.store, ids[0], w.hp), 4 * ONE + ONE / 2);
}

/// 一帧迁移**多行**（`insert_many` 的真实形态）：行号必须在每次 `swap_remove` 之后重查。
///
/// 这条用例是缺陷回归：曾经的 plan 在①校验阶段把行号固化下来，于是第一次迁移的末行填洞
/// 就把后续行号弄错位——缩小的源桶会越界 panic，或者迁走错行的值（静默错值）。
/// 发现路径是 10⁴ 行的基准（`tests/perf_query.rs` 的 `op=insert_columns`）。
#[test]
fn insert_columns_migrates_many_rows_in_one_frame() {
    let mut w = world();
    let ids = w
        .store
        .spawn_columns(
            6,
            &[(
                w.pos,
                i64_col(&[ONE, 2 * ONE, 3 * ONE, 4 * ONE, 5 * ONE, 6 * ONE]),
            )],
            &[],
        )
        .unwrap();
    let payload = i64_col(&[10, 20, 30, 40, 50, 60]);
    let written = w
        .store
        .insert_columns(&ids, &[(w.vel, payload)], &[])
        .unwrap();
    assert_eq!(written, 6);
    assert_eq!(w.store.entity_count(), 6);
    assert_eq!(
        w.store.archetype_count(),
        2,
        "源桶被清空后仍保留（桶不回收）"
    );
    for (k, id) in ids.iter().enumerate() {
        assert_eq!(
            as_i64(&w.store, *id, w.vel),
            (k as i64 + 1) * 10,
            "载荷必须落到对的实体上（k={k}）"
        );
        assert_eq!(
            as_i64(&w.store, *id, w.pos),
            (k as i64 + 1) * ONE,
            "旧列值随行搬走"
        );
    }
    let mut out = Vec::new();
    w.store.collect_ids(&[w.pos, w.vel], &mut out);
    assert_eq!(out, ids, "目标桶行序 = 帧内 id 序");
}

/// 去重快路径（排序 + 相邻扫描）必须保留**报错口径**：重复 id 与未知 id 都是 entity 错，
/// 且「输入序里先出现的那个」优先（`locate` 先于重复判定，与 O(n²) 版逐字相同）。
#[test]
fn insert_columns_rejects_duplicates_and_unknown_ids_with_unchanged_precedence() {
    let mut w = world();
    let ids = w
        .store
        .spawn_columns(3, &[(w.pos, i64_col(&[ONE, 2 * ONE, 3 * ONE]))], &[])
        .unwrap();
    let before = w.store.clone();

    let payload = i64_col(&[7, 8]);
    let err = w
        .store
        .insert_columns(&[ids[1], ids[1]], &[(w.vel, payload)], &[])
        .unwrap_err();
    assert_eq!(err.code(), codes::ENTITY);
    assert!(
        err.to_string()
            .contains(&format!("实体 id {} 出现两次", ids[1])),
        "重复报错必须仍旧指向该 id：{err}"
    );

    // 未知 id 在输入序里靠前 ⇒ 先报未知（不是先报后面的重复）。
    let payload = i64_col(&[7, 8, 9]);
    let err = w
        .store
        .insert_columns(&[999, ids[1], ids[1]], &[(w.vel, payload)], &[])
        .unwrap_err();
    assert_eq!(err.code(), codes::ENTITY);
    assert!(
        err.to_string().contains("未知实体 id：999"),
        "输入序优先：{err}"
    );

    assert_eq!(w.store, before, "拒绍不改状态（不含多余空桶）");
}
