//! 单文件规则回归：解析门带位置 + golden（pkg-selftest）零诊断 + 副作用面只
//! 属于整包面。
//!
//! M1-E 之后单文件面只剩解析门：`YGG-R1`（choice 前 emit）/ `YGG-R2`（local
//! 存实体句柄）/ `YGG-R3`（`ygg.narrative.goto` 点形态）的判据随事件层与
//! `ygg` 的 `entity` / `ygg.narrative` 一起删除，规则与用例同步退役——留着会对一份
//! 已经不存在的 API 报错。

use ygg_lint::diagnostic::{code, Severity};
use ygg_lint::{lint_source, Diagnostic};

/// golden 真实 Mod（零诊断回归锚；路径相对 tests/ 到仓库根）。
const GOLDEN_MOD: &str = include_str!("../../../packages/mods/pkg-selftest/mod.lua");

fn codes(diagnostics: &[Diagnostic]) -> Vec<&'static str> {
    diagnostics.iter().map(|d| d.code).collect()
}

#[test]
fn parse_error_is_reported() {
    let diagnostics = lint_source("mod.lua", "world.component{ name = \"hp\", kind = end");
    assert_eq!(codes(&diagnostics), vec![code::PARSE]);
    assert_eq!(diagnostics[0].severity, Severity::Error);
    assert!(diagnostics[0].line >= 1);
}

#[test]
fn golden_pkg_selftest_is_clean() {
    assert!(
        lint_source("mod.lua", GOLDEN_MOD).is_empty(),
        "golden 真实 Mod 必须零告警"
    );
}

// ── YGG-MOD-EFFECT 只属于整包面 ──────────────────────────────────────

#[test]
fn top_level_effect_is_a_pack_rule_not_a_single_file_rule() {
    // 单文件 Mod 的顶层就是装配面：顶层建组件、写 kv 都是标准写法。
    // YGG-MOD-EFFECT 只在 `--pack` 的 `code.modules` 上生效（入口豁免）。
    let diagnostics = lint_source("mod.lua", "ygg.kv.set(\"k\", 1)\n");
    assert!(
        diagnostics.iter().all(|d| d.code != code::MOD_EFFECT),
        "{diagnostics:?}"
    );
}
