//! 宿主批量写面：`WorldOp` 与「先全量校验、后落笔」的**唯一**实现。
//!
//! 为什么这份代码在沙箱 crate 而不是宿主 crate：校验是内核语义（列型、位集、行数、
//! 实体在场），宿主复刻一份就必然分叉——两份校验里更松的那份会成为漏洞。宿主面
//! （`ygg/1` 的 `submit` 帧、WASM、C ABI）只做「协议结构 → [`WorldOp`]」的翻译，
//! 判定全部落在本模块。
//!
//! 两段式（`validate` → `write`）而不是「边写边校验」：
//!
//! 1. **只读预检**在写之前把整批看一遍，任何一条不合格就整批拒绝——一个注定失败的
//!    批量提交不会留下半个世界（也不会留下只属于这条失败帧的空 archetype）；
//! 2. **写阶段**仍在 [`Sim::transact`] 里（undo 日志 + 逐格旧值），所以即使预检没看见
//!    的内部失败也整批回滚。
//!
//! 预检只看**提交前的状态**：同一批里「先 `Insert` 再 `Remove` 刚插入的组件」会被
//! 保守拒绝（`Remove` 在提交前的世界里找不到那个组件）。批量提交的语义是「一次性
//! 宣言目标状态」，想要时序语义就分两批提交——这比让校验器模拟一遍写入历史更安全。

use ygg_sim::{Column, ComponentId, EntityId, Sim, SimError};

use crate::sandbox::StepError;

/// 一条批量写操作（与 Lua `world.*` 写面一一对应）。
///
/// 与 Lua 面的差异只有形状：Lua 一次只写一个实体/一个组件，这里是它的批量形态
/// （`Spawn` 一次多行、`Set` 一次多实体）。`Spawn.rows` 是行数（协议帧的 `n`，
/// Lua 面恒为 1）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorldOp {
    /// 生成 `rows` 个带同样列集合的实体（组件 id 与列型由 `cols` 决定，`tags` 是零宽成员）。
    Spawn {
        rows: u32,
        cols: Vec<(ComponentId, Column)>,
        tags: Vec<ComponentId>,
    },
    /// 删除若干实体（不存在的 id 在预检阶段就整批拒绝）。
    Despawn { ids: Vec<EntityId> },
    /// 给若干实体加列（位集严格增长；组件已在场是错误，不是覆盖）。
    Insert {
        ids: Vec<EntityId>,
        cols: Vec<(ComponentId, Column)>,
        tags: Vec<ComponentId>,
    },
    /// 从若干实体上摘掉若干组件（位集严格收缩）。
    Remove {
        ids: Vec<EntityId>,
        cids: Vec<ComponentId>,
    },
    /// 改写一个组件的一列（`ids.len()` 个值，按 `ids` 顺序）。
    Set {
        ids: Vec<EntityId>,
        col: ComponentId,
        values: Column,
    },
}

/// 一次批量提交的回执。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplyReport {
    /// 实际生效的写次数：值写入按格计，结构操作按受影响实体计。
    pub writes: usize,
    /// 本批新生成的实体 id（按生成顺序；`spawn` 协议帧没有回执行，宿主需要它来回答
    /// 「我刚生成了谁」）。
    pub spawned: Vec<EntityId>,
}

/// 执行一批写操作：**整批全成或整批不动**。
pub fn apply_ops(sim: &mut Sim, ops: &[WorldOp]) -> Result<ApplyReport, StepError> {
    validate(sim, ops)?;
    sim.transact(|sim| write(sim, ops)).map_err(StepError::from)
}

/// 只读预检：按提交前状态逐条校验，任一失败即整批拒绝。
fn validate(sim: &Sim, ops: &[WorldOp]) -> Result<(), StepError> {
    for op in ops {
        match op {
            WorldOp::Spawn { rows, cols, tags } => {
                if *rows == 0 {
                    return Err(SimError::frame("Spawn 的 rows 必须 ≥ 1").into());
                }
                sim.world()
                    .check_batch(*rows as usize, cols, tags)
                    .map(|_| ())?;
            }
            WorldOp::Despawn { ids } => sim.world().validate_despawn(ids)?,
            WorldOp::Insert { ids, cols, tags } => sim.world().validate_insert(ids, cols, tags)?,
            WorldOp::Remove { ids, cids } => {
                for id in ids {
                    sim.world().validate_remove(*id, cids)?;
                }
            }
            WorldOp::Set { ids, col, values } => sim.world().validate_set(*col, ids, values)?,
        }
    }
    Ok(())
}

