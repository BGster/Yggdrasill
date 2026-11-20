//! 基准：10⁴ 实体 / 4 archetype / 3 系统 / 200 tick（sim-kernel §5）。
//!
//! # 这个文件只在 `--release` 下有断言
//!
//! `cfg!(debug_assertions)` 为真时**只打印** `bench_skipped=dev-profile` 并返回。
//! 理由不是「debug 慢一点」而是**口径**：dev profile 的 tick 时间是 release 的十倍以上，
//! 拿它去对 16.6 ms 契约线只会得到假红。而 `cargo test --workspace` 走 dev profile——
//! 所以跳过是让字面三件套保持全绿的设计约束，不是妥协。
//!
//! # 方法（可复算）
//!
//! - 每轮**新建** Sim（避免状态累积影响）；前 20 tick warmup 丢弃；
//! - 单 tick 计时区只含 `sim.step(dt)`（不含实体构造）；
//! - 3 轮，各取逐 tick 中位，再取三轮中位的中位；
//! - 断言只用契约线（16.6 ms / 100 ms），并打印三轮原始样本与 p95/max 供复核。
//!
//! 期望量级：~0.24 MB/tick 载荷 + 一次 ~0.7 MB 快照哈希 ⇒ 预期中位 **1–3 ms**。
//! 若实测 >6 ms，按缺陷处理（命中 O(n²) 或每实体分配），而不是调阈值。

use std::time::Instant;

use ygg_sim::canonical::FrameWriter;
use ygg_sim::component::{ComponentDef, ComponentId};
use ygg_sim::fixed::{self, ONE};
use ygg_sim::sim::Sim;

const ROUNDS: usize = 3;
const TICKS: usize = 200;
const WARMUP: usize = 20;
const DT: i64 = ONE / 60; // 60 fps 的定点步长

struct Layout {
    pos: ComponentId,
    vel: ComponentId,
    hp: ComponentId,
    enemy: ComponentId,
}

fn layout(sim: &mut Sim) -> Layout {
    Layout {
        pos: sim.register("Pos", ComponentDef::number()).unwrap(),
        vel: sim.register("Vel", ComponentDef::number()).unwrap(),
        hp: sim.register("Hp", ComponentDef::number()).unwrap(),
        enemy: sim.register("Enemy", ComponentDef::tag()).unwrap(),
    }
}

/// 造 `spawn_batch` 帧：`names` 是组件名序，`columns` 与之一一对应（tag 无列）。
fn spawn_frame(rows: usize, names: &[&str], columns: &[Vec<i64>]) -> Vec<u8> {
    let mut writer = FrameWriter::new();
    writer.u32(rows as u32);
    writer.u16(names.len() as u16);
    for name in names {
        writer.u16(name.len() as u16);
        writer.raw(name.as_bytes());
    }
    for column in columns {
        for value in column {
            writer.i64(*value);
        }
    }
    writer.finish()
}

fn despawn_frame(ids: &[u64]) -> Vec<u8> {
    let mut writer = FrameWriter::new();
    writer.u32(ids.len() as u32);
    for id in ids {
        writer.u64(*id);
    }
    writer.finish()
}

