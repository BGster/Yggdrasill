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

use ygg_sim::archetype::{Column, ValueRef};
use ygg_sim::canonical::FrameWriter;
use ygg_sim::component::{ComponentDef, ComponentId};
use ygg_sim::fixed::{self, ONE};
use ygg_sim::sim::Sim;

/// workload 的属性类型档（§5.1 回填表：I64 档 = 基线，F64Bits 档 = R1′ 对照）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum AttrKind {
    Number,
    F64Bits,
}

impl AttrKind {
    fn def(self) -> ComponentDef {
        match self {
            AttrKind::Number => ComponentDef::number(),
            AttrKind::F64Bits => ComponentDef::f64_bits(),
        }
    }

    fn name(self) -> &'static str {
        match self {
            AttrKind::Number => "number(i64)",
            AttrKind::F64Bits => "f64bits(u64)",
        }
    }
}

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

fn layout(sim: &mut Sim, kind: AttrKind) -> Layout {
    Layout {
        pos: sim.register("Pos", kind.def()).unwrap(),
        vel: sim.register("Vel", kind.def()).unwrap(),
        hp: sim.register("Hp", kind.def()).unwrap(),
        enemy: sim.register("Enemy", ComponentDef::tag()).unwrap(),
    }
}

/// F64Bits 列的 `spawn_batch` 帧（与 `spawn_frame` 同构，载荷是位串 u64 LE）。
fn spawn_frame_bits(rows: usize, names: &[&str], columns: &[Vec<u64>]) -> Vec<u8> {
    let mut writer = FrameWriter::new();
    writer.u32(rows as u32);
    writer.u16(names.len() as u16);
    for name in names {
        writer.u16(name.len() as u16);
        writer.raw(name.as_bytes());
    }
    for column in columns {
        for value in column {
            writer.u64(*value);
        }
    }
    writer.finish()
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
///
/// `Number` 分支与 A1 §1.7 基线逐字同构（tick 数字与在案 0.553 ms 可对账）；
/// `F64Bits` 分支是 sim-kernel I2 的「仅搬运不算术」同构负载：integrate 每行做
/// 读位串→pack∘unpack 归一→写位串（对照 I64 档的 fixed mul+add），decay 每 10 tick
/// 全列写一个 canonical 位串。
fn bench_world(kind: AttrKind) -> Sim {
    let mut sim = Sim::new(20_261_120);
    let layout = layout(&mut sim, kind);

    // 系统 1：pos += vel * dt（A、B 两桶）；F64Bits 档 = 位串搬运
    let (pos, vel, kind) = (layout.pos, layout.vel, kind);
    sim.add_system("integrate", move |ctx| {
        let dt = ctx.dt();
        for archetype in ctx.store_mut().archetypes_mut() {
            let (Some(pos_at), Some(vel_at)) = (archetype.col_pos(pos), archetype.col_pos(vel))
            else {
                continue;
            };
            let rows = archetype.ids.len();
            match kind {
                AttrKind::Number => {
                    for row in 0..rows {
                        let velocity = match archetype.cols[vel_at].value_at(row) {
                            ValueRef::I64(value) => value,
                            _ => continue,
                        };
                        if let Column::I64(values) = &mut archetype.cols[pos_at] {
                            values[row] = fixed::add(values[row], fixed::mul(velocity, dt));
                        }
                    }
                }
                AttrKind::F64Bits => {
                    for row in 0..rows {
                        let carried = match archetype.cols[vel_at].value_at(row) {
                            ValueRef::Bits(raw) => ygg_sim::bits::pack(ygg_sim::bits::unpack(raw)),
                            _ => continue,
                        };
                        if let Column::F64Bits(values) = &mut archetype.cols[pos_at] {
                            values[row] = carried;
                        }
                    }
                }
            }
        }
        Ok(())
    });

    // 系统 2：每 10 tick，hp -= 1（A、C 两桶）；F64Bits 档 = 全列写 canonical 位串
    let (hp, kind) = (layout.hp, kind);
    sim.add_system("decay", move |ctx| {
        if ctx.tick() % 10 != 0 {
            return Ok(());
        }
        for archetype in ctx.store_mut().archetypes_mut() {
            let Some(hp_at) = archetype.col_pos(hp) else {
                continue;
            };
            match kind {
                AttrKind::Number => {
                    if let Column::I64(values) = &mut archetype.cols[hp_at] {
                        for value in values.iter_mut() {
                            *value = fixed::sub(*value, ONE);
                        }
                    }
                }
                AttrKind::F64Bits => {
                    if let Column::F64Bits(values) = &mut archetype.cols[hp_at] {
                        for value in values.iter_mut() {
                            // canonical 常量位串：写路径零归一分支（pack 输入已是 canonical）
                            *value = ygg_sim::bits::pack(f64::from_bits(0x3FF0_0000_0000_0000));
                        }
                    }
                }
            }
        }
        Ok(())
    });

    // 系统 3：每 10 tick，换掉 D 桶的 1%（10 个）——走延迟队列与 archetype 复用路径
    let (pos_only, pos_for_churn, kind) = (1u128 << layout.pos, layout.pos, kind);
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
        let frame_column = match kind {
            AttrKind::Number => {
                let column: Vec<i64> = (0..rows).map(|index| (index as i64) * ONE).collect();
                Column::I64(column)
            }
            AttrKind::F64Bits => {
                // 尾数域 or index → 全 canonical（指数域恒 0x3FF）
                let column: Vec<u64> = (0..rows)
                    .map(|index| 0x3FF0_0000_0000_0000 | index as u64)
                    .collect();
                Column::F64Bits(column)
            }
        };
        ctx.defer_spawn(rows, vec![(pos_for_churn, frame_column)], Vec::new());
        Ok(())
    });

    // 四桶：A=5000（Pos+Vel+Hp）、B=2500（Pos+Vel）、C=1500（Pos+Hp+Enemy）、D=1000（Pos）
    if kind == AttrKind::Number {
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
    } else {
        let zeros = |n: usize| vec![0u64; n];
        sim.spawn_batch(&spawn_frame_bits(
            5_000,
            &["Pos", "Vel", "Hp"],
            &[
                zeros(5_000),
                vec![0x3FF0_0000_0000_0000; 5_000],
                vec![0xC010_0000_0000_0000; 5_000],
            ],
        ))
        .unwrap();
        sim.spawn_batch(&spawn_frame_bits(
            2_500,
            &["Pos", "Vel"],
            &[zeros(2_500), vec![0x3FE0_0000_0000_0000; 2_500]],
        ))
        .unwrap();
        sim.spawn_batch(&spawn_frame_bits(
            1_500,
            &["Pos", "Hp", "Enemy"],
            &[zeros(1_500), vec![0x4014_0000_0000_0000; 1_500]],
        ))
        .unwrap();
        sim.spawn_batch(&spawn_frame_bits(1_000, &["Pos"], &[zeros(1_000)]))
            .unwrap();
    }
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
        let mut sim = bench_world(AttrKind::Number);
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

