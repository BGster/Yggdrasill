//! S10 KernelFacade：装载/转换接线的单一驱动面（JSON 信封 API，镜像 A1
//! 线格式——未来 wasm 壳 JS 侧仅换 loader，语义面不变）。
//!
//! - **装载序**（S10 锁定，S11 扩展资源步，r1 扩展模块表；管线单一收口于
//!   `build_kernel`）：`Sandbox::new`（stdlib 裁剪 + 内存上限 + 白名单环境）
//!   → `inject::install`（math 精确集 + 内核种子
//!   random + ticks；原序中的正式燃料安装步已随 S34 燃料退役移除）→ `host_api::install`（装载期 `ygg` 完整面）→
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
//! - **一字宙一代决（M1-D）**：世界状态只有一份真身——`Sandbox` 持有的
//!   内核槽（`Rc<RefCell<Option<Box<Sim>>>>`，`world.*` 面与该槽共享）。
//!   旧的第二套实体模型（`ygg-core` 行存）连同它的事务形参一起删除后，
//!   `run_transition` 不再接收任何 store：handler 经 host 桥的即席状态写
//!   直接落在 canonical 内核上，envelope 的 `revision` 改为**提交序号**
//!   （世界时钟归内核 tick，两者语义不同：每 tick 推进一次，每提交一次）。
//!   读档 = 内核帧整体恢复（`apply_save`），后续 handler 写经同一 `Rc` 即时可见。
//! - **dispatch 原子性（r1 savepoint）**：一次 `cmd_event` 就是一个事务
//!   边界——取点在「校验全过后、转换开始前」，冻结**世界帧字节**（`Sim`
//!   快照，含 tick / 实体 / rng 流）+ `kv` 的基线副本；转换成功 = 提交
//!   （丢弃基线），转换失败 = **先整体还原 canonical、再返回错误信封**。
//!   基线必须是 canonical 的**副本**而非换出：`ygg.kv.get` / `world.*`
//!   的读路径直读 canonical，换出会让转换期的 handler 读到空世界。
//!   帧恢复是字节级回退，所以失败步连 tick 与内核 rng 流一并回退
//!   （比旧的实体面 clone 更严）；不覆盖的旁路面：`external_log` 照记
//!   失败步（它是尝试日志，也是回放前提）、`warnings`/`meta` 为瞬态
//!   回执面、Lua VM 全局与模块 upvalue、`math.random` 均不回滚
//!   （要跨步记忆只能落 KV）；panic 不在回滚覆盖内（进程已不可继续）。
//!   core `run_transition` 的「无事务语义」契约不变——它不持有状态所有权，
//!   也不接收状态形参，事务边界归调用方（即本驱动）。
//! - **JSON 信封（A1）**：[`KernelFacade::dispatch`] 是唯一同步入口，
//!   成功 = `{"ok":true,...}`，域错误 = `{"ok":false,"error":{code,message}}`
//!   ——**域错误永不 throw**（Rust 侧返回 String 信封）；只有内部编程
//!   错误（重入、NotExternal 这类不可能路径）落 [`code::INTERNAL`]。
//!   **错误信封的状态语义**：`{"ok":false,...}` ⇒ 本次 dispatch 对
//!   canonical 状态（世界帧 + KV + 提交序号）**零影响**（savepoint 已整体还原）；信封**不加**任何新字段（线格式零变更），下游据
//!   `ok:false` 即可判定「本次事件无效」。旁路面例外见上条。
//!   加性可选键 `warnings`（D1）：仅当 handler 在 `ygg.choice` 之前产生过
//!   `ygg.emit`（将被语义丢弃）时出现，单元素字符串数组；serde_json 字典序
//!   键序下确定性不变（同输入 → 同警告 → 同字节）。
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
use std::rc::Rc;

use serde_json::Value as Json;

use ygg_core::bus::{HandlerRegistry, PendingChoice};
use ygg_core::kv::KvStore;
use ygg_core::save::SaveFile;

use crate::host_api::HostApi;
use crate::inject::DeterminismHandle;
use crate::resources::{ResourceError, ResourceTable};
use crate::sandbox::{Sandbox, ENGINE_VERSION};

// —— 模块装配：按子系统拆文件，纯搬移（S-25 批六）——
mod build;
mod dispatch;
mod error;
mod surface;

#[cfg(test)]
mod tests;

pub use error::{code, KernelError};

/// 内核驱动门面：Mod 装载 + 事件转换 + 存读档 + 诊断面的单一入口
/// （native 探针/测试驱动形态；wasm 壳落地后语义面不变）。
pub struct KernelFacade {
    /// Lua 状态持有者（字段存活期 = VM 存活期；运行期经 host 桥间接使用）。
    ///
    /// 同时是**时间的唯一驱动面**： [`Sandbox::step`] / [`Sandbox::snapshot`]
    /// / [`Sandbox::restore`] 都挂在这里，Lua 侧没有推进时钟的入口。
    sandbox: Sandbox,
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
    /// canonical 持久 KV（同上）。
    kv: Rc<RefCell<KvStore>>,
    /// 提交序号（每次**成功的**事件转换 +1，失败步由 savepoint 回退）。
    ///
    /// 与内核 tick 是两根不同的钟：tick 由宿主 [`Sandbox::step`] 推进，
    /// 提交序号由 dispatch 推进。信封的 `revision` 取后者（旧行存时代的
    /// 实体表版本号已随行存删除）。
    revision: u64,
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

impl KernelFacade {
    /// 状态面覆写（读档语义）：engine_version 门（不符 =
    /// [`KernelError::EngineMismatch`]，存档由异版引擎写出即拒绝）后整体
    /// 覆写世界帧 / KV / pending_choice（覆写经驱动持有的同一 `Rc`，后续
    /// handler 写即时可见——一字宙代决）。
    ///
    /// 帧恢复是**先校验后改**：坏帧（长度/magic/尾 digest 不符）不产生
    /// 半改的世界，且此时 KV 尚未动——错误信封的状态语义因此逐字成立。
    fn apply_save(&mut self, save: &SaveFile) -> Result<(), KernelError> {
        if save.engine_version() != ENGINE_VERSION {
            return Err(KernelError::EngineMismatch {
                saved: save.engine_version().to_owned(),
                current: ENGINE_VERSION.to_owned(),
            });
        }
        self.sandbox.restore(save.world())?;
        *self.kv.borrow_mut() = save.kv().clone();
        self.pending_choice = save.pending_choice().cloned();
        Ok(())
    }
}
