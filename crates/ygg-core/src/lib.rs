//! Yggdrasill 规则内核数据面（纯逻辑 crate：零 IO / 零渲染 / 零平台依赖）。
//!
//! 依赖白名单（D-补充-1）：`serde`、`thiserror`、`serde_json`。
//! 确定性纪律（D-继承-5 / research §3）：确定性路径一律有序容器
//! （`BTreeMap` / `BTreeSet`），禁用随机化哈希的 std 默认无序容器（威胁 #3）。
//!
//! 模块清单：`value`（载荷标量）、`kv`（持久 KV）。
//! - S2 的 `entity`（行存实体表）与 `snapshot`（行存规范快照）随「第二套实体
//!   模型」整体删除——世界状态由 `ygg-sim` 的列存承载（M1-D）。
//! - `event` / `bus` / `transition` / `save` 随**事件-信封层**整体删除（M1-E，
//!   D2）：内核不再有 handler 注册表、事件 FIFO 与转换驱动；时间由宿主经
//!   `Sandbox::step` 推进，存档形态收敛为世界帧字节（见 `crate::modload::Loaded`）。
//!   规则层（emit / on / 事务 undo 日志）按 M2 计划在 `world.*` 面上重建。

pub mod kv;
pub mod value;
