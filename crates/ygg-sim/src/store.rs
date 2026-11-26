//! `ArchetypeStore`：archetype 容器 + 实体索引 + 组件注册表的唯一所有者。
//!
//! 契约：sim-kernel §2（数据形状）、§3.4（tick 内不改 archetype，结构操作延迟到边界）、
//! §2「行序与删除语义」。
//!
//! # 三条不变量（本模块是它们的唯一执行点）
//!
//! 1. **行序 = dense 插入序**，删除走 `swap_remove`（末行填洞）并**同步** `entity_index`。
//!    回放对行序强耦合：改掉 despawn 策略即改逐 tick hash。所以「被填洞的行 = 原末行」
//!    不是实现细节，是契约语义。
//! 2. **内部 id 不是持久事实**。`by_key` 用位集（[`CompSet`]）作键，而快照与 hash 一律按
//!    **组件名**排序（[`crate::component::Registry::names_in_byte_order`]），于是「两种注册
//!    顺序、同一世界」给出相同字节与相同 hash。
//! 3. **先校验后改**。所有面（spawn / insert / despawn）先做完整校验再落笔：拒绍不产生
//!    半改的模拟态，调用方拿到错误后不需要补偿逻辑。唯一做不到「零改动」的面是 spawn 的
//!    id 分配与 insert 的逐实体迁移（它们是纯结构写，不可逐格撤销）——所以 step 里的结构
//!    操作先经 [`crate::system::QueuePreview`] 只读预检、全绿才落笔；值写入则由本结构持有
//!    的 [`Journal`] 逐格记旧值，step 失败时逆序回放（[`crate::journal`]）。
//!
//! # 为什么 `by_key` 是 `BTreeMap` 而不是 `HashMap`
//!
//! 不是性能选择（10² 量级的桶，两者都够快），是**确定性纪律**：`HashMap` 的迭代序由
//! 随机 hasher 种子决定，任何「遍历桶」的路径都会跨进程换序。`BTreeMap` 的序由键决定，
//! 与进程无关。`tests/determinism_scan.rs` 的 R3 扫描保证本 crate 里没有 `HashMap`。

use std::collections::BTreeMap;

use crate::archetype::{Archetype, Column, ValueRef};
use crate::component::{AttrType, CompSet, ComponentDef, ComponentId, EntityId, Registry};
use crate::error::{SimError, SimResult};
use crate::journal::Journal;

/// 一个组件类型对应的空列（`Tag` 没有列 ⇒ `None`）。
fn column_for(ty: AttrType) -> Option<Column> {
    match ty {
        AttrType::Number { .. } => Some(Column::I64(Vec::new())),
        AttrType::Ref => Some(Column::U64(Vec::new())),
        AttrType::Bool => Some(Column::U8(Vec::new())),
        AttrType::Str => Some(Column::Bytes {
            off: vec![0],
            buf: Vec::new(),
        }),
        AttrType::F64Bits => Some(Column::F64Bits(Vec::new())),
        AttrType::Tag => None,
    }
}

/// 列式实体存储：archetype 桶 + 实体索引 + 注册表。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArchetypeStore {
    archetypes: Vec<Archetype>,
    /// 位集 → 桶下标（`BTreeMap`：序由键决定，与进程无关）。
    by_key: BTreeMap<CompSet, u32>,
    /// 实体 id → (桶下标, 行号)。
    entity_index: BTreeMap<EntityId, (u32, u32)>,
    /// 下一个可用实体 id（从 1 起、单调、**不复用**）。
    next_id: EntityId,
    registry: Registry,
    /// step 事务的 undo 日志（`None` = 不在 step 里；`Some` = 值写入逐格登记）。
    ///
    /// 为什么挂在存储上而不是当参数层层传：唯一需要在 step 中途拿到它的是
    /// [`crate::query::RowSet`]，而它已经从存储借走了 `&mut [Archetype]`——把日志放进
    /// 同一结构，`query.rs` 才能用一次字段拆分同时拿到两者（不引入第二把锁、不加 `Rc`）。
    journal: Option<Journal>,
}

