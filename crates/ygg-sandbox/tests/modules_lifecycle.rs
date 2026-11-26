//! r4 生命周期面：存读档 / 热载 / 确定性 / 性能。
//!
//! 这一组盯的是「装载是**一次性快照**还是会在运行中漂移」：
//!
//! - **存读档**：模块缓存属于"代码的加载产物"，不入存档——读档必须重建缓存
//!   （模块各执行一次、旧对象不得泄漏、模块内状态归零）；
//! - **热载**：模块文件变了要能换新代码，换不动时必须**旧 mod 继续跑**（不留
//!   半更新态）。判据在 `ModBundle`（内容寻址），换装层（= 本文件）走
//!   `Loaded::restore_from_bundle`；
//! - **确定性**：同一 Mod 反复装载，模块执行序与入口装配序必须逐项相等。这条
//!   防的是"依赖图遍历用了 HashMap 之类"——Rust 的 HashMap 迭代序在实例之间
//!   可以不同，一旦泄漏进装载序，Mod 的行为就成了概率函数；
//! - **性能**：128 个模块（表上限）整包装载要留在宽松阈值内，别让"能力上限"
//!   变成"实际上用不了"。

mod support;

use std::fs;
use std::time::{Duration, Instant};

use ygg_sandbox::modload::{Loaded, ModBundle};
use ygg_sandbox::modules::MAX_MODULES;

use support::{
    copy_fixture, fixture_bundle, fixture_loaded, narrate, reseal, temp_dir, write_package,
};

/// 入口装配序期望值（模块纯导出：顺序由入口的调用序决定，与 require 顺序无关）。
///
/// `src.util` / `src.shared` 在 `src.handlers` 里先被 require，`src.handlers`
/// 自己随后注册，入口最后注册——顺序由入口的调用图唯一决定。
const ORDER: [&str; 4] = ["util", "shared", "handlers", "entry"];

/// fixture 装载后的"全新状态"观测值（模块各执行一次、模块内状态零值）。
const FRESH: &str = "util_execs=1;shared_runs=1;silent_runs=1;util_count=0;shared_count=0";

/// 模块函数被调用后的观测值（`util.bump(2)` 的返回值 = 新计数）。
///
/// 这条同时是**模块状态泄漏探针**：`bump` 返回的是"累加后的计数"，所以
/// 活过上一次装载的旧对象会给出 7 而不是 2。
const BUMPED: &str = "handler_from_table=util;bumped=2";

// ───────────────────────── 存读档 ─────────────────────────

#[test]
fn restore_rebuilds_the_module_cache_without_leaking_module_state() {
    let bundle = fixture_bundle();
    let loaded = Loaded::from_bundle(&bundle).expect("装载");

    // 装载后：模块各执行一次，入口装配序固定。
    assert_eq!(narrate(&loaded, "probe.stats"), [FRESH]);
    assert_eq!(narrate(&loaded, "probe.order"), ORDER);

    // 两条独立的状态面：世界状态（世界帧）与模块内状态（`util.count`，不入帧）。
    assert_eq!(narrate(&loaded, "probe.kv"), ["stage=begin"]);
    assert_eq!(narrate(&loaded, "probe.bump"), ["util_count=5"]);
    assert_eq!(narrate(&loaded, "probe.table"), [BUMPED]);

    let frame = loaded.snapshot().expect("世界帧");
    let tick_before = loaded.tick();

    // 读档重建（宿主的热载/续局同一条路径：`Loaded::restore_from_bundle`）。
    let restored = Loaded::restore_from_bundle(&bundle, &frame).expect("读档重建");

    // 世界帧随档往返。
    assert_eq!(restored.tick(), tick_before);
    // 模块缓存重建：各执行一次（不是两次 = 重复装载，也不是零次 = 用了旧内核）。
    assert_eq!(narrate(&restored, "probe.stats"), [FRESH]);
    // 旧对象不泄漏：`util.bump(2)` 仍返回 2（旧对象会给出 7）。
    assert_eq!(
        narrate(&restored, "probe.table"),
        [BUMPED],
        "模块内状态必须归零：读档重建的是缓存，不是复用旧 Lua 状态"
    );
    // 装配不重复：仍是那 4 条，顺序不变。
    assert_eq!(narrate(&restored, "probe.order"), ORDER);
    // `return nil` 的模块同样只执行一次（缓存重建不放过它）。
    assert_eq!(
        narrate(&restored, "probe.nil"),
        ["silent=true;again=true;same=true;runs=1"]
    );
}

// ───────────────────────── 热载 ─────────────────────────

