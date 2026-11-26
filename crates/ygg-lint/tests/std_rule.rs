//! `YGG-STD` 回归：沙箱白名单外的全局 / `math` 收窄表外的成员 / 浮点幂 /
//! 受控 `require`。
//!
//! 核心论证在「不误报」一侧：注释、长短字符串，以及**只是包含**关键字的标识符
//! （`os_name` / `load_level` / `table.io`）都不能报——逐行文本匹配做不到这一
//! 点，AST 天然做得到，这也是本规则不用正则的理由。

use ygg_lint::diagnostic::{code, Severity};
use ygg_lint::pack::{lint_pack, PackOutcome};
use ygg_lint::{lint_source, Diagnostic};

fn std_diagnostics(source: &str) -> Vec<Diagnostic> {
    lint_source("mod.lua", source)
        .into_iter()
        .filter(|diagnostic| diagnostic.code == code::STD)
        .collect()
}

fn assert_clean(source: &str) {
    let diagnostics = lint_source("mod.lua", source);
    assert!(
        diagnostics.is_empty(),
        "源码不该有诊断：{source:?} → {diagnostics:?}"
    );
}

// ── 该报的 ──────────────────────────────────────────────────────────

#[test]
fn banned_global_call_names_the_global_and_its_position() {
    let diagnostics = std_diagnostics("os.remove(\"x\")\n");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].severity, Severity::Error);
    assert_eq!(
        (diagnostics[0].line, diagnostics[0].column),
        (1, 1),
        "定到 `os` 本身，不是整条调用"
    );
    assert!(
        diagnostics[0].message.contains("`os`"),
        "{}",
        diagnostics[0].message
    );
}

#[test]
fn bare_global_reference_is_reported() {
    // 间接持有整张全局表同样能绕过后面的成员检查（`local x = os`）。
    let diagnostics = std_diagnostics("local x = os\nreturn x\n");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(
        diagnostics[0].message.contains("`os`"),
        "{}",
        diagnostics[0].message
    );
}

#[test]
fn removed_math_member_is_reported() {
    let diagnostics = std_diagnostics("local y = math.sin(x)\nreturn y\n");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(
        diagnostics[0].message.contains("math.sin"),
        "{}",
        diagnostics[0].message
    );
}

#[test]
fn float_power_operator_is_reported() {
    let diagnostics = std_diagnostics("local y = x ^ 2\nreturn y\n");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(
        diagnostics[0].message.contains("`^`"),
        "{}",
        diagnostics[0].message
    );
}

#[test]
fn require_alias_is_reported() {
    let diagnostics = std_diagnostics("local r = require\nreturn r\n");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(
        diagnostics[0].message.contains("require"),
        "{}",
        diagnostics[0].message
    );
}

#[test]
fn computed_require_name_is_reported() {
    let diagnostics = std_diagnostics("local name = \"src.util\"\nreturn require(name)\n");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(
        diagnostics[0].message.contains("字面量"),
        "{}",
        diagnostics[0].message
    );
}

// ── 不该报的（逐行文本匹配会在这里翻车） ─────────────────────────────

#[test]
fn comments_are_not_code() {
    assert_clean("-- os.remove(\"x\")\nlocal x = 1\nreturn x\n");
    assert_clean("--[[\nio.write(\"x\")\nmath.sin(1)\n]]\nreturn 1\n");
}

#[test]
fn short_strings_are_not_code() {
    assert_clean("local s = \"os.remove\"\nreturn s\n");
    assert_clean("local s = 'io.write'\nreturn s\n");
}

#[test]
fn long_strings_are_not_code() {
    assert_clean("local s = [==[ io.write ]==]\nreturn s\n");
    assert_clean("local s = [[ os.getenv ]]\nreturn s\n");
}

#[test]
fn identifiers_merely_containing_a_token_are_clean() {
    assert_clean("local function load_level() return 1 end\nreturn load_level()\n");
    assert_clean("local os_name = \"linux\"\nreturn os_name\n");
    assert_clean("local loaded, loader = true, nil\nreturn loaded, loader\n");
}

#[test]
fn indexed_field_named_like_a_banned_global_is_clean() {
    // `table.io` 的 `io` 是点索引名，不是全局名——只有 `io.write` 才是。
    assert_clean("local t = { io = 1 }\nreturn t.io\n");
    assert_clean("local t = {}\nreturn t[\"io\"]\n");
}

#[test]
fn allowed_math_members_and_globals_are_clean() {
    assert_clean("local y = math.floor(1.5) + math.max(1, 2)\nreturn y\n");
    assert_clean("local s = string.format(\"%d\", 1)\nreturn table.concat({ s }, \",\")\n");
}

#[test]
fn require_literal_call_is_allowed() {
    assert_clean("local util = require(\"src.util\")\nreturn util\n");
    assert_clean("local util = require \"src.util\"\nreturn util\n");
}

// ── 整包接线 ────────────────────────────────────────────────────────

const PACK: &str = r#"{"code": {"entry": "mod.lua", "modules": ["src/util.lua"]}}"#;

#[test]
fn clean_pack_has_zero_diagnostics() {
    let outcome = lint_pack("pack.json", PACK, |relative| match relative {
        "mod.lua" => Some(
            "local util = require(\"src.util\")\n\
             return { label = string.format(\"%d\", math.floor(1.5)), pick = util.pick }\n"
                .to_owned(),
        ),
        "src/util.lua" => Some(
            "return { pick = function() return table.concat({ \"a\" }, \",\") end }\n".to_owned(),
        ),
        _ => None,
    });
    match outcome {
        PackOutcome::Modules(report) => {
            assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
            assert_eq!(report.modules, 1);
        }
        PackOutcome::SingleFile => panic!("`code` 段存在就是多文件 Mod"),
    }
}

#[test]
fn pack_reports_std_on_entry_and_modules() {
    // 入口与模块都要跑：入口同样是 Lua，`os` 在那里也是 nil。
    let pack = r#"{"code": {"entry": "mod.lua", "modules": ["src/util.lua"]}}"#;
    let outcome = lint_pack("pack.json", pack, |relative| match relative {
        "mod.lua" => Some("local util = require(\"src.util\")\nreturn os.time()\n".to_owned()),
        "src/util.lua" => Some("return { now = function() return os.time() end }\n".to_owned()),
        _ => None,
    });
    match outcome {
        PackOutcome::Modules(report) => {
            let std: Vec<&Diagnostic> = report
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == code::STD)
                .collect();
            assert_eq!(std.len(), 2, "{:?}", report.diagnostics);
            let files: Vec<&str> = std
                .iter()
                .map(|diagnostic| diagnostic.file.as_str())
                .collect();
            assert_eq!(files, vec!["mod.lua", "src/util.lua"]);
        }
        PackOutcome::SingleFile => panic!("`code` 段存在就是多文件 Mod"),
    }
}
