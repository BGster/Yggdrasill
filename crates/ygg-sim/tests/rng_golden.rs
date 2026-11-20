//! 随机数同源用例：参考 KAT + 跨语言向量表（Rust 侧断言）。
//!
//! 两个层次刻意分开：
//! - **参考 KAT**（`11520, 0, …`）来自 xoshiro128\*\* 的公开参考实现，是「算法有没有抄错」
//!   的证据——不是派生值，两语言各存一份也不违反「唯一算法」纪律。
//! - **派生值向量**（`fixtures/fnv64-vectors.json`）由**第三实现**（Python，独立于 Rust
//!   与 Lua）生成；Lua 侧 `packages/libs/procgen/test/pkg-procgen-digest.lua` 断言同一个
//!   文件。派生值才可能分叉，所以派生值必须共享一份文件而不是各算一份。

use std::path::PathBuf;

use ygg_sim::rng::{self, RngRegistry};

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/fnv64-vectors.json")
}

fn fixture() -> serde_json::Value {
    let text = std::fs::read_to_string(fixture_path())
        .unwrap_or_else(|err| panic!("读不到向量夹具 {}: {err}", fixture_path().display()));
    serde_json::from_str(&text).expect("向量夹具不是合法 JSON")
}

/// xoshiro128** 的公开参考向量（`state = {1,2,3,4}` 的前 8 个 `next32`）。
///
/// 同一组值也出现在 `packages/libs/procgen/test/pkg-procgen.lua` 的 KAT 用例里——
/// 公开参考值允许两处各存，派生值不行。
#[test]
fn xoshiro_reference_kat_matches() {
    let mut state = [1u32, 2, 3, 4];
    let got: Vec<u32> = (0..8).map(|_| rng::next32(&mut state)).collect();
    assert_eq!(
        got,
        vec![11520, 0, 5927040, 70819200, 2031721883, 1637235492, 1287239034, 3734860849]
    );
}

#[test]
fn fnv64_vectors_match_the_shared_fixture() {
    let doc = fixture();
    let list = doc["fnv64"].as_array().expect("夹具缺 fnv64 数组");
    assert!(list.len() >= 6, "向量太少（{}）", list.len());
    for entry in list {
        let text = entry["text"].as_str().expect("向量缺 text");
        let expected = entry["fnv64"].as_str().expect("向量缺 fnv64");
        let got = format!("0x{:016x}", rng::fnv64(text));
        assert_eq!(got, expected, "fnv64({text:?}) 与共享向量不符");
    }
}

#[test]
fn derived_streams_match_the_shared_fixture() {
    let doc = fixture();
    let list = doc["streams"].as_array().expect("夹具缺 streams 数组");
    assert!(list.len() >= 4, "流向量太少（{}）", list.len());
    for entry in list {
        let seed = entry["seed"].as_str().expect("向量缺 seed（必须是文本）");
        let stream = entry["stream"].as_str().expect("向量缺 stream");
        let state: Vec<u32> = entry["state"]
            .as_array()
            .expect("向量缺 state")
            .iter()
            .map(|value| value.as_u64().expect("state 必须是整数") as u32)
            .collect();
        assert_eq!(state.len(), 4, "state 必须是四字");
        let derived = rng::derive_state(seed, stream);
        assert_eq!(
            derived,
            [state[0], state[1], state[2], state[3]],
            "derive_state({seed:?}, {stream:?}) 与共享向量不符"
        );
        let first8: Vec<u32> = entry["first8"]
            .as_array()
            .expect("向量缺 first8")
            .iter()
            .map(|value| value.as_u64().expect("first8 必须是整数") as u32)
            .collect();
        let mut words = derived;
        let got: Vec<u32> = (0..8).map(|_| rng::next32(&mut words)).collect();
        assert_eq!(
            got, first8,
            "{seed}/{stream} 的前 8 个 next32 与共享向量不符"
        );
    }
}

/// 流的独立性：新增一条流不得移动既有流的序列（procgen 契约 I2 的 Tier 1 镜像）。
#[test]
fn adding_a_stream_does_not_shift_existing_ones() {
    let mut a = RngRegistry::new(7);
    let first: Vec<u32> = (0..4).map(|_| a.next32("sim/integrate/0")).collect();
    // 插入一条新流（不同系统、不同通道）
    let _ = a.next32(rng::stream_name("decay", 1).as_str());
    let second: Vec<u32> = (0..4).map(|_| a.next32("sim/integrate/0")).collect();
    let mut b = RngRegistry::new(7);
    for _ in 0..4 {
        b.next32("sim/integrate/0");
    }
    let expected: Vec<u32> = (0..4).map(|_| b.next32("sim/integrate/0")).collect();
    assert_eq!(second, expected, "插入新流改变了既有流的序列");
    assert_ne!(first, second);
}

/// 状态往返：`state_of` → `from_parts` 之后序列接着走（存档续跑的最小证据）。
#[test]
fn registry_state_round_trips_and_continues() {
    let mut registry = RngRegistry::new(99);
    for _ in 0..5 {
        registry.next32("sim/churn/0");
    }
    let snapshot: std::collections::BTreeMap<String, [u32; 4]> = registry
        .entries()
        .map(|(name, state)| (name.to_string(), state))
        .collect();
    let mut restored = RngRegistry::from_parts(99, snapshot).unwrap();
    let mut original = registry;
    assert_eq!(
        restored.next32("sim/churn/0"),
        original.next32("sim/churn/0")
    );
    // 退化态（四字全 0）必须在重建时被拒：xoshiro 全 0 状态永远吐 0
    let mut broken = std::collections::BTreeMap::new();
    broken.insert("sim/x/0".to_string(), [0u32; 4]);
    let err = RngRegistry::from_parts(1, broken).unwrap_err();
    assert!(err.to_string().starts_with("YGG-SIM-FRAME"), "{err}");
}

/// seed 不同 ⇒ 序列不同；seed 相同 ⇒ 序列逐项相同（I5 的唯一入口）。
#[test]
fn seed_is_the_only_source_of_variation() {
    let mut a = RngRegistry::new(1);
    let mut b = RngRegistry::new(1);
    let mut c = RngRegistry::new(2);
    let stream = rng::stream_name("integrate", 0);
    let seq_a: Vec<u32> = (0..8).map(|_| a.next32(&stream)).collect();
    let seq_b: Vec<u32> = (0..8).map(|_| b.next32(&stream)).collect();
    let seq_c: Vec<u32> = (0..8).map(|_| c.next32(&stream)).collect();
    assert_eq!(seq_a, seq_b);
    assert_ne!(seq_a, seq_c);
    // u64 上界的十进制口径（`u64::to_string`）也要稳定
    assert_eq!(rng::seed_text(u64::MAX), "18446744073709551615");
}
