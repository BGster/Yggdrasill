//! S9 host API（D-S9-1..D-S9-4）：`ygg.*` 完整面 + 收集-回放桥 + 装载/运行期分相。
//!
//! - **生命周期与分相**（D-S9-1）：[`install`] 在**装载期**安装完整 `ygg` 面
//!   （`on / emit / narrate / choice / kv.* / entity.*`）；[`HostApi::seal`]
//!   在装载结束后把 `ygg.on` **原位替换**为可读报错桩——运行期注册禁令
//!   （D-B4 的运行时侧权威执行面）。无 mlua Scope、无线程局部存储或静态可变
//!   全局：全部共享状态经 `Rc<RefCell<HostState>>` 单线程持有。
//! - **收集-回放桥**（D-S9-2）：`ygg.emit / narrate / choice` 在 Lua 执行期间
//!   只向 [`HostState::calls`] **按序追加** [`HostCall`]；[`LuaHandler`]
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
//!   `HandlerRegistry::register`（每条一个 [`LuaHandler`]），注册序即派发序。
//! - **结构化 meta 通道**（TD1）：`ygg.meta(key, value)` 把**结构化数据**写进
//!   [`HostState::meta`]（`BTreeMap<String, MetaValue>`），与叙事面完全分离——
//!   Mod 不必再把人读文本当结构化传输（旧 `【摘要】<JSON>` 前端耦合已删除）。
//!   三条纪律：① 不走 [`HostState::calls`] 缓冲（不回放、不改 core trait），
//!   驱动在转换结束后经 [`HostApi::take_meta`] 取走并拼进信封 `meta` 键；
//!   ② key 必须是长度 1..=`META_KEY_MAX_BYTES` 的字符串（非空、可读报错）；
//!   ③ value 只允许标量 / 稠密序列 / 字符串键表（深度 `META_MAX_DEPTH` 封顶），
//!   非有限值（NaN/±∞）fail-fast 拒绝——与 `ygg.emit` 同纪律。
//!   **meta 是瞬态回执面**：不入 canonical store / 存档 / 状态哈希（驱动侧
//!   每次 dispatch 入口 clear、转换后 take，生命周期不跨事件）。
//!   **值类型取 `serde_json::Value` 而非 core `Value`**：core `Value` 只有四个
//!   标量，装不下回执需要的对象/序列（`meta.summary.conflicts` 就是数组）；
//!   标量 leaf 仍走 [`value_from_lua`]（同一套类型门），容器内层用
//!   `BTreeMap` 序 + serde_json 默认字典序 Map 保证确定性字节。
//! - **payload 双向 marshal**：Integer↔`Int`、Number↔`Float`、String↔`Str`、
//!   Boolean↔`Bool`；Lua→core 遇 NaN/±∞ 由 core 门拒绝（可读 mlua 错误）、
//!   `-0.0` 由 core 规范化；`nil` 仅在 get 类miss 上作为 Lua `nil` 返回；
//!   table / function / 其他类型 → 可读 `unsupported payload type: {lua_type}`。
//!   `ygg.choice` 的 options 必须是字符串的 Lua 序列，否则可读报错。

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use mlua::{Error, Function, Lua, Table, Value as LuaValue};
use serde_json::{Map as JsonMap, Value as MetaValue};
use ygg_core::bus::{EventHandler, HandlerError, HandlerRegistry, NarrationSink, PendingChoice};
use ygg_core::entity::{
    like_shape, normalize_f64, CompareOp, EntityStore, IsTest, Order, Predicate, Selector, Value,
};
use ygg_core::event::Event;
use ygg_core::kv::KvStore;
use ygg_schema::validate::sha256_hex;

use crate::inject::splitmix64;

// ---- 模块化拆分 shim(S-25 批三 C1:纯搬移;既有路径逐字节不变)----
mod entity_decode;
mod fs;
mod kv;
mod meta;
mod selector;
mod sim;
mod tool;
mod value;

pub use entity_decode::*;
pub use fs::*;
// `pub mod ygg_fs_code` 嵌套模块原路径保留(项本身 pub,glob 已含;显式列出为自证)。
pub use fs::ygg_fs_code;
pub use meta::*;
pub use selector::*;
pub use sim::*;
pub use tool::*;
pub use value::*;

/// 运行期注册禁令的可读报错文案（seal 后 `ygg.on` 桩与 sealed 双检共用）。
const SEALED_MESSAGE: &str = "ygg.on is sealed: runtime handler registration is forbidden \
 (handlers may only be registered during the load phase)";

/// 宿主相位：`ygg.tool.call` 仅允许在转换期（`Phase::Transition`）调用。
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

// ============================================================================
// §VFS（`ygg.fs` 的内存态：装载期写入面与转换期读面共用同一个 Rc）
// ============================================================================

/// 桥共享状态：注册序缓冲 + 调用缓冲 + 宿主句柄 + 相位标志 + per-kernel
/// choice sentinel（D-S9-1 / D-S9-3）。
pub struct HostState {
    registered: Vec<(String, Function)>,
    calls: Vec<HostCall>,
    handles: HostHandles,
    /// `ygg.fs` 的内存态 VFS（与驱动共享同一 `Rc`；savepoint 基线由驱动克隆）。
    ///
    /// **不在 `HostHandles` 里**：句柄是宿主注入面（`crates/ygg-sandbox/tests`
    /// 与 `stdlib` 都构造它），加字段会破外部构造点；VFS 是 `install` 自建的
    /// 内部状态，经 `HostApi::vfs_handle()` 交给驱动。
    vfs: Rc<RefCell<VfsStore>>,
    /// 宿主侧同步工具执行器（未挂载时 `None`，`ygg.tool.call` 返回 `unmounted = true`）。
    tool_runner: Rc<RefCell<Option<Box<dyn HostToolRunner>>>>,
    /// 宿主侧文件挂载桥（未挂载时 `None`，访问 `mounts/…` 抛 `YGG-FS-UNMOUNTED`）。
    ///
    /// 与 `vfs` 的区别是纪律性的：VFS 是沙箱内存态（dispatch 级回滚），
    /// 挂载桥是写穿磁盘的对外面（不回滚、每次写审计）。两者共享同一个 `ygg.fs` 表，
    /// 按路径首段分派。
    mount: Rc<RefCell<Option<Box<dyn MountFs>>>>,
    /// 宿主侧模拟内核实例（Tier 1；未注入时 `None`，`ygg.sim.*` 全部抛 `YGG-SIM-UNMOUNTED`）。
    ///
    /// **不在 `HostHandles` 里**（与 `vfs` 同一条纪律）：句柄是外部构造点，加字段会破
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
    /// 结构化 meta（TD1）：`ygg.meta(key, value)` 写入面，键序 = `BTreeMap`
    /// 序（天然确定性）。瞬态——驱动每次 dispatch 入口 clear、转换后 take，
    /// 不入 canonical snapshot / autosave / 状态哈希。
    meta: BTreeMap<String, MetaValue>,
}

