//! ygg-lint：Yggdrasill Lua 静态检查（D12 第五 crate，M1 拆出实装）。
//!
//! lib 面 = [`rules::lint_source`]（纯函数：源码进、诊断出，零 IO）；
//! bin 面 = `ygg-lint` CLI（文件/目录收集 + `--json` 机器契约 + codespan
//! 人读渲染，退出码契约见 bin 头注）。**零内核依赖**：不依赖 ygg-core /
//! ygg-schema / ygg-sandbox，独立于管线运行（FR-5）。
//!
//! 首批规则（FR-8，诊断码为稳定契约——下游按 code 分支不解析 message）：
//! - `YGG-R1` 同一 handler 体内 `ygg.emit` 先于 `ygg.choice`（choice 哨兵
//!   终结静默丢弃先入队派生事件，docs/mod-authoring.md §2）；
//! - `YGG-R2` 实体句柄存 local 未落 kv（error = 顶层捕获；warning =
//!   handler 内捕获且本体内无 kv 化证据——D-11 双档口径）；
//! - `YGG-R3` `ygg.narrative.goto(` 点形态（Lua 5.4 语法错误，parse 前
//!   文本预扫——AST 在该处无法构建）；
//! - `YGG-PARSE` 解析失败（非 R3 形态的语法错误）。

pub mod diagnostic;
pub mod locate;
pub mod rules;

pub use diagnostic::{Diagnostic, Severity};
pub use rules::lint_source;
