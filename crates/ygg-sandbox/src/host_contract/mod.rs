//! host ↔ Mod 的**显式契约**（通用 Mod 工具的零硬编码面）。
//!
//! 四样以前靠宿主字面量硬编码的东西——事件名与叙事约定、inbox 记录字段与
//! kind → 动作、结算判定项、自动推进阈值——加载期从 Mod 自己的声明里读：
//!
//! | 声明位置 | 段 | 消费方 |
//! | --- | --- | --- |
//! | `pack.json` | `agent`（**必填**，八个字段） | `examples/egg.rs`、`src/playtest.rs` |
//! | `pack.json` | `inboxRoutes`（**必填**，三个保留键 + 若干 kind） | `examples/egg.rs` |
//! | `data/config.json` | `settlement`（**必填**） | `examples/egg.rs --settle` |
//! | `data/config.json` | `drive`（**必填**） | `examples/egg.rs` |
//! | `pack.json` | `tools`（**可选**，缺省 / 空对象 = 本 Mod 不请求外部工具） | `examples/egg.rs` |
//! | `data/config.json` | `allowedTools`（**可选**，缺省 = 不允许任何工具） | `examples/egg.rs` |
//!
//! **无默认值纪律**：所有段都是必填，缺失 / 字段缺失 / 类型不符 / 非正数一律
//! 返回可读错误（带缺失字段名 + 应有形状 + 文档定位），由调用方 fail-closed
//! （报错退出或 `ok=false` 报告）——宿主**不**提供任何回退默认值。旧 Mod 因此
//! 必须在升级时补齐这四段（见 `docs/DESIGN.md` 与
//! `.ygg/mods/agent-seed/README.md` 的「host 契约」章节）。
//!
//! # 两层工具白名单（`tools` / `allowedTools`，两者都可选）
//!
//! 第一层（`pack.json` 的 `tools`）定**形状与能力**：每个工具声明 kind（`cli`
//! 直接 spawn、不经 shell；`http` 经 `curl`）、超时、输出上限、是否只读、结果
//! 事件名。宿主逐条校验，**半声明**（cli 带 `url`、http 带 `program`）一律拒绝。
//! 第二层（`data/config.json` 的 `allowedTools`）定**本局放行清单**：`tools` 里
//! 声明了但没放行的 id 在**装载期**即失败（[`require_tool_allowlist`]），不推迟到
//! 执行时。两层都缺失 = 工具能力整体不可用（既有 Mod 零改动照常工作）。
//!
//! **诚实声明**：`allowedTools` 与 `pack.json` 同在 Mod 目录、同一个作者——它是
//! **防漂移 / 防半声明**的门禁，**不是**防恶意 Mod 的安全边界。真正的边界是第一层
//! 的形状限制（无 shell、无绝对路径、无 `..` 路径段、`$ENV{}` 间接取凭据）+ `readOnly`
//! + 审计面。若将来要真正的边界，放行清单应搬出 Mod 目录（运维侧持有）。
//!
//! # 代码面契约（`pack.json` 的 `code` 段，可选）
//!
//! 同一份 `pack.json` 上还有第五个声明面，但它**不由本模块解析**（消费方是
//! [`crate::pack`] 与 [`crate::modload`]）——列在这里是因为它与上面四段同属
//! 「宿主不猜、出事即失败」的契约面，且两者容易漂移：
//!
//! | 面 | 口径 | 消费方 |
//! | --- | --- | --- |
//! | 登记面 | `code.{entry, modules[]}`：入口**不**登记为模块（同一源码两个装载身份让缓存与定位歧义）；模块**逐条枚举**，无 glob；`entry ↔ modules` 与 `manifest.json` 的 `code` 数组双向相等 | [`crate::pack::CodeSpec`]、[`crate::modload::ModBundle`] |
//! | 装载期边界 | **只有注册面有边界**：`host.seal` 封 `ygg.on`（运行期注册禁令）。`require` **不**在边界上——装载期与 handler 内部一视同仁（懒加载成立） | [`crate::modules`]、[`crate::host_api::HostApi::seal`] |
//! | 副作用边界 | **入口可注册，模块只导出**：入口 chunk 顶层就是装配清单（注册 / 写状态 / 输出都是它的职责）；模块 chunk 顶层只做纯导出（定义函数与常量），注册 / 写状态 / 输出一律搬进模块函数体，由入口显式调用（`M.install(deps)`）。理由：`ygg.on` 的注册序就是派发序——模块自己注册，等于让「文件怎么拆」决定「运行时行为」，拆文件或调 require 顺序就改行为。禁止面清单在 `ygg_modules::effects`，运行期由影子表执行、静态由 `ygg-lint --pack` 的 `YGG-MOD-EFFECT` 报 | [`crate::modules`]、[`ygg_modules::effects`] |
//! | 错误前缀口径 | 装载器自判的拒绝带**调用点**（`src/a.lua:3: <原因>`）；模块 chunk 自身的错误按 Lua 原生错误链浮出（每层 `path:line:` + traceback）——两类同形，都带真实文件与行号，禁止「拼回单文件再报错」 | [`crate::modules`] |
//!
//! 与四段 host 契约的差别：`code` 段可以整段缺失（缺 = 单文件 Mod，行为逐字节
//! 不变）；存在则**全量校验**，不允许半声明。边界细节见 `docs/DESIGN.md`
//! §3.2「Mod 包与多文件纯度契约」。
//!
//! 绑定特定 Mod 的应用（`examples/adventure.rs`、`clients/godot`、
//! `packages/client-web`…）不属本模块的消费方：它们各自服务固定 Mod，文件头
//! 注明绑定对象即可（归属边界见 `data/playbook/ch4-mod-host.json` 的 `contract.literal-ownership`）。

use std::fs;
use std::path::Path;

use serde_json::Value as Json;

/// 契约文档定位（所有契约错误文案的统一尾巴）。
pub const CONTRACT_DOC: &str = "见 .ygg/mods/agent-seed/README.md 的「host 契约」章节";

/// 读一个 JSON 文件（缺失 / 非法 → 可读错误，不做任何兜底）。
pub fn read_json(path: &Path) -> Result<Json, String> {
    let text =
        fs::read_to_string(path).map_err(|err| format!("读取 {} 失败：{err}", path.display()))?;
    serde_json::from_str(&text).map_err(|err| format!("{} 不是合法 JSON：{err}", path.display()))
}

/// JSON 值的类型名（错误文案用，避免把整段值打进日志）。
fn json_kind(value: &Json) -> &'static str {
    match value {
        Json::Null => "null",
        Json::Bool(_) => "布尔",
        Json::Number(_) => "数字",
        Json::String(_) => "字符串",
        Json::Array(_) => "数组",
        Json::Object(_) => "对象",
    }
}

// —— 模块装配：按契约面拆文件，纯搬移（S-25 批四）——
mod agent;
mod drive;
mod inbox;
mod settlement;
mod tools;

pub use agent::AgentContract;
pub use drive::DriveLimits;
pub use inbox::{
    join_segments, sanitize_field, InboxRoute, InboxRoutes, InboxWriteBinding, WriteShape,
};
pub use settlement::SettlementPolicy;
pub use tools::{
    require_tool_allowlist, ToolAccess, ToolKind, ToolRegistry, ToolSpec, SELF_PROGRAM,
};
