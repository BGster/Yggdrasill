//! 规则回归：三反例逐条报错带位置 + 正例/边界零告警 + golden
//! （test-adventure）零诊断（FRD 验收锚；lint 落地后 R1/R2/R3 的静态门
//! 与内核运行期警告/内容寻址校验构成双保险）。

use ygg_lint::diagnostic::{code, Severity};
use ygg_lint::{lint_source, Diagnostic};

/// golden few-shot 种子（零诊断回归锚；路径相对 tests/ 到仓库根）。
const GOLDEN_MOD: &str = include_str!("../../../packages/mods/test-adventure/mod.lua");

fn codes(diagnostics: &[Diagnostic]) -> Vec<&'static str> {
    diagnostics.iter().map(|d| d.code).collect()
}

// ── R1：choice 前 emit ──────────────────────────────────────────────

#[test]
fn r1_emit_before_choice_is_reported_with_position() {
    let source = r#"
ygg.narrative.node("node:x", function()
  ygg.emit("chapter.advance", "to_road")
  ygg.choice("去哪儿？", { "地窖", "客房" })
end)
"#;
    let diagnostics = lint_source("mod.lua", source);
    assert_eq!(codes(&diagnostics), vec![code::R1_EMIT_BEFORE_CHOICE]);
    assert_eq!(diagnostics[0].severity, Severity::Error);
    assert_eq!(diagnostics[0].line, 3);
}

#[test]
fn r1_emit_in_choice_selected_handler_is_allowed() {
    // 正确姿势原文（mod-authoring §2）：emit 在 choice_selected handler 安全。
    let source = r#"
ygg.on("ygg.choice_selected", function(index)
  ygg.emit("chapter.advance", "to_road")
  ygg.narrative["goto"]("node:crossroad")
end)
"#;
    assert!(lint_source("mod.lua", source).is_empty());
}

#[test]
fn r1_narrate_before_choice_is_allowed() {
    // §2 边界①：narrate 不丢（直进叙述缓冲），不计入 R1。
    let source = r#"
ygg.narrative.node("node:tavern", function()
  ygg.narrate("酒馆")
  ygg.choice("带上提灯？", { "是", "否" })
end)
"#;
    assert!(lint_source("mod.lua", source).is_empty());
}

// ── R2：local 存实体句柄（D-11 双档）─────────────────────────────────

#[test]
fn r2_top_level_spawn_capture_is_error() {
    let diagnostics = lint_source("mod.lua", "local hero = ygg.entity.spawn()\n");
    assert_eq!(codes(&diagnostics), vec![code::R2_LOCAL_SPAWN]);
    assert_eq!(diagnostics[0].severity, Severity::Error);
    assert_eq!(diagnostics[0].line, 1);
}

#[test]
fn r2_handler_spawn_without_kv_is_warning() {
    let source = r#"
ygg.on("event.start", function()
  local hero = ygg.entity.spawn()
  ygg.entity.set_attribute(hero, "hp", 10)
end)
"#;
    let diagnostics = lint_source("mod.lua", source);
    assert_eq!(codes(&diagnostics), vec![code::R2_LOCAL_SPAWN]);
    assert_eq!(diagnostics[0].severity, Severity::Warning);
}

#[test]
fn r2_handler_spawn_with_kv_gate_is_clean() {
    // golden 模式：kv 门 + id 落 kv（同一 handler 内 kv.set 第二实参 = 同名 local）。
    let source = r#"
ygg.narrative.node("node:prologue", function()
  if ygg.kv.get("hero.id") == nil then
    local hero = ygg.entity.spawn()
    ygg.entity.set_attribute(hero, "hp", 10)
    ygg.kv.set("hero.id", hero)
  end
end)
"#;
    assert!(lint_source("mod.lua", source).is_empty());
}

// ── R3：goto 点形态（parse 前预扫）────────────────────────────────────

#[test]
fn r3_goto_dot_form_is_reported_with_position() {
    let source = "ygg.on(\"event.start\", function()\n  ygg.narrative.goto(\"node:x\")\nend)\n";
    let diagnostics = lint_source("mod.lua", source);
    assert_eq!(codes(&diagnostics), vec![code::R3_GOTO_DOT_FORM]);
    assert_eq!(diagnostics[0].severity, Severity::Error);
    assert_eq!(diagnostics[0].line, 2);
}

#[test]
fn r3_goto_dot_form_in_string_literal_is_ignored() {
    let source = "ygg.narrate(\"ygg.narrative.goto( 只是文本\")\n";
    assert!(lint_source("mod.lua", source).is_empty());
}

#[test]
fn r3_goto_bracket_form_is_clean() {
    let source =
        "ygg.on(\"event.start\", function()\n  ygg.narrative[\"goto\"](\"node:x\")\nend)\n";
    assert!(lint_source("mod.lua", source).is_empty());
}

// ── 解析失败与 golden ────────────────────────────────────────────────

#[test]
fn parse_error_is_reported() {
    let diagnostics = lint_source("mod.lua", "ygg.narrative.node(\"x\", function( end)");
    assert_eq!(codes(&diagnostics), vec![code::PARSE]);
}

#[test]
fn golden_test_adventure_is_clean() {
    assert!(
        lint_source("mod.lua", GOLDEN_MOD).is_empty(),
        "golden few-shot 种子必须零告警（FRD 验收锚）"
    );
}
