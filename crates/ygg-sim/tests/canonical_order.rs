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

use ygg_sim::component::ComponentDef;
use ygg_sim::sim::Sim;

fn fixture() -> Json {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/canonical-order.json");
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
