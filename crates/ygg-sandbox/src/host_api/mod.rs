//! S9 host API（D-S9-1..D-S9-4）：`ygg.*` 完整面 + 收集-回放桥 + 装载/运行期分相。
//!
//! - **生命周期与分相**（D-S9-1）：[`install`] 在**装载期**安装完整 `ygg` 面
//!   （`on / emit / narrate / choice / kv.* / entity.*`）；[`HostApi::seal`]
//!   在装载结束后把 `ygg.on` **原位替换**为可读报错桩——运行期注册禁令
//!   （D-B4 的运行时侧权威执行面）。无 mlua Scope、无线程局部存储或静态可变
//!   全局：全部共享状态经 `Rc<RefCell<HostState>>` 单线程持有。
//! - **收集-回放桥**（D-S9-2）：`ygg.emit / narrate / choice` 在 Lua 执行期间
//!   只向 [`HostState::calls`] **按序追加** [`HostCall`]；LuaHandler
//!   （core `EventHandler` 反转面实现）在每次 handler 调用前清空调用缓冲，
//!   以事件 payload（core `Value` → mlua `Value` marshal）调用装载期存下的
//!   Lua 函数，随后把缓冲**按原序回放**进 sink（emit→`sink.emit`、
//!   narrate→`sink.narrate`、choose→`sink.choose`）。
//!   **等价性论证**：回放序 = 追加序 = Lua 执行序，故 emit / narrate / choose
//!   三类输出的**相对顺序**在桥前后逐项保真——downstream handler 观察到的
//!   FIFO 序、叙述序与 pending_choice 与「直接直通 sink」的实现逐项一致。
//!   KV / 实体操作是**有副作用的即席状态写**，不走回放：经 `Rc<RefCell>`
//!   立即调用 core 把关方法，副作用时点 = Lua 执行时点（其相对输出序在
//!   handler 内部由构造保证；门拒绝如非有限值以可读 mlua 错误浮现）。
//! - **sentinel 机制**（D-S9-3）：`ygg.choice` 记录 `Choose` 后以
//!   install 时从内核种子派生的 **per-kernel sentinel**（[`HostState::sentinel`]，
//!   不可预测 token）错误终止当前 Lua 执行（后续代码不得运行——
//!   choice-as-state：挂起即数据）；桥把首行恰好等于 sentinel 的 mlua 错误识别为
//!   **正常终止**（继续回放缓冲），其余错误包成 core [`HandlerError`]。
//!   Mod 面无法预测该 token（错误伪造只会成为普通错误浮出，不会被哨兵吞掉）。
//! - **choice 前置输出丢弃的运行期警告**（M0 迭代 D1）：`ygg.choice` 被调用时
//!   统计调用缓冲中已有的 `Emit` 条目（这些派生事件将因 `pending_choice`
//!   终结清空 FIFO 而被语义丢弃）；replay 后若计数非零，记一条人读警告到
//!   [`HostState::warning`]（每次 dispatch 至多一条，首个胜出，由驱动经
//!   [`HostApi::take_dispatch_warning`] drain 进信封加性 `warnings` 键）。
//!   只计 `Emit` 不计 `Narrate`：narrate 直进叙述缓冲、不随 FIFO 丢弃——
//!   把它计入警告会与信封 `narrative` 字段自相矛盾。静态 ygg-lint 仍挂账 M1。
//! - **注册序**（D-S9-4）：`ygg.on` 调用序 = `Vec` 追加序；
//!   [`HostApi::register_into`] 在 seal 后按该序逐条
//!   `HandlerRegistry::register`（每条一个 LuaHandler），注册序即派发序。
//! - **结构化 meta 通道已删除**（TD1 退役）：本 crate 不再有回执 meta 面。
//! - **payload 双向 marshal**：Integer↔`Int`、Number↔`Float`、String↔`Str`、
//!   Boolean↔`Bool`；Lua→core 遇 NaN/±∞ 由 core 门拒绝（可读 mlua 错误）、
//!   `-0.0` 由 core 规范化；`nil` 仅在 get 类miss 上作为 Lua `nil` 返回；
//!   table / function / 其他类型 → 可读 `unsupported payload type: {lua_type}`。
//!   `ygg.choice` 的 options 必须是字符串的 Lua 序列，否则可读报错。

