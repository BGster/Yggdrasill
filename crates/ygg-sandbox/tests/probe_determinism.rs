//! S12 确定性永久探针（FR-9 native 侧验收 + FRD:66/67）——**永久**集成测试，
//! 随 `cargo test --workspace` 常驻门禁（非一次性脚本）。
//!
//! 探针清单：
//! 1. `probe_native_replay_hash_stability`——双 facade 同事件序列哈希一致，
//!    外加「回放 = 外部事件日志重放」端到端（D-B1：日志仅外部事件，
//!    派生事件由重放自动再产生）。
//! 2. `probe_two_lua_states_same_seed`——跨 facade RNG 序列相等（D-B5
//!    splitmix64 + KERNEL_RNG_SEED，经完整 facade 链路含 random 消费 Mod）。
//! 3. `probe_golden_vector`——golden-vector 基线（`fixtures/golden_vector.json`，
//!    include_str! 编译期嵌入）：Mod + 事件序列 + 期望叙事 + 期望状态哈希
//!    四元组钉死整条确定性链（装载、stdlib、handler、状态、叙事、
//!    canonical bytes、SHA-256）。（原探针 3「燃料耗尽负例」随 S34 燃料
//!    退役删除——FRD:67 锚由裁定本身取代；原探针 5「hook 粒度开销」的
//!    度量对象不复存在，同批删除。）
//! 4. `probe_scale_cost_baseline_documents_shard_decision`——规模成本基线
//!    （D17 串行确定性内核决策的实证锚）：10k 实体下单事件 dispatch 与
//!    全量 save 的墙钟成本，取多次运行最小值；断言为宽松 sanity 界
//!    （防 O(N)/转换成本在快照/实体结构改动时悄然恶化一个数量级以上），
//!    实测数据经 println! 落入测试输出作为决策记录（openwiki/overview.md D17 引用）。
//!
//! **引擎版本与 golden 哈希的关系（探针口径声明）**：canonical 快照字节是
//! **内核世界帧**（`magic / fmt_version / tick / next_entity / 组件表按名升序 /
//! resources rng 流 / archetype 段 / 尾 digest`），**不含 engine_version**——
//! 引擎补丁版本升级若不改状态语义，golden 哈希依然成立；任何使本探针失败的
//! 改动 = 状态语义或确定性链的实际漂移，必须作为显式事件处置（更新 fixture
//! 并在 Design History 记录原因），不得静默改基线。M1-D 删除第二套实体模型
//! （`ygg-core` 行存）时该哈希必然变一次：帧的表示从「行存 BTreeMap 序列化」
//! 换成「列存帧」，叙事与事件序列一字未动——这正是一次显式重基线。
//!
//! **wasm 侧状态（FR-9，M1 C2 起）**：本文件证明的是 native 侧确定性。
//! native/wasm 双跑哈希等价已由 node 泳道承载：`crates/ygg-wasm/verify-determinism.mjs`
//! 把本探针的 golden-vector 四元组（同一份 fixture 文件）在 node + wasm 内核上
//! 双跑重放，以壳导出 `YggKernel::state_hash()`（= `ygg_sandbox::hash::state_hash`
//! 同一函数）比对 `expected_state_hash`——随 `scripts/build-wasm.sh` 构建 +
//! smoke 后运行，门禁口径见 playbook `gates.no-wasm-pack`。
//! 残余缺口：`wasm-bindgen-test` 浏览器泳道（真实浏览器 wasm 实例重放同一
//! 断言）仍未实现，不得静默吸收。

use std::time::Instant;

use serde_json::{json, Value as Json};
use ygg_sandbox::driver::KernelFacade;
use ygg_sandbox::hash::state_hash;

/// 探针 Mod：KV 写 + 世界写（入哈希）+ 叙事（入回放）+ 派生事件（FIFO 级联）
/// ——覆盖「同事件序列 → 同状态」链路上的全部写入面。
///
/// M1-D 备注：旧 mod 里的 `ygg.entity.add_tag(id, "probed")` 无对应表达——
/// `world.component` 只接受 i64|u64|u8|bytes（零宽 tag 无声明入口），所以本探针
/// 不再覆盖 tag 写面；tag 面本身是 M1-C 就存在的缺口，在 D1 报告中显式列出。
const PROBE_MOD: &str = r#"
world.component{ name = "hp", kind = "i64" }
ygg.on("event.probe", function(p)
  local hits = (ygg.kv.get("probe.hits") or 0) + 1
  ygg.kv.set("probe.hits", hits)
  world.spawn{ hp = 30 - hits }
  ygg.narrate("hits=" .. tostring(hits))
  ygg.narrate("payload=" .. tostring(p))
  if p < 3 then
    ygg.emit("event.probe.echo", p + 1)
  end
end)
ygg.on("event.probe.echo", function(p)
  ygg.narrate("echo=" .. tostring(p))
end)
"#;

