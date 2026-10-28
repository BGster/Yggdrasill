//! 规范快照序列化（D-继承-5）：`EntityStore` → 排序键 canonical JSON bytes。
//!
//! - `serde_json` 默认 `Map` = `BTreeMap`（**禁止启用 `preserve_order` feature**）；
//! - 属性键序 = `BTreeMap` 排序、实体序 = ID 升序——插入次序不进字节；
//! - `-0.0 → 0.0`：写入路径已在 `entity::set_attribute` 规范化（存储值即规范形），
//!   编码侧 `Value` 手写 `Serialize` 兜底（纵深防御，覆盖反序列化旁路）；
//! - 编码前校验：非有限值（NaN/±∞）拒绝为错误（非有限值是错误，不是静默改写）；
//! - 快照在**每个事件边界**取一次（S4 转换驱动消费）；SHA-256 归 ygg-sandbox
//!   （D-继承-5）——本 crate 只产 canonical bytes，不做哈希。

use std::collections::BTreeMap;

use serde::Serialize;

use crate::entity::{Entity, EntityId, EntityStore, StateError, Value};

/// M0 快照格式版本（本格式的读时校验版本；与 S5 存档 `SAVE_FORMAT_VERSION`
/// 语义同型——各自格式的读时版本校验，互不共用、各自演进）。
pub const SNAPSHOT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error(transparent)]
    State(#[from] StateError),
    #[error("canonical JSON encoding failed: {0}")]
    Json(#[from] serde_json::Error),
}

/// 事件边界快照载体：格式版本 + 存储版本 + next_id + 全量实体表（序 = 规范序）。
#[derive(Serialize)]
struct CanonicalSnapshot<'a> {
    schema_version: u32,
    revision: u64,
    next_entity_id: EntityId,
    entities: &'a BTreeMap<EntityId, Entity>,
}

/// 产出 canonical JSON bytes（UTF-8）。
/// 同一状态无论以何种插入次序构造，输出逐字节一致（确定性哈希的前置面）。
pub fn canonical_snapshot(store: &EntityStore) -> Result<Vec<u8>, SnapshotError> {
    validate_finite(store)?;
    let snap = CanonicalSnapshot {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        revision: store.revision(),
        next_entity_id: store.next_entity_id(),
        entities: store.entities(),
    };
    Ok(serde_json::to_vec(&snap)?)
}

/// 非有限值拒绝（纵深防御）：公开写入路径已把关，此处覆盖反序列化旁路。
fn validate_finite(store: &EntityStore) -> Result<(), StateError> {
    for entity in store.iter() {
        for (name, value) in entity.attributes() {
            if let Value::Float(f) = value {
                if !f.is_finite() {
                    return Err(StateError::NonFiniteF64 {
                        entity_id: entity.id,
                        attribute: name.to_owned(),
                    });
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 以显式 ID 序构造同内容 store（`insert` 路径），插入次序由参数控制。
    fn build(order: [u64; 2]) -> EntityStore {
        let mut store = EntityStore::new();
        for id in order {
            let mut e = Entity::new(id);
            if id == 1 {
                e.set_attribute("name", Value::Str("feni".into())).unwrap();
                e.set_attribute("hp", Value::Int(30)).unwrap();
                e.set_attribute("ratio", Value::float(0.5).unwrap())
                    .unwrap();
                assert!(e.add_tag("protagonist"));
            } else {
                e.set_attribute("hp", Value::Int(12)).unwrap();
            }
            store.insert(e).unwrap();
        }
        store
    }

    #[test]
    fn insertion_order_does_not_affect_canonical_bytes() {
        let a = build([1, 2]);
        let b = build([2, 1]);
        assert_eq!(a.revision(), b.revision());
        assert_eq!(
            canonical_snapshot(&a).unwrap(),
            canonical_snapshot(&b).unwrap()
        );
    }

    #[test]
    fn canonical_output_is_sorted_and_versioned() {
        let store = build([1, 2]);
        let s = String::from_utf8(canonical_snapshot(&store).unwrap()).unwrap();
        // 属性键升序出现（BTreeMap 排序键即规范序）
        let hp = s.find("\"hp\"").unwrap();
        let name = s.find("\"name\"").unwrap();
        let ratio = s.find("\"ratio\"").unwrap();
        assert!(hp < name && name < ratio);
        // 实体 ID 升序出现
        assert!(s.find("\"1\":").unwrap() < s.find("\"2\":").unwrap());
        // 版本字段入字节
        assert!(s.contains(&format!("\"schema_version\":{SNAPSHOT_SCHEMA_VERSION}")));
        assert!(s.contains("\"revision\":2"));
    }

    #[test]
    fn negative_zero_encodes_as_zero() {
        let mut store = EntityStore::new();
        let id = store.spawn().unwrap();
        // 写入路径已把存储值规范为 +0.0；编码侧 Serialize 二次兜底。
        store.set_attribute(id, "bias", Value::Float(-0.0)).unwrap();
        let s = String::from_utf8(canonical_snapshot(&store).unwrap()).unwrap();
        assert!(!s.contains("-0.0"));
        assert!(s.contains("0.0"));
    }

    #[test]
    fn nan_is_rejected_as_error_not_silently_rewritten() {
        assert!(Value::float(f64::NAN).is_none());
        let mut store = EntityStore::new();
        let id = store.spawn().unwrap();
        let err = store
            .set_attribute(id, "x", Value::Float(f64::NAN))
            .unwrap_err();
        assert!(matches!(err, StateError::NonFiniteF64 { .. }));
    }
}