/// 写阶段（预检已通过；仍在事务里，内部失败整批回滚）。
fn write(sim: &mut Sim, ops: &[WorldOp]) -> Result<ApplyReport, SimError> {
    let mut report = ApplyReport::default();
    for op in ops {
        match op {
            WorldOp::Spawn { rows, cols, tags } => {
                let ids = sim.world_mut().spawn_columns(*rows as usize, cols, tags)?;
                report.writes += ids.len() * cols.len();
                report.spawned.extend(ids);
            }
            WorldOp::Despawn { ids } => {
                report.writes += sim.world_mut().despawn_ids(ids)?;
            }
            WorldOp::Insert { ids, cols, tags } => {
                report.writes += sim.world_mut().insert_columns(ids, cols, tags)?;
            }
            WorldOp::Remove { ids, cids } => {
                for id in ids {
                    report.writes += sim.world_mut().remove_components(*id, cids)?;
                }
            }
            WorldOp::Set { ids, col, values } => {
                report.writes += sim.world_mut().set_many(*col, ids, values)?;
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use ygg_sim::{ComponentDef, ValueRef};

    use super::*;

    fn sim() -> (Sim, ComponentId, ComponentId) {
        let mut sim = Sim::new(7);
        let hp = sim.register("hp", ComponentDef::number()).expect("注册 hp");
        let mark = sim.register("mark", ComponentDef::tag()).expect("注册 tag");
        sim.seal().expect("封盘");
        (sim, hp, mark)
    }

    #[test]
    fn spawn_returns_ids_and_set_writes_one_cell() {
        let (mut sim, hp, mark) = sim();
        let report = apply_ops(
            &mut sim,
            &[WorldOp::Spawn {
                rows: 2,
                cols: vec![(hp, Column::I64(vec![3, 4]))],
                tags: vec![mark],
            }],
        )
        .expect("生成两行");
        assert_eq!(report.spawned.len(), 2);
        assert_eq!(report.writes, 2);
        assert_eq!(sim.world().entity_count(), 2);

        let id = report.spawned[0];
        apply_ops(
            &mut sim,
            &[WorldOp::Set {
                ids: vec![id],
                col: hp,
                values: Column::I64(vec![9]),
            }],
        )
        .expect("改一个格");
        assert_eq!(
            sim.world().value_of(id, hp).expect("读数"),
            ValueRef::I64(9)
        );
        assert_eq!(sim.changed(), &[(id, hp)]);
    }

    #[test]
    fn one_bad_op_rejects_the_whole_batch() {
        let (mut sim, hp, _mark) = sim();
        let before = sim.hash();
        let err = apply_ops(
            &mut sim,
            &[
                WorldOp::Spawn {
                    rows: 1,
                    cols: vec![(hp, Column::I64(vec![5]))],
                    tags: Vec::new(),
                },
                // 未知实体：预检就拒，且整批不落笔。
                WorldOp::Set {
                    ids: vec![999],
                    col: hp,
                    values: Column::I64(vec![1]),
                },
            ],
        )
        .expect_err("未知实体必须整批拒绝");
        assert_eq!(err.code(), "YGG-SIM-ENTITY");
        assert_eq!(sim.world().entity_count(), 0, "拒绝的批次不能留下实体");
        assert_eq!(sim.hash(), before, "拒绝的批次不能改状态");
        assert!(sim.changed().is_empty(), "拒绝的批次不能留回执");
    }

    #[test]
    fn remove_shrinks_the_bitset_and_reports_cells() {
        let (mut sim, hp, mark) = sim();
        let report = apply_ops(
            &mut sim,
            &[WorldOp::Spawn {
                rows: 1,
                cols: vec![(hp, Column::I64(vec![1]))],
                tags: vec![mark],
            }],
        )
        .expect("生成");
        let id = report.spawned[0];
        apply_ops(
            &mut sim,
            &[WorldOp::Remove {
                ids: vec![id],
                cids: vec![mark],
            }],
        )
        .expect("摘 tag");
        let store = sim.world();
        assert!(store.value_of(id, hp).is_ok(), "hp 还在");
        assert!(
            store.value_of(id, mark).is_err(),
            "tag 已摘（零宽组件没有列）"
        );
    }
}