/// 固定外部事件序列（探针 1/2 共用；负载各异以钉住 payload 路径）。
const PROBE_SEQUENCE: &[(&str, &str)] = &[
    ("event.probe", "1"),
    ("event.probe", "5"),
    ("event.probe", "9"),
];

fn parse(reply: &str) -> Json {
    serde_json::from_str(reply).expect("reply must be valid JSON")
}

fn narrative_of(reply: &str) -> Vec<String> {
    parse(reply)["narrative"]
        .as_array()
        .expect("narrative array")
        .iter()
        .map(|v| v.as_str().expect("string").to_owned())
        .collect()
}

/// 在 facade 上驱动固定序列（探针复用）。
fn drive(kernel: &mut KernelFacade, sequence: &[(&str, &str)]) {
    for (cmd, payload) in sequence {
        let reply = kernel.dispatch(cmd, payload);
        assert_eq!(
            parse(&reply)["ok"],
            json!(true),
            "probe sequence step must succeed: {reply}"
        );
    }
}

#[test]
fn probe_native_replay_hash_stability() {
    // ① 双独立 facade + 同事件序列 → 状态哈希一致（FR9 native 侧）。
    let mut a = KernelFacade::new(PROBE_MOD).expect("facade a");
    let mut b = KernelFacade::new(PROBE_MOD).expect("facade b");
    drive(&mut a, PROBE_SEQUENCE);
    drive(&mut b, PROBE_SEQUENCE);
    let hash_a = state_hash(&a).expect("state hash a");
    let hash_b = state_hash(&b).expect("state hash b");
    assert_eq!(
        hash_a, hash_b,
        "same mod + same sequence => same state hash"
    );

    // ② 「回放 = 外部事件日志重放」端到端：取 A 的外部事件日志（仅外部
    //    事件，D-B1），在全新 facade C 上按日志重放同一序列 → 同哈希。
    let log_reply = a.dispatch("log", "");
    let log = parse(&log_reply)["log"]
        .as_array()
        .expect("external log array")
        .clone();
    assert_eq!(log.len(), PROBE_SEQUENCE.len(), "external-only log");
    let mut c = KernelFacade::new(PROBE_MOD).expect("facade c");
    for entry in &log {
        let name = entry["name"].as_str().expect("event name");
        let payload = entry["payload"].to_string();
        let reply = c.dispatch(name, &payload);
        assert_eq!(parse(&reply)["ok"], json!(true), "replay step: {reply}");
    }
    let hash_c = state_hash(&c).expect("state hash c");
    assert_eq!(hash_a, hash_c, "log replay must reproduce the state hash");
}

#[test]
fn probe_two_lua_states_same_seed() {
    // 跨 facade RNG 序列相等：Mod 在 handler 内消费 math.random——两个
    // 独立 facade（各自 Lua 状态）对同事件序列必须产出逐条相同的随机
    // 序列（splitmix64 + KERNEL_RNG_SEED，D-B5；S8 锚经完整 facade 链路）。
    const RNG_MOD: &str = r#"
ygg.on("event.roll", function(p)
  ygg.narrate("a=" .. tostring(math.random(1, 100)))
  ygg.narrate("b=" .. tostring(math.random(1, 100)))
  ygg.narrate("p=" .. tostring(p))
end)
"#;
    let mut a = KernelFacade::new(RNG_MOD).expect("facade a");
    let mut b = KernelFacade::new(RNG_MOD).expect("facade b");
    for (cmd, payload) in [
        ("event.roll", "1"),
        ("event.roll", "2"),
        ("event.roll", "3"),
    ] {
        assert_eq!(
            narrative_of(&a.dispatch(cmd, payload)),
            narrative_of(&b.dispatch(cmd, payload)),
            "RNG sequences must match across independent facades ({cmd})"
        );
    }
}

