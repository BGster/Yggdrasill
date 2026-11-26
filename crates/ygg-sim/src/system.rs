//! 系统面：阶段化注册表（Pre → Rules → Post，阶段内注册序即执行序）+ tick 内**延迟结构操作**队列。
//!
//! 契约：sim-kernel §3.4（系统执行序 = 列表下标序；tick 内不改 archetype）、§4（系统列表是数据）；
//! `artifacts/w2-world-api-design.md` §2（调度器声明与 `seal`）。
//!
//! # 五个设计点
//!
//! 1. **系统是数据**：`entries` 是 `Vec<(Stage, SystemDecl, SystemFn)>`。执行序 = 阶段序
//!    （Pre → Rules → Post）→ 阶段内注册序。`after` 只是**约束校验器**，不是调度器：
//!    [`SystemSet::seal`] 从不排序——排序会造出第二套执行序来源，两个来源迟早不一致。
//! 2. **tick 内不改 archetype**：系统要 spawn / despawn / insert 时只能把操作推进
//!    [`StructuralOp`] 队列，由 `Sim::step` 在**tick 边界按 FIFO** 应用。于是系统看到的
//!    行集与行序在整帧内稳定，换进/换出的顺序也可复算。
//! 3. **世界只有一个入口：查询面**。`SystemCtx` 不交出 `&mut ArchetypeStore`——系统只能
//!    [`SystemCtx::query`] / [`SystemCtx::query_mut`] 读写列，结构变更只能 `defer_*`。
//!    「绕过声明的写入」在类型上就不成立，不靠约定（`tests/decl_honesty.rs` 的静态前提）。
//! 4. **闭包用 `Rc<RefCell<..>>` 共享**：`Sim::replay` 需要在一份**新** Sim 上跑同一批系统，
//!    而系统是宿主注册的 `FnMut`（不可序列化）。用共享句柄复制列表，比要求宿主「重新注册
//!    一遍同样的系统」少一条隐式契约。
//! 5. **`seal` 是终点线**：校验通过后 `add_system` 一律 `YGG-SIM-SEALED`（[`crate::error::codes::SEALED`]），且 `seal` 可重入
//!    （同一声明再调一次给出同一答案）。于是「注册完忘了封盘」不会被漏检。

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use crate::archetype::Column;
use crate::component::{ComponentId, EntityId, Registry};
use crate::error::{SimError, SimResult};
use crate::fixed::Fixed;
use crate::query::RowSet;
use crate::rng::RngRegistry;
use crate::store::ArchetypeStore;

// ---------------------------------------------------------------------------
// 阶段与声明
// ---------------------------------------------------------------------------

/// 一 tick 内的三个阶段（执行序 = 本枚举的声明序：Pre → Rules → Post）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Stage {
    /// 输入 / 预处理（读上 tick 的世界、写本 tick 的输入）。
    Pre,
    /// 规则主体（游戏逻辑）。
    Rules,
    /// 收尾（统计、清理、呈现投影）。
    Post,
}

impl Stage {
    /// 全部阶段，**按执行序**（`SystemSet::run` 与 `names()` 的唯一遍历源）。
    pub const ALL: [Stage; 3] = [Stage::Pre, Stage::Rules, Stage::Post];

    /// 诊断名（与枚举变体逐字一致，便于日志对账）。
    pub fn name(self) -> &'static str {
        match self {
            Stage::Pre => "Pre",
            Stage::Rules => "Rules",
            Stage::Post => "Post",
        }
    }
}

/// 一条系统的声明：名称 + 读写组件集 + 同阶段先后约束。
///
/// 声明是**数据**（可打印、可校验、可进报告），与实现（`FnMut`）分开存放：
/// `seal` 只看声明，`step` 只看实现。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemDecl {
    /// 系统名（同一 Sim 内唯一——`after` 按名引用，重名会让引用有歧义）。
    pub name: String,
    /// 读的组件（只读；与他系统同阶段的写入不冲突）。
    pub reads: Vec<ComponentId>,
    /// 写的组件（同阶段内一个组件只能有一个写者）。
    pub writes: Vec<ComponentId>,
    /// 同阶段的先后约束（「本系统排在谁之后」，按名引用）。
    pub after: Vec<String>,
}

