//! S10 KernelFacade：装载/转换接线的单一驱动面（JSON 信封 API，镜像 A1
//! 线格式——未来 wasm 壳 JS 侧仅换 loader，语义面不变）。
//!
//! - **装载序**（S10 锁定，S11 扩展资源步，r1 扩展模块表；管线单一收口于
//!   [`build_kernel`]）：`Sandbox::new`（stdlib 裁剪 + 内存上限 + 白名单环境）
//!   → `fuel::install_with_load_budget`（正式燃料，每 dispatch 面重置
//!   [`DEFAULT_FUEL_BUDGET`]；装载期记独立预算
//!   [`fuel::FUEL_LOAD_BUDGET`]）→ `inject::install`（math 精确集 + 内核种子
//!   random + ticks）→ `host_api::install`（装载期 `ygg` 完整面）→
//!   `resources::install`（S11：`ygg.data` 只读数据面——host 之后、stdlib 之前、
//!   Mod chunk 之前，Mod 顶层即可读资源）→ `stdlib::install`（S10 stdlib：先于
//!   Mod chunk，注册序即派发序）→ **模块表注入**（r1：`ModuleHandle::set_modules`,
//!   入口 chunk 之前——入口顶层即可 `require`）→ Mod chunk（失败 =
//!   [`KernelError::ModLoad`]）→ `host.seal`（运行期注册禁令：只封
//!   `ygg.on`）→ `register_into(&mut registry, "mod")`（ygg.on 调用序 =
//!   注册序 = 派发序）。
//!
//!   **`require` 不在这条分界上**：装载期与运行期（handler 内部）一视同仁。
//!   模块顶层是纯导出面（禁止面清单在 `ygg_modules::effects`，构造在
//!   [`crate::modules`] 的影子表），所以模块的执行时机不影响 handler 派发序——
//!   这也正是懒加载能成立的前提。
//! - **统一 store 代决（S10）**：`entities` / `kv` 为 `Rc<RefCell<..>>`
//!   单实例——handler 经 host 桥的即席状态写**直接落在 canonical store**；
//!   转换输入侧以快照克隆（snapshot-out，见 `cmd_event`）喂给
//!   `run_transition` 满足其 `&mut EntityStore` 形参，转换残余的
//!   `TransitionResult::revision_after` 为冻结诊断值，信封 revision 一律
//!   **转换结束后从 canonical store 现读**。读档 = canonical store 整体
//!   覆写（`apply_save`），后续 handler 写经同一 `Rc` 即时可见。
//! - **dispatch 原子性（r1 savepoint）**：一次 `cmd_event` 就是一个事务
//!   边界——取点在「校验全过后、转换开始前」，冻结 `entities` + `kv` 的
//!   基线副本；转换成功 = 提交（丢弃基线），转换失败 = **先整体还原
//!   canonical、再返回错误信封**（两个 `revision` 计数器随基线一并回退）。
//!   基线必须是 canonical 的**副本**而非换出：`ygg.kv.get` /
//!   `ygg.entity.*` 的读路径直读 canonical，换出会让转换期的 handler
//!   读到空表。不覆盖的旁路面：`external_log` 照记失败步（它是尝试日志，
//!   也是回放前提）、`warnings`/`meta` 为瞬态回执面、Lua VM 全局与模块
//!   upvalue、RNG / ticks 均不回滚（要跨步记忆只能落 KV）；panic 不在
//!   回滚覆盖内（进程已不可继续）。core `run_transition` 的「无事务语义」
//!   契约不变——它不持有状态所有权，事务边界归调用方（即本驱动）。
//! - **JSON 信封（A1）**：[`KernelFacade::dispatch`] 是唯一同步入口，
//!   成功 = `{"ok":true,...}`，域错误 = `{"ok":false,"error":{code,message}}`
//!   ——**域错误永不 throw**（Rust 侧返回 String 信封）；只有内部编程
//!   错误（重入、NotExternal 这类不可能路径）落 [`code::INTERNAL`]。
//!   **错误信封的状态语义**：`{"ok":false,...}` ⇒ 本次 dispatch 对
//!   canonical 状态（实体表 + KV + 两个 revision）**零影响**（savepoint
//!   已整体还原）；信封**不加**任何新字段（线格式零变更），下游据
//!   `ok:false` 即可判定「本次事件无效」。旁路面例外见上条。
//!   加性可选键 `warnings`（D1）：仅当 handler 在 `ygg.choice` 之前产生过
//!   `ygg.emit`（将被语义丢弃）时出现，单元素字符串数组；serde_json 字典序
//!   键序下确定性不变（同输入 → 同警告 → 同字节）。
//!   加性可选键 `meta`（TD1）：仅当 handler 调用过 `ygg.meta`（结构化回执面）
//!   时出现，内容为 `BTreeMap` 序的对象——与 `narrative` 完全分离，宿主不再
//!   从人读文本里剖结构化数据。**瞬态**：每次 dispatch 入口 clear、转换后
//!   take，不入 canonical snapshot / 存档 / 状态哈希。
//! - **payload 口径（A3）**：事件路径 payload 必填且须为 JSON 标量——
//!   空串、嵌套对象/数组/null 均为 VALIDATION（空 payload 放宽与否后续
//!   有真实需求再议，当前不放宽）；保留命令（save/load/state/log/drain）
//!   忽略 payload——不对称是既定口径，非疏漏。
//! - **保留命令名**：`save / load / state / log / drain` 五个名字保留为
//!   驱动命令，Mod 不得以其为事件名（`ygg.choice_selected` 等不受限）。
//! - **drain 恒空**：派生事件已在转换内 FIFO 排空（core `run_transition`
//!   单点收口），驱动面无剩余派生事件可 drain——`{"ok":true,"drained":[]}`
//!   为常量响应，字段保留以镜像 A1 线格式（历史形态兼容）。

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use mlua::Error as LuaError;
use serde_json::{json, Map as JsonMap, Value as Json};
use thiserror::Error as ThisError;

use ygg_core::bus::{HandlerError, HandlerRegistry, PendingChoice};
use ygg_core::entity::{EntityStore, Value};
use ygg_core::event::Event;
use ygg_core::kv::KvStore;
use ygg_core::save::{SaveError, SaveFile};
use ygg_core::snapshot::{canonical_snapshot, SnapshotError};
use ygg_core::transition::{run_transition, TransitionError};

use crate::fuel::{self, HookFuelMeter};
use crate::host_api::{self, HostApi, HostHandles, HostToolRunner, VfsStore};
use crate::inject::{self, DeterminismHandle};
use crate::modules::{ModuleError, ModuleSet};
use crate::resources::{ResourceError, ResourceTable};
use crate::sandbox::{Sandbox, ENGINE_VERSION};
use crate::stdlib;

/// 每次 dispatch 事件转换的燃料预算（转换边界全额重置，D-B1 每转换共享）。
pub const DEFAULT_FUEL_BUDGET: u64 = 100_000;

/// 运行期预算解析（r13 起纯函数化到 [`crate::fuel::resolve_budget`]，此处只
/// 剩构造期入口）：`YGG_FUEL_BUDGET` 为正整数时**显式最优先、精确采用、
/// 不受 cap 削减**；非法值或缺省回落自动扩缩 [`crate::fuel::auto_budget`]
/// （构造期 KV 尚未建立，`kv_entries = 0` → 地板 [`DEFAULT_FUEL_BUDGET`]，
/// 与旧契约行为逐字节一致；每个转换的真实预算在 `cmd_event` 步骤④ 由
/// [`KernelFacade::transition_budget`] 现读 `kv.len()` 重算）。
fn fuel_budget() -> u64 {
    fuel::resolve_budget(std::env::var("YGG_FUEL_BUDGET").ok().as_deref(), 0)
}

/// 信封错误码（稳定字符串契约，A1 线格式；下游按码分支不解析 message）。
pub mod code {
    /// 燃料耗尽（内核派发税或 VM 指令计量任一面触发）。
    pub const FUEL_EXHAUSTED: &str = "FUEL_EXHAUSTED";
    /// 沙箱拒绝（模块装载拒绝、内存超限等沙箱结构面拒绝）。
    pub const SANDBOX_DENIED: &str = "SANDBOX_DENIED";
    /// 校验拒绝（payload 非标量、事件构造失败、未注册事件、handler 报错、
    /// 存档版本/内容不符）。
    pub const VALIDATION: &str = "VALIDATION";
    /// 资源包边界体积限（S11 激活：`KernelError::ResourceBudget` → 本码）。
    pub const KV_LIMIT: &str = "KV_LIMIT";
    /// 内部错误（重入驱动、不可能路径——出现即驱动 bug）。
    pub const INTERNAL: &str = "INTERNAL";
}

