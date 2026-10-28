//! 实体存储：实体 / 属性 / 标签（Rust 原生、可序列化、带版本号，FR1）。
//!
//! 确定性纪律（D-继承-5 / research §3）：
//! - 实体表 `BTreeMap<EntityId, Entity>`：ID 升序 = 天然规范遍历序（威胁 #3/#4；
//!   禁用随机化哈希的 std 默认无序容器，确定性路径一律有序容器）；
//! - 属性表 `BTreeMap<String, Value>`、标签 `BTreeSet<String>`：序列化即排序键规范序；
//! - 浮点写入即把关：非有限值（NaN/±∞）拒绝为错误、`-0.0` 入表前规范化为
//!   `0.0`（威胁 #7；编码侧 `Serialize` 二次兜底，纵深防御）；
//! - ID 从 1 起严格单调分配（`new()` + `spawn`，无零值缺省起点；
//!   宿主注入序分歧不产生遍历序分歧，威胁 #4）。

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// 实体 ID：内核/宿主分配的持久标识（`u64`，规范序 = 数值升序）。
pub type EntityId = u64;

/// 属性标量值。M0 收敛为数字 / 布尔 / 字符串（research §3 威胁 #9：
/// 可哈希状态禁表 / userdata 键；复合结构属资源面，走 `ygg.data`，S11）。
///
/// 序列化形态 = **未标记标量**（`Serialize` / `Deserialize` 均手写、两向对称）：
/// 派生 `Deserialize` 是外部标记形态（如 `{"Int":7}`），与本类型未标记标量
/// 编码（`7`）不对称，往返即断——故两个方向都手写（S2 amendment 2，2026-10-27）。
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
}

impl Value {
    /// 便利构造 `Float`：NaN 返回 `None`（写入路径的守门构造器）。
    pub fn float(x: f64) -> Option<Self> {
        if x.is_nan() {
            None
        } else {
            Some(Value::Float(x))
        }
    }
}

// `Serialize` 手写：`-0.0 → 0.0` 在编码侧兜底（写入路径 `set_attribute` 已先行
// 规范化，此处覆盖反序列化旁路构造出的非规范值——纵深防御，非唯一防线）。
// 非有限值不做规范化——`canonical_snapshot` 先行校验拒绝（非有限值是错误，
// 不是静默改写）。
impl Serialize for Value {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::Int(v) => v.serialize(serializer),
            Value::Float(v) => normalize_f64(*v).serialize(serializer),
            Value::Bool(v) => v.serialize(serializer),
            Value::Str(v) => v.serialize(serializer),
        }
    }
}

// `Deserialize` 手写（S2 amendment 2）：与手写 `Serialize` 对称的未标记标量
// 形态。`deserialize_any` + 标量 visitor：
// - 整数 token 正数走 `visit_u64`、负数走 `visit_i64`，均归一为 `Int`；
//   超 `i64` 范围的 `u64` 降级为 `Float`（f64 对该区间可表示但可能失精度——
//   M0 标量场景的文档化取舍）；
// - 浮点走 `visit_f64`：非有限值在反序列化旁路即拒绝（`Error::custom`——
//   反序列化发生在裸 `Value` 上、无实体上下文，带定位的 `NonFiniteF64`
//   错误在此不可得；`snapshot::validate_finite` 仍是权威层，本处为纵深防御）；
// - 布尔 / 字符串直通。
impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ScalarVisitor;
        impl<'de> serde::de::Visitor<'de> for ScalarVisitor {
            type Value = Value;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an integer, float, boolean or string scalar")
            }

            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Value, E> {
                Ok(Value::Int(v))
            }

            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Value, E> {
                Ok(match i64::try_from(v) {
                    Ok(i) => Value::Int(i),
                    Err(_) => Value::Float(v as f64),
                })
            }

            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Value, E> {
                if v.is_finite() {
                    Ok(Value::Float(v))
                } else {
                    Err(serde::de::Error::custom("non-finite float in stored value"))
                }
            }

            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Value, E> {
                Ok(Value::Bool(v))
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Value, E> {
                Ok(Value::Str(v.to_owned()))
            }
        }
        deserializer.deserialize_any(ScalarVisitor)
    }
}