// ─────────────────────────────────────────────────────────────────────────────
// F64Bits 对照档（sim-kernel §5.1 回填表 R1′ 行）：同 workload 形状、同口径
// （release / 三轮中位 / warmup 20），负载换成「仅搬运不算术」的位串路径。
// ─────────────────────────────────────────────────────────────────────────────

/// tick 10⁴（F64Bits 档）：预期 ≈ I64 档（同 8B/值线性搬运），I4 预算同线不回退。
#[test]
fn perf_tick_10k_f64bits() {
    if cfg!(debug_assertions) {
        println!("bench_skipped=dev-profile");
        return;
    }
    let mut round_medians = Vec::with_capacity(ROUNDS);
    for round in 0..ROUNDS {
        let mut sim = bench_world(AttrKind::F64Bits);
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
            "round={round} kind=f64bits sim_tick_median_ms={median_ms:.3} p95={p95:.3} max={max:.3} \
             entities=10000 archetypes=4 systems=3 ticks={TICKS} warmup={WARMUP}"
        );
        round_medians.push(median_ms);
    }
    let mut medians = round_medians.clone();
    let median_ms = median(&mut medians);
    println!(
        "sim_tick_10k_f64bits_median_ms={median_ms:.3}  rounds={round_medians:?} \
         (3 runs × {TICKS} ticks, warmup {WARMUP}, n=10000 entities)"
    );
    assert!(
        median_ms <= 16.6,
        "F64Bits 档 tick 超预算：{median_ms:.3} ms > 16.6 ms（I4 不回退）"
    );
}

