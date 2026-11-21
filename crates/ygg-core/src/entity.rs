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

use std::cmp::Ordering;
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
#[derive(Debug, Serialize, Deserialize)]
pub struct EntityStore {
    entities: BTreeMap<EntityId, Entity>,
    next_id: EntityId,
    revision: u64,
    /// 属性索引（S24 v2 §2）：按需惰性首建、写点增量维护。
    ///
    /// **不是状态**：`#[serde(skip)]` + 手写 `PartialEq` / `Clone` 同时挡三个面——存档
    /// 字节逐字节不变（`SAVE_FORMAT_VERSION` 不动）、规范快照走 `entities()` 视图本就不经
    /// 过它、`save.rs` 的 `assert_eq!(save, restored)` 不会把「有索引 vs 无索引」判不等。
    #[serde(skip)]
    indices: BTreeMap<String, AttrIndex>,
}

impl Clone for EntityStore {
    /// 克隆**丢弃索引**（v2 §2.2）：索引是派生结构，重建比搬运便宜，而「带索引的克隆」
    /// 会把一份缓存变成可见语义。
    fn clone(&self) -> Self {
        Self {
            entities: self.entities.clone(),
            next_id: self.next_id,
            revision: self.revision,
            indices: BTreeMap::new(),
        }
    }
}

impl PartialEq for EntityStore {
    /// 比较**忽略索引**（派生比较会把「有索引 vs 无索引」判不等，见字段注释）。
    fn eq(&self, other: &Self) -> bool {
        self.entities == other.entities
            && self.next_id == other.next_id
            && self.revision == other.revision
    }
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

    /// 查询（S24）：`selector` 决定存活集与输出序。
    ///
    /// 序：无 `order` ⇒ **id 升序**（`BTreeMap` 序遍历）；有 `order` ⇒ `(值, id)` 升序
    /// （`desc` 只反转值序，**id 兜底不反转**；缺属性排最后）。
    ///
    /// **`&mut self` 的理由**：可走索引的谓词会惰性首建该属性的索引（v2 §2.2）；v1 §3.1
    /// 写的 `&self` 在带索引的形态下不可实现（无内部可变性，而把索引放宿主侧已被 v2
    /// 否证）。候选集出来以后不再变——索引路径与全扫路径都逐条跑
    /// [`Selector::matches`]，索引只是剪枝。
    pub fn select(&mut self, selector: &Selector) -> Vec<&Entity> {
        let candidates = self.index_candidates(selector);
        let mut rows: Vec<&Entity> = match &candidates {
            Some(ids) => ids
                .iter()
                .filter_map(|id| self.entities.get(id))
                .filter(|entity| selector.matches(entity))
                .collect(),
            None => self
                .entities
                .values()
                .filter(|entity| selector.matches(entity))
                .collect(),
        };
        self.apply_order(&mut rows, selector);
        rows
    }

    /// `order` 排序（键预取一次，比较只读键：k 行 = k 次属性查 + 一次排序）。
    fn apply_order<'a>(&self, rows: &mut Vec<&'a Entity>, selector: &Selector) {
        let Some(order) = &selector.order else {
            return;
        };
        let mut keyed: Vec<(Option<QueryValue<'a>>, &'a Entity)> = rows
            .iter()
            .map(|entity| (entity.query_value(&order.attr), *entity))
            .collect();
        keyed.sort_unstable_by(|(left, a), (right, b)| {
            compare_order_keys(left, right, order.descending).then(a.id.cmp(&b.id))
        });
        *rows = keyed.into_iter().map(|(_, entity)| entity).collect();
    }
}

// ============================================================================
// 通用实体查询引擎（S24；契约 `docs/design/packages/entity.md` §9.8）
// ============================================================================
//
// 语义住在 core（无 VM 可单测）：选择器、比较矩阵与序遍历都在这里，宿主侧只做文法
// 解码（`host_api::entity_query_selector`）与两阶段材质化。这样「跨族比较是假不是
// 错」「缺属性一律假」「`is` 管存在性」这几条裁断不会随宿主重构漂。

/// 比较运算（PostgREST 族裁剪版的**执行形态**）。
///
/// `not` 不是独立 op，它是 [`Predicate::negated`] 上的前缀修饰；`in` 在谓词层展开
/// （任一元素 `eq` 成立即真）。所以枚举里**没有算术臂**——浮点不可结合性无从发生
/// （确定性论证第一条，v1 §4）。
#[derive(Clone, Debug, PartialEq)]
pub enum CompareOp {
    Eq(Value),
    Neq(Value),
    Gt(Value),
    Gte(Value),
    Lt(Value),
    Lte(Value),
    /// 标量数组，任一 `eq` 成立即真；**空数组恒假**。
    In(Vec<Value>),
    /// `*` 唯一通配符、四形态（[`like_shape`]）；越界形态在解码期已拒。
    Like(String),
    /// `like` + **ASCII** 小写折叠（Unicode 折叠表是版本相关依赖，不做）。
    ILike(String),
    /// **存在性测试**：唯一不受「缺属性 ⇒ 假」管辖的 op。
    Is(IsTest),
}

/// `is` 的四种测试（`null` = 不在场、`notnull` = 在场、`true`/`false` = 在场且布尔相等）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IsTest {
    Null,
    NotNull,
    True,
    False,
}

impl IsTest {
    /// 解码 `is` 的操作数文本（`null` / `notnull` / `true` / `false`）。
    pub fn of(text: &str) -> Option<Self> {
        match text {
            "null" => Some(Self::Null),
            "notnull" => Some(Self::NotNull),
            "true" => Some(Self::True),
            "false" => Some(Self::False),
            _ => None,
        }
    }

    fn holds(self, found: Option<&Value>) -> bool {
        match self {
            Self::Null => found.is_none(),
            Self::NotNull => found.is_some(),
            Self::True => found == Some(&Value::Bool(true)),
            Self::False => found == Some(&Value::Bool(false)),
        }
    }
}

