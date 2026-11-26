//! 规范序的唯一 fixture：两个平面各断言一次（契约 `sim-kernel.md` §2 / `packages/codec.md` §4.1）。
//!
//! # 这条测试在钉什么
//!
//! 「复用排序规范 ≠ 复用文本容器」（sim-kernel §2 的澄清段）：Tier 1 快照是二进制
//! 列帧、`pkg.codec` 是 JSON 文本，两者的**字节永远不相等**，也不该相等。真正被复用
//! 的是**序**——键/名一律按 UTF-8 字节序升序。
//!
//! 所以判据不是「两边字节相同」，而是「两边都等于同一份 fixture 的期望序」：
//! `fixtures/canonical-order.json` 给出 `names`（洗序）与 `expected`（字节序升序），
//! 本文件断言快照帧里的组件名表 == `expected`，`crates/ygg-sandbox/tests/codec_face.rs`
//! 断言 codec 输出的键序 == `expected`。一处定义、两处消费——没有第三份说法。

use serde_json::Value as Json;

use ygg_sim::archetype::Column;
use ygg_sim::component::{ComponentDef, ComponentId, EntityId};
use ygg_sim::sim::Sim;

fn fixture() -> Json {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/canonical-order.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读 {} 失败：{error}", path.display()));
    serde_json::from_str(&raw).expect("fixture 必须是合法 JSON")
}

fn names_of(value: &Json) -> Vec<String> {
    value
        .as_array()
        .expect("fixture 字段是数组")
        .iter()
        .map(|item| item.as_str().expect("名字是字符串").to_owned())
        .collect()
}

/// 从快照帧里按**出现序**取出组件名表（帧布局见 `snapshot.rs` 头注）。
fn component_names(frame: &[u8]) -> Vec<String> {
    // magic 8 | fmt_version 2 | flags 2 | tick 8 | next_entity 8 = 28
    let mut cursor = 28usize;
    let read_u32 = |bytes: &[u8], at: usize| -> u32 {
        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    };
    let count = read_u32(frame, cursor) as usize;
    cursor += 4;
    let mut names = Vec::with_capacity(count);
    for _ in 0..count {
        let length = u16::from_le_bytes([frame[cursor], frame[cursor + 1]]) as usize;
        cursor += 2;
        let name = std::str::from_utf8(&frame[cursor..cursor + length])
            .expect("组件名是 UTF-8")
            .to_owned();
        // type 1B + frac_bits 1B
        cursor += length + 2;
        names.push(name);
    }
    names
}

#[test]
fn snapshot_component_names_follow_the_canonical_order_fixture() {
    let fixture = fixture();
    let names = names_of(&fixture["names"]);
    let expected = names_of(&fixture["expected"]);

    // 洗序注册（fixture 的顺序本身就是打乱的）；快照帧必须按字节序给出名字表
    let mut sim = Sim::new(7);
    for name in &names {
        sim.register(name, ComponentDef::number())
            .unwrap_or_else(|error| panic!("注册组件 {name} 失败：{error}"));
    }
    let frame = sim.snapshot();
    assert_eq!(
        component_names(&frame),
        expected,
        "快照的组件名表必须等于 fixture 的字节序期望序（sim-kernel §2）"
    );

    // 同态两次字节相同（规范序 ⇒ 确定性字节；frame 里的 id 不参与排序）
    assert_eq!(frame, sim.snapshot(), "同态两次快照必须逐字节相同");

    // 反向注册序必须给出**同一份字节**（名称键控 ⇒ 注册序不影响规范序）
    let mut reversed = Sim::new(7);
    for name in names.iter().rev() {
        reversed
            .register(name, ComponentDef::number())
            .unwrap_or_else(|error| panic!("注册组件 {name} 失败：{error}"));
    }
    assert_eq!(
        component_names(&reversed.snapshot()),
        expected,
        "注册序不影响快照的组件名表序"
    );
}

// ---------------------------------------------------------------------------
// T2：序不泄漏（`artifacts/w2-m1-world-plan.md` §4；设计稿 §6 风险 2）
// ---------------------------------------------------------------------------

fn world() -> (Sim, ComponentId, ComponentId) {
    let mut sim = Sim::new(11);
    let hp = sim.register("Hp", ComponentDef::number()).unwrap();
    let pos = sim.register("Pos", ComponentDef::number()).unwrap();
    (sim, hp, pos)
}

fn collected(sim: &Sim, set: &[ComponentId]) -> Vec<EntityId> {
    let mut out = Vec::new();
    sim.world().collect_ids(set, &mut out);
    out
}

fn traversal(sim: &Sim, set: &[ComponentId]) -> Vec<EntityId> {
    let mut out = Vec::new();
    sim.world().for_each(set, |rs| out.push(rs.id()));
    out
}