/// 10⁴ 实体的固定构成（A/B/C/D 四桶）。
fn bench_world() -> Sim {
    let mut sim = Sim::new(20_261_120);
    let layout = layout(&mut sim);

    // 系统 1：pos += vel * dt（A、B 两桶）
    let (pos, vel) = (layout.pos, layout.vel);
    sim.add_system("integrate", move |ctx| {
        let dt = ctx.dt();
        for archetype in ctx.store_mut().archetypes_mut() {
            let (Some(pos_at), Some(vel_at)) = (archetype.col_pos(pos), archetype.col_pos(vel))
            else {
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

    // 系统 2：每 10 tick，hp -= 1（A、C 两桶）
    let hp = layout.hp;
    sim.add_system("decay", move |ctx| {
        if ctx.tick() % 10 != 0 {
            return Ok(());
        }
        for archetype in ctx.store_mut().archetypes_mut() {
            let Some(hp_at) = archetype.col_pos(hp) else {
                continue;
            };
            if let ygg_sim::archetype::Column::I64(values) = &mut archetype.cols[hp_at] {
                for value in values.iter_mut() {
                    *value = fixed::sub(*value, ONE);
                }
            }
        }
        Ok(())
    });

    // 系统 3：每 10 tick，换掉 D 桶的 1%（10 个）——走延迟队列与 archetype 复用路径
    let pos_only = 1u128 << layout.pos;
    let pos_for_churn = layout.pos;
    sim.add_system("churn", move |ctx| {
        if ctx.tick() % 10 != 0 {
            return Ok(());
        }
        let mut victims = Vec::new();
        for archetype in ctx.store_mut().archetypes_mut() {
            if archetype.key == pos_only {
                victims.extend(archetype.ids.iter().copied().take(10));
            }
        }
        if !victims.is_empty() {
            ctx.defer_despawn(victims);
        }
        let rows = 10usize;
        let column: Vec<i64> = (0..rows).map(|index| (index as i64) * ONE).collect();
        ctx.defer_spawn(
            rows,
            vec![(pos_for_churn, ygg_sim::archetype::Column::I64(column))],
            Vec::new(),
        );
        Ok(())
    });

    // 四桶：A=5000（Pos+Vel+Hp）、B=2500（Pos+Vel）、C=1500（Pos+Hp+Enemy）、D=1000（Pos）
    let zeros = |n: usize| vec![0i64; n];
    sim.spawn_batch(&spawn_frame(
        5_000,
        &["Pos", "Vel", "Hp"],
        &[zeros(5_000), vec![ONE; 5_000], vec![10 * ONE; 5_000]],
    ))
    .unwrap();
    sim.spawn_batch(&spawn_frame(
        2_500,
        &["Pos", "Vel"],
        &[zeros(2_500), vec![ONE / 2; 2_500]],
    ))
    .unwrap();
    sim.spawn_batch(&spawn_frame(
        1_500,
        &["Pos", "Hp", "Enemy"],
        &[zeros(1_500), vec![5 * ONE; 1_500]],
    ))
    .unwrap();
    sim.spawn_batch(&spawn_frame(1_000, &["Pos"], &[zeros(1_000)]))
        .unwrap();
    let _ = layout.enemy;
    sim
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn ms(elapsed: std::time::Duration) -> f64 {
    elapsed.as_secs_f64() * 1_000.0
}

#[test]
fn perf_tick_10k_entities() {
    if cfg!(debug_assertions) {
        println!("bench_skipped=dev-profile");
        return;
    }
    let mut round_medians = Vec::with_capacity(ROUNDS);
    for round in 0..ROUNDS {
        let mut sim = bench_world();
        for _ in 0..WARMUP {
            sim.step(DT).unwrap();
        }
        let mut samples = Vec::with_capacity(TICKS);
        for _ in 0..TICKS {
            let started = Instant::now();
            sim.step(DT).unwrap();
            samples.push(ms(started.elapsed()));
        }
        let mut ordered = samples.clone();
        ordered.sort_by(f64::total_cmp);
        let p95 = ordered[(ordered.len() as f64 * 0.95) as usize];
        let max = *ordered.last().unwrap();
        let median_ms = median(&mut samples);
        println!(
            "round={round} sim_tick_median_ms={median_ms:.3} p95={p95:.3} max={max:.3} \
             entities=10000 archetypes=4 systems=3 ticks={TICKS} warmup={WARMUP}"
        );
        round_medians.push(median_ms);
    }
    let mut medians = round_medians.clone();
    let median_ms = median(&mut medians);
    println!(
        "sim_tick_10k_median_ms={median_ms:.3}  rounds={round_medians:?} \
         (3 runs × {TICKS} ticks, warmup {WARMUP}, n=10000 entities)"
    );
    assert!(
        median_ms <= 16.6,
        "tick 超预算：{median_ms:.3} ms > 16.6 ms（按缺陷处理，不调阈值）"
    );
}

#[test]
fn perf_snapshot_restore_100k() {
    if cfg!(debug_assertions) {
        println!("bench_skipped=dev-profile");
        return;
    }
    // 1 archetype × 8 attrs × 100000 行
    let mut sim = Sim::new(7);
    let names = ["A0", "A1", "A2", "A3", "A4", "A5", "A6", "A7"];
    for name in names {
        sim.register(name, ComponentDef::number()).unwrap();
    }
    let rows = 100_000usize;
    let columns: Vec<Vec<i64>> = (0..8).map(|_| vec![ONE; rows]).collect();
    sim.spawn_batch(&spawn_frame(rows, &names, &columns))
        .unwrap();

    let started = Instant::now();
    let bytes = sim.snapshot();
    let snapshot_ms = ms(started.elapsed());

    let mut target = Sim::new(7);
    for name in names {
        target.register(name, ComponentDef::number()).unwrap();
    }
    let started = Instant::now();
    target.restore(&bytes).unwrap();
    let restore_ms = ms(started.elapsed());

    println!(
        "sim_snapshot_100k_ms={snapshot_ms:.3}  sim_restore_100k_ms={restore_ms:.3}  \
         bytes={} (1 archetype × 8 attrs, n=100000)",
        bytes.len()
    );
    assert_eq!(
        target.hash(),
        sim.hash(),
        "restore 后 hash 必须与原世界一致"
    );
    assert!(
        snapshot_ms <= 100.0,
        "snapshot 超预算：{snapshot_ms:.3} ms > 100 ms"
    );
    assert!(
        restore_ms <= 100.0,
        "restore 超预算：{restore_ms:.3} ms > 100 ms"
    );
}
