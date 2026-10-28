//! Yggdrasill 规则内核数据面（纯逻辑 crate：零 IO / 零渲染 / 零平台依赖）。
//!
//! 依赖白名单（D-补充-1）：`serde`、`thiserror`、`serde_json`。
//! 确定性纪律（D-继承-5 / research §3）：确定性路径一律有序容器
//! （`BTreeMap` / `BTreeSet`），禁用随机化哈希的 std 默认无序容器（威胁 #3）。
//!
//! 模块接入计划（随 slice 逐个接入模块声明）：
//! - S2：`entity`（实体存储）、`snapshot`（规范快照序列化）
//! - S3：`event`（事件模型）、`bus`（handler trait，反转面）
//! - S4：`transition`（转换驱动）
//! - S5：`kv`（持久 KV）、`save`（存档格式）——本 slice

pub mod bus;
pub mod entity;
pub mod event;
pub mod kv;
pub mod save;
pub mod snapshot;
pub mod transition;
