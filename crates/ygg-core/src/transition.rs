//! 转换驱动：`(state, event) → state'` 单点收口（FR2 / D-B1 / D-B2）。
//!
//! - **FIFO 级联**：handler 发出的派生事件入 per-transition FIFO，内核在同一
//!   转换内按入队序排空；派生事件可再触发派生事件（递归深度界于一帧 Lua 栈，
//!   FIFO 序天然确定）；
//! - **choice 终结裁定**（D-B2）：排空循环每轮**先查 `pending_choice`**——
//!   已设即终结转换：**首个 choice 胜出**（后续 choose 调用被忽略，first-wins
//!   语义见驱动器对 [`NarrationSink`] 的实现），FIFO 剩余事件全部丢弃并计入
//!   `events_dropped`（诊断面，不静默）；
//! - **回放面**（D-B1）：派生事件不进回放日志——回放 = 外部事件日志重放时
//!   自动再产生派生事件；[`TransitionResult`] 只携带计数、输出缓冲与
//!   pending_choice，不携带事件日志（B3：事件日志旁路不入档，日志面归调用方）；
//! - **无事务语义（M0 明示，责任归调用方）**：handler 执行即提交（世界状态就地变更），
//!   错误传播即中断排空、已执行部分不回滚——**本函数不做事务，因为它不持有
//!   状态所有权**：M1-D 起它连一个状态形参都没有（原 `&mut EntityStore` 形参
//!   只用于 revision 记账，行存删除后一并消失）：Lua handler 的世界/KV 写经
//!   `Rc<RefCell<..>>` 直落 canonical 槽，根本不经过本函数。
//!   事务边界因此归调用方——`ygg-sandbox::driver::cmd_event` 在 dispatch 层
//!   实现 savepoint（取点冻结世界快照 + KV，失败整体还原；见
//!   `crates/ygg-sandbox/src/driver/dispatch.rs` ④/⑤ 段与模块文档
//!   「dispatch 原子性」）。本 crate 的假 handler 用例以「无事务」形态验证
//!   core 自身契约；转换前后的 tick/revision 是调用方的诊断面，不在这里。
//! - **无燃料计量**（S34 裁定）：燃料机制整体退役（用户裁定「限制远大于
//!   收益」），转换驱动不再携带计量面——handler 错误是唯一的转换中断源。

use std::collections::VecDeque;

use crate::bus::{HandlerError, HandlerRegistry, NarrationSink, PendingChoice};
use crate::event::Event;

/// 转换错误（thiserror，白名单内）。
#[derive(Debug, thiserror::Error)]
pub enum TransitionError {
    #[error("transition input must be an external event, got `{0}`")]
    NotExternal(String),
    #[error(transparent)]
    Handler(#[from] HandlerError),
}

/// 转换结果：输出缓冲、pending_choice 与事件计数（诊断面）。
///
/// 不携带事件日志（D-B1/B3：派生事件不进回放日志，日志面归调用方）。
#[derive(Clone, Debug)]
pub struct TransitionResult {
    external: Event,
    narrative: Vec<String>,
    pending_choice: Option<PendingChoice>,
    events_processed: u64,
    events_derived: u64,
    events_dropped: u64,
}

impl TransitionResult {
    /// 本次转换的外部事件（回放日志的记录对象，由调用方记账）。
    pub fn external(&self) -> &Event {
        &self.external
    }

    /// 叙述输出缓冲（`narrate` 累积序）。
    pub fn narrative(&self) -> &[String] {
        &self.narrative
    }

    /// 待决选择（choice 终结时 `Some`；选择 = 下一个外部事件，D-B2）。
    pub fn pending_choice(&self) -> Option<&PendingChoice> {
        self.pending_choice.as_ref()
    }

    /// 实际派发的事件数（外部 1 + FIFO 排空数；不含被 choice 终结丢弃的余量）。
    pub fn events_processed(&self) -> u64 {
        self.events_processed
    }

