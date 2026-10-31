//! r1 模块装载面端到端（`KernelFacade` 公共路径）。
//!
//! 覆盖 require 契约：同状态同 globals 共享、只经返回值跨模块、缓存与 table
//! 身份、`nil`/无返回值 → `true` 且只执行一次、chunk 名（错误文本带真实文件
//! 与行号）、循环与深度的可读链条、**调用时机不受装载期限制**（封的只是注册
//! 面，见 `host.seal`）、装载期独立燃料预算（不挤占单转换预算）。

use serde_json::{json, Value as Json};
use ygg_sandbox::driver::{KernelError, KernelFacade, DEFAULT_FUEL_BUDGET};
use ygg_sandbox::fuel::{self, LoadBudget};
use ygg_sandbox::modules::ModuleSet;
use ygg_sandbox::sandbox::Sandbox;

/// 由 `(名字, 源码)` 构造模块表（测试便捷面）。
fn modules(pairs: &[(&str, &str)]) -> ModuleSet {
    ModuleSet::from_pairs(
        pairs
            .iter()
            .map(|(name, code)| ((*name).to_owned(), code.as_bytes().to_vec())),
    )
    .expect("module set")
}

/// 装载失败的错误（断言必须落在 `ModLoad` 面）。
fn load_err(mod_source: &str, set: ModuleSet) -> KernelError {
    match KernelFacade::with_modules(mod_source, set) {
        Ok(_) => panic!("expected the mod to fail loading"),
        Err(err) => err,
    }
}

/// 装载失败文本。
fn load_err_text(mod_source: &str, set: ModuleSet) -> String {
    let err = load_err(mod_source, set);
    assert!(
        matches!(err, KernelError::ModLoad(_)),
        "module failures must surface as ModLoad, got: {err:?}"
    );
    err.to_string()
}

fn parse(reply: &str) -> Json {
    serde_json::from_str(reply).expect("json envelope")
}

fn reply_ok(reply: &str) -> Json {
    let parsed = parse(reply);
    assert_eq!(parsed["ok"], json!(true), "expected ok reply: {reply}");
    parsed
}

#[test]
fn module_state_is_shared_only_through_returned_values() {
    // src.util 持有一个表 + 一个顶层 local；src.user 只能经返回值拿到它。
    let kernel = KernelFacade::with_modules(
        r#"
local util = require("src.util")
require("src.user")
ygg.on("probe", function()
  ygg.narrate("count=" .. tostring(util.count) .. ";hidden=" .. tostring(hidden))
end)
"#,
        modules(&[
            (
                "src.util",
                r#"local hidden = "secret"
local util = { count = 0 }
function util.bump() util.count = util.count + 1 end
return util
"#,
            ),
            (
                "src.user",
                r#"local util = require("src.util")
util.bump()
"#,
            ),
        ]),
    )
    .expect("multi-file mod loads");

    let mut kernel = kernel;
    let parsed = reply_ok(&kernel.dispatch("probe", "0"));
    // src.user 的 bump 经共享 table 生效（1）；src.util 的顶层 local 不跨模块
    // 可见（handler 侧 `hidden` 是未定义全局 → nil）。
    assert_eq!(parsed["narrative"], json!(["count=1;hidden=nil"]));
}

#[test]
fn module_runs_once_and_keeps_table_identity() {
    let mut kernel = KernelFacade::with_modules(
        r#"
local a = require("src.count")
local b = require("src.count")
ygg.on("probe", function()
  ygg.narrate("execs=" .. tostring(execs) .. ";same=" .. tostring(a == b) .. ";tag=" .. a.tag)
end)
"#,
        modules(&[(
            "src.count",
            "execs = (execs or 0) + 1\nreturn { tag = \"count\" }\n",
        )]),
    )
    .expect("multi-file mod loads");

    let parsed = reply_ok(&kernel.dispatch("probe", "0"));
    // 只执行一次；两次 require 拿到同一个 table 对象。
    assert_eq!(parsed["narrative"], json!(["execs=1;same=true;tag=count"]));
}

#[test]
fn nil_module_is_cached_as_true_and_runs_once() {
    let mut kernel = KernelFacade::with_modules(
        r#"
local v = require("src.side")
local again = require("src.side")
ygg.on("probe", function()
  ygg.narrate("v=" .. tostring(v) .. ";runs=" .. tostring(runs)
    .. ";nilresult_cached=" .. tostring(v == true and again == true))
end)
"#,
        modules(&[("src.side", "runs = (runs or 0) + 1\n")]),
    )
    .expect("multi-file mod loads");

    let parsed = reply_ok(&kernel.dispatch("probe", "0"));
    // 无返回值 → 缓存 `true`；重复 require 不重复执行。
    assert_eq!(
        parsed["narrative"],
        json!(["v=true;runs=1;nilresult_cached=true"])
    );
}

