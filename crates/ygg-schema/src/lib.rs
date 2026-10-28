//! ygg-schema：Mod 资源格式契约的 Rust 侧校验（JSON Schema draft 2020-12）。
//!
//! 职责（D-A2 / FR6 / FR12）：
//! - `manifest`：资源包 manifest 模型 + manifest.json 自身 schema 校验 +
//!   逐文件内容寻址（sha256 / byteLength）；
//! - `validate`：资源 payload 校验（schema 注册表 + 离线 + draft 2020-12 锁定）
//!   与 FR-12 作者友好错误信封。
//!
//! 契约纪律：JSON Schema 文本是跨语言契约的单一事实源（AGENTS.md 形态约束 2）；
//! M0 阶段 schema 以常量内嵌于本 crate（packages/schema 泳道落地后抽出为共享
//! 文件，契约交叉校验脚本消费同一份文本）。校验只操作 `serde_json` 值，不触达
//! 内核状态；对内核 crate 仅声明单向依赖边（模块边界文档），本 crate API 不
//! 反向暴露内核类型——资源 manifest 条目是纯 serde 结构，校验直接作用于
//! `serde_json::Value`。
//!
//! 校验配置纪律（native/TS 双端 parity，research §4）：
//! - draft 锁定 2020-12（`draft202012::options` 显式指定，不做自动探测；schema
//!   文本必须携带 draft 2020-12 的 `$schema` 元锚——构造期强制检查，测试另锚）；
//! - 禁 `$data`（本 crate 不提供；TS 侧 ajv 同禁）；
//! - format 断言双端显式关闭（`.should_validate_formats(false)`，format 仅注解）；
//! - 正则限于双端交集能力：本 crate 用线性时间引擎（`PatternOptions::regex()`，
//!   lookaround/反向引用等回溯型特性构造期即拒绝）；
//! - 外部引用离线化（`.offline()`）：注册表未收录的 `$ref` 构造期即失败，永不
//!   发起网络/文件取用（wasm32 面无 IO 依赖——`default-features = false`）。

pub mod manifest;
pub mod validate;
