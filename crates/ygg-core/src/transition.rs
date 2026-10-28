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
//! - **无事务语义（M0 明示）**：handler 执行即提交（实体表就地变更），错误
//!   传播即中断排空、已执行部分不回滚——「无事务面」是 M0 契约的一部分，
//!   转换前后的 `revision_before/after` 与计数是诊断面；
//! - **燃料预算每转换共享**（D-B1）：per-dispatch 重置会让互发 handler 永动；
//!   计量面抽象为 [`FuelMeter`] trait，具体计量（Lua 指令级计数）由沙箱
//!   crate 实现（trait 反转同 D-继承-4，本 crate 不依赖沙箱）。

use std::collections::VecDeque;

use crate::bus::{HandlerError, HandlerRegistry, NarrationSink, PendingChoice};
use crate::entity::EntityStore;
use crate::event::Event;

/// 燃料计量面（D-B1：每转换共享预算，超支即拒绝）。
///
/// 实现方约定：`charge` 只做记账与超支判定，不做执行——执行在调用方
/// （[`run_transition`] 在 charge 成功后才派发事件）。`remaining` 语义由
/// 实现方定义（M0 沙箱侧 = 剩余指令额度）。
pub trait FuelMeter {
    fn charge(&mut self, units: u64) -> Result<(), FuelExhausted>;
}

/// 燃料超支（thiserror，白名单内）。
#[derive(Debug, thiserror::Error)]
#[error("fuel exhausted: requested {requested}, remaining {remaining}")]
pub struct FuelExhausted {
    pub requested: u64,
    pub remaining: u64,
}