#[test]
fn module_errors_carry_the_real_file_and_line() {
    // 模块 chunk 名 = `@src/util.lua`：错误文本带真实文件与行号（不是 `[C]` 或
    // `@mod.lua`）——traceback 可定位到模块自身。
    let text = load_err_text(
        r#"require("src.util")"#,
        modules(&[("src.util", "local x = 1\n\nerror(\"boom\")\nreturn x\n")]),
    );
    assert!(text.contains("src/util.lua:3"), "real file + line: {text}");
    assert!(text.contains("boom"), "original message kept: {text}");
    assert!(
        text.contains("mod.lua:1"),
        "require call site kept in the chain: {text}"
    );
}

#[test]
fn self_cycle_and_two_module_cycle_report_the_chains() {
    // 自环：链条从首次出现处起算。
    let self_cycle = load_err_text(
        r#"require("src.a")"#,
        modules(&[("src.a", "require(\"src.a\")")]),
    );
    assert!(
        self_cycle.contains("circular require: src.a -> src.a"),
        "self cycle chain: {self_cycle}"
    );

    // 两模块互依赖：完整链条 + require 发生点。
    let mutual = load_err_text(
        r#"require("src.a")"#,
        modules(&[
            ("src.a", "require(\"src.b\")"),
            ("src.b", "require(\"src.a\")"),
        ]),
    );
    assert!(
        mutual.contains("circular require: src.a -> src.b -> src.a"),
        "mutual cycle chain: {mutual}"
    );
    assert!(
        mutual.contains("src/b.lua:1"),
        "cycle reported at the require site: {mutual}"
    );
}

/// 生成 `n` 层依赖链（`m0` → `m1` → … → 末端返回 `true`）。
fn chain(n: usize) -> ModuleSet {
    let mut pairs = Vec::new();
    for i in 0..n {
        let code = if i + 1 < n {
            format!("return require(\"m{}\")", i + 1)
        } else {
            "return true".to_owned()
        };
        pairs.push((format!("m{i}"), code.into_bytes()));
    }
    ModuleSet::from_pairs(pairs).expect("chain module set")
}