#[test]
fn changing_a_module_file_hot_swaps_the_running_mod() {
    // 只改 `src/util.lua`（入口一字未动）：r2 之前这种改动**静默不生效**
    //（签名只覆盖已声明文件，模块文件没纳管 → 热载不触发、续局跑旧代码）。
    let dir = copy_fixture("lifecycle-hot-swap");
    let before = ModBundle::load(&dir).expect("原始包");
    let old_loaded = Loaded::from_bundle(&before).expect("旧装载");
    assert_eq!(narrate(&old_loaded, "probe.table"), [BUMPED]);

    let original = fs::read_to_string(dir.join("src/util.lua")).expect("读模块源码");
    let reloaded = original
        .replace("label = \"util\"", "label = \"util-hot\"")
        .replace("（多文件 Mod）", "（热载后的模块）");
    assert_ne!(original, reloaded, "改动必须真的落到模块源码上");
    fs::write(dir.join("src/util.lua"), &reloaded).expect("写模块源码");
    reseal(&dir); // 作者改完模块要重算内容寻址（打包工具职责）

    let after = ModBundle::load(&dir).expect("新包");
    // 模块表内容确实变了——宿主 `mod_signature` 说"变了"之后，装载面落实的结果。
    assert_ne!(before.modules, after.modules, "模块字节不同则模块表不同");
    assert_eq!(before.mod_source, after.mod_source, "入口没动");

    let new_loaded = Loaded::from_bundle(&after).expect("新装载");
    assert_eq!(
        narrate(&new_loaded, "probe.table"),
        ["handler_from_table=util-hot;bumped=2"],
        "新装载必须跑新模块代码"
    );

    // 换装语义（`Session::maybe_reload` 的实现路径）：现态帧 → 新包 restore，
    // 一次原子构造完成"换代码 + 回灌状态"，不回退到旧模块。
    let frame = old_loaded.snapshot().expect("世界帧");
    let swapped = Loaded::restore_from_bundle(&after, &frame).expect("热载换装");
    assert_eq!(
        narrate(&swapped, "probe.table"),
        ["handler_from_table=util-hot;bumped=2"],
        "换装后跑的是新模块"
    );
    assert_eq!(narrate(&swapped, "probe.order"), ORDER, "换装不重复装配");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_failed_hot_reload_keeps_the_old_mod_running() {
    let dir = copy_fixture("lifecycle-hot-fail");
    let bundle = ModBundle::load(&dir).expect("原始包");
    let mut loaded = Loaded::from_bundle(&bundle).expect("装载");
    assert_eq!(narrate(&loaded, "probe.table"), [BUMPED]);

    // 改模块但**不**重算 manifest（作者忘了跑打包工具）→ 装载前即拒绝。
    fs::write(dir.join("src/util.lua"), "return { drifted = true }\n").expect("写漂移模块");
    let err = ModBundle::load(&dir)
        .expect_err("内容寻址必须拒绝漂移的模块")
        .to_string();
    assert!(
        err.contains("src/util.lua") && err.contains("content"),
        "拒绝必须点名模块与内容寻址：{err}"
    );

    // 旧 mod 继续跑：模块表是装载期快照，磁盘改动不半途污染在跑的局。
    assert_eq!(
        narrate(&loaded, "probe.table"),
        [BUMPED],
        "热载失败后旧装载必须照旧工作（不留半更新）"
    );
    assert_eq!(narrate(&loaded, "probe.order"), ORDER);
    // 真的还在跑：世界照样能前进。
    let report = loaded.step(1, 1).expect("旧装载仍可推进");
    assert_eq!(report.tick, 1);

    let _ = fs::remove_dir_all(&dir);
}

// ───────────────────────── 确定性 ─────────────────────────

#[test]
fn repeated_loads_produce_the_same_execution_and_assembly_order() {
    // 装载序与装配序：同一 Mod 连续装载时逐项相等（不是"集合相等"）。
    let reference = fixture_loaded();
    let expected_order = narrate(&reference, "probe.order");
    assert_eq!(expected_order, ORDER, "装配序必须与入口调用序一致");
    let expected_trace = narrate(&reference, "probe.trace");
    assert_eq!(expected_trace, ["trace=>handlers>util>shared>silent"]);

    // 反复装载（含换目录装载：路径不得影响顺序）。
    for round in 0..8 {
        let loaded = if round % 2 == 0 {
            fixture_loaded()
        } else {
            let copy = copy_fixture(&format!("lifecycle-determinism-{round}"));
            let loaded =
                Loaded::from_bundle(&ModBundle::load(&copy).expect("副本装载")).expect("副本装载");
            let _ = fs::remove_dir_all(&copy);
            loaded
        };
        assert_eq!(
            narrate(&loaded, "probe.order"),
            expected_order,
            "第 {round} 次装载的装配序漂移了"
        );
        assert_eq!(
            narrate(&loaded, "probe.trace"),
            expected_trace,
            "第 {round} 次装载的模块执行序漂移了"
        );
    }
}

// ───────────────────────── 性能 ─────────────────────────

#[test]
fn a_full_module_table_loads_within_the_perf_budget() {
    // 128 个模块（表上限）+ 入口逐个 require：整包装载 < 2s（宽松阈值，容忍
    // 慢机与 CI 抖动——这里防的是"上限变成实际上用不了"的量级问题，不是微基准）。
    let dir = temp_dir("lifecycle-perf");
    let table: Vec<(String, String)> = (0..MAX_MODULES)
        .map(|index| {
            (
                format!("src/m{index:03}.lua"),
                format!("local m = {{ index = {index} }}\nreturn m\n"),
            )
        })
        .collect();
    let mut entry = String::new();
    for index in 0..MAX_MODULES {
        entry.push_str(&format!("local m{index} = require(\"src.m{index:03}\")\n"));
    }
    entry.push_str(
        "ygg.kv.set(\"probe\", \"first=\" .. tostring(m0.index) .. \";last=\" .. tostring(m127.index))\n",
    );
    write_package(&dir, &entry, &table);

    let started = Instant::now();
    let bundle = ModBundle::load(&dir).expect("128 模块包必须可装载");
    let loaded = Loaded::from_bundle(&bundle).expect("128 模块装载必须可构造");
    let elapsed = started.elapsed();

    assert_eq!(bundle.modules.len(), MAX_MODULES);
    assert!(
        elapsed < Duration::from_secs(2),
        "128 模块装载耗时 {elapsed:?} 超过 2s 上限"
    );
    // 真装了（不是"校验过了但没注入"）：128 个模块全部被 require 到。
    assert_eq!(narrate(&loaded, "probe"), ["first=0;last=127"]);

    let _ = fs::remove_dir_all(&dir);
}
