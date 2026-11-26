//! step 事务（M2-B）：**失败的 tick 什么都不改**。
//!
//! # 契约
//!
//! `Sim::step` 是模拟态唯一的可失败写面，所以它是唯一的事务边界。返回 `Err` 时：
//!
//! - `tick` 不动（第 ④ 步没走到）；
//! - 世界不动（值由 undo 日志逐格复原，结构由队列预检挡在落笔之前）；
//! - `rng` 流不前进（流状态退回开场拷贝）；
//! - 输入日志不多一条（长度截回开场值）。
//!
//! 「不动」的口径是**逐字节**：`hash()` = 快照字节的 fnv，于是 `hash` 相等 =
//! 世界 + `tick` + `resources` + `rng` 全部相等。本文件每个用例都以 hash 对账，
//! 再加一条具体面（值 / 实体数 / 下一次随机数）的断言，避免「hash 相等但其实
//! 两边都没动」这种空断言。
//!
//! # 为什么失败要在中途制造
//!
//! 只在「系统一开头就返回错误」时测，会把「系统已经写过几格」的那一半漏掉——
//! 而那一半才是 undo 日志存在的理由。所以本文件的失败系统都排在**写过东西的
//! 系统之后**（`Pre` 写、`Rules` 报错），并且失败前先 `rng32` 一次（否则随机流的
//! 回滚永远走不到）。

use std::cell::Cell;
use std::rc::Rc;

use ygg_sim::archetype::Column;
use ygg_sim::component::{ComponentDef, ComponentId, EntityId};
use ygg_sim::error::{codes, SimError};
use ygg_sim::fixed::ONE;
use ygg_sim::sim::Sim;
use ygg_sim::system::{Stage, SystemCtx, SystemDecl};
use ygg_sim::ValueRef;

/// 一个可装载的测试世界：`Pos` / `Hp` 两列 + 一个可开关的「本 tick 必失败」系统。
struct World {
    sim: Sim,
    pos: ComponentId,
    armed: Rc<Cell<bool>>,
}

/// 4 行实体；系统先取随机数、再（可选地）失败。
///
/// 取数在失败判断**之前**是刻意的：这样「失败前随机流已经前进过」是必然事件。
/// 回滚若不覆盖 `rng`，下一次成功 step 的 hash 就会与从未失败过的世界分叉。
fn world(armed: bool) -> World {
    let mut sim = Sim::new(20_261_126);
    let pos = sim.register("Pos", ComponentDef::number()).unwrap();
    let hp = sim.register("Hp", ComponentDef::number()).unwrap();
    spawn_rows(&mut sim, &[(pos, 1), (hp, 10)], 4);
    let flag = Rc::new(Cell::new(armed));
    let captured = Rc::clone(&flag);
    sim.add_system(
        SystemDecl::new("writer").writes(&[pos]).reads(&[hp]),
        Stage::Rules,
        move |ctx: &mut SystemCtx<'_>| {
            let roll = ctx.rng32("roll");
            if captured.get() {
                return Err(SimError::state("用例故意失败"));
            }
            ctx.query_mut(&[pos], |row| row.set_fixed(0, i64::from(roll)))?;
            Ok(())
        },
    )
    .unwrap();
    sim.seal().unwrap();
    World {
        sim,
        pos,
        armed: flag,
    }
}

/// 生成 `rows` 行：`(组件, 每行初值)`。
fn spawn_rows(sim: &mut Sim, cols: &[(ComponentId, i64)], rows: usize) {
    let payload: Vec<(ComponentId, Column)> = cols
        .iter()
        .map(|(comp, value)| (*comp, Column::I64(vec![*value; rows])))
        .collect();
    sim.world_mut()
        .spawn_columns(rows, &payload, &[])
        .expect("生成必须成功");
}

/// 某列全部行的定点值（行序 = dense 插入序）。
fn numbers(sim: &Sim, comp: ComponentId) -> Vec<i64> {
    let mut ids: Vec<EntityId> = Vec::new();
    sim.world().collect_ids(&[comp], &mut ids);
    ids.iter()
        .map(
            |id| match sim.world().value_of(*id, comp).expect("列必须存在") {
                ValueRef::I64(value) => value,
                other => panic!("Pos/Hp 是 number 列，不该读到 {other:?}"),
            },
        )
        .collect()
}

