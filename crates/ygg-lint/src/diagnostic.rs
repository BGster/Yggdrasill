//! 诊断结构（D-14）：lib 面的稳定产物，机器面（`--json` serde 序列化）与
//! 人读面（bin 侧 codespan 渲染）共用同一份 Diagnostic。

use serde::Serialize;

/// 严重级（`--json` 序列化为 "error" / "warning"；lint 门失败判据 =
/// 存在 error 级诊断，warning 放行进报告——D-11）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

/// 诊断码（稳定契约，与内核 A1 稳定码纪律同构）。
pub mod code {
    /// R1：handler 体内 emit 先于 choice（choice 终结丢事件）。
    pub const R1_EMIT_BEFORE_CHOICE: &str = "YGG-R1";
    /// R2：实体句柄存 local 未落 kv（顶层 = error，handler 内未 kv 化 = warning）。
    pub const R2_LOCAL_SPAWN: &str = "YGG-R2";
    /// R3：`ygg.narrative.goto(` 点形态（Lua 5.4 语法错误）。
    pub const R3_GOTO_DOT_FORM: &str = "YGG-R3";
    /// 解析失败（非 R3 形态；message 为 full_moon 错误原文）。
    pub const PARSE: &str = "YGG-PARSE";
}

/// 一条 lint 诊断（line/column 由 offset 推导，均 1 起、字节列口径；
/// offset/endOffset 供 codespan Label 划线）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    /// 文件显示名（bin 按调用方给定路径原样回填）。
    pub file: String,
    pub offset: usize,
    pub end_offset: usize,
    pub line: usize,
    pub column: usize,
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
    /// 修复建议（人读 + 反喂重生成回路的 hint 面）。
    pub fix: String,
}

impl Diagnostic {
    pub fn is_error(&self) -> bool {
        matches!(self.severity, Severity::Error)
    }
}
