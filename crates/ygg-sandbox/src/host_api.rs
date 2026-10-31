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
use std::collections::BTreeMap;
use std::rc::Rc;

use mlua::{Error, Function, Lua, Table, Value as LuaValue};
use serde_json::{Map as JsonMap, Value as MetaValue};
use ygg_core::bus::{EventHandler, HandlerError, HandlerRegistry, NarrationSink, PendingChoice};
use ygg_core::entity::{normalize_f64, EntityStore, Value};
use ygg_core::event::Event;
use ygg_core::kv::KvStore;

use crate::inject::splitmix64;

/// 运行期注册禁令的可读报错文案（seal 后 `ygg.on` 桩与 sealed 双检共用）。
const SEALED_MESSAGE: &str = "ygg.on is sealed: runtime handler registration is forbidden \
 (handlers may only be registered during the load phase)";

/// `ygg.meta` 键长上限（字节）。键是**回执字段名**，不是数据面通道——
/// 上限防「把人读内容塞键名」的滥用回流。
const META_KEY_MAX_BYTES: usize = 64;

/// `ygg.meta` 值嵌套深度上限（含顶层表：1 层）。超限可读报错而非爆栈。
const META_MAX_DEPTH: usize = 8;

/// 宿主侧状态句柄（S9 测试 / S10 驱动的注入面）。
pub struct HostHandles {
    pub kv: Rc<RefCell<KvStore>>,
    pub entities: Rc<RefCell<EntityStore>>,
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
        // ① 清空调用缓冲与 choice 前置计数（上一 handler 的余量不跨 handler 泄漏）。
        {
            let mut state = self.state.borrow_mut();
            state.calls.clear();
            state.pre_choice_emits = 0;
        }
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

/// mlua `Value` 的 Lua 类型名（可读报错用）。
fn lua_type_name(value: &LuaValue) -> &'static str {
    match value {
        LuaValue::Nil => "nil",
        LuaValue::Boolean(_) => "boolean",
        LuaValue::LightUserData(_) => "light userdata",
        LuaValue::Integer(_) => "integer",
        LuaValue::Number(_) => "number",
        LuaValue::String(_) => "string",
        LuaValue::Table(_) => "table",
        LuaValue::Function(_) => "function",
        LuaValue::Thread(_) => "thread",
        LuaValue::UserData(_) => "userdata",
        _ => "unknown",
    }
}

/// core `Value` → mlua `Value`（core→Lua 方向；只映射四个标量）。
fn value_to_lua(lua: &Lua, value: &Value) -> Result<LuaValue, Error> {
    Ok(match value {
        Value::Int(v) => LuaValue::Integer(*v),
        Value::Float(v) => LuaValue::Number(*v),
        Value::Bool(v) => LuaValue::Boolean(*v),
        Value::Str(v) => LuaValue::String(lua.create_string(v)?),
    })
}

/// mlua `Value` → core `Value`（Lua→core 方向）：四个标量直通；
/// table / function / 其他 → 可读 `unsupported payload type` 拒绝。
/// NaN / ±∞ 在此放行为 `Value::Float`，由调用点的 core 门拒绝（事件构造 /
/// KvStore::set / EntityStore::set_attribute 均带定位）。
fn value_from_lua(value: &LuaValue) -> Result<Value, Error> {
    Ok(match value {
        LuaValue::Integer(v) => Value::Int(*v),
        LuaValue::Number(v) => Value::Float(*v),
        LuaValue::Boolean(v) => Value::Bool(*v),
        LuaValue::String(v) => Value::Str(v.to_str()?.to_owned()),
        other => {
            return Err(Error::runtime(format!(
                "unsupported payload type: {}",
                lua_type_name(other)
            )))
        }
    })
}

/// options 必须是字符串的 Lua 序列（数组形态），否则可读报错。
fn options_from_lua(options: &LuaValue) -> Result<Vec<String>, Error> {
    let table = match options {
        LuaValue::Table(t) => t,
        other => {
            return Err(Error::runtime(format!(
                "ygg.choice options must be a sequence of strings, got {}",
                lua_type_name(other)
            )))
        }
    };
    let len = table.raw_len();
    let mut out = Vec::with_capacity(len);
    for i in 1..=len {
        let item: LuaValue = table.raw_get(i)?;
        match &item {
            LuaValue::String(s) => out.push(s.to_str()?.to_owned()),
            other => {
                return Err(Error::runtime(format!(
                    "ygg.choice options[{i}] must be a string, got {}",
                    lua_type_name(other)
                )))
            }
        }
    }
    Ok(out)
}

