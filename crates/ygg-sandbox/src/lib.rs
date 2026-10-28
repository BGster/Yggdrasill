//! ygg-sandbox：mlua Lua 5.4 确定性沙箱
//! （燃料计量、host API 白名单、ygg stdlib 嵌入、KernelFacade JSON 信封 API）。
//!
//! S1 空壳占位（D-补充-2）；S7–S12 逐 slice 扩展模块声明。
//! S7 填充 `sandbox`（沙箱构造 + 负例）；S8 填充 `fuel`（正式燃料）与
//! `inject`（确定性注入）；S9 填充 `host_api`（收集-回放桥 + 分相）；
//! S10 填充 `driver`（KernelFacade：装载序 + 事件转换 + 存读档信封面）与
//! `stdlib`（include_dir 嵌入 + 语义装载序）；S11 填充 `resources`
//! 【S11 改名说明：原拟名 `data`，避让 Lua 侧 `ygg.data` API 名——模块名
//! 采用资源面本名，Rust 侧不再出现与 API 同名的模块】；S12 填充 `hash`
//! （状态哈希，D-继承-5 归位；探针在 tests/probe_determinism.rs）。

pub mod driver;
pub mod fuel;
pub mod hash;
pub mod host_api;
pub mod inject;
pub mod playtest;
pub mod resources;
pub mod sandbox;
pub mod stdlib;
