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
    /// 解析失败（message 为 full_moon 错误原文）。
    pub const PARSE: &str = "YGG-PARSE";
    /// `--pack`：`pack.json` 的 `code` 段形状不合法（字段缺失 / 类型不符 /
    /// 重复登记 / 入口与模块重叠）。
    pub const PACK_DECL: &str = "YGG-PACK";
    /// `--pack`：模块路径不能规范化为模块名（标识符字符集 / 段内含点 /
    /// 非 `.lua` / 绝对路径 / 超长）。
    pub const MOD_NAME: &str = "YGG-MOD-NAME";
    /// `--pack`：`code.modules` 声明了但磁盘上读不到该文件。
    pub const MOD_MISSING: &str = "YGG-MOD-MISSING";
    /// `--pack`：源码 `require` 了但仍未在 `code.modules` 登记（装载期必挂）。
    pub const MOD_UNDECLARED: &str = "YGG-MOD-UNDECLARED";
    /// `--pack`：循环依赖（链条文本与装载期同形）。
    pub const MOD_CYCLE: &str = "YGG-MOD-CYCLE";
    /// `--pack`：模块数 / 装载深度 / 单模块或整包字节数超限（`ygg_modules`
    /// 的 `MAX_MODULES` / `MAX_DEPTH` / `MAX_CODE_BYTES` 同一份预算）。
    ///
    /// M1-E 前这条叫 `YGG-MOD-LIMIT`，与「不可达警告」（`YGG-MOD-UNUSED`）
    /// 一同钉在旧的模块运行期模型上；收窄后只剩「超预算」一个语义，码随之
    /// 改名。
    pub const MOD_BUDGET: &str = "YGG-MOD-BUDGET";
    /// `--pack`：模块 chunk **顶层**触碰有副作用的 host API（调用或取引用）。
    ///
    /// 与 `ygg-sandbox` 装载期的影子表（`ShadowBanned`）是**同一条不变量**的
    /// 两套执行面：影子表在装载时拒绝，这条在编辑 / CI 阶段就报——静态面更
    /// 早（不必真的装载）也更完整（懒加载没走到也一样报）。两份清单只有一份
    /// （[`ygg_modules::effects`]）。
    pub const MOD_EFFECT: &str = "YGG-MOD-EFFECT";
    /// 沙箱白名单面（单文件与 `--pack` 都跑）：引用了环境装配时被摘掉的全局
    /// （`os` / `io` / `debug` / `package` …）、`math` 收窄表外的成员
    /// （`math.sin` …），或用了浮点幂 `^`。
    ///
    /// 与 `ygg-sandbox` 的 deny-by-default 环境是**同一条不变量**的两套执行面：
    /// 装载期白名单拦的是真的调用，这条在编辑期就报——不必执行、也不必真的
    /// 跑到那一行（懒加载分支里同样报）。
    pub const STD: &str = "YGG-STD";
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
