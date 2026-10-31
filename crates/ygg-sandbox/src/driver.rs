//! S10 KernelFacade：装载/转换接线的单一驱动面（JSON 信封 API，镜像 A1
//! 线格式——未来 wasm 壳 JS 侧仅换 loader，语义面不变）。
//!
//! - **装载序**（S10 锁定，S11 扩展资源步；管线单一收口于 [`build_kernel`]）：
//!   `Sandbox::new`（stdlib 裁剪 + 内存上限 + 白名单环境）→ `fuel::install`
//!   （正式燃料，每 dispatch 面重置 [`DEFAULT_FUEL_BUDGET`]）→
//!   `inject::install`（math 精确集 + 内核种子 random + ticks）→
//!   `host_api::install`（装载期 `ygg` 完整面）→ `resources::install`
//!   （S11：`ygg.data` 只读数据面——host 之后、stdlib 之前、Mod chunk 之前，
//!   Mod 顶层即可读资源）→ `stdlib::install`（S10 stdlib：先于 Mod chunk，
//!   注册序即派发序）→ Mod chunk（失败 = [`KernelError::ModLoad`]）→
//!   `host.seal`（运行期注册禁令收口）→ `register_into(&mut registry, "mod")`
//!   （ygg.on 调用序 = 注册序 = 派发序）。
//! - **统一 store 代决（S10）**：`entities` / `kv` 为 `Rc<RefCell<..>>`
//!   单实例——handler 经 host 桥的即席状态写**直接落在 canonical store**；
//!   转换输入侧以快照克隆（snapshot-out，见 `cmd_event`）喂给
//!   `run_transition` 满足其 `&mut EntityStore` 形参，转换残余的
//!   `TransitionResult::revision_after` 为冻结诊断值，信封 revision 一律
//!   **转换结束后从 canonical store 现读**。读档 = canonical store 整体
//!   覆写（`apply_save`），后续 handler 写经同一 `Rc` 即时可见。
//! - **JSON 信封（A1）**：[`KernelFacade::dispatch`] 是唯一同步入口，
//!   成功 = `{"ok":true,...}`，域错误 = `{"ok":false,"error":{code,message}}`
//!   ——**域错误永不 throw**（Rust 侧返回 String 信封）；只有内部编程
//!   错误（重入、NotExternal 这类不可能路径）落 [`code::INTERNAL`]。
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
use crate::host_api::{self, HostApi, HostHandles};
use crate::inject::{self, DeterminismHandle};
use crate::resources::{ResourceError, ResourceTable};
use crate::sandbox::{Sandbox, ENGINE_VERSION};
use crate::stdlib;

/// 每次 dispatch 事件转换的燃料预算（转换边界全额重置，D-B1 每转换共享）。
pub const DEFAULT_FUEL_BUDGET: u64 = 100_000;

/// 信封错误码（稳定字符串契约，A1 线格式；下游按码分支不解析 message）。
pub mod code {
    /// 燃料耗尽（内核派发税或 VM 指令计量任一面触发）。
    pub const FUEL_EXHAUSTED: &str = "FUEL_EXHAUSTED";
    /// 沙箱拒绝（模块装载拒绝、内存超限等沙箱结构面拒绝）。
    pub const SANDBOX_DENIED: &str = "SANDBOX_DENIED";
    /// 校验拒绝（payload 非标量、事件构造失败、handler 报错、存档版本/内容不符）。
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
}

