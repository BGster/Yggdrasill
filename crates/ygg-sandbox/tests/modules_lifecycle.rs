//! r4 生命周期面：存读档 / 热载 / 确定性 / 性能。
//!
//! 这一组盯的是「装载是**一次性快照**还是会在运行中漂移」：
//!
//! - **存读档**：模块缓存属于"代码的加载产物"，不入存档——读档必须重建缓存
//!   （模块各执行一次）、旧对象不得泄漏（模块内状态归零）、handler 不得重复注册；
//! - **热载**：模块文件变了要能换新代码，换不动时必须**旧 mod 继续跑**（不留
//!   半更新态）。宿主的 `mod_signature` 判据与服务端的 `restore` 换装语义在各
//!   自的层测：签名层在 `examples/egg.rs` 的测试里，换装层（= 本文件）走
//!   `ModBundle::restore` —— 这正是 `Session::maybe_reload` 的实现路径；
//! - **确定性**：同一 Mod 反复装载，handler 注册序（= 派发序）必须逐项相等。
//!   这条防的是"依赖图遍历用了 HashMap 之类"——Rust 的 HashMap 迭代序在实例
//!   之间可以不同，一旦泄漏进注册序，Mod 的行为就成了概率函数；
//! - **性能**：128 个模块（表上限）整包装载要留在宽松阈值内，别让"能力上限"
//!   变成"实际上用不了"。

mod support;

use std::fs;
use std::time::{Duration, Instant};

use serde_json::{json, Value as Json};
use ygg_sandbox::modload::ModBundle;
use ygg_sandbox::modules::MAX_MODULES;

use support::{
    copy_fixture, fixture_bundle, fixture_kernel, narrate, reseal, temp_dir, write_package,
};

/// fixture 的 handler 注册序期望值（= require 图执行序 + 入口自身）。
///
/// `src.util` / `src.shared` 在 `src.handlers` 里先被 require，`src.handlers`
/// 自己随后注册，入口最后注册——顺序由依赖图唯一决定。
const ORDER: [&str; 4] = ["util", "shared", "handlers", "entry"];

/// fixture 装载后的"全新状态"观测值（模块各执行一次、模块内状态零值）。
const FRESH: &str = "util_execs=1;shared_runs=1;silent_runs=1;util_count=0;shared_count=0";

// ───────────────────────── 存读档 ─────────────────────────

#[test]
fn restore_rebuilds_the_module_cache_without_leaking_module_state() {
    let bundle = fixture_bundle();
    let mut kernel = bundle.kernel().expect("内核构造");

    // 装载后：模块各执行一次，handler 注册序固定。
    assert_eq!(narrate(&mut kernel, "probe.stats"), [FRESH]);
    assert_eq!(narrate(&mut kernel, "probe.order"), ORDER);

    // 两条独立的状态：世界状态（KV，入档）与模块内状态（`util.count`，不入档）。
    narrate(&mut kernel, "event.start"); // node:begin 写 kv.stage
    assert_eq!(narrate(&mut kernel, "probe.kv"), ["stage=begin"]);
    assert_eq!(narrate(&mut kernel, "probe.bump"), ["util_count=3"]);

    let save: Json = serde_json::from_str(&kernel.dispatch("save", "")).expect("save 回执");
    assert_eq!(save["ok"], json!(true), "save 失败：{save}");
    let save_text = save["save"].to_string();

    // 读档重建（宿主的热载/续局同一条路径：`ModBundle::restore`）。
    let mut restored = bundle.restore(save_text.as_bytes()).expect("读档重建");

    // 世界状态随档往返。
    assert_eq!(narrate(&mut restored, "probe.kv"), ["stage=begin"]);
    // 模块缓存重建：各执行一次（不是两次 = 重复装载，也不是零次 = 用了旧内核）。
    assert_eq!(narrate(&mut restored, "probe.stats"), [FRESH]);
    // 旧对象不泄漏：装载期写动的 `util.count` 归零（模块内状态不随档）。
    // handler 不重复注册：注册序仍是那 4 条，顺序不变。
    assert_eq!(narrate(&mut restored, "probe.order"), ORDER);
    // `return nil` 的模块同样只执行一次（缓存重建不放过它）。
    assert_eq!(
        narrate(&mut restored, "probe.nil"),
        ["silent=true;again=true;same=true;runs=1"]
    );

    // 读档后的世界继续跑：同一个模块对象服务后续 handler（不是每步重新 require）。
    assert_eq!(narrate(&mut restored, "probe.bump"), ["util_count=3"]);
}

