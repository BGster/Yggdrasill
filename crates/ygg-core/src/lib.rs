//! Yggdrasill 规则内核数据面（纯逻辑 crate：零 IO / 零渲染 / 零平台依赖）。
//!
//! 依赖白名单（D-补充-1）：`serde`、`thiserror`、`serde_json`。
//! 确定性纪律（D-继承-5 / research §3）：确定性路径一律有序容器
//! （`BTreeMap` / `BTreeSet`），禁用随机化哈希的 std 默认无序容器（威胁 #3）。
//!
//! 模块清单（M1-D 起）：`value`（载荷标量）、`event`、`bus`、`transition`、
//! `kv`、`save`。原 S2 的 `entity`（行存实体表 + 属性索引 + 选择器 DSL）与
//! `snapshot`（行存规范快照）随「第二套实体模型」整体删除——世界状态由
//! `ygg-sim` 的列存承载，快照由 `ygg_sim::snapshot` 独占一份实现（M1-D）。

pub mod bus;
pub mod event;
pub mod kv;
pub mod save;
pub mod transition;
pub mod value;