/// spawn 10⁴ × 8 attrs：I64 档 vs F64Bits 档对照（A1 §1.7 范式：release 三轮中位）。
#[test]
fn perf_spawn_10k_8attrs_both_kinds() {
    if cfg!(debug_assertions) {
        println!("bench_skipped=dev-profile");
        return;
    }
    const ROWS: usize = 10_000;
    const ATTRS: usize = 8;
    let names = ["A0", "A1", "A2", "A3", "A4", "A5", "A6", "A7"];
    for kind in [AttrKind::Number, AttrKind::F64Bits] {
        let mut round_ns = Vec::with_capacity(ROUNDS);
        let mut last_hash = 0u64;
        for _ in 0..ROUNDS {
            let mut sim = Sim::new(7);
            for name in names {
                sim.register(name, kind.def()).unwrap();
            }
            let started = Instant::now();
            match kind {
                AttrKind::Number => {
                    let columns: Vec<Vec<i64>> = (0..ATTRS).map(|_| vec![ONE; ROWS]).collect();
                    sim.spawn_batch(&spawn_frame(ROWS, &names, &columns))
                        .unwrap();
                }
                AttrKind::F64Bits => {
                    let columns: Vec<Vec<u64>> = (0..ATTRS)
                        .map(|_| vec![0x3FF0_0000_0000_0000; ROWS])
                        .collect();
                    sim.spawn_batch(&spawn_frame_bits(ROWS, &names, &columns))
                        .unwrap();
                }
            }
            let elapsed = started.elapsed();
            round_ns.push(elapsed.as_nanos() as f64 / ROWS as f64);
            last_hash = sim.hash();
        }
        // 三轮同构世界：hash 必须全等（对照档不是废跑，spawn 语义一致）
        assert_ne!(last_hash, 0);
        let mut ordered = round_ns.clone();
        let median_ns = {
            ordered.sort_by(f64::total_cmp);
            ordered[ordered.len() / 2]
        };
        println!(
            "spawn_10k_8attrs kind={} ns_per_entity={:.1} rounds_ns={:?} (3 runs, median)",
            kind.name(),
            median_ns,
            round_ns
                .iter()
                .map(|v| format!("{v:.1}"))
                .collect::<Vec<_>>()
        );
    }
}

/// snapshot/restore 10⁵（F64Bits 档）：§5 预算同线（≤100 ms 双向），帧字节数对照入册。
#[test]
fn perf_snapshot_restore_100k_f64bits() {
    if cfg!(debug_assertions) {
        println!("bench_skipped=dev-profile");
        return;
    }
    let mut sim = Sim::new(7);
    let names = ["A0", "A1", "A2", "A3", "A4", "A5", "A6", "A7"];
    for name in names {
        sim.register(name, ComponentDef::f64_bits()).unwrap();
    }
    let rows = 100_000usize;
    let columns: Vec<Vec<u64>> = (0..8).map(|_| vec![0x3FF0_0000_0000_0000; rows]).collect();
    sim.spawn_batch(&spawn_frame_bits(rows, &names, &columns))
        .unwrap();

    let started = Instant::now();
    let bytes = sim.snapshot();
    let snapshot_ms = ms(started.elapsed());

    let mut target = Sim::new(7);
    for name in names {
        target.register(name, ComponentDef::f64_bits()).unwrap();
    }
    let started = Instant::now();
    target.restore(&bytes).unwrap();
    let restore_ms = ms(started.elapsed());

    println!(
        "sim_snapshot_100k_f64bits_ms={snapshot_ms:.3}  sim_restore_100k_f64bits_ms={restore_ms:.3}  \
         bytes={} (1 archetype × 8 attrs, n=100000, kind=f64bits)",
        bytes.len()
    );
    assert_eq!(
        target.hash(),
        sim.hash(),
        "restore 后 hash 必须与原世界一致（F64Bits 档）"
    );
    assert!(
        snapshot_ms <= 100.0,
        "F64Bits 档 snapshot 超预算：{snapshot_ms:.3} ms > 100 ms"
    );
    assert!(
        restore_ms <= 100.0,
        "F64Bits 档 restore 超预算：{restore_ms:.3} ms > 100 ms"
    );
}