impl CompareOp {
    /// 存储值在场时的求值（缺属性由 [`Predicate::matches`] 先判）。
    ///
    /// 「跨族 ⇒ 假」对**所有** op 成立（含 `neq`）：跨族比较不是错误（扫描中报错会
    /// 产出半截结果），故 `neq` 不是全关系——契约明写。
    pub fn matches(&self, stored: &Value) -> bool {
        match self {
            Self::Eq(v) => value_compare(stored, v) == Some(Ordering::Equal),
            Self::Neq(v) => value_compare(stored, v).is_some_and(|ord| ord != Ordering::Equal),
            Self::Gt(v) => value_compare(stored, v) == Some(Ordering::Greater),
            Self::Gte(v) => matches!(
                value_compare(stored, v),
                Some(Ordering::Greater | Ordering::Equal)
            ),
            Self::Lt(v) => value_compare(stored, v) == Some(Ordering::Less),
            Self::Lte(v) => matches!(
                value_compare(stored, v),
                Some(Ordering::Less | Ordering::Equal)
            ),
            Self::In(list) => list
                .iter()
                .any(|v| value_compare(stored, v) == Some(Ordering::Equal)),
            Self::Like(pattern) => match stored {
                Value::Str(text) => like_matches(text, pattern),
                _ => false,
            },
            Self::ILike(pattern) => match stored {
                Value::Str(text) => ilike_matches(text, pattern),
                _ => false,
            },
            Self::Is(test) => test.holds(Some(stored)),
        }
    }
}

/// `like` 的四种合法形态（v2 §4「越窄越好」：四形态各对应一条线性扫描原语，
/// 零正则、零回溯、零 ReDoS 面）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LikeShape {
    /// 无 `*`（≡ `eq`）。
    Exact,
    /// `P*`
    Prefix,
    /// `*S`
    Suffix,
    /// `*C*`
    Contains,
}

/// 判 `like` 形态；`None` = 越界形态（解码期按 `YGG-ENTITY-FRAME` 拒）。
///
/// 合法：无 `*` / `P*` / `*S` / `*C*`，且**字面段非空**——裸 `*` 与 `**` 不属于四形态
/// 中任何一种（字面段为空 = 「全匹配」，与笔误不可区分）。`*` 不可转义：要匹配字面
/// `*` 用 `eq`。
pub fn like_shape(pattern: &str) -> Option<LikeShape> {
    let stars = pattern.bytes().filter(|byte| *byte == b'*').count();
    match stars {
        0 => Some(LikeShape::Exact),
        1 if pattern.starts_with('*') => non_empty(&pattern[1..]).map(|()| LikeShape::Suffix),
        1 if pattern.ends_with('*') => {
            non_empty(&pattern[..pattern.len() - 1]).map(|()| LikeShape::Prefix)
        }
        2 if pattern.starts_with('*') && pattern.ends_with('*') => {
            non_empty(&pattern[1..pattern.len() - 1]).map(|()| LikeShape::Contains)
        }
        _ => None,
    }
}

fn non_empty(part: &str) -> Option<()> {
    if part.is_empty() {
        None
    } else {
        Some(())
    }
}

/// `like` 匹配（越界形态保守为假——解码期已拒，此处不 panic）。
pub fn like_matches(haystack: &str, pattern: &str) -> bool {
    match like_shape(pattern) {
        Some(LikeShape::Exact) => haystack == pattern,
        Some(LikeShape::Prefix) => haystack.starts_with(&pattern[..pattern.len() - 1]),
        Some(LikeShape::Suffix) => haystack.ends_with(&pattern[1..]),
        Some(LikeShape::Contains) => haystack.contains(&pattern[1..pattern.len() - 1]),
        None => false,
    }
}

/// `ilike`：两侧都做 ASCII 折叠后走同一套形态匹配（`*` 是 ASCII，形态不变）。
pub fn ilike_matches(haystack: &str, pattern: &str) -> bool {
    like_matches(
        &haystack.to_ascii_lowercase(),
        &pattern.to_ascii_lowercase(),
    )
}

/// 跨族比较的唯一判据（v1 §2.3 矩阵）：**同族**返回值序，**跨族**返回 `None`
/// （调用方按「假」处置）。
///
/// `Int × Float` 按 f64 提升值比较（`|i| > 2⁵³` 亦如此）；`NaN` 不可达（写入路径三处
/// 拒绝），故 `partial_cmp` 的 `None` 只会因跨族出现。不给 `Value` 实现 `PartialOrd`：
/// 跨族语义不该长成 `Ord`。
pub fn value_compare(a: &Value, b: &Value) -> Option<Ordering> {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => Some(x.cmp(y)),
        (Value::Int(x), Value::Float(y)) => (*x as f64).partial_cmp(y),
        (Value::Float(x), Value::Int(y)) => x.partial_cmp(&(*y as f64)),
        (Value::Float(x), Value::Float(y)) => x.partial_cmp(y),
        (Value::Bool(x), Value::Bool(y)) => Some(x.cmp(y)),
        (Value::Str(x), Value::Str(y)) => Some(x.cmp(y)),
        _ => None,
    }
}

/// 查询取值：真属性借用；`id` 伪属性就地构造（`Value::Int`，零分配）。
///
/// `id` 是**保留伪属性**（v1 §2.1 修订 ③）：真属性名 `id` 因此在 `where` / `order`
/// 里不可寻址，换来 `query` 对 `scan(lo, hi, tag)` 的语义覆盖。超出 `i64` 的 id 视同
/// 不在场（Lua 整数也表达不了）。
pub enum QueryValue<'a> {
    Attr(&'a Value),
    Id(Value),
}

impl QueryValue<'_> {
    pub fn value(&self) -> &Value {
        match self {
            Self::Attr(value) => value,
            Self::Id(value) => value,
        }
    }
}

impl Entity {
    /// 查询取值（`where` / `order` 的同一出处）：`id` 伪属性 + 真属性。
    pub fn query_value(&self, attr: &str) -> Option<QueryValue<'_>> {
        if attr == "id" {
            i64::try_from(self.id)
                .ok()
                .map(|id| QueryValue::Id(Value::Int(id)))
        } else {
            self.attribute(attr).map(QueryValue::Attr)
        }
    }
}

/// 一条谓词：属性名 + 运算 + `not.` 前缀。
#[derive(Clone, Debug, PartialEq)]
pub struct Predicate {
    pub attr: String,
    pub op: CompareOp,
    pub negated: bool,
}

impl Predicate {
    pub fn new(attr: impl Into<String>, op: CompareOp) -> Self {
        Self {
            attr: attr.into(),
            op,
            negated: false,
        }
    }

    /// `not.` 前缀修饰：取**整个谓词**的反，但**不翻**「缺属性 ⇒ 假」（v2 §1.2）。
    pub fn negate(mut self) -> Self {
        self.negated = true;
        self
    }

    /// 求值：`is` 管存在性，其余 op 对缺属性一律假。
    pub fn matches(&self, entity: &Entity) -> bool {
        let found = entity.query_value(&self.attr);
        if let CompareOp::Is(test) = &self.op {
            return test.holds(found.as_ref().map(QueryValue::value)) != self.negated;
        }
        let Some(found) = found else {
            return false;
        };
        self.op.matches(found.value()) != self.negated
    }
}

