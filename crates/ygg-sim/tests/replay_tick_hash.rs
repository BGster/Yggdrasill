//! 回放用例：逐 tick hash 一致（同进程 / 从创世）+ 篡改必须被抓到。
//!
//! 契约：sim-kernel I1/I3。三条用例的分工：
//! - `same_process`：从**快照**续跑（存档 → 读档 → 继续跑）的等价性；
//! - `from_genesis`：仅 seed + 日志（证明 seed 流派生是**唯一**随机源）；
//! - `mismatch_detected`：**负例**——把某 tick 的 `dt` 改 1，回放必须报出那个 tick。
//!   没有这条，前两条就只是「断言恒真」的漂亮话。

use ygg_sim::canonical::FrameWriter;
use ygg_sim::component::{ComponentDef, ComponentId};
use ygg_sim::fixed::{self, ONE};
use ygg_sim::input::ReplayLog;
use ygg_sim::rng::stream_name;
use ygg_sim::sim::Sim;

const SEED: u64 = 20_261_120;

struct Scenario {
    sim: Sim,
    pos: ComponentId,
    vel: ComponentId,
    hp: ComponentId,
}

fn scenario() -> Scenario {
    let mut sim = Sim::new(SEED);
    let pos = sim.register("Pos", ComponentDef::number()).unwrap();
    let vel = sim.register("Vel", ComponentDef::number()).unwrap();
    let hp = sim.register("Hp", ComponentDef::number()).unwrap();
    // 系统 1：pos += vel * dt（只看有 Pos+Vel 的桶）
    let (p, v) = (pos, vel);
    sim.add_system("integrate", move |ctx| {
        let dt = ctx.dt();
        for archetype in ctx.store_mut().archetypes_mut() {
            let (Some(pos_at), Some(vel_at)) = (archetype.col_pos(p), archetype.col_pos(v)) else {
                continue;
            };
            let rows = archetype.ids.len();
            for row in 0..rows {
                let velocity = match archetype.cols[vel_at].value_at(row) {
                    ygg_sim::archetype::ValueRef::I64(value) => value,
                    _ => continue,
                };
                if let ygg_sim::archetype::Column::I64(values) = &mut archetype.cols[pos_at] {
                    values[row] = fixed::add(values[row], fixed::mul(velocity, dt));
                }
            }
        }
        Ok(())
    });
    // 系统 2：每 10 tick 抽一次随机数并消耗 Hp（顺带把 rng 状态推进，让回放必须复现它）
    let h = hp;
    sim.add_system("decay", move |ctx| {
        if ctx.tick() % 10 != 0 {
            return Ok(());
        }
        let stream = stream_name("decay", 0);
        // 借用顺序：先读布局（行数与 Hp 列位置），再一次性取够随机数，最后拿列写面。
        // 行数由**上一个** tick 的行集决定——它在整帧内稳定，所以先读后写是安全的。
        let plan: Vec<(Option<usize>, usize)> = ctx
            .store_mut()
            .archetypes_mut()
            .iter()
            .map(|archetype| (archetype.col_pos(h), archetype.ids.len()))
            .collect();
        let total: usize = plan.iter().map(|(_, rows)| *rows).sum();
        let draws: Vec<u32> = (0..total).map(|_| ctx.rng32(&stream)).collect();
        let mut cursor = 0usize;
        for (archetype, (hp_at, rows)) in
            ctx.store_mut().archetypes_mut().iter_mut().zip(plan.iter())
        {
            let Some(hp_at) = hp_at else {
                cursor += rows;
                continue;
            };
            for row in 0..*rows {
                let damage = i64::from(draws[cursor + row] % 3);
                if let ygg_sim::archetype::Column::I64(values) = &mut archetype.cols[*hp_at] {
                    values[row] = fixed::sub(values[row], damage * ONE);
                }
            }
            cursor += rows;
        }
        Ok(())
    });
    Scenario { sim, pos, vel, hp }
}

/// 造一个 `spawn_batch` 帧（3 个实体：Pos/Vel/Hp 各一列）。
fn spawn_frame(
    pos: ComponentId,
    vel: ComponentId,
    hp: ComponentId,
    values: &[(i64, i64, i64)],
) -> Vec<u8> {
    let mut writer = FrameWriter::new();
    writer.u32(values.len() as u32);
    writer.u16(3);
    for (id, name) in [(pos, "Pos"), (vel, "Vel"), (hp, "Hp")] {
        let _ = id;
        writer.u16(name.len() as u16);
        writer.raw(name.as_bytes());
    }
    for (p, _, _) in values {
        writer.i64(*p);
    }
    for (_, v, _) in values {
        writer.i64(*v);
    }
    for (_, _, h) in values {
        writer.i64(*h);
    }
    writer.finish()
}

/// 造一个 `despawn_batch` 帧。
fn despawn_frame(ids: &[u64]) -> Vec<u8> {
    let mut writer = FrameWriter::new();
    writer.u32(ids.len() as u32);
    for id in ids {
        writer.u64(*id);
    }
    writer.finish()
}

