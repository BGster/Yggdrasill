//! 组件注册表：`AttrType`（类型词汇）与名称 → 内部 id 的唯一映射。
//!
//! 契约：sim-kernel §2 的组件形状投影表（Tier 2 属性袋的**类型词汇 + 命名投影**）。
//!
//! # 三条硬规矩
//!
//! 1. **内部 id 不是持久事实**。`ComponentId` 只是本进程内的列下标；快照与 state hash
//!    一律用**组件名**排序（[`Registry::names_in_byte_order`]）。所以「同一个世界，两种
//!    注册顺序」得到不同的内部 id、但快照字节与 hash **完全相同**——这条由
//!    `tests/snapshot_roundtrip.rs` 的 `id_order_independent` 用例钉住，也是跨进程一致性
//!    不依赖注册顺序的证据。
//! 2. **注册后永不移除**。`Registry` 只增；删除会让位集（[`CompSet`]）出现洞，而位集是
//!    archetype 的键——带洞的键会让「同一组组件」有多个等价表示。代价是组件名占驻内存，
//!    这在 10² 量级是无意义的开销。
//! 3. **类型词汇单源**。四个 Tier 2 属性类型字面量（`number` / `string` / `boolean` / `ref`）
//!    与 [`AttrType::label`] 逐字相同，`tests/determinism_scan.rs` 扫
//!    `packages/libs/entity/init.lua` 并断言两侧一致——改一侧即红（assetgen 的
//!    `rng-not-copied` 同范式）。`tag` 是 Tier 1 追加的第五词汇（Tier 2 侧写作
//!    `tags.<name>`，无值标记）。

use std::collections::BTreeMap;

use crate::error::{SimError, SimResult};

/// 实体 id：从 1 起、单调、**不复用**（与 `ygg-core` 的 `EntityStore` 同纪律）。
/// `0` 保留为「无引用」（`AttrType::Ref` 的空值）。
pub type EntityId = u64;

/// 组件内部 id（列下标）。**不进快照、不进 hash、不进 Lua 面**——见模块头注 1。
pub type ComponentId = u32;

/// 组件集合的位集表示（位 = 组件内部 id）。
///
/// `u128` 是刻意的上界：一份 archetype 的组件数不会超过 128（超过即 [`SimError::schema`]），
/// 而位集让「键比较」是一条整数比较，也让 `by_key: BTreeMap<CompSet, _>` 的序天然确定。
pub type CompSet = u128;

/// 一份组件的类型（Tier 2 属性袋的投影）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttrType {
    /// `type = "number"`：定点整数列。`frac_bits` 只是**声明**（内核不解释小数位）。
    Number { frac_bits: u8 },
    /// `type = "string"`：UTF-8 字节列（`off` + `buf`）。
    Str,
    /// `type = "boolean"`：`u8` 列，只允许 0/1。
    Bool,
    /// `type = "ref"`：`u64` 列，`0` = 无引用；悬垂 ref 不自动修。
    Ref,
    /// `tags.<name>`：零宽组件，只进 archetype key，不产生列。
    Tag,
}

impl AttrType {
    /// Tier 2 的类型字面量（**单点声明**：`determinism_scan` 用例断言它与 `entity.lua` 逐字相同）。
    pub const fn label(self) -> &'static str {
        match self {
            AttrType::Number { .. } => "number",
            AttrType::Str => "string",
            AttrType::Bool => "boolean",
            AttrType::Ref => "ref",
            AttrType::Tag => "tag",
        }
    }

    /// 线格式的类型字节（快照与批量帧共用）：1..4 + 5 = Tag。
    pub const fn wire_tag(self) -> u8 {
        match self {
            AttrType::Number { .. } => 1,
            AttrType::Str => 2,
            AttrType::Bool => 3,
            AttrType::Ref => 4,
            AttrType::Tag => 5,
        }
    }

    /// 线格式的类型字节 → 类型（`frac_bits` 由调用方从帧里补，非 Number 时被忽略）。
    pub fn from_wire(tag: u8, frac_bits: u8) -> SimResult<AttrType> {
        Ok(match tag {
            1 => AttrType::Number {
                frac_bits: if frac_bits == 0 { 32 } else { frac_bits },
            },
            2 => AttrType::Str,
            3 => AttrType::Bool,
            4 => AttrType::Ref,
            5 => AttrType::Tag,
            other => {
                return Err(SimError::frame(format!(
                    "未知的组件类型字节：{other}（合法 1..=5）"
                )))
            }
        })
    }

    /// 是否产生存储列（`Tag` 不产生）。
    pub const fn has_column(self) -> bool {
        !matches!(self, AttrType::Tag)
    }
}

