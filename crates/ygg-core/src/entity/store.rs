use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::index::IndexKey;
use super::value::{normalize_f64, EntityId, Value};
use super::EntityStore;

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

impl EntityStore {
    pub fn new() -> Self {
        Self {
            entities: BTreeMap::new(),
            next_id: 1,
            revision: 0,
            indices: BTreeMap::new(),
        }
    }

    /// 分配新 ID（从 1 起严格单调递增——确定性，research §3 威胁 #4）并插入空实体。
    ///
    /// 索引零工作：新实体没有属性，而索引只登记**在场属性的值**（v2 §2.2 的五个写点之一）。
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
        self.index_entity(&entity, true);
        self.entities.insert(id, entity);
        self.revision += 1;
        Ok(())
    }

    /// 下一个将要分配的 id（只读镜像；**不是**「当前存活数」）。
    ///
    /// 批式面（`ygg.entity.spawn_batch`）要用它把水位一次回给 Tier 2，免得逐条读。
    /// 与 `EntityStore::iter` 的区别是语义：这里吃的是**分配水位**，与存活无关。
    pub fn next_id(&self) -> EntityId {
        self.next_id
    }

    /// 删除实体；返回被删实体（不存在时 `None` 且不计变更）。
    pub fn remove(&mut self, id: EntityId) -> Option<Entity> {
        let removed = self.entities.remove(&id);
        if let Some(entity) = &removed {
            self.index_entity(entity, false);
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
            return Ok(()); // 幂等写：状态未变，不计变更（索引零工作）
        }
        // 非有限值在**任何副作用之前**拒（`Entity::set_attribute` 的同一道门，提前到
        // 索引补丁之前：先改索引再报错会留下与实体表不一致的索引）。
        if let Value::Float(f) = &value {
            if !f.is_finite() {
                return Err(StateError::NonFiniteF64 {
                    entity_id: id,
                    attribute: name,
                });
            }
        }
        if let Some(index) = self.indices.get_mut(&name) {
            if let Some(old) = entity.attribute(&name) {
                index.remove(&IndexKey::of(old), id);
            }
            index.insert(IndexKey::of(&value), id);
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
        if let Some(value) = &removed {
            if let Some(index) = self.indices.get_mut(name) {
                index.remove(&IndexKey::of(value), id);
            }
            self.revision += 1;
        }
        Ok(removed)
    }

    /// 把关加标签；实际改变成员关系才计变更。
    ///
    /// 标签不碰索引：属性索引与 tag 无关（v2 §2.2 末句）。
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
