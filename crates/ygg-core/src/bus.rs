//! 触发注册表 + 事件 handler trait（D-继承-4 trait 反转面 / D-B1）。
//!
//! - `EventHandler`：本 crate 只依赖自身 trait，不依赖沙箱 crate——
//!   Lua handler 桥由 ygg-sandbox 实现，driver 持有两者接线（DAG 无环）；
//! - `EventSink`：handler 发出派生事件的出口（实现方 = S4 转换驱动的
//!   per-transition FIFO），handler 本体不直接触达内核状态；
//! - `NarrationSink`（S4 amendment，D-B2）：`EventSink` 的叙述/选择扩展面——
//!   `narrate` 为纯输出累积（per-transition 输出缓冲，转换结果的一部分）；
//!   `choose` 以 `PendingChoice` 设入内核状态并终结当前 handler 执行
//!   （挂起即数据，玩家选择 = 新外部事件）；handler 面与注册表派发面的
//!   sink 签名收窄为 `&mut dyn NarrationSink`；
//! - 注册表规范序 = 注册序（桶内 `Vec` 顺序；事件名经 `BTreeMap` 分桶），
//!   每条注册持稳定键 `(mod id, 注册序号)`——身份、tiebreak 与诊断核对面，
//!   全程无哈希容器参与（D-B1「永不哈希容器」）；
//! - 注册仅发生在 Mod 装载期（handler 集 = 装载的纯函数，回放日志缩小为
//!   仅外部事件的前提）；运行期注册禁令的权威执行面在 S9/S10 host API
//!   （D-B4 双保险的运行时侧），本 crate 不承载运行期拦截。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::event::Event;

/// handler 稳定键：Mod 标识 + 装载期全局单调分配的注册序号（D-B1 tiebreak）。
///
/// 装载序确定性由上游保证（D-A2：Mod 按 sorted manifest 序装载），
/// 故 `(mod id, 注册序号)` 全局唯一且可复现；键只作身份与诊断面，
/// handler 遍历序一律以桶内 `Vec` 的注册序为准。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct HandlerKey {
    pub mod_id: String,
    pub seq: u64,
}

/// handler 执行错误（反转面最小形态：沙箱桥可携带定位信息扩展消息体）。
#[derive(Debug, thiserror::Error)]
#[error("event handler failed: {0}")]
pub struct HandlerError(pub String);

/// handler 发出派生事件的接收面（S4 转换驱动以 per-transition FIFO 实现）。
pub trait EventSink {
    /// 发出一个派生事件；入当前转换的 FIFO，内核在同一 `(state,event)→state'`
    /// 转换内排空（D-B1；燃料预算每转换共享，S4 承载）。
    fn emit(&mut self, event: Event);
}

/// 叙述/选择扩展面（D-B2）：handler 的输出与终结通道（实现方 = S4 转换驱动）。
pub trait NarrationSink: EventSink {
    /// 纯输出累积：写入 per-transition 叙述缓冲（不触达内核状态）。
    fn narrate(&mut self, line: String);
    /// 设入待决选择并终结当前 handler 执行（first-wins 裁定在驱动层：
    /// 首个 choose 胜出；桥实现方应在首个 choose 后终止执行——B2 挂起即数据）。
    fn choose(&mut self, choice: PendingChoice);
}

/// 待决选择（D-B2 choice-as-state 的状态载体）：纯数据，可序列化（入存档面）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingChoice {
    prompt: String,
    options: Vec<String>,
}