    /// handler 发出的派生事件总数（含被终结丢弃的余量）。
    pub fn events_derived(&self) -> u64 {
        self.events_derived
    }

    /// 被 choice 终结丢弃的 FIFO 余量（诊断面，不静默）。
    pub fn events_dropped(&self) -> u64 {
        self.events_dropped
    }
}

/// 转换内驱动器：per-transition FIFO + 输出缓冲 + pending_choice。
///
/// 实现 [`EventSink`]（emit → FIFO 入队 + 派生计数）与 [`NarrationSink`]
/// （narrate → 缓冲累积；choose → first-wins：首个 choice 胜出）。
struct TransitionDriver {
    queue: VecDeque<Event>,
    narrative: Vec<String>,
    pending_choice: Option<PendingChoice>,
    events_derived: u64,
}

impl TransitionDriver {
    fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            narrative: Vec::new(),
            pending_choice: None,
            events_derived: 0,
        }
    }
}

impl crate::bus::EventSink for TransitionDriver {
    fn emit(&mut self, event: Event) {
        self.queue.push_back(event);
        self.events_derived += 1;
    }
}

impl NarrationSink for TransitionDriver {
    fn narrate(&mut self, line: String) {
        self.narrative.push(line);
    }

    fn choose(&mut self, choice: PendingChoice) {
        // first-wins：首个 choice 胜出（后续 choose 调用被忽略）
        if self.pending_choice.is_none() {
            self.pending_choice = Some(choice);
        }
    }
}

/// 执行一次 `(state, event) → state'` 转换（FR2 单点收口）。
///
/// 流程：外部事件判据先行 → 外部事件派发 → 排空循环
/// （每轮先查 pending_choice 终结 → FIFO 出队 → 派发）。
/// handler 错误即中断传播（无事务面，不回滚已执行部分）。
///
/// **事务边界归调用方**：本函数只驱动派发，不持有状态所有权，也没有状态形参。
/// Lua facade 路径下 handler 的世界/KV 写落在 canonical 槽，回滚由调用方承担：
/// `ygg-sandbox::driver::cmd_event` 在 dispatch 层取 savepoint，失败时整体
/// 还原世界快照 + `kv`（见该函数 ⑤/⑥ 段与模块文档「dispatch 原子性」）。
pub fn run_transition(
    registry: &mut HandlerRegistry,
    external: Event,
) -> Result<TransitionResult, TransitionError> {
    // 外部事件判据先行（D-B1：派生事件不能作为转换输入——否则
    // 回放日志「仅外部事件」的前提崩塌）。
    if !external.is_external() {
        return Err(TransitionError::NotExternal(external.name().to_owned()));
    }
    let mut driver = TransitionDriver::new();
    dispatch(registry, &external, &mut driver)?;
    let mut events_processed = 1;
    let mut events_dropped = 0;
    loop {
        // 终结裁定先行（choice-as-state：挂起即数据，玩家选择是下一个外部事件）
        if driver.pending_choice.is_some() {
            events_dropped = driver.queue.len() as u64;
            driver.queue.clear();
            break;
        }
        let Some(event) = driver.queue.pop_front() else {
            break;
        };
        dispatch(registry, &event, &mut driver)?;
        events_processed += 1;
    }
    Ok(TransitionResult {
        external,
        narrative: driver.narrative,
        pending_choice: driver.pending_choice,
        events_processed,
        events_derived: driver.events_derived,
        events_dropped,
    })
}

