//! 列式查询面：`RowSet` + `for_each(_mut)` / `collect_ids` / `col(_mut)` + 批量读写两面。
//!
//! 契约：`artifacts/w2-world-api-design.md` §1（查询面）、§3（批量面）；实施序见
//! `artifacts/w2-m1-world-plan.md` §3 步 1–2。本文件是这些签名的唯一实现点。
//!
//! # 为什么列位置按桶解析一次
//!
//! [`Archetype::col_pos`]（`archetype.rs:262`）是线性搜索；旧面 `value_of`（`store.rs:118`）
//! 每实体调一次，于是「一次遍历」退化成人均 O(cols)。本模块把 `set` 解析成**桶内位置每桶
//! 一次**（`pos: [u32; MAX_QUERY]`），行循环里只剩下标寻址——这是 O(cols) → O(1) 的那一步。
//!
//! # 遍历序是契约，不是实现细节
//!
//! 桶下标升序（`archetypes: Vec`，创建序）→ 行号升序（dense 插入序，`store.rs` 头注 1）。
//! 两种创建交错若给出不同的**最终**桶/行布局，遍历序就不同；所以「序不泄漏」要证的是
//! **hash 与桶下标无关**（桶序由快照的规范序抹平，`snapshot.rs` 的 archetype 段），
//! 而不是「遍历序与创建序无关」——后者与本节第一句直接矛盾。
//!
//! # 为什么 `RowSet::cols` 是 [`ColsRef`] 而不是 `&mut [Column]`
//!
//! 设计稿的 `cols: &'a mut [Column]` 只够 `for_each_mut`；`for_each(&self)` 拿不到 `&mut`。
//! 无 `unsafe` 的前提下只有两条路：两份访问器代码（两处迟早漂移），或一份带「可变性」
//! 的引用（本文件的 [`ColsRef`]）。选后者：访问器只有一个实现点，`set_fixed` 对共享引用
//! 返回 schema 错——「只读遍历里写」在类型上就不成立，不靠约定。
//!
//! # 遍历中改结构不可能
//!
//! 闭包拿到的是 [`RowSet`]：它不持有 `&mut ArchetypeStore`，除 `ids`/`cols` 之外没有任何
//! 入口。结构变更唯一的门仍是 `SystemCtx::defer_*` → tick 边界（`apply_pending`）。这是
//! 设计稿要求「删 `store_mut()`」那条不变量的类型级表述。

use std::mem::discriminant;
use std::slice;

use crate::archetype::{Archetype, Column, ValueRef};
use crate::component::{CompSet, ComponentId, EntityId};
use crate::error::{SimError, SimResult};
use crate::store::ArchetypeStore;

/// 一次查询的组件数上界（= [`CompSet`] 的位宽：位集装不下的集合根本不存在）。
pub const MAX_QUERY: usize = 128;

/// 「本桶没有这一列」的位置哨兵（`u32::MAX` 不可能是合法列下标）。
const NO_COL: u32 = u32::MAX;

// ---------------------------------------------------------------------------
// 列型选择器
// ---------------------------------------------------------------------------

mod sealed {
    pub trait Sealed {}
    impl Sealed for i64 {}
    impl Sealed for u64 {}
    impl Sealed for u8 {}
    impl Sealed for super::Bits {}
}

/// [`ArchetypeStore::col`] / [`ArchetypeStore::col_mut`] 的列型选择器。
///
/// 只对**类型匹配**的桶产出值；类型不符或组件不在桶里 ⇒ 该桶零产出。签名没有 `Result`，
/// 所以这里不做错误上报（要报错的路径是 [`ArchetypeStore::get_many`] / `for_each_mut`）。
pub trait ColumnKind: sealed::Sealed + Copy {
    /// 行值的形态（标量按值产出；字符串列不走本 trait，见 [`RowSet::bytes`]）。
    type Item: Copy;