/// `order`：属性名 + 方向（族拼写 `field.asc` / `field.desc`；缺省 `asc`）。
#[derive(Clone, Debug, PartialEq)]
pub struct Order {
    pub attr: String,
    pub descending: bool,
}

/// 查询选择器（`ygg.entity.query(selector)` 的解码结果）。
///
/// 四个字段对应四个顶层键（多一个即 `FRAME`：拼错 `whre` 是错误，不是无过滤）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selector {
    /// tag 过滤（最便宜，先于 `where`）。
    pub tag: Option<String>,
    /// 隐式 AND 的谓词表，**保持解码序**（重排会把「写的序」变成隐形性能旋钮）。
    pub predicates: Vec<Predicate>,
    /// 输出序（`None` = id 升序）。
    pub order: Option<Order>,
    /// 投影键（空 = 裸 id 数组）。
    pub select: Vec<String>,
}

impl Selector {
    /// 存活判定：tag 先判、谓词数组序短路。
    pub fn matches(&self, entity: &Entity) -> bool {
        if let Some(tag) = &self.tag {
            if !entity.has_tag(tag) {
                return false;
            }
        }
        self.predicates.iter().all(|pred| pred.matches(entity))
    }
}

/// 排序的**值族**（v2 §5：三族，整浮同族）：`Num < Bool < Str`。
pub fn sort_family(value: &Value) -> u8 {
    match value {
        Value::Int(_) | Value::Float(_) => 0,
        Value::Bool(_) => 1,
        Value::Str(_) => 2,
    }
}

/// 跨族全序（`order` 用）：同族按值序，跨族按 [`sort_family`]。
/// 同族数值恒有 `Some`（`NaN` 不可达），`unwrap_or(Equal)` 只是形式兜底。
pub fn value_sort_cmp(a: &Value, b: &Value) -> Ordering {
    match sort_family(a).cmp(&sort_family(b)) {
        Ordering::Equal => match (a, b) {
            (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
            (Value::Str(x), Value::Str(y)) => x.cmp(y),
            _ => value_compare(a, b).unwrap_or(Ordering::Equal),
        },
        other => other,
    }
}

/// `(值, id)` 序的比较：缺属性排最后（nulls last，与方向无关），值序按方向反转，
/// **id 兜底永不反转**（v2 §5）。
fn compare_order_keys(
    left: &Option<QueryValue<'_>>,
    right: &Option<QueryValue<'_>>,
    descending: bool,
) -> Ordering {
    match (left, right) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => {
            let ord = value_sort_cmp(x.value(), y.value());
            if descending {
                ord.reverse()
            } else {
                ord
            }
        }
    }
}

// ── 属性索引（S24 v2 §2：惰性首建 + 写点增量维护）─────────────────────────
//
// 选型与已否证的两条路（v2 §2.2 逐条论证过，此处只留结论）：
// ① 不「标脏 + 下次重建」——重建 10⁴ 实体 ≈ 0.3 ms，**比不建索引的全扫（0.15 ms）
//    还贵 2×**，写多读少的循环里是净负；
// ② 不放宿主侧（`Rc<RefCell<QueryIndex>>`）——那样失效即重建回到①，而增量补丁要在
//    宿主 6 个写键上各调一次，漏一处 = 静默错答。放进 `EntityStore` 是唯一能让
//    「每个写点必须维护」成为结构约束的位置（五处补丁与 `revision += 1` 同行）。

/// 索引键：四种标量族各占一段，族序即变体声明序（`Num < Float < Bool < Str`）。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum IndexKey {
    Num(i64),
    /// 浮点段的键是**位序化 `u64`**（升序 u64 序 = 数值序）：零新依赖、逐位确定。
    Float(u64),
    Bool(bool),
    Str(String),
}

impl IndexKey {
    pub fn of(value: &Value) -> Self {
        match value {
            Value::Int(number) => Self::Num(*number),
            Value::Float(number) => Self::Float(Self::float_bits(*number)),
            Value::Bool(flag) => Self::Bool(*flag),
            Value::Str(text) => Self::Str(text.clone()),
        }
    }

    /// f64 → 单调 u64（负数取反、非负数翻符号位）。`NaN` 不可达（写入路径三处已拒）。
    fn float_bits(x: f64) -> u64 {
        let bits = x.to_bits();
        if bits >> 63 == 1 {
            !bits
        } else {
            bits | (1 << 63)
        }
    }

    /// [`IndexKey::float_bits`] 的逆。
    fn float_of(bits: u64) -> f64 {
        f64::from_bits(if bits >> 63 == 1 {
            bits ^ (1 << 63)
        } else {
            !bits
        })
    }
}

/// 单属性的有序索引：值 → id 列表（**桶内 id 升序**，`binary_search` 插）。
#[derive(Clone, Debug, Default, PartialEq)]
struct AttrIndex {
    buckets: BTreeMap<IndexKey, Vec<EntityId>>,
}

impl AttrIndex {
    fn insert(&mut self, key: IndexKey, id: EntityId) {
        let bucket = self.buckets.entry(key).or_default();
        if let Err(at) = bucket.binary_search(&id) {
            bucket.insert(at, id);
        }
    }

    fn remove(&mut self, key: &IndexKey, id: EntityId) {
        if let Some(bucket) = self.buckets.get_mut(key) {
            if let Ok(at) = bucket.binary_search(&id) {
                bucket.remove(at);
            }
            if bucket.is_empty() {
                self.buckets.remove(key);
            }
        }
    }
}

/// 比较谓词要的序关系集合（`eq` / `gt` / `gte` / `lt` / `lte` 各一条）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Want {
    Eq,
    Gt,
    Gte,
    Lt,
    Lte,
}

impl Want {
    fn holds(self, ord: Ordering) -> bool {
        match self {
            Self::Eq => ord == Ordering::Equal,
            Self::Gt => ord == Ordering::Greater,
            Self::Gte => ord != Ordering::Less,
            Self::Lt => ord == Ordering::Less,
            Self::Lte => ord != Ordering::Greater,
        }
    }
}

/// 一条谓词在索引上的候选区间（四段上的**并集**；AND = 两两求交）。
///
/// 它是**剪枝结构**，不是权威：候选集出表后仍逐条跑 [`Selector::matches`]。所以区间
/// 只须是超集，而 I2 差分用例钉住「索引路径 ≡ 全扫路径」逐值逐序相等。
#[derive(Clone, Debug, Default, PartialEq)]
struct Probe {
    num: Vec<(i64, i64)>,
    float: Vec<(u64, u64)>,
    boolean: Vec<bool>,
    text: Vec<String>,
}

