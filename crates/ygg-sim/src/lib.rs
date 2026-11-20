//! Yggdrasill Tier 1 原生模拟内核（确定性 tick / 列式 ECS / 快照 / 回放 / state hash）。
//!
//! 契约：`docs/design/kernel/sim-kernel.md`（本 crate 是它的**唯一**载体，逐节引用写在
//! 各模块头注里）。本文件只做一件事：把「为什么这个 crate 这样写」的四条不变量钉在
//! crate 级，使后续执行者不必重推。
//!
//! # 四条不变量（违反即破确定性，不是风格问题）
//!
//! 1. **模拟态零浮点**（I2）：数值一律 `i64` 定点（[`fixed`]），模拟态里不存在 `f32`/`f64`
//!    字段、不存在带小数的字面量、不存在 `as f64` 转换。理由不是「浮点慢」，而是浮点
//!    的**求和顺序**会改结果：a+b+c 与 a+(b+c) 在 x86 与 aarch64 上可以不同，
//!    而整数加法的结合律是数学事实。`tests/determinism_scan.rs` 的 R1 用正则钉死源码。
//! 2. **不读时钟、不读外部随机源**（I5）：`dt` 只来自 `step(dt)` 的参数并进输入日志；
//!    随机数只由 seed 流派生（[`rng`]），状态随快照往返。R2 静态扫描钉死
//!    `Instant`/`SystemTime`/`std::time`/`std::thread`/`std::process`。
//! 3. **迭代序来自有序容器**：`BTreeMap` 可以，`HashMap`/`HashSet`/`DefaultHasher` 不行
//!    （后者跨进程、跨运行都可能换序）。R3 静态扫描钉死。
//! 4. **一条等式统摄快照与哈希**：`state_hash == fnv1a64(snapshot_bytes)`，且
//!    `restore(snapshot).hash() == hash`。一个序列化器、一个哈希、一个规范序——不给
//!    自己留第二条路径（第二条路径的意思是：两份代码迟早给出两个答案）。
//!
//! # 为什么注释里可以出现 `f64`，代码里不行
//!
//! 静态扫描**先剥行注释与块注释**再匹配（与 assetgen 的「注释里也不写」相反）。差别是
//! 刻意的：本 crate 的可读性恰恰靠注释解释「为什么禁浮点」，严格版会把解释禁令的文档
//! 判成违规，反而鼓励删注释。剥注释后规则更准：**代码里一个 float token 都不能有，
//! 注释里必须解释为什么没有**。
//!
//! # wasm 可见性
//!
//! 本 crate 经 `ygg-wasm → ygg-sandbox → ygg-sim` 传递可见，故 `src/**` 里不得出现
//! `std::time` / `std::fs` / 线程 / `std::process`。基准与跨进程用例只在 `tests/**`
//! 用这些（tests 不进 wasm 树）。

#![forbid(unsafe_code)]

pub mod error;
pub mod fixed;

pub use error::SimError;
pub use fixed::Fixed;
