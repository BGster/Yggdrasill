//! ygg-host：`ygg/1` 线协议的信封层（字段契约 + 边界浮点拒收）。
//!
//! 契约：`artifacts/w2-m3-protocol-plan.md`（消息表与 §0 五条已定决策）。
//!
//! # 为什么这是一个独立 crate（members 的第四次受控例外）
//!
//! 它不能挂在任何既有 crate 下：`ygg-sim` 明令禁 serde（快照是自写的规范二进制编码器）
//! 而信封恰恰只有 serde；`ygg-sandbox` 拥有 mlua（信封必须能被宿主侧非 Rust 语言手写）；
//! `ygg-core` 是值 / 事件 / KV 的**语义**面（信封没有语义，只有字段）。本 crate 因此
//! **不依赖任何内部 crate**，也不被任何 crate 依赖——绑定层（计划 §3 步 3）才是第一个
//! 消费方。
//!
//! # 一条铁律：协议层不持有世界（决策 4）
//!
//! 本 crate 只做「信封 ↔ 内核调用」的**翻译**外壳：任何决策（能不能 spawn、能不能
//! step、`pack_hash` 对不对）都在 `ygg-sim` / `ygg-sandbox` / 会话层里。所以这里
//! 没有 `if`，只有类型、`serde` 属性和一处边界检查（浮点拒收，见 [`proto`]）。
//!
//! # 从哪读起
//!
//! - [`proto`]：消息联合、支撑类型、[`proto::decode`] / [`proto::encode`]、严格整数面。
//! - `packages/schema/protocol/README.md`：字段表 + 三条不变式（面向非 Rust 宿主）。
//! - `packages/schema/protocol/ygg-1.schema.json`：draft 2020-12 schema（跨语言事实源）。

#![forbid(unsafe_code)]

pub mod proto;

pub use proto::{
    decode, encode, Changed, Component, ComponentId, ComponentKind, ComponentValues, EntityId,
    ErrorCode, Frame, Log, LogHeader, LogLevel, LogLine, Message, Op, ProtocolError, Snapshot,
    Stage, SystemInfo, Value, CODEC, PROTOCOL,
};