/// 装载期安装完整 `ygg` 面（D-S9-1）。必须在 `Sandbox` 构造之后调用
/// （依赖 build 创建的 `ygg` 占位表；fuel / inject 之后再装本面不冲突——
/// 各自 raw_set 不同成员）。
pub fn install(lua: &Lua, handles: HostHandles) -> Result<HostApi, Error> {
    let state = Rc::new(RefCell::new(HostState {
        registered: Vec::new(),
        calls: Vec::new(),
        handles,
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
    ygg.raw_set("entity", entity_table)?;

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

/// `ygg.meta` 的键校验：必须是长度 1..=`META_KEY_MAX_BYTES` 的字符串
/// （非字符串 / 空串 / 超长均为可读报错；键是回执字段名，不承载数据）。
fn meta_key_from_lua(key: &LuaValue) -> Result<String, Error> {
    let name = match key {
        LuaValue::String(s) => s.to_str()?.to_owned(),
        other => {
            return Err(Error::runtime(format!(
                "ygg.meta key must be a string, got {}",
                lua_type_name(other)
            )))
        }
    };
    if name.is_empty() {
        return Err(Error::runtime("ygg.meta key must not be empty"));
    }
    if name.len() > META_KEY_MAX_BYTES {
        return Err(Error::runtime(format!(
            "ygg.meta key `{name}` is too long: {} bytes exceeds limit {META_KEY_MAX_BYTES}",
            name.len()
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(Error::runtime(format!(
            "ygg.meta key `{name}` must not contain control characters"
        )));
    }
    Ok(name)
}

/// `ygg.meta` 的值转换：标量走 [`value_from_lua`]（同一套四标量校验），
/// 表递归成 JSON 序列/对象（深度封顶 [`META_MAX_DEPTH`]）。
/// 数值纪律与 core 写入门同源：非有限值拒绝（`-0.0` 经 `normalize_f64`
/// 规范化——meta 是回执面，跨进程字节稳定由规范形承担）。
fn meta_value_from_lua(value: &LuaValue, depth: usize) -> Result<MetaValue, Error> {
    if let LuaValue::Table(table) = value {
        if depth >= META_MAX_DEPTH {
            return Err(Error::runtime(format!(
                "ygg.meta value nesting exceeds {META_MAX_DEPTH} levels"
            )));
        }
        return meta_table_to_json(table, depth + 1);
    }
    meta_scalar_from_lua(value)
}

/// 标量 leaf：`value_from_lua` 把关类型，此处补浮点纪律（非有限值拒绝 +
/// 规范形）。
fn meta_scalar_from_lua(value: &LuaValue) -> Result<MetaValue, Error> {
    Ok(match value_from_lua(value)? {
        Value::Int(v) => MetaValue::from(v),
        Value::Float(v) => {
            if !v.is_finite() {
                return Err(Error::runtime(
                    "ygg.meta value is non-finite (NaN / +inf / -inf are rejected)",
                ));
            }
            match serde_json::Number::from_f64(normalize_f64(v)) {
                Some(number) => MetaValue::Number(number),
                None => {
                    return Err(Error::runtime(
                        "ygg.meta value is non-finite (NaN / +inf / -inf are rejected)",
                    ))
                }
            }
        }
        Value::Bool(v) => MetaValue::Bool(v),
        Value::Str(v) => MetaValue::String(v),
    })
}

/// Lua 表 → JSON：稠密 `1..n` 数值键 = 序列；纯字符串键 = 对象；
/// 混合 / 稀疏数值键 / 其他键型都是可读报错（**不做静默猜测**：
/// 猜测会让同一 Mod 在不同输入下产出不同形状的回执）。
///
/// **空表 = 空序列 `[]`**：Lua 无法区分空表 / 空序列，取「空列表」这一更常用
/// 语义（`conflicts = {}` 读成 `[]`）；需要空对象时给一个键（或读侧按缺键处理）。
fn meta_table_to_json(table: &Table, depth: usize) -> Result<MetaValue, Error> {
    let len = table.raw_len();
    let mut int_keys: Vec<i64> = Vec::new();
    let mut str_keys: Vec<String> = Vec::new();
    for pair in table.pairs::<LuaValue, LuaValue>() {
        let (key, _) = pair?;
        match &key {
            LuaValue::Integer(i) => int_keys.push(*i),
            LuaValue::Number(n) if n.fract() == 0.0 && n.is_finite() => {
                int_keys.push(*n as i64);
            }
            LuaValue::String(s) => str_keys.push(s.to_str()?.to_owned()),
            other => {
                return Err(Error::runtime(format!(
                    "ygg.meta table keys must be integers or strings, got {}",
                    lua_type_name(other)
                )))
            }
        }
    }
    if str_keys.is_empty() {
        int_keys.sort_unstable();
        let dense = int_keys.len() == len
            && int_keys
                .iter()
                .enumerate()
                .all(|(position, key)| *key == position as i64 + 1);
        if !dense {
            return Err(Error::runtime(
                "ygg.meta table is not a dense 1..n sequence (numeric keys must be exactly 1..n)",
            ));
        }
        let mut out = Vec::with_capacity(len);
        for index in 1..=len {
            let item: LuaValue = table.raw_get(index)?;
            out.push(meta_value_from_lua(&item, depth)?);
        }
        return Ok(MetaValue::Array(out));
    }
    if !int_keys.is_empty() {
        return Err(Error::runtime(
            "ygg.meta table mixes array indices and string keys",
        ));
    }
    let mut map = JsonMap::new();
    for key in str_keys {
        let item: LuaValue = table.raw_get(key.as_str())?;
        map.insert(key, meta_value_from_lua(&item, depth)?);
    }
    Ok(MetaValue::Object(map))
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
}