#[test]
fn probe_golden_vector() {
    // golden-vector 基线：fixture 四元组（mod_source / event_sequence /
    // expected_narrative / expected_state_hash）编译期嵌入，回放必须逐项
    // 复现。基线值由 rig 一次性运行生成后钉入（重跑必须原样通过——
    // 失败即确定性链漂移，须显式处置，见模块文档口径声明）。
    let golden: Json =
        serde_json::from_str(include_str!("fixtures/golden_vector.json")).expect("fixture json");
    let run = |label: &str| -> (Vec<Vec<String>>, String) {
        let mut kernel = KernelFacade::new(golden["mod_source"].as_str().expect("mod_source"))
            .unwrap_or_else(|e| panic!("{label}: facade construction failed: {e}"));
        let mut narratives = Vec::new();
        for step in golden["event_sequence"].as_array().expect("sequence") {
            let cmd = step["cmd"].as_str().expect("cmd");
            let payload = step["payload"].as_str().expect("payload");
            let reply = kernel.dispatch(cmd, payload);
            let parsed = parse(&reply);
            assert_eq!(
                parsed["ok"],
                json!(true),
                "{label}: golden step {cmd} must succeed: {reply}"
            );
            narratives.push(narrative_of(&reply));
        }
        (narratives, state_hash(&kernel).expect("golden state hash"))
    };

    let (narratives_a, hash_a) = run("run-a");
    let (narratives_b, hash_b) = run("run-b");
    assert_eq!(narratives_a, narratives_b, "double-run narrative stability");
    assert_eq!(hash_a, hash_b, "double-run hash stability");

    let expected_narrative: Vec<Vec<String>> = golden["expected_narrative"]
        .as_array()
        .expect("expected_narrative")
        .iter()
        .map(|per_event| {
            per_event
                .as_array()
                .expect("per-event array")
                .iter()
                .map(|v| v.as_str().expect("string").to_owned())
                .collect()
        })
        .collect();
    assert_eq!(
        narratives_a, expected_narrative,
        "narrative must reproduce the pinned golden narrative"
    );
    let expected_hash = golden["expected_state_hash"].as_str().expect("hash");
    assert_eq!(
        hash_a, expected_hash,
        "state hash must reproduce the pinned golden hash (FR9 native)"
    );
}

/// 探针 6 规模 Mod：`event.add` 每次装载 100 个实体（两个数值组件）；
/// `event.touch` 做 50 次组件读 + 1 次 KV 写——「大世界、小事件」的典型
/// 叙事负载形态（单事件只触碰实体总体的常数个）。
const SCALE_MOD: &str = r#"
world.component{ name = "hp", kind = "i64" }
world.component{ name = "bench", kind = "u8" }
ygg.on("event.add", function()
  for i = 1, 100 do
    world.spawn{ hp = 10, bench = 1 }
  end
end)
ygg.on("event.touch", function()
  local n = 0
  for id = 1, 50 do
    if world.get(id, "bench") == 1 then n = n + 1 end
  end
  ygg.kv.set("touched", n)
end)
"#;

#[test]
fn probe_scale_cost_baseline_documents_shard_decision() {
    // D17（openwiki/overview.md 决策表）规模实证锚：串行确定性内核在叙事规模下的
    // 单事件/存档成本量级。**口径声明**：本探针量的是成本基线（perf 记
    // 录），不是确定性；计时不确定性可接受（数据取多次运行最小值）；断
    // 言为宽松 sanity 界——只在成本悄然恶化一个数量级以上时翻红（例如
    // snapshot-out 被改成更贵的形态、实体结构引入意外深拷贝），不承担
    // 精确性能回归职责；实测数据以 println! 落入测试输出为准（openwiki/overview.md
    // D17 / Design History 引用本输出）。
    const TARGET_ENTITIES: u32 = 10_000;
    const ADD_ROUNDS: u32 = TARGET_ENTITIES / 100;
    const TOUCH_RUNS: u32 = 7;
    const SAVE_RUNS: u32 = 3;

    let mut kernel = KernelFacade::new(SCALE_MOD).expect("facade builds");
    for _ in 0..ADD_ROUNDS {
        let reply = kernel.dispatch("event.add", "0");
        assert!(
            reply.contains("\"ok\":true"),
            "scale mod load round must succeed: {reply}"
        );
    }

    // ---- 单事件 dispatch（典型读多写少叙事事件）----
    let mut touch_min = u128::MAX;
    for _ in 0..TOUCH_RUNS {
        let start = Instant::now();
        let reply = kernel.dispatch("event.touch", "0");
        let micros = start.elapsed().as_micros();
        assert!(
            reply.contains("\"ok\":true"),
            "touch dispatch must succeed: {reply}"
        );
        touch_min = touch_min.min(micros);
    }

    // ---- 全量 save（O(N) 序列化口径）----
    let mut save_min = u128::MAX;
    for _ in 0..SAVE_RUNS {
        let start = Instant::now();
        let reply = kernel.dispatch("save", "");
        let micros = start.elapsed().as_micros();
        assert!(
            reply.contains("\"ok\":true"),
            "save dispatch must succeed: {reply}"
        );
        save_min = save_min.min(micros);
    }

    println!(
        "scale n={TARGET_ENTITIES}: touch_dispatch_min={}ms save_min={}ms \
         (dev-profile; release typically 10-30x faster)",
        touch_min / 1000,
        save_min / 1000
    );

    // 宽松 sanity 界（dev profile 口径，实测 ~6ms / ~120ms 量级，界留
    // 一个数量级余量防 CI 抖动；恶化越界 = snapshot/实体结构回归信号）。
    assert!(
        touch_min < 1_000 * 1000,
        "touch dispatch must stay under 1s at 10k entities (min={touch_min}us)"
    );
    assert!(
        save_min < 5_000 * 1000,
        "save must stay under 5s at 10k entities (min={save_min}us)"
    );
}