    /// 本型在某列上的只读切片（变体不符 ⇒ `None`）。
    fn slice(col: &Column) -> Option<&[Self::Item]>;

    /// 本型在某列上的可写切片（变体不符 ⇒ `None`）。
    fn slice_mut(col: &mut Column) -> Option<&mut [Self::Item]>;
}

impl ColumnKind for i64 {
    type Item = i64;

    fn slice(col: &Column) -> Option<&[i64]> {
        match col {
            Column::I64(values) => Some(values),
            _ => None,
        }
    }

    fn slice_mut(col: &mut Column) -> Option<&mut [i64]> {
        match col {
            Column::I64(values) => Some(values),
            _ => None,
        }
    }
}

impl ColumnKind for u64 {
    type Item = u64;

    fn slice(col: &Column) -> Option<&[u64]> {
        match col {
            Column::U64(values) => Some(values),
            _ => None,
        }
    }

    fn slice_mut(col: &mut Column) -> Option<&mut [u64]> {
        match col {
            Column::U64(values) => Some(values),
            _ => None,
        }
    }
}

impl ColumnKind for u8 {
    type Item = u8;

    fn slice(col: &Column) -> Option<&[u8]> {
        match col {
            Column::U8(values) => Some(values),
            _ => None,
        }
    }

    fn slice_mut(col: &mut Column) -> Option<&mut [u8]> {
        match col {
            Column::U8(values) => Some(values),
            _ => None,
        }
    }
}

/// 位串列（`Column::F64Bits`）的选择器：行值是**原始 `u64` 位串**（仅搬运，不算术）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Bits;

impl ColumnKind for Bits {
    type Item = u64;

    fn slice(col: &Column) -> Option<&[u64]> {
        match col {
            Column::F64Bits(values) => Some(values),
            _ => None,
        }
    }