// ───────────────────────── 热载 ─────────────────────────

#[test]
fn changing_a_module_file_hot_swaps_the_running_mod() {
    // 只改 `src/util.lua`（入口一字未动）：r2 之前这种改动**静默不生效**
    //（签名只覆盖已声明文件，模块文件没纳管 → 热载不触发、续局跑旧代码）。
    let dir = copy_fixture("lifecycle-hot-swap");
    let before = ModBundle::load(&dir).expect("原始包");
    let mut old_kernel = before.kernel().expect("旧内核");
    assert_eq!(
        narrate(&mut old_kernel, "probe.table"),
        ["handler_from_table=util;bumped=2"]
    );

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

    let mut new_kernel = after.kernel().expect("新内核");
    assert_eq!(
        narrate(&mut new_kernel, "probe.table"),
        ["handler_from_table=util-hot;bumped=2"],
        "新内核必须跑新模块代码"
    );

    // 换装语义（`Session::maybe_reload` 的实现路径）：现态存档 → 新包 restore，
    // 一次原子构造完成"换代码 + 回灌状态"，不回退到旧模块。
    let save: Json = serde_json::from_str(&old_kernel.dispatch("save", "")).expect("save 回执");
    let mut swapped = after
        .restore(save["save"].to_string().as_bytes())
        .expect("热载换装");
    assert_eq!(
        narrate(&mut swapped, "probe.table"),
        ["handler_from_table=util-hot;bumped=2"],
        "换装后跑的是新模块"
    );
    assert_eq!(
        narrate(&mut swapped, "probe.order"),
        ORDER,
        "换装不重复注册"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_failed_hot_reload_keeps_the_old_mod_running() {
    let dir = copy_fixture("lifecycle-hot-fail");
    let bundle = ModBundle::load(&dir).expect("原始包");
    let mut kernel = bundle.kernel().expect("内核");
    assert_eq!(
        narrate(&mut kernel, "probe.table"),
        ["handler_from_table=util;bumped=2"]
    );

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
        narrate(&mut kernel, "probe.table"),
        ["handler_from_table=util;bumped=4"],
        "热载失败后旧内核必须照旧工作（不留半更新）"
    );
    assert_eq!(narrate(&mut kernel, "probe.order"), ORDER);

    let _ = fs::remove_dir_all(&dir);
}

// ───────────────────────── 确定性 ─────────────────────────

#[test]
fn repeated_loads_register_handlers_in_the_same_order() {
    // 注册序 = 派发序：同一 Mod 连续装载时逐项相等（不是"集合相等"）。
    let mut reference = fixture_kernel();
    let expected_order = narrate(&mut reference, "probe.order");
    assert_eq!(expected_order, ORDER, "注册序必须与依赖图一致");
    let expected_trace = narrate(&mut reference, "probe.trace");
    assert_eq!(expected_trace, ["trace=>handlers>util>shared>silent"]);

    // 反复装载（含换目录装载：路径不得影响顺序）。
    for round in 0..8 {
        let mut kernel = if round % 2 == 0 {
            fixture_kernel()
        } else {
            let copy = copy_fixture(&format!("lifecycle-determinism-{round}"));
            let kernel = ModBundle::load(&copy)
                .expect("副本装载")
                .kernel()
                .expect("内核");
            let _ = fs::remove_dir_all(&copy);
            kernel
        };
        assert_eq!(
            narrate(&mut kernel, "probe.order"),
            expected_order,
            "第 {round} 次装载的 handler 注册序漂移了"
        );
        assert_eq!(
            narrate(&mut kernel, "probe.trace"),
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
        "ygg.on(\"probe\", function()\n  ygg.narrate(\"first=\" .. tostring(m0.index) .. \";last=\" .. tostring(m127.index))\nend)\n",
    );
    write_package(&dir, &entry, &table);

    let started = Instant::now();
    let bundle = ModBundle::load(&dir).expect("128 模块包必须可装载");
    let mut kernel = bundle.kernel().expect("128 模块内核必须可构造");
    let elapsed = started.elapsed();

    assert_eq!(bundle.modules.len(), MAX_MODULES);
    assert!(
        elapsed < Duration::from_secs(2),
        "128 模块装载耗时 {elapsed:?} 超过 2s 上限"
    );
    // 真装了（不是"校验过了但没注入"）：128 个模块全部被 require 到。
    assert_eq!(narrate(&mut kernel, "probe"), ["first=0;last=127"]);

    let _ = fs::remove_dir_all(&dir);
}
