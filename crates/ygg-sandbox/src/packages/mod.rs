//! packages 装载面：`packages/libs/<id>/` 是**第二个受控模块命名空间**。
//!
//! # 为什么需要这一层
//!
//! r1 的装载器只认一张模块表（`ModuleSet`），r2 的 `pack::assemble` 只把**本
//! Mod 的** `code.modules` 对成那张表。于是「可复用的能力包」无处落地：把包源码
//! 复制进每个 mod 目录会造成双源（改包不重 pack 即静默漂移），而让 `require`
//! 走文件系统会开出一个内容寻址之外的装载通道。本模块把包变成**同一张
//! `ModuleSet` 的第二段**——名字前缀 `pkg.`，`require` 的查表机制一个字不改。
//!
//! # 与「挂入 `package.path`」的关系（与需求字面的偏差，已登记）
//!
//! 沙箱里**没有** `package` 全局，而且被白名单测试钉死（`sandbox.rs` 的
//! `whitelisted_environment_exposes_only_the_allow_list`：`package == nil`）。
//! 所以「挂入 `package.path`」在本仓的两个载体上是两件不同的实事：
//!
//! | 载体 | 等价机制 | 为什么不能字面实现 |
//! | --- | --- | --- |
//! | 内核沙箱 | 模块表注入（[`crate::modules`] 的受控 `require`） | 造一个 `package` 表会把「解析顺序」交给 mod 可变状态 ⇒ 同一 seed 两次回放可能装到不同模块，破确定性判据 |
//! | 普通 Lua（`examples/selftest`、`ygg-lint` 静态面） | `package.searchers` 追加项 | `package.path` 只认文件路径，而包索引是 `(name → 包内相对路径)`，需要一次查表 |
//!
//! 两个载体读**同一份**包索引（[`PackageIndex`] 的 `name → relpath` 表），所以
//! 两侧不可能漂移。
//!
//! # 三层职责（本模块一条线走完）
//!
//! - **声明面**（[`PackagesSpec`] + [`PackageManifest`]）：`pack.json` 的
//!   `packages[]` 与各包 `pkg.json` 的解析与校验，全部是纯函数（无 IO）。
//! - **闭包与根解析**（[`resolve`]）：把声明展开成「哪些包、哪些文件」，包根
//!   由 [`resolve_root`] 决定（`$YGG_PACKAGES_ROOT` > `<mod>/../../packages/libs`）。
//! - **装配面**（[`index`]）：manifest 内容寻址 ↔ 声明闭包 ↔ 实际字节三方对账，
//!   产出模块表与 chunk 路径映射。
//!
//! # 目录形状（P1 严格面）
//!
//! ```text
//! packages/libs/<id>/          # <id> 必须匹配 ^[a-z][a-z0-9_-]*$
//! ├── pkg.json                # 包自描述（必须存在）
//! ├── init.lua                # 包根模块 → pkg.<id>
//! ├── <sub>.lua               # → pkg.<id>.<sub>
//! └── test/*.lua              # 包自测；**不进模块表、不进 manifest**
//! ```
//!
//! `id` 就是目录名（单源，不另声明）。id → 模块前缀时 `-` 折叠为 `_`：模块名
//! 段字符集只有 `[A-Za-z0-9_]`（[`crate::modules::normalize`]），`pkg.role-skill`
//! 非法而 `pkg.role_skill` 合法——折叠不是风格选择，是唯一可行映射。
//!
//! 只允许 `pkg.json` / `*.lua` / `test/**`；以 `.` 开头的条目静默跳过（`.DS_Store`
//! 之类，与 git 惯例一致）。其它任何文件 = 打包期可读拒绝——「无未声明文件」纪律。
//!
//! # 失败面
//!
//! [`PackageError`] 一律在**进入内核之前**失败（零副作用），文案风格对齐
//! [`crate::pack::PackError`]：英文小写、点名 JSON 位置、直接说该写什么。

mod assemble;
mod closure;
mod declare;
mod error;
#[cfg(test)]
mod tests;

pub use assemble::{index, union, PackageIndex};
pub use closure::{resolve, resolve_root, PackageFile, ResolvedPackage, ResolvedPackages};
pub use declare::{
    is_valid_package_id, module_prefix, PackageManifest, PackageRef, PackagesSpec, PACKAGE_ID_RULE,
    PACKAGE_PREFIX, PKG_FILE, PKG_SCHEMA_VERSION, PKG_TEST_DIR,
};
pub use error::PackageError;
