//! Mod 目录 → 已装载内核的**唯一装载口**（r3；M1-E 收敛为 [`Loaded`]）：读盘、
//! 声明对账、装载序三件事收在一处。
//!
//! # 为什么必须有这一层
//!
//! 「把 `<mod目录>` 装成内核」曾经散在三处：读盘、声明对账、组装内核各写一遍，
//! 于是同一条语义在三处各自漂移（典型症状：读盘不看 `pack.json` 的 `code` 段，
//! 多文件 Mod 被当单文件装，入口 `require("src.util")` 对空模块表得到可读拒绝，
//! 或跑到一半才炸）。现在只有这一条路径：读盘 + 声明对账 + 装载序收在一处，
//! 谁装 Mod 都吃同一份装载语义。
//!
//! # 与「内核零 IO」纪律的关系
//!
//! 本模块**属于宿主侧**：只有它做文件系统读取。`sandbox` / `modules` 依旧零 IO
//! （装载器拿到的只是 [`ModuleSet`] 字节表）——纪律没有放宽，只是把「宿主读盘」
//! 只放在这一处。
//!
//! # 装载序（唯一一份）
//!
//! ① 资源表 fail-fast（预算 / 寻址 / 校验，任何 Lua 状态之前）→ ② `Sandbox::new`
//! （stdlib 裁剪 + 内存上限 + 白名单环境）→ ③ `inject::install`（math 精确集 +
//! 内核种子 random）→ ④ 世界内核挂上（`Sim::new(KERNEL_RNG_SEED)`，与
//!   `math.random` 同源）→ ⑤ `host_api::install`（`ygg.kv` / `ygg.codec` /
//!   `world.*`）→ ⑥ `resources::install`（`ygg.data`）→ ⑦ `stdlib::install`
//!   （版本面）→ ⑧ 模块表注入（入口 chunk 之前，入口顶层即可 `require`）→
//!   ⑨ 入口 chunk（失败 = [`LoadError::ModChunk`]）。
//!
//! 装载完的内核**不再有派发面**：时间只由宿主经 [`Loaded::step`] 推进，
//! 存档面只有世界帧字节（[`Loaded::snapshot`] / [`Loaded::restore`]）。
//!
//! # 失败面
//!
//! 读盘与声明错误的错误类型是 [`LoadError::Bundle`]（人读文案，带目录与文件
//! 路径）——这一层的错误直接进调用方的报告，不需要机器分支；内核构造错误按
//! 来源分变体（资源包 / manifest / Lua 状态 / 入口 chunk / 步进），调用方按变体
//! 决定怎么呈现，不解析 message。

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::rc::Rc;

use serde_json::Value as Json;
use thiserror::Error as ThisError;

use ygg_core::kv::KvStore;
use ygg_schema::manifest::{parse_manifest, Manifest};
use ygg_schema::validate::SchemaError;
use ygg_sim::Sim;

use crate::host_api::{self, HostApi, HostHandles};
use crate::inject;
use crate::modules::{ModuleSet, ModuleTable};
use crate::pack::{self, CodeSpec};
use crate::packages::{self, PackagesSpec};
use crate::resources::{self, ResourceError, ResourceTable};
use crate::sandbox::{Sandbox, StepError, StepReport};
use crate::stdlib;
use crate::world_ops::WorldOp;

/// Mod 包内的声明文件名（`entries` / `schemaFiles` / `code` / `packages`）。
pub const PACK_FILE: &str = "pack.json";

/// 无资源路径的最小 manifest（`schemaVersion` + 空 `entries` 是仅有的必填字段）。
///
/// 内存装载参数（wasm 壳、单文件夹具）总得给 manifest：资源表构建是无条件
/// 的一步，没有 manifest 就没有「装载」这回事。这一份是它的最小合法值。
pub const MINIMAL_MANIFEST: &str = r#"{"schemaVersion":1,"entries":[]}"#;

/// Mod 包内的资源内容寻址清单（`entries` + `code` 的 sha256 / byteLength）。
pub const MANIFEST_FILE: &str = "manifest.json";

