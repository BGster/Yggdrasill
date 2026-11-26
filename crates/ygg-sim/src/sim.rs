//! `Sim`：tick 循环、延迟结构操作、hash trace、录制与回放（Tier 1 的对外门面）。
//!
//! 契约：sim-kernel §3.4（tick 纪律）+ §4（API 面）+ I1/I3（确定性回放）。
//!
//! # 一次 `step` 的五步（顺序即语义，别调换）
//!
//! 1. 应用**上一个 tick 边界**留下的结构操作（FIFO）——正常情况下这一步是空的；
//!    它存在是为了「宿主在两次 step 之间投了批量帧」与「系统内部投了延迟操作」共用一个入口。
//! 2. 按 `systems` 的**阶段序 → 阶段内注册序**逐个执行，每个系统拿到同一份 `SystemCtx`。
//! 3. 应用系统在本 tick 内提交的延迟结构操作（FIFO）——**只有**这里会改 archetype，
//!    所以系统看到的行集与行序在整帧内稳定。
//! 4. `tick += 1`。
//! 5. 算 state hash（= 快照尾 digest），进 trace / 录制日志。
//!
//! # 五步是一笔事务（M2-B）
//!
//! 任何一步返回 `Err`，模拟态必须回到调用前的样子——`tick`、世界（值 + 结构 + 实体数）、
//! `rng` 流位置、输入日志长度全不变。所以 [`Sim::step`] 在开场取三份便宜的快照，
//! 失败时按三个通道各自复原：
//!
//! - **值写入**：逐格 undo 日志（[`crate::journal`]），回滚代价 ≈ 本 tick 写过的 cell 数；
//! - **结构操作**：落笔前先用只读影子状态整队预检（[`QueuePreview`]），"可能失败" 的队列根本不落笔，
//!   失败时整队丢弃（同时丢掉本 tick 系统新投的那几条）；
//! - **随机流与日志**：流状态拷贝（每流一个 `[u32;4]`）与日志长度，回退到开场值。
//!
//! 三份都不含"整个世界"：undo 日志随写入量走，流数量一般是 1–3，日志长度是一个 `usize`。
//! 所以原子化不改变 tick 的复杂度量级（不用 world clone）。
//!
//! # 为什么 hash 每次都真的编码一遍快照
//!
//! 「一条等式统摄快照与哈希」（`hash == fnv(快照字节)`）只有在**真的编码一次**时才是
//! 可执行的等式。另写一条「不编码直接算 hash」的快路径会让两者迟早分叉，而分叉的表现
//! 是「存档能读、回放对不上」这种最难查的故障。代价是一次编码（10⁴ 实体约 0.2–0.7 MB），
//! 在 §5 的 16.6 ms 预算里占比很小。
//!
//! # 回放为什么是实例方法而不是关联函数
//!
//! 计划稿写的是 `Sim::replay(snapshot, log)`。**系统是宿主注册的闭包**（不可序列化），
//! 所以回放必须拿到「同一批系统」才能重跑——关联函数凭空拿不到它们。于是回放做成
//! 实例方法：`self` 只提供系统列表（`Rc` 共享），实际重跑发生在一份**新建** Sim 上。
//! 语义边界不变（调用者不传状态，只借用系统表）。

use crate::archetype::Column;
use crate::canonical::FrameReader;
use crate::component::{AttrType, ComponentDef, ComponentId, EntityId};
use crate::error::{SimError, SimResult};
use crate::fixed::Fixed;
use crate::hash;
use crate::input::{OpKind, ReplayLog};
use crate::rng::RngRegistry;
use crate::snapshot;
use crate::store::ArchetypeStore;
use crate::system::{
    QueuePreview, Resources, Stage, StructuralOp, SystemCtx, SystemDecl, SystemSet,
};

/// `spawn_batch` 的回执。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpawnReport {
    /// 新生成的实体数。
    pub spawned: u32,
    /// 下一个可用 id（不含）。
    pub next_id: EntityId,
}

