//! 叙事收集-回放桥:`ygg.on/emit/narrate/choice` 注册、core `EventHandler` 反转桥
//! (LuaHandler)与 choice sentinel 识别(S-25 批三自 host_api 单文件迁出)。
use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Error, Function, Lua, Table, Value as LuaValue};
use ygg_core::bus::{EventHandler, HandlerError, NarrationSink, PendingChoice};
use ygg_core::event::Event;

use super::entity_decode::options_from_lua;
use super::{value_from_lua, value_to_lua, HostCall, HostState, Phase};

/// 运行期注册禁令的可读报错文案（seal 后 `ygg.on` 桩与 sealed 双检共用）。
pub(super) const SEALED_MESSAGE: &str =
    "ygg.on is sealed: runtime handler registration is forbidden \
 (handlers may only be registered during the load phase)";

/// 装载期注册的单个 Lua handler 的桥实现（core `EventHandler` 反转面）。
pub(super) struct LuaHandler {
    pub(super) lua: Lua,
    pub(super) function: Function,
    pub(super) state: Rc<RefCell<HostState>>,
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
pub(super) fn is_choice_sentinel(sentinel: &str, err: &Error) -> bool {
    match err {
        Error::RuntimeError(msg) => msg.split('\n').next() == Some(sentinel),
        Error::CallbackError { cause, .. } => is_choice_sentinel(sentinel, cause),
        _ => false,
    }
}

/// `ygg.on/emit/narrate/choice` 注册面(D-S9-1..4;S-25 批三自 install() 原文迁入)。
pub(super) fn install_narrate(
    lua: &Lua,
    state: &Rc<RefCell<HostState>>,
    ygg: &Table,
) -> Result<(), Error> {
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

    Ok(())
}