impl Probe {
    /// 并（`in` 的元素之间）。
    fn union(&self, other: &Self) -> Self {
        let mut out = self.clone();
        out.num.extend_from_slice(&other.num);
        out.float.extend_from_slice(&other.float);
        out.boolean.extend_from_slice(&other.boolean);
        out.text.extend_from_slice(&other.text);
        out.num.sort_unstable();
        out.float.sort_unstable();
        out.boolean.sort_unstable();
        out.text.sort_unstable();
        out
    }

    /// 交（同一属性上的多条谓词 = 隐式 AND；`gte`+`lte` 在这里收成一个区间）。
    fn intersect(&self, other: &Self) -> Self {
        Self {
            num: intersect_spans(&self.num, &other.num),
            float: intersect_spans(&self.float, &other.float),
            boolean: self
                .boolean
                .iter()
                .copied()
                .filter(|flag| other.boolean.contains(flag))
                .collect(),
            text: self
                .text
                .iter()
                .filter(|text| other.text.contains(text))
                .cloned()
                .collect(),
        }
    }

    /// 数值段区间：`hit` 吃「该段的存储值」，`want` 非 `eq` 时真值区必是半线。
    ///
    /// 浮点段的二分域必须裁到 **`[bits(-∞), bits(+∞)]`**：位序化把 NaN 模式推到全域两端
    /// （`Float(u64::MAX)` 是 NaN 位型），全 u64 域上真值区的单调查性不成立。写入路径
    /// 本就拒非有限值 ⇒ 可达键全在这个闭区间内。
    fn numeric_spans(value: &Value, want: Want) -> (Option<(i64, i64)>, Option<(u64, u64)>) {
        let hit = |stored: &Value| value_compare(stored, value).is_some_and(|ord| want.holds(ord));
        let num = monotone_span(i64::MIN as i128, i64::MAX as i128, |raw| {
            hit(&Value::Int(raw as i64))
        })
        .map(|(lo, hi)| (lo as i64, hi as i64));
        let float = monotone_span(
            IndexKey::float_bits(f64::NEG_INFINITY) as i128,
            IndexKey::float_bits(f64::INFINITY) as i128,
            |raw| hit(&Value::Float(IndexKey::float_of(raw as u64))),
        )
        .map(|(lo, hi)| (lo as u64, hi as u64));
        (num, float)
    }
}

/// 两个闭区间列表的交（两两求交，丢空）。列表长度 = `in` 的元素数，量级极小。
fn intersect_spans<T: Copy + Ord>(left: &[(T, T)], right: &[(T, T)]) -> Vec<(T, T)> {
    let mut out = Vec::new();
    for (lo_a, hi_a) in left {
        for (lo_b, hi_b) in right {
            let lo = (*lo_a).max(*lo_b);
            let hi = (*hi_a).min(*hi_b);
            if lo <= hi {
                out.push((lo, hi));
            }
        }
    }
    out
}

