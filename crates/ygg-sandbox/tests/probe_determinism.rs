//! S12 确定性永久探针（FR-9 native 侧验收 + FRD:66/67）——**永久**集成测试，
//! 随 `cargo test --workspace` 常驻门禁（非一次性脚本）。
//!
//! 探针清单：
//! 1. `probe_native_replay_hash_stability`——双 facade 同事件序列哈希一致，
//!    外加「回放 = 外部事件日志重放」端到端（D-B1：日志仅外部事件，
//!    派生事件由重放自动再产生）。
//! 2. `probe_two_lua_states_same_seed`——跨 facade RNG 序列相等（D-B5
//!    splitmix64 + KERNEL_RNG_SEED，经完整 facade 链路含 random 消费 Mod）。
//! 3. `probe_fuel_exhaustion_negative`——FRD:67 验收锚：无限循环 Mod
//!    到 FUEL_EXHAUSTED 信封（燃料杀循环的永久负例探针）。
//! 4. `probe_golden_vector`——golden-vector 基线（`fixtures/golden_vector.json`，
//!    include_str! 编译期嵌入）：Mod + 事件序列 + 期望叙事 + 期望状态哈希
//!    四元组钉死整条确定性链（装载、stdlib、handler、状态、叙事、
//!    canonical bytes、SHA-256）。
//! 5. `probe_fuel_granularity_overhead_documents_default`——燃料 hook 粒度
//!    开销实测（D-继承-2 n 值义务清偿，S12 检查点代决交付）。**本探针量的是
//!    开销比，不是确定性**：对固定 Lua 工作负载分别测（a）裸 Sandbox
//!    （无 hook）与（b）fuel::install（粒度 = FUEL_HOOK_GRANULARITY = 1000）
//!    及（c）探针自装 hook 粒度 100 的墙钟时间，取多次运行最小值算比值；
//!    断言为宽松 sanity 界（hooked < 20× bare）以避免 CI 计时抖动翻绿翻红；
//!    实测比值经 println! 落入测试输出作为数据记录（工件 Performance
//!    Considerations / Design History 引用本输出）。
//! 6. `probe_scale_cost_baseline_documents_shard_decision`——规模成本基线
//!    （D17 串行确定性内核决策的实证锚）：10k 实体下单事件 dispatch 与
//!    全量 save 的墙钟成本，取多次运行最小值；断言为宽松 sanity 界
//!    （防 O(N)/转换成本在快照/实体结构改动时悄然恶化一个数量级以上），
//!    实测数据经 println! 落入测试输出作为决策记录（vision.md D17 引用）。
//!
//! **引擎版本与 golden 哈希的关系（探针口径声明）**：canonical 快照字节含
//! `schema_version / revision / next_entity_id / entities`，**不含
//! engine_version**——引擎补丁版本升级若不改实体状态语义，golden 哈希
//! 依然成立；任何使本探针失败的改动 = 状态语义或确定性链的实际漂移，
//! 必须作为显式事件处置（更新 fixture 并在 Design History 记录原因），
//! 不得静默改基线。
//!
//! **wasm 侧状态（FR-9，M1 C2 起）**：本文件证明的是 native 侧确定性。
//! native/wasm 双跑哈希等价已由 node 泳道承载：`crates/ygg-wasm/verify-determinism.mjs`
//! 把本探针的 golden-vector 四元组（同一份 fixture 文件）在 node + wasm 内核上
//! 双跑重放，以壳导出 `YggKernel::state_hash()`（= `ygg_sandbox::hash::state_hash`
//! 同一函数）比对 `expected_state_hash`（实证：三哈希一致，a994030a…c850）——
//! 随 `scripts/build-wasm.sh` 构建 + smoke 后运行，门禁口径见 AGENTS.md §五。
//! 残余缺口：`wasm-bindgen-test` 浏览器泳道（真实浏览器 wasm 实例重放同一
//! 断言）仍未实现，不得静默吸收。

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use mlua::{HookTriggers, VmState};
use serde_json::{json, Value as Json};
use ygg_sandbox::driver::{code, KernelFacade};
use ygg_sandbox::fuel;
use ygg_sandbox::hash::state_hash;
use ygg_sandbox::sandbox::Sandbox;

