//! 列式 archetype：一个组件集 = 一个桶，桶内每组件一列（SoA）。
//!
//! 契约：sim-kernel §2「列式 SoA，按 archetype 分桶」+ §2 的「行序与删除语义」段。
//!
//! # 为什么列这么切
//!
//! - `Bytes { off, buf }`：字符串列保持**一块连续 buf**（快照与批量载荷直接 memcpy，
//!   不逐行分配）。`off` 是 `n + 1` 个前缀偏移（`off[i]..off[i + 1]` 是第 i 行），
//!   于是「取一行」是一次切片、`swap_remove` 只需搬 `off` + 那一段字节。
//! - `Bool` 用 `u8` 而不是位打包：memcpy 更快、编码更简单；10⁴ 量级省下的 7/8 字节
//!   没有意义，而位运算会引入第二种「按位取行」的语义要人审。
//! - `Tag` 没有列（零宽组件只进 archetype key）。
//!
//! # 行序
//!
//! 行是 **dense 插入序**：新行永远追加在末尾。删除一律 `swap_remove`（末行填洞）并同步
//! `entity_index`，于是「被填洞的行 = 原末行」是显式语义而不是实现巧合——改掉它即改逐
//! tick hash（回放对行序强耦合，见 `store.rs` 头注）。

use crate::component::{CompSet, ComponentId, EntityId};
use crate::error::{SimError, SimResult};

/// 单列载荷（SoA）。变体与 `AttrType` 一一对应（`Tag` 除外——它没有列）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Column {
    /// `number`：定点整数列。
    I64(Vec<i64>),
    /// `ref`：引用列（`0` = 无引用）。
    U64(Vec<u64>),
    /// `boolean`：0/1 列（值域在写入与 restore 各校验一次）。
    U8(Vec<u8>),
    /// `string`：前缀偏移 + 连续字节。
    Bytes { off: Vec<u32>, buf: Vec<u8> },
}

impl Column {
    /// 空列（`Bytes` 以单元素 `off` 起头：`off[0] = 0`）。
    pub fn empty_like(&self) -> Column {
        match self {
            Column::I64(_) => Column::I64(Vec::new()),
            Column::U64(_) => Column::U64(Vec::new()),
            Column::U8(_) => Column::U8(Vec::new()),
            Column::Bytes { .. } => Column::Bytes {
                off: vec![0],
                buf: Vec::new(),
            },
        }
    }

    /// 行数。
    pub fn len(&self) -> usize {
        match self {
            Column::I64(v) => v.len(),
            Column::U64(v) => v.len(),
            Column::U8(v) => v.len(),
            Column::Bytes { off, .. } => off.len().saturating_sub(1),
        }
    }

    /// 是否空列。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 追加一行的值（类型不符即 [`SimError::schema`]；`U8` 的值域在写入处校验）。
    pub fn push_value(&mut self, value: &ValueRef<'_>, comp: ComponentId) -> SimResult<()> {
        match (self, value) {
            (Column::I64(v), ValueRef::I64(x)) => v.push(*x),
            (Column::U64(v), ValueRef::U64(x)) => v.push(*x),
            (Column::U8(v), ValueRef::U8(x)) => {
                if *x > 1 {
                    return Err(SimError::schema(format!(
                        "组件 id {comp} 是 boolean 列，值必须是 0/1，得到 {x}"
                    )));
                }
                v.push(*x)
            }
            (Column::Bytes { off, buf }, ValueRef::Bytes(bytes)) => {
                buf.extend_from_slice(bytes);
                let end = u32::try_from(buf.len()).map_err(|_| {
                    SimError::schema(format!("组件 id {comp} 的字符串列超过 4 GiB 偏移上限"))
                })?;
                off.push(end);
            }
            (column, value) => {
                return Err(SimError::schema(format!(
                    "组件 id {comp} 的列类型与值不符：列={} 值={}",
                    column.kind_name(),
                    value.kind_name()
                )))
            }
        }
        Ok(())
    }

    /// 把 `src` 的第 `row` 行追加到本列（迁移用；类型必须相同）。
    pub fn push_from(&mut self, src: &Column, row: usize, comp: ComponentId) -> SimResult<()> {
        match (self, src) {
            (Column::I64(dst), Column::I64(src)) => dst.push(src[row]),
            (Column::U64(dst), Column::U64(src)) => dst.push(src[row]),
            (Column::U8(dst), Column::U8(src)) => dst.push(src[row]),
            (
                Column::Bytes { off, buf },
                Column::Bytes {
                    off: src_off,
                    buf: src_buf,
                },
            ) => {
                let (start, end) = (src_off[row] as usize, src_off[row + 1] as usize);
                buf.extend_from_slice(&src_buf[start..end]);
                let end = u32::try_from(buf.len()).map_err(|_| {
                    SimError::schema(format!("组件 id {comp} 的字符串列超过 4 GiB 偏移上限"))
                })?;
                off.push(end);
            }
            _ => {
                return Err(SimError::schema(format!(
                    "组件 id {comp} 的列类型在迁移中不一致"
                )))
            }
        }
        Ok(())
    }

