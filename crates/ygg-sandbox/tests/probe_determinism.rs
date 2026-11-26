//! S12 确定性永久探针（FR-9 native 侧验收 + FRD:66/67）——**永久**集成测试，
//! 随 `cargo test --workspace` 常驻门禁（非一次性脚本）。
//!
//! 探针清单（M1-E 口径：宿主推进取代了事件派发，探针目标不变）：
//! 1. `probe_native_step_hash_stability`——两个独立装载 + 同一 `step` 序列 →
//!    状态哈希一致；重建后重放同序列仍同哈希（「回放 = 同一序列」在新面上的形态）。
//! 2. `probe_two_lua_states_same_seed`——跨装载 RNG 序列相等（D-B5 splitmix64 +
//!    KERNEL_RNG_SEED，经完整装载链路）。
//! 3. `probe_golden_vector`——golden-vector 基线（`fixtures/golden_vector.json`，
//!    include_str! 编译期嵌入）：Mod 源码 + 期望状态哈希二元组钉死整条确定性链
//!    （装载、注入、世界写、canonical bytes、SHA-256）。
//! 4. `probe_scale_cost_baseline_documents_shard_decision`——规模成本基线
//!    （D17 串行确定性内核决策的实证锚）：10k 实体下单步推进与全量快照的墙钟
//!    成本，取多次运行最小值；断言为宽松 sanity 界，实测数据经 println! 落入
//!    测试输出作为决策记录（openwiki/overview.md D17 引用）。
//!
//! **引擎版本与 golden 哈希的关系（探针口径声明）**：canonical 快照字节是
//! **内核世界帧**，**不含 engine_version**——引擎补丁版本升级若不改状态语义，
//! golden 哈希依然成立；任何使本探针失败的改动 = 状态语义或确定性链的实际漂移，
//! 必须作为显式事件处置（更新 fixture 并在 Design History 记录原因），不得静默
//! 改基线。M1-D 删除第二套实体模型时该哈希变过一次（行存 → 列存帧）；M1-E 删除
//! 事件层又变过一次（帧里不再有事件队列 / pending choice），两次都是显式重基线。
//!
//! **wasm 侧状态（FR-9，M1 C2 起）**：本文件证明的是 native 侧确定性。
//! native/wasm 双跑哈希等价由 node 泳道承载：`crates/ygg-wasm/verify-determinism.mjs`
//! 把本探针的 golden-vector 二元组（同一份 fixture 文件）在 node + wasm 内核上
//! 双跑，以壳导出 `YggWorld::snapshot()` / `step()`（= `ygg_sandbox::hash::state_hash`
//! 同一条链）比对 `expected_state_hash`——随 `scripts/build-wasm.sh` 构建 + smoke
//! 后运行，门禁口径见 playbook `gates.no-wasm-pack`。
//! 残余缺口：`wasm-bindgen-test` 浏览器泳道（真实浏览器 wasm 实例重放同一断言）
//! 仍未实现，不得静默吸收。

use std::time::Instant;

use serde_json::Value as Json;
use ygg_sandbox::hash::state_hash;
use ygg_sandbox::modload::{Loaded, ModBundle};

/// 探针 Mod：世界写（入哈希）+ KV 写 + RNG 消费——覆盖「同序列 → 同状态」
/// 链路上的全部写入面。世界写在装载期完成（入口 chunk 豁免 MOD_EFFECT），
/// 之后由宿主 `step` 推进。
const PROBE_MOD: &str = r#"
world.component{ name = "hp", kind = "i64" }
world.component{ name = "bench", kind = "u8" }
local seed = math.random(1, 100)
ygg.kv.set("probe.seed", tostring(seed))
for i = 1, 3 do
  world.spawn{ hp = 30 - i, bench = 1 }
end
"#;

/// 固定推进序列（探针 1 共用）。
const PROBE_STEPS: &[(u32, u32)] = &[(2, 16), (3, 16), (5, 16)];

/// 装载探针 Mod（每次调用都是一份全新的 Lua 状态）。
fn probe() -> Loaded {
    Loaded::from_bundle(&ModBundle::in_memory(PROBE_MOD)).expect("probe mod loads")
}

#[test]
fn probe_native_step_hash_stability() {
    // ① 两个独立装载 + 同一 `step` 序列 → 状态哈希一致（FR9 native 侧）。
    let mut a = probe();
    let mut b = probe();
    for (ticks, dt) in PROBE_STEPS {
        a.step(*ticks, *dt).expect("step a");
        b.step(*ticks, *dt).expect("step b");
    }
    let hash_a = state_hash(&a).expect("state hash a");
    let hash_b = state_hash(&b).expect("state hash b");
    assert_eq!(
        hash_a, hash_b,
        "same mod + same step sequence => same state hash"
    );
    assert_eq!(
        a.tick().expect("tick a"),
        10,
        "tick must accumulate exactly"
    );

    // ② 「重放 = 同一序列重跑」端到端：全新装载上重放同一序列 → 同哈希。
    let mut c = probe();
    for (ticks, dt) in PROBE_STEPS {
        c.step(*ticks, *dt).expect("step c");
    }
    assert_eq!(
        hash_a,
        state_hash(&c).expect("state hash c"),
        "replaying the same sequence must reproduce the state hash"
    );
}

