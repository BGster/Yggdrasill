//! ygg-host：`ygg/1` 线协议的信封层（字段契约 + 边界浮点拒收）。
//!
//! 契约：`artifacts/w2-m3-protocol-plan.md`（消息表与 §0 五条已定决策）。
//!
//! # 为什么这是一个独立 crate（members 的第四次受控例外）
//!
//! 它不能挂在任何既有 crate 下：`ygg-sim` 明令禁 serde（快照是自写的规范二进制编码器）
//! 而信封恰恰只有 serde；`ygg-sandbox` 拥有 mlua（信封必须能被宿主侧非 Rust 语言手写）；
//! `ygg-core` 是值 / 事件 / KV 的**语义**面（信封没有语义，只有字段）。
//!
//! 两个模块的依赖面不同：
//!
//! - [`proto`]：**零内核依赖**（只有 serde / serde_json / thiserror）。它是一份可独立
//!   阅读的跨语言契约，组件 id / 实体 id 是本地别名（见 [`proto::EntityId`]）。
//! - [`session`]：`ygg/1` 会话状态机（M3.2），**必须持有内核**——一个会话 = 一个世界
//!   实例（决策 5），于是它依赖 `ygg-sandbox`（[`Loaded`](ygg_sandbox::modload::Loaded)）
//!   与 `ygg-sim`（列 / 值类型）。绑定层（计划 §3 步 3）是它第一个外部消费方。
//! - `base64` / [`image`]：两条**共用设施**。镜像解析（包字节 → 装载参数）不能住在
//!   绑定层（绑定层只搬字节），于是它上提成内核面的第二条入口；base64 编解码就被
//!   快照帧与镜像资源两个消费方共用。
//!
//! # 一条铁律：协议层不持有世界（决策 4）
//!
//! [`proto`] 只做字段：任何决策（能不能 spawn、能不能 step、`pack_hash` 对不对）都在
//! `ygg-sim` / `ygg-sandbox` / [`session`] 里。所以 `proto` 里没有 `if`，只有类型、
//! `serde` 属性和一处边界检查（浮点拒收）。[`session`] 会「调用」内核，但仍不**判断**
//! 内核语义：批量写走 `ygg_sandbox::world_ops`（与 Lua `world.*` 同一份实现），时间
//! 推进走 `Loaded::step`，本层只切帧、搬运值与分类错误。
//!
//! # 从哪读起
//!
//! - [`proto`]：消息联合、支撑类型、[`proto::decode`] / [`proto::encode`] / 帧流打包
//!   （[`proto::encode_frames`]）、严格整数面。
//! - [`session`]：`Fresh → Ready → Closed` 状态机、帧切分（`step` 答 `events` + `delta`）、
//!   `submit` → `WorldOp` 批量写、`replay` 复算比对、文本入口
//!   （[`Session::handle_json`]）。
//! - [`image`]：包镜像（`modSource` / `manifest` / `schemas` / `resources`）→ 装载。
//! - `packages/schema/protocol/README.md`：字段表 + 三条不变式（面向非 Rust 宿主）。
//! - `packages/schema/protocol/ygg-1.schema.json`：draft 2020-12 schema（跨语言事实源）。

#![forbid(unsafe_code)]

mod base64;

pub mod image;
pub mod proto;
pub mod session;

pub use proto::{
    decode, encode, encode_frames, Changed, Component, ComponentId, ComponentKind, ComponentValues,
    EntityId, ErrorCode, Frame, Log, LogHeader, LogLevel, LogLine, Message, Op, ProtocolError,
    Snapshot, Stage, SystemInfo, Value, CODEC, PROTOCOL,
};
pub use session::{Session, State};