impl SystemDecl {
    /// 只有名字的声明（其余字段按需追加）。
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            reads: Vec::new(),
            writes: Vec::new(),
            after: Vec::new(),
        }
    }

    /// 追加读集。
    pub fn reads(mut self, comps: &[ComponentId]) -> Self {
        self.reads.extend_from_slice(comps);
        self
    }

    /// 追加写集。
    pub fn writes(mut self, comps: &[ComponentId]) -> Self {
        self.writes.extend_from_slice(comps);
        self
    }

    /// 追加同阶段先后约束。
    pub fn after(mut self, names: &[&str]) -> Self {
        self.after
            .extend(names.iter().map(|name| (*name).to_string()));
        self
    }
}

// ---------------------------------------------------------------------------
// 系统上下文
// ---------------------------------------------------------------------------

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

    /// 按组件集遍历匹配行并**写入**（`ArchetypeStore::for_each_mut` 的系统面）。
    ///
    /// 遍历序 = 桶下标升序 → 行号升序；闭包拿不到结构入口，结构变更只能走 `defer_*`。
    pub fn query_mut<F>(&mut self, set: &[ComponentId], f: F) -> SimResult<u64>
    where
        F: FnMut(&mut RowSet<'_>) -> SimResult<()>,
    {
        self.store.for_each_mut(set, f)
    }

    /// 按组件集遍历匹配行（只读）。未知组件按「匹配 0 行」处理（要报错用 `query_mut`）。
    pub fn query<F>(&self, set: &[ComponentId], f: F) -> u64
    where
        F: FnMut(&RowSet<'_>),
    {
        self.store.for_each(set, f)
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

// ---------------------------------------------------------------------------
// 注册表
// ---------------------------------------------------------------------------

/// 系统注册表：`Vec<(Stage, SystemDecl, SystemFn)>`，注册序即阶段内执行序。
#[derive(Default, Clone)]
pub struct SystemSet {
    entries: Vec<(Stage, SystemDecl, SystemFn)>,
    sealed: bool,
}

impl SystemSet {
    /// 空注册表（未封盘）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 追加一条系统。插入序 = 阶段内执行序（不排序、不重排）。
    ///
    /// `seal` 之后一律 `YGG-SIM-SEALED`（[`crate::error::codes::SEALED`]）：追加等于在已校验的声明表后面接一条未校验的
    /// 声明，让 `seal` 的结论实际失效。
    pub fn add<F>(&mut self, decl: SystemDecl, stage: Stage, func: F) -> SimResult<()>
    where
        F: FnMut(&mut SystemCtx<'_>) -> SimResult<()> + 'static,
    {
        if self.sealed {
            return Err(SimError::sealed(format!(
                "已 seal，拒绝再注册系统 {:?}（先改声明再重开一次 run）",
                decl.name
            )));
        }
        self.entries
            .push((stage, decl, Rc::new(RefCell::new(Box::new(func)))));
        Ok(())
    }

    /// 封盘：**只校验，不排序**。校验通过后 `add` 一律拒绝；可重入（再调给出同一答案）。
    ///
    /// 校验项（按此顺序，先命中先返回）：
    /// 1. 系统名重复；
    /// 2. `reads`/`writes` 里的组件 id 未注册；
    /// 3. 同一阶段内两个系统写同一组件（写写冲突）；
    /// 4. `after` 指向不存在的系统名；
    /// 5. `after` 指向**别的阶段**的系统（跨阶段先后没有意义：阶段序已定）；
    /// 6. 同阶段 `after` 成环。
    ///
    /// **同阶段「一个写、另一个只读」是合法的**：读不产生冲突，也不需要排序——
    /// 真需要先后，就把两者放进不同阶段，或用 `after` 表达（写者在前）。
    pub fn seal(&mut self, registry: &Registry) -> SimResult<()> {
        self.validate(registry)?;
        self.sealed = true;
        Ok(())
    }

    /// 是否已封盘。
    pub fn is_sealed(&self) -> bool {
        self.sealed
    }

    /// 系统数。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 系统名（**执行序**：阶段序 → 阶段内注册序；诊断用）。
    pub fn names(&self) -> Vec<&str> {
        let mut out = Vec::with_capacity(self.entries.len());
        for stage in Stage::ALL {
            for (entry_stage, decl, _) in &self.entries {
                if *entry_stage == stage {
                    out.push(decl.name.as_str());
                }
            }
        }
        out
    }

    /// 按阶段序、阶段内注册序执行一遍。
    ///
    /// 借用纪律：每次调用只 `borrow_mut` 一个系统，绝不跨系统持有。
    pub fn run(&self, ctx: &mut SystemCtx<'_>) -> SimResult<()> {
        for stage in Stage::ALL {
            for (entry_stage, _, func) in &self.entries {
                if *entry_stage != stage {
                    continue;
                }
                let mut func = func.borrow_mut();
                func(ctx)?;
            }
        }
        Ok(())
    }

    /// `seal` 的校验部分（`&self`：可被诊断面复用而不改状态）。
    pub fn validate(&self, registry: &Registry) -> SimResult<()> {
        // ① 名称唯一（`after` 按名引用 ⇒ 重名即歧义）。
        let mut stages: BTreeMap<&str, Stage> = BTreeMap::new();
        for (stage, decl, _) in &self.entries {
            if stages.insert(decl.name.as_str(), *stage).is_some() {
                return Err(SimError::sealed(format!("系统名重复：{:?}", decl.name)));
            }
        }

        // ② 组件 id 必须已注册；③ 同阶段写写冲突。
        let mut writers: BTreeMap<(Stage, ComponentId), &str> = BTreeMap::new();
        for (stage, decl, _) in &self.entries {
            for &comp in decl.reads.iter().chain(decl.writes.iter()) {
                registry.type_of(comp).map_err(|_| {
                    SimError::sealed(format!("系统 {:?} 声明了未注册的组件 id {comp}", decl.name))
                })?;
            }
            for &comp in &decl.writes {
                if let Some(first) = writers.insert((*stage, comp), decl.name.as_str()) {
                    return Err(SimError::sealed(format!(
                        "阶段 {} 内写写冲突：系统 {first:?} 与 {:?} 都写组件 id {comp}",
                        stage.name(),
                        decl.name
                    )));
                }
            }
        }

        // ④⑤ `after` 必须指向本阶段的已知系统。
        for (stage, decl, _) in &self.entries {
            for name in &decl.after {
                let Some(target) = stages.get(name.as_str()) else {
                    return Err(SimError::sealed(format!(
                        "系统 {:?} 的 after 指向未知系统 {name:?}",
                        decl.name
                    )));
                };
                if target != stage {
                    return Err(SimError::sealed(format!(
                        "系统 {:?}（阶段 {}）的 after 指向别的阶段的系统 {name:?}（阶段 {}）",
                        decl.name,
                        stage.name(),
                        target.name()
                    )));
                }
            }
        }

        // ⑥ 同阶段 `after` 成环（跨阶段边已被 ⑤ 拒，环只可能在阶段内）。
        let names: Vec<&str> = self
            .entries
            .iter()
            .map(|(_, d, _)| d.name.as_str())
            .collect();
        let mut deps: Vec<Vec<usize>> = vec![Vec::new(); self.entries.len()];
        for (index, (_, decl, _)) in self.entries.iter().enumerate() {
            for name in &decl.after {
                if let Some(target) = names.iter().position(|known| known == name) {
                    deps[index].push(target);
                }
            }
        }
        // 0 = 未访问、1 = 栈上、2 = 已完成（DFS 着色；下标存在 `Vec` 里，无 HashMap）。
        let mut colour = vec![0u8; self.entries.len()];
        for start in 0..self.entries.len() {
            if colour[start] == 0 {
                cycle_check(start, &deps, &mut colour, &names)?;
            }
        }
        Ok(())
    }
}

/// 迭代式 DFS 着色：沿 `after` 边走，撞到仍在栈上的节点即成环。
fn cycle_check(
    start: usize,
    deps: &[Vec<usize>],
    colour: &mut [u8],
    names: &[&str],
) -> SimResult<()> {
    // 栈元素：(节点, 下一个待访问的依赖下标)
    let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
    colour[start] = 1;
    while let Some((node, cursor)) = stack.last_mut().map(|frame| (frame.0, &mut frame.1)) {
        let next = deps[node].get(*cursor).copied();
        *cursor += 1;
        let Some(dep) = next else {
            colour[node] = 2;
            stack.pop();
            continue;
        };
        match colour[dep] {
            0 => {
                colour[dep] = 1;
                stack.push((dep, 0));
            }
            1 => {
                return Err(SimError::sealed(format!(
                    "after 约束成环：系统 {:?} 与 {:?} 互为先后",
                    names[node], names[dep]
                )));
            }
            _ => {}
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 延迟结构操作
// ---------------------------------------------------------------------------

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