/// 调用切分（一次批量 spawn vs 逐行 spawn）不得漏进 hash、id 表或遍历序。
#[test]
fn spawn_batching_does_not_leak_into_hash_order_or_traversal() {
    // A：两次批量调用。
    let (mut batched, hp, pos) = world();
    let first = batched
        .world_mut()
        .spawn_columns(2, &[(hp, Column::I64(vec![100, 200]))], &[])
        .unwrap();
    let second = batched
        .world_mut()
        .spawn_columns(
            2,
            &[
                (hp, Column::I64(vec![300, 400])),
                (pos, Column::I64(vec![30, 40])),
            ],
            &[],
        )
        .unwrap();

    // B：四次逐行调用，id 序与 A 完全相同（切分不同、实体集同一个）。
    let (mut rowed, rhp, rpos) = world();
    let mut ids = Vec::new();
    for (hp_value, pos_value) in [(100, None), (200, None), (300, Some(30)), (400, Some(40))] {
        let mut cols = vec![(rhp, Column::I64(vec![hp_value]))];
        if let Some(pos_value) = pos_value {
            cols.push((rpos, Column::I64(vec![pos_value])));
        }
        let id = rowed.world_mut().spawn_columns(1, &cols, &[]).unwrap();
        ids.extend(id);
    }
    assert_eq!(ids, [first, second].concat(), "两种切分必须分配同一批 id");

    assert_eq!(batched.hash(), rowed.hash(), "切分不得进入 state hash");
    assert_eq!(
        collected(&batched, &[hp]),
        collected(&rowed, &[rhp]),
        "id 表逐字相等（桶下标与行序都没变）"
    );
    assert_eq!(
        traversal(&batched, &[hp]),
        traversal(&rowed, &[rhp]),
        "遍历序逐字相等：批量调用不得换序"
    );
}

/// 桶**创建序**不得漏进 hash：两种创建次序（insert 迁移路径不同）建出同一终态。
///
/// 遍历序则**按契约不同**（桶下标升序 → 行号升序）：这条用例同时钉住「hash 与桶下标无关」
/// 与「遍历序就是创建序」两件事，免得后来者拿前者当后者用。
#[test]
fn bucket_creation_order_does_not_leak_into_hash() {
    // A：先建 {Hp}，再迁出两行（迁移后桶内只剩 id3）。
    let (mut a, hp_a, pos_a) = world();
    let a_ids = a
        .world_mut()
        .spawn_columns(2, &[(hp_a, Column::I64(vec![100, 200]))], &[])
        .unwrap();
    a.world_mut()
        .insert_columns(&[a_ids[0]], &[(pos_a, Column::I64(vec![10]))], &[])
        .unwrap();
    let a_third = a
        .world_mut()
        .spawn_columns(1, &[(hp_a, Column::I64(vec![300]))], &[])
        .unwrap();
    a.world_mut()
        .insert_columns(&[a_ids[1]], &[(pos_a, Column::I64(vec![20]))], &[])
        .unwrap();

    // B：先建 {Hp,Pos}（id1 直接带两列），id2 后迁入，{Hp} 桶被清空后由 id3 填回。
    let (mut b, hp_b, pos_b) = world();
    let b_first = b
        .world_mut()
        .spawn_columns(
            1,
            &[
                (hp_b, Column::I64(vec![100])),
                (pos_b, Column::I64(vec![10])),
            ],
            &[],
        )
        .unwrap();
    let b_second = b
        .world_mut()
        .spawn_columns(1, &[(hp_b, Column::I64(vec![200]))], &[])
        .unwrap();
    b.world_mut()
        .insert_columns(&[b_second[0]], &[(pos_b, Column::I64(vec![20]))], &[])
        .unwrap();
    let b_third = b
        .world_mut()
        .spawn_columns(1, &[(hp_b, Column::I64(vec![300]))], &[])
        .unwrap();

    // 终态相同：两个桶、行序 {Hp}=[id3]、{Hp,Pos}=[id1,id2]，但桶下标互换（A: {Hp}=0；B: {Hp}=1）。
    assert_eq!((a_ids[0], a_ids[1], a_third[0]), (1, 2, 3));
    assert_eq!((b_first[0], b_second[0], b_third[0]), (1, 2, 3));
    assert_eq!(a.world().archetype_count(), 2);
    assert_eq!(b.world().archetype_count(), 2);
    assert_eq!(
        a.hash(),
        b.hash(),
        "桶创建序不得进入 state hash（快照按组件名规范序）"
    );

    let mut a_ids_all = collected(&a, &[hp_a]);
    let mut b_ids_all = collected(&b, &[hp_b]);
    assert_ne!(
        a_ids_all, b_ids_all,
        "遍历/收集序就是桶下标序，两条创建路径不同"
    );
    a_ids_all.sort_unstable();
    b_ids_all.sort_unstable();
    assert_eq!(a_ids_all, b_ids_all, "排序后的 id 表必须相等（同一实体集）");

    // 遍历序各自钉在自己的契约序上：A = {Hp}[3] 后 {Hp,Pos}[1,2]；B = {Hp,Pos}[1,2] 后 {Hp}[3]。
    assert_eq!(traversal(&a, &[hp_a]), vec![3, 1, 2]);
    assert_eq!(traversal(&b, &[hp_b]), vec![1, 2, 3]);
}