/// 宿主 API 的 Rust 侧控制柄：seal / 注册进注册表。
pub struct HostApi {
    lua: Lua,
    state: Rc<RefCell<HostState>>,
}

/// 装载期注册的单个 Lua handler 的桥实现（core `EventHandler` 反转面）。
struct LuaHandler {
    lua: Lua,
    function: Function,
    state: Rc<RefCell<HostState>>,
}

impl LuaHandler {
    /// 本 kernel install 的 choice sentinel（audit 代决：不可预测 token，
    /// 防 Mod 以 `\u{1}` 转义伪造首行吞掉自身错误——伪造只会成为普通错误浮出）。
    fn is_choice_sentinel(&self, err: &Error) -> bool {
        is_choice_sentinel(&self.state.borrow().sentinel, err)
    }
}

impl EventHandler for LuaHandler {
    fn handle(&mut self, event: &Event, sink: &mut dyn NarrationSink) -> Result<(), HandlerError> {
        // ① 清空调用缓冲与 choice 前置计数（上一 handler 的余量不跨 handler 泄漏），进入 Transition 相位。
        let prev_phase = {
            let mut state = self.state.borrow_mut();
            state.calls.clear();
            state.pre_choice_emits = 0;
            let prev = state.phase;
            state.phase = Phase::Transition;
            prev
        };
        struct PhaseReset {
            state: Rc<RefCell<HostState>>,
            prev: Phase,
        }
        impl Drop for PhaseReset {
            fn drop(&mut self) {
                self.state.borrow_mut().phase = self.prev;
            }
        }
        let _phase_guard = PhaseReset {
            state: Rc::clone(&self.state),
            prev: prev_phase,
        };
        // ② core Value → mlua Value，装载期存下的函数以 payload 为唯一实参。
        let payload = value_to_lua(&self.lua, event.payload()).map_err(|e| {
            HandlerError(format!(
                "event `{}` payload marshal failed: {e}",
                event.name()
            ))
        })?;
        // ③ 调用：sentinel = choice 正常终结；其余错误包成 HandlerError。
        match self.function.call::<()>(payload) {
            Ok(()) => {}
            Err(err) if self.is_choice_sentinel(&err) => {}
            Err(err) => {
                return Err(HandlerError(format!(
                    "lua handler for event `{}` failed: {err}",
                    event.name()
                )))
            }
        }
        // ③' D1 警告：choice 前置 Emit 已被 pending_choice 终结语义丢弃——
        //    事件名在此可得（choice 回调时只有缓冲可见），每次 dispatch
        //    首条胜出（warning 为 None 才写入）。
        let pre_choice = self.state.borrow().pre_choice_emits;
        if pre_choice > 0 {
            let mut state = self.state.borrow_mut();
            if state.warning.is_none() {
                state.warning = Some(format!(
                    "事件 `{}` 的 handler 在 ygg.choice 之前产生了 {} 条派生事件（ygg.emit），\
                     已按语义丢弃（choice 须为 handler 最后一个输出调用）",
                    event.name(),
                    pre_choice
                ));
            }
        }
        // ④ 按追加序回放进 sink（等价性论证见模块文档）。
        let calls = std::mem::take(&mut self.state.borrow_mut().calls);
        for call in calls {
            match call {
                HostCall::Emit { name, payload } => {
                    let derived =
                        Event::derived(name, payload).map_err(|e| HandlerError(e.to_string()))?;
                    sink.emit(derived);
                }
                HostCall::Narrate(line) => sink.narrate(line),
                HostCall::Choose(choice) => sink.choose(choice),
            }
        }
        Ok(())
    }
}

/// 恰好以 per-kernel sentinel 为首行的错误才是 choice 终结（D-S9-3）。
/// 错误跨 Lua 边界后 mlua 包装为 `CallbackError{ cause }` 链（每层边界一层，
/// 原 RuntimeError 在链尾），故递归解包 cause；消息比对取首行——mlua 在
/// 某些路径会把 stack traceback 并入消息体。sentinel 为 install 时由内核种子
/// 经 splitmix64 派生的不可预测 token（S9 审计代决：静态常量可被 Mod 以
/// `\u{1}` 转义伪造首行而吞掉自身错误；token 化后伪造只会成为普通错误浮出）。
fn is_choice_sentinel(sentinel: &str, err: &Error) -> bool {
    match err {
        Error::RuntimeError(msg) => msg.split('\n').next() == Some(sentinel),
        Error::CallbackError { cause, .. } => is_choice_sentinel(sentinel, cause),
        _ => false,
    }
}

/// 形状不对（非数组 / 元素不是表 / 字段类型错 / 值非标量）。
pub const ENTITY_FRAME: &str = "YGG-ENTITY-FRAME";
/// 值不在值域（非有限浮点）。
pub const ENTITY_VALUE: &str = "YGG-ENTITY-VALUE";
/// 未知实体 id。
pub const ENTITY_ID: &str = "YGG-ENTITY-ID";

/// 取实体句柄（批式面的统一入口；`HostState` 借用只在调用瞬间）。
fn entities_of(state: &Rc<RefCell<HostState>>) -> Rc<RefCell<EntityStore>> {
    Rc::clone(&state.borrow().handles.entities)
}

pub(super) fn entity_frame(detail: impl std::fmt::Display) -> Error {
    Error::runtime(format!("{ENTITY_FRAME}: {detail}"))
}

pub(super) fn entity_value(detail: impl std::fmt::Display) -> Error {
    Error::runtime(format!("{ENTITY_VALUE}: {detail}"))
}

pub(super) fn entity_id_error(id: u64) -> Error {
    Error::runtime(format!("{ENTITY_ID}: 未知实体 id {id}"))
}

