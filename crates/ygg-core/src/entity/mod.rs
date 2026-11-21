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

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// —— 模块装配：按子系统拆文件，纯搬移（S-25 批五）——
mod compare;
mod index;
mod selector;
mod store;
mod value;

#[cfg(test)]
mod tests;

pub use compare::{
    ilike_matches, like_matches, like_shape, value_compare, CompareOp, IsTest, LikeShape,
};
pub use index::{sort_family, value_sort_cmp, IndexKey};
pub use selector::{Order, Predicate, QueryValue, Selector};
pub use store::{Entity, StateError};
pub use value::{normalize_f64, EntityId, Value};

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

impl EntityStore {
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

// —— 跨面件归位装配面（批五编译器终审）：IsTest::holds 被 selector 面调用、index_entity 被
//    store 写面调用、ensure_index 被 tests 面调用——私有性以 entity 根为界即全部可见，
//    零可见性放宽（批四 json_kind 同法）——
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