    fn slice_mut(col: &mut Column) -> Option<&mut [u64]> {
        match col {
            Column::F64Bits(values) => Some(values),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// 行视图
// ---------------------------------------------------------------------------

/// 一行在本次查询里的可变性：`for_each` 给共享、`for_each_mut` 给独占。
enum ColsRef<'a> {
    Shared(&'a [Column]),
    Exclusive(&'a mut [Column]),
}

impl ColsRef<'_> {
    fn get(&self, pos: usize) -> &Column {
        match self {
            ColsRef::Shared(cols) => &cols[pos],
            ColsRef::Exclusive(cols) => &cols[pos],
        }
    }

    fn get_mut(&mut self, pos: usize) -> Option<&mut Column> {
        match self {
            ColsRef::Shared(_) => None,
            ColsRef::Exclusive(cols) => Some(&mut cols[pos]),
        }
    }
}

/// 一行 + 本次查询已解析的列位置（一个桶一个实例，行号在桶内推进）。
pub struct RowSet<'a> {
    ids: &'a [EntityId],
    cols: ColsRef<'a>,
    /// `set[k]` 在本桶的列位置；`NO_COL` = 该组件在本桶没有列（tag，或列型不符）。
    pos: [u32; MAX_QUERY],
    /// `set[k]` 本身（错误消息与写回需要组件 id）。
    comps: [ComponentId; MAX_QUERY],
    /// `set` 的有效长度。
    n: u8,
    row: usize,
}

impl RowSet<'_> {
    /// 当前行指向的实体 id。
    pub fn id(&self) -> EntityId {
        self.ids[self.row]
    }

    /// 当前行号（桶内 dense 序）。
    pub fn row(&self) -> usize {
        self.row
    }

    /// 本次查询的列数（= `set.len()`）。
    pub fn len(&self) -> usize {
        self.n as usize
    }

    /// 是否零列查询（`for_each(&[])` 不会产生行，故停在构造期以外的分支里）。
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// `set[k]` 在本桶是否有**列**（tag 没有列 ⇒ `false`，设计稿 §1 的约定）。
    pub fn present(&self, k: u8) -> bool {
        self.slot(k).is_some_and(|slot| self.pos[slot] != NO_COL)
    }

    /// `set[k]` 的定点整数值（无列或列型不是 `number` ⇒ `None`）。
    pub fn fixed(&self, k: u8) -> Option<i64> {
        match self.column(k)? {
            Column::I64(values) => Some(values[self.row]),
            _ => None,
        }
    }

    /// `set[k]` 的 `u64` 值（`ref` 列与位串列；其余 ⇒ `None`）。
    pub fn u64v(&self, k: u8) -> Option<u64> {
        match self.column(k)? {
            Column::U64(values) | Column::F64Bits(values) => Some(values[self.row]),
            _ => None,
        }
    }

    /// `set[k]` 的字符串字节（`string` 列；其余 ⇒ `None`）。
    pub fn bytes(&self, k: u8) -> Option<&[u8]> {
        match self.column(k)? {
            Column::Bytes { off, buf } => {
                Some(&buf[off[self.row] as usize..off[self.row + 1] as usize])
            }
            _ => None,
        }
    }

    /// 写 `set[k]` 的定点整数值（只改值，**不改行序**）。
    pub fn set_fixed(&mut self, k: u8, value: i64) -> SimResult<()> {
        let slot = self.slot(k).ok_or_else(|| {
            SimError::schema(format!("列下标 {k} 越界：本次查询只有 {} 列", self.n))
        })?;
        let pos = self.pos[slot];
        if pos == NO_COL {
            return Err(SimError::schema(format!(
                "组件 id {} 在本 archetype 没有列（tag），不能写入",
                self.comps[slot]
            )));
        }
        let (row, comp) = (self.row, self.comps[slot]);
        match self.cols.get_mut(pos as usize) {
            Some(col) => col.set_value(row, &ValueRef::I64(value), comp),
            None => Err(SimError::schema(
                "只读遍历（for_each）不能写入：结构外写入请走 for_each_mut",
            )),
        }
    }

    /// 写 `set[k]` 的 `u64` 值（`ref` 列与位串列；其余列型 ⇒ schema 错）。
    ///
    /// 位串列写回走 [`Column::set_value`] 的写入归一（±0→+0、NaN→NAN_BITS），
    /// 于是「读得到位串、写不回去」不会成为一个洞：`u64v` 与 `set_u64` 对称。
    pub fn set_u64(&mut self, k: u8, value: u64) -> SimResult<()> {
        let slot = self.slot(k).ok_or_else(|| {
            SimError::schema(format!("列下标 {k} 越界：本次查询只有 {} 列", self.n))
        })?;
        let pos = self.pos[slot];
        if pos == NO_COL {
            return Err(SimError::schema(format!(
                "组件 id {} 在本 archetype 没有列（tag），不能写入",
                self.comps[slot]
            )));
        }
        let (row, comp) = (self.row, self.comps[slot]);
        let is_bits = matches!(self.cols.get(pos as usize), Column::F64Bits(_));
        let value = if is_bits {
            ValueRef::Bits(value)
        } else {
            ValueRef::U64(value)
        };
        match self.cols.get_mut(pos as usize) {
            Some(col) => col.set_value(row, &value, comp),
            None => Err(SimError::schema(
                "只读遍历（for_each）不能写入：结构外写入请走 for_each_mut",
            )),
        }
    }

    /// `k` → 槽位（越界 ⇒ `None`）。
    fn slot(&self, k: u8) -> Option<usize> {
        let slot = k as usize;
        (slot < self.n as usize).then_some(slot)
    }

    /// `k` → 列出视图（越界 / 无列 ⇒ `None`）。
    fn column(&self, k: u8) -> Option<&Column> {
        let slot = self.slot(k)?;
        let pos = self.pos[slot];
        (pos != NO_COL).then(|| self.cols.get(pos as usize))
    }
}

// ---------------------------------------------------------------------------
// 整列迭代器
// ---------------------------------------------------------------------------

/// [`ArchetypeStore::col`] 的产出：按「桶下标升序 → 行号升序」逐行给值。
pub struct ColIter<'a, T: ColumnKind> {
    buckets: slice::Iter<'a, Archetype>,
    comp: ComponentId,
    inner: Option<slice::Iter<'a, T::Item>>,
}