#[test]
fn probe_two_lua_states_same_seed() {
    // 跨装载 RNG 序列相等：Mod 在装载期消费 math.random——两个独立装载
    // （各自 Lua 状态）必须产出逐条相同的随机序列（splitmix64 +
    // KERNEL_RNG_SEED，D-B5；S8 锚经完整装载链路）。
    const RNG_MOD: &str = r#"
ygg.kv.set("rolls", tostring(math.random(1, 100))
  .. "," .. tostring(math.random(1, 100))
  .. "," .. tostring(math.random(1, 100)))
"#;
    let rolls = |label: &str| -> String {
        let loaded = Loaded::from_bundle(&ModBundle::in_memory(RNG_MOD))
            .unwrap_or_else(|e| panic!("{label}: load failed: {e}"));
        let kv = loaded.kv();
        let kv = kv.borrow();
        let Some(ygg_core::value::Value::Str(text)) = kv.get("rolls") else {
            panic!("{label}: rolls missing");
        };
        text.clone()
    };
    let a = rolls("a");
    let b = rolls("b");
    assert_eq!(a, b, "RNG sequences must match across independent loads");
    assert_eq!(a.split(',').count(), 3, "three draws recorded: {a}");
}

#[test]
fn probe_golden_vector() {
    // golden-vector 基线：fixture 由 mod_source + expected_state_hash 钉死，
    // 双跑必须逐项复现。失败即确定性链漂移，须显式处置（见模块文档口径声明）。
    let golden: Json =
        serde_json::from_str(include_str!("fixtures/golden_vector.json")).expect("fixture json");
    let source = golden["mod_source"].as_str().expect("mod_source");
    let run = |label: &str| -> String {
        let loaded = Loaded::from_bundle(&ModBundle::in_memory(source))
            .unwrap_or_else(|e| panic!("{label}: load failed: {e}"));
        state_hash(&loaded).unwrap_or_else(|e| panic!("{label}: state hash failed: {e}"))
    };

    let hash_a = run("run-a");
    let hash_b = run("run-b");
    assert_eq!(hash_a, hash_b, "double-run hash stability");

    let expected_hash = golden["expected_state_hash"].as_str().expect("hash");
    assert_eq!(
        hash_a, expected_hash,
        "state hash must reproduce the pinned golden hash (FR9 native)"
    );

    // 跨 `step` 后仍然双跑一致（推进链本身也必须确定）。
    let stepped = |label: &str| -> String {
        let mut loaded = Loaded::from_bundle(&ModBundle::in_memory(source))
            .unwrap_or_else(|e| panic!("{label}: load failed: {e}"));
        loaded
            .step(1, 16)
            .unwrap_or_else(|e| panic!("{label}: step failed: {e}"));
        state_hash(&loaded).unwrap_or_else(|e| panic!("{label}: state hash failed: {e}"))
    };
    assert_eq!(stepped("run-a"), stepped("run-b"), "stepped hash stability");
}

/// 探针 6 规模 Mod：装载期铺满 10k 个实体（两个数值组件）——「大世界」的典型
/// 形态；之后的成本都落在宿主推进与全量快照上。
const SCALE_MOD: &str = r#"
world.component{ name = "hp", kind = "i64" }
world.component{ name = "bench", kind = "u8" }
for i = 1, 10000 do
  world.spawn{ hp = 10, bench = 1 }
end
"#;

#[test]
fn probe_scale_cost_baseline_documents_shard_decision() {
    // D17（openwiki/overview.md 决策表）规模实证锚：串行确定性内核在叙事规模下的
    // 单步推进/全量快照成本量级。**口径声明**：本探针量的是成本基线（perf 记
    // 录），不是确定性；计时不确定性可接受（数据取多次运行最小值）；断言为
    // 宽松 sanity 界——只在成本悄然恶化一个数量级以上时翻红（例如 snapshot-out
    // 被改成更贵的形态、实体结构引入意外深拷贝），不承担精确性能回归职责。
    const TOUCH_RUNS: u32 = 7;
    const SAVE_RUNS: u32 = 3;

    let mut loaded =
        Loaded::from_bundle(&ModBundle::in_memory(SCALE_MOD)).expect("scale mod loads");

    // ---- 单步推进（典型「世界动一帧」成本）----
    let mut step_min = u128::MAX;
    for _ in 0..TOUCH_RUNS {
        let start = Instant::now();
        loaded.step(1, 16).expect("scale step must succeed");
        step_min = step_min.min(start.elapsed().as_micros());
    }

    // ---- 全量快照（O(N) 序列化口径）----
    let mut save_min = u128::MAX;
    for _ in 0..SAVE_RUNS {
        let start = Instant::now();
        let frame = loaded.snapshot().expect("scale snapshot must succeed");
        assert!(!frame.is_empty(), "snapshot must not be empty");
        save_min = save_min.min(start.elapsed().as_micros());
    }

    println!(
        "scale n=10000: step_min={}ms snapshot_min={}ms \
         (dev-profile; release typically 10-30x faster)",
        step_min / 1000,
        save_min / 1000
    );

    // 宽松 sanity 界（dev profile 口径，界留一个数量级余量防 CI 抖动；
    // 恶化越界 = snapshot/实体结构回归信号）。
    assert!(
        step_min < 1_000 * 1000,
        "step must stay under 1s at 10k entities (min={step_min}us)"
    );
    assert!(
        save_min < 5_000 * 1000,
        "snapshot must stay under 5s at 10k entities (min={save_min}us)"
    );
}