/// `-0.0 → 0.0` 浮点规范化唯一入口（native / wasm 位模式统一）。
pub fn normalize_f64(x: f64) -> f64 {
    if x == 0.0 {
        0.0
    } else {
        x
    }
}

/// 实体存储层错误（thiserror，白名单内）。
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("attribute `{attribute}` of entity {entity_id} is non-finite")]
    NonFiniteF64 {
        entity_id: EntityId,
        attribute: String,
    },
    #[error("entity {0} already exists")]
    DuplicateEntity(EntityId),
    #[error("entity {0} not found")]
    NoSuchEntity(EntityId),
    #[error("entity id space exhausted")]
    IdSpaceExhausted,
}

/// 单个实体：属性表 + 标签集（均为有序容器，序列化即规范序）。
///
/// 可变方法（`set_attribute` / `remove_attribute` / `add_tag` / `remove_tag`）
/// 仅供**构造期**（插入 store 前）使用；已入 store 的实体字段不可直接可变借用，
/// 存量变更只能走 `EntityStore` 的把关方法——revision 推进、非有限值门与
/// `-0.0` 规范化因此在 store 层单点收口。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub id: EntityId,
    attributes: BTreeMap<String, Value>,
    tags: BTreeSet<String>,
}

impl Entity {
    pub fn new(id: EntityId) -> Self {
        Self {
            id,
            attributes: BTreeMap::new(),
            tags: BTreeSet::new(),
        }
    }

    /// 写属性（非有限值（NaN/±∞）拒绝为带定位的错误；`-0.0` 入表前规范化
    /// 为 `0.0`——存储值即规范形，读取与快照不依赖编码侧兜底）。
    pub fn set_attribute(
        &mut self,
        name: impl Into<String>,
        value: Value,
    ) -> Result<(), StateError> {
        let name = name.into();
        if let Value::Float(f) = &value {
            if !f.is_finite() {
                return Err(StateError::NonFiniteF64 {
                    entity_id: self.id,
                    attribute: name,
                });
            }
        }
        let value = match value {
            Value::Float(f) => Value::Float(normalize_f64(f)),
            other => other,
        };
        self.attributes.insert(name, value);
        Ok(())
    }

    pub fn attribute(&self, name: &str) -> Option<&Value> {
        self.attributes.get(name)
    }

    pub fn remove_attribute(&mut self, name: &str) -> Option<Value> {
        self.attributes.remove(name)
    }

    /// 属性只读遍历（`BTreeMap` 键升序 = 规范序）。
    pub fn attributes(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.attributes.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// 加标签；返回是否实际改变成员关系（已存在则 `false`）。
    pub fn add_tag(&mut self, tag: impl Into<String>) -> bool {
        self.tags.insert(tag.into())
    }

    /// 去标签；返回是否实际改变成员关系（原本不存在则 `false`）。
    pub fn remove_tag(&mut self, tag: &str) -> bool {
        self.tags.remove(tag)
    }

    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.contains(tag)
    }

    /// 标签只读遍历（`BTreeSet` 升序 = 规范序）。
    pub fn tags(&self) -> impl Iterator<Item = &str> {
        self.tags.iter().map(String::as_str)
    }
}

/// 实体存储：`BTreeMap<EntityId, Entity>`，ID 升序为规范遍历序（FR1 / D-继承-5）。
///
/// `revision` 在每次**实际状态变更**后 +1（FR1「带版本号」；事件边界快照消费，S4）：
/// `spawn` / `insert` / `remove` / `set_attribute` 写入即计；标签增删与属性删除仅在
/// 成员关系 / 键实际改变时计。所有存量实体变更只能经本层把关方法进行
/// （无直接可变实体借用出口），变更语义因此单点可审计。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntityStore {
    entities: BTreeMap<EntityId, Entity>,
    next_id: EntityId,
    revision: u64,
}

