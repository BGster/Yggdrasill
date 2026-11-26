//! r1 模块装载面端到端（`Loaded` 装载口）。
//!
//! 覆盖 require 契约：同状态同 globals 共享、只经返回值跨模块、缓存与 table
//! 身份、`nil`/无返回值 → `true` 且只执行一次、chunk 名（错误文本带真实文件
//! 与行号）、循环与深度的可读链条、`require` 在模块代码里也永远可用（装载期
//! 没有第二道闸）。（原「装载期独立燃料预算」覆盖项已随 S34 燃料退役删除。）

use serde_json::json;
use ygg_sandbox::modload::LoadError;
use ygg_sandbox::modules::ModuleSet;

mod support;

use support::loaded_of;

/// 由 `(名字, 源码)` 构造模块表（测试便捷面）。
fn modules(pairs: &[(&str, &str)]) -> ModuleSet {
    ModuleSet::from_pairs(
        pairs
            .iter()
            .map(|(name, code)| ((*name).to_owned(), code.as_bytes().to_vec())),
    )
    .expect("module set")
}

/// 装载失败的错误（断言必须落在 chunk 拒绝面）。
fn load_err(mod_source: &str, set: ModuleSet) -> LoadError {
    match loaded_of(mod_source, set) {
        Ok(_) => panic!("expected the mod to fail loading"),
        Err(err) => err,
    }
}

/// 装载失败文本。
fn load_err_text(mod_source: &str, set: ModuleSet) -> String {
    let err = load_err(mod_source, set);
    assert!(
        matches!(err, LoadError::ModChunk(_)),
        "module failures must surface as a chunk rejection, got: {err:?}"
    );
    err.to_string()
}

/// 入口顶部就把观测值写进 KV，测试读回来逐字比对。
///
/// 观测面为什么走 KV 而不是事件：事件层（`ygg` 的 `on` / `ygg` 的 `narrate`）已随
/// 信封层删除；入口 chunk 豁免 MOD_EFFECT，所以「装配 + 留痕」是它的事。
fn probe_of(loaded: &ygg_sandbox::modload::Loaded, topic: &str) -> serde_json::Value {
    let kv = loaded.kv();
    let kv = kv.borrow();
    match kv.get(topic) {
        Some(ygg_core::value::Value::Str(text)) => json!(text),
        other => panic!("观测值缺失或不是字符串：{topic} / {other:?}"),
    }
}

#[test]
fn module_state_is_shared_only_through_returned_values() {
    // src.util 持有一个表 + 一个顶层 local；src.user 只能经返回值拿到它。
    let loaded = loaded_of(
        r#"
local util = require("src.util")
require("src.user")
ygg.kv.set("probe", "count=" .. tostring(util.count) .. ";hidden=" .. tostring(hidden))
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

    // src.user 的 bump 经共享 table 生效（1）；src.util 的顶层 local 不跨模块
    // 可见（入口侧 `hidden` 是未定义全局 → nil）。
    assert_eq!(probe_of(&loaded, "probe"), json!("count=1;hidden=nil"));
}

#[test]
fn module_runs_once_and_keeps_table_identity() {
    let loaded = loaded_of(
        r#"
local a = require("src.count")
local b = require("src.count")
ygg.kv.set("probe", "execs=" .. tostring(execs) .. ";same=" .. tostring(a == b) .. ";tag=" .. a.tag)
"#,
        modules(&[(
            "src.count",
            "execs = (execs or 0) + 1\nreturn { tag = \"count\" }\n",
        )]),
    )
    .expect("multi-file mod loads");

    // 只执行一次；两次 require 拿到同一个 table 对象。
    assert_eq!(
        probe_of(&loaded, "probe"),
        json!("execs=1;same=true;tag=count")
    );
}

#[test]
fn nil_module_is_cached_as_true_and_runs_once() {
    let loaded = loaded_of(
        r#"
local v = require("src.side")
local again = require("src.side")
ygg.kv.set("probe", "v=" .. tostring(v) .. ";runs=" .. tostring(runs)
  .. ";nilresult_cached=" .. tostring(v == true and again == true))
"#,
        modules(&[("src.side", "runs = (runs or 0) + 1\n")]),
    )
    .expect("multi-file mod loads");

    // 无返回值 → 缓存 `true`；重复 require 不重复执行。
    assert_eq!(
        probe_of(&loaded, "probe"),
        json!("v=true;runs=1;nilresult_cached=true")
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
    let ok = loaded_of(r#"require("m0")"#, chain(64));
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
fn require_is_available_from_module_code_not_just_the_entry() {
    // `require` 没有第二道闸：模块代码里（不在入口 chunk 里）照样能 require，
    // 拿到的是同一份模块表——懒加载不需要任何额外开关。
    let loaded = loaded_of(
        r#"
local a = require("src.a")
ygg.kv.set("probe", "late=" .. a.tag() .. ";kind=" .. type(require))
"#,
        modules(&[
            (
                "src.a",
                r#"local M = {}
function M.tag() return require("src.b").tag end
return M
"#,
            ),
            ("src.b", "return { tag = \"util\" }"),
        ]),
    )
    .expect("multi-file mod loads");
    assert_eq!(probe_of(&loaded, "probe"), json!("late=util;kind=function"));
}

#[test]
fn single_file_mod_path_is_unchanged() {
    // 回归闸门：不注入模块表的构造路径行为不变——require 仍是可读拒绝。
    let loaded = loaded_of(
        r#"
local ok, err = pcall(require, "src.util")
ygg.kv.set("probe", "denied=" .. tostring(ok == false) .. ";kind=" .. type(err)
  .. ";readable=" .. tostring(tostring(err):find("sandbox denied", 1, true) ~= nil))
"#,
        ModuleSet::new(),
    )
    .expect("single-file mod loads");
    // `pcall` 拿到的错误对象是 mlua 包装的 error userdata（与旧拒绝桩同形，
    // mlua 对 Rust 侧错误的既有语义），其 `tostring` 即人读文案。
    assert_eq!(
        probe_of(&loaded, "probe"),
        json!("denied=true;kind=userdata;readable=true")
    );
}