/// 单文件 Mod 的入口文件名（`code.entry` 缺省时的回落）。
pub const MOD_SOURCE_FILE: &str = "mod.lua";

/// 装载失败（读盘 / 对账 / 构造 / 步进 / 快照，单点出口）。
#[derive(Debug, ThisError)]
pub enum LoadError {
    /// 读盘与声明对账失败（`pack.json` / `manifest.json` / 模块三方对账）。
    #[error("{0}")]
    Bundle(String),
    /// `manifest.json` 不合契约（内容寻址清单本身无法解析）。
    #[error(transparent)]
    Manifest(#[from] SchemaError),
    /// Lua 状态构造 / 注入 / 资源面安装失败（结构面错误，非作者可修复）。
    #[error(transparent)]
    Sandbox(#[from] mlua::Error),
    /// 入口 chunk 拒绝（含入口顶层的 `require` 失败；文本带 `mod.lua:<line>`）。
    #[error("mod chunk rejected: {0}")]
    ModChunk(String),
    /// 资源包拒绝（预算 / 寻址 / payload schema）。
    #[error(transparent)]
    Resources(#[from] ResourceError),
    /// 步进 / 快照 / 恢复失败（世界内核错误）。
    #[error(transparent)]
    Step(#[from] StepError),
}

/// 一个 Mod 目录装出来的全部装载参数（五件套 + 模块表 + chunk 覆盖表）。
///
/// 字段是「装载参数的完整集合」：入口源码、manifest 字节、schema 文本、资源
/// 字节、代码模块表、chunk 名覆盖表。持有它就能反复构造内核（热载重读、读档
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
    /// 无资源路径的内存装载参数（wasm 壳与单文件夹具的最小形态）。
    ///
    /// 不读盘、不携带资源：manifest 取 [`MINIMAL_MANIFEST`]，模块表为空
    /// （入口顶层的 `require` 得到可读拒绍——要模块就自己填 `modules`）。
    pub fn in_memory(mod_source: impl Into<String>) -> Self {
        Self {
            mod_source: mod_source.into(),
            manifest_bytes: MINIMAL_MANIFEST.as_bytes().to_vec(),
            schemas: Vec::new(),
            resources: Vec::new(),
            modules: ModuleSet::new(),
            chunk_paths: BTreeMap::new(),
        }
    }

    /// 读盘装载一个 Mod 目录（`pack.json` 是声明入口，缺失即失败）。
    ///
    /// 顺序即语义：① `pack.json` 解析 → ② `code` 段校验（可选；存在即多文件
    /// Mod，入口名取 `code.entry`）→ ③ 入口源码 / manifest 读取 → ④ 模块表
    /// 三方对账（声明 ↔ manifest 登记 ↔ 实际字节，全部在装载前）→ ⑤ schema
    /// 文本按文件内 `$id` 注册 → ⑥ 资源字节按 `entries` 声明序读。
    pub fn load(dir: &Path) -> Result<Self, String> {
        Self::load_with_packages_root(dir, None)
    }

    /// [`load`](Self::load) 的显式包根变体（CLI `--packages` 装载路径）。
    ///
    /// `None` = 沿用 [`packages::resolve_root`] 既有优先级（explicit >
    /// 环境变量 `$YGG_PACKAGES_ROOT` > `<mod>/../../packages/libs` 布局推导）；
    /// `Some(root)` = 显式根直接生效。既有调用点（tests / 工具面）继续走
    /// [`load`]，签名零改动。
    pub fn load_with_packages_root(
        dir: &Path,
        packages_root: Option<&Path>,
    ) -> Result<Self, String> {
        if !dir.is_dir() {
            return Err(format!("mod 目录不存在或不可读：{}", dir.display()));
        }
        let pack_text = fs::read_to_string(dir.join(PACK_FILE))
            .map_err(|err| format!("读取 {}/{PACK_FILE} 失败：{err}", dir.display()))?;
        let pack: Json = serde_json::from_str(&pack_text)
            .map_err(|err| format!("{}/{PACK_FILE} 不是合法 JSON：{err}", dir.display()))?;
        // 数据条目段：v2 叫 `data`，v1 叫 `entries`（同一份声明，只是名字换了）。
        // 两段同时出现 = 声明面年代不明，报错而不是猜。
        let entries = match (pack.get("data"), pack.get("entries")) {
            (Some(_), Some(_)) => {
                return Err(format!(
                    "{}/{PACK_FILE} 同时声明了 data 与 entries（二选一）",
                    dir.display()
                ))
            }
            (Some(data), None) => data,
            (None, Some(legacy)) => legacy,
            (None, None) => {
                return Err(format!(
                    "{}/{PACK_FILE} 缺少 data 数组（v1 叫 entries）",
                    dir.display()
                ))
            }
        };
        let entries = entries
            .as_array()
            .ok_or_else(|| format!("{}/{PACK_FILE} 的 data 必须是数组", dir.display()))?;
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
                parse_manifest(&manifest_bytes)
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
            (Some(spec), Some(manifest)) => {
                load_packages(dir, spec, manifest, &mod_modules, packages_root)?
            }
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

    /// 装载期代码面（模块表 + chunk 覆盖表；两者同进同出，见 [`ModuleTable`]）。
    fn module_table(&self) -> ModuleTable {
        ModuleTable::with_chunk_paths(self.modules.clone(), self.chunk_paths.clone())
    }
}

/// 已装载的内核：世界面 + 资源面 + KV，**没有派发面**。
///
/// 宿主拿到的就是这个类型：推进时间（[`step`](Self::step)）、取世界帧
/// （[`snapshot`](Self::snapshot)）、按帧重建（[`restore`](Self::restore)）、
/// 读诊断面（[`take_world_log`](Self::take_world_log) / [`mod_source`](Self::mod_source)）。
pub struct Loaded {
    /// Mod 声明（`manifest.json` 的解析形态；内容寻址的「声明面」）。
    pub manifest: Manifest,
    /// Lua 状态持有者：字段存活期 = VM 存活期，同时是时间的唯一驱动面
    /// （`step` / `snapshot` / `restore` 都挂在这里，Lua 侧没有推进时钟的入口）。
    pub sandbox: Sandbox,
    /// 世界内核槽（与 `sandbox` / `world.*` 共享**同一个** `Rc`——见下）。
    ///
    /// 为什么是句柄而不是 `Sim` 值：`world.*` 的闭包装在 Lua 状态里，它们必须
    /// 与宿主操作**同一个**世界，而这个槽的所有权在 `Sandbox` 上（自引用结构
    /// 无法用值字段表达，也不许 `unsafe`）。因此这里给的是同一个 `Rc`——写穿过
    /// 它就是写穿到内核，与 `sandbox.sim_handle()` 逐字节同一。
    ///
    /// **私有**：`Rc` 交到外部就能被 `*slot.borrow_mut() = 另一个 Sim`，于是
    /// 「`Loaded` 是时间的唯一驱动面」立刻失效（宿主推进 A 世界、`world.*` 看见 B 世界）。
    /// 外部只需读（hash / 快照 / 诊断），走 [`Loaded::sim`] 借一个 `Ref`。
    sim: Rc<RefCell<Option<Box<Sim>>>>,
    /// host 控制柄（KV / world 日志 / 内核句柄的宿主面）。
    host: HostApi,
    /// 只读资源表（数据面非状态：不入世界帧 / 状态哈希）。
    resources: Rc<ResourceTable>,
    /// 装载的 Mod 源码（诊断只读面）。
    mod_source: String,
    /// 装载参数的内存形态：复算 / 重建直接拿它当输入，**不再读盘**。
    bundle: ModBundle,
    /// `manifest.json` 字节的 SHA-256（hex）：协议 `hello.pack_hash` 的对账对象。
    ///
    /// 它是**声明面**的指纹，不是世界帧：同一份 manifest 逐字节同值，脚本改一行
    /// 不动它。宿主能比对的是「我载的是不是你以为的那份声明」；脚本身份由装载
    /// 参数本身（源码 + 模块表 + 资源）承载。
    manifest_hash: String,
}

impl Loaded {
    /// 读盘装载 `<mod目录>`（`pack.json` 声明 + manifest 对账 + 全序装载）。
    pub fn from_dir(dir: &Path) -> Result<Self, LoadError> {
        Self::from_dir_with_packages(dir, None)
    }

    /// [`from_dir`](Self::from_dir) 的显式包根变体（CLI `--packages` 路径）。
    pub fn from_dir_with_packages(
        dir: &Path,
        packages_root: Option<&Path>,
    ) -> Result<Self, LoadError> {
        let bundle =
            ModBundle::load_with_packages_root(dir, packages_root).map_err(LoadError::Bundle)?;
        Self::from_bundle(&bundle)
    }

    /// 已读盘的装载参数 → 内核（本模块唯一的组装点；`from_dir*` 与内存夹具共用）。
    ///
    /// 资源表构建**先于任何 Lua 状态**（fail-fast：预算 / 寻址 / 校验失败零 Lua
    /// 侧副作用）；空模块表跳过模块注入（`require` 面对任何名字可读拒绝）。
    pub fn from_bundle(bundle: &ModBundle) -> Result<Self, LoadError> {
        Self::build(bundle, inject::KERNEL_RNG_SEED)
    }

    /// [`from_bundle`](Self::from_bundle) 的显式种子变体：**同一份装载参数、另起一条
    /// 内核谱系**。
    ///
    /// 真实用途只有一个：复算（`replay`）必须在一个「干净但同源」的内核上重跑历史，
    /// 种子来自日志头而不是内核常量；顺带让测试能在同一进程里并排两条谱系。
    fn build(bundle: &ModBundle, seed: u64) -> Result<Self, LoadError> {
        // ① 资源表 fail-fast（manifest 解析也在这里，失败先于 VM）。
        let table =
            ResourceTable::build(&bundle.manifest_bytes, &bundle.schemas, &bundle.resources)?;
        let manifest = parse_manifest(&bundle.manifest_bytes)?;
        // ② 沙箱：stdlib 裁剪 + 内存上限 + 白名单环境。
        let mut sandbox = Sandbox::new()?;
        let lua = sandbox.lua().clone();
        // ③ 世界内核挂上：任何内核实例出生起就有世界（`world.*` 无静默降级面），
        //    种子取内核常量同一谱系。
        sandbox.set_sim(Some(Box::new(Sim::new(seed))));
        let sim = sandbox.sim_handle();
        // ④ 确定性注入：math 精确集 + **世界流** random（时间不在这里）。
        //    次序不弹：注入在 set_sim 之后，闭包拿到的槽才是有世界的那一个；
        //    反过来也成立（槽是 `Rc`，后装也看得见），但先挂后注更好读。
        inject::install(&lua, Rc::clone(&sim))?;
        // ⑤ host API：ygg.kv / ygg.codec / world.*（与 VM 共享 KV 句柄）。
        let host = host_api::install(
            &lua,
            HostHandles::new(Rc::new(RefCell::new(KvStore::new()))),
            Rc::clone(&sim),
        )?;
        // ⑥ 资源：ygg.data 只读数据面（host 之后、入口 chunk 之前）。
        resources::install(&lua, &table)?;
        // ⑦ stdlib：版本面（Mod 顶层即可读）。
        stdlib::install(&lua)?;
        // ⑧ 模块表注入：入口 chunk 之前（入口顶层即可 require）；chunk 覆盖表
        //    与模块表同一时机（首次 require 的 traceback 就要是真路径）。
        if !bundle.modules.is_empty() {
            let (set, chunk_paths) = bundle.module_table().into_parts();
            sandbox.modules().set_modules(set);
            sandbox.modules().set_chunk_paths(chunk_paths);
        }
        // ⑨ 入口 chunk：`@mod.lua` 命名让 traceback 携带 mod 侧位置。
        //    预算先重置：装载面是**第一个**宿主入口，之前 build 里装 hook 之后
        //    跑的库代码已经花掉一些余额，入口从满额起算才可复现。
        sandbox.reset_budget();
        lua.load(&bundle.mod_source)
            .set_name("@mod.lua")
            .exec()
            .map_err(|err| LoadError::ModChunk(err.to_string()))?;
        Ok(Self {
            manifest,
            sandbox,
            sim,
            host,
            resources: Rc::new(table),
            mod_source: bundle.mod_source.clone(),
            bundle: bundle.clone(),
            manifest_hash: crate::hash::hex_sha256(&bundle.manifest_bytes),
        })
    }

    /// 世界内核槽的只读视图（诊断与断言用：`sim().as_ref()` 给 `&Sim`）。
    ///
    /// 给 `Ref` 而不给 `Rc`，是为了让「换掉内核」这件事只能发生在 `Loaded` 内部
    /// （见字段 `sim` 的注释）；需要写世界的宿主走 [`step`](Self::step) /
    /// [`restore`](Self::restore)，那两条是时间所有权的一部分。
    pub fn sim(&self) -> std::cell::Ref<'_, Option<Box<Sim>>> {
        self.sim.borrow()
    }

    /// **宿主推进时间**：推进 `ticks` 个 tick，每 tick `dt` 整数秒。
    ///
    /// 逐 T 委派 [`Sandbox::step`]（单次上限与错误码同源）；脚本决定一个 tick
    /// 里算什么，宿主决定走多少 tick——这就是可中断性与时间所有权的分界。
    pub fn step(&mut self, ticks: u32, dt: u32) -> Result<StepReport, LoadError> {
        self.sandbox.step(ticks, dt).map_err(LoadError::from)
    }

    /// 世界帧规范字节（同一状态 ⇒ 同一字节；哈希与存档共用这一个序列化面）。
    ///
    /// 帧里只有世界面（tick / 实体 / 组件 / rng 流）；**KV 不在帧内**——它不是
    /// 世界状态，读回走 [`kv`](Self::kv) 的宿主面。引擎版本也不在帧内
    /// （golden-vector 基线跨引擎补丁版本稳定）。
    pub fn snapshot(&self) -> Result<Vec<u8>, LoadError> {
        self.sandbox.snapshot().map_err(LoadError::from)
    }

    /// 按世界帧重建内核：`from_dir(dir)` 全序装载（脚本即世界定义来源）后整体
    /// 覆写世界面。坏帧（长度 / magic / 尾 digest 不符）先校验后改，不产生半改
    /// 的世界；KV 不随帧恢复（重建后的 KV 是「装载期写下的值」）。
    pub fn restore(dir: &Path, bytes: &[u8]) -> Result<Self, LoadError> {
        let bundle = ModBundle::load(dir).map_err(LoadError::Bundle)?;
        Self::restore_from_bundle(&bundle, bytes)
    }

    /// [`restore`](Self::restore) 的内存侧（已有装载参数）：wasm 壳与热载宿主走
    /// 这条——帧只含世界面，脚本依旧是世界定义来源。
    pub fn restore_from_bundle(bundle: &ModBundle, bytes: &[u8]) -> Result<Self, LoadError> {
        let mut loaded = Self::from_bundle(bundle)?;
        loaded.sandbox.restore(bytes)?;
        Ok(loaded)
    }

    /// 当前世界时钟（内核 tick）。
    pub fn tick(&self) -> Result<u64, StepError> {
        self.sandbox.tick()
    }

    /// 当前状态哈希（世界帧的 FNV-1a 64 摘要；与快照字节同源）。
    pub fn hash(&self) -> Result<u64, StepError> {
        self.sandbox.hash()
    }

    /// 装载的 Mod 源码（诊断只读面）。
    pub fn mod_source(&self) -> &str {
        &self.mod_source
    }

    /// `manifest.json` 字节的 SHA-256（hex）：`hello.pack_hash` 的对账对象
    /// （见字段 [`manifest_hash`](Self) 的口径说明）。
    pub fn manifest_hash(&self) -> &str {
        &self.manifest_hash
    }

    /// 装载参数（内存形态）：复算 / 重建的输入，不必再碰文件系统。
    pub fn bundle(&self) -> &ModBundle {
        &self.bundle
    }

    /// 世界内核是否已 `seal`。
    ///
    /// 内核自己不在 `step` 里拦「未 seal」（`seal` 收的是系统图，不是时间门），
    /// 所以这是**宿主面的判定输入**：未 seal 的世界不得推进时间（协议 `ready.sealed`）。
    pub fn sealed(&self) -> bool {
        self.sim
            .borrow()
            .as_ref()
            .is_some_and(|sim| sim.is_sealed())
    }

    /// 同装载参数、另起一条内核谱系（复算用；见 [`build`](Self::build)）。
    pub fn fork_with_seed(&self, seed: u64) -> Result<Self, LoadError> {
        Self::build(&self.bundle, seed)
    }

    /// 上一步 / 上一次批量写动过的单元格（**已排序去重**）：`(实体 id, 组件 id)`。
    pub fn changed(&self) -> Vec<(u64, u32)> {
        self.sim
            .borrow()
            .as_ref()
            .map_or_else(Vec::new, |sim| sim.changed().to_vec())
    }

    /// **宿主批量写面**：一批 [`WorldOp`] 走内核的「先全验、后落笔」实现
    /// （与 Lua `world.*` 同一份代码，见 [`crate::world_ops`]）。
    pub fn apply(&mut self, ops: &[WorldOp]) -> Result<usize, StepError> {
        self.sandbox.apply(ops)
    }

    /// 只读资源表（资源不可变，无写面外溢）。
    pub fn resources(&self) -> &ResourceTable {
        &self.resources
    }

    /// 持久 KV 句柄（宿主读回脚本写入的 KV）。
    pub fn kv(&self) -> Rc<RefCell<KvStore>> {
        self.host.kv()
    }

    /// 取走 `world.log` 累积的日志行（环形缓冲，满则丢最旧）。
    pub fn take_world_log(&self) -> Vec<String> {
        self.host.take_world_log()
    }

    /// 换入另一个世界内核实例（宿主面：载入特定存档、或换测试种子）。
    ///
    /// 写穿的是与 `sandbox` / `world.*` 共享的同一个槽。
    pub fn set_sim(&mut self, sim: Option<Box<Sim>>) {
        self.host.set_sim(sim);
    }
}

/// `<mod目录>` → 已装载内核（读盘 + 组装一步到位）。
///
/// 只需要「装上就能步进」的调用方（走查、探针脚本、wasm `open`）走这条；
/// 需要反复重建的宿主（热载 / 读档）先 [`ModBundle::load`] 拿到参数再
/// [`Loaded::from_bundle`]。
pub fn load_mod(dir: &Path) -> Result<Loaded, LoadError> {
    Loaded::from_dir(dir)
}

/// 多文件 Mod 的模块表：声明 → manifest 登记 → 实际字节三者对账。
///
/// 装载器不对文件系统做任何事，所以「读盘」与「对账」都必须在进入内核之前
/// 完成——对账失败即装载失败（部分登记的模块表只会换来个无关的 `NotFound`）。
fn load_code_modules(
    dir: &Path,
    spec: &CodeSpec,
    manifest: &Manifest,
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
    manifest: &Manifest,
    mod_modules: &ModuleSet,
    explicit: Option<&Path>,
) -> Result<(ModuleSet, BTreeMap<String, String>), String> {
    let wrap =
        |err: packages::PackageError| format!("{}/packages 段校验失败：{err}", dir.display());
    // 包根解析（`explicit` > `$YGG_PACKAGES_ROOT` > `<mod>/../../packages/libs`，
    // 见 [`packages::resolve_root`]）——CLI 缺省一行不改就能用，`--packages` 时
    // 显式根直接生效。
    let root = packages::resolve_root(dir, explicit).map_err(wrap)?;
    let resolved = packages::resolve(&root, spec).map_err(wrap)?;
    let index = packages::index(&root, &resolved, manifest.packages()).map_err(wrap)?;
    let table = packages::union(mod_modules, index).map_err(wrap)?;
    Ok(table.into_parts())
}