impl Default for EntityStore {
    /// 显式委托 `new()`：`next_id` 从 1 起（派生缺省会给出 0 值起点，
    /// 违背「ID 从 1 起严格单调分配」纪律，故手写而非派生）。
    fn default() -> Self {
        Self::new()
    }
}

impl EntityStore {
    pub fn new() -> Self {
        Self {
            entities: BTreeMap::new(),
            next_id: 1,
            revision: 0,
        }
    }

    /// 分配新 ID（从 1 起严格单调递增——确定性，research §3 威胁 #4）并插入空实体。
    pub fn spawn(&mut self) -> Result<EntityId, StateError> {
        let id = self.next_id;
        self.next_id = id.checked_add(1).ok_or(StateError::IdSpaceExhausted)?;
        self.entities.insert(id, Entity::new(id));
        self.revision += 1;
        Ok(id)
    }

    /// 以指定 ID 插入（宿主注入 / 读档恢复路径）；`next_id` 推进保持 `spawn` 单调。
    pub fn insert(&mut self, entity: Entity) -> Result<(), StateError> {
        let id = entity.id;
        if self.entities.contains_key(&id) {
            return Err(StateError::DuplicateEntity(id));
        }
        self.next_id = self
            .next_id
            .max(id.checked_add(1).ok_or(StateError::IdSpaceExhausted)?);
        self.entities.insert(id, entity);
        self.revision += 1;
        Ok(())
    }

    /// 删除实体；返回被删实体（不存在时 `None` 且不计变更）。
    pub fn remove(&mut self, id: EntityId) -> Option<Entity> {
        let removed = self.entities.remove(&id);
        if removed.is_some() {
            self.revision += 1;
        }
        removed
    }

    /// 把关写属性：非有限值拒绝（`-0.0` 入表前规范化）；实际改变属性值才计变更
    /// （同值覆写为幂等写，不计）。
    pub fn set_attribute(
        &mut self,
        id: EntityId,
        name: impl Into<String>,
        value: Value,
    ) -> Result<(), StateError> {
        let value = match value {
            Value::Float(f) => Value::Float(normalize_f64(f)),
            other => other,
        };
        let name = name.into();
        let entity = self
            .entities
            .get_mut(&id)
            .ok_or(StateError::NoSuchEntity(id))?;
        if entity.attribute(&name) == Some(&value) {
            return Ok(()); // 幂等写：状态未变，不计变更
        }
        entity.set_attribute(name, value)?;
        self.revision += 1;
        Ok(())
    }

    /// 把关删属性；实际删除才计变更。
    pub fn remove_attribute(
        &mut self,
        id: EntityId,
        name: &str,
    ) -> Result<Option<Value>, StateError> {
        let entity = self
            .entities
            .get_mut(&id)
            .ok_or(StateError::NoSuchEntity(id))?;
        let removed = entity.remove_attribute(name);
        if removed.is_some() {
            self.revision += 1;
        }
        Ok(removed)
    }

    /// 把关加标签；实际改变成员关系才计变更。
    pub fn add_tag(&mut self, id: EntityId, tag: impl Into<String>) -> Result<bool, StateError> {
        let entity = self
            .entities
            .get_mut(&id)
            .ok_or(StateError::NoSuchEntity(id))?;
        let inserted = entity.add_tag(tag);
        if inserted {
            self.revision += 1;
        }
        Ok(inserted)
    }

    /// 把关去标签；实际改变成员关系才计变更。
    pub fn remove_tag(&mut self, id: EntityId, tag: &str) -> Result<bool, StateError> {
        let entity = self
            .entities
            .get_mut(&id)
            .ok_or(StateError::NoSuchEntity(id))?;
        let removed = entity.remove_tag(tag);
        if removed {
            self.revision += 1;
        }
        Ok(removed)
    }

    pub fn get(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(&id)
    }