/// `insert_batch` 的回执。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InsertReport {
    /// 被迁移的实体数。
    pub entities: u32,
    /// 写入的值个数（含零宽组件计数为 0）。
    pub values: u32,
}

/// 回放报告（`first_mismatch = None` 才算逐 tick 一致）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplayReport {
    /// 回放了几个 tick。
    pub ticks: u64,
    /// hash 与前缀一致的 tick 数。
    pub matched: u64,
    /// 第一个失配的 tick（`None` = 全程一致）。
    pub first_mismatch: Option<u64>,
    /// 回放结束时的 state hash。
    pub final_hash: u64,
}

/// 模拟态全量（tick + 世界 + 资源）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimState {
    /// tick 计数。
    pub tick: u64,
    /// 世界（列式 archetype）。
    pub world: ArchetypeStore,
    /// 资源表（`KV<u64>`：键升序）。
    pub resources: Resources,
}

/// 模拟内核门面。
pub struct Sim {
    seed: u64,
    state: SimState,
    systems: SystemSet,
    pending: Vec<StructuralOp>,
    trace: Option<Vec<(u64, u64)>>,
    recorder: Option<ReplayLog>,
    rng: RngRegistry,
    /// 上次成功事务写过的格 `(实体, 组件)`（升序去重）。
    ///
    /// 只在事务成功时刷新、失败时清空：它描述「刚刚发生的那一件事」的状态，
    /// 不是累计面也不是事件流。来源是 undo 日志（[`ArchetypeStore::written_cells`]），
    /// 所以不存在「某个写点忘了报」这类漏报。
    changed: Vec<(EntityId, ComponentId)>,
}

