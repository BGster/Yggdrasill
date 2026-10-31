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
//! （状态哈希，D-继承-5 归位；探针在 tests/probe_determinism.rs）；
//! r1 填充 `modules`（受控 require 的模块装载面：点分名解析 + 装载栈 + 缓存，
//! 解析面下沉到 `ygg-modules` 后本 crate 只留 Lua 注入半段并全量再导出）；
//! r2 填充 `pack`（`pack.json` 的 `code` 段声明 + manifest `code` 数组的内容寻址，
//! 装载前收口成 `ModuleSet`）；r3 填充 `modload`（Mod 目录 → 内核的**唯一装载口**：
//! 走查与实跑共用同一份读盘/对账/构造语义，宿主侧 IO 只在这里）。

pub mod driver;
pub mod fuel;
pub mod hash;
pub mod host_api;
pub mod host_contract;
pub mod inject;
pub mod modload;
pub mod modules;
pub mod pack;
pub mod playtest;
pub mod resources;
pub mod sandbox;
pub mod stdlib;