impl<'a, T: ColumnKind> Iterator for ColIter<'a, T> {
    type Item = T::Item;

    fn next(&mut self) -> Option<T::Item> {
        loop {
            if let Some(value) = self.inner.as_mut().and_then(|it| it.next()) {
                return Some(*value);
            }
            self.inner = None;
            let arch = self.buckets.next()?;
            if let Some(pos) = arch.col_pos(self.comp) {
                if let Some(values) = T::slice(&arch.cols[pos]) {
                    self.inner = Some(values.iter());
                }
            }
        }
    }
}

/// [`ArchetypeStore::col_mut`] 的产出：逐行给 `&mut` 值（序同 [`ColIter`]）。
pub struct ColIterMut<'a, T: ColumnKind> {
    buckets: slice::IterMut<'a, Archetype>,
    comp: ComponentId,
    inner: Option<slice::IterMut<'a, T::Item>>,
}

impl<'a, T: ColumnKind> Iterator for ColIterMut<'a, T> {
    type Item = &'a mut T::Item;

    fn next(&mut self) -> Option<&'a mut T::Item> {
        loop {
            if let Some(value) = self.inner.as_mut().and_then(|it| it.next()) {
                return Some(value);
            }
            self.inner = None;
            let arch = self.buckets.next()?;
            if let Some(pos) = arch.col_pos(self.comp) {
                if let Some(values) = T::slice_mut(&mut arch.cols[pos]) {
                    self.inner = Some(values.iter_mut());
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ArchetypeStore 的查询面
// ---------------------------------------------------------------------------

impl ArchetypeStore {
    /// `set` → 位集，并做完整校验（未知组件 / 重复组件 / 超位宽 ⇒ schema 错）。
    ///
    /// 空集合法但**匹配 0 行**（见 `for_each_mut` 的文档）：不做「空集 = 全世界」的解释，
    /// 那是把「忘了声明组件」变成一次全表扫描。
    fn query_plan(&self, set: &[ComponentId]) -> SimResult<CompSet> {
        if set.len() > MAX_QUERY {
            return Err(SimError::schema(format!(
                "一次查询的组件数 {} 超过位集上限 {MAX_QUERY}",
                set.len()
            )));
        }
        let mut bits: CompSet = 0;
        for &comp in set {
            self.registry().type_of(comp)?;
            let bit = 1u128 << comp;
            if bits & bit != 0 {
                return Err(SimError::schema(format!("查询里组件 id {comp} 出现两次")));
            }
            bits |= bit;
        }
        Ok(bits)
    }

    /// 在桶 `arch` 上解析 `set` 的列位置（每桶一次，行循环里不再搜索）。
    fn resolve_positions(
        arch: &Archetype,
        set: &[ComponentId],
        pos: &mut [u32; MAX_QUERY],
        comps: &mut [ComponentId; MAX_QUERY],
    ) {
        for (k, &comp) in set.iter().enumerate() {
            pos[k] = arch.col_pos(comp).map_or(NO_COL, |p| p as u32);
            comps[k] = comp;
        }
    }

    /// 对每个匹配行调用 `f`（可写），返回访问的行数。
    ///
    /// - 匹配条件：桶的组件位集 ⊇ `set`；遍历序 = 桶下标升序 → 行号升序。
    /// - `set` 为空 ⇒ 0 行（fail-closed：空集不等于「遍地都是」）。
    /// - 未知/重复组件 ⇒ schema 错；闭包返回错 ⇒ 立刻中止并透传。
    pub fn for_each_mut<F>(&mut self, set: &[ComponentId], mut f: F) -> SimResult<u64>
    where
        F: FnMut(&mut RowSet<'_>) -> SimResult<()>,
    {
        let bits = self.query_plan(set)?;
        if bits == 0 {
            return Ok(0);
        }
        let mut visited = 0u64;
        for arch in self.archetypes_mut() {
            if arch.key & bits != bits || arch.ids.is_empty() {
                continue;
            }
            let mut pos = [NO_COL; MAX_QUERY];
            let mut comps = [0u32; MAX_QUERY];
            Self::resolve_positions(arch, set, &mut pos, &mut comps);

            // 字段级拆分（`&ids` + `&mut cols`）：两个不相交借用，无 unsafe。
            let Archetype { ids, cols, .. } = arch;
            let mut rs = RowSet {
                ids: ids.as_slice(),
                cols: ColsRef::Exclusive(cols.as_mut_slice()),
                pos,
                comps,
                n: set.len() as u8,
                row: 0,
            };
            for row in 0..rs.ids.len() {
                rs.row = row;
                f(&mut rs)?;
                visited += 1;
            }
        }
        Ok(visited)
    }

    /// 对每个匹配行调用 `f`（只读），返回访问的行数。语义同 [`Self::for_each_mut`]；
    /// 本面没有 `Result`，未知组件按「匹配 0 行」处理（要报错用 `for_each_mut`）。
    pub fn for_each<F>(&self, set: &[ComponentId], mut f: F) -> u64
    where
        F: FnMut(&RowSet<'_>),
    {
        let Ok(bits) = self.query_plan(set) else {
            return 0;
        };
        if bits == 0 {
            return 0;
        }
        let mut visited = 0u64;
        for arch in self.archetypes() {
            if arch.key & bits != bits || arch.ids.is_empty() {
                continue;
            }
            let mut pos = [NO_COL; MAX_QUERY];
            let mut comps = [0u32; MAX_QUERY];
            Self::resolve_positions(arch, set, &mut pos, &mut comps);
            let mut rs = RowSet {
                ids: arch.ids.as_slice(),
                cols: ColsRef::Shared(arch.cols.as_slice()),
                pos,
                comps,
                n: set.len() as u8,
                row: 0,
            };
            for row in 0..rs.ids.len() {
                rs.row = row;
                f(&rs);
                visited += 1;
            }
        }
        visited
    }

    /// 收集匹配行的 id（序同 `for_each`；追加到 `out`，不清空）。
    pub fn collect_ids(&self, set: &[ComponentId], out: &mut Vec<EntityId>) {
        let Ok(bits) = self.query_plan(set) else {
            return;
        };
        if bits == 0 {
            return;
        }
        for arch in self.archetypes() {
            if arch.key & bits == bits {
                out.extend_from_slice(&arch.ids);
            }
        }
    }

    /// 整列只读迭代（单组件；按桶下标升序 → 行号升序给值）。
    ///
    /// 类型不符的桶零产出；组件是 tag 或从未注册 ⇒ 空迭代器（签名没有 `Result`）。
    pub fn col<T: ColumnKind>(&self, comp: ComponentId) -> ColIter<'_, T> {
        ColIter {
            buckets: self.archetypes().iter(),
            comp,
            inner: None,
        }
    }

    /// 整列可写迭代（序同 [`Self::col`]）。
    pub fn col_mut<T: ColumnKind>(&mut self, comp: ComponentId) -> ColIterMut<'_, T> {
        ColIterMut {
            buckets: self.archetypes_mut().iter_mut(),
            comp,
            inner: None,
        }
    }

    /// 批量写：`values` 的第 `i` 行写到 `ids[i]`（**行序不变**，只改值）。
    ///
    /// 先全量校验（id 可定位 / 桶内有该列 / 列型与载荷一致）再落笔：拒绍不留半改。
    pub fn set_many(
        &mut self,
        comp: ComponentId,
        ids: &[EntityId],
        values: &Column,
    ) -> SimResult<usize> {
        if values.len() != ids.len() {
            return Err(SimError::schema(format!(
                "组件 id {comp} 的载荷有 {} 行，id 表有 {} 个",
                values.len(),
                ids.len()
            )));
        }
        let ty = self.registry().type_of(comp)?;
        if !ty.has_column() {
            return Err(SimError::schema(format!(
                "组件 id {comp} 是 tag（没有列），不能批量写"
            )));
        }
        let mut plan: Vec<(u32, u32)> = Vec::with_capacity(ids.len());
        for id in ids {
            let (arch_idx, row) = self.locate(*id)?;
            let arch = &self.archetypes()[arch_idx as usize];
            let pos = arch.col_pos(comp).ok_or_else(|| {
                SimError::schema(format!("实体 {id} 所在 archetype 没有组件 id {comp} 的列"))
            })?;
            if discriminant(&arch.cols[pos]) != discriminant(values) {
                return Err(SimError::schema(format!(
                    "组件 id {comp} 的列型 {} 与载荷 {} 不符",
                    arch.cols[pos].kind_name(),
                    values.kind_name()
                )));
            }
            plan.push((arch_idx, row));
        }
        for (i, (arch_idx, row)) in plan.into_iter().enumerate() {
            let arch = &mut self.archetypes_mut()[arch_idx as usize];
            let pos = arch.col_pos(comp).ok_or_else(|| {
                SimError::schema(format!(
                    "实体 {} 所在 archetype 没有组件 id {comp} 的列",
                    ids[i]
                ))
            })?;
            arch.cols[pos].set_value(row as usize, &values.value_at(i), comp)?;
        }
        Ok(ids.len())
    }

    /// 批量读：按 `ids` 的顺序把某组件值追加到 `out`（`out` 必须是空列，追加语义）。
    ///
    /// 先全量校验再写 `out`：拒绍时 `out` 保持原样（不被写一半）。
    pub fn get_many(
        &self,
        comp: ComponentId,
        ids: &[EntityId],
        out: &mut Column,
    ) -> SimResult<usize> {
        let ty = self.registry().type_of(comp)?;
        if !ty.has_column() {
            return Err(SimError::schema(format!(
                "组件 id {comp} 是 tag（没有列），不能批量读"
            )));
        }
        if !out.is_empty() {
            return Err(SimError::schema(format!(
                "get_many 的输出列必须是空列（追加语义），当前 {} 行",
                out.len()
            )));
        }
        let mut plan: Vec<(u32, u32)> = Vec::with_capacity(ids.len());
        for (i, id) in ids.iter().enumerate() {
            let (arch_idx, row) = self.locate(*id)?;
            let arch = &self.archetypes()[arch_idx as usize];
            let pos = arch.col_pos(comp).ok_or_else(|| {
                SimError::schema(format!("实体 {id} 所在 archetype 没有组件 id {comp} 的列"))
            })?;
            // 列型由注册表按组件决定，故查一次即可代表全部行。
            if i == 0 && discriminant(&arch.cols[pos]) != discriminant(out) {
                return Err(SimError::schema(format!(
                    "组件 id {comp} 的列型 {} 与输出列 {} 不符",
                    arch.cols[pos].kind_name(),
                    out.kind_name()
                )));
            }
            plan.push((arch_idx, row));
        }
        for (arch_idx, row) in plan {
            let arch = &self.archetypes()[arch_idx as usize];
            let pos = arch
                .col_pos(comp)
                .ok_or_else(|| SimError::schema(format!("archetype 没有组件 id {comp} 的列")))?;
            out.push_value(&arch.cols[pos].value_at(row as usize), comp)?;
        }
        Ok(ids.len())
    }
}
