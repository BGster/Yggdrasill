//! step 事务的 undo 日志：值写入前的旧值快照 + 逆序回放。
//!
//! 契约：sim-kernel §4「tick 边界」+ [`crate::error`] 头注「错误与状态」段。
//!
//! # 为什么是 undo 日志而不是快照
//!
//! 一个 step 里失败的方式只有两种，代价差三个数量级：
//!
//! - **值写入**（`set_many` / `RowSet::set_fixed` / `set_u64`）改的是已有行，逐格可撤销；
//!   日志大小 ≈ 本 step 写过的 cell 数，与「整个世界」无关。
//! - **结构操作**（`defer_spawn` / `defer_despawn` / `defer_insert`）改的是桶、行序与
//!   `entity_index`，撤销要还原整个世界的拓扑——那是 world clone 的代价，本文件不做。
//!   它们因此被挡在 tick 边界之外：`Sim::apply_pending` 先用只读的影子状态把整队预检
//!   一遍，全绿才落笔（[`crate::system::QueuePreview`]）。
//!
//! # 一条日志的布局
//!
//! ```text
//! CellWrite { arch: u32, pos: u32, row: u32, old: CellValue }
//!            桶下标     列下标     行号      写前值（含 Bytes 的行副本）
//! ```
//!
//! 三个下标都是 `u32`（桶/列/行各自的上限远低于 2³²，`store.rs` 的编码面同口径）；
//! `old` 是**写前**值，`Bytes` 变体持有该行的完整字节副本（字符串列原地 `splice` 会把
//! 后面的偏移全部挪动，不留副本就无法复原）。
//!
//! # 回滚顺序
//!
//! 逆序（后写的先复原）。同一格的多次写因此落到**最初**值，而不是中间态；顺序反过来
//! 只能还原成「第一次写之后」的样子。
//!
//! # 不覆盖的写面
//!
//! [`crate::query::RowSet`] 之外的裸列视图（`ArchetypeStore::col_mut` / `ColIterMut`）
//! 直接给 `&mut [T]`，写进去的值无从登记——它们不在系统可见的 API 面上
//! （`SystemCtx` 不暴露 `col_mut`、不暴露 `world_mut`），宿主侧写世界发生在 step 之外，
//! 天然不在事务里。

use crate::archetype::{replace_bytes, Archetype, Column};
use crate::component::{ComponentId, EntityId};

/// 一格旧值（列内表示；`Bytes` 是该行的字节副本）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CellValue {
    I64(i64),
    U64(u64),
    U8(u8),
    Bits(u64),
    Bytes(Vec<u8>),
}

impl CellValue {
    /// 读第 `row` 行的现值（「写前快照」）。
    ///
    /// 越界不报错：调用方都在写前读自己刚校验过的行（`set_many` 先探过
    /// `locate`，`RowSet` 的行来自查询结果本身）。
    pub(crate) fn read(col: &Column, row: usize) -> CellValue {
        match col {
            Column::I64(v) => CellValue::I64(v[row]),
            Column::U64(v) => CellValue::U64(v[row]),
            Column::U8(v) => CellValue::U8(v[row]),
            Column::Bytes { off, buf } => {
                CellValue::Bytes(buf[off[row] as usize..off[row + 1] as usize].to_vec())
            }
            Column::F64Bits(v) => CellValue::Bits(v[row]),
        }
    }

    /// 把快照写回第 `row` 行（回滚路径：不校验、不归一）。
    ///
    /// 快照来自本列本行，类型必然相符、值已经过写入边界归一（位串已 canonical），
    /// 所以这里不走 [`Column::set_value`] 的校验——回滚路径不该有第二种拒绝理由。
    pub(crate) fn write_into(self, col: &mut Column, row: usize) {
        match (col, self) {
            (Column::I64(v), CellValue::I64(x)) => v[row] = x,
            (Column::U64(v), CellValue::U64(x)) => v[row] = x,
            (Column::U8(v), CellValue::U8(x)) => v[row] = x,
            (Column::F64Bits(v), CellValue::Bits(x)) => v[row] = x,
            (Column::Bytes { off, buf }, CellValue::Bytes(bytes)) => {
                replace_bytes(off, buf, row, &bytes)
            }
            (column, value) => debug_assert!(
                false,
                "undo 日志的旧值类型与列不符：列={} 值={value:?}",
                column.kind_name()
            ),
        }
    }
}