impl PendingChoice {
    pub fn new(prompt: impl Into<String>, options: Vec<String>) -> Self {
        Self {
            prompt: prompt.into(),
            options,
        }
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    pub fn options(&self) -> &[String] {
        &self.options
    }
}

/// 事件 handler 抽象（反转面）：ygg-sandbox 的 Lua handler 桥实现本 trait。
pub trait EventHandler {
    /// 处理单个事件。派生事件经 `sink.emit` 发出；pending_choice 类终结场景
    /// 由实现方在设状态后终止后续执行（B2 choice-as-state，挂起即数据）。
    fn handle(&mut self, event: &Event, sink: &mut dyn NarrationSink) -> Result<(), HandlerError>;
}

/// 单条注册：稳定键 + handler 实例（handler 为代码，不入存档，B3）。
struct Registration {
    key: HandlerKey,
    handler: Box<dyn EventHandler>,
}

/// 按 (事件名 → 注册序列) 分桶的 handler 注册表。
///
/// 分桶容器为 `BTreeMap`（事件名字典序仅作诊断遍历序）；同一事件的
/// handler 遍历序 = 桶内 `Vec` 的注册序（D-B1 规范序）。
pub struct HandlerRegistry {
    buckets: BTreeMap<String, Vec<Registration>>,
    next_seq: u64,
}

impl Default for HandlerRegistry {
    /// 显式委托 `new()`：注册序号从 0 起单调分配。
    fn default() -> Self {
        Self::new()
    }
}

impl HandlerRegistry {
    pub fn new() -> Self {
        Self {
            buckets: BTreeMap::new(),
            next_seq: 0,
        }
    }

    /// 装载期注册（D-B1/D-B4：运行期禁令执行面在 S9/S10 host API）。
    /// 返回本次注册的稳定键 `(mod id, 注册序号)`；同事件桶内追加，序 = 注册序。
    pub fn register(
        &mut self,
        mod_id: impl Into<String>,
        event_name: impl Into<String>,
        handler: Box<dyn EventHandler>,
    ) -> HandlerKey {
        let seq = self.next_seq;
        self.next_seq += 1;
        let key = HandlerKey {
            mod_id: mod_id.into(),
            seq,
        };
        self.buckets
            .entry(event_name.into())
            .or_default()
            .push(Registration {
                key: key.clone(),
                handler,
            });
        key
    }

    /// 按注册序驱动某事件的全部 handler（S4 转换驱动的触发面）。
    /// 任一 handler 出错即中止本事件派发并返回该错误（错误传播语义；
    /// 中断后 FIFO 余量是否继续排空由 S4 转换驱动裁定）。
    pub fn handle_event(
        &mut self,
        event_name: &str,
        event: &Event,
        sink: &mut dyn NarrationSink,
    ) -> Result<(), HandlerError> {
        if let Some(bucket) = self.buckets.get_mut(event_name) {
            for reg in bucket.iter_mut() {
                reg.handler.handle(event, sink)?;
            }
        }
        Ok(())
    }

    /// 某事件的注册键序列（诊断/测试面：注册序可核对性）。
    pub fn keys(&self, event_name: &str) -> Vec<HandlerKey> {
        self.buckets
            .get(event_name)
            .map(|bucket| bucket.iter().map(|reg| reg.key.clone()).collect())
            .unwrap_or_default()
    }

