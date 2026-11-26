//! M2 指令预算：`E_BUDGET` 必须在**墙上时钟可观测的短时间**内截断失控代码，
//! 且**不能被脚本捕获后继续跑**。
//!
//! 三条断言口径（与设计文档 §「预算」一致）：
//!
//! 1. 失控循环以 `E_BUDGET` 失败（类型化错误，不靠解析文本）；
//! 2. 捕获不了：`pcall` / `xpcall` 记得住「本次入口耗尽过」这件事，并把
//!    「受保护调用成功了」重新变成错误——所以 `pcall(function() while true do end end)`
//!    既不是 `false` 也不是能继续跑的返回值；
//! 3. 重试也不行：`while true do pcall(...) end` 同样被截断（预算是粘滞的，
//!    直到宿主 `reset_budget`）。
//!
//! 时间上限取宽松值（2 s）而不是 design 里的几十毫秒：debug 构建 + hook 每
//! 1000 条一次 Rust 回调，机器负载差异远大于那个量级；真正要钉的是「有限时间
//! 内结束」，而 release 数字在 evidence 里单独记。

use std::path::PathBuf;
use std::time::Instant;

use ygg_sandbox::modload::Loaded;
use ygg_sandbox::sandbox::{
    budget_exhausted, Sandbox, CODE_BUDGET, DEFAULT_INSTRUCTION_BUDGET, MAX_INSTRUCTION_BUDGET,
};

/// 墙钟上限（debug 构建下的宽松护栏；只证明「有限」）。
const WALL_CLOCK_LIMIT_MS: u128 = 2_000;

fn run(sandbox: &Sandbox, chunk: &str) -> mlua::Error {
    sandbox.reset_budget();
    sandbox
        .lua()
        .load(chunk)
        .exec()
        .expect_err("chunk 必须以错误结束")
}

#[test]
fn runaway_loop_is_cut_by_the_budget() {
    let sb = Sandbox::new().expect("沙箱构造");
    let started = Instant::now();
    let err = run(&sb, "while true do end");
    let elapsed = started.elapsed().as_millis();
    eprintln!("debug 构建：`while true do end` 在 {elapsed} ms 内被预算截断");
    assert!(
        elapsed < WALL_CLOCK_LIMIT_MS,
        "预算必须让失控循环在有限时间内结束，实际 {elapsed} ms"
    );
    let budget = budget_exhausted(&err).expect("错误必须是预算哨兵");
    assert_eq!(budget.code(), CODE_BUDGET);
    assert_eq!(budget.limit, DEFAULT_INSTRUCTION_BUDGET);
    assert_eq!(
        budget_exhausted(&err).map(|e| e.limit),
        Some(DEFAULT_INSTRUCTION_BUDGET)
    );
    assert!(err.to_string().contains(CODE_BUDGET), "码在首段：{err}");
    // 状态面：失败之后 `budget_exhausted()` 为真，重置之后为假。
    assert!(sb.budget_exhausted(), "耗尽后状态必须置位");
    sb.reset_budget();
    assert!(!sb.budget_exhausted(), "重置之后状态必须清回");
}

#[test]
fn pcall_cannot_swallow_the_budget() {
    let sb = Sandbox::new().expect("沙箱构造");
    // 受保护调用本身「成功了」，但它保护的是一个耗尽预算的体：shim 把它重抛。
    let err = run(
        &sb,
        "local ok = pcall(function() while true do end end) return ok",
    );
    assert!(
        budget_exhausted(&err).is_some(),
        "pcall 不得把预算中止返回值化：{err}"
    );
}

#[test]
fn xpcall_cannot_swallow_the_budget() {
    let sb = Sandbox::new().expect("沙箱构造");
    let err = run(
        &sb,
        r#"local ok = xpcall(function() while true do end end, function() return "handler" end)
           return ok"#,
    );
    assert!(
        budget_exhausted(&err).is_some(),
        "xpcall 不得把预算中止返回值化（连 handler 也不行）：{err}"
    );
}

#[test]
fn retrying_inside_pcall_still_terminates() {
    // 预算是粘滞的：第一次耗尽之后，后续每次 hook 都立即中止——所以「捕获后
    // 换个写法重试」最多再跑一个 quantum 就又被打断，外层循环不可能无限转。
    let sb = Sandbox::new().expect("沙箱构造");
    let started = Instant::now();
    let err = run(
        &sb,
        "while true do pcall(function() local i = 0 while true do i = i + 1 end end) end",
    );
    let elapsed = started.elapsed().as_millis();
    assert!(
        budget_exhausted(&err).is_some(),
        "重试必须同样被截断：{err}"
    );
    assert!(
        elapsed < WALL_CLOCK_LIMIT_MS,
        "捕获-重试不得把墙钟拖长，实际 {elapsed} ms"
    );
}

#[test]
fn budget_resets_per_tick_so_long_runs_do_not_drift() {
    // ADR-S3-1 的回归：预算是「一次宿主入口」的上限，不是沙箱生命周期的累计
    // 配额。`step` 不重置时，参考包的 `ai.move` 会把余额一点点花光，第 1586
    // tick 就把世界截断——同一世界崩在不同 tick 取决于宿主怎么切 tick。
    // 连跑两轮 3000 tick：只要有一次跨 tick 累计，第一轮就过不去。
    // 另两条断言（失控循环在毫秒级被截断、`pcall` 吞不掉）分别在
    // `runaway_loop_is_cut_by_the_budget` / `pcall_cannot_swallow_the_budget`。
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packs/example");
    let loaded = Loaded::from_dir(&dir).expect("装载 packs/example");
    let mut loaded = loaded.fork_with_seed(0x5eed).expect("按种子重建");
    for round in 1..=2u32 {
        let report = loaded.step(3_000, 1).unwrap_or_else(|err| {
            panic!("第 {round} 轮 3000 tick 必须跑得完（旧代码在第 1586 tick 崩）：{err}")
        });
        assert_eq!(report.tick, u64::from(round) * 3_000);
    }
}

#[test]
fn budget_is_configurable_and_clamped() {
    let sb = Sandbox::new().expect("沙箱构造");
    // 10k 条预算：10 万次迭代的循环必然超。
    sb.set_instruction_budget(10_000);
    assert_eq!(sb.instruction_budget(), 10_000);
    let err = run(
        &sb,
        "local i = 0 for _ = 1, 100000 do i = i + 1 end return i",
    );
    assert!(budget_exhausted(&err).is_some(), "{err}");
    // 重置 + 小脚本：同一沙箱照常工作（预算是「每次入口的燃料」，不是一次性的废态）。
    sb.reset_budget();
    let n: i64 = sb.lua().load("return 1 + 1").eval().unwrap();
    assert_eq!(n, 2);
    assert!(!sb.budget_exhausted());
    // 上限夹断：宿主也不能把预算调到实质无限。
    sb.set_instruction_budget(u64::MAX);
    assert_eq!(sb.instruction_budget(), MAX_INSTRUCTION_BUDGET);
}

#[test]
fn a_legitimate_workload_still_passes() {
    // 合法负载必须够宽：10 万次迭代（约 1e6 条指令级）在默认预算内跑通。
    let sb = Sandbox::new().expect("沙箱构造");
    sb.reset_budget();
    let sum: i64 = sb
        .lua()
        .load("local s = 0 for i = 1, 100000 do s = s + i end return s")
        .eval()
        .expect("合法负载不得被预算误伤");
    assert_eq!(sum, 5_000_050_000);
}