/// 装载序单一管线（S10 锁定序 + S11 资源步；`new` / `with_resources` 共享）。
///
/// `resources = Some(table)` 时在 host API 之后、stdlib 之前安装 `ygg.data`；
/// `None` 时跳过该步——`KernelFacade::new` 全序行为与 S10 逐字节一致
/// （S10 测试不加改动保持全绿即本函数的回归证据）。
fn build_kernel(
    mod_source: &str,
    resources: Option<&ResourceTable>,
) -> Result<KernelParts, KernelError> {
    // ① 沙箱构造：stdlib 裁剪 + 内存上限 + 白名单环境（ygg 占位表就位）。
    let sandbox = Sandbox::new().map_err(KernelError::Sandbox)?;
    let lua = sandbox.lua().clone();
    // ② 正式燃料：两面共享预算，注入 hook（转换边界 reset）。
    let fuel_meter = fuel::install(&lua, DEFAULT_FUEL_BUDGET).map_err(KernelError::Sandbox)?;
    // ③ 确定性注入：math 精确集 + 内核种子 random + ygg.ticks。
    let determinism = inject::install(&lua).map_err(KernelError::Sandbox)?;
    // ④ canonical store 先建：host 桥句柄与驱动共享同一 Rc（统一 store）。
    let entities = Rc::new(RefCell::new(EntityStore::new()));
    let kv = Rc::new(RefCell::new(KvStore::new()));
    // ⑤ host API：装载期 ygg 完整面（on/emit/narrate/choice/kv/entity）。
    let host = host_api::install(
        &lua,
        HostHandles {
            kv: Rc::clone(&kv),
            entities: Rc::clone(&entities),
        },
    )
    .map_err(KernelError::Sandbox)?;
    // ⑤' S11 资源步：ygg.data 只读数据面（host 之后、stdlib 之前、
    // Mod chunk 之前——Mod 顶层即可读资源）。
    if let Some(table) = resources {
        crate::resources::install(&lua, table).map_err(KernelError::Sandbox)?;
    }
    // ⑥ stdlib：命名空间 + fsm + narrative（先于 Mod chunk，注册序即派发序）。
    stdlib::install(&lua).map_err(KernelError::Sandbox)?;
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
        let parts = build_kernel(mod_source, None)?;
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
        let parts = build_kernel(mod_source, Some(&table))?;
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
            "snapshotBytes": snapshot_bytes.len(),
            "pendingChoice": self.pending_choice.as_ref().map(choice_to_json),
        })
        .to_string()
    }

    /// `log`：外部事件日志（仅外部事件；驱动旁路面，不入档）。
    fn cmd_log(&self) -> String {
        json!({"ok": true, "log": self.external_log}).to_string()
    }

    /// 事件转换路径（非保留名命令）：payload 解析（标量门）→ 外部事件
    /// 构造 → 外部日志 → 燃料重置 → snapshot-out 转换 → 信封回包。
    fn cmd_event(&mut self, name: &str, payload_json: &str) -> String {
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
        // ④ 燃料预算每转换重置（D-B1：转换内级联共享同一预算）。
        self.fuel_meter.reset(DEFAULT_FUEL_BUDGET);
        // ⑤ snapshot-out：以 canonical 快照克隆喂 run_transition（其
        //    `&mut EntityStore` 形参只作 revision 记账）；canonical RefCell
        //    在转换期间不被借用——handler 即席写经 host 桥 `Rc` 直接落在
        //    canonical store（统一 store 代决）。借用为语句级临时
        //    `Ref`，语句结束即释放，不跨越 run_transition 存活。
        let mut working = self.entities.borrow().clone();
        let outcome = run_transition(
            &mut working,
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
        // ⑥ 裁定：成功更新 pending_choice；错误映射稳定码信封。
        let result = match outcome {
            Ok(result) => result,
            Err(err) => return map_transition_error(err),
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
            if msg.contains("fuel exhausted") {
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

    /// 空转 Mod：无任何注册——事件无 handler，转换空跑即成功。
    const IDLE_MOD: &str = "-- idle mod: no handlers";

    #[test]
    fn envelope_error_shape_uses_stable_codes() {
        let mut kernel = KernelFacade::new(IDLE_MOD).expect("kernel construction");
        // 非标量 payload → VALIDATION。
        let reply = kernel.dispatch("event.x", r#"{"nested":{"deep":1}}"#);
        assert_eq!(reply_code(&reply), code::VALIDATION);
        // 保留命令名 + 非法存档 → VALIDATION。
        let reply = kernel.dispatch("load", "not json");
        assert_eq!(reply_code(&reply), code::VALIDATION);
        // 成功信封无 error 字段。
        let reply = kernel.dispatch("event.any", "0");
        let parsed: Json = serde_json::from_str(&reply).unwrap();
        assert_eq!(parsed["ok"], Json::Bool(true));
        assert!(parsed.get("error").is_none());
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
}