/// 单事件派发：触发 handler（handler 错误即中断，D-B1）。
fn dispatch(
    registry: &mut HandlerRegistry,
    event: &Event,
    driver: &mut TransitionDriver,
) -> Result<(), TransitionError> {
    registry.handle_event(event.name(), event, driver)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::EventHandler;
    use crate::value::Value;
    use std::cell::RefCell;
    use std::rc::Rc;

    type Log = Rc<RefCell<Vec<String>>>;
    /// handler 脚本动作：叙述 / 选择 / 发派生事件。
    enum Action {
        Narrate(&'static str),
        Choose(&'static str),
        Emit(&'static str),
    }

    /// 按脚本动作执行的 fake handler（harness 先例：S9 fake-host）。
    struct Scripted {
        tag: &'static str,
        actions: Vec<Action>,
        log: Log,
    }

    impl EventHandler for Scripted {
        fn handle(
            &mut self,
            event: &Event,
            sink: &mut dyn NarrationSink,
        ) -> Result<(), HandlerError> {
            self.log
                .borrow_mut()
                .push(format!("{}:{}", self.tag, event.name()));
            for action in &self.actions {
                match action {
                    Action::Narrate(line) => sink.narrate(format!("{}:{line}", self.tag)),
                    Action::Choose(prompt) => sink.choose(PendingChoice::new(
                        format!("{}:{prompt}", self.tag),
                        vec!["yes".to_string(), "no".to_string()],
                    )),
                    Action::Emit(name) => {
                        sink.emit(Event::derived(*name, Value::Int(1)).unwrap());
                    }
                }
            }
            Ok(())
        }
    }

    fn scripted(tag: &'static str, actions: Vec<Action>, log: &Log) -> Box<dyn EventHandler> {
        Box::new(Scripted {
            tag,
            actions,
            log: Rc::clone(log),
        })
    }

    #[test]
    fn external_event_drives_registered_handler() {
        let mut reg = HandlerRegistry::new();
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        reg.register("m", "evt.start", scripted("h1", vec![], &log));
        let result = run_transition(
            &mut reg,
            Event::external("evt.start", Value::Str("go".into())).unwrap(),
        )
        .unwrap();
        assert_eq!(*log.borrow(), vec!["h1:evt.start".to_string()]);
        assert_eq!(result.events_processed(), 1);
        assert_eq!(result.events_derived(), 0);
        assert!(result.narrative().is_empty());
        assert!(result.pending_choice().is_none());
        assert_eq!(result.external().name(), "evt.start");
    }

    #[test]
    fn fifo_drains_derived_events_in_push_order() {
        let mut reg = HandlerRegistry::new();
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        // 外部 handler 先入队 evt.d2 再入队 evt.d1（入队顺序 = 排空顺序）
        reg.register(
            "m",
            "evt.start",
            scripted(
                "ext",
                vec![Action::Emit("evt.d2"), Action::Emit("evt.d1")],
                &log,
            ),
        );
        reg.register("m", "evt.d2", scripted("h_d2", vec![], &log));
        reg.register("m", "evt.d1", scripted("h_d1", vec![], &log));
        let result = run_transition(
            &mut reg,
            Event::external("evt.start", Value::Int(0)).unwrap(),
        )
        .unwrap();
        assert_eq!(
            *log.borrow(),
            vec![
                "ext:evt.start".to_string(),
                "h_d2:evt.d2".to_string(),
                "h_d1:evt.d1".to_string(),
            ]
        );
        assert_eq!(result.events_processed(), 3);
        assert_eq!(result.events_derived(), 2);
        assert_eq!(result.events_dropped(), 0);
    }

    #[test]
    fn narrative_accumulates_in_dispatch_order() {
        let mut reg = HandlerRegistry::new();
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        // 同事件双 handler（注册序）+ 派生事件 handler 的叙述累积
        reg.register(
            "m",
            "evt.start",
            scripted(
                "b1",
                vec![Action::Narrate("first"), Action::Emit("evt.loop")],
                &log,
            ),
        );
        reg.register(
            "m",
            "evt.start",
            scripted("a1", vec![Action::Narrate("second")], &log),
        );
        reg.register(
            "m",
            "evt.loop",
            scripted("c1", vec![Action::Narrate("third")], &log),
        );
        let result = run_transition(
            &mut reg,
            Event::external("evt.start", Value::Int(0)).unwrap(),
        )
        .unwrap();
        assert_eq!(
            result.narrative(),
            [
                "b1:first".to_string(),
                "a1:second".to_string(),
                "c1:third".to_string()
            ]
        );
    }

    #[test]
    fn pending_choice_terminates_drain_and_reports_dropped() {
        let mut reg = HandlerRegistry::new();
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        // handler：choose → choose（第二个被忽略）→ emit（FIFO 将残留一个派生事件）
        reg.register(
            "m",
            "evt.ask",
            scripted(
                "h",
                vec![
                    Action::Choose("go?"),
                    Action::Choose("late?"),
                    Action::Emit("evt.after"),
                ],
                &log,
            ),
        );
        reg.register("m", "evt.after", scripted("h_after", vec![], &log));
        let result =
            run_transition(&mut reg, Event::external("evt.ask", Value::Int(0)).unwrap()).unwrap();
        // first-wins：首个 choice 胜出
        let choice = result.pending_choice().unwrap();
        assert_eq!(choice.prompt(), "h:go?");
        assert_eq!(choice.options(), ["yes".to_string(), "no".to_string()]);
        // 排空停在 choice：终结后未派发任何事件，FIFO 余量丢弃并计数
        assert_eq!(result.events_processed(), 1);
        assert_eq!(result.events_dropped(), 1);
        assert_eq!(result.events_derived(), 1);
        assert_eq!(*log.borrow(), vec!["h:evt.ask".to_string()]);
    }

    #[test]
    fn handler_error_halts_drain_and_propagates() {
        struct Failing;
        impl EventHandler for Failing {
            fn handle(
                &mut self,
                _e: &Event,
                _s: &mut dyn NarrationSink,
            ) -> Result<(), HandlerError> {
                Err(HandlerError("bridge aborted".into()))
            }
        }
        let mut reg = HandlerRegistry::new();
        reg.register("m", "evt.bad", Box::new(Failing));
        let err = run_transition(&mut reg, Event::external("evt.bad", Value::Int(0)).unwrap())
            .unwrap_err();
        assert!(matches!(err, TransitionError::Handler(_)));
    }

    #[test]
    fn derived_input_rejected_before_any_dispatch() {
        let mut reg = HandlerRegistry::new();
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        reg.register("m", "evt.d", scripted("h", vec![], &log));
        let err =
            run_transition(&mut reg, Event::derived("evt.d", Value::Int(1)).unwrap()).unwrap_err();
        // 派生输入被拒（D-B1：转换输入只能是外部事件）
        assert!(matches!(err, TransitionError::NotExternal(name) if name == "evt.d"));
        // 先于任何派发：handler 未执行
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn derived_events_are_counted_not_relogged() {
        let mut reg = HandlerRegistry::new();
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        reg.register(
            "m",
            "evt.start",
            scripted(
                "ext",
                vec![
                    Action::Emit("evt.d1"),
                    Action::Emit("evt.d2"),
                    Action::Emit("evt.d3"),
                ],
                &log,
            ),
        );
        // 派发面 fail-closed：派生名也必须注册（无操作 handler，仅让 FIFO
        // 排空发生——本测的意图是「计数而非回放」，不是「未注册静默跳过」）。
        reg.register("m", "evt.d1", scripted("h_d1", vec![], &log));
        reg.register("m", "evt.d2", scripted("h_d2", vec![], &log));
        reg.register("m", "evt.d3", scripted("h_d3", vec![], &log));
        let result = run_transition(
            &mut reg,
            Event::external("evt.start", Value::Int(0)).unwrap(),
        )
        .unwrap();
        // D-B1：派生事件只计数（结果不携带事件日志——回放面归调用方，B3）
        assert_eq!(result.events_derived(), 3);
        assert_eq!(result.events_processed(), 4);
        assert_eq!(result.events_dropped(), 0);
        assert!(result.pending_choice().is_none());
        assert!(result.external().is_external());
    }
}