use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Error, Function, Lua, Table};
use ygg_core::bus::{HandlerRegistry, PendingChoice};
use ygg_core::entity::{EntityStore, Value};
use ygg_core::kv::KvStore;

use crate::inject::splitmix64;

// ---- 模块化拆分 shim(S-25 批三 C1:纯搬移;既有路径逐字节不变)----
mod entity;
mod entity_decode;
mod kv;
mod narrate;
mod selector;
mod sim;
mod value;
mod world;

pub use entity::*;
pub use entity_decode::*;
use kv::*;
use narrate::*;
pub use selector::*;
pub use sim::*;
use value::*;
use world::install_world;
pub use world::*;

/// 宿主相位：`ygg.sim.*` 仅在转换期（`Phase::Transition`）可写。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Loading,
    Idle,
    Transition,
}

/// 宿主侧状态句柄（S9 测试 / S10 驱动的注入面）。
pub struct HostHandles {
    pub kv: Rc<RefCell<KvStore>>,
    pub entities: Rc<RefCell<EntityStore>>,
}

impl HostHandles {
    /// 构造句柄（供需要同时挂载或替换工具执行器的宿主使用）。
    pub fn new(kv: Rc<RefCell<KvStore>>, entities: Rc<RefCell<EntityStore>>) -> Self {
        Self { kv, entities }
    }
}

/// 单条待回放的 host 调用（D-S9-2 收集面的载体，按追加序回放）。
pub enum HostCall {
    /// `ygg.emit(name, payload)` → 回放为 `sink.emit(Event::derived(..))`。
    Emit { name: String, payload: Value },
    /// `ygg.narrate(line)` → 回放为 `sink.narrate(line)`。
    Narrate(String),
    /// `ygg.choice(prompt, options)` → 回放为 `sink.choose(..)`（first-wins 在驱动层）。
    Choose(PendingChoice),
}

/// 桥共享状态：注册序缓冲 + 调用缓冲 + 宿主句柄 + 相位标志 + per-kernel
/// choice sentinel（D-S9-1 / D-S9-3）。
pub struct HostState {
    registered: Vec<(String, Function)>,
    calls: Vec<HostCall>,
    handles: HostHandles,
    /// 宿主侧模拟内核实例（Tier 1；未注入时 `None`，`ygg.sim.*` 全部抛 `YGG-SIM-UNMOUNTED`）。
    ///
    /// **不在 `HostHandles` 里**：句柄是外部构造点，加字段会破
    /// `crates/ygg-sandbox/tests` 与 stdlib 的既有构造；Sim 只是 `install` 自建的空槽。
    sim: Rc<RefCell<Option<Box<ygg_sim::Sim>>>>,
    /// 当前执行相位（装载期 `Loading`，封口后待机 `Idle`，handler 执行期间 `Transition`）。
    phase: Phase,
    sealed: bool,
    sentinel: String,
    /// 本次 handler 调用中，`ygg.choice` 之前已入缓冲的 `Emit` 条目数
    /// （D1：这些条目将被 pending_choice 终结丢弃；每 handler 调用重置）。
    pre_choice_emits: u64,
    /// 本次 dispatch 的人读警告（D1；至多一条，首个胜出，驱动 drain 后清空）。
    warning: Option<String>,
    /// `world` 面的宿主侧状态（kind 表 + 日志环；M1-C）。
    world: WorldState,
}

/// 宿主 API 的 Rust 侧控制柄：seal / 注册进注册表。
pub struct HostApi {
    lua: Lua,
    state: Rc<RefCell<HostState>>,
}

