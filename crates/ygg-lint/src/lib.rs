//! ygg-lint：Yggdrasill Lua 静态检查（D12 第五 crate，M1 拆出实装）。
//!
//! lib 面 = [`rules::lint_source`]（单文件：源码进、诊断出，零 IO）与
//! [`pack::lint_pack`]（多文件 Mod：声明 + 逐模块 parse + 依赖图，读文件由调用方
//! 注入）；bin 面 = `ygg-lint` CLI（文件/目录收集 + `--pack <mod目录>` +
//! `--json` 机器契约 + codespan 人读渲染，退出码契约见 bin 头注）。
//!
//! **零内核依赖**：不依赖 ygg-core / ygg-schema / ygg-sandbox，独立于管线运行
//! （FR-5）。唯一的 `ygg-*` 依赖是 [`ygg_modules`]——模块名/路径/上限的纯解析面
//! （**零依赖叶子 crate**，无 Lua、无 IO），装载器与打包面用的也是它。为了复用
//! 这几百行判定而依赖 `ygg-sandbox` 会把 lint 拖进 mlua + 整个 host 面，那才是
//! 这条纪律要拦的东西。
//!
//! 当前规则（FR-8，诊断码为稳定契约——下游按 code 分支不解析 message）：
//! - `YGG-PARSE` 解析失败（语法错误按错误区间报真实行列）；
//! - `YGG-STD` 沙箱白名单面：环境装配时摘掉的全局（`os` / `io` / `debug` /
//!   `package` / `coroutine` / `load*` / `dofile` / `loadfile` / `collectgarbage`）、
//!   `math` 收窄表外的成员（`math.sin` …）与浮点幂 `^` 一律报 error；单文件与
//!   `--pack` 都跑（纯单文件判定，不跨文件、不执行）；
//! - `YGG-MOD-EFFECT` `--pack` 面：模块 chunk **顶层**调用 / 引用有副作用的 host
//!   API。禁止面清单来自 [`ygg_modules::effects`]——与装载期影子表同一份，lint
//!   不另写名单。
//!
//! `YGG-R1` / `YGG-R2` / `YGG-R3` 的判据随 M1-E 删除事件层而退役，不再有对应
//! 代码（不要按旧文档期待它们报错）。

pub mod diagnostic;
pub mod locate;
pub mod pack;
pub mod rules;

pub use diagnostic::{Diagnostic, Severity};
pub use pack::{lint_pack, PackOutcome, PackReport};
pub use rules::lint_source;