/// 驱动构造错误（dispatch 信封面之外的 Rust 侧错误：构造/restore 即失败）。
#[derive(Debug, ThisError)]
pub enum KernelError {
    #[error(transparent)]
    Sandbox(#[from] LuaError),
    #[error("mod load failed: {0}")]
    ModLoad(String),
    #[error(transparent)]
    Save(#[from] SaveError),
    #[error("engine version mismatch: save written by `{saved}`, current engine `{current}`")]
    EngineMismatch { saved: String, current: String },
    #[error(transparent)]
    Snapshot(#[from] SnapshotError),
    /// 资源包**校验**失败伞形变体（manifest 违规 / payload 违规 / 未知
    /// schema id）；`details` 为首条违规的人读文案（含 file/pointer/code）。
    #[error("resource package rejected: {details}")]
    InvalidManifest { details: String },
    /// 资源内容寻址失败（文件缺失 / byteLength 不符 / sha256 不符）。
    #[error("resource content mismatch: {details}")]
    ContentMismatch { details: String },
    /// 资源预算超限（`KV_LIMIT` 预留码激活点）。
    #[error("resource budget exceeded: {total} bytes requested, limit {limit}")]
    ResourceBudget { total: usize, limit: usize },
}

impl KernelError {
    /// 错误 → A1 稳定码的**单点映射面**（下游按码分支不解析 message；
    /// 全仓任何 KernelError→code 判定必须经本方法，禁止散落 match）：
    ///
    /// | 变体 | 码 | 依据 |
    /// | --- | --- | --- |
    /// | `InvalidManifest` / `ContentMismatch` | `VALIDATION` | 资源包内容不合契约（作者侧可修复） |
    /// | `ResourceBudget` | `KV_LIMIT` | S11 预留码激活（体积限面） |
    /// | `EngineMismatch` | `VALIDATION` | 存档/引擎版本不符（与 `load` 命令同码） |
    /// | `Sandbox` / `ModLoad` / `Save` / `Snapshot` | `INTERNAL` | 结构面/编程面错误（非作者可修复） |
    pub fn code(&self) -> &'static str {
        match self {
            KernelError::InvalidManifest { .. } | KernelError::ContentMismatch { .. } => {
                code::VALIDATION
            }
            KernelError::ResourceBudget { .. } => code::KV_LIMIT,
            KernelError::EngineMismatch { .. } => code::VALIDATION,
            KernelError::Sandbox(_)
            | KernelError::ModLoad(_)
            | KernelError::Save(_)
            | KernelError::Snapshot(_) => code::INTERNAL,
        }
    }
}

/// [`ResourceError`] → [`KernelError`]（with_resources/restore 共用单点）。
fn resource_error_to_kernel(err: ResourceError) -> KernelError {
    match err {
        ResourceError::BudgetExceeded { total, limit } => {
            KernelError::ResourceBudget { total, limit }
        }
        ResourceError::Content(inner) => KernelError::ContentMismatch {
            details: inner.to_string(),
        },
        // Manifest / Payload / UnknownSchema 均为资源包校验失败 → VALIDATION 族
        //（伞形变体；Payload 的 Display 已携首条 file/pointer/code）。
        other => KernelError::InvalidManifest {
            details: other.to_string(),
        },
    }
}

/// [`ModuleError`] → [`KernelError`]：模块面拒绝一律走 `ModLoad`（装载期
/// 模块错误与 Mod 源码错误的同一出口，便于上层按码分类）。
impl From<ModuleError> for KernelError {
    fn from(err: ModuleError) -> Self {
        KernelError::ModLoad(err.to_string())
    }
}

/// S10/S11 装载管线的构件包（[`build_kernel`] 产出、
/// [`KernelFacade::from_parts`] 消费；字段顺序即生命周期挂靠面）。
struct KernelParts {
    sandbox: Sandbox,
    fuel_meter: HookFuelMeter,
    determinism: DeterminismHandle,
    host: HostApi,
    registry: HandlerRegistry,
    entities: Rc<RefCell<EntityStore>>,
    kv: Rc<RefCell<KvStore>>,
    /// `ygg.fs` 的内存态 VFS（`host_api::install` 自建，这里只接管同一个 `Rc`）。
    vfs: Rc<RefCell<VfsStore>>,
}

/// 装载序单一管线（S10 锁定序 + S11 资源步 + r1 模块表；`new` /
/// `with_resources` / `with_modules` / `with_resources_and_modules` 共享）。
///
/// `resources = Some(table)` 时在 host API 之后、stdlib 之前安装 `ygg.data`；
/// `None` 时跳过该步——`KernelFacade::new` 全序行为与 S10 逐字节一致（S10
/// 测试不加改动保持全绿即本函数的回归证据）。`modules = Some(set)` 时在入口
/// chunk 之前注入模块表；`None` 时 require 保持空表（单文件 Mod 的既有行为）。
fn build_kernel(
    mod_source: &str,
    resources: Option<&ResourceTable>,
    modules: Option<ModuleSet>,
) -> Result<KernelParts, KernelError> {
    // ① 沙箱构造：stdlib 裁剪 + 内存上限 + 白名单环境（ygg 占位表 + require 面就位）。
    let sandbox = Sandbox::new().map_err(KernelError::Sandbox)?;
    let lua = sandbox.lua().clone();
    // ② 正式燃料：转换预算 + 装载预算分流（hook 按 require 边界判相位），
    //    转换边界 reset 转换预算。
    let fuel_meter =
        fuel::install_with_load_budget(&lua, fuel_budget(), sandbox.modules().load_budget())
            .map_err(KernelError::Sandbox)?;
    // ③ 确定性注入：math 精确集 + 内核种子 random + ygg.ticks。
    let determinism = inject::install(&lua).map_err(KernelError::Sandbox)?;
    // ④ canonical store 先建：host 桥句柄与驱动共享同一 Rc（统一 store）。
    let entities = Rc::new(RefCell::new(EntityStore::new()));
    let kv = Rc::new(RefCell::new(KvStore::new()));
    // ⑤ host API：装载期 ygg 完整面（on/emit/narrate/choice/kv/entity/fs）。
    let host = host_api::install(
        &lua,
        HostHandles {
            kv: Rc::clone(&kv),
            entities: Rc::clone(&entities),
        },
    )
    .map_err(KernelError::Sandbox)?;
    // ⑤'' VFS：`install` 自建、与 Lua 阴面同一个 `Rc`。驱动必须拿它才能做
    //       dispatch 级 savepoint（失败步里 `ygg.fs.write` 过的文件不得漏到盘上）。
    let vfs = host.vfs_handle();
    // ⑤' S11 资源步：ygg.data 只读数据面（host 之后、stdlib 之前、
    // Mod chunk 之前——Mod 顶层即可读资源）。
    if let Some(table) = resources {
        crate::resources::install(&lua, table).map_err(KernelError::Sandbox)?;
    }
    // ⑥ stdlib：命名空间 + fsm + narrative（先于 Mod chunk，注册序即派发序）。
    stdlib::install(&lua).map_err(KernelError::Sandbox)?;
    // ⑥' r1 模块表注入：入口 chunk 之前（入口顶层即可 require）；空模块表时
    //     跳过——require 面仍在，只是对任何名字可读拒绝。
    if let Some(set) = modules {
        sandbox.modules().set_modules(set);
    }
    // ⑦ Mod chunk：装载期注册面收集 Mod handler；失败 = ModLoad。chunk
    //    命名 `@mod.lua`（与 stdlib `@stdlib/<name>` 同法，迭代 C3）：
    //    handler 报错 traceback 携带 mod 侧位置，排障不再只见 Lua 桥行号
    //    （迭代 D2 实证收口：mlua `CallbackError.traceback` 在 Rust 回调
    //    返回错误时捕获调用点 Lua 栈，错误文本已含 `mod.lua:<line>` 帧——
    //    host 回调拒绝同样可定位，无穿透修复必要）。
    if let Err(err) = lua.load(mod_source.to_owned()).set_name("@mod.lua").exec() {
        return Err(KernelError::ModLoad(err.to_string()));
    }
    // ⑧ 分相收口：seal（运行期注册禁令）→ 注册转出进注册表。
    //    只封 `ygg.on`——`require` 不在装载期边界上（模块顶层是纯导出面，
    //    执行时机不影响派发序，见 `crate::modules::ShadowBanned`）。
    host.seal(&lua).map_err(KernelError::Sandbox)?;
    let mut registry = HandlerRegistry::new();
    host.register_into(&mut registry, "mod");
    Ok(KernelParts {
        sandbox,
        fuel_meter,
        determinism,
        host,
        registry,
        entities,
        kv,
        vfs,
    })
}

/// 内核驱动门面：Mod 装载 + 事件转换 + 存读档 + 诊断面的单一入口
/// （native 探针/测试驱动形态；wasm 壳落地后语义面不变）。
pub struct KernelFacade {
    /// Lua 状态持有者（字段存活期 = VM 存活期；运行期经 host 桥间接使用）。
    _sandbox: Sandbox,
    fuel_meter: HookFuelMeter,
    /// 确定性注入句柄（时钟推进当前无驱动级调用面——ticks 只增不推，
    /// 句柄持有以固定 RNG/ticks 状态生命周期；S12 探针按需消费）。
    _determinism: DeterminismHandle,
    /// host API 控制柄（seal 已收口；注册面已转出进 `registry`，持有以
    /// 固定收集缓冲与 sentinel 生命周期；`take_dispatch_warning` 经此读取
    /// D1 choice 前置丢弃警告）。
    host: HostApi,
    /// handler 注册表（`register_into` 转出后的唯一派发面；驱动直有
    /// `&mut` —— 不经 `Rc`，dispatch 内独占借用无冲突）。
    registry: HandlerRegistry,
    /// canonical 实体表（handler 即席写与存读档的唯一真身；`Rc<RefCell>`
    /// 与 host 桥共享 = 统一 store 代决）。
    entities: Rc<RefCell<EntityStore>>,
    /// canonical 持久 KV（同上）。
    kv: Rc<RefCell<KvStore>>,
    /// `ygg.fs` 的内存态 VFS（与 host 桥共享 = 同一份内容面）。
    ///
    /// **不进 canonical 存档**：`SaveFile` / `ENGINE_VERSION` /
    /// `canonical_state_bytes` 零改动。VFS 是宿主侧的旁路面（落 `RunStore`
    /// 的制品面，开局由宿主回灌），它的代可以独立于事件代（reset 归档）。
    vfs: Rc<RefCell<VfsStore>>,
    /// 只读资源表（S11：构建后不可变；数据面非状态——不入 canonical
    /// store/存档/状态哈希，经 host 桥闭包间接消费，驱动不直接读面）。
    resources: Rc<ResourceTable>,
    /// 装载的 Mod 源码（诊断面：[`KernelFacade::mod_source`] 只读导出）。
    mod_source: String,
    /// 待决选择（choice-as-state：挂起即数据；转换终结后更新、随存档往返）。
    pending_choice: Option<PendingChoice>,
    /// 外部事件日志（**仅外部事件**，D-B1 回放前提；驱动侧旁路面，不入档）。
    external_log: Vec<Json>,
    /// dispatch 重入守卫（重入 = 驱动 bug → INTERNAL，不静默递归）。
    dispatching: bool,
}

impl KernelFacade {
    /// 按锁定装载序构造驱动（见模块文档）；Mod chunk 失败 =
    /// [`KernelError::ModLoad`]（含 mlua 可读错误文本）。无资源路径：资源表
    /// 为空基线（[`ResourceTable::empty`]），其余序与 `with_resources` 全同。
    pub fn new(mod_source: &str) -> Result<Self, KernelError> {
        let parts = build_kernel(mod_source, None, None)?;
        Ok(Self::from_parts(
            parts,
            Rc::new(ResourceTable::empty()),
            mod_source,
        ))
    }

    /// r1：多文件 Mod 构造——除入口 source 外另给已登记的模块表（`src/*.lua`
    /// 拆出的子模块）。模块表在入口 chunk 之前注入，所以入口顶层就能
    /// `require("src.util")`；入口之后的 handler 里同样可以 `require`（懒加
    /// 载），模块 chunk 受影子表约束（只能纯导出）。
    ///
    /// 无资源路径（资源表为空基线）；带资源的多文件 Mod 走
    /// [`KernelFacade::with_resources_and_modules`]。
    pub fn with_modules(mod_source: &str, modules: ModuleSet) -> Result<Self, KernelError> {
        let parts = build_kernel(mod_source, None, Some(modules))?;
        Ok(Self::from_parts(
            parts,
            Rc::new(ResourceTable::empty()),
            mod_source,
        ))
    }

    /// S11：带资源包构造驱动。资源表构建**先于任何 Lua 状态**（fail-fast：
    /// 预算/寻址/校验失败零 Lua 侧副作用）；通过后走与 [`KernelFacade::new`]
    /// 全同的装载管线（`resources::install` 在 host 之后、stdlib 之前）。
    pub fn with_resources(
        mod_source: &str,
        manifest_bytes: &[u8],
        schemas: &[(String, String)],
        resources: &[(String, Vec<u8>)],
    ) -> Result<Self, KernelError> {
        let table = ResourceTable::build(manifest_bytes, schemas, resources)
            .map_err(resource_error_to_kernel)?;
        let parts = build_kernel(mod_source, Some(&table), None)?;
        Ok(Self::from_parts(parts, Rc::new(table), mod_source))
    }

    /// S11 + r1：带资源包的多文件 Mod——资源表 fail-fast 与模块表注入的
    /// 唯一组合面（装载序：资源 → stdlib → 模块表 → 入口 chunk → seal）。
    pub fn with_resources_and_modules(
        mod_source: &str,
        modules: ModuleSet,
        manifest_bytes: &[u8],
        schemas: &[(String, String)],
        resources: &[(String, Vec<u8>)],
    ) -> Result<Self, KernelError> {
        let table = ResourceTable::build(manifest_bytes, schemas, resources)
            .map_err(resource_error_to_kernel)?;
        let parts = build_kernel(mod_source, Some(&table), Some(modules))?;
        Ok(Self::from_parts(parts, Rc::new(table), mod_source))
    }

    /// S11：带资源包的读档重建——`from_bytes` 先判存档形态，再走
    /// [`KernelFacade::with_resources`] 全序（资源表 fail-fast → 装载管线），
    /// 最后 [`KernelFacade::apply_save`] 覆写状态面（资源不在存档内、不受
    /// 读档影响——资源是数据不是状态）。
    pub fn restore_with_resources(
        save_bytes: &[u8],
        mod_source: &str,
        manifest_bytes: &[u8],
        schemas: &[(String, String)],
        resources: &[(String, Vec<u8>)],
    ) -> Result<Self, KernelError> {
        let save = SaveFile::from_bytes(save_bytes)?;
        let mut kernel = Self::with_resources(mod_source, manifest_bytes, schemas, resources)?;
        kernel.apply_save(&save)?;
        Ok(kernel)
    }

    /// S11 + r2：带资源包的多文件 Mod 读档重建——[`KernelFacade::restore_with_resources`]
    /// 的模块表变体。热载路径必须走这条：模块文件变了而入口没变时，宿主签名
    /// 会变并触发重装载，若重装载丢掉模块表（只重供资源），跑起来的仍是旧代码
    /// 或入口 require 直接失败——两种都不是「热载成功」。
    pub fn restore_with_resources_and_modules(
        save_bytes: &[u8],
        mod_source: &str,
        modules: ModuleSet,
        manifest_bytes: &[u8],
        schemas: &[(String, String)],
        resources: &[(String, Vec<u8>)],
    ) -> Result<Self, KernelError> {
        let save = SaveFile::from_bytes(save_bytes)?;
        let mut kernel = Self::with_resources_and_modules(
            mod_source,
            modules,
            manifest_bytes,
            schemas,
            resources,
        )?;
        kernel.apply_save(&save)?;
        Ok(kernel)
    }

    /// 构件包 + 资源表 + Mod 源 → 驱动实例（字段装配单点；三个构造入口共用）。
    fn from_parts(parts: KernelParts, resources: Rc<ResourceTable>, mod_source: &str) -> Self {
        Self {
            _sandbox: parts.sandbox,
            fuel_meter: parts.fuel_meter,
            _determinism: parts.determinism,
            host: parts.host,
            registry: parts.registry,
            entities: parts.entities,
            kv: parts.kv,
            vfs: parts.vfs,
            resources,
            mod_source: mod_source.to_owned(),
            pending_choice: None,
            external_log: Vec::new(),
            dispatching: false,
        }
    }

    /// 从存档 bytes 重建驱动：`new(mod_source)` 全序重建（handler 为代码
    /// 不入档、重装载即恢复）后 [`KernelFacade::apply_save`] 覆写状态面。
    pub fn restore(save_bytes: &[u8], mod_source: &str) -> Result<Self, KernelError> {
        let save = SaveFile::from_bytes(save_bytes)?;
        let mut kernel = Self::new(mod_source)?;
        kernel.apply_save(&save)?;
        Ok(kernel)
    }

    /// 状态面覆写（读档语义）：engine_version 门（不符 =
    /// [`KernelError::EngineMismatch`]，存档由异版引擎写出即拒绝）后整体
    /// 覆写 canonical 实体表 / KV / pending_choice（覆写经驱动持有的同一
    /// `Rc`，后续 handler 写即时可见——统一 store 代决）。
    fn apply_save(&mut self, save: &SaveFile) -> Result<(), KernelError> {
        if save.engine_version() != ENGINE_VERSION {
            return Err(KernelError::EngineMismatch {
                saved: save.engine_version().to_owned(),
                current: ENGINE_VERSION.to_owned(),
            });
        }
        *self.entities.borrow_mut() = save.entities().clone();
        *self.kv.borrow_mut() = save.kv().clone();
        self.pending_choice = save.pending_choice().cloned();
        Ok(())
    }

    /// 唯一同步入口（A1）：命令分发 + JSON 信封封装。域错误以信封返回
    /// （永不 throw）；重入 dispatch = 驱动 bug → INTERNAL（不静默递归）。
    pub fn dispatch(&mut self, cmd: &str, payload_json: &str) -> String {
        if self.dispatching {
            return error_envelope(code::INTERNAL, "re-entrant dispatch detected (driver bug)");
        }
        self.dispatching = true;
        let reply = self.dispatch_inner(cmd, payload_json);
        self.dispatching = false;
        reply
    }

    /// 命令分发（保留名 `save/load/state/log/drain`；其余一律按外部事件名
    /// 进入转换路径 [`KernelFacade::cmd_event`]）。
    ///
    /// TD1：入口先清空结构化 meta——上一条回执的 meta 不得残留到本条
    /// （尤其不得让保留命令的回执带上上一条事件的 meta）。
    fn dispatch_inner(&mut self, cmd: &str, payload_json: &str) -> String {
        self.host.clear_meta();
        match cmd {
            "save" => self.cmd_save(),
            "load" => self.cmd_load(payload_json),
            "state" => self.cmd_state(),
            "log" => self.cmd_log(),
            // 派生事件已在转换内 FIFO 排空：drain 恒空（线格式兼容保留）。
            "drain" => json!({"ok": true, "drained": []}).to_string(),
            _ => self.cmd_event(cmd, payload_json),
        }
    }

    /// `save`：完整状态快照（实体表 + KV + pending_choice，B3）序列化为
    /// bytes（确定性序）后原样解析回 Json 嵌入信封——信封内容与存档
    /// bytes 逐字段一致（读档方可直接把 `save` 字段作为 load payload）。
    fn cmd_save(&mut self) -> String {
        let save = SaveFile::new(
            ENGINE_VERSION,
            self.entities.borrow().clone(),
            self.kv.borrow().clone(),
            self.pending_choice.clone(),
        );
        let outcome = save
            .to_bytes()
            .and_then(|bytes| serde_json::from_slice::<Json>(&bytes).map_err(SaveError::Json));
        match outcome.map_err(KernelError::Save) {
            Ok(parsed) => json!({"ok": true, "save": parsed}).to_string(),
            Err(err) => error_envelope(code::INTERNAL, err),
        }
    }

    /// `load`：payload = 存档 JSON 字符串；版本/内容不符 → VALIDATION
    /// 信封（EngineMismatch / SaveError），成功覆写后 `{"ok":true,
    /// "restored":true,"pendingChoice":<同 state 形态>}`——待决选择
    /// 随存档往返（B3），读档方零额外 dispatch 即可恢复提示面（A1）。
    fn cmd_load(&mut self, payload_json: &str) -> String {
        let save = match SaveFile::from_bytes(payload_json.as_bytes()) {
            Ok(save) => save,
            Err(err) => return error_envelope(code::VALIDATION, format!("load rejected: {err}")),
        };
        match self.apply_save(&save) {
            Ok(()) => json!({
                "ok": true,
                "restored": true,
                "pendingChoice": self.pending_choice.as_ref().map(choice_to_json),
            })
            .to_string(),
            Err(KernelError::EngineMismatch { saved, current }) => error_envelope(
                code::VALIDATION,
                format!("load rejected: engine version mismatch: save written by `{saved}`, current engine `{current}`"),
            ),
            Err(err) => error_envelope(code::INTERNAL, err),
        }
    }

    /// `state`：诊断面——引擎/实体/KV 版本号 + 快照字节数 + 待决选择。
    fn cmd_state(&self) -> String {
        let snapshot_bytes = match self.canonical_state_bytes() {
            Ok(bytes) => bytes,
            Err(err) => return error_envelope(code::INTERNAL, err),
        };
        json!({
            "ok": true,
            "engineVersion": ENGINE_VERSION,
            "revision": self.entities.borrow().revision(),
            "kvRevision": self.kv.borrow().revision(),
            // 加性诊断键（同 `kvRevision` 的性质，不是状态）：VFS 的写代数与
            // 件数。`state` 信封的消费方（`agent.rs::print_state` / 测试）都按
            // 键取值，没有严格键集比较，所以加键不破线格式。
            "vfsRevision": self.vfs.borrow().revision(),
            "vfsFiles": self.vfs.borrow().len(),
            "snapshotBytes": snapshot_bytes.len(),
            "pendingChoice": self.pending_choice.as_ref().map(choice_to_json),
        })
        .to_string()
    }

    /// `log`：外部事件日志（仅外部事件；驱动旁路面，不入档）。
    fn cmd_log(&self) -> String {
        json!({"ok": true, "log": self.external_log}).to_string()
    }

    /// 本转换的燃料预算（r13 自动扩缩的唯壹取值点，`cmd_event` 步骤④消费）：
    ///
    /// - 显式 `YGG_FUEL_BUDGET`（进程 env）最优先、精确采用、不受 cap 削减；
    /// - 缺省/非法 → [`crate::fuel::auto_budget`]：线性 + 地板 + 硬顶，
    ///   `clamp(2000 × kv_len, 100k, 4M)`。
    ///
    /// `kv.len()` 是 canonical KV 条目总数（`BTreeMap::len` O(1)），**每转换
    /// 现读、零缓存**：预算只依赖转换起点的 canonical 状态规模，回放/续局
    /// 重建同状态 → 同预算序列 → 同结局；预算不入 save、不入状态哈希。因子
    /// 取 `kv.len()` 是 mod 无关的充分统计（域键约定只归 mod，P3 宿主零语义），
    /// 标定与取值论证见 [`crate::fuel::auto_budget`]。
    fn transition_budget(&self) -> u64 {
        fuel::resolve_budget(
            std::env::var("YGG_FUEL_BUDGET").ok().as_deref(),
            self.kv.borrow().len(),
        )
    }

    /// 事件转换路径（非保留名命令）：registry 预检（fail-closed）→ payload
    /// 解析（标量门）→ 外部事件构造 → 外部日志 → 燃料重置 → snapshot-out
    /// 转换 → 信封回包。
    fn cmd_event(&mut self, name: &str, payload_json: &str) -> String {
        // ⓪⁻ registry 预检（fail-closed 门面侧）：未注册事件名在**一切副作用
        //    之前**拒绝——不进外部日志（③）、不耗燃料（④）、不动状态基线（⑤）。
        //    「事件不存在」先于「序号越界 / 载荷非法」（registry 是运行时唯一
        //    事实，宿主不持事件名白名单：`ygg.*` 保留事件同走本预检，已注册者
        //    天然通过）。先 `collect` 再判断：`event_names()` 借用 `&self.registry`，
        //    收集完即释放，错误分支才能继续用 `self`。
        //    文案三要素：事件名原样 + 已注册计数 + 全集（字典序）——Agent 一次
        //    往返即可自纠（与 daemon unknown-op 全列合法 op 同先例）。
        let names: Vec<&str> = self.registry.event_names().collect();
        if !names.contains(&name) {
            return error_envelope(
                code::VALIDATION,
                format!(
                    "未注册事件 `{name}`：本 Mod 已注册 {} 个事件：{}",
                    names.len(),
                    names.join("、")
                ),
            );
        }
        // ⓪ `ygg.choice_selected` 域内校验（A2）：待决选择存在时，序号
        //    必须是 1..=options_len 的整数——越界/非整数 = 可读 VALIDATION
        //    （玩家视角「选项不存在」，不进内核转换、不耗燃料、不改状态）。
        if name == "ygg.choice_selected" {
            if let Some(choice) = &self.pending_choice {
                let n = choice.options().len();
                let index = serde_json::from_str::<Json>(payload_json)
                    .ok()
                    .and_then(|v| v.as_i64());
                if !matches!(index, Some(i) if (1..=n as i64).contains(&i)) {
                    return error_envelope(
                        code::VALIDATION,
                        format!("选项不存在：有效范围 1..{n}"),
                    );
                }
            }
        }
        // ① payload 标量门：JSON 解析失败或非标量（嵌套对象/数组/null）
        //    → VALIDATION（core Value 只有四标量）。
        let payload_value: Json = match serde_json::from_str(payload_json) {
            Ok(value) => value,
            Err(err) => {
                return error_envelope(
                    code::VALIDATION,
                    format!("event payload is not valid JSON: {err}"),
                )
            }
        };
        let payload = match json_payload_to_value(&payload_value) {
            Ok(value) => value,
            Err(message) => return error_envelope(code::VALIDATION, message),
        };
        // ② 外部事件构造（判据先于燃料扣除；失败 → VALIDATION）。
        let event = match Event::external(name.to_owned(), payload.clone()) {
            Ok(event) => event,
            Err(err) => return error_envelope(code::VALIDATION, err),
        };
        // ③ 外部日志（仅外部事件；派生事件由回放重放时自动再产生）。
        self.external_log.push(json!({
            "name": event.name(),
            "payload": value_to_json(&payload),
        }));
        // ④ 燃料预算每转换重置（D-B1：转换内级联共享同一预算）。r13：预算
        //    取 [`Self::transition_budget`]——显式 env 最优先；否则按转换起点
        //    的 canonical 状态规模自动扩缩（`kv.len()` O(1) 现读，零缓存：
        //    回放/续局重建同状态 → 同预算序列）。
        self.fuel_meter.reset(self.transition_budget());
        // ⑤ dispatch 级 savepoint，**取点**：校验（⓪①②）全过、副作用
        //    （③ 日志、④ 燃料）已落之后，转换开始之前——在此冻结 canonical
        //    的两块状态面基线。基线是 canonical 的**副本**（不是换出）：
        //    handler 的即席写经 host 桥 `Rc` 直落 canonical，而 `ygg.kv.get`
        //    / `ygg.entity.*` 的读路径同样直读 canonical——换出会让转换期的
        //    handler 读到空表。实体基线同时充当 `run_transition` 的
        //    `&mut EntityStore` 形参（该形参在 facade 路径下只作 revision
        //    记账诱饵，handler 从不持有它的引用）。借用均为语句级临时
        //    `Ref`/`RefMut`，不跨越 `run_transition` 存活。
        let mut baseline = self.entities.borrow().clone();
        let kv_baseline = self.kv.borrow().clone();
        // VFS 与 kv 同法：基线是 canonical 的**副本**，不是换出（换出会让转换期的
        // `ygg.fs.read` 读到空表）；`VfsStore` 自身 `Clone`，`dirty` 集也随基线
        // 克隆，所以失败 dispatch 里 `ygg.fs.write` 过的路径不会漏进下一次 flush。
        let vfs_baseline = self.vfs.borrow().clone();
        let outcome = run_transition(
            &mut baseline,
            &mut self.registry,
            &mut self.fuel_meter,
            event,
        );
        // ⑤' D1：无论转换成败都取走本次 dispatch 的警告（choice 前置丢弃
        // 面；每 dispatch 至多一条，drain 即清空，不泄漏到后续 dispatch）。
        let warning = self.host.take_dispatch_warning();
        // ⑤'' TD1：meta 同样无论成败都 drain（两条路径不留残渣），但只在
        // 成功信封里露出——错误信封不夹带 Mod 侧数据。
        let meta = self.host.take_meta();
        // ⑥ 裁定：成功 = **提交**（丢弃基线——canonical 保留本次全部写入）；
        //    失败 = **先还原 canonical、再返回信封**。还原语句必须落在错误
        //    分支内、且先于该分支的返回（`map_transition_error` 只把错误映射
        //    成信封，不碰状态）；把它挪到 match 之后（错误分支已返回，不可
        //    达）或只写在成功分支，savepoint 就完全不生效。
        let result = match outcome {
            Ok(result) => {
                drop(baseline);
                drop(kv_baseline);
                result
            }
            Err(err) => {
                *self.entities.borrow_mut() = baseline;
                *self.kv.borrow_mut() = kv_baseline;
                // 必须落在错误分支内、且先于该分支的返回：失败步的文件不能
                // 出现在 `ygg.fs.list` 里，否则 Mod 会拿半个状态上的产物做判定。
                *self.vfs.borrow_mut() = vfs_baseline;
                return map_transition_error(err);
            }
        };
        self.pending_choice = result.pending_choice().cloned();
        // ⑦ 信封：revision 在转换结束后从 canonical 现读
        //    （TransitionResult.revision_after 为快照克隆的冻结诊断值，
        //    不进信封——统一 store 代决的 revision 口径）。
        let revision = self.entities.borrow().revision();
        let mut envelope = json!({
            "ok": true,
            "event": result.external().name(),
            "narrative": result.narrative(),
            "pendingChoice": self.pending_choice.as_ref().map(choice_to_json),
            "eventsProcessed": result.events_processed(),
            "eventsDerived": result.events_derived(),
            "eventsDropped": result.events_dropped(),
            "revision": revision,
        });
        // D1：加性可选键——仅在非空时写入（serde_json 字典序键序，确定性
        // 不变：同输入 → 同警告 → 同字节）。
        if let Some(warning) = warning {
            envelope["warnings"] = json!([warning]);
        }
        // TD1：加性可选键——结构化 meta（Mod 自报数据，与叙事分离）。
        // 键序 = BTreeMap 遍历序 → serde_json 默认 Map（BTreeMap）序：
        // 同输入同字节，不依赖 Mod 的写入次序。
        if !meta.is_empty() {
            envelope["meta"] = meta_to_json(&meta);
        }
        envelope.to_string()
    }

    /// canonical 状态字节（实体表规范快照，SHA 之前的确定性序列化面；
    /// store 写入门保证非有限值不可能入场，错误仅在驱动 bug 时出现）。
    pub fn canonical_state_bytes(&self) -> Result<Vec<u8>, SnapshotError> {
        canonical_snapshot(&self.entities.borrow())
    }

    /// 装载的 Mod 源码（诊断只读面）。
    pub fn mod_source(&self) -> &str {
        &self.mod_source
    }

    /// 只读资源表（S11 诊断面；资源不可变，无写面外溢）。
    pub fn resources(&self) -> &ResourceTable {
        &self.resources
    }

    // ---- `ygg.fs` 的宿主机侧面（预载 / 落盘水位 / 诊断）----

    /// 预载一件 VFS 文件（宿主开局从 `RunStore::artifact_get` 回灌）。
    /// **不置 dirty / 不动 revision**：它不产生新的落盘需求。
    pub fn vfs_load(&mut self, path: &str, content: &[u8], sha256: &str) {
        self.vfs.borrow_mut().load(path, content, sha256);
    }

    /// 宿主侧写 VFS（`--artifact-put` 预载 / 宿主直接落地）。返回摘要。
    ///
    /// 与 Lua 写面**同一道门**：错误串已是可读原因。
    pub fn vfs_put(&mut self, path: &str, content: &[u8]) -> Result<String, String> {
        self.vfs
            .borrow_mut()
            .put(path, content)
            .map(|file| file.sha256)
    }

    /// 全量条目（`BTreeMap` 序）——诊断面与「强制重刷」面。
    pub fn vfs_entries(&self) -> Vec<(String, Vec<u8>, String)> {
        self.vfs
            .borrow()
            .entries()
            .map(|(path, file)| (path.to_owned(), file.bytes.clone(), file.sha256.clone()))
            .collect()
    }

    /// 取走未落盘条目（排空 dirty）。宿主 flush 成功后调用。
    pub fn vfs_take_dirty(&mut self) -> Vec<(String, Vec<u8>, String)> {
        self.vfs
            .borrow_mut()
            .take_dirty()
            .into_iter()
            .map(|(path, file)| (path, file.bytes, file.sha256))
            .collect()
    }

    /// 把一批路径重新标脏（落盘中途失败后的回填：内容寻址让重试只重写相同字节）。
    pub fn vfs_requeue_dirty(&mut self, paths: &[String]) {
        self.vfs.borrow_mut().requeue_dirty(paths);
    }

    /// VFS 写代数（每次成功 `put` +1）。
    pub fn vfs_revision(&self) -> u64 {
        self.vfs.borrow().revision()
    }

    /// VFS 当前件数。
    pub fn vfs_len(&self) -> usize {
        self.vfs.borrow().len()
    }

    /// 挂载或卸载宿主侧同步工具执行器（供 `ygg.tool.call` 同步调用）。
    pub fn set_tool_runner(&mut self, runner: Option<Box<dyn HostToolRunner>>) {
        self.host.set_tool_runner(runner);
    }
}

/// 转换错误 → 稳定码信封（A1：下游按码分支，message 只作人读定位）。
///
/// Handler 面（mlua 桥把 Lua 侧错误包成 HandlerError）按文案归类：
/// "fuel exhausted"（VM 指令计量耗尽经 Lua 错误浮出）→ FUEL_EXHAUSTED；
/// "sandbox denied" / "memory error"（模块拒绝桩 / 内存上限）→
/// SANDBOX_DENIED；其余 handler 错误 → VALIDATION。NotExternal 为不可能
/// 路径（外部事件构造先行判据）→ INTERNAL。
fn map_transition_error(err: TransitionError) -> String {
    match err {
        TransitionError::Fuel(fuel) => error_envelope(
            code::FUEL_EXHAUSTED,
            format!(
                "fuel exhausted: requested {}, remaining {}",
                fuel.requested, fuel.remaining
            ),
        ),
        TransitionError::Handler(HandlerError(msg)) => {
            if msg.starts_with("unregistered event") {
                // 未注册事件（bus fail-closed；外部名已被门面预检拦截，走到这里的
                // 都是派生面：handler `ygg.emit` 了未注册名）→ VALIDATION。显式
                // 分支钉住码面，不依赖末尾兜底；消息带 bus 稳定前缀 + 事件名。
                error_envelope(code::VALIDATION, msg)
            } else if msg.contains("fuel exhausted") {
                error_envelope(code::FUEL_EXHAUSTED, msg)
            } else if msg.contains("sandbox denied") || msg.contains("memory error") {
                error_envelope(code::SANDBOX_DENIED, msg)
            } else {
                error_envelope(code::VALIDATION, msg)
            }
        }
        TransitionError::NotExternal(name) => error_envelope(
            code::INTERNAL,
            format!("internal bug: transition input must be an external event, got `{name}`"),
        ),
    }
}

/// JSON payload → core `Value`（标量门）：Bool / Number（整数优先取 i64，
/// 其余按 f64）/ String 直通；嵌套对象、数组、null → 可读 VALIDATION 文案。
fn json_payload_to_value(value: &Json) -> Result<Value, String> {
    match value {
        Json::Bool(b) => Ok(Value::Bool(*b)),
        Json::Number(n) => match n.as_i64() {
            Some(i) => Ok(Value::Int(i)),
            None => Ok(Value::Float(n.as_f64().unwrap_or_default())),
        },
        Json::String(s) => Ok(Value::Str(s.clone())),
        other => Err(format!(
            "event payload must be a scalar, got {}",
            json_type_name(other)
        )),
    }
}

/// JSON 值类型名（payload 标量门的可读报错文案）。
fn json_type_name(value: &Json) -> &'static str {
    match value {
        Json::Null => "null",
        Json::Bool(_) => "boolean",
        Json::Number(_) => "number",
        Json::String(_) => "string",
        Json::Array(_) => "array",
        Json::Object(_) => "object",
    }
}

/// core `Value` → JSON（诊断/日志序列化面）：四标量直通；Float 非有限值
/// 经 `Number::from_f64` 落 `null`（信封永不携带非法 JSON 数值——事件构造
/// 门已拒绝非有限值，此为序列化层纵深）。
fn value_to_json(value: &Value) -> Json {
    match value {
        Value::Int(i) => (*i).into(),
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(Json::Number)
            .unwrap_or(Json::Null),
        Value::Bool(b) => (*b).into(),
        Value::Str(s) => s.as_str().into(),
    }
}

/// 结构化 meta → JSON 对象（TD1）：键序 = `BTreeMap` 序遍历（确定性；
/// serde_json 默认无 `preserve_order`，`Map` 也是 `BTreeMap`，两重保险）。
fn meta_to_json(meta: &BTreeMap<String, Json>) -> Json {
    let mut map = JsonMap::new();
    for (key, value) in meta {
        map.insert(key.clone(), value.clone());
    }
    Json::Object(map)
}

/// 待决选择 → JSON（prompt + options 序保真）。
fn choice_to_json(choice: &PendingChoice) -> Json {
    json!({
        "prompt": choice.prompt(),
        "options": choice.options(),
    })
}

/// 成功信封碎片无统一形态（各命令字段不同），错误信封统一于此。
fn error_envelope(code_str: &str, message: impl std::fmt::Display) -> String {
    json!({
        "ok": false,
        "error": {"code": code_str, "message": message.to_string()},
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::state_hash;

    fn reply_code(reply: &str) -> String {
        let parsed: Json = serde_json::from_str(reply).unwrap();
        parsed["error"]["code"].as_str().unwrap_or("").to_owned()
    }

    fn reply(reply: &str) -> Json {
        serde_json::from_str(reply).unwrap()
    }

    /// 空转 Mod：无任何 Mod 注册（仅 stdlib 内置注册 `ygg.narrative.goto`）
    /// ——未注册名预检的最小环境。
    const IDLE_MOD: &str = "-- idle mod: no handlers";

    /// 最小注册 Mod：一个无操作 handler——成功信封形态断言必须来自
    /// **已注册**事件（fail-closed 后未注册名不再有成功信封）。
    const NOOP_MOD: &str = r#"ygg.on("event.ok", function() end)"#;

    #[test]
    fn envelope_error_shape_uses_stable_codes() {
        let mut kernel = KernelFacade::new(NOOP_MOD).expect("kernel construction");
        // 非标量 payload → VALIDATION（payload 标量门；事件已注册，走的是
        // 门不是预检）。
        let reply = kernel.dispatch("event.ok", r#"{"nested":{"deep":1}}"#);
        assert_eq!(reply_code(&reply), code::VALIDATION);
        // 保留命令名 + 非法存档 → VALIDATION。
        let reply = kernel.dispatch("load", "not json");
        assert_eq!(reply_code(&reply), code::VALIDATION);
        // 未注册事件名 → VALIDATION（预检，零副作用；见下一用例）。
        let reply = kernel.dispatch("event.any", "0");
        assert_eq!(reply_code(&reply), code::VALIDATION);
        // 成功信封无 error 字段（已注册事件）。
        let reply = kernel.dispatch("event.ok", "0");
        let parsed: Json = serde_json::from_str(&reply).unwrap();
        assert_eq!(parsed["ok"], Json::Bool(true));
        assert!(parsed.get("error").is_none());
    }

    // ---- 未注册事件 fail-closed（门面预检 T5 / 派生面拒+回滚 T6）----

    /// T5：预检拒绝零副作用——不进外部日志（回放日志零污染）、不动状态
    /// 基线（revision / kvRevision / pendingChoice 原样），且文案三要素
    /// 齐全（事件名 + 计数 + 字典序全集）；拒绝后 registry 未受损，已注册
    /// 事件照常可用。空注册表（IDLE_MOD）文案同构（「已注册 0 个事件：」）。
    #[test]
    fn precheck_rejects_unknown_event_with_zero_side_effects() {
        let mut kernel = KernelFacade::new(SAVEPOINT_MOD).expect("kernel construction");
        let rejected = kernel.dispatch("event.nope", "0");
        assert_eq!(reply_code(&rejected), code::VALIDATION);
        let parsed = reply(&rejected);
        let message = parsed["error"]["message"].as_str().unwrap();
        assert!(
            message.contains("`event.nope`"),
            "文案要点名事件：{message}"
        );
        assert!(
            message.contains("已注册 4 个事件"),
            "文案要带计数（3 个 Mod 注册 + 1 个 stdlib 内置）：{message}"
        );
        assert_eq!(
            message,
            "未注册事件 `event.nope`：本 Mod 已注册 4 个事件：event.bad、event.ok、event.spawn、ygg.narrative.goto",
            "文案三要素（名字 + 计数 + 字典序全集）必须逐字节确定（含 stdlib 内置注册 ygg.narrative.goto）：{message}"
        );
        // 零副作用 ①：外部日志（回放依据）不得被污染。
        let log = reply(&kernel.dispatch("log", ""));
        assert_eq!(log["log"], json!([]), "预检拒绝不得进回放日志：{log}");
        // 零副作用 ②：状态基线原样（revision / kvRevision 均 0，无待决选择）。
        let state = reply(&kernel.dispatch("state", ""));
        assert_eq!(state["revision"], Json::from(0));
        assert_eq!(state["kvRevision"], Json::from(0));
        assert!(state["pendingChoice"].is_null());
        // 零副作用 ③：registry 未受损，已注册事件照常可用。
        let ok = reply(&kernel.dispatch("event.ok", "0"));
        assert_eq!(ok["ok"], Json::Bool(true), "预检不得损伤已注册面：{ok}");
        // 空注册表边缘实际不可达：stdlib 恒注册 `ygg.narrative.goto`（内置面
        // 也算成员）——文案如实反映运行时 registry，这正是「registry 是唯一
        // 事实」的体现：不特判、不白名单、内置注册同样计入全集。
        let mut idle = KernelFacade::new(IDLE_MOD).expect("kernel construction");
        let idle_reply = idle.dispatch("event.any", "0");
        assert_eq!(reply_code(&idle_reply), code::VALIDATION);
        assert_eq!(
            reply(&idle_reply)["error"]["message"],
            json!("未注册事件 `event.any`：本 Mod 已注册 1 个事件：ygg.narrative.goto"),
            "仅 stdlib 内置注册的空 Mod：文案如实反映 registry：{idle_reply}"
        );
    }

    /// T6：派生面 fail-closed——handler `ygg.emit` 未注册名，整次转换失败
    /// 回滚（bus Err → TransitionError::Handler → savepoint 还原），错误码
    /// 显式分支钉在 VALIDATION，消息带 bus 稳定前缀 + 派生名。
    #[test]
    fn derived_unregistered_event_fails_and_rolls_back_the_transition() {
        const DERIVED_MISS_MOD: &str = r#"
ygg.on("event.ok", function()
  ygg.kv.set("clean", "yes")
end)
ygg.on("event.mis.emit", function()
  ygg.kv.set("dirty", "yes")
  ygg.emit("event.never.registered", 0)
end)
"#;
        let mut kernel = KernelFacade::new(DERIVED_MISS_MOD).expect("kernel construction");
        // 先提交一个非空基线（否则「回滚」与「清空」不可区分）。
        let ok = reply(&kernel.dispatch("event.ok", "0"));
        assert_eq!(ok["ok"], Json::Bool(true));
        let baseline_save = reply(&kernel.dispatch("save", ""))["save"].to_string();

        let failed = reply(&kernel.dispatch("event.mis.emit", "0"));
        assert_eq!(failed["ok"], Json::Bool(false), "{failed}");
        assert_eq!(failed["error"]["code"], code::VALIDATION);
        let message = failed["error"]["message"].as_str().unwrap();
        assert!(
            message.starts_with("unregistered event `event.never.registered`"),
            "派生面消息带 bus 稳定前缀 + 派生名：{message}"
        );
        // 回滚：失败步的 kv 写（dirty）不得留下，基线字节逐字节保持。
        assert_eq!(
            reply(&kernel.dispatch("save", ""))["save"].to_string(),
            baseline_save,
            "派生未注册必须整转换回滚"
        );
    }

    #[test]
    fn drain_is_always_empty_and_state_reports_diagnostic_fields() {
        let mut kernel = KernelFacade::new(IDLE_MOD).expect("kernel construction");
        let reply = kernel.dispatch("drain", "");
        let parsed: Json = serde_json::from_str(&reply).unwrap();
        assert_eq!(parsed["drained"], Json::Array(vec![]));
        let reply = kernel.dispatch("state", "");
        let parsed: Json = serde_json::from_str(&reply).unwrap();
        assert_eq!(parsed["engineVersion"], ENGINE_VERSION);
        assert_eq!(parsed["revision"], 0);
        assert_eq!(parsed["kvRevision"], 0);
        assert!(parsed["pendingChoice"].is_null());
        assert!(parsed["snapshotBytes"].is_number());
        // log 初始为空。
        let reply = kernel.dispatch("log", "");
        let parsed: Json = serde_json::from_str(&reply).unwrap();
        assert_eq!(parsed["log"], Json::Array(vec![]));
    }

    #[test]
    fn engine_mismatch_is_rejected_on_restore() {
        let mut kernel = KernelFacade::new(IDLE_MOD).expect("kernel construction");
        let reply = kernel.dispatch("save", "");
        // 信封内嵌 save 对象才是存档本体（信封键序 = serde_json 默认字典序）。
        let envelope: Json = serde_json::from_str(&reply).unwrap();
        let save = envelope["save"].to_string();
        // from_bytes 只校验 format_version；engine 门在 restore 的 apply_save
        // （存档 JSON 的字段值即字面替换锚点）。
        let tampered = save.replace(
            &format!("\"engine_version\":\"{ENGINE_VERSION}\""),
            "\"engine_version\":\"0.0.0-foreign\"",
        );
        let outcome = KernelFacade::restore(tampered.as_bytes(), IDLE_MOD);
        assert!(
            matches!(
                outcome,
                Err(KernelError::EngineMismatch { ref saved, .. }) if saved == "0.0.0-foreign"
            ),
            "engine mismatch must be rejected (restore must not succeed)"
        );
    }

    // ---- TD1：结构化 meta 通道（回执 `meta` 键 / 瞬态 / 不进状态哈希）----

    /// 同一套状态写；`META` 版额外调 `ygg.meta`（瞬态回执面）——
    /// 两版状态哈希必须一致（meta 不入 canonical snapshot）。
    const PLAIN_MOD: &str = r#"
ygg.on("event.work", function()
  ygg.narrate("干活")
  local id = ygg.entity.spawn()
  ygg.entity.set_attribute(id, "hp", 7)
  ygg.kv.set("k", "v")
end)
"#;

    const META_MOD: &str = r#"
ygg.on("event.work", function()
  ygg.narrate("干活")
  local id = ygg.entity.spawn()
  ygg.entity.set_attribute(id, "hp", 7)
  ygg.kv.set("k", "v")
  ygg.meta("summary", {
    stage = "review",
    ending = false,
    pendingProposals = 2,
    conflicts = { { id = "r1", peer = "r2", note = "同一文件" } },
  })
end)

-- 嵌套 / 非 ASCII / 序列 / 空表
 ygg.on("event.shapes", function()
  ygg.meta("obj", { deep = { deeper = { 1, 2.5, true, "中文" } }, empty = {}, list = { "a", "b" } })
end)

-- 两个 meta 键是同一次 dispatch 里**两个独立槽位**（互不覆盖；Mod 侧不必写任何防御代码）
ygg.on("event.two", function()
  ygg.meta("tool_calls", { { id = "t1", tool = "git.head_short", args = { rev = "HEAD" } } })
  ygg.meta("summary", { stage = "review", nextEndsRun = false })
end)

-- 校验面：空键 / 超长键 / 非有限值都应在 Lua 侧 fail-fast（→ VALIDATION）
ygg.on("event.badkey", function()
  ygg.meta("", 1)
end)
ygg.on("event.longkey", function()
  ygg.meta("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", 1)
end)
ygg.on("event.nankey", function()
  ygg.meta("x", 0 / 0)
end)
"#;

    #[test]
    fn meta_lands_in_the_receipt_and_only_for_the_event_that_wrote_it() {
        let mut kernel = KernelFacade::new(META_MOD).expect("kernel construction");
        // 无 meta 的事件：信封不带 meta 键（线格式与旧版一致）
        let plain = reply(&kernel.dispatch("event.plain", "0"));
        assert!(
            plain.get("meta").is_none(),
            "未调 ygg.meta 的事件不得带 meta"
        );
        // 调了 ygg.meta 的事件：结构化字段原样进 meta
        let envelope = reply(&kernel.dispatch("event.work", "0"));
        assert_eq!(envelope["ok"], Json::Bool(true));
        assert_eq!(envelope["meta"]["summary"]["stage"], json!("review"));
        assert_eq!(envelope["meta"]["summary"]["pendingProposals"], json!(2));
        assert_eq!(
            envelope["meta"]["summary"]["conflicts"][0]["peer"],
            json!("r2")
        );
        // 叙事不含任何结构化残留（meta 与 narrative 完全分离）
        let narrative = envelope["narrative"].to_string();
        assert!(
            !narrative.contains("meta"),
            "叙事不得携带 meta：{narrative}"
        );

        // 瞬态：保留命令不产生 meta（上一条的 meta 不得残留）
        for command in ["state", "save", "log", "drain"] {
            let envelope = reply(&kernel.dispatch(command, ""));
            assert!(
                envelope.get("meta").is_none(),
                "保留命令 `{command}` 不得带 meta"
            );
        }
        // 且下一条事件也没继承上一条的 meta
        let again = reply(&kernel.dispatch("event.plain", "0"));
        assert!(again.get("meta").is_none(), "meta 不得跨 dispatch 残留");
    }

    #[test]
    fn meta_values_round_trip_as_json_structures() {
        let mut kernel = KernelFacade::new(META_MOD).expect("kernel construction");
        let envelope = reply(&kernel.dispatch("event.shapes", "0"));
        assert_eq!(
            envelope["meta"]["obj"],
            json!({
                "deep": {"deeper": [1, 2.5, true, "中文"]},
                // 空表只能读成空序列（Lua 无空对象字面形态，已在 host_api 声明）
                "empty": [],
                "list": ["a", "b"]
            })
        );
    }

    #[test]
    fn meta_keys_are_independent_slots_of_one_dispatch() {
        // `ygg.meta` 是 `BTreeMap<String, MetaValue>` 的按键插入：`summary` 与 `tool_calls` 分属
        // 两个槽位，信封拼装是一次性遍历，谁也不会覆盖谁。这条把「不用写防御代码」变成断言。
        let mut kernel = KernelFacade::new(META_MOD).expect("kernel construction");
        let envelope = reply(&kernel.dispatch("event.two", "0"));
        assert_eq!(envelope["meta"]["summary"]["stage"], json!("review"));
        assert_eq!(envelope["meta"]["summary"]["nextEndsRun"], json!(false));
        assert_eq!(envelope["meta"]["tool_calls"][0]["id"], json!("t1"));
        assert_eq!(
            envelope["meta"]["tool_calls"][0]["args"]["rev"],
            json!("HEAD")
        );
    }

    #[test]
    fn meta_key_and_value_validation_fails_fast() {
        let mut kernel = KernelFacade::new(META_MOD).expect("kernel construction");
        let empty_key = kernel.dispatch("event.badkey", "0");
        assert_eq!(reply_code(&empty_key), code::VALIDATION);
        assert!(
            empty_key.contains("ygg.meta key must not be empty"),
            "{empty_key}"
        );
        let long_key = kernel.dispatch("event.longkey", "0");
        assert_eq!(reply_code(&long_key), code::VALIDATION);
        assert!(long_key.contains("too long"), "{long_key}");
        let nan = kernel.dispatch("event.nankey", "0");
        assert_eq!(reply_code(&nan), code::VALIDATION);
        assert!(nan.contains("non-finite"), "{nan}");
        // 拒绝不污染后续：同进程再派发仍正常
        let envelope = reply(&kernel.dispatch("event.work", "0"));
        assert_eq!(envelope["ok"], Json::Bool(true));
        assert!(envelope["meta"]["summary"].is_object());
    }

    /// 确定性硬要求：同一事件序列，调不调 `ygg.meta` 状态哈希必须相同；
    /// meta 也不得进存档（`save` 的字节里没有它）。
    #[test]
    fn meta_never_enters_state_hash_or_save() {
        let mut plain = KernelFacade::new(PLAIN_MOD).expect("plain kernel");
        let mut with_meta = KernelFacade::new(META_MOD).expect("meta kernel");
        for _ in 0..3 {
            assert_eq!(
                reply(&plain.dispatch("event.work", "0"))["ok"],
                Json::Bool(true)
            );
            assert_eq!(
                reply(&with_meta.dispatch("event.work", "0"))["ok"],
                Json::Bool(true)
            );
        }
        assert_eq!(
            state_hash(&plain).expect("plain hash"),
            state_hash(&with_meta).expect("meta hash"),
            "meta 调用与否不得改变 canonical 状态哈希"
        );
        let save = with_meta.dispatch("save", "");
        assert!(!save.contains("\"meta\""), "meta 不得进存档：{save}");
        // 存档往返后哈希仍一致（瞬态面不参与持久化）
        let envelope = reply(&save);
        let restored = KernelFacade::restore(envelope["save"].to_string().as_bytes(), META_MOD)
            .expect("restore");
        assert_eq!(
            state_hash(&plain).expect("plain hash"),
            state_hash(&restored).expect("restored hash")
        );
    }

    // ---- dispatch 级 savepoint（实体面 + KV 面整体提交/回滚）----

    /// 半写 Mod：handler 先写实体（spawn + set_attribute）与 KV 再报错；
    /// `event.ok` 同形但不报错（提交对照组），`event.spawn` 暴露 next_id。
    const SAVEPOINT_MOD: &str = r#"
ygg.on("event.ok", function()
  ygg.kv.set("clean", "yes")
end)
ygg.on("event.bad", function()
  local id = ygg.entity.spawn()
  ygg.entity.set_attribute(id, "hp", 1)
  ygg.kv.set("dirty", "yes")
  error("boom")
end)
ygg.on("event.spawn", function()
  ygg.narrate("id=" .. tostring(ygg.entity.spawn()))
end)
"#;

    #[test]
    fn failed_dispatch_restores_the_canonical_baseline() {
        // 驱动内部状态机：取点冻结基线 → 失败整体还原（实体面与 KV 面）。
        let mut kernel = KernelFacade::new(SAVEPOINT_MOD).expect("kernel construction");
        // 先提交一个非空基线（否则「还原到基线」与「清空」不可区分）。
        let ok = reply(&kernel.dispatch("event.ok", "0"));
        assert_eq!(ok["ok"], Json::Bool(true));
        let baseline_bytes = kernel.canonical_state_bytes().expect("bytes");
        let baseline_save = reply(&kernel.dispatch("save", ""))["save"].to_string();
        assert_eq!(
            reply_code(&kernel.dispatch("event.bad", "0")),
            code::VALIDATION
        );
        // 实体面：canonical 字节逐字节回到基线（含 revision / next_entity_id）。
        assert_eq!(
            kernel.canonical_state_bytes().expect("bytes"),
            baseline_bytes
        );
        // KV 面不在 canonical 快照里（hash.rs 覆盖面声明），读 save 回执。
        assert_eq!(
            reply(&kernel.dispatch("save", ""))["save"].to_string(),
            baseline_save,
            "failed dispatch must not change the save bytes"
        );
        let state = reply(&kernel.dispatch("state", ""));
        assert_eq!(
            state["revision"],
            Json::from(0),
            "entity revision rolls back"
        );
        assert_eq!(
            state["kvRevision"],
            Json::from(1),
            "kv revision keeps commit"
        );
        // 提交路径未被改动：成功步的写入留在 canonical。
        let ok_again = reply(&kernel.dispatch("event.ok", "0"));
        assert_eq!(ok_again["ok"], Json::Bool(true));
        let save = reply(&kernel.dispatch("save", ""));
        assert_eq!(save["save"]["kv"]["entries"]["clean"], json!("yes"));
    }

    #[test]
    fn rollback_rewinds_next_entity_id() {
        // 回滚连 `next_entity_id` 一起退：失败步 spawn 掉的 id 不得被占用。
        let mut kernel = KernelFacade::new(SAVEPOINT_MOD).expect("kernel construction");
        assert_eq!(
            reply_code(&kernel.dispatch("event.bad", "0")),
            code::VALIDATION
        );
        let spawned = reply(&kernel.dispatch("event.spawn", "0"));
        assert_eq!(spawned["narrative"], json!(["id=1"]));
    }

    #[test]
    fn handler_reads_its_own_writes_during_the_dispatch() {
        // savepoint 基线是**副本**而非换出：转换期间 canonical 保留原状态 +
        // 本次写入，`ygg.kv.get` / `ygg.entity.get_attribute` 的读路径看到的
        // 是自己的写（若改成 mem::swap 换出，此测必红——读路径直读 canonical）。
        const READ_BACK_MOD: &str = r#"
ygg.on("event.rb", function()
  ygg.kv.set("k", "v")
  local id = ygg.entity.spawn()
  ygg.entity.set_attribute(id, "hp", 5)
  local bumped = ygg.entity.get_attribute(id, "hp") + 1
  ygg.entity.set_attribute(id, "hp", bumped)
  ygg.narrate("kv=" .. tostring(ygg.kv.get("k")) .. " hp=" .. tostring(ygg.entity.get_attribute(id, "hp")))
end)
"#;
        let mut kernel = KernelFacade::new(READ_BACK_MOD).expect("kernel construction");
        let out = reply(&kernel.dispatch("event.rb", "0"));
        assert_eq!(out["narrative"], json!(["kv=v hp=6"]));
        // 成功步提交后同样可读（canonical 就是写入目标）。
        let save = reply(&kernel.dispatch("save", ""));
        assert_eq!(save["save"]["kv"]["entries"]["k"], json!("v"));
        assert_eq!(
            save["save"]["entities"]["entities"]["1"]["attributes"]["hp"],
            json!(6)
        );
    }

    // ---- Part B：`ygg.fs` 确定性 VFS（写 / 读 / stat / list + savepoint 回滚）----

    /// VFS 全集 Mod：四个事件分别打写 / 读 / stat / list 面，叙事即断言载体。
    ///
    /// 沙箱裁掉了 `string` / `table` / `math`（`BASE_WHITELIST` 只留 base 纯计算子集），
    /// 所以这里只能用 `#` / `..` / `tostring` / `type`——别顺手写 `string.len`。
    const VFS_MOD: &str = r#"
ygg.on("event.write", function()
  local b = ygg.fs.write("artifacts/b.md", "second")
  local a = ygg.fs.write("artifacts/a.md", "first")
  local sb, sa = b.sha256, a.sha256
  ygg.narrate("w=" .. b.path .. ":" .. tostring(b.bytes) .. ":" .. tostring(#sb))
  ygg.narrate("w=" .. a.path .. ":" .. tostring(a.bytes) .. ":" .. tostring(#sa))
end)
ygg.on("event.read", function()
  ygg.narrate("hit=" .. tostring(ygg.fs.read("artifacts/a.md")))
  ygg.narrate("miss=" .. tostring(ygg.fs.read("artifacts/none.md")))
  ygg.narrate("misstype=" .. type(ygg.fs.read("artifacts/none.md")))
end)
ygg.on("event.stat", function()
  local hit = ygg.fs.stat("artifacts/a.md")
  local miss = ygg.fs.stat("artifacts/none.md")
  local hs, ms = hit.sha256, miss.sha256
  ygg.narrate("hit=" .. tostring(hit.exists) .. ":" .. tostring(hit.bytes)
    .. ":" .. tostring(#hs))
  ygg.narrate("miss=" .. tostring(miss.exists) .. ":" .. tostring(miss.bytes)
    .. ":" .. tostring(#ms) .. ":" .. type(miss))
end)
ygg.on("event.list", function()
  local rows = ygg.fs.list()
  local acc = ""
  for i = 1, #rows do
    local sha = rows[i].sha256
    if i > 1 then acc = acc .. "," end
    acc = acc .. rows[i].path .. "@" .. tostring(rows[i].bytes) .. ":" .. tostring(#sha)
  end
  ygg.narrate("list=" .. tostring(#rows) .. ":" .. acc)
end)
"#;

    #[test]
    fn fs_write_read_stat_list_roundtrip() {
        let mut kernel = KernelFacade::new(VFS_MOD).expect("kernel construction");
        let written = reply(&kernel.dispatch("event.write", "0"));
        assert_eq!(written["ok"], Json::Bool(true), "envelope={written}");
        // write 回执：path / bytes / sha256（64 字符）——长度在叙事里断言。
        assert_eq!(
            written["narrative"],
            json!(["w=artifacts/b.md:6:64", "w=artifacts/a.md:5:64"])
        );

        // read：命中回原文，miss 回 nil（不 throw）。
        let read = reply(&kernel.dispatch("event.read", "0"));
        assert_eq!(
            read["narrative"],
            json!(["hit=first", "miss=nil", "misstype=nil"])
        );

        // stat：**恒回表**（miss 也是表，字段齐全）。
        let stat = reply(&kernel.dispatch("event.stat", "0"));
        assert_eq!(
            stat["narrative"],
            json!(["hit=true:5:64", "miss=false:0:0:table"])
        );

        // list：稠密数组（`#rows` 可数），按 path 升序。
        let list = reply(&kernel.dispatch("event.list", "0"));
        assert_eq!(
            list["narrative"],
            json!(["list=2:artifacts/a.md@5:64,artifacts/b.md@6:64"])
        );
    }

    /// 同序列两次 → 同字节（确定性锤子：`BTreeMap` 序 + `sha2` 确定性）。
    #[test]
    fn fs_list_is_deterministic_across_repeated_dispatch() {
        let mut first = KernelFacade::new(VFS_MOD).expect("kernel construction");
        let mut second = KernelFacade::new(VFS_MOD).expect("kernel construction");
        for kernel in [&mut first, &mut second] {
            reply(&kernel.dispatch("event.write", "0"));
            reply(&kernel.dispatch("event.list", "0"));
        }
        let a = reply(&first.dispatch("event.list", "0"));
        let b = reply(&second.dispatch("event.list", "0"));
        assert_eq!(a["narrative"], b["narrative"], "list 回执必须同字节");
        assert_eq!(first.vfs_entries(), second.vfs_entries());
        assert_eq!(first.vfs_revision(), second.vfs_revision());
    }

    /// 成功派发：写入留在 VFS 且入 dirty 水位；排空后二次排空为空。
    #[test]
    fn successful_dispatch_keeps_vfs_and_marks_dirty() {
        let mut kernel = KernelFacade::new(VFS_MOD).expect("kernel construction");
        reply(&kernel.dispatch("event.write", "0"));
        assert_eq!(kernel.vfs_len(), 2);
        assert_eq!(kernel.vfs_revision(), 2, "两次成功 put 各 +1");

        let dirty = kernel.vfs_take_dirty();
        assert_eq!(
            dirty.iter().map(|(p, _, _)| p.as_str()).collect::<Vec<_>>(),
            vec!["artifacts/a.md", "artifacts/b.md"],
            "dirty 必须按 BTreeMap 序排空"
        );
        assert_eq!(dirty[0].1, b"first".to_vec(), "带出的是内容字节");
        assert!(kernel.vfs_take_dirty().is_empty(), "排空后不得重复上报");
        assert_eq!(kernel.vfs_len(), 2, "排空 dirty 不删文件");
    }

    /// **savepoint 回滚的 VFS 面**：失败步里写的文件必须一起回滚，且不得入
    /// dirty（否则一次失败会在下一轮把半个状态落盘）。
    #[test]
    fn failed_dispatch_rolls_back_vfs() {
        const HALF_WRITE_MOD: &str = r#"
ygg.on("event.ok", function()
  ygg.fs.write("artifacts/clean.md", "keep")
end)
ygg.on("event.bad", function()
  ygg.fs.write("artifacts/dirty.md", "leak")
  error("boom")
end)
"#;
        let mut kernel = KernelFacade::new(HALF_WRITE_MOD).expect("kernel construction");
        // 先提交一个非空基线（否则「回滚到基线」与「清空」不可区分）。
        let ok = reply(&kernel.dispatch("event.ok", "0"));
        assert_eq!(ok["ok"], Json::Bool(true));
        let _ = kernel.vfs_take_dirty(); // 排空基线写入，只观察失败步的增量
        let baseline_revision = kernel.vfs_revision();

        assert_eq!(
            reply_code(&kernel.dispatch("event.bad", "0")),
            code::VALIDATION
        );
        // 文件面：失败步写的键不在表里，基线件还在。
        assert_eq!(kernel.vfs_len(), 1, "失败步的文件不得留下");
        assert_eq!(
            kernel
                .vfs_entries()
                .iter()
                .map(|(path, _, _)| path.as_str())
                .collect::<Vec<_>>(),
            vec!["artifacts/clean.md"]
        );
        // 水位面：dirty 与 revision 一起回滚。
        assert!(
            kernel.vfs_take_dirty().is_empty(),
            "失败步不得留下待落盘条目"
        );
        assert_eq!(
            kernel.vfs_revision(),
            baseline_revision,
            "revision 随基线回滚"
        );
    }

    /// 路径 / 内容门在 Lua 调用点 fail-fast（`ygg.fs.write` 当场报错，
    /// 不进 `files`、不动 revision）——与 `ygg_store` 的端口门同规则。
    #[test]
    fn fs_write_path_gate_rejects_at_the_call_site() {
        let mut kernel = KernelFacade::new(IDLE_MOD).expect("kernel construction");
        for bad in [
            "artifact/x.md",
            "artifacts/",
            "artifacts//x",
            "artifacts/../x",
            "/artifacts/x",
            "artifacts/a b.md",
            "artifacts/a|b.md",
            "artifacts/a@b.md",
        ] {
            let err = kernel
                .vfs_put(bad, b"x")
                .expect_err(&format!("`{bad}` 必须被拒"));
            assert!(err.contains("ygg.fs.write 拒绝："), "必须点名写面：{err}");
        }
        // 内容上限（256 KiB）：恰好上限放行，多一字节即拒。
        let exactly = vec![0u8; 256 * 1024];
        assert!(kernel.vfs_put("artifacts/max.md", &exactly).is_ok());
        let over = vec![0u8; 256 * 1024 + 1];
        assert!(kernel.vfs_put("artifacts/over.md", &over).is_err());
        // 被拒的写不留下任何痕迹（revision 只记成功的两次）。
        assert_eq!(kernel.vfs_len(), 1);
        assert_eq!(kernel.vfs_revision(), 1);
    }

    /// 宿主预载：`vfs_load` **不**入 dirty、**不**动 revision（不是本会话的写）。
    #[test]
    fn vfs_load_does_not_dirty_or_bump_revision() {
        let mut kernel = KernelFacade::new(IDLE_MOD).expect("kernel construction");
        kernel.vfs_load("artifacts/pre.md", b"preset", "deadbeef");
        assert_eq!(kernel.vfs_len(), 1);
        assert_eq!(kernel.vfs_revision(), 0, "预载不得当成本次写入");
        assert!(kernel.vfs_take_dirty().is_empty(), "预载不得入 dirty");
        // 预载件参与 list / 诊断面。
        assert_eq!(
            kernel.vfs_entries()[0],
            (
                "artifacts/pre.md".to_owned(),
                b"preset".to_vec(),
                "deadbeef".to_owned()
            )
        );
    }

    /// 多文件 Mod 支持面：`ModuleSet` 由 `(规范名, 源码)` 构造——名字是点分
    /// 规范名（`src.writer`），不是路径（`src/writer.lua` 由 `path_of` 派生）。
    /// 下面两个用例共用。
    fn module_set(pairs: &[(&str, &str)]) -> ModuleSet {
        ModuleSet::from_pairs(
            pairs
                .iter()
                .map(|(name, code)| ((*name).to_owned(), code.as_bytes().to_vec())),
        )
        .expect("登记模块")
    }

    /// **模块顶层禁令**：`ygg.fs.write` 与 `ygg.kv.set` 同类（写宿主状态），
    /// 模块 chunk 顶层就被影子表拒绍，报错带调用点与合法写法。
    #[test]
    fn fs_write_is_banned_at_module_top_level() {
        let modules = module_set(&[("src.writer", "ygg.fs.write(\"artifacts/x.md\", \"y\")\n")]);
        let err = match KernelFacade::with_modules("require(\"src.writer\")", modules) {
            Ok(_) => panic!("模块顶层写 ygg.fs.write 必须被拒"),
            Err(err) => err,
        };
        let text = err.to_string();
        for fragment in ["ygg.fs.write", "src/writer.lua", "模块顶层"] {
            assert!(
                text.contains(fragment),
                "错误文本缺少「{fragment}」：{text}"
            );
        }
    }

    /// 纯读面（`fs.read` / `fs.stat` / `fs.list`）在模块顶层**合法**——
    /// 禁令只压写面，否则「模块顶层读一下自己上次的产物」这种正当用法会被误杀。
    #[test]
    fn fs_reads_are_allowed_at_module_top_level() {
        let modules = module_set(&[(
            "src.reader",
            "local v = ygg.fs.read(\"artifacts/x.md\")\n\
             local s = ygg.fs.stat(\"artifacts/x.md\")\n\
             return { n = #ygg.fs.list(), nil_read = (v == nil), exists = s.exists }\n",
        )]);
        let mut kernel = KernelFacade::with_modules(
            "local r = require(\"src.reader\")\n\
             ygg.on(\"event.peek\", function()\n\
               ygg.narrate(\"n=\" .. tostring(r.n) .. \" nil=\" .. tostring(r.nil_read) .. \" exists=\" .. tostring(r.exists))\n\
             end)",
            modules,
        )
        .expect("纯读面必须放行");
        let out = reply(&kernel.dispatch("event.peek", "0"));
        assert_eq!(out["narrative"], json!(["n=0 nil=true exists=false"]));
    }
}