pub fn install(lua: &Lua, handles: HostHandles) -> Result<HostApi, Error> {
    let state = Rc::new(RefCell::new(HostState {
        registered: Vec::new(),
        calls: Vec::new(),
        handles,
        sim: Rc::new(RefCell::new(None)),
        phase: Phase::Loading,
        sealed: false,
        // per-kernel sentinel：内核种子派生（splitmix64 四轮），Mod 面不可预测
        //（S9 审计代决：伪造首行只会成为普通错误浮出，不会被哨兵吞掉）。
        sentinel: {
            let mut s = crate::inject::KERNEL_RNG_SEED ^ 0x5345_4E54_5E4C_u64;
            let mut token = String::from("\u{1}__ygg_choice_");
            for _ in 0..4 {
                token.push_str(&format!("{:016x}", splitmix64(&mut s)));
            }
            token.push_str("__\u{1}");
            token
        },
        pre_choice_emits: 0,
        warning: None,
        world: WorldState::default(),
    }));

    let ygg: Table = lua.globals().raw_get("ygg")?;

    install_narrate(lua, &state, &ygg)?;
    install_kv(lua, &state, &ygg)?;
    install_entity(lua, &state, &ygg)?;

    install_sim(lua, &state)?;
    // `ygg.codec.*`（Tier 2 pkg.codec 的原生加速面；契约 lua-api.md §3.1.4）。
    // 无状态、无实例，所以没有 `set_*` 注入面，也不需要 CLI 激活。
    crate::codec_face::install(lua)?;
    // `world.*`（M1-C）：须在 `ygg.kv` 之后——`world.kv` 直接复用那张表。
    install_world(lua, &state)?;

    Ok(HostApi {
        lua: lua.clone(),
        state,
    })
}

impl HostApi {
    /// 分相收口（D-S9-1）：置 sealed 标志并把 `ygg.on` 原位替换为可读报错桩
    /// （运行期注册禁令）。重复 seal 为可读错误。
    pub fn seal(&self, lua: &Lua) -> Result<(), Error> {
        {
            let mut s = self.state.borrow_mut();
            if s.sealed {
                return Err(Error::runtime("host api is already sealed"));
            }
            s.sealed = true;
            s.phase = Phase::Idle;
        }
        let ygg: Table = lua.globals().raw_get("ygg")?;
        let stub = lua.create_function(|_, _: (String, Function)| -> Result<(), Error> {
            Err(Error::runtime(SEALED_MESSAGE))
        })?;
        ygg.raw_set("on", stub)?;
        Ok(())
    }

    /// 把装载期收集的注册按 `ygg.on` 调用序逐条装入注册表（D-S9-4；
    /// 每条一个 [`LuaHandler`]，注册序 = 派发序）。在 seal 之后调用。
    pub fn register_into(&self, registry: &mut HandlerRegistry, mod_id: &str) {
        let entries: Vec<(String, Function)> = self.state.borrow().registered.clone();
        for (event_name, function) in entries {
            registry.register(
                mod_id,
                event_name.clone(),
                Box::new(LuaHandler {
                    lua: self.lua.clone(),
                    function,
                    state: Rc::clone(&self.state),
                }),
            );
        }
    }

    /// 取走本次 dispatch 累积的人读警告（D1；驱动在事件路径结束时 drain，
    /// 每次 dispatch 至多一条，信封仅在非空时加 `warnings` 键）。
    pub fn take_dispatch_warning(&self) -> Option<String> {
        self.state.borrow_mut().warning.take()
    }

    /// 取走 `world.log` 累积的日志行（M1-C；环形缓冲，满则丢最旧）。
    ///
    /// 只由宿主 drain：日志不进叙述流、不进事件流（混进去会让「输出 = 状态」
    /// 的对账多出一类噪声）。
    pub fn take_world_log(&self) -> Vec<String> {
        let mut state = self.state.borrow_mut();
        state.world.take_logs()
    }

    /// 装载期收集的事件名（按注册序；诊断面）。
    pub fn registered_event_names(&self) -> Vec<String> {
        self.state
            .borrow()
            .registered
            .iter()
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// 注入或卸载模拟内核实例（`ygg.sim.*` 的宿主面，Tier 1）。
    ///
    /// 未注入时 10 个函数全部抛 `YGG-SIM-UNMOUNTED`——**不是** `nil` / `false`：
    /// `nil` 与「世界是空的」不可分辨，静默降级会让 Mod 把「宿主没这个能力」读成
    /// 「世界为空」并据此写下错误状态。
    ///
    /// 具体类型而不是 trait object：只有一份实现（`ygg_sim::Sim`），`dyn` 只会给
    /// 每次调用加一层 vtable（AGENTS §1.7「能不加就不加」）。
    pub fn set_sim(&self, sim: Option<Box<ygg_sim::Sim>>) {
        *self.state.borrow().sim.borrow_mut() = sim;
    }

    /// 取模拟内核句柄（驱动层共享；宿主批量帧走这条，不经 Lua 边界）。
    pub fn sim_handle(&self) -> Rc<RefCell<Option<Box<ygg_sim::Sim>>>> {
        Rc::clone(&self.state.borrow().sim)
    }
}
