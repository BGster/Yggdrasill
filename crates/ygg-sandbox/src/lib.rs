//! ygg-sandbox：mlua Lua 5.4 确定性沙箱
//! （host API 白名单 + ygg stdlib + 世界面 `world.*`；原燃料计量面已于 S34 退役，
//! 事件 / 信封 / 派发面已于 M1-E 退役——内核只由宿主经 [`modload::Loaded::step`]
//! 推进时间，存档面只有世界帧字节）。
//!
//! S1 空壳占位（D-补充-2）；S7–S12 逐 slice 扩展模块声明。
//! S7 填充 `sandbox`（沙箱构造 + 负例）；S8 填充 `inject`（确定性注入）；
//! S9 填充 `host_api`（`ygg.kv` / `ygg.codec` / `world.*` 的安装面）；
//! S10 填充 `stdlib`（版本面）；S11 填充 `resources`
//! 【S11 改名说明：原拟名 `data`，避让 Lua 侧 `ygg.data` API 名——模块名
//! 采用资源面本名，Rust 侧不再出现与 API 同名的模块】；S12 填充 `hash`
//! （状态哈希，D-继承-5 归位；探针在 tests/probe_determinism.rs）；
//! r1 填充 `modules`（受控 require 的模块装载面：点分名解析 + 装载栈 + 缓存，
//! 解析面下沉到 `ygg-modules` 后本 crate 只留 Lua 注入半段并全量再导出）；
//! r2 填充 `pack`（`pack.json` 的 `code` 段声明 + manifest `code` 数组的内容寻址，
//! 装载前收口成 `ModuleSet`）；r3 填充 `modload`（Mod 目录 → 内核的**唯一装载口**：
//! 宿主侧 IO 只在这里，调用方拿到的是已对账的内核）。

pub mod codec_face;
pub mod hash;
pub mod host_api;
pub mod inject;
pub mod modload;
pub mod modules;
pub mod pack;
pub mod packages;
pub mod resources;
pub mod sandbox;
pub mod stdlib;
pub mod world_ops;
