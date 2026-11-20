//! Mod 目录 → 内核的**唯一装载口**（r3）：读盘、声明对账、构造驱动三件事收在一处。
//!
//! # 为什么必须有这一层
//!
//! 在这之前，「把 `<mod目录>` 装成内核」有三份独立实现：`driver` 侧的
//! `build_kernel`（组装序）、`playtest` 侧的 `read_mod_bundle`（读盘）、
//! `examples/egg.rs` 的 `ModBundle`（读盘 + 热载重读）。前两者本应完全一致，
//! 实际差了一处**静默**：playtest 读盘时不看 `pack.json` 的 `code` 段，于是
//! 多文件 Mod 在走查里被当成单文件跑——入口 `require("src.util")` 对空模块表
//! 直接得到可读拒绝，或（入口自己有兜底时）跑到一半才炸。这类偏差的症状是
//! 「playtest 全绿、实跑必挂」或反过来，而两边日志都不指向根因。
//!
//! 所以读盘 + 声明对账 + 构造驱动只有一条路径：**走查与实跑吃同一份装载语义**，
//! 差异只可能来自事件驱动策略（那是 playtest 的本职，不是装载面）。
//!
//! # 与「内核零 IO」纪律的关系
//!
//! 本模块**属于宿主侧**：只有它做文件系统读取。`driver` / `sandbox` / `modules`
//! 依旧零 IO（装载器拿到的只是 [`ModuleSet`] 字节表）——纪律没有放宽，只是把
//! 「宿主读盘」从三个 example/走查实现里收成一份。
//!
//! # 失败面
//!
//! 读盘与声明错误的错误类型是 `String`（人读文案，带目录与文件路径）——这一层
//! 的错误直接进 CLI 报告 / 走查报告，不需要机器分支；内核构造错误仍是
//! [`KernelError`]（有稳定码映射面），由调用方决定怎么呈现。

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::Value as Json;

use crate::driver::{KernelError, KernelFacade};
use crate::modules::{ModuleSet, ModuleTable};
use crate::pack::{self, CodeSpec};
use crate::packages::{self, PackagesSpec};

/// Mod 包内的声明文件名（`entries` / `schemaFiles` / `code` / `agent` / `inboxRoutes`）。
pub const PACK_FILE: &str = "pack.json";

/// Mod 包内的资源内容寻址清单（`entries` + `code` 的 sha256 / byteLength）。
pub const MANIFEST_FILE: &str = "manifest.json";

/// 单文件 Mod 的入口文件名（`code.entry` 缺省时的回落）。
pub const MOD_SOURCE_FILE: &str = "mod.lua";

/// 一个 Mod 目录装出来的全部装载参数（五件套 + 模块表 + chunk 覆盖表）。
///
/// 字段是「装载参数的完整集合」：入口源码、manifest 字节、schema 文本、资源
/// 字节、代码模块表、chunk 名覆盖表。持有它就能反复构造驱动（热载重读、读档
/// 重建），不必再碰文件系统——这也是宿主侧「签名变了才重读」的实现基础。
#[derive(Debug, Clone)]
pub struct ModBundle {
    /// 入口 Mod 源码（`code.entry` 或 [`MOD_SOURCE_FILE`]）。
    pub mod_source: String,
    /// `manifest.json` 原始字节（内容寻址校验由 `ResourceTable::build` 消费）。
    pub manifest_bytes: Vec<u8>,
    /// `($id, 文本)` 序列（schema 不进 manifest，装载方按 id 供给）。
    pub schemas: Vec<(String, String)>,
    /// `(包内相对路径, 字节)` 序列（顺序 = `pack.json` 的 `entries` 声明序）。
    pub resources: Vec<(String, Vec<u8>)>,
    /// 代码模块表（mod 自有 + 并集进来的包模块；空 = 单文件 Mod）。
    ///
    /// 包模块与 mod 模块进**同一张**表（只有名字前缀不同：`pkg.*` vs `src.*`），
    /// 所以 `require` 的查表机制一个字不改。
    pub modules: ModuleSet,
    /// chunk 名覆盖表（`模块名 → 真实相对路径`；D3）。
    ///
    /// 空的含义与 `modules` 为空不同：这里空 = 没有包模块（全部名字走
    /// `modules::path_of` 的默认双射，即现有行为逐字节不变）。
    pub chunk_paths: BTreeMap<String, String>,
}