    pub fn contains(&self, id: EntityId) -> bool {
        self.entities.contains_key(&id)
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// 按 ID 升序遍历（规范序，research §3 威胁 #4 的宿主侧对应面）。
    pub fn iter(&self) -> impl Iterator<Item = &Entity> {
        self.entities.values()
    }

    /// 内部实体表只读引用（`canonical_snapshot` 序列化面）。
    pub fn entities(&self) -> &BTreeMap<EntityId, Entity> {
        &self.entities
    }

    /// 下一个将分配的实体 ID（快照恢复 `spawn` 单调性的必要字段）。
    pub fn next_entity_id(&self) -> EntityId {
        self.next_id
    }

    /// 存储版本号：每次实际状态变更 +1（FR1「带版本号」；语义见结构体文档）。
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_allocates_monotonic_ids_from_one() {
        let mut store = EntityStore::new();
        assert_eq!(store.spawn().unwrap(), 1);
        assert_eq!(store.spawn().unwrap(), 2);
        assert_eq!(store.spawn().unwrap(), 3);
        assert_eq!(store.next_entity_id(), 4);
    }

    #[test]
    fn manual_default_starts_ids_at_one() {
        // 手写 Default 委托 new()：派生缺省会给出 next_id = 0（CONCERN 2 回归锚）。
        let mut store = EntityStore::default();
        assert_eq!(store.next_entity_id(), 1);
        assert_eq!(store.spawn().unwrap(), 1);
    }

    #[test]
    fn insert_advances_next_id_and_rejects_duplicates() {
        let mut store = EntityStore::new();
        let mut e = Entity::new(10);
        assert!(e.add_tag("hero"));
        store.insert(e).unwrap();
        assert_eq!(store.next_entity_id(), 11);
        assert!(store.spawn().unwrap() >= 11);
        assert!(matches!(
            store.insert(Entity::new(10)),
            Err(StateError::DuplicateEntity(10))
        ));
    }

    #[test]
    fn set_attribute_rejects_non_finite_with_context() {
        let mut store = EntityStore::new();
        let id = store.spawn().unwrap();
        let err = store
            .set_attribute(id, "hp", Value::Float(f64::NAN))
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("hp") && msg.contains("is non-finite"));
    }

    #[test]
    fn set_attribute_rejects_infinity() {
        let mut store = EntityStore::new();
        let id = store.spawn().unwrap();
        assert!(matches!(
            store.set_attribute(id, "atk", Value::Float(f64::INFINITY)),
            Err(StateError::NonFiniteF64 { .. })
        ));
        assert!(matches!(
            store.set_attribute(id, "def", Value::Float(f64::NEG_INFINITY)),
            Err(StateError::NonFiniteF64 { .. })
        ));
    }

    #[test]
    fn negative_zero_is_normalized_on_write() {
        let mut store = EntityStore::new();
        let id = store.spawn().unwrap();
        store.set_attribute(id, "bias", Value::Float(-0.0)).unwrap();
        // 断言对象 = 存储值本身（写入路径单点门），不依赖编码侧兜底。
        match store.get(id).unwrap().attribute("bias") {
            Some(Value::Float(f)) => {
                assert_eq!(*f, 0.0);
                assert!(f.is_sign_positive());
            }
            other => panic!("expected Float(0.0), got {other:?}"),
        }
    }

    #[test]
    fn value_float_constructor_gates_nan() {
        assert!(Value::float(f64::NAN).is_none());
        assert!(matches!(Value::float(1.5), Some(Value::Float(_))));
    }