/// 转换错误（thiserror，白名单内）。
#[derive(Debug, thiserror::Error)]
pub enum TransitionError {
    #[error("transition input must be an external event, got `{0}`")]
    NotExternal(String),
    #[error(transparent)]
    Handler(#[from] HandlerError),
    #[error(transparent)]
    Fuel(#[from] FuelExhausted),
}

/// 转换结果：输出缓冲、pending_choice、事件计数与 revision 边界（诊断面）。
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
    revision_before: u64,
    revision_after: u64,
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

    /// 转换入场的 store revision。
    pub fn revision_before(&self) -> u64 {
        self.revision_before
    }

    /// 转换出场（排空结束或错误中断前）的 store revision。
    pub fn revision_after(&self) -> u64 {
        self.revision_after
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
/// 流程：外部事件判据先于燃料扣除 → 外部事件派发 → 排空循环
/// （每轮先查 pending_choice 终结 → FIFO 出队 → 扣 1 单位燃料 → 派发）。
/// 燃料超支或 handler 错误即中断传播（无事务面，不回滚已执行部分）。
pub fn run_transition(
    store: &mut EntityStore,
    registry: &mut HandlerRegistry,
    fuel: &mut dyn FuelMeter,
    external: Event,
) -> Result<TransitionResult, TransitionError> {
    // 外部事件判据先于燃料扣除（D-B1：派生事件不能作为转换输入——否则
    // 回放日志「仅外部事件」的前提崩塌）。
    if !external.is_external() {
        return Err(TransitionError::NotExternal(external.name().to_owned()));
    }
    let revision_before = store.revision();
    let mut driver = TransitionDriver::new();
    dispatch(registry, fuel, &external, &mut driver)?;
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
        dispatch(registry, fuel, &event, &mut driver)?;
        events_processed += 1;
    }
    Ok(TransitionResult {
        external,
        narrative: driver.narrative,
        pending_choice: driver.pending_choice,
        events_processed,
        events_derived: driver.events_derived,
        events_dropped,
        revision_before,
        revision_after: store.revision(),
    })
}

/// 单事件派发：先扣 1 单位燃料，成功后才触发 handler（超支即中断，D-B1）。
fn dispatch(
    registry: &mut HandlerRegistry,
    fuel: &mut dyn FuelMeter,
    event: &Event,
    driver: &mut TransitionDriver,
) -> Result<(), TransitionError> {
    fuel.charge(1)?;
    registry.handle_event(event.name(), event, driver)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::EventHandler;
    use crate::entity::Value;
    use std::cell::RefCell;
    use std::rc::Rc;

    type Log = Rc<RefCell<Vec<String>>>;

    /// 预算燃料表：逐次扣减，不足即超支。
    struct BudgetFuel {
        remaining: u64,
        charged_calls: usize,
    }

    impl BudgetFuel {
        fn new(remaining: u64) -> Self {
            Self {
                remaining,
                charged_calls: 0,
            }
        }
    }

    impl FuelMeter for BudgetFuel {
        fn charge(&mut self, units: u64) -> Result<(), FuelExhausted> {
            self.charged_calls += 1;
            if units > self.remaining {
                return Err(FuelExhausted {
                    requested: units,
                    remaining: self.remaining,
                });
            }
            self.remaining -= units;
            Ok(())
        }
    }

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
        let mut store = EntityStore::new();
        let mut reg = HandlerRegistry::new();
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        reg.register("m", "evt.start", scripted("h1", vec![], &log));
        let mut fuel = BudgetFuel::new(10);
        let result = run_transition(
            &mut store,
            &mut reg,
            &mut fuel,
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
        let mut store = EntityStore::new();
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
        let mut fuel = BudgetFuel::new(10);
        let result = run_transition(
            &mut store,
            &mut reg,
            &mut fuel,
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
        let mut store = EntityStore::new();
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
        let mut fuel = BudgetFuel::new(10);
        let result = run_transition(
            &mut store,
            &mut reg,
            &mut fuel,
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
        let mut store = EntityStore::new();
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
        let mut fuel = BudgetFuel::new(10);
        let result = run_transition(
            &mut store,
            &mut reg,
            &mut fuel,
            Event::external("evt.ask", Value::Int(0)).unwrap(),
        )
        .unwrap();
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
    fn fuel_exhaustion_halts_drain_and_propagates() {
        let mut store = EntityStore::new();
        let mut reg = HandlerRegistry::new();
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        // 外部(1) + 派生 evt.a(1) = 预算 2；派发 evt.b 扣费时超支
        reg.register(
            "m",
            "evt.start",
            scripted(
                "ext",
                vec![Action::Emit("evt.a"), Action::Emit("evt.b")],
                &log,
            ),
        );
        reg.register("m", "evt.a", scripted("ha", vec![], &log));
        reg.register("m", "evt.b", scripted("hb", vec![], &log));
        let mut fuel = BudgetFuel::new(2);
        let err = run_transition(
            &mut store,
            &mut reg,
            &mut fuel,
            Event::external("evt.start", Value::Int(0)).unwrap(),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            TransitionError::Fuel(FuelExhausted {
                requested: 1,
                remaining: 0
            })
        ));
        assert_eq!(fuel.remaining, 0);
        // 无事务面：已派发部分不回滚（ext 与 ha 已执行，hb 未执行）
        assert_eq!(
            *log.borrow(),
            vec!["ext:evt.start".to_string(), "ha:evt.a".to_string()]
        );
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
        let mut store = EntityStore::new();
        let mut reg = HandlerRegistry::new();
        reg.register("m", "evt.bad", Box::new(Failing));
        let mut fuel = BudgetFuel::new(10);
        let err = run_transition(
            &mut store,
            &mut reg,
            &mut fuel,
            Event::external("evt.bad", Value::Int(0)).unwrap(),
        )
        .unwrap_err();
        assert!(matches!(err, TransitionError::Handler(_)));
    }

    #[test]
    fn derived_input_rejected_before_any_fuel_charge() {
        let mut store = EntityStore::new();
        let mut reg = HandlerRegistry::new();
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        reg.register("m", "evt.d", scripted("h", vec![], &log));
        let mut fuel = BudgetFuel::new(5);
        let err = run_transition(
            &mut store,
            &mut reg,
            &mut fuel,
            Event::derived("evt.d", Value::Int(1)).unwrap(),
        )
        .unwrap_err();
        // 派生输入被拒（D-B1：转换输入只能是外部事件）
        assert!(matches!(err, TransitionError::NotExternal(name) if name == "evt.d"));
        // 先于燃料扣除：计数与余额均未动
        assert_eq!(fuel.charged_calls, 0);
        assert_eq!(fuel.remaining, 5);
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn revision_boundaries_capture_store_revision_at_entry_and_exit() {
        let mut store = EntityStore::new();
        let mut reg = HandlerRegistry::new();
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        reg.register("m", "evt.tick", scripted("h", vec![], &log));
        let id = store.spawn().unwrap();
        assert_eq!(store.revision(), 1);
        let mut fuel = BudgetFuel::new(10);
        let r1 = run_transition(
            &mut store,
            &mut reg,
            &mut fuel,
            Event::external("evt.tick", Value::Int(0)).unwrap(),
        )
        .unwrap();
        // 内核侧 handler 不直接触达 store（Lua 桥经 host API 走把关面），边界相等
        assert_eq!(r1.revision_before(), 1);
        assert_eq!(r1.revision_after(), 1);
        store.set_attribute(id, "hp", Value::Int(1)).unwrap();
        assert_eq!(store.revision(), 2);
        let r2 = run_transition(
            &mut store,
            &mut reg,
            &mut fuel,
            Event::external("evt.tick", Value::Int(0)).unwrap(),
        )
        .unwrap();
        assert_eq!(r2.revision_before(), 2);
        assert_eq!(r2.revision_after(), 2);
    }

    #[test]
    fn derived_events_are_counted_not_relogged() {
        let mut store = EntityStore::new();
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
        let mut fuel = BudgetFuel::new(10);
        let result = run_transition(
            &mut store,
            &mut reg,
            &mut fuel,
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