pub fn install(lua: &Lua, handles: HostHandles) -> Result<HostApi, Error> {
    let state = Rc::new(RefCell::new(HostState {
        registered: Vec::new(),
        calls: Vec::new(),
        handles,
        // VFS 的内存态：`install` 自建（不属注入面），驱通过
        // [`HostApi::vfs_handle`] 拿到同一个 `Rc` 后接入 savepoint 回滚。
        vfs: Rc::new(RefCell::new(VfsStore::default())),
        tool_runner: Rc::new(RefCell::new(None)),
        mount: Rc::new(RefCell::new(None)),
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
        meta: BTreeMap::new(),
    }));

    let ygg: Table = lua.globals().raw_get("ygg")?;

    // ---- ygg.on（装载期注册；seal 后以双检 + 原位桩拒绝，D-S9-1 / D-S9-4）----
    let st = Rc::clone(&state);
    ygg.raw_set(
        "on",
        lua.create_function(
            move |_, (event_name, handler): (String, Function)| -> Result<(), Error> {
                let mut s = st.borrow_mut();
                if s.sealed {
                    return Err(Error::runtime(SEALED_MESSAGE));
                }
                s.registered.push((event_name, handler));
                Ok(())
            },
        )?,
    )?;

    // ---- ygg.emit（收集面：校验 + fail-fast 后追加，D-S9-2）----
    let st = Rc::clone(&state);
    ygg.raw_set(
        "emit",
        lua.create_function(
            move |_, (name, payload): (String, LuaValue)| -> Result<(), Error> {
                let value = value_from_lua(&payload)?;
                // fail-fast：NaN/±∞ 在调用点即以事件名定位拒绝（不进缓冲）。
                Event::derived(name.clone(), value.clone())
                    .map_err(|e| Error::runtime(e.to_string()))?;
                st.borrow_mut().calls.push(HostCall::Emit {
                    name,
                    payload: value,
                });
                Ok(())
            },
        )?,
    )?;

    // ---- ygg.narrate（收集面，D-S9-2）----
    let st = Rc::clone(&state);
    ygg.raw_set(
        "narrate",
        lua.create_function(move |_, line: String| -> Result<(), Error> {
            st.borrow_mut().calls.push(HostCall::Narrate(line));
            Ok(())
        })?,
    )?;

    // ---- ygg.choice（收集 + sentinel 终止，D-S9-3）----
    let st = Rc::clone(&state);
    ygg.raw_set(
        "choice",
        lua.create_function(
            move |_, (prompt, options): (String, LuaValue)| -> Result<(), Error> {
                let opts = options_from_lua(&options)?;
                let sentinel = st.borrow().sentinel.clone();
                let mut state = st.borrow_mut();
                // D1：此刻缓冲中已有的 Emit 条目将因 pending_choice 终结被丢弃
                //（narrate 不丢弃，直接进叙述缓冲，不计数）。
                state.pre_choice_emits = state
                    .calls
                    .iter()
                    .filter(|call| matches!(call, HostCall::Emit { .. }))
                    .count() as u64;
                state
                    .calls
                    .push(HostCall::Choose(PendingChoice::new(prompt, opts)));
                Err(Error::runtime(sentinel))
            },
        )?,
    )?;

    // ---- ygg.meta（结构化回执面，TD1）：不进 calls 缓冲、不回放、不改 core
    //      trait；驱动转换结束后 take_meta 拼进信封。key 长度/类型与非有限值
    //      在调用点 fail-fast 拒绝（与 ygg.emit 同纪律）。----
    let st = Rc::clone(&state);
    ygg.raw_set(
        "meta",
        lua.create_function(
            move |_, (key, payload): (LuaValue, LuaValue)| -> Result<(), Error> {
                let name = meta_key_from_lua(&key)?;
                let value = meta_value_from_lua(&payload, 0)?;
                st.borrow_mut().meta.insert(name, value);
                Ok(())
            },
        )?,
    )?;

    // ---- ygg.fs.*（内容寻址的小文件面：写经 VfsStore，读 / 列是纯读）。
    //      `fs.write` 在模块顶层被影子表拒绍（effects::BANNED_TABLE_MEMBERS），
    //      `fs.read` / `fs.stat` / `fs.list` 是纯读，任何相位都允许。
    //      合约：write 的参数类型必须是 (String, String)——不猜、不隐式转换
    //      （mlua 自己的 arity/type 报错已够人读）；read miss = nil；stat miss
    //      = `{exists=false,bytes=0,sha256=""}`（**恒回表，永不 nil**）；list 是
    //      从 1 开始的**稠密数组**。
    let fs_table = lua.create_table()?;

    let fs_write = Rc::clone(&state);
    fs_table.raw_set(
        "write",
        lua.create_function(
            move |lua, (path, content): (String, String)| -> Result<Table, Error> {
                let out = lua.create_table()?;
                // 首段分派：`mounts` 走挂载桥（写穿磁盘 + 审计），其余走旧内存 VFS。
                if is_mount_path(&path) {
                    let bridge = Rc::clone(&fs_write.borrow().mount);
                    let mut slot = bridge.borrow_mut();
                    let Some(bridge) = slot.as_mut() else {
                        return Err(Error::runtime(unmounted_message("fs.write", &path)));
                    };
                    let written = bridge
                        .write(&path, content.as_bytes())
                        .map_err(Error::runtime)?;
                    out.raw_set("path", written.path)?;
                    out.raw_set("bytes", written.bytes)?;
                    out.raw_set("sha256", written.sha256)?;
                    return Ok(out);
                }
                let vfs = Rc::clone(&fs_write.borrow().vfs);
                let file = vfs
                    .borrow_mut()
                    .put(&path, content.as_bytes())
                    .map_err(Error::runtime)?;
                out.raw_set("path", path)?;
                out.raw_set("bytes", file.bytes.len())?;
                out.raw_set("sha256", file.sha256)?;
                Ok(out)
            },
        )?,
    )?;

    let fs_read = Rc::clone(&state);
    fs_table.raw_set(
        "read",
        lua.create_function(move |lua, path: String| -> Result<LuaValue, Error> {
            if is_mount_path(&path) {
                let bridge = Rc::clone(&fs_read.borrow().mount);
                let slot = bridge.borrow();
                let Some(bridge) = slot.as_ref() else {
                    return Err(Error::runtime(unmounted_message("fs.read", &path)));
                };
                let hit = bridge.read(&path).map_err(Error::runtime)?;
                return Ok(match hit {
                    Some(bytes) => LuaValue::String(lua.create_string(&bytes)?),
                    None => LuaValue::Nil, // miss = nil（与旧面同口径）
                });
            }
            let vfs = Rc::clone(&fs_read.borrow().vfs);
            let hit = vfs.borrow().get(&path).map(|file| file.bytes.clone());
            Ok(match hit {
                Some(bytes) => LuaValue::String(lua.create_string(&bytes)?),
                None => LuaValue::Nil, // miss = nil，不 throw
            })
        })?,
    )?;

    let fs_stat = Rc::clone(&state);
    fs_table.raw_set(
        "stat",
        lua.create_function(move |lua, path: String| -> Result<Table, Error> {
            let out = lua.create_table()?;
            if is_mount_path(&path) {
                let bridge = Rc::clone(&fs_stat.borrow().mount);
                let slot = bridge.borrow();
                let Some(bridge) = slot.as_ref() else {
                    return Err(Error::runtime(unmounted_message("fs.stat", &path)));
                };
                let stat = bridge.stat(&path).map_err(Error::runtime)?;
                out.raw_set("exists", stat.exists)?;
                out.raw_set("bytes", stat.bytes)?;
                out.raw_set("sha256", stat.sha256)?;
                out.raw_set("kind", stat.kind.as_str())?;
                return Ok(out);
            }
            let vfs = Rc::clone(&fs_stat.borrow().vfs);
            let hit = vfs
                .borrow()
                .get(&path)
                .map(|file| (file.bytes.len(), file.sha256.clone()));
            match hit {
                Some((bytes, sha256)) => {
                    out.raw_set("exists", true)?;
                    out.raw_set("bytes", bytes)?;
                    out.raw_set("sha256", sha256)?;
                }
                None => {
                    out.raw_set("exists", false)?;
                    out.raw_set("bytes", 0)?;
                    out.raw_set("sha256", String::new())?;
                }
            }
            Ok(out)
        })?,
    )?;

    let fs_list = Rc::clone(&state);
    fs_table.raw_set(
        "list",
        lua.create_function(move |lua, prefix: Option<String>| -> Result<Table, Error> {
            let out = lua.create_table()?;
            // 无参 = 旧面（字节级不变）；有参且以 `mounts` 起头 = 挂载面；
            // 其余前缀 → 拒（不做「按前缀列 artifacts」的第二套遍历语义）。
            if let Some(path) = prefix {
                if !is_mount_path(&path) {
                    return Err(Error::runtime(format!(
                        "{}: ygg.fs.list 的前缀必须以 `mounts` 起头（当前是 `{path}`）",
                        ygg_fs_code::JAIL
                    )));
                }
                let bridge = Rc::clone(&fs_list.borrow().mount);
                let slot = bridge.borrow();
                let Some(bridge) = slot.as_ref() else {
                    return Err(Error::runtime(unmounted_message("fs.list", &path)));
                };
                let rows = bridge.list(&path).map_err(Error::runtime)?;
                for (index, row) in rows.into_iter().enumerate() {
                    let table = lua.create_table()?;
                    table.raw_set("name", row.name)?;
                    table.raw_set("path", row.path)?;
                    table.raw_set("bytes", row.bytes)?;
                    table.raw_set("sha256", String::new())?; // 不算摘要（键在场、值留白）
                    table.raw_set("kind", row.kind.as_str())?;
                    out.raw_set(index + 1, table)?; // 稠密数组：1..=n
                }
                return Ok(out);
            }
            let vfs = Rc::clone(&fs_list.borrow().vfs);
            let rows = vfs.borrow().snapshot(); // 已按 BTreeMap 序
            for (index, (path, bytes, sha256)) in rows.into_iter().enumerate() {
                let row = lua.create_table()?;
                row.raw_set("path", path)?;
                row.raw_set("bytes", bytes)?;
                row.raw_set("sha256", sha256)?;
                out.raw_set(index + 1, row)?; // **稠密数组**：1..=n
            }
            Ok(out)
        })?,
    )?;

    ygg.raw_set("fs", fs_table)?;

    // ---- ygg.tool.*（同步宿主工具桥：装载期由影子表禁止，转换期按挂载态执行或回退）----
    let tool_table = lua.create_table()?;
    let tool_call = Rc::clone(&state);
    tool_table.raw_set(
        "call",
        lua.create_function(
            move |lua, (tool_id, args_val): (String, Option<LuaValue>)| -> Result<Table, Error> {
                let trimmed = tool_id.trim();
                if trimmed.is_empty() || trimmed.len() > TOOL_ID_MAX_BYTES {
                    return Err(Error::runtime(format!(
                        "ygg.tool.call tool_id must be 1..={TOOL_ID_MAX_BYTES} bytes, got `{tool_id}`"
                    )));
                }
                let args = tool_args_from_lua(args_val.as_ref())?;
                let (phase, runner_rc) = {
                    let s = tool_call.borrow();
                    (s.phase, Rc::clone(&s.tool_runner))
                };
                if phase != Phase::Transition {
                    return Err(Error::runtime(format!(
                        "ygg.tool.call is only allowed during transition phase (current phase: {phase:?})"
                    )));
                }
                let mut runner_slot = runner_rc.borrow_mut();
                let out = lua.create_table()?;
                match runner_slot.as_mut() {
                    None => {
                        out.raw_set("ok", false)?;
                        out.raw_set("unmounted", true)?;
                        out.raw_set("code", -1)?;
                        out.raw_set("error", "no host tool runner mounted")?;
                    }
                    Some(runner) => match runner.call(trimmed, &args) {
                        Ok(data_map) => {
                            let data_table = lua.create_table()?;
                            for (k, v) in data_map {
                                data_table.raw_set(k, v)?;
                            }
                            out.raw_set("ok", true)?;
                            out.raw_set("unmounted", false)?;
                            out.raw_set("code", 0)?;
                            out.raw_set("data", data_table)?;
                        }
                        Err(err_msg) => {
                            out.raw_set("ok", false)?;
                            out.raw_set("unmounted", false)?;
                            out.raw_set("code", 1)?;
                            out.raw_set("error", err_msg)?;
                        }
                    },
                }
                Ok(out)
            },
        )?,
    )?;
    ygg.raw_set("tool", tool_table)?;

    // ---- ygg.kv.*（即席状态写：core 把关方法，门拒绝为可读 mlua 错误；
    //      句柄经 HostState.handles 现取现用，字段即真实读取面）----
    let kv_table = lua.create_table()?;
    let kv_get = Rc::clone(&state);
    kv_table.raw_set(
        "get",
        lua.create_function(move |lua, key: String| -> Result<LuaValue, Error> {
            let kv = Rc::clone(&kv_get.borrow().handles.kv);
            let found = kv.borrow().get(&key).cloned();
            Ok(match found {
                Some(v) => value_to_lua(lua, &v)?,
                None => LuaValue::Nil, // get-miss = nil（D-S9-2 marshal 规则）
            })
        })?,
    )?;
    let kv_set = Rc::clone(&state);
    kv_table.raw_set(
        "set",
        lua.create_function(
            move |_, (key, payload): (String, LuaValue)| -> Result<(), Error> {
                let value = value_from_lua(&payload)?;
                let kv = Rc::clone(&kv_set.borrow().handles.kv);
                let outcome = kv
                    .borrow_mut()
                    .set(key.clone(), value)
                    .map_err(|e| Error::runtime(e.to_string()));
                outcome
            },
        )?,
    )?;
    ygg.raw_set("kv", kv_table)?;

    // ---- ygg.entity.*（即席状态写；get 类 miss = nil / false，写类 miss = 可读错误）----
    let entity_table = lua.create_table()?;
    let ent_spawn = Rc::clone(&state);
    entity_table.raw_set(
        "spawn",
        lua.create_function(move |_, ()| -> Result<i64, Error> {
            let entities = Rc::clone(&ent_spawn.borrow().handles.entities);
            let id = entities
                .borrow_mut()
                .spawn()
                .map_err(|e| Error::runtime(e.to_string()))?;
            i64::try_from(id).map_err(|_| Error::runtime("entity id overflow"))
        })?,
    )?;
    let ent_get = Rc::clone(&state);
    entity_table.raw_set(
        "get_attribute",
        lua.create_function(
            move |lua, (id, name): (u64, String)| -> Result<LuaValue, Error> {
                let entities = Rc::clone(&ent_get.borrow().handles.entities);
                let found = entities
                    .borrow()
                    .get(id)
                    .and_then(|entity| entity.attribute(&name).cloned());
                Ok(match found {
                    Some(v) => value_to_lua(lua, &v)?,
                    None => LuaValue::Nil,
                })
            },
        )?,
    )?;
    let ent_set = Rc::clone(&state);
    entity_table.raw_set(
        "set_attribute",
        lua.create_function(
            move |_, (id, name, payload): (u64, String, LuaValue)| -> Result<(), Error> {
                let value = value_from_lua(&payload)?;
                let entities = Rc::clone(&ent_set.borrow().handles.entities);
                let outcome = entities
                    .borrow_mut()
                    .set_attribute(id, name, value)
                    .map_err(|e| Error::runtime(e.to_string()));
                outcome
            },
        )?,
    )?;
    let ent_has = Rc::clone(&state);
    entity_table.raw_set(
        "has_tag",
        lua.create_function(move |_, (id, tag): (u64, String)| -> Result<bool, Error> {
            let entities = Rc::clone(&ent_has.borrow().handles.entities);
            Ok(ent_get_tag(&entities, id, &tag))
        })?,
    )?;
    let ent_add = Rc::clone(&state);
    entity_table.raw_set(
        "add_tag",
        lua.create_function(move |_, (id, tag): (u64, String)| -> Result<bool, Error> {
            let entities = Rc::clone(&ent_add.borrow().handles.entities);
            let changed = entities
                .borrow_mut()
                .add_tag(id, tag)
                .map_err(|e| Error::runtime(e.to_string()))?;
            Ok(changed)
        })?,
    )?;
    let ent_remove = Rc::clone(&state);
    entity_table.raw_set(
        "remove_tag",
        lua.create_function(move |_, (id, tag): (u64, String)| -> Result<bool, Error> {
            let entities = Rc::clone(&ent_remove.borrow().handles.entities);
            let removed = entities
                .borrow_mut()
                .remove_tag(id, &tag)
                .map_err(|e| Error::runtime(e.to_string()))?;
            Ok(removed)
        })?,
    )?;
    // ---- 批式五面（P-K A1；一个逐实体 getter 都没有）------------------------
    //
    // 入参是 Lua 数组（铁律② 原文「bytes/数组进出」）：n=1 的热路径不必付 pack 成本，
    // 测试夹具也不必写第二份编解码器。五面一律**先校验后改**。
    let ent_spawn_batch = Rc::clone(&state);
    entity_table.raw_set(
        "spawn_batch",
        lua.create_function(move |lua, rows: Table| -> Result<Table, Error> {
            let specs = entity_rows(&rows)?;
            let entities = Rc::clone(&ent_spawn_batch.borrow().handles.entities);
            let (ids, next_id) = {
                let mut store = entities.borrow_mut();
                let mut ids = Vec::with_capacity(specs.len());
                for spec in &specs {
                    let id = store.spawn().map_err(|e| Error::runtime(e.to_string()))?;
                    for (name, value) in &spec.attrs {
                        store
                            .set_attribute(id, name.clone(), value.clone())
                            .map_err(|e| Error::runtime(e.to_string()))?;
                    }
                    for tag in &spec.tags {
                        store
                            .add_tag(id, tag.clone())
                            .map_err(|e| Error::runtime(e.to_string()))?;
                    }
                    ids.push(id);
                }
                (ids, store.next_id())
            };
            let out = lua.create_table()?;
            out.raw_set("spawned", ids.len())?;
            out.raw_set("next_id", next_id)?;
            let list = lua.create_table()?;
            for (index, id) in ids.iter().enumerate() {
                list.raw_set(index + 1, *id)?;
            }
            out.raw_set("ids", list)?;
            Ok(out)
        })?,
    )?;

    let ent_attr_apply = Rc::clone(&state);
    entity_table.raw_set(
        "attr_apply",
        lua.create_function(move |lua, ops: Table| -> Result<Table, Error> {
            let parsed = entity_attr_ops(&ops)?;
            let entities = Rc::clone(&ent_attr_apply.borrow().handles.entities);
            let (applied, changed) = {
                let mut store = entities.borrow_mut();
                // 先校验后改：整批 id 全部在场，否则零改（错误带首个坏 id）。
                for (id, _) in &parsed {
                    if store.get(*id).is_none() {
                        return Err(entity_id_error(*id));
                    }
                }
                let mut applied = 0usize;
                let mut changed = 0usize;
                for (id, kvs) in &parsed {
                    for (name, value) in kvs {
                        // 「值是否真的变了」由内核判（含整/浮身份：`Value::Int(2) != Value::Float(2.0)`），
                        // Tier 2 据 `changed` 决定发不发事件（entity.md I6）。
                        let before = store.get(*id).and_then(|e| e.attribute(name)).cloned();
                        if before.as_ref() != Some(value) {
                            changed += 1;
                        }
                        store
                            .set_attribute(*id, name.clone(), value.clone())
                            .map_err(|e| Error::runtime(e.to_string()))?;
                        applied += 1;
                    }
                }
                (applied, changed)
            };
            let out = lua.create_table()?;
            out.raw_set("applied", applied)?;
            out.raw_set("changed", changed)?;
            Ok(out)
        })?,
    )?;

    let ent_attr_fetch = Rc::clone(&state);
    entity_table.raw_set(
        "attr_fetch",
        lua.create_function(move |lua, queries: Table| -> Result<Table, Error> {
            let parsed = entity_fetch_queries(&queries)?;
            // 先把数据拷出来再建表：不在 create_* 期间持 RefCell 借用（借用纪律同 ygg.sim）。
            let rows: Vec<(Vec<(String, Value)>, Vec<String>)> = {
                let store = entities_of(&ent_attr_fetch);
                let store = store.borrow();
                parsed
                    .iter()
                    .map(|(id, keys)| {
                        let entity = store.get(*id);
                        let values = keys
                            .iter()
                            .filter_map(|key| {
                                entity
                                    .and_then(|e| e.attribute(key))
                                    .map(|v| (key.clone(), v.clone()))
                            })
                            .collect::<Vec<_>>();
                        let tags = entity
                            .map(|e| e.tags().map(str::to_owned).collect::<Vec<_>>())
                            .unwrap_or_default();
                        (values, tags)
                    })
                    .collect()
            };
            let values_table = lua.create_table()?;
            let tags_table = lua.create_table()?;
            for (index, (values, tags)) in rows.iter().enumerate() {
                let row = lua.create_table()?;
                for (key, value) in values {
                    row.raw_set(key.as_str(), value_to_lua(lua, value)?)?;
                }
                values_table.raw_set(index + 1, row)?;
                let list = lua.create_table()?;
                for (position, tag) in tags.iter().enumerate() {
                    list.raw_set(position + 1, tag.as_str())?;
                }
                tags_table.raw_set(index + 1, list)?;
            }
            let out = lua.create_table()?;
            out.raw_set("values", values_table)?;
            out.raw_set("tags", tags_table)?;
            Ok(out)
        })?,
    )?;

    let ent_tag_batch = Rc::clone(&state);
    entity_table.raw_set(
        "tag_batch",
        lua.create_function(move |_, ops: Table| -> Result<usize, Error> {
            let parsed = entity_tag_ops(&ops)?;
            let entities = Rc::clone(&ent_tag_batch.borrow().handles.entities);
            let mut store = entities.borrow_mut();
            for (id, _, _) in &parsed {
                if store.get(*id).is_none() {
                    return Err(entity_id_error(*id));
                }
            }
            let mut changed = 0usize;
            for (id, tag, add) in &parsed {
                let outcome = if *add {
                    store.add_tag(*id, tag.clone())
                } else {
                    store.remove_tag(*id, tag.as_str())
                }
                .map_err(|e| Error::runtime(e.to_string()))?;
                if outcome {
                    changed += 1;
                }
            }
            Ok(changed)
        })?,
    )?;

    let ent_scan = Rc::clone(&state);
    entity_table.raw_set(
        "scan",
        lua.create_function(
            move |lua, (lo, hi, tag): (u64, u64, Option<String>)| -> Result<Table, Error> {
                if hi < lo {
                    return Err(entity_frame(format!("scan 的区间非法：lo={lo} > hi={hi}")));
                }
                let entities = Rc::clone(&ent_scan.borrow().handles.entities);
                // **吃调用方给的 [lo, hi]，绝不读 next_id**：上界是 Tier 2 的 `seq` 水位，
                // 用它挡「本包 before 出生 / 他处直接 spawn」的实体（entity.md §2 冷启动边界）。
                let ids: Vec<u64> = {
                    let store = entities.borrow();
                    store
                        .iter()
                        .filter(|e| e.id >= lo && e.id <= hi)
                        .filter(|e| match &tag {
                            Some(needle) => e.has_tag(needle.as_str()),
                            None => true,
                        })
                        .map(|e| e.id)
                        .collect()
                };
                let out = lua.create_table()?;
                let list = lua.create_table()?;
                for (index, id) in ids.iter().enumerate() {
                    list.raw_set(index + 1, *id)?;
                }
                out.raw_set("ids", list)?;
                Ok(out)
            },
        )?,
    )?;

    // ---- 控制面两键（S9）：存在性 + 真删 ------------------------------------
    //
    // 形状是「单标量进出」：各吃一个 u64、回一个 bool，n=1 热路径不必构造帧
    // （批式化它按 packages/entity.md §9.7 实测是负收益：表形 485 ns/8 值贵过
    // 8 次逐项穿越 ≈320 ns）。两键**不新增错误码**：三族（FRAME / VALUE / ID）
    // 只属批式面——批式拒绝是为了「整批零副作用」，单条删除没有半截状态，
    // 缺失 id 是正常答案（`false`），不是错误（契约：kernel/lua-api.md §3.1）。
    let ent_exists = Rc::clone(&state);
    entity_table.raw_set(
        "exists",
        lua.create_function(move |_, id: u64| -> Result<bool, Error> {
            let entities = entities_of(&ent_exists);
            let present = entities.borrow().contains(id);
            Ok(present)
        })?,
    )?;
    let ent_destroy = Rc::clone(&state);
    entity_table.raw_set(
        "destroy",
        lua.create_function(move |_, id: u64| -> Result<bool, Error> {
            let entities = entities_of(&ent_destroy);
            // 真删：属性 / tag / 记录一并回收；`next_id` 不动 ⇒ id 永不复用（I1）。
            let removed = entities.borrow_mut().remove(id).is_some();
            Ok(removed)
        })?,
    )?;

    // ---- 查询面（S24）：一次跨界、内核全扫（可走索引时走索引）、id 升序 --------
    //
    // 两阶段材质化（借用纪律同 `attr_fetch`）：阶段 1 持借用、零 Lua 调用（拷 id 与
    // 投影值的连续 Vec）；阶段 2 无借用、只建表。`count` 复用阶段 1 长度，不重数。
    let ent_query = Rc::clone(&state);
    entity_table.raw_set(
        "query",
        lua.create_function(move |lua, selector: Table| -> Result<Table, Error> {
            let selector = entity_query_selector(&selector)?;
            let entities = entities_of(&ent_query);
            let (count, ids, values) = {
                let mut store = entities.borrow_mut();
                let rows = store.select(&selector);
                let count = rows.len();
                let stride = selector.select.len();
                let mut ids: Vec<u64> = Vec::with_capacity(count);
                let mut values: Vec<Option<Value>> = Vec::with_capacity(count * stride);
                for entity in &rows {
                    ids.push(entity.id);
                    for key in &selector.select {
                        values.push(entity.attribute(key).cloned());
                    }
                }
                (count, ids, values)
            };
            let out = lua.create_table()?;
            out.raw_set("count", count)?;
            if selector.select.is_empty() {
                let list = lua.create_sequence_from(ids.iter().copied())?;
                out.raw_set("ids", list)?;
            } else {
                let stride = selector.select.len();
                let list = lua.create_table_with_capacity(count, 0)?;
                for (row, id) in ids.iter().enumerate() {
                    let record = lua.create_table_with_capacity(0, 1 + stride)?;
                    record.raw_set("id", *id)?;
                    for (slot, key) in selector.select.iter().enumerate() {
                        // 缺属性 ⇒ 行上**不设键**（`row.hp == nil`，与 `attr_fetch` 同一条纪律）
                        if let Some(value) = &values[row * stride + slot] {
                            record.raw_set(key.as_str(), value_to_lua(lua, value)?)?;
                        }
                    }
                    list.raw_set(row + 1, record)?;
                }
                out.raw_set("rows", list)?;
            }
            Ok(out)
        })?,
    )?;

    ygg.raw_set("entity", entity_table)?;

    install_sim(lua, &state)?;
    // `ygg.codec.*`（Tier 2 pkg.codec 的原生加速面；契约 lua-api.md §3.1.4）。
    // 无状态、无实例，所以没有 `set_*` 注入面，也不需要 CLI 激活。
    crate::codec_face::install(lua)?;

    Ok(HostApi {
        lua: lua.clone(),
        state,
    })
}