impl ModBundle {
    /// 读盘装载一个 Mod 目录（`pack.json` 是声明入口，缺失即失败）。
    ///
    /// 顺序即语义：① `pack.json` 解析 → ② `code` 段校验（可选；存在即多文件
    /// Mod，入口名取 `code.entry`）→ ③ 入口源码 / manifest 读取 → ④ 模块表
    /// 三方对账（声明 ↔ manifest 登记 ↔ 实际字节，全部在装载前）→ ⑤ schema
    /// 文本按文件内 `$id` 注册 → ⑥ 资源字节按 `entries` 声明序读。
    pub fn load(dir: &Path) -> Result<Self, String> {
        if !dir.is_dir() {
            return Err(format!("mod 目录不存在或不可读：{}", dir.display()));
        }
        let pack_text = fs::read_to_string(dir.join(PACK_FILE))
            .map_err(|err| format!("读取 {}/{PACK_FILE} 失败：{err}", dir.display()))?;
        let pack: Json = serde_json::from_str(&pack_text)
            .map_err(|err| format!("{}/{PACK_FILE} 不是合法 JSON：{err}", dir.display()))?;
        let entries = pack["entries"]
            .as_array()
            .ok_or_else(|| format!("{}/{PACK_FILE} 缺少 entries 数组", dir.display()))?;
        let schema_files = pack["schemaFiles"]
            .as_array()
            .ok_or_else(|| format!("{}/{PACK_FILE} 缺少 schemaFiles 数组", dir.display()))?;

        // `code` 段（可选）：声明了就是多文件 Mod——入口名不一定叫 mod.lua，
        // 模块字节只从 manifest 校验过的模块表取（未登记的模块装载前就失败）。
        let code = CodeSpec::parse(&pack)
            .map_err(|err| format!("{}/{PACK_FILE} 的 code 段非法：{err}", dir.display()))?;
        // `packages` 段（可选）：声明了就是「本 Mod 依赖若干个能力包」。
        // 段存在即全量校验，拒绝发生在读取任何包字节之前（零副作用）。
        let packages_spec = PackagesSpec::parse(&pack)
            .map_err(|err| format!("{}/{PACK_FILE} 的 packages 段非法：{err}", dir.display()))?;
        let entry_file = code.as_ref().map_or(MOD_SOURCE_FILE, |spec| spec.entry());
        let mod_source = fs::read_to_string(dir.join(entry_file))
            .map_err(|err| format!("读取 {}/{entry_file} 失败：{err}", dir.display()))?;
        let manifest_bytes = fs::read(dir.join(MANIFEST_FILE))
            .map_err(|err| format!("读取 {}/{MANIFEST_FILE} 失败：{err}", dir.display()))?;
        // manifest 只解析**一次**：`code` 的模块对账与 `packages` 的文件对账
        // 读同一份登记（分头解析会留下两套失败文案与两次校验开销）。
        let manifest = if code.is_some() || packages_spec.is_some() {
            Some(
                ygg_schema::manifest::parse_manifest(&manifest_bytes)
                    .map_err(|err| format!("{}/{MANIFEST_FILE} 不合契约：{err}", dir.display()))?,
            )
        } else {
            None
        };
        let mod_modules = match (&code, &manifest) {
            (Some(spec), Some(manifest)) => load_code_modules(dir, spec, manifest)?,
            _ => ModuleSet::new(),
        };
        // ②′④′⑦⑧ packages 段：包根解析 → 闭包 → 三方对账 → 并集。
        // 全部在进入内核之前完成；失败文案统一前缀（与 code 段同形）。
        let (modules, chunk_paths) = match (&packages_spec, &manifest) {
            (Some(spec), Some(manifest)) => load_packages(dir, spec, manifest, &mod_modules)?,
            _ => (mod_modules, BTreeMap::new()),
        };

        // schema 文本按文件内 `$id` 注册（schema 不进 manifest，装载方按 id 供给）。
        let mut schemas = Vec::new();
        for file in schema_files {
            let rel = file.as_str().ok_or_else(|| {
                format!(
                    "{}/{PACK_FILE} 的 schemaFiles 必须是字符串数组",
                    dir.display()
                )
            })?;
            let text = fs::read_to_string(dir.join(rel))
                .map_err(|err| format!("读取 schema 文件 {rel} 失败：{err}"))?;
            let parsed: Json = serde_json::from_str(&text)
                .map_err(|err| format!("schema 文件 {rel} 不是合法 JSON：{err}"))?;
            let id = parsed["$id"]
                .as_str()
                .ok_or_else(|| format!("schema 文件 {rel} 缺少 $id"))?
                .to_owned();
            schemas.push((id, text));
        }

        let mut resources = Vec::new();
        for entry in entries {
            let rel = entry["path"].as_str().ok_or_else(|| {
                format!(
                    "{}/{PACK_FILE} 的 entries[].path 必须是字符串",
                    dir.display()
                )
            })?;
            let bytes =
                fs::read(dir.join(rel)).map_err(|err| format!("读取资源文件 {rel} 失败：{err}"))?;
            resources.push((rel.to_owned(), bytes));
        }

        Ok(Self {
            mod_source,
            manifest_bytes,
            schemas,
            resources,
            modules,
            chunk_paths,
        })
    }

