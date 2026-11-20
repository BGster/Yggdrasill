//! 系统面：注册制系统列表（顺序即执行序）+ tick 内**延迟结构操作**队列。
//!
//! 契约：sim-kernel §3.4（系统执行序 = 列表下标序；tick 内不改 archetype）、§4（系统列表是数据）。
//!
//! # 三个设计点
//!
//! 1. **系统是数据**：`systems` 是 `Vec`，下标序即执行序。不引入「优先级字段 + 排序」——
//!    那是第二套执行序来源，两个来源迟早不一致。
//! 2. **tick 内不改 archetype**：系统要 spawn / despawn / insert 时只能把操作推进
//!    [`StructuralOp`] 队列，由 `Sim::step` 在**tick 边界按 FIFO** 应用。于是系统看到的
//!    行集与行序在整帧内稳定，换进/换出的顺序也可复算。
//! 3. **闭包用 `Rc<RefCell<..>>` 共享**：`Sim::replay` 需要在一份**新** Sim 上跑同一批系统，
//!    而系统是宿主注册的 `FnMut`（不可序列化）。用共享句柄复制列表，比要求宿主「重新注册
//!    一遍同样的系统」少一条隐式契约。

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use crate::archetype::Column;
use crate::component::EntityId;
use crate::error::SimResult;
use crate::fixed::Fixed;
use crate::rng::RngRegistry;
use crate::store::ArchetypeStore;

/// 一次 tick 的系统上下文（系统拿到的一切都在这里，不再有全局态）。
pub struct SystemCtx<'a> {
    tick: u64,
    dt: Fixed,
    store: &'a mut ArchetypeStore,
    rng: &'a mut RngRegistry,
    pending: &'a mut Vec<StructuralOp>,
}

impl<'a> SystemCtx<'a> {
    /// 构造（`Sim::step` 是唯一调用者）。
    pub fn new(
        tick: u64,
        dt: Fixed,
        store: &'a mut ArchetypeStore,
        rng: &'a mut RngRegistry,
        pending: &'a mut Vec<StructuralOp>,
    ) -> Self {
        Self {
            tick,
            dt,
            store,
            rng,
            pending,
        }
    }

    /// 本 tick 的序号（从 0 起；`step` 返回后 +1）。
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// 本 tick 的步长（定点）。
    pub fn dt(&self) -> Fixed {
        self.dt
    }

    /// 列写面（按桶下标序、行序 dense）。
    pub fn store_mut(&mut self) -> &mut ArchetypeStore {
        self.store
    }

    /// 取一个 32 位随机数（流名用 [`crate::rng::stream_name`] 生成）。
    pub fn rng32(&mut self, stream: &str) -> u32 {
        self.rng.next32(stream)
    }

    /// 当前随机流状态（诊断用）。
    pub fn rng_state(&self, stream: &str) -> Option<[u32; 4]> {
        self.rng.state_of(stream)
    }

    /// 延迟 spawn（tick 边界按 FIFO 应用）。
    pub fn defer_spawn(
        &mut self,
        rows: usize,
        cols: Vec<(crate::component::ComponentId, Column)>,
        tags: Vec<crate::component::ComponentId>,
    ) {
        self.pending.push(StructuralOp::Spawn { rows, cols, tags });
    }

    /// 延迟 despawn。
    pub fn defer_despawn(&mut self, ids: Vec<EntityId>) {
        self.pending.push(StructuralOp::Despawn { ids });
    }

    /// 延迟 insert（加列迁移）。
    pub fn defer_insert(
        &mut self,
        ids: Vec<EntityId>,
        cols: Vec<(crate::component::ComponentId, Column)>,
        tags: Vec<crate::component::ComponentId>,
    ) {
        self.pending.push(StructuralOp::Insert { ids, cols, tags });
    }
}

/// 系统函数（`FnMut`：系统可以持有跨 tick 的私有状态，但那份状态必须自己保证确定性）。
pub type SystemFn = Rc<RefCell<Box<dyn FnMut(&mut SystemCtx<'_>) -> SimResult<()>>>>;

/// 系统 id（注册序；只在本次运行内有效）。
pub type SystemId = u32;

/// 一条系统登记（名称 + 实现）。
pub struct SystemEntry {
    name: String,
    func: SystemFn,
}

impl SystemEntry {
    /// 名称（诊断面；同一 Sim 内不要求唯一，但重名会让证据难读）。
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Clone for SystemEntry {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            func: Rc::clone(&self.func),
        }
    }
}

/// 系统集合（`Vec` 序即执行序）。
#[derive(Default, Clone)]
pub struct SystemSet {
    entries: Vec<SystemEntry>,
}

impl SystemSet {
    /// 空集合。
    pub fn new() -> Self {
        Self::default()
    }

    /// 追加一条系统（返回其 id = 下标）。
    pub fn add<F>(&mut self, name: &str, func: F) -> SystemId
    where
        F: FnMut(&mut SystemCtx<'_>) -> SimResult<()> + 'static,
    {
        let id = self.entries.len() as SystemId;
        self.entries.push(SystemEntry {
            name: name.to_string(),
            func: Rc::new(RefCell::new(Box::new(func))),
        });
        id
    }

    /// 系统数。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 系统名（执行序）。
    pub fn names(&self) -> Vec<&str> {
        self.entries.iter().map(|entry| entry.name()).collect()
    }

    /// 按序执行一遍（借用纪律：每次调用只 `borrow_mut` 一个系统，绝不跨系统持有）。
    pub fn run(&self, ctx: &mut SystemCtx<'_>) -> SimResult<()> {
        for entry in &self.entries {
            let mut func = entry.func.borrow_mut();
            func(ctx)?;
        }
        Ok(())
    }
}

/// 系统在 tick 内提交的**结构操作**（tick 边界按 FIFO 应用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructuralOp {
    /// 生成 `rows` 个实体（列式载荷 + 零宽成员）。
    Spawn {
        /// 行数。
        rows: usize,
        /// 有列组件的载荷。
        cols: Vec<(crate::component::ComponentId, Column)>,
        /// 零宽成员。
        tags: Vec<crate::component::ComponentId>,
    },
    /// 删除实体。
    Despawn {
        /// 目标实体。
        ids: Vec<EntityId>,
    },
    /// 加列迁移。
    Insert {
        /// 目标实体。
        ids: Vec<EntityId>,
        /// 有列组件的载荷。
        cols: Vec<(crate::component::ComponentId, Column)>,
        /// 零宽成员。
        tags: Vec<crate::component::ComponentId>,
    },
}

impl StructuralOp {
    /// 应用一条操作（`Sim::step` 在 tick 边界调用；顺序即 `Vec` 序）。
    pub fn apply(self, store: &mut ArchetypeStore) -> SimResult<()> {
        match self {
            StructuralOp::Spawn { rows, cols, tags } => {
                store.spawn_columns(rows, &cols, &tags)?;
            }
            StructuralOp::Despawn { ids } => {
                store.despawn_ids(&ids)?;
            }
            StructuralOp::Insert { ids, cols, tags } => {
                store.insert_columns(&ids, &cols, &tags)?;
            }
        }
        Ok(())
    }
}

/// 资源表的类型别名（`KV<u64>`：键升序、值 u64、无浮点、无表）。
pub type Resources = BTreeMap<u64, u64>;