/// `register` 的入参（形状即 Tier 2 的 `{ type = ... }`，`required` / `default` 不下沉）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentDef {
    /// 组件类型。
    pub ty: AttrType,
}

impl ComponentDef {
    /// 便捷构造：`number`（缺省 Q32.32）。
    pub const fn number() -> Self {
        Self {
            ty: AttrType::Number { frac_bits: 32 },
        }
    }

    /// 便捷构造：`string`。
    pub const fn string() -> Self {
        Self { ty: AttrType::Str }
    }

    /// 便捷构造：`boolean`。
    pub const fn boolean() -> Self {
        Self { ty: AttrType::Bool }
    }

    /// 便捷构造：`ref`。
    pub const fn reference() -> Self {
        Self { ty: AttrType::Ref }
    }

    /// 便捷构造：`tag`（零宽）。
    pub const fn tag() -> Self {
        Self { ty: AttrType::Tag }
    }
}

/// 名称 → 内部 id 的唯一映射（只增）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Registry {
    /// 按注册序（下标即 [`ComponentId`]）。
    names: Vec<String>,
    /// 按类型（与 `names` 同下标）。
    types: Vec<AttrType>,
    /// 名称 → id（`BTreeMap`：查找不依赖哈希序）。
    ids: BTreeMap<String, ComponentId>,
}

impl Registry {
    /// 空注册表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 已注册组件数。
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// 注册一个组件；同名重复注册**允许且幂等**（返回既有 id，类型不同即 [`SimError::schema`]）。
    ///
    /// 幂等而不是报错：`Sim::register` 会被宿主在装载期反复调用（同一份 schema 声明可能
    /// 来自多个包），把「已经声明过同样的东西」判成错误会让装载序变成隐式契约。
    pub fn register(&mut self, name: &str, def: ComponentDef) -> SimResult<ComponentId> {
        if name.is_empty() {
            return Err(SimError::schema("组件名不得为空串"));
        }
        if let Some(&id) = self.ids.get(name) {
            if self.types[id as usize] != def.ty {
                return Err(SimError::schema(format!(
                    "组件 `{name}` 已注册为 {}，不能改注册为 {}",
                    self.types[id as usize].label(),
                    def.ty.label()
                )));
            }
            return Ok(id);
        }
        let id = self.names.len() as ComponentId;
        if id >= 128 {
            return Err(SimError::schema(format!(
                "组件数超过位集上限 128（注册 `{name}` 时）"
            )));
        }
        self.names.push(name.to_string());
        self.types.push(def.ty);
        self.ids.insert(name.to_string(), id);
        Ok(id)
    }

    /// 名称 → id；未注册即 [`SimError::schema`]。
    pub fn id_of(&self, name: &str) -> SimResult<ComponentId> {
        self.ids
            .get(name)
            .copied()
            .ok_or_else(|| SimError::schema(format!("未知组件名：`{name}`")))
    }

    /// 名称 → id（不报错版，供「是否已注册」判断）。
    pub fn find(&self, name: &str) -> Option<ComponentId> {
        self.ids.get(name).copied()
    }

    /// id → 名称。
    pub fn name_of(&self, id: ComponentId) -> SimResult<&str> {
        self.names
            .get(id as usize)
            .map(String::as_str)
            .ok_or_else(|| SimError::schema(format!("未知组件 id：{id}")))
    }

    /// id → 类型。
    pub fn type_of(&self, id: ComponentId) -> SimResult<AttrType> {
        self.types
            .get(id as usize)
            .copied()
            .ok_or_else(|| SimError::schema(format!("未知组件 id：{id}")))
    }

    /// 位集 → 组件 id 升序列表。
    pub fn ids_of_set(&self, set: CompSet) -> Vec<ComponentId> {
        (0..self.names.len() as ComponentId)
            .filter(|id| set & (1u128 << id) != 0)
            .collect()
    }

    /// 位集 → 组件名，按**名字节序升序**（快照/批量帧的规范序；见模块头注 1）。
    pub fn names_in_byte_order(&self, set: CompSet) -> Vec<ComponentId> {
        let mut ids = self.ids_of_set(set);
        ids.sort_by(|a, b| {
            self.names[*a as usize]
                .as_bytes()
                .cmp(self.names[*b as usize].as_bytes())
        });
        ids
    }
}