/// ① 系统写过三列之后另一系统报错 ⇒ hash / tick / 实体数 / 每格值全部回到调用前，
/// 且重复 100 次结果不变（undo 日志被清空，不会越回滚越多）。
#[test]
fn failing_system_leaves_hash_tick_and_values_untouched() {
    let mut sim = Sim::new(31);
    let pos = sim.register("Pos", ComponentDef::number()).unwrap();
    let hp = sim.register("Hp", ComponentDef::number()).unwrap();
    let flag = sim.register("Flag", ComponentDef::number()).unwrap();
    spawn_rows(&mut sim, &[(pos, 1), (hp, 10), (flag, 100)], 3);
    sim.add_system(
        SystemDecl::new("mutator").writes(&[pos, hp, flag]),
        Stage::Pre,
        move |ctx: &mut SystemCtx<'_>| {
            ctx.query_mut(&[pos, hp, flag], |row| {
                row.set_fixed(0, 99)?;
                row.set_fixed(1, 88)?;
                row.set_fixed(2, 77)
            })?;
            // 本 tick 还没完：随机流也动一格。
            ctx.rng32("noise");
            Ok(())
        },
    )
    .unwrap();
    sim.add_system(
        SystemDecl::new("boom"),
        Stage::Rules,
        |_ctx: &mut SystemCtx<'_>| Err(SimError::state("用例故意失败")),
    )
    .unwrap();
    sim.seal().unwrap();

    let before_hash = sim.hash();
    let before_values = (numbers(&sim, pos), numbers(&sim, hp), numbers(&sim, flag));
    let before_entities = sim.world().entity_count();

    for attempt in 0..100 {
        let err = sim.step(ONE).expect_err("系统报错必须让 step 失败");
        assert_eq!(err.code(), codes::STATE, "错误码必须原样透传：{err}");
        assert_eq!(sim.tick(), 0, "第 {attempt} 次失败后 tick 不得前进");
        assert_eq!(
            sim.hash(),
            before_hash,
            "第 {attempt} 次失败后 hash 必须逐字节不变"
        );
        assert_eq!(
            (numbers(&sim, pos), numbers(&sim, hp), numbers(&sim, flag)),
            before_values,
            "第 {attempt} 次失败后每格值必须回到初值"
        );
        assert_eq!(
            sim.world().entity_count(),
            before_entities,
            "第 {attempt} 次失败不得动实体数"
        );
    }
    // 事务只包 step：失败回滚不会把世界变成只读（宿主仍可直接写世界）。
    spawn_rows(&mut sim, &[(pos, 0), (hp, 0), (flag, 0)], 2);
    assert_eq!(
        sim.world().entity_count(),
        before_entities + 2,
        "事务外的结构写仍应生效"
    );
}

/// ② `defer_spawn` 已入队、随后系统报错 ⇒ 队列整队丢弃，一个实体也不生成，
/// 连 `next_id` 都不前进（重复 100 次仍 0）。
#[test]
fn deferred_spawn_is_dropped_when_the_tick_fails() {
    let mut sim = Sim::new(32);
    let pos = sim.register("Pos", ComponentDef::number()).unwrap();
    sim.add_system(
        SystemDecl::new("spawner"),
        Stage::Pre,
        move |ctx: &mut SystemCtx<'_>| {
            ctx.defer_spawn(5, vec![(pos, Column::I64(vec![1; 5]))], Vec::new());
            Ok(())
        },
    )
    .unwrap();
    sim.add_system(
        SystemDecl::new("boom"),
        Stage::Rules,
        |_ctx: &mut SystemCtx<'_>| Err(SimError::state("用例故意失败")),
    )
    .unwrap();
    sim.seal().unwrap();

    let before_hash = sim.hash();
    for attempt in 0..100 {
        sim.step(ONE).expect_err("系统报错必须让 step 失败");
        assert_eq!(
            sim.world().entity_count(),
            0,
            "第 {attempt} 次失败后不得留下结构操作的结果"
        );
        assert_eq!(sim.world().next_id(), 1, "id 分配不得前进");
        assert_eq!(sim.hash(), before_hash, "第 {attempt} 次失败后 hash 不得变");
    }
}