impl ArchetypeStore {
    /// 空世界（`next_id` 从 1 起：`0` 保留为「无引用」）。
    pub fn new() -> Self {
        Self {
            next_id: 1,
            ..Self::default()
        }
    }

    /// 注册一个组件（幂等；同名不同型即 [`SimError::schema`]）。
    pub fn register(&mut self, name: &str, def: ComponentDef) -> SimResult<ComponentId> {
        self.registry.register(name, def)
    }

    /// 注册表只读视图。
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// 注册表可写视图（**只有** `restore` 的重绑定路径需要它：快照里可能有本进程还没登记的组件）。
    pub fn registry_mut(&mut self) -> &mut Registry {
        &mut self.registry
    }

    /// 全部 archetype（桶下标 = 创建序）。
    pub fn archetypes(&self) -> &[Archetype] {
        &self.archetypes
    }

    /// 全部 archetype（系统执行的写面：按桶下标序迭代，行序 dense）。
    pub fn archetypes_mut(&mut self) -> &mut [Archetype] {
        &mut self.archetypes
    }

    /// 实体数。
    pub fn entity_count(&self) -> usize {
        self.entity_index.len()
    }

    /// 桶数。
    pub fn archetype_count(&self) -> usize {
        self.archetypes.len()
    }

    /// 下一个实体 id（不含）。
    pub fn next_id(&self) -> EntityId {
        self.next_id
    }

    /// 实体 id → (桶下标, 行号)；未知 id 即 [`SimError::entity`]。
    pub fn locate(&self, id: EntityId) -> SimResult<(u32, u32)> {
        self.entity_index
            .get(&id)
            .copied()
            .ok_or_else(|| SimError::entity(format!("未知实体 id：{id}")))
    }

    /// 实体所在的桶键（未知 id 即 [`SimError::entity`]）。
    ///
    /// [`crate::system::QueuePreview`] 用它判断「加宽后的位集是否已经是一个桶」——
    /// 预检要模拟 `ensure_archetype` 的建桶决策，否则「先建桶、后迁移失败」会留下空桶。
    pub(crate) fn key_of(&self, id: EntityId) -> SimResult<CompSet> {
        let (arch_idx, _) = self.locate(id)?;
        Ok(self.archetypes[arch_idx as usize].key)
    }

    // ---- step 事务：undo 日志的生命周期（`crate::journal`） ----

    /// 开事务：此后本 step 的每一次值写入都逐格登记旧值。
    ///
    /// 只在 [`crate::sim::Sim::step`] 里调用一次；`debug_assert` 钉死不可嵌套。
    pub(crate) fn begin_tx(&mut self) {
        debug_assert!(self.journal.is_none(), "step 事务不可嵌套");
        self.journal = Some(Journal::default());
    }

    /// 提交：step 成功，日志直接丢弃（不为回滚付任何代价）。
    pub(crate) fn commit_tx(&mut self) {
        self.journal = None;
    }

    /// 回滚：逆序复原本 step 写过的每个 cell，然后丢弃日志。
    pub(crate) fn rollback_tx(&mut self) {
        if let Some(journal) = self.journal.take() {
            journal.rewind(&mut self.archetypes);
        }
    }

    /// 日志条目数（= 本 step 到目前为止写过的 cell 数；不在事务里即 0）。
    #[cfg(test)]
    pub(crate) fn journal_len(&self) -> usize {
        self.journal.as_ref().map_or(0, Journal::len)
    }

