//! 持久 KV：host API 背书的键值存储（FR4 持久面 / FR7：持久状态只存 Rust 侧）。
//!
//! 确定性纪律（同 S2，research §3）：
//! - `BTreeMap<String, Value>`：键字典序 = 天然规范遍历序（禁用随机化哈希的
//!   std 默认无序容器，确定性路径一律有序容器）；
//! - 值复用 S2 `Value`（标量；构造/写入路径的浮点纪律同 S2——非有限值拒绝、
//!   `-0.0` 入表前规范化）；
//! - `revision` 语义与 `EntityStore` 同口径：仅**实际状态变更**计 +1
//!   （同值覆写幂等不计），存档面（S5 `save.rs`）与状态哈希消费该边界。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::entity::{normalize_f64, Value};

/// 持久 KV 错误（thiserror，白名单内）。
#[derive(Debug, thiserror::Error)]
pub enum KvError {
    #[error("kv value for key `{0}` is non-finite")]
    NonFiniteF64(String),
}

/// 持久键值存储：键值对 + 版本号（FR4 持久面；随存档入快照，B3）。
///
/// 所有变更只能经本层把关方法进行（无内部表可变借用出口）——非有限值门、
/// `-0.0` 规范化与 revision 推进单点收口，纪律与 `EntityStore` 一致。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct KvStore {
    entries: BTreeMap<String, Value>,
    revision: u64,
}

impl KvStore {
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            revision: 0,
        }
    }

    /// 把关写值：非有限值（NaN/±∞）拒绝为错误（`-0.0` 入表前规范化为 `0.0`）；
    /// 同值覆写为幂等写，不计变更。
    pub fn set(&mut self, key: impl Into<String>, value: Value) -> Result<(), KvError> {
        let value = match value {
            Value::Float(f) => {
                if !f.is_finite() {
                    return Err(KvError::NonFiniteF64(key.into()));
                }
                Value::Float(normalize_f64(f))
            }
            other => other,
        };
        let key = key.into();
        if self.entries.get(&key) == Some(&value) {
            return Ok(()); // 幂等写：状态未变，不计变更
        }
        self.entries.insert(key, value);
        self.revision += 1;
        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries.get(key)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    /// 把关删键；实际删除才计变更（返回被删值，不存在时 `None` 且不计）。
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        let removed = self.entries.remove(key);
        if removed.is_some() {
            self.revision += 1;
        }
        removed
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 按键字典序遍历（规范序）。
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// 存储版本号：每次实际状态变更 +1（语义见结构体文档）。
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_remove_round_trip() {
        let mut kv = KvStore::new();
        assert!(kv.is_empty());
        kv.set("gold", Value::Int(30)).unwrap();
        kv.set("name", Value::Str("feni".into())).unwrap();
        assert_eq!(kv.get("gold"), Some(&Value::Int(30)));
        assert!(kv.contains("name"));
        assert_eq!(kv.len(), 2);
        assert_eq!(kv.remove("gold"), Some(Value::Int(30)));
        assert!(kv.get("gold").is_none());
        assert!(kv.remove("gold").is_none());
        assert_eq!(kv.len(), 1);
    }

    #[test]
    fn revision_bumps_only_on_actual_changes() {
        let mut kv = KvStore::new();
        assert_eq!(kv.revision(), 0);
        kv.set("gold", Value::Int(30)).unwrap();
        assert_eq!(kv.revision(), 1);
        kv.set("gold", Value::Int(30)).unwrap(); // 同值覆写：幂等不计
        assert_eq!(kv.revision(), 1);
        kv.set("gold", Value::Int(31)).unwrap();
        assert_eq!(kv.revision(), 2);
        assert!(kv.remove("gold").is_some());
        assert_eq!(kv.revision(), 3);
        assert!(kv.remove("gold").is_none()); // 幂等：不计变更
        assert_eq!(kv.revision(), 3);
    }

    #[test]
    fn non_finite_values_are_rejected_with_key_context() {
        let mut kv = KvStore::new();
        let err = kv.set("ratio", Value::Float(f64::NAN)).unwrap_err();
        assert!(err.to_string().contains("ratio"));
        assert!(matches!(
            kv.set("ratio", Value::Float(f64::INFINITY)),
            Err(KvError::NonFiniteF64(_))
        ));
        assert!(matches!(
            kv.set("ratio", Value::Float(f64::NEG_INFINITY)),
            Err(KvError::NonFiniteF64(_))
        ));
        assert!(kv.get("ratio").is_none()); // 拒绝路径不落表
    }

    #[test]
    fn negative_zero_is_normalized_on_write() {
        let mut kv = KvStore::new();
        kv.set("bias", Value::Float(-0.0)).unwrap();
        match kv.get("bias") {
            Some(Value::Float(f)) => {
                assert_eq!(*f, 0.0);
                assert!(f.is_sign_positive());
            }
            other => panic!("expected Float(0.0), got {other:?}"),
        }
    }

    #[test]
    fn iteration_is_in_sorted_key_order() {
        let mut kv = KvStore::new();
        kv.set("zeta", Value::Int(1)).unwrap();
        kv.set("alpha", Value::Int(2)).unwrap();
        kv.set("mid", Value::Int(3)).unwrap();
        let keys: Vec<_> = kv.iter().map(|(k, _)| k).collect();
        assert_eq!(keys, ["alpha", "mid", "zeta"]);
    }

    #[test]
    fn serde_round_trip_preserves_entries_and_revision() {
        let mut kv = KvStore::new();
        kv.set("gold", Value::Int(30)).unwrap();
        kv.set("ratio", Value::float(0.5).unwrap()).unwrap();
        let bytes = serde_json::to_vec(&kv).unwrap();
        let restored: KvStore = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(kv, restored);
        assert_eq!(restored.revision(), 2); // 全字段序列化含 revision
        let keys: Vec<_> = restored.iter().map(|(k, _)| k).collect();
        assert_eq!(keys, ["gold", "ratio"]); // 反序列化表仍为规范序
    }
}