/// 单调真值区的二分（`hit` 必须单调 = 真值区是**前/后缀**）。
/// `None` = 全假；`Some((真区下界, 上界))`。
///
/// 只在索引路径的**探测建立期**跑（每次 ≤ 126 轮），不在扫描热路径上。
fn monotone_span(lo0: i128, hi0: i128, hit: impl Fn(i128) -> bool) -> Option<(i128, i128)> {
    if hit(hi0) {
        if hit(lo0) {
            return Some((lo0, hi0));
        }
        let (mut lo, mut hi) = (lo0, hi0);
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            if hit(mid) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        return Some((hi, hi0));
    }
    if !hit(lo0) {
        return None;
    }
    let (mut lo, mut hi) = (lo0, hi0);
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if hit(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some((lo0, lo))
}

/// 一条谓词的可索引探测；`None` = 不可索引（`neq` / `like` / `ilike` / `is`、`not.`、
/// 以及 Str/Bool 上的**序**比较——补集与存在性无法从桶枚举，v2 §3 回全扫）。
fn probe_of(op: &CompareOp) -> Option<Probe> {
    let (value, want) = match op {
        CompareOp::Eq(value) => (value, Want::Eq),
        CompareOp::Gt(value) => (value, Want::Gt),
        CompareOp::Gte(value) => (value, Want::Gte),
        CompareOp::Lt(value) => (value, Want::Lt),
        CompareOp::Lte(value) => (value, Want::Lte),
        CompareOp::In(values) => return in_probe(values),
        CompareOp::Neq(_) | CompareOp::Like(_) | CompareOp::ILike(_) | CompareOp::Is(_) => {
            return None
        }
    };
    let mut probe = Probe::default();
    match value {
        Value::Int(_) | Value::Float(_) if want == Want::Eq => {
            // `eq` 是**点**（Int 段最多两处：整/浮同族的舍入原像），不做二分：
            // 单调二分要求真值区是半线，点不是。
            probe.num = int_preimage(value);
            probe.float = float_preimage(value);
        }
        Value::Int(_) | Value::Float(_) => {
            let (num, float) = Probe::numeric_spans(value, want);
            probe.num.extend(num);
            probe.float.extend(float);
        }
        Value::Bool(flag) => {
            if want != Want::Eq {
                return None;
            }
            probe.boolean.push(*flag);
        }
        Value::Str(text) => {
            if want != Want::Eq {
                return None;
            }
            probe.text.push(text.clone());
        }
    }
    Some(probe)
}

/// `in`：逐元素 `eq` 的并；**空数组的探测是空集**（与「恒假」一致）。
fn in_probe(values: &[Value]) -> Option<Probe> {
    let mut probe: Option<Probe> = None;
    for value in values {
        let one = probe_of(&CompareOp::Eq(value.clone()))?;
        probe = Some(match probe {
            None => one,
            Some(acc) => acc.union(&one),
        });
    }
    Some(probe.unwrap_or_default())
}

/// Int 段上「与 `value` 数值相等」的点集。
fn int_preimage(value: &Value) -> Vec<(i64, i64)> {
    match value {
        Value::Int(number) => vec![(*number, *number)],
        Value::Float(number) => {
            // `(i as f64) == f` 的原像是**舍入原像**（|f| ≥ 2⁵³ 时可能有两个整数），
            // 候选只需 base-1/base/base+1（`as` 饱和，故用 saturating 运算）。
            let base = *number as i64;
            let mut out: Vec<(i64, i64)> = Vec::new();
            for candidate in [base.saturating_sub(1), base, base.saturating_add(1)] {
                if (candidate as f64) == *number && !out.contains(&(candidate, candidate)) {
                    out.push((candidate, candidate));
                }
            }
            out
        }
        _ => Vec::new(),
    }
}

/// Float 段上「与 `value` 数值相等」的点集（整浮同族：`Int(i)` 命中 `f = i as f64`）。
fn float_preimage(value: &Value) -> Vec<(u64, u64)> {
    let number = match value {
        Value::Int(number) => *number as f64,
        Value::Float(number) => *number,
        _ => return Vec::new(),
    };
    let bits = IndexKey::float_bits(number);
    vec![(bits, bits)]
}

impl EntityStore {
    /// 走索引判定（v2 §3 一句话可背）：**所有谓词落在同一属性、每条都可索引、且无
    /// `not.`** ⇒ 候选 id 集（升序去重）；其余一律 `None` = 全扫。
    ///
    /// 空 `where` 也回全扫（那就是列活物，v1 原路径）。`id` 伪属性走
    /// `entities.range()`——`BTreeMap` 本身就是 id 索引，零构建。
    fn index_candidates(&mut self, selector: &Selector) -> Option<Vec<EntityId>> {
        let first = selector.predicates.first()?;
        if selector
            .predicates
            .iter()
            .any(|predicate| predicate.attr != first.attr || predicate.negated)
        {
            return None;
        }
        let mut probe: Option<Probe> = None;
        for predicate in &selector.predicates {
            let one = probe_of(&predicate.op)?;
            probe = Some(match probe {
                None => one,
                Some(acc) => acc.intersect(&one),
            });
        }
        let probe = probe?;
        if first.attr == "id" {
            return Some(self.id_candidates(&probe));
        }
        self.ensure_index(&first.attr);
        Some(self.collect_candidates(&first.attr, &probe))
    }

    /// 惰性首建某属性的索引（已建则零工作）。
    ///
    /// **增量维护只作用在已建索引上**：未建属性的写不付代价，首建时一次性扫全表——
    /// 两向拼起来仍是「增量 ≡ 从零重建」（I1 差分用例就是这个命题）。
    fn ensure_index(&mut self, attr: &str) {
        if self.indices.contains_key(attr) {
            return;
        }
        let mut index = AttrIndex::default();
        for entity in self.entities.values() {
            if let Some(value) = entity.attribute(attr) {
                index.insert(IndexKey::of(value), entity.id);
            }
        }
        self.indices.insert(attr.to_owned(), index);
    }

    /// 从索引取候选 id（升序、去重）。
    fn collect_candidates(&self, attr: &str, probe: &Probe) -> Vec<EntityId> {
        let Some(index) = self.indices.get(attr) else {
            return Vec::new();
        };
        let mut out: Vec<EntityId> = Vec::new();
        for (lo, hi) in &probe.num {
            for (_, ids) in index.buckets.range(IndexKey::Num(*lo)..=IndexKey::Num(*hi)) {
                out.extend_from_slice(ids);
            }
        }
        for (lo, hi) in &probe.float {
            for (_, ids) in index
                .buckets
                .range(IndexKey::Float(*lo)..=IndexKey::Float(*hi))
            {
                out.extend_from_slice(ids);
            }
        }
        for flag in &probe.boolean {
            if let Some(ids) = index.buckets.get(&IndexKey::Bool(*flag)) {
                out.extend_from_slice(ids);
            }
        }
        for text in &probe.text {
            if let Some(ids) = index.buckets.get(&IndexKey::Str(text.clone())) {
                out.extend_from_slice(ids);
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// `id` 上的谓词：直接走 `BTreeMap` 键区间（零索引构建）。
    /// 超出 `i64` 的 id 在 `where` 里恒假（[`QueryValue`] 的同一裁断）。
    fn id_candidates(&self, probe: &Probe) -> Vec<EntityId> {
        let mut out: Vec<EntityId> = Vec::new();
        for (lo, hi) in &probe.num {
            let lo = (*lo).max(1);
            if *hi < lo {
                continue;
            }
            let (Ok(lo), Ok(hi)) = (u64::try_from(lo), u64::try_from(*hi)) else {
                continue;
            };
            out.extend(self.entities.range(lo..=hi).map(|(id, _)| *id));
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// 实体级索引补丁（`insert` / `remove` 两处：属性表整体进出）。
    fn index_entity(&mut self, entity: &Entity, add: bool) {
        for (name, index) in self.indices.iter_mut() {
            let Some(value) = entity.attribute(name) else {
                continue;
            };
            if add {
                index.insert(IndexKey::of(value), entity.id);
            } else {
                index.remove(&IndexKey::of(value), entity.id);
            }
        }
    }

    /// 测试观察面：某属性某键的桶（**桶内 id 升序**是 I3 的断言对象）。
    #[cfg(test)]
    fn index_bucket(&self, attr: &str, key: &IndexKey) -> Option<&[EntityId]> {
        self.indices.get(attr)?.buckets.get(key).map(Vec::as_slice)
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
    fn remove_keeps_the_id_watermark_so_ids_are_never_reused() {
        // S9 锚：真删回收「本体」，但**不回收 id**——`remove` 只动实体表与 `revision`，
        // `next_id` 是独立水位。所以 I1（id 永不复用）与「本体是否永存」无关：
        // 宿主面 `ygg.entity.destroy` 正是建在这条性质上的（回收面不该破分配纪律）。
        let mut store = EntityStore::new();
        let first = store.spawn().unwrap();
        let doomed = store.spawn().unwrap();
        store.set_attribute(doomed, "hp", Value::Int(5)).unwrap();
        store.add_tag(doomed, "enemy").unwrap();
        let watermark = store.next_entity_id();
        let revision_before = store.revision();

        let removed = store.remove(doomed).expect("在场实体必须被移除");
        assert_eq!(
            removed.attribute("hp"),
            Some(&Value::Int(5)),
            "属性随记录一起返回（真删而不是丢引用）"
        );
        assert!(removed.has_tag("enemy"), "tag 也随记录一起回收");
        assert_eq!(store.next_entity_id(), watermark, "真删不得回收 id 水位");
        assert_eq!(store.revision(), revision_before + 1, "真删计一次变更");
        assert!(
            !store.contains(doomed),
            "真删后 contains = false（exists 的 store 层语义）"
        );
        assert!(store.get(doomed).is_none());
        assert!(
            store.remove(doomed).is_none(),
            "幂等：二次删除返回 None、不报错"
        );
        assert_eq!(
            store.revision(),
            revision_before + 1,
            "幂等删除不涨 revision"
        );

        let next = store.spawn().unwrap();
        assert_ne!(next, doomed, "I1：id 永不复用");
        assert!(
            next > doomed && next > first,
            "删后新 id 必须更大：{next} vs doomed={doomed}"
        );
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

    // ── 查询引擎（S24）─────────────────────────────────────────────────────

    /// 建一个实体（属性袋 + tag 名单）。
    fn seeded(id: EntityId, attrs: &[(&str, Value)], tags: &[&str]) -> Entity {
        let mut entity = Entity::new(id);
        for (name, value) in attrs {
            entity.set_attribute(*name, value.clone()).unwrap();
        }
        for tag in tags {
            entity.add_tag(*tag);
        }
        entity
    }

    /// 四族矩阵（v1 §2.3）：同族有值序，**跨族一律 `None`**（调用方按假处置）。
    #[test]
    fn value_compare_matrix_is_within_family_only() {
        let int = Value::Int(2);
        let float = Value::Float(2.0);
        let boolean = Value::Bool(true);
        let text = Value::Str("2".into());

        assert_eq!(value_compare(&int, &Value::Int(3)), Some(Ordering::Less));
        assert_eq!(
            value_compare(&float, &Value::Float(2.5)),
            Some(Ordering::Less)
        );
        assert_eq!(
            value_compare(&Value::Bool(false), &boolean),
            Some(Ordering::Less)
        );
        assert_eq!(
            value_compare(&Value::Str("a".into()), &Value::Str("b".into())),
            Some(Ordering::Less)
        );
        // 整浮同族 + 双向：2 == 2.0（f64 提升值比较）
        assert_eq!(value_compare(&int, &float), Some(Ordering::Equal));
        assert_eq!(value_compare(&float, &int), Some(Ordering::Equal));
        // 整浮身份仍不同：`eq 2` 命中 `2.0`，但 `eq "2"` 不命中任何数值
        assert!(CompareOp::Eq(Value::Int(2)).matches(&float));
        assert!(!CompareOp::Eq(Value::Str("2".into())).matches(&int));
        // 跨族四例
        for (a, b) in [
            (&int, &boolean),
            (&float, &text),
            (&boolean, &text),
            (&text, &int),
        ] {
            assert_eq!(value_compare(a, b), None, "{a:?} × {b:?} 必须跨族");
        }
    }

    /// G1（v2 §6）：10 个 op + `not` 前缀的**命中 / 未命中 / 跨族**三态。
    /// 不做 11×4 穷举（语义只有一份 = [`CompareOp::matches`]）：这里钉「每条 op 的
    /// 接线」与三类边界——跨族、空 `in`、`in` 的任一命中。
    #[test]
    fn where_op_semantics_cover_all_ops() {
        let mut store = EntityStore::new();
        let id = store.spawn().unwrap();
        store.set_attribute(id, "hp", Value::Int(50)).unwrap();
        store
            .set_attribute(id, "name", Value::Str("orc".into()))
            .unwrap();
        let entity = store.get(id).unwrap();

        let cases: Vec<(&str, &str, CompareOp, bool)> = vec![
            ("eq hit", "hp", CompareOp::Eq(Value::Int(50)), true),
            ("eq miss", "hp", CompareOp::Eq(Value::Int(51)), false),
            (
                "eq cross",
                "hp",
                CompareOp::Eq(Value::Str("50".into())),
                false,
            ),
            ("neq hit", "hp", CompareOp::Neq(Value::Int(51)), true),
            ("neq miss", "hp", CompareOp::Neq(Value::Int(50)), false),
            (
                "neq cross",
                "hp",
                CompareOp::Neq(Value::Str("50".into())),
                false,
            ),
            ("gt hit", "hp", CompareOp::Gt(Value::Int(49)), true),
            ("gt miss", "hp", CompareOp::Gt(Value::Int(50)), false),
            ("gte hit", "hp", CompareOp::Gte(Value::Float(50.0)), true),
            ("gte miss", "hp", CompareOp::Gte(Value::Int(51)), false),
            ("lt hit", "hp", CompareOp::Lt(Value::Int(51)), true),
            ("lt miss", "hp", CompareOp::Lt(Value::Int(50)), false),
            ("lte hit", "hp", CompareOp::Lte(Value::Int(50)), true),
            ("lte miss", "hp", CompareOp::Lte(Value::Float(49.5)), false),
            (
                "in hit",
                "hp",
                CompareOp::In(vec![Value::Int(1), Value::Int(50)]),
                true,
            ),
            ("in miss", "hp", CompareOp::In(vec![Value::Int(1)]), false),
            ("in empty", "hp", CompareOp::In(Vec::new()), false),
            ("like hit", "name", CompareOp::Like("or*".into()), true),
            ("like miss", "name", CompareOp::Like("rz*".into()), false),
            ("ilike hit", "name", CompareOp::ILike("OR*".into()), true),
            ("ilike miss", "name", CompareOp::ILike("rc*".into()), false),
            ("is notnull", "hp", CompareOp::Is(IsTest::NotNull), true),
            ("is null", "hp", CompareOp::Is(IsTest::Null), false),
        ];
        for (label, attr, op, expected) in cases {
            let predicate = Predicate::new(attr, op);
            assert_eq!(predicate.matches(entity), expected, "{label}");
            // `not.` 是整谓词取反：在场时逐条翻转
            assert_eq!(
                predicate.clone().negate().matches(entity),
                !expected,
                "not.{label}"
            );
        }
        // `is.true` / `is.false` = 在场且布尔相等（不是「真值性」）
        store.set_attribute(id, "boss", Value::Bool(true)).unwrap();
        store.set_attribute(id, "hp", Value::Int(1)).unwrap();
        let entity = store.get(id).unwrap();
        assert!(Predicate::new("boss", CompareOp::Is(IsTest::True)).matches(entity));
        assert!(!Predicate::new("hp", CompareOp::Is(IsTest::True)).matches(entity));
        assert!(Predicate::new("hp", CompareOp::Is(IsTest::False)).matches(entity) == false);
    }

    /// 缺属性：除 `is` 外一律假——**含 `neq`**（v1 §2.3 的裁断）
    #[test]
    fn missing_attribute_is_false_even_for_neq() {
        let store = EntityStore::new();
        let mut store = store;
        let id = store.spawn().unwrap();
        let entity = store.get(id).unwrap();
        assert!(!Predicate::new("hp", CompareOp::Eq(Value::Int(1))).matches(entity));
        assert!(!Predicate::new("hp", CompareOp::Neq(Value::Int(1))).matches(entity));
        assert!(!Predicate::new("hp", CompareOp::Gt(Value::Int(0))).matches(entity));
        assert!(!Predicate::new("hp", CompareOp::Lte(Value::Int(9))).matches(entity));
        assert!(!Predicate::new("hp", CompareOp::In(vec![Value::Int(1)])).matches(entity));
        assert!(!Predicate::new("hp", CompareOp::Like("*x*".into())).matches(entity));
        assert!(!Predicate::new("hp", CompareOp::ILike("*x*".into())).matches(entity));
    }

    /// G4（v2 §6）：`is` **不受**「缺属性 ⇒ 假」管辖
    #[test]
    fn is_null_and_notnull_are_presence() {
        let mut store = EntityStore::new();
        let present = store.spawn().unwrap();
        let absent = store.spawn().unwrap();
        store.set_attribute(present, "hp", Value::Int(0)).unwrap();
        assert!(
            Predicate::new("hp", CompareOp::Is(IsTest::Null)).matches(store.get(absent).unwrap())
        );
        assert!(!Predicate::new("hp", CompareOp::Is(IsTest::NotNull))
            .matches(store.get(absent).unwrap()));
        assert!(Predicate::new("hp", CompareOp::Is(IsTest::NotNull))
            .matches(store.get(present).unwrap()));
        assert!(
            !Predicate::new("hp", CompareOp::Is(IsTest::Null)).matches(store.get(present).unwrap())
        );
    }

    /// G5（v2 §6）：`not.lt.100` 对**缺属性**仍假（`not.` 不翻这条）
    #[test]
    fn not_prefix_keeps_absent_false() {
        let mut store = EntityStore::new();
        let id = store.spawn().unwrap();
        let entity = store.get(id).unwrap();
        assert!(!Predicate::new("hp", CompareOp::Lt(Value::Int(100)))
            .negate()
            .matches(entity));
        assert!(!Predicate::new("hp", CompareOp::Eq(Value::Int(1)))
            .negate()
            .matches(entity));
        // 在场时 `not.lt.100` ≡ `gte.100`
        store.set_attribute(id, "hp", Value::Int(100)).unwrap();
        let entity = store.get(id).unwrap();
        assert!(Predicate::new("hp", CompareOp::Lt(Value::Int(100)))
            .negate()
            .matches(entity));
    }

    /// `like` / `ilike` 的**四形态**与越界拒绝（v2 §4）
    #[test]
    fn like_shapes_are_exactly_four() {
        assert_eq!(like_shape("orc"), Some(LikeShape::Exact));
        assert_eq!(like_shape("orc*"), Some(LikeShape::Prefix));
        assert_eq!(like_shape("*orc"), Some(LikeShape::Suffix));
        assert_eq!(like_shape("*orc*"), Some(LikeShape::Contains));
        for bad in ["*", "**", "a*b", "a*b*", "*a*b", "a**"] {
            assert_eq!(like_shape(bad), None, "`{bad}` 必须是越界形态");
        }
        assert!(like_matches("goblin", "gob*"));
        assert!(like_matches("goblin", "*lin"));
        assert!(like_matches("goblin", "*obl*"));
        assert!(like_matches("goblin", "goblin"));
        assert!(!like_matches("goblin", "lin"));
        assert!(!like_matches("goblin", "goblins*"));
        // `ilike` 只做 ASCII 折叠（大小写不敏感仅限 ASCII）
        assert!(!like_matches("Goblin", "gob*"));
        assert!(ilike_matches("Goblin", "gob*"));
        assert!(ilike_matches("goblin", "GOBLIN"));
        assert!(!ilike_matches("goblin", "GOBLINS*"));
    }

    /// 确定性第二条：序遍历（乱序 `insert` 也按 id 升序出）
    #[test]
    fn select_orders_by_id_under_shuffled_insert() {
        let mut store = EntityStore::new();
        for id in [7u64, 1, 9, 3] {
            store.insert(Entity::new(id)).unwrap();
        }
        let selector = Selector::default();
        let ids: Vec<u64> = store
            .select(&selector)
            .into_iter()
            .map(|entity| entity.id)
            .collect();
        assert_eq!(ids, vec![1, 3, 7, 9]);
    }

    /// `id` 是保留伪属性：可过滤 id 区间，且**真属性名 `id` 因此不可寻址**
    #[test]
    fn id_pseudo_attribute_filters() {
        let mut store = EntityStore::new();
        for _ in 0..5 {
            store.spawn().unwrap();
        }
        // 真属性 `id` 存在也不参与查询（伪属性优先）
        store.set_attribute(1, "id", Value::Int(999)).unwrap();
        let mut query = |op: CompareOp| {
            let selector = Selector {
                predicates: vec![Predicate::new("id", op)],
                ..Selector::default()
            };
            store
                .select(&selector)
                .into_iter()
                .map(|entity| entity.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(query(CompareOp::Gte(Value::Int(3))), vec![3, 4, 5]);
        assert_eq!(query(CompareOp::Lt(Value::Int(3))), vec![1, 2]);
        assert_eq!(query(CompareOp::Eq(Value::Int(999))), Vec::<u64>::new());
        assert_eq!(query(CompareOp::Is(IsTest::Null)), Vec::<u64>::new());
        assert_eq!(
            query(CompareOp::In(vec![Value::Int(2), Value::Int(4)])),
            vec![2, 4]
        );
    }

    /// `order`：值同 ⇒ id 升序（asc/desc 都不反转）、缺属性排最后、跨族固定族序
    #[test]
    fn order_tiebreak_nulls_last_and_family_rank() {
        let mut store = EntityStore::new();
        store
            .insert(seeded(1, &[("hp", Value::Int(7))], &[]))
            .unwrap();
        store
            .insert(seeded(2, &[("hp", Value::Int(7))], &[]))
            .unwrap();
        store.insert(seeded(3, &[], &[])).unwrap();
        store
            .insert(seeded(4, &[("hp", Value::Int(2))], &[]))
            .unwrap();
        store
            .insert(seeded(5, &[("hp", Value::Str("x".into()))], &[]))
            .unwrap();

        let mut ordered = |descending: bool| {
            let selector = Selector {
                order: Some(Order {
                    attr: "hp".into(),
                    descending,
                }),
                ..Selector::default()
            };
            store
                .select(&selector)
                .into_iter()
                .map(|entity| entity.id)
                .collect::<Vec<_>>()
        };
        // 2 < 7 = 7（id 兜底 1 < 2）< "x"（族序 Num < Str）< 缺属性
        assert_eq!(ordered(false), vec![4, 1, 2, 5, 3]);
        // desc 只反转**值序**：族序随之反转，7 = 7 的 id 兜底仍是升序，缺属性仍在最后
        assert_eq!(ordered(true), vec![5, 1, 2, 4, 3]);
    }

    // ── 索引（S24 v2 §6 的 I1/I2/I3）───────────────────────────────────────

    /// 一个选择器的 id 出表（两条路径的比较样板）。
    fn ids_of(store: &mut EntityStore, selector: &Selector) -> Vec<EntityId> {
        store
            .select(selector)
            .into_iter()
            .map(|entity| entity.id)
            .collect()
    }

    fn one_predicate(op: CompareOp) -> Selector {
        Selector {
            predicates: vec![Predicate::new("hp", op)],
            ..Selector::default()
        }
    }

    /// 浮点段按**数值序**（位序化 u64），且段序 `Num < Float < Bool < Str`
    #[test]
    fn index_keys_follow_numeric_order() {
        let keys: Vec<IndexKey> = [-3.5, -2.0, -1e-9, 0.0, 1e-9, 1.0, 2.0]
            .iter()
            .map(|x| IndexKey::of(&Value::Float(*x)))
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "浮点段必须按数值序（位序化 u64）");
        assert!(IndexKey::Num(i64::MAX) < IndexKey::Float(0));
        assert!(IndexKey::Float(0) < IndexKey::Bool(false));
        assert!(IndexKey::Bool(true) < IndexKey::Str(String::new()));
    }

    /// I3（v2 §6）：乱序 `insert` 后桶内仍 id 升序
    #[test]
    fn index_buckets_are_id_ascending() {
        let mut store = EntityStore::new();
        for id in [7u64, 1, 9, 3] {
            store
                .insert(seeded(id, &[("hp", Value::Int(5))], &[]))
                .unwrap();
        }
        store.ensure_index("hp");
        assert_eq!(
            store.index_bucket("hp", &IndexKey::Num(5)),
            Some([1u64, 3, 7, 9].as_slice())
        );
    }

    /// I2（v2 §6）：**索引路径 ≡ 全扫路径**逐值逐序相等。
    ///
    /// 两条路径的造法：给同一条选择器补一条必然成立的 `id >= 0` ⇒ 两个不同属性 ⇒
    /// 按 v2 §3 强制回全扫；除谓词表之外两选择器逐字相同。
    #[test]
    fn index_path_equals_scan_path() {
        let mut store = EntityStore::new();
        store
            .insert(seeded(1, &[("hp", Value::Int(50))], &[]))
            .unwrap();
        store
            .insert(seeded(2, &[("hp", Value::Float(50.0))], &[]))
            .unwrap();
        store
            .insert(seeded(3, &[("hp", Value::Int(7))], &[]))
            .unwrap();
        store.insert(seeded(4, &[], &[])).unwrap();
        store
            .insert(seeded(5, &[("hp", Value::Str("50".into()))], &[]))
            .unwrap();

        let ops = vec![
            CompareOp::Eq(Value::Int(50)),
            CompareOp::Gte(Value::Int(7)),
            CompareOp::Lt(Value::Float(50.0)),
            CompareOp::Lte(Value::Int(50)),
            CompareOp::In(vec![Value::Int(7), Value::Str("50".into())]),
            CompareOp::In(Vec::new()),
        ];
        for op in ops {
            let indexed = one_predicate(op.clone());
            let mut scanned = indexed.clone();
            scanned
                .predicates
                .push(Predicate::new("id", CompareOp::Gte(Value::Int(0))));
            let via_index = ids_of(&mut store, &indexed);
            let via_scan = ids_of(&mut store, &scanned);
            assert_eq!(via_index, via_scan, "{op:?} 的两条路径必须逐值逐序相等");
        }
        // 索引路径真的建了索引（否则上面那组用例是空转）
        assert!(store.indices.contains_key("hp"));
        // 命中值的逐条核对（不是「两条路径一起错」）
        assert_eq!(
            ids_of(&mut store, &one_predicate(CompareOp::Eq(Value::Int(50)))),
            vec![1, 2]
        );
        assert_eq!(
            ids_of(&mut store, &one_predicate(CompareOp::Lte(Value::Int(50)))),
            vec![1, 2, 3]
        );
        assert_eq!(
            ids_of(
                &mut store,
                &one_predicate(CompareOp::Lt(Value::Float(50.0)))
            ),
            vec![3]
        );
    }

    /// I1（v2 §6）：随机写 200 次（spawn / insert / set / remove_attr / destroy）后，
    /// **增量索引 ≡ 从零重建**。重建侧 = 克隆体（`Clone` 丢索引 ⇒ 首查即从零建）。
    #[test]
    fn index_incremental_equals_rebuild() {
        let mut store = EntityStore::new();
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..5 {
            store.spawn().unwrap();
        }
        let probe_selector = one_predicate(CompareOp::Gte(Value::Int(0)));
        // 先建索引：后续 200 次写全部走增量维护
        ids_of(&mut store, &probe_selector);

        let values = [
            Value::Int(1),
            Value::Int(50),
            Value::Float(2.5),
            Value::Float(-0.0),
            Value::Str("x".into()),
            Value::Bool(true),
        ];
        for step in 0..200 {
            let roll = next() % 5;
            let target = (next() % 10 + 1) as EntityId;
            let value = values[(next() % 6) as usize].clone();
            match roll {
                0 => {
                    store.spawn().unwrap();
                }
                1 => {
                    let _ = store.insert(seeded(target, &[("hp", value)], &[]));
                }
                2 => {
                    let _ = store.set_attribute(target, "hp", value);
                }
                3 => {
                    let _ = store.remove_attribute(target, "hp");
                }
                _ => {
                    store.remove(target);
                }
            }
            // 读写交错：只写不读测不出「增量维护漏一处」的交错形状
            if step % 20 == 0 {
                ids_of(&mut store, &probe_selector);
            }
        }

        let ops = [
            CompareOp::Eq(Value::Int(50)),
            CompareOp::Gte(Value::Int(1)),
            CompareOp::Lt(Value::Float(2.5)),
            CompareOp::In(vec![Value::Int(1), Value::Str("x".into())]),
        ];
        let mut rebuilt = store.clone();
        assert!(rebuilt.indices.is_empty(), "克隆必须丢掉索引（重建侧前提）");
        for op in ops {
            let selector = one_predicate(op.clone());
            let incremental = ids_of(&mut store, &selector);
            let from_scratch = ids_of(&mut rebuilt, &selector);
            assert_eq!(incremental, from_scratch, "{op:?}：增量索引 ≡ 从零重建");
        }
    }
}