/// `has_tag` 的 store 层读取（store 无 has_tag 时经 Entity 只读面；miss = false）。
fn ent_get_tag(entities: &Rc<RefCell<EntityStore>>, id: u64, tag: &str) -> bool {
    entities
        .borrow()
        .get(id)
        .map(|e| e.has_tag(tag))
        .unwrap_or(false)
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

    /// 取走本次 dispatch 累积的结构化 meta（TD1；**取走即清空**）。驱动在
    /// 转换结束后 drain：非空才在信封加 `meta` 键，空则保持既有线格式。
    pub fn take_meta(&self) -> BTreeMap<String, MetaValue> {
        std::mem::take(&mut self.state.borrow_mut().meta)
    }

    /// 清空结构化 meta（驱动在每次 dispatch **入口**调用：上一条的残留
    /// 不得混进本条的收据）。
    pub fn clear_meta(&self) {
        self.state.borrow_mut().meta.clear();
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

    /// `ygg.fs` 的内存态句柄（驱动拿去接入 dispatch 级 savepoint 基线）。
    ///
    /// 返回与 Lua 阴面**同一个** `Rc`：驱动克隆出来做基线、回滚时写回，
    /// Lua 侧下一次 `ygg.fs.read` 就读到回滚后的状态。
    pub fn vfs_handle(&self) -> Rc<RefCell<VfsStore>> {
        Rc::clone(&self.state.borrow().vfs)
    }

    /// 挂载或卸载宿主侧同步工具执行器。
    pub fn set_tool_runner(&self, runner: Option<Box<dyn HostToolRunner>>) {
        *self.state.borrow().tool_runner.borrow_mut() = runner;
    }

    /// 挂载或卸载宿主侧文件挂载桥（`ygg.fs.*` 的 `mounts/` 面）。
    pub fn set_mount_fs(&self, bridge: Option<Box<dyn MountFs>>) {
        *self.state.borrow().mount.borrow_mut() = bridge;
    }

    /// 取文件挂载桥句柄（驱动层共享）。
    pub fn mount_fs_handle(&self) -> Rc<RefCell<Option<Box<dyn MountFs>>>> {
        Rc::clone(&self.state.borrow().mount)
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

    /// 取同步工具执行器句柄（驱动层共享）。
    pub fn tool_runner_handle(&self) -> Rc<RefCell<Option<Box<dyn HostToolRunner>>>> {
        Rc::clone(&self.state.borrow().tool_runner)
    }
}

// ============================================================================
// §crate 内单测：`VfsStore` 的账本（revision / dirty / load）与路径门向量表
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 路径门向量表——**与 `ygg-store/src/lib.rs` 的同名用例逐字同步**。
    ///
    /// 两份门的存在是有意的（`ygg-store` 是 dev-dependency，运行时无权调用）；
    /// 这份向量表就是漂移锤子：规则只改一边时必有一侧红。
    fn path_gate_vectors() -> (Vec<String>, Vec<String>) {
        let max_ok = format!(
            "artifacts/{}.md",
            "a".repeat(VFS_PATH_MAX_BYTES - VFS_PREFIX.len() - 3)
        );
        let too_long = format!(
            "artifacts/{}.md",
            "a".repeat(VFS_PATH_MAX_BYTES - VFS_PREFIX.len() - 2)
        );
        let accepted = vec![
            "artifacts/a.md".to_owned(),
            "artifacts/notes/x.md".to_owned(),
            "artifacts/a.b-c_d/e.md".to_owned(),
            "artifacts/0.md".to_owned(),
            "artifacts/a/b/c/d.md".to_owned(),
            max_ok,
        ];
        let rejected = vec![
            "artifacts/".to_owned(),
            "artifacts//x".to_owned(),
            "artifacts/x/".to_owned(),
            "artifacts/../x".to_owned(),
            "artifacts/a/../../x".to_owned(),
            "artifacts/a..b".to_owned(),
            "artifact/x".to_owned(),
            "/artifacts/x".to_owned(),
            "artifacts/a b.md".to_owned(),
            "artifacts/a\\b.md".to_owned(),
            "artifacts/a|b.md".to_owned(),
            "artifacts/a@b.md".to_owned(),
            too_long,
            String::new(),
            "artifacts".to_owned(),
        ];
        (accepted, rejected)
    }

    #[test]
    fn path_gate_accepts_legal_shapes() {
        let (accepted, _) = path_gate_vectors();
        for path in accepted {
            validate_vfs_path(&path).unwrap_or_else(|reason| {
                panic!("`{path}`（{} 字节）必须放行：{reason}", path.len())
            });
        }
        // 边界自证：最长向量恰好是上限（否则「刚好 128」根本没被测到）。
        assert_eq!(
            format!(
                "artifacts/{}.md",
                "a".repeat(VFS_PATH_MAX_BYTES - VFS_PREFIX.len() - 3)
            )
            .len(),
            VFS_PATH_MAX_BYTES
        );
    }

    #[test]
    fn path_gate_rejects_illegal_shapes() {
        let (_, rejected) = path_gate_vectors();
        for path in rejected {
            let reason = validate_vfs_path(&path).expect_err(&format!("`{path}` 必须被拒"));
            assert!(!reason.is_empty(), "拒绝必须带人读原因");
        }
    }

    /// `put` 的账本：revision +1、入 dirty、内容与摘要正确。
    #[test]
    fn put_bookkeeps_revision_and_dirty() {
        let mut vfs = VfsStore::default();
        assert!(vfs.is_empty());
        let file = vfs.put("artifacts/a.md", b"first").expect("写入");
        assert_eq!(file.bytes, b"first".to_vec());
        assert_eq!(file.sha256.len(), 64, "小写十六进制 64 字符");
        assert_eq!(vfs.len(), 1);
        assert_eq!(vfs.revision(), 1);
        assert_eq!(
            vfs.get("artifacts/a.md").map(|f| f.bytes.clone()),
            Some(b"first".to_vec())
        );

        // 覆盖写：仍然只有一件，revision 继续增。
        vfs.put("artifacts/a.md", b"second").expect("覆盖");
        assert_eq!(vfs.len(), 1);
        assert_eq!(vfs.revision(), 2);
        assert_eq!(
            vfs.get("artifacts/a.md").map(|f| f.bytes.clone()),
            Some(b"second".to_vec())
        );
    }

    /// `put` 的两道门：坏路径 / 超限内容都不建键、不动 revision。
    #[test]
    fn put_rejects_bad_path_and_oversize_content() {
        let mut vfs = VfsStore::default();
        let err = vfs.put("artifacts/../x", b"y").expect_err("穿越必须被拒");
        assert!(err.starts_with("ygg.fs.write 拒绝："), "{err}");
        let over = vec![0u8; VFS_CONTENT_MAX_BYTES + 1];
        let err = vfs
            .put("artifacts/big.md", &over)
            .expect_err("超限必须被拒");
        assert!(err.contains("256 KiB"), "错误必须给出上限：{err}");
        // 恰好上限放行（否则边界不可验证）。
        vfs.put("artifacts/max.md", &vec![0u8; VFS_CONTENT_MAX_BYTES])
            .expect("恰好上限必须放行");
        assert_eq!(vfs.len(), 1, "被拒的写不得建键");
        assert_eq!(vfs.revision(), 1, "被拒的写不得动 revision");
    }

    /// `load` 是预载面：不 dirty、不动 revision（它不是本次会话的写入）。
    #[test]
    fn load_is_not_dirty_and_does_not_bump_revision() {
        let mut vfs = VfsStore::default();
        vfs.load("artifacts/pre.md", b"preset", "deadbeef");
        assert_eq!(vfs.len(), 1);
        assert_eq!(vfs.revision(), 0, "预载不得当成本次写入");
        assert!(vfs.take_dirty().is_empty(), "预载不得入 dirty");
        assert_eq!(
            vfs.get("artifacts/pre.md").map(|f| f.sha256.clone()),
            Some("deadbeef".to_owned()),
            "预载的摘要原样接受（不重算——盘上那份才是权威）"
        );
        // 非法路径的预载静默跳过：不建键，也不污染键空间。
        vfs.load("../escape.md", b"x", "y");
        assert_eq!(vfs.len(), 1);
    }

    /// `take_dirty` 按 `BTreeMap` 序排空，且排空后不复报。
    #[test]
    fn take_dirty_is_ordered_and_drains() {
        let mut vfs = VfsStore::default();
        for path in ["artifacts/z.md", "artifacts/a.md", "artifacts/m.md"] {
            vfs.put(path, path.as_bytes()).expect("写入");
        }
        let dirty = vfs.take_dirty();
        assert_eq!(
            dirty
                .iter()
                .map(|(path, _)| path.as_str())
                .collect::<Vec<_>>(),
            vec!["artifacts/a.md", "artifacts/m.md", "artifacts/z.md"]
        );
        assert!(vfs.take_dirty().is_empty(), "排空后不得重复上报");
        assert_eq!(vfs.len(), 3, "排空 dirty 不删文件");
    }

    /// `snapshot`（=`ygg.fs.list` 的回执形状）按路径升序，与写入序无关。
    #[test]
    fn snapshot_is_sorted_by_path() {
        let mut vfs = VfsStore::default();
        for path in ["artifacts/b.md", "artifacts/a.md"] {
            vfs.put(path, b"x").expect("写入");
        }
        let rows: Vec<String> = vfs
            .snapshot()
            .into_iter()
            .map(|(path, bytes, sha)| format!("{path}:{bytes}:{}", sha.len()))
            .collect();
        assert_eq!(rows, vec!["artifacts/a.md:1:64", "artifacts/b.md:1:64"]);
    }

    /// 克隆语义：`VfsStore` 整体可克隆（savepoint 基线的加载前提）——
    /// 副本与原件独立，改副本不影响原件。
    #[test]
    fn clone_is_an_independent_snapshot() {
        let mut vfs = VfsStore::default();
        vfs.put("artifacts/a.md", b"one").expect("写入");
        let mut baseline = vfs.clone();
        vfs.put("artifacts/b.md", b"two").expect("后续写入");
        assert_eq!(vfs.len(), 2);
        assert_eq!(baseline.len(), 1, "基线必须冻结在克隆时点");
        assert_eq!(baseline.revision(), 1);
        // dirty 也随基线克隆：基线里只有克隆时点那一条，所以回滚到它
        // **不会**把失败步的路径带进下一次 flush。
        assert_eq!(
            baseline
                .take_dirty()
                .iter()
                .map(|(path, _)| path.as_str())
                .collect::<Vec<_>>(),
            vec!["artifacts/a.md"]
        );
    }

    struct MockRunner;
    impl HostToolRunner for MockRunner {
        fn call(
            &mut self,
            tool_id: &str,
            args: &BTreeMap<String, String>,
        ) -> Result<BTreeMap<String, String>, String> {
            if tool_id == "mock.fail" {
                return Err("boom".to_owned());
            }
            let mut out = BTreeMap::new();
            out.insert("tool".to_owned(), tool_id.to_owned());
            if let Some(v) = args.get("k") {
                out.insert("echo".to_owned(), v.clone());
            }
            Ok(out)
        }
    }

    #[test]
    fn ygg_tool_call_respects_phase_and_runner_mount_state() {
        let sandbox = crate::sandbox::Sandbox::new().expect("sandbox");
        let lua = sandbox.lua().clone();
        let handles = HostHandles {
            kv: Rc::new(RefCell::new(KvStore::new())),
            entities: Rc::new(RefCell::new(EntityStore::new())),
        };
        let host = install(&lua, handles).expect("install host");
        // Loading 相位下直接调用 ygg.tool.call 必须被拒。
        let loading_err = lua
            .load(r#"ygg.tool.call("mock.ok", { k = "v" })"#)
            .exec()
            .expect_err("loading phase must reject ygg.tool.call");
        assert!(
            loading_err.to_string().contains("transition phase"),
            "{loading_err}"
        );

        lua.load(
            r#"
            ygg.on("test.call", function(tool_id)
                local res = ygg.tool.call(tool_id, { k = "val", n = 42, b = true })
                ygg.kv.set("ok", res.ok == true)
                ygg.kv.set("unmounted", res.unmounted == true)
                ygg.kv.set("code", res.code or -99)
                if res.data then
                    ygg.kv.set("echo", res.data.echo or "")
                end
                if res.error then
                    ygg.kv.set("err", res.error)
                end
            end)
        "#,
        )
        .exec()
        .expect("register handler");
        host.seal(&lua).expect("seal");
        let mut registry = HandlerRegistry::new();
        host.register_into(&mut registry, "mod");
        let mut entities = EntityStore::new();
        let mut fuel = crate::fuel::install(&lua, 10_000).expect("fuel");

        // 1) 未挂载：返回 unmounted = true, ok = false, code = -1
        ygg_core::transition::run_transition(
            &mut entities,
            &mut registry,
            &mut fuel,
            Event::external("test.call", Value::Str("mock.ok".to_owned())).expect("event"),
        )
        .expect("unmounted transition");
        assert_eq!(
            host.state.borrow().handles.kv.borrow().get("unmounted"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            host.state.borrow().handles.kv.borrow().get("ok"),
            Some(&Value::Bool(false))
        );
        assert_eq!(
            host.state.borrow().handles.kv.borrow().get("code"),
            Some(&Value::Int(-1))
        );

        // 2) 挂载后成功调用：ok = true, unmounted = false, code = 0, data 映射回传
        host.set_tool_runner(Some(Box::new(MockRunner)));
        fuel.reset(10_000);
        ygg_core::transition::run_transition(
            &mut entities,
            &mut registry,
            &mut fuel,
            Event::external("test.call", Value::Str("mock.ok".to_owned())).expect("event"),
        )
        .expect("mounted ok transition");
        assert_eq!(
            host.state.borrow().handles.kv.borrow().get("unmounted"),
            Some(&Value::Bool(false))
        );
        assert_eq!(
            host.state.borrow().handles.kv.borrow().get("ok"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            host.state.borrow().handles.kv.borrow().get("echo"),
            Some(&Value::Str("val".to_owned()))
        );

        // 3) 挂载后返回 Err：ok = false, unmounted = false, code = 1, error 回传
        fuel.reset(10_000);
        ygg_core::transition::run_transition(
            &mut entities,
            &mut registry,
            &mut fuel,
            Event::external("test.call", Value::Str("mock.fail".to_owned())).expect("event"),
        )
        .expect("mounted err transition");
        assert_eq!(
            host.state.borrow().handles.kv.borrow().get("ok"),
            Some(&Value::Bool(false))
        );
        assert_eq!(
            host.state.borrow().handles.kv.borrow().get("code"),
            Some(&Value::Int(1))
        );
        assert_eq!(
            host.state.borrow().handles.kv.borrow().get("err"),
            Some(&Value::Str("boom".to_owned()))
        );
    }
}