/// 一条日志：`(桶下标, 列下标, 行号, 实体, 组件, 写前值)`。
///
/// `eid` / `cid` 与 `arch` / `pos` / `row` 是**同一格的两套坐标**：前者是语义坐标
/// （跨结构操作稳定），后者是存储坐标（`rewind` 复原要的就是它）。两套都记，是因为
/// 它们各自服务一个消费者：`rewind` 逆序回放写回列，`cells` 汇总本步写过的格给
/// 宿主（[`crate::Sim::changed`]）——后者若现算，结构操作搬过行之后就对不上了。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CellWrite {
    pub(crate) arch: u32,
    pub(crate) pos: u32,
    pub(crate) row: u32,
    pub(crate) eid: EntityId,
    pub(crate) cid: ComponentId,
    pub(crate) old: CellValue,
}

/// step 期间的值写入日志（append-only，tick 边界清空）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Journal {
    entries: Vec<CellWrite>,
}

impl Journal {
    /// 登记一次即将发生的写（调用方必须在真正写列之前调用）。
    pub(crate) fn record(
        &mut self,
        arch: u32,
        pos: u32,
        row: u32,
        eid: EntityId,
        cid: ComponentId,
        old: CellValue,
    ) {
        self.entries.push(CellWrite {
            arch,
            pos,
            row,
            eid,
            cid,
            old,
        });
    }