#[test]
fn depth_limit_is_explicit_and_sits_at_the_constant() {
    // 64 层（默认上限）正常装载。
    let ok = KernelFacade::with_modules(
        r#"require("m0")
ygg.on("probe", function() ygg.narrate("deep-ok") end)
"#,
        chain(64),
    );
    assert!(
        ok.is_ok(),
        "64 nested modules must load: {}",
        ok.err().map(|err| err.to_string()).unwrap_or_default()
    );

    // 65 层由显式判栈拒绝（不是 Lua/Rust 栈溢出）。
    let text = load_err_text(r#"require("m0")"#, chain(65));
    assert!(
        text.contains("require depth exceeded (64 nested modules)"),
        "explicit depth rejection: {text}"
    );
    assert!(
        text.contains("m0 -> m1 -> m2 -> … -> m62 -> m63 -> m64"),
        "chain reported (middle elided): {text}"
    );
}

#[test]
fn unknown_module_is_denied_with_the_require_chain() {
    let text = load_err_text(r#"require("src.missing")"#, modules(&[]));
    assert!(text.contains("sandbox denied"), "readable denial: {text}");
    assert!(
        text.contains("module 'src.missing' is not available"),
        "names the module: {text}"
    );
    assert!(
        text.contains("require chain: src.missing"),
        "chain reported: {text}"
    );
    assert!(text.contains("mod.lua:1"), "call site reported: {text}");

    // 深层未登记：链条自外向内。
    let nested = load_err_text(
        r#"require("src.a")"#,
        modules(&[("src.a", "require(\"src.zzz\")")]),
    );
    assert!(
        nested.contains("require chain: src.a -> src.zzz"),
        "nested chain reported: {nested}"
    );
    assert!(
        nested.contains("src/a.lua:1"),
        "call site is the requiring module: {nested}"
    );
}

#[test]
fn invalid_module_names_are_readable_denials() {
    for (raw, expected) in [
        ("src/util", "contains a forbidden character"),
        ("/abs/path", "must be relative"),
        ("..", "empty segment"),
        ("src.", "empty segment"),
        ("src\\util", "contains a backslash"),
    ] {
        let text = load_err_text(&format!("require({raw:?})"), modules(&[]));
        assert!(
            text.contains(expected),
            "require({raw:?}) must reject with {expected:?}: {text}"
        );
    }
}

#[test]
fn require_has_no_load_time_boundary() {
    // r1：`require` 不再封在装载期——封的是注册面（`host.seal` 只锁 `ygg.on`），
    // 不是模块面。这两件事共用一个开关曾经把懒加載直接封死。
    // 完整的懒加载契约（缓存 / 只执行一次 / 反射边界）在 tests/modules_effects.rs。
    let mut kernel = KernelFacade::with_modules(
        r#"
ygg.on("probe", function()
  local util = require("src.util")
  ygg.narrate("late=" .. util.tag .. ";kind=" .. type(require))
end)
"#,
        modules(&[("src.util", "return { tag = \"util\" }")]),
    )
    .expect("multi-file mod loads");

    // 装载结束后（= seal 之后）的 dispatch 里 require 仍然可用。
    let parsed = reply_ok(&kernel.dispatch("probe", "0"));
    assert_eq!(parsed["narrative"], json!(["late=util;kind=function"]));
}

#[test]
fn load_fuel_is_independent_of_the_transition_budget() {
    // 装载期烧掉远超单转换预算（DEFAULT_FUEL_BUDGET = 100k 指令单位）的模块：
    // 若装载与转换共享预算，这里会在装载期以 fuel exhausted 失败。
    let heavy = format!(
        "local x = 0 for i = 1, {} do x = x + i end return x",
        DEFAULT_FUEL_BUDGET * 2
    );
    let mut kernel = KernelFacade::with_modules(
        r#"
local n = require("src.heavy")
ygg.on("probe", function() ygg.narrate("heavy=" .. tostring(n > 0)) end)
"#,
        modules(&[("src.heavy", heavy.as_str())]),
    )
    .expect("heavy module loads under the load budget");

    // 转换预算未被装载期挤占：本条 dispatch 正常工作（且在满载预算下）。
    let parsed = reply_ok(&kernel.dispatch("probe", "0"));
    assert_eq!(parsed["narrative"], json!(["heavy=true"]));
}

#[test]
fn load_budget_exhaustion_is_reported_and_leaves_the_transition_budget_alone() {
    let sandbox = Sandbox::new().expect("sandbox construction");
    let load = LoadBudget::new(2_000);
    let meter =
        fuel::install_with_load_budget(sandbox.lua(), 50_000, load.clone()).expect("fuel install");

    // 装载期：独立预算耗尽 → 装载面文案（不冒充转换预算）。
    load.begin();
    let msg = sandbox
        .lua()
        .load("local x = 0 for i = 1, 100000 do x = x + i end")
        .exec()
        .expect_err("load budget must be drained")
        .to_string();
    load.end();
    assert!(
        msg.contains("module load budget consumed"),
        "load-phase message expected: {msg}"
    );
    assert!(
        !msg.contains("transition budget"),
        "load phase must not report as transition exhaustion: {msg}"
    );

    // 转换预算一单位未动（装载期消耗只记在装载预算上）。
    assert_eq!(meter.remaining(), 50_000, "transition budget untouched");
    assert_eq!(load.remaining(), 0, "load budget drained");
}

#[test]
fn single_file_mod_path_is_unchanged() {
    // 回归闸门（与 tests/kernel_facade.rs 同形的最小面）：不注入模块表的构造
    // 路径行为不变——require 仍是可读拒绝，事件照常派发。
    let mut kernel = KernelFacade::new(
        r#"
ygg.on("probe", function()
  local ok, err = pcall(require, "src.util")
  ygg.narrate("denied=" .. tostring(ok == false) .. ";kind=" .. type(err)
    .. ";readable=" .. tostring(tostring(err):find("sandbox denied", 1, true) ~= nil))
end)
"#,
    )
    .expect("single-file mod loads");
    let parsed = reply_ok(&kernel.dispatch("probe", "0"));
    // `pcall` 拿到的错误对象是 mlua 包装的 error userdata（与旧拒绝桩同形，
    // mlua 对 Rust 侧错误的既有语义），其 `tostring` 即人读文案。
    assert_eq!(
        parsed["narrative"],
        json!(["denied=true;kind=userdata;readable=true"])
    );
}