/// 探针 Mod：KV 写 + 实体写（入哈希）+ 叙事（入回放）+ 派生事件（FIFO 级联）
/// ——覆盖「同事件序列 → 同状态」链路上的全部写入面。
const PROBE_MOD: &str = r#"
ygg.on("event.probe", function(p)
  local hits = (ygg.kv.get("probe.hits") or 0) + 1
  ygg.kv.set("probe.hits", hits)
  local id = ygg.entity.spawn()
  ygg.entity.set_attribute(id, "hp", 30 - hits)
  ygg.entity.add_tag(id, "probed")
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
fn probe_fuel_exhaustion_negative() {
    // FRD:67 验收锚（永久探针化）：无限循环 Mod → VM 指令计量耗尽 →
    // FUEL_EXHAUSTED 信封（域错误不 throw；D-继承-2 set_hook 燃料）。
    const LOOP_MOD: &str = r#"
ygg.on("event.forever", function()
  while true do end
end)
"#;
    let mut kernel = KernelFacade::new(LOOP_MOD).expect("facade");
    let reply = kernel.dispatch("event.forever", "0");
    let parsed = parse(&reply);
    assert_eq!(parsed["ok"], json!(false), "reply: {reply}");
    assert_eq!(
        parsed["error"]["code"],
        json!(code::FUEL_EXHAUSTED),
        "infinite loop must die by fuel (FRD:67): {reply}"
    );
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

#[test]
fn probe_fuel_granularity_overhead_documents_default() {
    // D-继承-2 n 值义务清偿（S12 检查点代决交付）：对固定 Lua 工作负载实测
    // 指令计数 hook 的开销比。**口径声明**：本探针量的是开销比（perf 计量），
    // 不是确定性——计时不确定性对本探针是可接受的（数据取多次运行最小值，
    // 比值只用于文档化默认值依据）；断言为宽松 sanity 界（hooked < 20×
    // bare）防 CI 计时抖动，实测数据以 println! 落入测试输出为准。
    const WORKLOAD: &str = "local s = 0 for i = 1, 300000 do s = s + i end return s";
    const EXPECTED: i64 = 45_000_150_000; // Σ 1..=300000
    const RUNS: u32 = 5;
    // 探针预算取天文量级（1<<62），hook 永不耗尽——纯计量、不触发终止。
    const PROBE_BUDGET: u64 = 1u64 << 62;

    /// 逐次计时取最小值（最小值对系统抖动最鲁棒，适合比值口径）。
    fn min_run_micros(lua: &mlua::Lua) -> u128 {
        // 预热一次（首次执行含 chunk 编译/加载摊销）。
        let warm: i64 = lua.load(WORKLOAD).eval().expect("workload eval");
        assert_eq!(warm, EXPECTED, "workload result must be the fixed sum");
        let mut min = u128::MAX;
        for _ in 0..RUNS {
            let start = Instant::now();
            let v: i64 = lua.load(WORKLOAD).eval().expect("workload eval");
            let micros = start.elapsed().as_micros();
            assert_eq!(v, EXPECTED, "workload result must be stable across runs");
            min = min.min(micros);
        }
        min
    }

    // (a) 裸 Sandbox：Sandbox::new 单独给出无 hook Lua（S8 已移除临时 hook，
    //     无燃料面——基准线）。
    let bare = Sandbox::new().expect("bare sandbox");
    let bare_us = min_run_micros(bare.lua());

    // (b) 出厂路径：fuel::install（粒度 = FUEL_HOOK_GRANULARITY = 1000）。
    let g1000 = Sandbox::new().expect("g1000 sandbox");
    let _meter = fuel::install(g1000.lua(), PROBE_BUDGET).expect("fuel install");
    let g1000_us = min_run_micros(g1000.lua());

    // (c) 对照数据点：粒度 100（探针自装 hook，闭包语义同 fuel::install，
    //     仅粒度不同）——展示「低粒度开销更高」的实测形态。
    let g100 = Sandbox::new().expect("g100 sandbox");
    let budget = Rc::new(Cell::new(PROBE_BUDGET));
    let counter = Rc::clone(&budget);
    g100.lua()
        .set_hook(
            HookTriggers::new().every_nth_instruction(100),
            move |_, _| {
                let left = counter.get().saturating_sub(100);
                counter.set(left);
                if left == 0 {
                    Err(mlua::Error::runtime("probe budget exhausted"))
                } else {
                    Ok(VmState::Continue)
                }
            },
        )
        .expect("probe g100 hook install");
    let g100_us = min_run_micros(g100.lua());

    let ratio = |hooked: u128| hooked as f64 / bare_us.max(1) as f64;
    let r1000 = ratio(g1000_us);
    let r100 = ratio(g100_us);
    println!(
        "[fuel-granularity] overhead measurement (min of {RUNS} runs, 300k-iteration workload):"
    );
    println!("[fuel-granularity]   bare           = {bare_us} us");
    println!("[fuel-granularity]   granularity 1000 = {g1000_us} us  (ratio x{r1000:.2})");
    println!("[fuel-granularity]   granularity  100 = {g100_us} us  (ratio x{r100:.2})");

    // 宽松 sanity 界（20×）：只排除「hook 开销灾难性」的回归，不做精确
    // 计时断言（CI 计时抖动不构成失败面）。数据记录以 println! 输出为准。
    assert!(
        g100_us < bare_us.saturating_mul(20),
        "hooked workload must stay under 20x bare (bare={bare_us}us, g100={g100_us}us)"
    );
}

/// 探针 6 规模 Mod：`event.add` 每次装载 100 个实体（属性 + 标签）；
/// `event.touch` 做 50 次 `has_tag` 读 + 1 次 KV 写——「大世界、小事件」
/// 的典型叙事负载形态（单事件只触碰实体总体的常数个）。
const SCALE_MOD: &str = r#"
ygg.on("event.add", function()
  for i = 1, 100 do
    local e = ygg.entity.spawn()
    ygg.entity.set_attribute(e, "hp", 10)
    ygg.entity.set_attribute(e, "mood", "idle")
    ygg.entity.add_tag(e, "bench")
  end
end)
ygg.on("event.touch", function()
  local n = 0
  for id = 1, 50 do
    if ygg.entity.has_tag(id, "bench") then n = n + 1 end
  end
  ygg.kv.set("touched", n)
end)
"#;

#[test]
fn probe_scale_cost_baseline_documents_shard_decision() {
    // D17（vision.md 决策表）规模实证锚：串行确定性内核在叙事规模下的
    // 单事件/存档成本量级。**口径声明**：本探针量的是成本基线（perf 记
    // 录），不是确定性；计时不确定性可接受（数据取多次运行最小值）；断
    // 言为宽松 sanity 界——只在成本悄然恶化一个数量级以上时翻红（例如
    // snapshot-out 被改成更贵的形态、实体结构引入意外深拷贝），不承担
    // 精确性能回归职责；实测数据以 println! 落入测试输出为准（vision.md
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