    /// 按本包构造驱动（资源表 fail-fast 先于任何 Lua 状态）。
    ///
    /// 有模块表走多文件变体（入口顶层即可 `require`），否则走单文件变体——
    /// 两条路径的差别只在「模块表注不注入」，装载序其余部分完全相同。
    pub fn kernel(&self) -> Result<KernelFacade, KernelError> {
        if self.modules.is_empty() {
            KernelFacade::with_resources(
                &self.mod_source,
                &self.manifest_bytes,
                &self.schemas,
                &self.resources,
            )
        } else {
            KernelFacade::with_resources_and_module_table(
                &self.mod_source,
                self.module_table(),
                &self.manifest_bytes,
                &self.schemas,
                &self.resources,
            )
        }
    }

    /// 装载期代码面（模块表 + chunk 覆盖表；两者同进同出，见 [`ModuleTable`]）。
    fn module_table(&self) -> ModuleTable {
        ModuleTable::with_chunk_paths(self.modules.clone(), self.chunk_paths.clone())
    }

    /// 读档重建（热载 / 续局路径）：与 [`ModBundle::kernel`] 同一分派。
    ///
    /// 必须与 `kernel` 同分派：模块文件变了而入口没变时，热载签名会变并走到
    /// 这里；若这里丢掉模块表，跑起来的仍是旧代码或入口 `require` 直接失败。
    pub fn restore(&self, save_bytes: &[u8]) -> Result<KernelFacade, KernelError> {
        if self.modules.is_empty() {
            KernelFacade::restore_with_resources(
                save_bytes,
                &self.mod_source,
                &self.manifest_bytes,
                &self.schemas,
                &self.resources,
            )
        } else {
            KernelFacade::restore_with_resources_and_module_table(
                save_bytes,
                &self.mod_source,
                self.module_table(),
                &self.manifest_bytes,
                &self.schemas,
                &self.resources,
            )
        }
    }
}

/// `<mod目录>` → 已装载的内核（读盘 + 构造一步到位）。
///
/// 只需要「装上就能派发」的调用方（走查、探针脚本）走这条；需要反复重建的
/// 宿主（热载 / 读档）先 [`ModBundle::load`] 拿到参数再自行调
/// [`ModBundle::kernel`] / [`ModBundle::restore`]。
pub fn load_mod(dir: &Path) -> Result<KernelFacade, String> {
    ModBundle::load(dir)?
        .kernel()
        .map_err(|err| err.to_string())
}

/// 多文件 Mod 的模块表：声明 → manifest 登记 → 实际字节三者对账。
///
/// 装载器不对文件系统做任何事，所以「读盘」与「对账」都必须在进入内核之前
/// 完成——对账失败即装载失败（部分登记的模块表只会换来个无关的 `NotFound`）。
fn load_code_modules(
    dir: &Path,
    spec: &CodeSpec,
    manifest: &ygg_schema::manifest::Manifest,
) -> Result<ModuleSet, String> {
    let module_bytes = pack::collect_bytes(spec, |rel| fs::read(dir.join(rel)))
        .map_err(|err| format!("{}/code 段读取失败：{err}", dir.display()))?;
    pack::assemble(spec, manifest.code(), &module_bytes)
        .map_err(|err| format!("{}/code 段校验失败：{err}", dir.display()))
}

/// 包面：包根解析 → 闭包 → manifest 三方对账 → 与 mod 自有模块表并集。
///
/// 四步全部在进入内核之前，任一步失败即装载失败并统一前缀（与 `code` 段同形，
/// 读者不用猜是哪个段坏了）。并集预算与名字冲突在 [`packages::union`] 里收口
/// ——按并集统算，否则两个各自合法的半边合起来超预算。
fn load_packages(
    dir: &Path,
    spec: &PackagesSpec,
    manifest: &ygg_schema::manifest::Manifest,
    mod_modules: &ModuleSet,
) -> Result<(ModuleSet, BTreeMap<String, String>), String> {
    let wrap =
        |err: packages::PackageError| format!("{}/packages 段校验失败：{err}", dir.display());
    // 包根由本函数自行解析（`$YGG_PACKAGES_ROOT` > `<mod>/../../packages/libs`）
    // ——CLI 一行不改就能用，工具面需要显式指定时用 `--packages` 自己解。
    let root = packages::resolve_root(dir, None).map_err(wrap)?;
    let resolved = packages::resolve(&root, spec).map_err(wrap)?;
    let index = packages::index(&root, &resolved, manifest.packages()).map_err(wrap)?;
    let table = packages::union(mod_modules, index).map_err(wrap)?;
    Ok(table.into_parts())
}
