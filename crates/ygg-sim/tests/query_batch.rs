//! 列式查询面与批量面的行为用例（`artifacts/w2-world-api-design.md` §1 / §3）。
//!
//! 六条要钉住的性质：
//! 1. `for_each_mut` 的写入是**真实写入**（tick hash 随之改变），且闭包拿不到结构入口。
//! 2. `get_many` / `set_many` 往返一致，且**不改行序**（`collect_ids` 前后逐字相等）。
//! 3. `col` / `col_mut` 按「桶下标升序 → 行号升序」整列给值；列型不符零产出。
//! 4. 未知组件：带 `Result` 的面报 schema 错；不带 `Result` 的面匹配 0 行。
//! 5. 空集匹配 0 行（不做「空集 = 全世界」的解释）。
//! 6. tag 没有列：`present` 为假、`fixed` 为 `None`、写入是错——但它仍参与匹配。
//!
//! 为什么用 `ArchetypeStore` 而不是 `Sim`（除 hash 用例外）：store 是查询面的唯一实现点，
//! 直接盯着它断言，失败信息指向行为而不是封装层。

use ygg_sim::archetype::Column;
use ygg_sim::component::{ComponentDef, ComponentId, EntityId};
use ygg_sim::error::codes;
use ygg_sim::query::{Bits, RowSet};
use ygg_sim::sim::Sim;
use ygg_sim::store::ArchetypeStore;

struct World {
    store: ArchetypeStore,
    pos: ComponentId,
    hp: ComponentId,
    enemy: ComponentId,
    name: ComponentId,
    bits: ComponentId,
    link: ComponentId,
}

fn world() -> World {
    let mut store = ArchetypeStore::new();
    let pos = store.register("Pos", ComponentDef::number()).unwrap();
    let hp = store.register("Hp", ComponentDef::number()).unwrap();
    let enemy = store.register("Enemy", ComponentDef::tag()).unwrap();
    let name = store.register("Name", ComponentDef::string()).unwrap();
    let bits = store.register("Bits", ComponentDef::f64_bits()).unwrap();
    let link = store.register("Link", ComponentDef::reference()).unwrap();
    World {
        store,
        pos,
        hp,
        enemy,
        name,
        bits,
        link,
    }
}

fn i64_col(values: &[i64]) -> Column {
    Column::I64(values.to_vec())
}

fn u64_col(values: &[u64]) -> Column {
    Column::U64(values.to_vec())
}

fn bytes_col(rows: &[&[u8]]) -> Column {
    let mut off = vec![0u32];
    let mut buf = Vec::new();
    for row in rows {
        buf.extend_from_slice(row);
        off.push(buf.len() as u32);
    }
    Column::Bytes { off, buf }
}

fn ids_of(store: &ArchetypeStore, set: &[ComponentId]) -> Vec<EntityId> {
    let mut out = Vec::new();
    store.collect_ids(set, &mut out);
    out
}

#[test]
fn for_each_mut_writes_through_and_changes_hash() {
    let mut sim = Sim::new(7);
    let pos = sim.register("Pos", ComponentDef::number()).unwrap();
    let hp = sim.register("Hp", ComponentDef::number()).unwrap();
    let ids = sim
        .world_mut()
        .spawn_columns(
            4,
            &[
                (pos, i64_col(&[1, 2, 3, 4])),
                (hp, i64_col(&[10, 20, 30, 40])),
            ],
            &[],
        )
        .unwrap();
    let before = sim.hash();

    let mut visited = Vec::new();
    let rows = sim
        .world_mut()
        .for_each_mut(&[pos, hp], |rs| {
            visited.push(rs.id());
            assert!(rs.present(0) && rs.present(1), "两列都该在场");
            let hp = rs.fixed(1).expect("Hp 是 number 列");
            rs.set_fixed(1, hp + 1)?;
            Ok(())
        })
        .unwrap();

    assert_eq!(rows, 4, "返回访问行数");
    assert_eq!(visited, ids, "遍历序 = 桶下标升序 → 行号升序 = 分配序");
    assert_ne!(before, sim.hash(), "写入必须落到状态里（hash 变）");

    let mut out = Column::I64(Vec::new());
    sim.world().get_many(hp, &ids, &mut out).unwrap();
    assert_eq!(out, i64_col(&[11, 21, 31, 41]), "写回的值对得上实体");
}