impl Sim {
    /// 新世界（`seed` 是**唯一**随机源入口；I5）。
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            state: SimState {
                tick: 0,
                world: ArchetypeStore::new(),
                resources: Resources::new(),
            },
            systems: SystemSet::new(),
            pending: Vec::new(),
            trace: None,
            recorder: None,
            rng: RngRegistry::new(seed),
            changed: Vec::new(),
        }
    }

    /// 注册一个组件（幂等）。
    pub fn register(&mut self, name: &str, def: ComponentDef) -> SimResult<ComponentId> {
        self.state.world.register(name, def)
    }

    /// 注册一条系统（`decl` + 阶段；插入序即阶段内执行序，不排序）。
    ///
    /// `seal()` 之后一律 `YGG-SIM-SEALED`：声明表已校验，追加等于让它失效。
    pub fn add_system<F>(&mut self, decl: SystemDecl, stage: Stage, func: F) -> SimResult<()>
    where
        F: FnMut(&mut SystemCtx<'_>) -> SimResult<()> + 'static,
    {
        self.systems.add(decl, stage, func)
    }

    /// 封盘：**只校验，不排序**（重复名 / 未注册组件 / 同阶段写写冲突 / 跨阶段 `after` /
    /// 未知 `after` / 同阶段 `after` 成环）。可重入：再调给出同一答案。
    pub fn seal(&mut self) -> SimResult<()> {
        self.systems.seal(self.state.world.registry())
    }

    /// 注册表是否已封盘。
    pub fn is_sealed(&self) -> bool {
        self.systems.is_sealed()
    }

    /// 当前 tick。
    pub fn tick(&self) -> u64 {
        self.state.tick
    }

    /// 种子。
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// 取一个 32 位随机数（流名进 `RngRegistry`，与系统内的 [`crate::system::SystemCtx::rng32`]
    /// 同一条流：宿主面与系统面用同一个名字 ⇒ 同一个序列，不会出现第二条随机源）。
    ///
    /// 这是 `world.rng` 的宿主面入口：宿主在 tick 之外也要能取数（例如装载期决定初始分布），
    /// 而**不能**在沙箱侧另起一个 `RngRegistry`——那份状态不进 `snapshot` / 回放，
    /// 会让「同种子同操作序两次运行」在第一次取随机数时分成两条历史。
    pub fn rng32(&mut self, stream: &str) -> u32 {
        self.rng.next32(stream)
    }

    /// 世界（只读）。
    pub fn world(&self) -> &ArchetypeStore {
        &self.state.world
    }

    /// 世界（写面：宿主直接改列/批量操作时用，风险自负——绕过确定性纪律的入口只有这一个）。
    pub fn world_mut(&mut self) -> &mut ArchetypeStore {
        &mut self.state.world
    }

    /// 资源表（只读）。
    pub fn resources(&self) -> &Resources {
        &self.state.resources
    }

    /// 资源表（写面）。
    pub fn resources_mut(&mut self) -> &mut Resources {
        &mut self.state.resources
    }

    /// 系统名（**执行序**：阶段序 → 阶段内注册序；诊断用）。
    pub fn system_names(&self) -> Vec<&str> {
        self.systems.names()
    }

    /// 系统表 `(名, 阶段)`（**执行序**）：宿主 `ready` 帧的唯一来源。
    ///
    /// 只走 `SystemSet`：内核之外再存一张系统表就是第二份真相。
    pub fn systems(&self) -> Vec<(&str, Stage)> {
        self.systems.list()
    }

    /// 上次成功事务写过的格 `(实体, 组件)`（升序去重；事务外或有错时为空）。
    ///
    /// 增量回执的**唯一**来源：宿主不需要（也不允许）在各个写点另装仪器，
    /// 逐格旧值本来就已经进了 undo 日志。
    pub fn changed(&self) -> &[(EntityId, ComponentId)] {
        &self.changed
    }

    /// 推进一 tick，返回该 tick 结束时的 state hash。
    ///
    /// # 失败 = 什么都没发生
    ///
    /// 本方法是模拟态唯一的可失败写面，也是唯一的事务边界：返回 `Err` 时 `tick`、世界
    /// （值 + 结构 + 实体数）、`rng` 流位置、输入日志都与调用前逐字节相等，调用方不需要
    /// 任何补偿逻辑（[`crate::error`] 头注「这条性质是怎么被满足的」三类写面）。
    pub fn step(&mut self, dt: Fixed) -> SimResult<u64> {
        // 事务开场：三份便宜的快照（结构队列与值写入走 undo 日志 + 预检）。
        let rng_mark = self.rng.snapshot_streams();
        let log_mark = self.recorder.as_ref().map_or(0, |log| log.records().len());
        self.state.world.begin_tx();
        match self.step_inner(dt) {
            Ok(digest) => {
                // 增量面必须在 commit 之前取：`commit_tx` 会丢掉整本 undo 日志。
                self.changed = self.state.world.written_cells();
                self.state.world.commit_tx();
                Ok(digest)
            }
            Err(err) => {
                // 顺序不重要（三个通道各自独立），但先收值、再收结构、最后收随机与日志，
                // 使「回滚做了什么」与「开场取快照的顺序」读起来对称。
                self.state.world.rollback_tx();
                self.rng.restore_streams(rng_mark);
                // 本 tick 系统新投的延迟操作随这次失败一起丢：它们只对「没发生的那一 tick」有意义。
                self.pending.clear();
                if let Some(log) = &mut self.recorder {
                    log.truncate_records(log_mark);
                }
                // 失败即什么都没写：增量面不能留下一个「看起来写了」的回执。
                self.changed.clear();
                Err(err)
            }
        }
    }

    /// 宿主批量写面的事务边界：**先全量校验、后落笔**由调用方保证，
    /// 这里只负责「要么全成、要么全回」并把增量面刷新到 [`Sim::changed`]。
    ///
    /// 允许嵌套：在 `step` 的事务里调用时不再开新事务（也不回滚）——
    /// 失败会被外层 `step` 整批回滚，回滚所有权始终在最外层。
    pub fn transact<R>(&mut self, write: impl FnOnce(&mut Sim) -> SimResult<R>) -> SimResult<R> {
        let outer = !self.state.world.in_tx();
        if outer {
            self.state.world.begin_tx();
        }
        match write(self) {
            Ok(value) => {
                self.changed = self.state.world.written_cells();
                if outer {
                    self.state.world.commit_tx();
                }
                Ok(value)
            }
            Err(err) => {
                if outer {
                    self.state.world.rollback_tx();
                    self.changed.clear();
                }
                Err(err)
            }
        }
    }

    /// 宿主命令面的事务边界：**开**。
    ///
    /// [`Sim::transact`] 把闭包里的写折成一个事务，代价是闭包期间持有 `&mut self`；
    /// 命令面的多次写却发生在**互不相邻的调用**里（一条命令里的每次 `world.*` 自己
    /// 走一层嵌套 `transact`，其间宿主还要借同一个内核读组件表），闭包形态在那里
    /// 直接撞上 `YGG-WORLD-BUSY`。开合因此拆成三个显式方法：`begin_write_tx` →
    /// 若干次写 → `commit_write_tx` / `rollback_write_tx`，共用**同一本** undo 日志。
    ///
    /// **undo 日志只盖值写入**：结构操作（建桶 / 行序 / `next_id`）与随机流位置不在
    /// 日志里，需要连同这两面一起复原的调用方自己取快照——宿主命令面就是这么叠的：
    /// 快照盖结构与随机，日志盖值。
    ///
    /// # Panics
    ///
    /// 事务已开时再开会撞上 `ArchetypeStore::begin_tx` 的嵌套检查（debug 断言）。
    /// 命令面的写入发生在 `step` 之外，正常路径上不会嵌套。
    pub fn begin_write_tx(&mut self) {
        self.state.world.begin_tx();
    }

    /// 宿主命令面的事务边界：**收**，给出整段写过的格（升序去重）。
    ///
    /// 与 [`Sim::step`] 同一口径：增量面必须在 `commit_tx` 之前取，那一步会丢掉整本
    /// undo 日志；这也是嵌套 `transact` 每次都能刷新 [`Sim::changed`] 的原因。
    pub fn commit_write_tx(&mut self) -> Vec<(EntityId, ComponentId)> {
        let cells = self.state.world.written_cells();
        self.state.world.commit_tx();
        self.changed = cells.clone();
        cells
    }

    /// 宿主命令面的事务边界：**回**（值写入逐格复原，[`Sim::changed`] 清空）。
    ///
    /// 结构写与随机流位置不经 undo 日志：要求「什么都没发生」覆盖这两面的调用方
    /// 自己叠一层 [`Sim::restore`]（宿主命令面在失败路径上就是这么做的）。
    pub fn rollback_write_tx(&mut self) {
        self.state.world.rollback_tx();
        self.changed.clear();
    }

    /// `step` 的事务体：五步全在这里，任何一步失败直接上抛，复原统一由 [`Sim::step`] 做。
    fn step_inner(&mut self, dt: Fixed) -> SimResult<u64> {
        // ① 上个边界留下的操作（宿主批量帧已在调用时立即应用；这里只可能是内部排队）。
        self.apply_pending()?;
        // ② 系统按序执行。
        let tick = self.state.tick;
        {
            let mut ctx = SystemCtx::new(
                tick,
                dt,
                &mut self.state.world,
                &mut self.rng,
                &mut self.pending,
            );
            self.systems.run(&mut ctx)?;
        }
        // ③ 系统提交的延迟结构操作（tick 边界，FIFO）。
        self.apply_pending()?;
        // ④ tick 前进。
        self.state.tick = tick + 1;
        // ⑤ hash 与证据面。
        let digest = self.hash();
        if let Some(trace) = &mut self.trace {
            trace.push((self.state.tick, digest));
        }
        if let Some(log) = &mut self.recorder {
            log.push_tick(self.state.tick, dt, digest);
        }
        Ok(digest)
    }

    /// 按 FIFO 应用延迟结构操作（**唯一**改 archetype 的入口之一）。
    ///
    /// 「全绿才落笔」：先用只读影子状态 ([`QueuePreview`]) 把整队预检一遍，任何一条会失败就
    /// 整队拒绍。结构写（建桶 / 行序 / `next_id` / `entity_index`）不可逐格撤销，所以
    /// 原子化的唯一手段是「落笔之前就知道会成功」。（剩余漏洞：预检过后 apply 仍可能撞到
    /// 内核内部 `YGG-SIM-STATE`——那是内核缺陷，不是载荷错。）
    fn apply_pending(&mut self) -> SimResult<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        {
            let mut preview = QueuePreview::new(&self.state.world);
            for op in &self.pending {
                preview.check(op)?;
            }
        }
        let ops = std::mem::take(&mut self.pending);
        for op in ops {
            op.apply(&mut self.state.world)?;
        }
        Ok(())
    }

    /// 当前 state hash（= 快照尾 digest）。
    pub fn hash(&self) -> u64 {
        hash::state_hash(&self.snapshot())
    }

    /// 当前快照帧（含尾 digest）。
    pub fn snapshot(&self) -> Vec<u8> {
        snapshot::encode(
            self.state.tick,
            &self.state.resources,
            &self.rng,
            &self.state.world,
        )
    }

    /// 从快照恢复（先校验后改：解码失败时本对象**不变**）。
    pub fn restore(&mut self, bytes: &[u8]) -> SimResult<()> {
        // 先校验后改：解码与重绑定都可能在半途失败，二者都不写 self。
        let registry = &mut self.state.world.registry_mut();
        let decoded = snapshot::rebind(snapshot::decode(bytes)?, registry)?;
        self.seed = decoded.rng.seed();
        self.state.tick = decoded.tick;
        self.state.resources = decoded.resources;
        self.state.world = decoded.store;
        self.rng = decoded.rng;
        self.pending.clear();
        // 录制从恢复点重新起算（旧日志的 start_tick 已经不对了，续录会造成 tick 倒退）。
        if self.recorder.is_some() {
            self.recorder = Some(ReplayLog::new(self.seed, self.state.tick));
        }
        Ok(())
    }

    /// 开关逐 tick hash trace。
    pub fn trace(&mut self, on: bool) {
        if on {
            if self.trace.is_none() {
                self.trace = Some(Vec::new());
            }
        } else {
            self.trace = None;
        }
    }

    /// trace 条目（`(tick, hash)`）。
    pub fn trace_entries(&self) -> Option<&[(u64, u64)]> {
        self.trace.as_deref()
    }

    /// 开关输入日志录制。
    pub fn record(&mut self, on: bool) {
        if on {
            if self.recorder.is_none() {
                self.recorder = Some(ReplayLog::new(self.seed, self.state.tick));
            }
        } else {
            self.recorder = None;
        }
    }

    /// 取走录制中的日志（`None` = 未在录制）。
    pub fn take_log(&mut self) -> Option<ReplayLog> {
        self.recorder.take()
    }

    /// 批量生成实体（列式帧）。
    pub fn spawn_batch(&mut self, frame: &[u8]) -> SimResult<SpawnReport> {
        let (rows, cols, tags) = self.decode_batch(frame)?;
        let ids = self.state.world.spawn_columns(rows, &cols, &tags)?;
        if let Some(log) = &mut self.recorder {
            log.push_op(OpKind::Spawn, frame.to_vec());
        }
        Ok(SpawnReport {
            spawned: ids.len() as u32,
            next_id: self.state.world.next_id(),
        })
    }

    /// 批量删除实体（`u32 n | u64 ids[n]`）。
    pub fn despawn_batch(&mut self, frame: &[u8]) -> SimResult<usize> {
        let mut reader = FrameReader::open(frame)?;
        let n = reader.u32()? as usize;
        let mut ids = Vec::with_capacity(n);
        for _ in 0..n {
            ids.push(reader.u64()?);
        }
        reader.finish()?;
        let removed = self.state.world.despawn_ids(&ids)?;
        if let Some(log) = &mut self.recorder {
            log.push_op(OpKind::Despawn, frame.to_vec());
        }
        Ok(removed)
    }

    /// 批量插入组件（列式帧 + 目标 id 列表）。
    pub fn insert_batch(&mut self, frame: &[u8]) -> SimResult<InsertReport> {
        let mut reader = FrameReader::open(frame)?;
        let rows = reader.u32()? as usize;
        let mut ids = Vec::with_capacity(rows);
        for _ in 0..rows {
            ids.push(reader.u64()?);
        }
        let rest = frame_from_reader(&mut reader, rows, &self.state.world)?;
        reader.finish()?;
        let (_, cols, tags) = rest;
        let values = self.state.world.insert_columns(&ids, &cols, &tags)?;
        if let Some(log) = &mut self.recorder {
            log.push_op(OpKind::Insert, frame.to_vec());
        }
        Ok(InsertReport {
            entities: ids.len() as u32,
            values: values as u32,
        })
    }

    /// 解码「`u32 n` 之后」的通用列式载荷（spawn 与 insert 共用）。
    fn decode_batch(&self, frame: &[u8]) -> SimResult<DecodedBatch> {
        let mut reader = FrameReader::open(frame)?;
        let rows = reader.u32()? as usize;
        let parsed = frame_from_reader(&mut reader, rows, &self.state.world)?;
        reader.finish()?;
        Ok(parsed)
    }

    /// 回放（`snapshot` 为空 = 从创世重跑；否则先从快照恢复）。
    pub fn replay(&self, snapshot: &[u8], log: &[u8]) -> SimResult<ReplayReport> {
        let log = ReplayLog::decode(log)?;
        let mut sim = Sim::new(log.seed());
        sim.systems = self.systems.clone();
        // 系统与批量帧都按**名称**寻址，所以原型 Sim 的注册表（schema）必须先搬过来；
        // 搬过来之后 `restore` 的 rebind 会保持 id 不变，系统捕获的 id 继续有效。
        sim.state.world =
            ArchetypeStore::from_parts(self.state.world.registry().clone(), Vec::new(), 1)?;
        if !snapshot.is_empty() {
            sim.restore(snapshot)?;
            if log.start_tick() != 0 && sim.tick() != log.start_tick() {
                return Err(SimError::frame(format!(
                    "日志起点 tick {} 与快照 tick {} 不符",
                    log.start_tick(),
                    sim.tick()
                )));
            }
        } else if log.start_tick() != 0 {
            return Err(SimError::frame(format!(
                "日志声明从 tick {} 起，但没有提供快照",
                log.start_tick()
            )));
        }
        let mut matched = 0u64;
        let mut first_mismatch: Option<u64> = None;
        for record in log.records() {
            // 该 tick 开始前的结构操作按录制序重投（原字节 → 同一个解码器）。
            for op in &record.ops {
                match op.kind {
                    OpKind::Spawn => {
                        sim.spawn_batch(&op.frame)?;
                    }
                    OpKind::Despawn => {
                        sim.despawn_batch(&op.frame)?;
                    }
                    OpKind::Insert => {
                        sim.insert_batch(&op.frame)?;
                    }
                }
            }
            let got = sim.step(record.dt)?;
            if got == record.hash {
                matched += 1;
            } else if first_mismatch.is_none() {
                first_mismatch = Some(record.tick);
            }
        }
        Ok(ReplayReport {
            ticks: log.records().len() as u64,
            matched,
            first_mismatch,
            final_hash: sim.hash(),
        })
    }
}