/// ②b 预检真的会「整队拒绝」：队列里后一条 op 注定失败（`despawn` 之后的 `insert`），
/// 于是前一条本来可以成功的 `spawn` 也不落笔——而不是「前一条先落了、后一条才失败」。
#[test]
fn a_queue_with_one_impossible_op_writes_nothing() {
    let mut sim = Sim::new(33);
    let pos = sim.register("Pos", ComponentDef::number()).unwrap();
    let tag = sim.register("Tag", ComponentDef::tag()).unwrap();
    sim.add_system(
        SystemDecl::new("queue"),
        Stage::Pre,
        move |ctx: &mut SystemCtx<'_>| {
            ctx.defer_spawn(2, vec![(pos, Column::I64(vec![1, 2]))], vec![tag]);
            // 这两条单看都合法，连起来不成立：5 号实体在 spawn 之后就存在，despawn 掉之后
            // 再 insert 必然 ENTITY 错。预检必须按队列累积判定（不能逐条对着原世界验）。
            ctx.defer_despawn(vec![5]);
            ctx.defer_insert(vec![5], vec![(pos, Column::I64(vec![3]))], Vec::new());
            Ok(())
        },
    )
    .unwrap();
    sim.seal().unwrap();

    let before_hash = sim.hash();
    let err = sim
        .step(ONE)
        .expect_err("队列里有一条注定失败，整队必须拒绍");
    assert_eq!(
        err.code(),
        codes::ENTITY,
        "错误码来自预检的粘影投影状态：{err}"
    );
    assert_eq!(
        sim.world().entity_count(),
        0,
        "整队拒绍 ⇒ 前面的 spawn 也不落笔"
    );
    assert_eq!(sim.hash(), before_hash);
}

/// ③ 失败 tick 里取过的随机数不算数：之后每个成功 tick 的 hash 与「从未失败过」的世界一致。
#[test]
fn rng_stream_is_not_advanced_by_a_failed_step() {
    let mut failing = world(true);
    for _ in 0..3 {
        failing.sim.step(ONE).expect_err("必须失败");
    }
    failing.armed.set(false);
    let mut clean = world(false);
    for tick in 0..5 {
        let actual = failing.sim.step(ONE).expect("失败已收起，必须成功");
        let expected = clean.sim.step(ONE).expect("成功");
        assert_eq!(
            actual, expected,
            "第 {tick} 个成功 tick 的 hash 必须与从未失败过的世界逐字节相同"
        );
    }
    assert_eq!(failing.sim.hash(), clean.sim.hash());
    assert_eq!(
        numbers(&failing.sim, failing.pos),
        numbers(&clean.sim, clean.pos),
        "写进 Pos 的随机值必须同序"
    );
}

/// ④ 先成功再失败：只有失败的那一 tick 被回滚，已经提交的那一 tick 保持。
#[test]
fn only_the_failing_step_is_rolled_back() {
    let mut w = world(false);
    let committed = w.sim.step(ONE).expect("第一步必须成功");
    let committed_values = numbers(&w.sim, w.pos);
    assert_eq!(w.sim.tick(), 1);
    assert_eq!(
        committed_values.len(),
        4,
        "4 行实体都写了（否则下面的值比对没有意义）"
    );

    w.armed.set(true);
    for attempt in 0..5 {
        w.sim.step(ONE).expect_err("必须失败");
        assert_eq!(
            w.sim.hash(),
            committed,
            "第 {attempt} 次失败后 hash 必须回到**上一个成功**的 tick"
        );
        assert_eq!(w.sim.tick(), 1, "tick 停在成功的那一步");
        assert_eq!(
            numbers(&w.sim, w.pos),
            committed_values,
            "第 {attempt} 次失败不得回滚已提交的写入"
        );
    }

    w.armed.set(false);
    let next = w.sim.step(ONE).expect("收起后必须成功");
    assert_ne!(next, committed, "世界真的在动（不是一直返回同一个 hash）");
    assert_eq!(w.sim.tick(), 2);
}