#[test]
fn for_each_reads_the_same_rows_as_for_each_mut() {
    let mut w = world();
    let ids = w
        .store
        .spawn_columns(
            3,
            &[
                (w.pos, i64_col(&[5, 6, 7])),
                (w.name, bytes_col(&[b"a", b"bb", b"ccc"])),
            ],
            &[w.enemy],
        )
        .unwrap();

    let mut read = Vec::new();
    let rows = w.store.for_each(&[w.pos, w.enemy], |rs: &RowSet<'_>| {
        read.push((rs.id(), rs.fixed(0), rs.present(1), rs.fixed(1)));
    });
    assert_eq!(rows, 3);
    assert_eq!(
        read,
        vec![
            (ids[0], Some(5), false, None),
            (ids[1], Some(6), false, None),
            (ids[2], Some(7), false, None),
        ],
        "tag 参与匹配但没有列：present=false、fixed=None"
    );

    // 另一个查询面：字符串列按行切片。
    let mut names = Vec::new();
    let mut walked = Vec::new();
    w.store.for_each(&[w.name], |rs: &RowSet<'_>| {
        walked.push(rs.id());
        names.push(rs.bytes(0).expect("Name 是 string 列").to_vec());
    });
    assert_eq!(walked, ids);
    assert_eq!(names, vec![b"a".to_vec(), b"bb".to_vec(), b"ccc".to_vec()]);
    assert_eq!(
        w.store.for_each(&[w.name], |rs| {
            assert_eq!(rs.fixed(0), None, "string 列没有定点整数视图");
        }),
        3
    );
}

#[test]
fn get_many_set_many_round_trip_keeps_row_order() {
    let mut w = world();
    let first = w
        .store
        .spawn_columns(2, &[(w.hp, i64_col(&[1, 2]))], &[])
        .unwrap();
    let second = w
        .store
        .spawn_columns(
            3,
            &[(w.hp, i64_col(&[3, 4, 5])), (w.pos, i64_col(&[30, 40, 50]))],
            &[],
        )
        .unwrap();
    let all = [first.clone(), second.clone()].concat();
    let order_before = ids_of(&w.store, &[w.hp]);

    let written = w
        .store
        .set_many(w.hp, &all, &i64_col(&[70, 80, 90, 100, 110]))
        .unwrap();
    assert_eq!(written, 5);
    assert_eq!(
        ids_of(&w.store, &[w.hp]),
        order_before,
        "批量写只改值，不改行序（行序是回放契约）"
    );

    let mut out = Column::I64(Vec::new());
    let read = w.store.get_many(w.hp, &all, &mut out).unwrap();
    assert_eq!(read, 5);
    assert_eq!(out, i64_col(&[70, 80, 90, 100, 110]));

    // 打乱 id 顺序：get_many 按入参序给值（不是按行序号）。
    let mut reordered = Column::I64(Vec::new());
    w.store
        .get_many(w.hp, &[all[3], all[0]], &mut reordered)
        .unwrap();
    assert_eq!(reordered, i64_col(&[100, 70]));

    // ref 列（u64）同一条往返。
    let refs = w
        .store
        .set_many(w.hp, &[], &Column::I64(Vec::new()))
        .unwrap();
    assert_eq!(refs, 0, "空 id 表是合法空写");
}

#[test]
fn set_many_rejects_before_writing_anything() {
    let mut w = world();
    let ids = w
        .store
        .spawn_columns(3, &[(w.hp, i64_col(&[1, 2, 3]))], &[])
        .unwrap();
    let h0 = w.store.clone();

    // 列型不符
    let err = w
        .store
        .set_many(w.hp, &ids, &u64_col(&[9, 9, 9]))
        .unwrap_err();
    assert_eq!(err.code(), codes::SCHEMA);
    // 行数不符
    let err = w.store.set_many(w.hp, &ids, &i64_col(&[9, 9])).unwrap_err();
    assert_eq!(err.code(), codes::SCHEMA);
    // 未知实体
    let err = w.store.set_many(w.hp, &[999], &i64_col(&[9])).unwrap_err();
    assert_eq!(err.code(), codes::ENTITY);
    // tag 没有列
    let err = w
        .store
        .set_many(w.enemy, &[ids[0]], &i64_col(&[9]))
        .unwrap_err();
    assert_eq!(err.code(), codes::SCHEMA);
    assert_eq!(w.store, h0, "拒绍不改状态（先校验后改）");

    // get_many 的拒绍同样不留半改：输出列必须空、列型必须同。
    let mut dirty = i64_col(&[123]);
    let err = w.store.get_many(w.hp, &ids, &mut dirty).unwrap_err();
    assert_eq!(err.code(), codes::SCHEMA);
    assert_eq!(dirty, i64_col(&[123]), "拒绍不动调用方的输出列");
    let mut wrong_kind = Column::U64(Vec::new());
    let err = w.store.get_many(w.hp, &ids, &mut wrong_kind).unwrap_err();
    assert_eq!(err.code(), codes::SCHEMA);
    assert!(wrong_kind.is_empty(), "拒绍时输出列仍是空列");
    let mut out = Column::I64(Vec::new());
    let err = w.store.get_many(w.hp, &[999], &mut out).unwrap_err();
    assert_eq!(err.code(), codes::ENTITY);
    assert!(out.is_empty());
}