    #[test]
    fn revision_bumps_exactly_on_actual_state_changes() {
        let mut store = EntityStore::new();
        assert_eq!(store.revision(), 0);
        let id = store.spawn().unwrap();
        assert_eq!(store.revision(), 1);
        store.set_attribute(id, "hp", Value::Int(1)).unwrap();
        assert_eq!(store.revision(), 2);
        store.set_attribute(id, "hp", Value::Int(1)).unwrap(); // 同值覆写：幂等不计
        assert_eq!(store.revision(), 2);
        assert!(store.add_tag(id, "hero").unwrap());
        assert_eq!(store.revision(), 3);
        assert!(!store.add_tag(id, "hero").unwrap()); // 幂等：不计变更
        assert_eq!(store.revision(), 3);
        assert!(store.remove_tag(id, "hero").unwrap());
        assert_eq!(store.revision(), 4);
        assert!(!store.remove_tag(id, "hero").unwrap()); // 幂等：不计变更
        assert_eq!(store.revision(), 4);
        let mut e = Entity::new(9);
        assert!(e.add_tag("npc"));
        store.insert(e).unwrap();
        assert_eq!(store.revision(), 5);
        store.set_attribute(9, "hp", Value::Int(2)).unwrap();
        assert_eq!(store.revision(), 6);
        assert!(store.remove_attribute(9, "hp").unwrap().is_some());
        assert_eq!(store.revision(), 7);
        assert!(store.remove_attribute(9, "hp").unwrap().is_none()); // 幂等
        assert_eq!(store.revision(), 7);
        assert!(store.remove(9).is_some());
        assert_eq!(store.revision(), 8);
        assert!(store.remove(9).is_none()); // 幂等
        assert_eq!(store.revision(), 8);
    }

    #[test]
    fn store_mutators_reject_missing_entity() {
        let mut store = EntityStore::new();
        assert!(matches!(
            store.set_attribute(7, "hp", Value::Int(1)),
            Err(StateError::NoSuchEntity(7))
        ));
        assert!(matches!(
            store.remove_attribute(7, "hp"),
            Err(StateError::NoSuchEntity(7))
        ));
        assert!(matches!(
            store.add_tag(7, "x"),
            Err(StateError::NoSuchEntity(7))
        ));
        assert!(matches!(
            store.remove_tag(7, "x"),
            Err(StateError::NoSuchEntity(7))
        ));
    }

    #[test]
    fn tags_and_attributes_iterate_in_sorted_order() {
        let mut e = Entity::new(1);
        assert!(e.add_tag("zeta"));
        assert!(e.add_tag("alpha"));
        e.set_attribute("z", Value::Int(1)).unwrap();
        e.set_attribute("a", Value::Int(2)).unwrap();
        let names: Vec<_> = e.attributes().map(|(k, _)| k).collect();
        assert_eq!(names, ["a", "z"]);
        let tags: Vec<_> = e.tags().collect();
        assert_eq!(tags, ["alpha", "zeta"]);
        assert!(!e.remove_tag("nope"));
    }

    #[test]
    fn value_round_trip_untagged_scalar_form() {
        // S2 amendment 2 回归锚：未标记标量形态两向往返（原派生 Deserialize
        // 为外部标记形态，与本类型未标记编码不对称，往返即断）。
        for v in [
            Value::Int(-3),
            Value::Int(42),
            Value::float(1.5).unwrap(),
            Value::Bool(true),
            Value::Str("feni".into()),
        ] {
            let bytes = serde_json::to_vec(&v).unwrap();
            let back: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(v, back);
        }
        // 整数 token：正数走 visit_u64、负数走 visit_i64，均归一为 Int
        let pos: Value = serde_json::from_str("7").unwrap();
        assert_eq!(pos, Value::Int(7));
        let neg: Value = serde_json::from_str("-7").unwrap();
        assert_eq!(neg, Value::Int(-7));
        // 超 i64 范围的 u64 降级为 Float（文档化取舍：M0 标量场景可接受）
        let huge: Value = serde_json::from_str("18446744073709551615").unwrap();
        assert!(matches!(huge, Value::Float(_)));
    }

    #[test]
    fn serde_round_trip_preserves_sorted_content() {
        let mut store = EntityStore::new();
        let id = store.spawn().unwrap();
        store.set_attribute(id, "hp", Value::Int(7)).unwrap();
        let bytes = serde_json::to_vec(&store).unwrap();
        let restored: EntityStore = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(store, restored);
        assert_eq!(restored.next_entity_id(), 2);
        // spawn(+1) + set_attribute(+1) = revision 2（全字段序列化含 revision）
        assert_eq!(restored.revision(), 2);
    }
}