    /// 已注册事件名（字典序，诊断面）。
    pub fn event_names(&self) -> impl Iterator<Item = &str> {
        self.buckets.keys().map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.buckets.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::Value;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// 记录收到事件名的 fake handler（harness 模式先例：S9 fake-host）。
    struct Recording {
        tag: &'static str,
        log: Rc<RefCell<Vec<String>>>,
    }

    impl EventHandler for Recording {
        fn handle(
            &mut self,
            event: &Event,
            _sink: &mut dyn NarrationSink,
        ) -> Result<(), HandlerError> {
            self.log
                .borrow_mut()
                .push(format!("{}:{}", self.tag, event.name()));
            Ok(())
        }
    }

    fn recorder(tag: &'static str, log: &Rc<RefCell<Vec<String>>>) -> Box<dyn EventHandler> {
        Box::new(Recording {
            tag,
            log: Rc::clone(log),
        })
    }

    /// 吞掉派生事件的空 sink。
    struct NullSink;
    impl EventSink for NullSink {
        fn emit(&mut self, _event: Event) {}
    }

    // S4 amendment：签名收窄为 `&mut dyn NarrationSink` 后，测试 sink 补空实现
    // （叙述/选择不进这些测试的关注面）。
    impl NarrationSink for NullSink {
        fn narrate(&mut self, _line: String) {}
        fn choose(&mut self, _choice: PendingChoice) {}
    }

    #[test]
    fn keys_are_unique_and_monotonic_across_mods_and_events() {
        let mut reg = HandlerRegistry::new();
        let log = Rc::new(RefCell::new(Vec::new()));
        let k1 = reg.register("mod_b", "evt.x", recorder("b1", &log));
        let k2 = reg.register("mod_a", "evt.x", recorder("a1", &log));
        let k3 = reg.register("mod_a", "evt.y", recorder("a2", &log));
        assert_eq!(k1.seq, 0);
        assert_eq!(k2.seq, 1);
        assert_eq!(k3.seq, 2);
        assert_eq!(k1.mod_id, "mod_b");
        // 注册序 = 调用序：跨 Mod 的同名 handler 也不重排（D-B1 规范序）
        assert_eq!(reg.keys("evt.x"), vec![k1, k2]);
        assert!(reg.keys("evt.missing").is_empty());
    }

    #[test]
    fn handlers_fire_in_registration_order_per_event() {
        let mut reg = HandlerRegistry::new();
        let log = Rc::new(RefCell::new(Vec::new()));
        reg.register("mod_b", "evt.x", recorder("b1", &log));
        reg.register("mod_a", "evt.x", recorder("a1", &log));
        reg.register("mod_a", "evt.y", recorder("a2", &log));
        let mut sink = NullSink;
        let e = Event::external("evt.x", Value::Int(1)).unwrap();
        reg.handle_event("evt.x", &e, &mut sink).unwrap();
        assert_eq!(
            *log.borrow(),
            vec!["b1:evt.x".to_string(), "a1:evt.x".to_string()]
        );
    }

    #[test]
    fn event_names_iterate_sorted_and_empty_registry_reports_empty() {
        let mut reg = HandlerRegistry::new();
        assert!(reg.is_empty());
        let log = Rc::new(RefCell::new(Vec::new()));
        reg.register("m", "evt.z", recorder("h1", &log));
        reg.register("m", "evt.a", recorder("h2", &log));
        let names: Vec<_> = reg.event_names().collect();
        assert_eq!(names, ["evt.a", "evt.z"]);
        assert!(!reg.is_empty());
    }

    #[test]
    fn handle_event_on_unknown_event_is_ok() {
        // 未知事件：空桶直接 Ok（派发面与 keys() 诊断面一致）
        let mut reg = HandlerRegistry::new();
        let log = Rc::new(RefCell::new(Vec::new()));
        reg.register("m", "evt.x", recorder("h1", &log));
        let mut sink = NullSink;
        let e = Event::external("evt.missing", Value::Int(1)).unwrap();
        assert!(reg.handle_event("evt.missing", &e, &mut sink).is_ok());
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn handler_error_propagates_from_trait_face() {
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
        reg.register("m", "evt.f", Box::new(Failing));
        let mut sink = NullSink;
        let e = Event::external("evt.f", Value::Bool(true)).unwrap();
        let err = reg.handle_event("evt.f", &e, &mut sink).unwrap_err();
        assert!(err.to_string().contains("bridge aborted"));
    }

    #[test]
    fn sink_receives_derived_events_emitted_by_handler() {
        struct Emitting;
        impl EventHandler for Emitting {
            fn handle(
                &mut self,
                _e: &Event,
                sink: &mut dyn NarrationSink,
            ) -> Result<(), HandlerError> {
                sink.emit(Event::derived("evt.derived", Value::Int(7)).unwrap());
                Ok(())
            }
        }
        struct Collect(Vec<Event>);
        impl EventSink for Collect {
            fn emit(&mut self, event: Event) {
                self.0.push(event);
            }
        }
        impl NarrationSink for Collect {
            fn narrate(&mut self, _line: String) {}
            fn choose(&mut self, _choice: PendingChoice) {}
        }
        let mut reg = HandlerRegistry::new();
        reg.register("m", "evt.src", Box::new(Emitting));
        let mut sink = Collect(Vec::new());
        let e = Event::external("evt.src", Value::Int(1)).unwrap();
        reg.handle_event("evt.src", &e, &mut sink).unwrap();
        assert_eq!(sink.0.len(), 1);
        assert_eq!(sink.0[0].name(), "evt.derived");
        assert!(!sink.0[0].is_external());
    }
}