#[test]
fn col_and_col_mut_iterate_whole_columns_in_contract_order() {
    let mut w = world();
    // 两个桶：创建序决定桶下标，行序 = dense 插入序。
    let a = w
        .store
        .spawn_columns(2, &[(w.hp, i64_col(&[1, 2]))], &[])
        .unwrap();
    let b = w
        .store
        .spawn_columns(
            2,
            &[(w.hp, i64_col(&[3, 4])), (w.pos, i64_col(&[0, 0]))],
            &[],
        )
        .unwrap();
    let order = ids_of(&w.store, &[w.hp]);
    assert_eq!(order, [a, b].concat());

    let seen: Vec<i64> = w.store.col::<i64>(w.hp).collect();
    assert_eq!(seen, vec![1, 2, 3, 4], "桶下标升序 → 行号升序");

    // 列型不符的桶零产出（签名没有 Result，不报错）。
    assert_eq!(w.store.col::<u64>(w.hp).count(), 0);
    assert_eq!(w.store.col::<i64>(w.pos).count(), 2, "只有 pos 桶产出");

    for value in w.store.col_mut::<i64>(w.hp) {
        *value *= 10;
    }
    let seen: Vec<i64> = w.store.col::<i64>(w.hp).collect();
    assert_eq!(seen, vec![10, 20, 30, 40]);
    let mut out = Column::I64(Vec::new());
    w.store.get_many(w.hp, &order, &mut out).unwrap();
    assert_eq!(out, i64_col(&[10, 20, 30, 40]), "值落在对的实体上");

    // 位串列走 `Bits` 选择器（行值是原始 u64 位串），与 `u64` 列互不串道。
    w.store
        .spawn_columns(2, &[(w.bits, Column::F64Bits(vec![0, 1]))], &[])
        .unwrap();
    assert_eq!(w.store.col::<Bits>(w.bits).count(), 2);
    assert_eq!(w.store.col::<u64>(w.bits).count(), 0);
    assert_eq!(w.store.col::<u64>(w.link).count(), 0, "ref 列还没被写入过");
}

#[test]
fn unknown_component_errors_or_matches_nothing() {
    let mut w = world();
    w.store
        .spawn_columns(2, &[(w.hp, i64_col(&[1, 2]))], &[])
        .unwrap();
    let bogus = 999u32;

    let err = w
        .store
        .for_each_mut(&[w.hp, bogus], |_rs| Ok(()))
        .unwrap_err();
    assert_eq!(err.code(), codes::SCHEMA, "带 Result 的面报 schema 错");
    let err = w
        .store
        .get_many(bogus, &[], &mut Column::I64(Vec::new()))
        .unwrap_err();
    assert_eq!(err.code(), codes::SCHEMA);
    let err = w
        .store
        .set_many(bogus, &[], &Column::I64(Vec::new()))
        .unwrap_err();
    assert_eq!(err.code(), codes::SCHEMA);

    // 不带 Result 的面：匹配 0 行，不 panic、不部分遍历。
    assert_eq!(w.store.for_each(&[w.hp, bogus], |_rs| {}), 0);
    assert!(ids_of(&w.store, &[bogus]).is_empty());
    assert_eq!(w.store.col::<i64>(bogus).count(), 0);
}

#[test]
fn empty_set_matches_zero_rows() {
    let mut w = world();
    w.store
        .spawn_columns(2, &[(w.hp, i64_col(&[1, 2]))], &[])
        .unwrap();
    assert_eq!(
        w.store
            .for_each_mut(&[], |_rs| panic!("空集不得访问任何行"))
            .unwrap(),
        0
    );
    assert_eq!(w.store.for_each(&[], |_rs| panic!("空集不得访问任何行")), 0);
    assert!(ids_of(&w.store, &[]).is_empty());
}