/// 读「`u32 n` 之后」的列式载荷：`u16 comp_n | [u16 len | name]×comp_n | 每有列组件一列`。
///
/// `tag` 只出现在名单里、没有载荷——所以「名单 → 有列 / 零宽」的分派必须查注册表，
/// 一帧解码结果：行数 × （组件列，声明序） × （本帧出现的组件 id，升序）。
type DecodedBatch = (usize, Vec<(ComponentId, Column)>, Vec<ComponentId>);

/// 而注册表是解析的唯一权威（帧里不重复写类型）。
fn frame_from_reader(
    reader: &mut FrameReader<'_>,
    rows: usize,
    world: &ArchetypeStore,
) -> SimResult<DecodedBatch> {
    // 帧布局是**两段式**：先把全部组件名读完，再按同一顺序读载荷。逐名读载荷会错位——
    // 因为载荷紧跟在整张名单之后（这条在 `tests/snapshot_roundtrip.rs` 的帧往返里钉住）。
    let comp_n = reader.u16()? as usize;
    let mut plan: Vec<(ComponentId, AttrType)> = Vec::with_capacity(comp_n);
    for _ in 0..comp_n {
        let name_len = reader.u16()? as usize;
        let raw = reader.raw(name_len)?;
        let name = std::str::from_utf8(raw)
            .map_err(|err| SimError::frame(format!("帧里的组件名不是合法 UTF-8：{err}")))?;
        let id = world.registry().id_of(name)?;
        let ty = world.registry().type_of(id)?;
        plan.push((id, ty));
    }
    let mut cols: Vec<(ComponentId, Column)> = Vec::with_capacity(comp_n);
    let mut tags: Vec<ComponentId> = Vec::new();
    for (id, ty) in plan {
        let column = match ty {
            // tag 只出现在名单里、没有载荷。
            AttrType::Tag => {
                tags.push(id);
                continue;
            }
            AttrType::Number { .. } => {
                let mut values = Vec::with_capacity(rows);
                for _ in 0..rows {
                    values.push(reader.i64()?);
                }
                Column::I64(values)
            }
            AttrType::Ref => {
                let mut values = Vec::with_capacity(rows);
                for _ in 0..rows {
                    values.push(reader.u64()?);
                }
                Column::U64(values)
            }
            AttrType::Bool => {
                let mut values = Vec::with_capacity(rows);
                for _ in 0..rows {
                    let byte = reader.u8()?;
                    if byte > 1 {
                        return Err(SimError::frame(format!(
                            "组件 id {id} 是 boolean 列，帧里出现非 0/1 值 {byte}"
                        )));
                    }
                    values.push(byte);
                }
                Column::U8(values)
            }
            AttrType::Str => {
                let mut off = vec![0u32];
                let mut buf: Vec<u8> = Vec::new();
                for _ in 0..rows {
                    let bytes = reader.bytes()?;
                    buf.extend_from_slice(bytes);
                    let end = u32::try_from(buf.len()).map_err(|_| {
                        SimError::frame(format!("组件 id {id} 的字符串列超出 4 GiB"))
                    })?;
                    off.push(end);
                }
                Column::Bytes { off, buf }
            }
            AttrType::F64Bits => {
                let mut values = Vec::with_capacity(rows);
                for _ in 0..rows {
                    // 读边界 fail-closed：非 canonical（-0、非规范 NaN 载荷）即拒
                    // （sim-kernel I2：列内存只收 pack 的像）。
                    let raw = reader.u64()?;
                    if !crate::bits::is_canonical(raw) {
                        return Err(SimError::frame(format!(
                            "组件 id {id} 的位串列出现非 canonical 位串：0x{raw:016X}"
                        )));
                    }
                    values.push(raw);
                }
                Column::F64Bits(values)
            }
        };
        cols.push((id, column));
    }
    Ok((rows, cols, tags))
}
