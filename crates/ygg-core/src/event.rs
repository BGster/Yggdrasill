//! 事件模型：事件（名称 + 规范 payload + 来源标志）（FR2 / D-B1）。
//!
//! 确定性纪律（同 S2 / research §3、§8）：
//! - payload 复用 S2 的 `Value`：数字编码即规范（浮点构造即拒非有限值（NaN/±∞）、
//!   `-0.0` 入构造即规范化为 `0.0`、编码侧手写 `Serialize` 兑底）——
//!   跨平台数字编码分歧通道在类型面上关闭；复合结构属资源面，走 `ygg.data`（S11）；
//! - 来源标志 `Origin`（D-B1）：外部事件是回放日志的唯一记录对象，
//!   派生事件回放时由重放自动产生——标志在本 crate 进入事件模型，
//!   供 S4 转换驱动与存档/日志面按来源分流。

use serde::{Deserialize, Serialize};

use crate::entity::{normalize_f64, Value};

/// 事件来源（D-B1：回放日志仅记录外部事件）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Origin {
    /// 外部事件：玩家输入 / 宿主注入，是回放日志的唯一记录对象。
    External,
    /// 派生事件：handler 执行期间发出，入 per-transition FIFO，回放时自动再产生。
    Derived,
}

/// 事件构造错误（thiserror，白名单内）。
#[derive(Debug, thiserror::Error)]
pub enum EventError {
    #[error("payload of event `{0}` is non-finite")]
    NonFinitePayload(String),
}

/// 事件：名称 + 单标量 payload + 来源标志。
///
/// payload 取 S2 `Value`（数字/布尔/字符串标量）——载荷序列化即规范
/// （`Value` 的手写 `Serialize` 已含 `-0.0 → 0.0` 兑底）；构造路径与 S2
/// 同纪律：非有限值拒绝、`-0.0` 入构造即规范化（传递值即规范形）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    name: String,
    payload: Value,
    origin: Origin,
}

impl Event {
    /// 外部事件构造（回放日志的记录对象）。
    pub fn external(name: impl Into<String>, payload: Value) -> Result<Self, EventError> {
        Self::build(name.into(), payload, Origin::External)
    }

    /// 派生事件构造（handler 经宿主 emit 发出；S4 FIFO 消费）。
    pub fn derived(name: impl Into<String>, payload: Value) -> Result<Self, EventError> {
        Self::build(name.into(), payload, Origin::Derived)
    }

    fn build(name: String, payload: Value, origin: Origin) -> Result<Self, EventError> {
        if let Value::Float(f) = &payload {
            if !f.is_finite() {
                return Err(EventError::NonFinitePayload(name));
            }
        }
        let payload = match payload {
            Value::Float(f) => Value::Float(normalize_f64(f)),
            other => other,
        };
        Ok(Self {
            name,
            payload,
            origin,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn payload(&self) -> &Value {
        &self.payload
    }

    pub fn origin(&self) -> Origin {
        self.origin
    }

    /// 是否外部事件（回放日志的记录判据，D-B1）。
    pub fn is_external(&self) -> bool {
        self.origin == Origin::External
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_carry_name_payload_origin() {
        let e = Event::external("event.start", Value::Str("feni".into())).unwrap();
        assert_eq!(e.name(), "event.start");
        assert_eq!(e.payload(), &Value::Str("feni".into()));
        assert_eq!(e.origin(), Origin::External);
        assert!(e.is_external());

        let d = Event::derived("evt.derived", Value::Int(3)).unwrap();
        assert_eq!(d.origin(), Origin::Derived);
        assert!(!d.is_external());
    }

    #[test]
    fn nan_payload_is_rejected_with_event_name() {
        let err = Event::external("evt.bad", Value::Float(f64::NAN)).unwrap_err();
        assert!(err.to_string().contains("evt.bad"));
    }

    #[test]
    fn infinity_payload_is_rejected() {
        assert!(matches!(
            Event::external("evt.inf", Value::Float(f64::INFINITY)),
            Err(EventError::NonFinitePayload(_))
        ));
        assert!(matches!(
            Event::derived("evt.ninf", Value::Float(f64::NEG_INFINITY)),
            Err(EventError::NonFinitePayload(_))
        ));
    }

    #[test]
    fn negative_zero_payload_is_normalized_on_construction() {
        let e = Event::external("evt.bias", Value::Float(-0.0)).unwrap();
        match e.payload() {
            Value::Float(f) => {
                assert_eq!(*f, 0.0);
                assert!(f.is_sign_positive());
            }
            other => panic!("expected Float(0.0), got {other:?}"),
        }
    }

    #[test]
    fn serde_round_trip_preserves_origin_and_payload() {
        let e = Event::derived("evt.hit", Value::float(1.5).unwrap()).unwrap();
        let bytes = serde_json::to_vec(&e).unwrap();
        let back: Event = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(e, back);
    }

    #[test]
    fn serialized_bytes_are_canonical_for_payload() {
        let e = Event::external("evt.bias", Value::Float(-0.0)).unwrap();
        let s = String::from_utf8(serde_json::to_vec(&e).unwrap()).unwrap();
        assert!(!s.contains("-0.0"));
    }
}