#[test]
fn rowset_rejects_out_of_range_and_tag_writes() {
    let mut w = world();
    w.store
        .spawn_columns(1, &[(w.hp, i64_col(&[7]))], &[w.enemy])
        .unwrap();
    let err = w
        .store
        .for_each_mut(&[w.hp, w.enemy], |rs| {
            assert_eq!(rs.len(), 2);
            assert_eq!(rs.row(), 0);
            assert_eq!(rs.u64v(0), None, "number 列没有 u64 视图");
            assert_eq!(rs.bytes(0), None, "number 列没有 bytes 视图");
            rs.set_fixed(1, 1)
        })
        .unwrap_err();
    assert_eq!(err.code(), codes::SCHEMA, "tag 没有列，不能写");

    let err = w
        .store
        .for_each_mut(&[w.hp], |rs| rs.set_fixed(4, 1))
        .unwrap_err();
    assert_eq!(err.code(), codes::SCHEMA, "列下标越界即错");

    // 闭包内的错会中止遍历并透传。
    let mut visited = 0u32;
    let err = w
        .store
        .for_each_mut(&[w.hp], |rs| {
            visited += 1;
            rs.set_fixed(0, 0)?;
            Err(ygg_sim::error::SimError::state("闭包哨兵"))
        })
        .unwrap_err();
    assert_eq!(err.code(), codes::STATE);
    assert_eq!(visited, 1, "出错即中止（不留半程）");
}

#[test]
fn u64_ref_column_reads_and_writes_through_both_faces() {
    let mut w = world();
    let ids = w
        .store
        .spawn_columns(2, &[(w.link, u64_col(&[10, 20]))], &[])
        .unwrap();
    assert_eq!(w.store.col::<u64>(w.link).collect::<Vec<_>>(), vec![10, 20]);

    let mut seen = Vec::new();
    w.store.for_each(&[w.link], |rs: &RowSet<'_>| {
        seen.push(rs.u64v(0).expect("ref 列走 u64v"));
        assert_eq!(rs.fixed(0), None, "ref 列没有定点整数视图");
    });
    assert_eq!(seen, vec![10, 20]);

    let mut out = Column::U64(Vec::new());
    w.store.get_many(w.link, &ids, &mut out).unwrap();
    assert_eq!(out, u64_col(&[10, 20]));
}

/// 同一桶的第二批 `spawn_columns`：行号必须从桶尾继续数。
///
/// 回归点：`entity_index` 是「id → (桶, 行)」的唯一真相，而行号只在对的起点上才成立。
/// 首批之后桶里已有行时，第二批若仍从 0 数，`locate` 会把每个新实体都指到首批的行——
/// 查询面（逐行读桶）看不出问题，`world.set` / `despawn`（走 `locate`）却会写到别人身上。
#[test]
fn second_spawn_batch_keeps_entity_index_rows_aligned() {
    let mut w = world();
    let first = w
        .store
        .spawn_columns(2, &[(w.hp, i64_col(&[11, 12]))], &[])
        .unwrap();
    let second = w
        .store
        .spawn_columns(2, &[(w.hp, i64_col(&[21, 22]))], &[])
        .unwrap();

    assert_eq!(w.store.locate(first[0]).unwrap(), (0, 0), "首批行号从 0 起");
    assert_eq!(
        w.store.locate(second[0]).unwrap(),
        (0, 2),
        "第二批行号接在桶尾（不是回到 0）"
    );
    assert_eq!(w.store.locate(second[1]).unwrap(), (0, 3));

    // 走 `locate` 的写面必须落在正确的行上。
    w.store
        .set_many(w.hp, &[second[0]], &i64_col(&[99]))
        .unwrap();
    let mut out = Column::I64(Vec::new());
    w.store.get_many(w.hp, &second, &mut out).unwrap();
    assert_eq!(out, i64_col(&[99, 22]));

    // 删除第二批的第一个：末行（second[1]）被搬进它的行，索引随之同步。
    w.store.despawn_ids(&[second[0]]).unwrap();
    assert!(w.store.locate(second[0]).is_err(), "被删的实体不再可定位");
    assert_eq!(w.store.locate(second[1]).unwrap(), (0, 2));
    assert_eq!(w.store.locate(first[1]).unwrap(), (0, 1));
}