/// 跑 200 tick，第 50 tick 取快照并把之后的日志录下来。
fn run_with_recording() -> (Scenario, Vec<u8>, Vec<u8>, Vec<(u64, u64)>) {
    let mut world = scenario();
    world.sim.trace(true);
    // 初始实体：Pos/Vel/Hp 三列
    let frame = spawn_frame(
        world.pos,
        world.vel,
        world.hp,
        &[
            (0, ONE, 10 * ONE),
            (2 * ONE, -ONE, 7 * ONE),
            (5 * ONE, ONE / 2, 3 * ONE),
        ],
    );
    world.sim.spawn_batch(&frame).unwrap();
    for _ in 0..50 {
        world.sim.step(ONE / 10).unwrap();
    }
    let snapshot = world.sim.snapshot();
    world.sim.record(true);
    // 第 60 tick 再投一帧（结构操作必须被录进日志，回放才会重投）
    for tick in 51..=200 {
        if tick == 60 {
            let frame = spawn_frame(world.pos, world.vel, world.hp, &[(9 * ONE, ONE, 4 * ONE)]);
            world.sim.spawn_batch(&frame).unwrap();
        }
        if tick == 90 {
            world.sim.despawn_batch(&despawn_frame(&[2])).unwrap();
        }
        world.sim.step(ONE / 10).unwrap();
    }
    let log = world.sim.take_log().expect("录制中的日志").encode();
    let trace = world.sim.trace_entries().unwrap().to_vec();
    (world, snapshot, log, trace)
}

#[test]
fn same_process_replay_matches_every_tick() {
    let (world, snapshot, log, trace) = run_with_recording();
    let report = world.sim.replay(&snapshot, &log).unwrap();
    println!(
        "replay.same_process ticks={} matched={} first_mismatch={:?} final_hash={:016x}",
        report.ticks, report.matched, report.first_mismatch, report.final_hash
    );
    assert_eq!(report.ticks, 150, "回放覆盖 51..=200");
    assert_eq!(report.matched, report.ticks, "逐 tick hash 必须全等");
    assert_eq!(report.first_mismatch, None);
    // 与原始运行的末 tick hash 相等（同一世界、同一序列）
    let (last_tick, last_hash) = *trace.last().unwrap();
    assert_eq!(last_tick, 200);
    assert_eq!(report.final_hash, last_hash, "回放终点 hash 与原始运行一致");
    // 抽过随机数的系统必须复现：rng 状态随快照往返，续跑不重播
    assert_eq!(world.sim.tick(), 200);
}

#[test]
fn from_genesis_matches_the_original_trace() {
    // 只给 seed + 日志（无快照）：从创世重跑
    let (world, _snapshot, _log, trace) = run_with_recording();

    // 重新录一份从 tick 0 起的完整日志（含最初那条 spawn 帧）
    let mut fresh = scenario();
    fresh.sim.trace(true);
    fresh.sim.record(true);
    let frame = spawn_frame(
        fresh.pos,
        fresh.vel,
        fresh.hp,
        &[
            (0, ONE, 10 * ONE),
            (2 * ONE, -ONE, 7 * ONE),
            (5 * ONE, ONE / 2, 3 * ONE),
        ],
    );
    fresh.sim.spawn_batch(&frame).unwrap();
    for tick in 1..=200 {
        if tick == 10 {
            let frame = spawn_frame(fresh.pos, fresh.vel, fresh.hp, &[(9 * ONE, ONE, 4 * ONE)]);
            fresh.sim.spawn_batch(&frame).unwrap();
        }
        if tick == 40 {
            fresh.sim.despawn_batch(&despawn_frame(&[2])).unwrap();
        }
        fresh.sim.step(ONE / 10).unwrap();
    }
    let log = fresh.sim.take_log().unwrap().encode();
    let genesis_trace = fresh.sim.trace_entries().unwrap().to_vec();
    assert_eq!(genesis_trace.len(), 200);

    let report = fresh.sim.replay(&[], &log).unwrap();
    println!(
        "replay.from_genesis ticks={} matched={} first_mismatch={:?} final_hash={:016x}",
        report.ticks, report.matched, report.first_mismatch, report.final_hash
    );
    assert_eq!(report.ticks, 200);
    assert_eq!(report.matched, 200, "从创世回放必须逐 tick 一致");
    assert_eq!(report.first_mismatch, None);
    assert_eq!(report.final_hash, genesis_trace.last().unwrap().1);
    // 两条路径（存档续跑 vs 创世重跑）在 200 tick 后不是同一个世界——不同起点决定了
    // 不同的结构操作历史——但它们各自内部一致。这条断言防止「两个用例其实在测同一个东西」。
    assert_ne!(world.sim.hash(), fresh.sim.hash());
    let _ = trace;
}

#[test]
fn mismatch_detected_when_dt_is_tampered() {
    let (world, snapshot, log, _trace) = run_with_recording();
    let mut decoded = ReplayLog::decode(&log).unwrap();
    // 篡改第 7 条记录的 dt（+1 ulp）：回放必须在那个 tick 报失配
    let target = decoded.records()[6].tick;
    decoded.records_mut()[6].dt += 1;
    let tampered = decoded.encode();
    let report = world.sim.replay(&snapshot, &tampered).unwrap();
    println!(
        "replay.tampered ticks={} matched={} first_mismatch={:?} expected={target}",
        report.ticks, report.matched, report.first_mismatch
    );
    assert_eq!(
        report.first_mismatch,
        Some(target),
        "篡改 dt 必须在该 tick 被抓到（否则断言恒真）"
    );
    assert!(report.matched < report.ticks);
    // 未篡改的日志仍然全等（负例不是环境的错）
    let clean = world.sim.replay(&snapshot, &log).unwrap();
    assert_eq!(clean.first_mismatch, None);
}

#[test]
fn replay_of_empty_log_is_a_no_op_with_a_final_hash() {
    let world = scenario();
    let report = world
        .sim
        .replay(&world.sim.snapshot(), &ReplayLog::new(SEED, 0).encode())
        .unwrap();
    assert_eq!(report.ticks, 0);
    assert_eq!(report.matched, 0);
    assert_eq!(report.first_mismatch, None);
    assert_eq!(report.final_hash, world.sim.hash());
}