    /// 写面拆分：`(桶, 事务日志)`。
    ///
    /// [`crate::query`] 的写路径要同时拿两样东西：写哪一列（`&mut [Archetype]`）与
    /// 把旧值记到哪（`&mut Journal`）。两者是本结构的两个不相交字段，拆开借用即可，
    /// 不需要 `Rc`/`RefCell`/`unsafe`；字段私有，所以只能由本方法拆。
    pub(crate) fn split_write(&mut self) -> (&mut [Archetype], Option<&mut Journal>) {
        (&mut self.archetypes, self.journal.as_mut())
    }

    /// 取某实体某组件的值（只读；`Tag` 无值 ⇒ [`SimError::schema`]）。
    pub fn value_of(&self, id: EntityId, comp: ComponentId) -> SimResult<ValueRef<'_>> {
        let (arch_idx, row) = self.locate(id)?;
        let arch = &self.archetypes[arch_idx as usize];
        let pos = arch.col_pos(comp).ok_or_else(|| {
            SimError::schema(format!(
                "实体 {id} 所在 archetype 没有组件 id {comp}（或该组件是 tag，无列）"
            ))
        })?;
        Ok(arch.cols[pos].value_at(row as usize))
    }

    /// 分配一个实体 id（溢出即 [`SimError::entity`]）。
    fn take_next_id(&mut self) -> SimResult<EntityId> {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| SimError::entity("实体 id 空间耗尽（next_id 溢出）"))?;
        Ok(id)
    }

    /// 取得（必要时创建）某位集对应的桶；**桶下标 = 创建序**。
    fn ensure_archetype(&mut self, key: CompSet) -> SimResult<u32> {
        if let Some(&idx) = self.by_key.get(&key) {
            return Ok(idx);
        }
        let mut col_ids = Vec::new();
        let mut cols = Vec::new();
        for comp in self.registry.ids_of_set(key) {
            if let Some(col) = column_for(self.registry.type_of(comp)?) {
                col_ids.push(comp);
                cols.push(col);
            }
        }
        let idx = self.archetypes.len() as u32;
        self.archetypes.push(Archetype::new(key, col_ids, cols));
        self.by_key.insert(key, idx);
        Ok(idx)
    }

    /// 校验一批载荷：`cols` 是有列组件的载荷，`tags` 是零宽成员；无重复、行数一致。
    ///
    /// 为什么 tag 要单列一个参数而不是「带一个空列」：`tag` 在存储里**没有列**（只进位集），
    /// 给它伪造一个空列会让「列数 = 有值组件数」这条不变量在解码侧消失——而快照与批量帧
    /// 共用同一个列编解码器，那条不变量是它们能互通的前提。
    ///
    /// 公开面：这是「生成/加列」两个写面的载荷合法性单点，沙箱的批量操作预检
    /// （[`Self::validate_insert`]）也走这里，不再另写一份。
    pub fn check_batch(
        &self,
        rows: usize,
        cols: &[(ComponentId, Column)],
        tags: &[ComponentId],
    ) -> SimResult<CompSet> {
        let mut set = 0u128;
        for (comp, col) in cols {
            let bit = 1u128 << comp;
            if set & bit != 0 {
                return Err(SimError::schema(format!("同一帧里组件 id {comp} 出现两次")));
            }
            set |= bit;
            let ty = self.registry.type_of(*comp)?;
            if !ty.has_column() {
                return Err(SimError::schema(format!(
                    "组件 id {comp} 是 {}（tag 没有列），不能带载荷",
                    ty.label()
                )));
            }
            if col.len() != rows {
                return Err(SimError::schema(format!(
                    "组件 id {comp} 的列有 {} 行，帧声明 {rows} 行",
                    col.len()
                )));
            }
        }
        for comp in tags {
            let bit = 1u128 << comp;
            if set & bit != 0 {
                return Err(SimError::schema(format!("同一帧里组件 id {comp} 出现两次")));
            }
            set |= bit;
            if self.registry.type_of(*comp)?.has_column() {
                return Err(SimError::schema(format!(
                    "组件 id {comp} 有列，不能作为零宽成员声明（缺载荷）"
                )));
            }
        }
        Ok(set)
    }

    /// 列式批量生成：一次追加 `rows` 个实体并返回其 id（升序 = 分配序）。
    ///
    /// `cols` 给有列组件的载荷（行数必须 = `rows`），`tags` 给零宽成员（无载荷）。
    pub fn spawn_columns(
        &mut self,
        rows: usize,
        cols: &[(ComponentId, Column)],
        tags: &[ComponentId],
    ) -> SimResult<Vec<EntityId>> {
        let set = self.check_batch(rows, cols, tags)?;
        let idx = self.ensure_archetype(set)?;
        // 先分配 id（不持桶借用），保证「id 分配序 = 行序」。
        let mut ids = Vec::with_capacity(rows);
        for _ in 0..rows {
            ids.push(self.take_next_id()?);
        }
        let arch = &mut self.archetypes[idx as usize];
        // 坑位起点：同一个桶里已有行时，本批行号从 `base` 开始——行号是 `entity_index` 的
        // 唯一真相，写错一次后面所有 `locate` 都指到别人的行（`world.spawn` 连调两次就能踩）。
        let base = arch.ids.len();
        // 桶内列序（升序组件 id）与载荷序无关：建一次位置映射，避免每行线性搜索。
        let mut order = Vec::with_capacity(arch.cols.len());
        for comp in &arch.col_ids {
            let pos = cols
                .iter()
                .position(|(c, _)| c == comp)
                .ok_or_else(|| SimError::schema(format!("帧缺少组件 id {comp} 的载荷")))?;
            order.push(pos);
        }
        for (row, id) in ids.iter().enumerate() {
            for p in 0..arch.cols.len() {
                let comp = arch.col_ids[p];
                let src = &cols[order[p]].1;
                arch.cols[p].push_value(&src.value_at(row), comp)?;
            }
            arch.ids.push(*id);
            self.entity_index.insert(*id, (idx, (base + row) as u32));
        }
        Ok(ids)
    }

    /// 列式批量插入组件（把实体迁到更大的 archetype）。返回写入的值个数。
    ///
    /// 「组件已存在」是错误而不是静默覆盖：`insert` 的语义是**加列**，覆盖是另一件事
    /// （直接写列），把两者混成一个面会让调用方无法表达「我只想加」。
    pub fn insert_columns(
        &mut self,
        ids: &[EntityId],
        cols: &[(ComponentId, Column)],
        tags: &[ComponentId],
    ) -> SimResult<usize> {
        let plan = self.plan_insert(ids, cols, tags)?;
        // ①b 建目标桶：**校验全部通过之后**才落笔。
        //
        // `ensure_archetype` 是写操作（新建桶）；把它留在①的循环里，一个注定被拒的帧
        // （重复 id / 未知 id）也会留下空桶——那是本文件头注不变量 3「拒绍不产生半改的
        // 模拟态」的反例。桶键不变，只是晚一个循环。
        let mut targets: Vec<(EntityId, u32)> = Vec::with_capacity(plan.len());
        for (id, target_key) in plan {
            let target = self.ensure_archetype(target_key)?;
            targets.push((id, target));
        }
        // ② 逐实体迁移（借两个不相交的桶：源桶读 + 目标桶写，零临时分配）。
        //
        // 行号必须在本步**重新查** `entity_index`：① 只给目标桶，不给行号——因为第一次
        // `swap_remove` 之后行号就会错位（末行填洞会把一个尚未迁移的实体搬进空洞，
        // 10⁴ 行批量迁移时旧行号必然越出缩小的源桶）。行号是 `entity_index` 的唯一
        // 真相，本步不另存一份（否则就是第二条行序说法）。
        for (row_idx, (id, target)) in targets.into_iter().enumerate() {
            let (arch_idx, local_row) = self.locate(id)?;
            if arch_idx == target {
                return Err(SimError::state(format!(
                    "insert 的目标桶与源桶相同（实体 {id}）——位集应当严格增长"
                )));
            }
            let (src, dst) = split_pair(&mut self.archetypes, arch_idx, target);
            for p in 0..dst.cols.len() {
                let comp = dst.col_ids[p];
                match src.col_pos(comp) {
                    Some(src_pos) => {
                        dst.cols[p].push_from(&src.cols[src_pos], local_row as usize, comp)?
                    }
                    None => {
                        let pos = cols.iter().position(|(c, _)| *c == comp).ok_or_else(|| {
                            SimError::schema(format!("帧缺少组件 id {comp} 的载荷"))
                        })?;
                        dst.cols[p].push_value(&cols[pos].1.value_at(row_idx), comp)?;
                    }
                }
            }
            let new_row = dst.ids.len() as u32;
            dst.ids.push(id);
            // 源桶删行（末行填洞）→ 被填洞的实体改索引。
            if let Some(moved) = src.swap_remove_row(local_row as usize)? {
                self.entity_index.insert(moved, (arch_idx, local_row));
            }
            self.entity_index.insert(id, (target, new_row));
        }
        Ok(ids.len() * cols.len())
    }

    /// 批量加列的只读预检：算出「实体 → 目标位集」计划，一字不写。
    ///
    /// 批量写面（沙箱 `WorldOp`）在整批落笔前调 [`Self::validate_insert`]，与本函数共用
    /// 同一段校验：先校验后改才是原子的，而校验不能有两份（两份就会分叉）。
    fn plan_insert(
        &self,
        ids: &[EntityId],
        cols: &[(ComponentId, Column)],
        tags: &[ComponentId],
    ) -> SimResult<Vec<(EntityId, CompSet)>> {
        let set = self.check_batch(ids.len(), cols, tags)?;
        // ① 全量校验（先校验后改）：实体存在、组件尚未在场、无重复 id。
        //
        // 重复检测用「排序 + 相邻扫描」而不是 `Vec::contains`（后者是 O(n²)：10⁴ 行约
        // 5·10⁷ 次比较，而行数由调用方的帧决定、没有上界）。但**报错口径不变**：仍在
        // 输入序的循环里、`locate` 之后抛出——「未知 id 先于重复 id」的优先级与此前逐字相同。
        let mut sorted: Vec<EntityId> = ids.to_vec();
        sorted.sort_unstable();
        let mut dups: Vec<EntityId> = sorted
            .windows(2)
            .filter(|pair| pair[0] == pair[1])
            .map(|pair| pair[0])
            .collect();
        dups.dedup();
        // 已见过一次的重复候选（只有真的重复时非空，量级是个位数）。
        let mut repeated: Vec<EntityId> = Vec::new();
        let mut plan: Vec<(EntityId, CompSet)> = Vec::with_capacity(ids.len());
        for id in ids.iter() {
            let (arch_idx, _) = self.locate(*id)?;
            if !dups.is_empty() && dups.binary_search(id).is_ok() {
                if repeated.contains(id) {
                    return Err(SimError::entity(format!("同一帧里实体 id {id} 出现两次")));
                }
                repeated.push(*id);
            }
            let key = self.archetypes[arch_idx as usize].key;
            if key & set != 0 {
                return Err(SimError::schema(format!(
                    "实体 {id} 已有所插入的组件（insert 只加列，不覆盖）"
                )));
            }
            plan.push((*id, key | set));
        }
        Ok(plan)
    }

    /// 批量加列的只读预检（外部写面的整批预检入口）。
    pub fn validate_insert(
        &self,
        ids: &[EntityId],
        cols: &[(ComponentId, Column)],
        tags: &[ComponentId],
    ) -> SimResult<()> {
        self.plan_insert(ids, cols, tags).map(|_| ())
    }

    /// 批量删除实体（`swap_remove` + 索引同步）。返回实际删除个数。
    ///
    /// 重复 id 只删一次（第二次查找落空 = 幂等），不报错——批量帧的幂等性比「发现调用方
    /// 重复了」更有价值（回放时同一帧可能被重放）。
    pub fn despawn_ids(&mut self, ids: &[EntityId]) -> SimResult<usize> {
        self.validate_despawn(ids)?;
        let mut removed = 0;
        for id in ids {
            let Some((arch_idx, row)) = self.entity_index.remove(id) else {
                continue;
            };
            let arch = &mut self.archetypes[arch_idx as usize];
            if let Some(moved) = arch.swap_remove_row(row as usize)? {
                self.entity_index.insert(moved, (arch_idx, row));
            }
            removed += 1;
        }
        Ok(removed)
    }

    /// 批量删除的只读预检：任何一个 id 不存在即整批拒绍（与 [`Self::despawn_ids`] 同一段校验）。
    pub fn validate_despawn(&self, ids: &[EntityId]) -> SimResult<()> {
        for id in ids {
            self.locate(*id)?;
        }
        Ok(())
    }

    /// 摘除组件的只读预检：实体存在、每个组件都在场。
    ///
    /// 与 [`Self::remove_components`] 共用同一段校验：前者是批量写面落笔前的预检，
    /// 后者在写前再调一次（两道门，同一份代码）。
    pub fn validate_remove(&self, id: EntityId, cids: &[ComponentId]) -> SimResult<()> {
        let (arch_idx, _) = self.locate(id)?;
        let key = self.archetypes[arch_idx as usize].key;
        for comp in cids {
            if key & (1u128 << comp) == 0 {
                let name = self.registry.name_of(*comp)?;
                return Err(SimError::schema(format!(
                    "实体 {id} 没有组件 `{name}`，不能移除"
                )));
            }
        }
        Ok(())
    }

    /// 从实体上摘掉若干组件（把行迁到位集缩小的桶里）。返回摘掉的组件数。
    ///
    /// 实现是「克隆桶表上拆件 + 最后一次 `from_parts` 重建派生面」：`O(世界)`——
    /// 「卡在移除一个组件」从来不是热路径，而快照一致性值得这一次克隆。
    pub fn remove_components(&mut self, id: EntityId, cids: &[ComponentId]) -> SimResult<usize> {
        self.validate_remove(id, cids)?;
        let (arch_idx, row) = self.locate(id)?;
        let removed_bits: u128 = cids.iter().fold(0u128, |acc, comp| acc | (1u128 << comp));

        // ① 拆件（克隆上操作：失败时原 store 一字未动）。
        let (registry, mut archetypes, next_id) = self.clone().into_parts();

        // ② 建/找目标桶：位集严格收缩 ⇒ 目标桶键必与源桶不同。
        let (target_key, target_cols) = {
            let src = &archetypes[arch_idx as usize];
            if src.key & !removed_bits == src.key {
                return Err(SimError::state(
                    "remove 的目标桶与源桶相同（位集应当严格收缩）",
                ));
            }
            let mut col_ids: Vec<ComponentId> = Vec::with_capacity(src.col_ids.len());
            let mut cols: Vec<Column> = Vec::with_capacity(src.col_ids.len());
            for (pos, comp) in src.col_ids.iter().enumerate() {
                if removed_bits & (1u128 << comp) != 0 {
                    continue;
                }
                col_ids.push(*comp);
                cols.push(src.cols[pos].empty_like());
            }
            (src.key & !removed_bits, (col_ids, cols))
        };
        let target_idx = match archetypes.iter().position(|arch| arch.key == target_key) {
            Some(idx) => idx,
            None => {
                archetypes.push(Archetype::new(target_key, target_cols.0, target_cols.1));
                archetypes.len() - 1
            }
        };

        // ③ 搬行：目标桶只追加不删，源桶摘尾填洞（索引在④统一重建）。
        {
            let (src, dst) = split_pair(&mut archetypes, arch_idx, target_idx as u32);
            for pos in 0..src.col_ids.len() {
                let comp = src.col_ids[pos];
                if removed_bits & (1u128 << comp) != 0 {
                    continue;
                }
                let dst_pos = dst.col_pos(comp).ok_or_else(|| {
                    SimError::state(format!("目标桶缺少组件 id {comp} 的列（remove 建桶失败）"))
                })?;
                dst.cols[dst_pos].push_from(&src.cols[pos], row as usize, comp)?;
            }
            dst.ids.push(id);
            src.swap_remove_row(row as usize)?;
        }

        // ④ 重建 `by_key` / `entity_index`（两者都是私有派生面，只有这条口）。
        *self = ArchetypeStore::from_parts(registry, archetypes, next_id)?;
        Ok(cids.len())
    }

    /// 本事务已登记写过的格 `(实体, 组件)`（去重升序；事务外为空）。
    ///
    /// 来源是 undo 日志，而不是在各个写点再装一份仪器：日志本来就记了每一格，
    /// 再让 `set_many`/`for_each_mut` 各报一次就是第二份真相（两分会静默）。
    pub fn written_cells(&self) -> Vec<(EntityId, ComponentId)> {
        self.journal
            .as_ref()
            .map(Journal::cells)
            .unwrap_or_default()
    }

    /// 是否在事务里（`Sim::transact` 的嵌套判定）。
    pub(crate) fn in_tx(&self) -> bool {
        self.journal.is_some()
    }

    /// 拆成 (注册表, 桶, next_id)（快照写出用）。
    pub fn into_parts(self) -> (Registry, Vec<Archetype>, EntityId) {
        (self.registry, self.archetypes, self.next_id)
    }

    /// 从三个部分重建（快照读入用）：重建 `by_key` 与 `entity_index` 并做完整性校验。
    pub fn from_parts(
        registry: Registry,
        archetypes: Vec<Archetype>,
        next_id: EntityId,
    ) -> SimResult<Self> {
        let mut store = Self {
            archetypes,
            by_key: BTreeMap::new(),
            entity_index: BTreeMap::new(),
            next_id,
            registry,
            journal: None,
        };
        let mut seen = 0usize;
        for (idx, arch) in store.archetypes.iter().enumerate() {
            if store.by_key.insert(arch.key, idx as u32).is_some() {
                return Err(SimError::frame(format!(
                    "快照里出现了重复的组件位集：{}",
                    arch.key
                )));
            }
            for (row, id) in arch.ids.iter().enumerate() {
                if !arch.cols.iter().all(|col| col.len() == arch.ids.len()) {
                    return Err(SimError::state(format!(
                        "桶 {idx} 的行数与列长不符（{} 行）",
                        arch.ids.len()
                    )));
                }
                if *id == 0 || *id >= store.next_id {
                    return Err(SimError::frame(format!(
                        "快照里的实体 id {id} 不在 [1, {}) 区间",
                        store.next_id
                    )));
                }
                if store
                    .entity_index
                    .insert(*id, (idx as u32, row as u32))
                    .is_some()
                {
                    return Err(SimError::frame(format!("快照里实体 id {id} 出现两次")));
                }
                seen += 1;
            }
        }
        if seen != store.entity_index.len() {
            return Err(SimError::state("实体索引行数与桶行数总和不符"));
        }
        Ok(store)
    }
}

/// 从 `archetypes` 里取两个**不相交**的可变引用（按下标）。
fn split_pair(archetypes: &mut [Archetype], a: u32, b: u32) -> (&mut Archetype, &mut Archetype) {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    let (left, right) = archetypes.split_at_mut(hi as usize);
    let lower = &mut left[lo as usize];
    let upper = &mut right[0];
    if a < b {
        (lower, upper)
    } else {
        (upper, lower)
    }
}
