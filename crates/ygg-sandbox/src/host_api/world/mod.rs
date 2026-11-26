//! M1-C：Lua 世界面 `world.*`——组件声明、实体增删改查、系统声明与 `seal` 上浮。
//!
//! 与 `ygg.*` 并列注册（[`super::install`] 两张表同一次安装），但**只走列式内核面**：
//! 一切读写都落到 `ArchetypeStore` 的批式 API（`spawn_columns` / `insert_columns` /
//! `set_many` / `for_each_mut` / `collect_ids` / `value_of`）。不发明第二套存储，
//! 也不发明逐实体的存储路径——单实体的增删改查在本面里都是「一行批式调用」。
//!
//! ## 组件 kind → 内核列型
//!
//! **声明序即组件 id 序**（`world.component` 的第 n 次成功声明得到 id n-1，与
//! `Registry::register` 的「只增」语义逐字一致）。kind 只有四个：
//!
//! | kind | 内核列型 | Lua 值域 |
//! | --- | --- | --- |
//! | `i64` | `number`（`Column::I64`） | 整数（过 `YGG-SIM-FIXED` 定点闸门） |
//! | `u64` | `ref`（`Column::U64`） | `0 ..= i64::MAX`（Lua 5.4 整数是 `i64`，越界报错不回绕） |
//! | `u8` | `number`（`Column::I64`）+ 本面 `0..=255` 把关 | 整数 |
//! | `bytes` | `string`（`Column::Bytes`） | 字符串 |
//!
//! `u8` 落 `number` 列而不是内核的 `boolean`(`Column::U8`) 列：`RowSet` 只有
//! `fixed` / `u64v` / `bytes` 三个读取口和 `set_fixed` / `set_u64` 两个写回口，
//! 走 `boolean` 列会让该组件在查询面上**完全不可达**（内核缺口，已上报）。
//! 值域因此由本面把关：`0..=255`，越界即 `YGG-SIM-FIXED`。
//!
//! ## 决定性
//!
//! - **迭代序** = 内核 `for_each_mut` 的遍历序（桶下标升序 → 行号升序）= `collect_ids`
//!   的序。同种子、同操作序列的两次运行逐项相同（`tests/world_facade.rs` 的
//!   「两次运行序相同」用例就是这个契约的可执行形式）。
//! - **无浮点**：所有数值入口过同一道定点闸门，`math` 的结果（`0.5` 之类）过不了面，
//!   因此双精度不可能经本面进入世界值。
//! - **未挂载 / 重入都是可读错误**（码在首段）：不是 `nil`、不是 `false`、不是 panic。
//!   `nil` 与「世界是空的」不可分辨，静默降级会让 Mod 把「宿主没这个能力」读成
//!   「世界为空」并据此写下错误状态。
//!
//! ## 系统体与重入
//!
//! `world.system{...}` 的 `run` 收 `(dt, ctx)`：`dt` 是本 tick 步长（Q32.32 原始整数），
//! `ctx` 是与 `world.*` 同名同义的函数表，但走 `SystemCtx` 的能力面（`tick / dt /
//! each / each_mut / ids / rng / spawn / despawn / insert`）。系统体里再调 `world.*`
//! 一律 `YGG-WORLD-BUSY`：`step` 期间内核槽已被独占借用，**不是**可以绕过的限制。
//! 结构变更（`ctx.spawn` / `ctx.despawn` / `ctx.insert`）按内核语义延迟到 tick 边界。

mod columns;
mod ctx;
mod install;
mod rows;
mod systems;
mod values;

pub use columns::WorldState;
pub(super) use install::install_world;

/// 重入码（`world.*` 在内核槽已被独占时被调用；与 `YGG-SIM-UNMOUNTED` 同一处置）。
pub const WORLD_BUSY: &str = "YGG-WORLD-BUSY";

/// `world.log` 环形缓冲上限（超出丢最旧的一行：日志是诊断面，不是状态）。
pub const WORLD_LOG_CAP: usize = 256;