    /// 取第 `row` 行的值（只读视图，供迁移与诊断）。
    pub fn value_at(&self, row: usize) -> ValueRef<'_> {
        match self {
            Column::I64(v) => ValueRef::I64(v[row]),
            Column::U64(v) => ValueRef::U64(v[row]),
            Column::U8(v) => ValueRef::U8(v[row]),
            Column::Bytes { off, buf } => {
                ValueRef::Bytes(&buf[off[row] as usize..off[row + 1] as usize])
            }
        }
    }

    /// `swap_remove`：末行填洞（行数 −1）。
    pub fn swap_remove(&mut self, row: usize) {
        match self {
            Column::I64(v) => {
                v.swap_remove(row);
            }
            Column::U64(v) => {
                v.swap_remove(row);
            }
            Column::U8(v) => {
                v.swap_remove(row);
            }
            Column::Bytes { off, buf } => {
                let (start, end) = (off[row] as usize, off[row + 1] as usize);
                let last = off.len() - 1;
                if row != last - 1 {
                    // 先把末行接到被删行的位置（按字节搬），再剥掉末尾那段。
                    let (last_start, last_end) = (off[last - 1] as usize, off[last] as usize);
                    let tail: Vec<u8> = buf[last_start..last_end].to_vec();
                    buf.splice(start..end, tail);
                    let delta = (last_end - last_start) as isize - (end - start) as isize;
                    for entry in off.iter_mut().skip(row + 1) {
                        *entry = (*entry as isize + delta) as u32;
                    }
                } else {
                    buf.truncate(start);
                }
                off.pop();
            }
        }
    }

    /// 线格式的列名（错误消息用）。
    pub const fn kind_name(&self) -> &'static str {
        match self {
            Column::I64(_) => "i64",
            Column::U64(_) => "u64",
            Column::U8(_) => "u8",
            Column::Bytes { .. } => "bytes",
        }
    }
}

/// 与 [`Column`] 对应的单值视图（批量帧解码与迁移的中间形）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueRef<'a> {
    /// 定点整数。
    I64(i64),
    /// 引用 / 任意 u64。
    U64(u64),
    /// boolean（0/1）。
    U8(u8),
    /// 字符串字节。
    Bytes(&'a [u8]),
}

impl ValueRef<'_> {
    const fn kind_name(&self) -> &'static str {
        match self {
            ValueRef::I64(_) => "i64",
            ValueRef::U64(_) => "u64",
            ValueRef::U8(_) => "u8",
            ValueRef::Bytes(_) => "bytes",
        }
    }
}

/// 一个 archetype 桶：键（组件位集）+ 行 id + 每组件一列。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archetype {
    /// 组件位集（桶键）。
    pub key: CompSet,
    /// 行 id（dense 插入序）。
    pub ids: Vec<EntityId>,
    /// 每组件一列，下标与 [`Archetype::col_ids`] 对齐。
    pub cols: Vec<Column>,
    /// 与 `cols` 对齐的组件内部 id（升序；只用于本进程内定位）。
    pub col_ids: Vec<ComponentId>,
}

impl Archetype {
    /// 空桶。
    pub fn new(key: CompSet, col_ids: Vec<ComponentId>, cols: Vec<Column>) -> Self {
        Self {
            key,
            ids: Vec::new(),
            cols,
            col_ids,
        }
    }

    /// 行数。
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// 是否空桶。
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// 某组件在本桶内的列下标。
    pub fn col_pos(&self, comp: ComponentId) -> Option<usize> {
        self.col_ids.iter().position(|id| *id == comp)
    }

    /// 追加一行（列载荷由调用方按 `col_ids` 序给齐）。
    pub fn push_row(
        &mut self,
        id: EntityId,
        values: &[(ComponentId, ValueRef<'_>)],
    ) -> SimResult<()> {
        if values.len() != self.cols.len() {
            return Err(SimError::state(format!(
                "archetype 行写入的列数不符：期望 {} 得到 {}",
                self.cols.len(),
                values.len()
            )));
        }
        for (pos, column) in self.cols.iter_mut().enumerate() {
            let (comp, value) = values[pos];
            if comp != self.col_ids[pos] {
                return Err(SimError::state(format!(
                    "archetype 行写入的组件序不符：位置 {pos} 期望 id {} 得到 {comp}",
                    self.col_ids[pos]
                )));
            }
            column.push_value(&value, comp)?;
        }
        self.ids.push(id);
        Ok(())
    }

    /// `swap_remove` 一行，返回「谁被搬到了 `row`」（若搬动）：供调用方同步 `entity_index`。
    pub fn swap_remove_row(&mut self, row: usize) -> SimResult<Option<EntityId>> {
        if row >= self.ids.len() {
            return Err(SimError::state(format!(
                "行号越界：{row} ≥ {}",
                self.ids.len()
            )));
        }
        let last = self.ids.len() - 1;
        for column in &mut self.cols {
            column.swap_remove(row);
        }
        self.ids.swap_remove(row);
        Ok(if row == last {
            None
        } else {
            Some(self.ids[row])
        })
    }
}