    /// 本事务写过的格 `(实体, 组件)`：按 `(eid, cid)` 升序、同一格只出现一次。
    ///
    /// 这是「本步写了什么」的唯一来源：宿主增量（`ygg/1` 的 `delta`）不另设仪器面，
    /// 而是把 undo 日志投影一遍——同一格写十次仍是一条，顺序由报告契约固定。
    pub(crate) fn cells(&self) -> Vec<(EntityId, ComponentId)> {
        let mut out: Vec<(EntityId, ComponentId)> = self
            .entries
            .iter()
            .map(|write| (write.eid, write.cid))
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// 条目数（诊断与测试用）。
    /// 日志条目数（= 已登记的格数；只有测试需要它，生产路径只关心 `rewind`）。
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// 逆序回放：后写的先复原，然后清空。
    ///
    /// 桶/列/行在事务期间不会消失（结构操作只在 tick 边界落笔，失败时队列整体丢弃），
    /// 所以三个下标必然落在界内；万一不落，是事务外改动了世界（宿主侧 `world_mut`），
    /// 此时 `debug_assert` 报出来、跳过该条，而不是把错误悄悄升级成另一种错误。
    pub(crate) fn rewind(self, archetypes: &mut [Archetype]) {
        for write in self.entries.into_iter().rev() {
            let Some(arch) = archetypes.get_mut(write.arch as usize) else {
                debug_assert!(false, "undo 日志指向不存在的桶：{}", write.arch);
                continue;
            };
            let Some(col) = arch.cols.get_mut(write.pos as usize) else {
                debug_assert!(
                    false,
                    "undo 日志指向不存在的列：桶 {}/列 {}",
                    write.arch, write.pos
                );
                continue;
            };
            let row = write.row as usize;
            if row >= col.len() {
                debug_assert!(false, "undo 日志的行号越界：{} ≥ {}", row, col.len());
                continue;
            }
            write.old.write_into(col, row);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::archetype::Column;
    use crate::component::{ComponentDef, ComponentId, EntityId};
    use crate::store::ArchetypeStore;
    use crate::ValueRef;

    /// 三行两列（`Num` = 10/20/30，`Text` = "a"/"bb"/"ccc"）。
    fn world() -> (ArchetypeStore, ComponentId, ComponentId) {
        let mut store = ArchetypeStore::new();
        let num = store.register("Num", ComponentDef::number()).unwrap();
        let text = store.register("Text", ComponentDef::string()).unwrap();
        store
            .spawn_columns(
                3,
                &[
                    (num, Column::I64(vec![10, 20, 30])),
                    (text, bytes_col(&[b"a", b"bb", b"ccc"])),
                ],
                &[],
            )
            .unwrap();
        (store, num, text)
    }

    fn bytes_col(rows: &[&[u8]]) -> Column {
        let mut off = vec![0u32];
        let mut buf = Vec::new();
        for row in rows {
            buf.extend_from_slice(row);
            off.push(buf.len() as u32);
        }
        Column::Bytes { off, buf }
    }

    fn ids(store: &ArchetypeStore, comp: ComponentId) -> Vec<EntityId> {
        let mut out = Vec::new();
        store.collect_ids(&[comp], &mut out);
        out
    }

    #[test]
    fn rollback_restores_set_many_writes() {
        let (mut store, num, _text) = world();
        let ids = ids(&store, num);
        store.begin_tx();
        store
            .set_many(num, &ids, &Column::I64(vec![100, 200, 300]))
            .unwrap();
        // 三格写 = 三条日志。
        assert_eq!(store.journal_len(), 3);
        store.rollback_tx();
        assert_eq!(store.journal_len(), 0);
        for (i, id) in ids.iter().enumerate() {
            assert_eq!(
                store.value_of(*id, num).unwrap(),
                ValueRef::I64((i as i64 + 1) * 10)
            );
        }
    }

    #[test]
    fn rollback_restores_bytes_columns_and_their_neighbours() {
        let (mut store, _num, text) = world();
        let ids = ids(&store, text);
        store.begin_tx();
        // 把首行换成比原串长很多的串：`buf` 中部 `splice` 会把后面所有 `off` 挪位，
        // 所以“回滚只修好被写的那一格、相邻行错位”这种半修复必须被本用例抓住。
        store
            .set_many(text, &ids, &bytes_col(&[b"zzzzzzzzzz", b"qq", b"r"]))
            .unwrap();
        store.rollback_tx();
        assert_eq!(store.value_of(ids[0], text).unwrap(), ValueRef::Bytes(b"a"));
        assert_eq!(
            store.value_of(ids[1], text).unwrap(),
            ValueRef::Bytes(b"bb")
        );
        assert_eq!(
            store.value_of(ids[2], text).unwrap(),
            ValueRef::Bytes(b"ccc")
        );
    }

    #[test]
    fn repeated_writes_land_on_the_original_value() {
        let (mut store, num, _text) = world();
        let ids = ids(&store, num);
        store.begin_tx();
        store
            .set_many(num, &ids, &Column::I64(vec![100, 200, 300]))
            .unwrap();
        store
            .set_many(num, &ids, &Column::I64(vec![7, 8, 9]))
            .unwrap();
        assert_eq!(store.journal_len(), 6);
        store.rollback_tx();
        // 逆序回放：同一格两次写后回到 10/20/30，而不是 100/200/300。
        for (i, id) in ids.iter().enumerate() {
            assert_eq!(
                store.value_of(*id, num).unwrap(),
                ValueRef::I64((i as i64 + 1) * 10)
            );
        }
    }

    #[test]
    fn writes_outside_a_transaction_are_not_journalled() {
        let (mut store, num, _text) = world();
        let ids = ids(&store, num);
        store
            .set_many(num, &ids, &Column::I64(vec![100, 200, 300]))
            .unwrap();
        assert_eq!(store.journal_len(), 0);
        store.rollback_tx();
        // 没开事务 ⇒ 没有日志可回放，写入保持。
        assert_eq!(store.value_of(ids[0], num).unwrap(), ValueRef::I64(100));
    }
}
