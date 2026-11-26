//! 查询面 / 批量面的性能基线（`artifacts/w2-m1-world-plan.md` §5）。
//!
//! 运行：`cargo test -p ygg-sim --release --test perf_query -- --ignored --nocapture`
//!
//! # 只打印、不设门禁
//!
//! §5 的阈值（`for_each` 10k 行 ≤ 0.3 ms、`insert_many` 10k 行 ≤ 2 ms）是设计意图，
//! 但在共享机器上用断言钉死会给出假红（同一份代码在 CI 与工作站上差数倍）。所以本文件
//! 打印 `op=… rows=… us=… budget_us=…`，把实测数与预算并排交给复核者；判别方式是比对
//! 数量级（O(n) 与 O(n²) 差两个数量级以上），不是比 1 µs。
//!
//! # 为什么 `insert_many` 这条真的能测出 O(n²)
//!
//! 10k 行 × 2 列走 `insert_columns` 的去重与迁移路径：去重若回到 `Vec::contains`
//! （10k² / 2 ≈ 5·10⁷ 次比较），release 下也会多出数十毫秒；排序 + 相邻扫描是 O(n log n)。

use std::time::Instant;

use ygg_sim::archetype::Column;
use ygg_sim::component::{ComponentDef, ComponentId};
use ygg_sim::sim::Sim;

const N: usize = 10_000;

fn ramp(scale: i64) -> Column {
    Column::I64((0..N).map(|i| i as i64 * scale).collect())
}

fn world() -> (Sim, ComponentId, ComponentId, ComponentId) {
    let mut sim = Sim::new(1);
    let pos = sim.register("Pos", ComponentDef::number()).unwrap();
    let vel = sim.register("Vel", ComponentDef::number()).unwrap();
    let hp = sim.register("Hp", ComponentDef::number()).unwrap();
    (sim, pos, vel, hp)
}

/// 计时一次调用：返回值 + 微秒数（计时区只含 `f`，不含数据构造）。
fn micros<T>(f: impl FnOnce() -> T) -> (T, u128) {
    let start = Instant::now();
    let value = f();
    (value, start.elapsed().as_micros())
}

#[test]
#[ignore = "性能基线：只在 --release 下手工跑（cargo test -p ygg-sim --release --test perf_query -- --ignored --nocapture）"]
fn query_and_batch_baseline() {
    let (mut sim, pos, vel, hp) = world();

    let (spawned, spawn_us) = micros(|| {
        sim.world_mut()
            .spawn_columns(N, &[(pos, ramp(1)), (vel, ramp(2)), (hp, ramp(3))], &[])
    });
    let ids = spawned.unwrap();
    assert_eq!(ids.len(), N);
    println!("op=spawn_columns rows={N} cols=3 us={spawn_us}");

    // for_each 10k 行 × 3 组件：读一列并累加（防闭包被优化成空转，同时证明真的走了行）。
    let mut checksum = 0i64;
    let (rows, for_each_us) = micros(|| {
        sim.world().for_each(&[pos, vel, hp], |rs| {
            checksum = checksum.wrapping_add(rs.fixed(2).unwrap_or(0));
        })
    });
    println!("op=for_each rows={rows} cols=3 us={for_each_us} budget_us=300 checksum={checksum}");
    assert_eq!(rows, N as u64);

    let mut out = Column::I64(Vec::new());
    let (read, get_us) = micros(|| sim.world().get_many(hp, &ids, &mut out).unwrap());
    println!("op=get_many rows={read} us={get_us}");

    let payload = ramp(5);
    let mut written = 0usize;
    let (_, set_us) = micros(|| {
        written = sim.world_mut().set_many(hp, &ids, &payload).unwrap();
    });
    println!("op=set_many rows={written} us={set_us}");

    // insert_many：另起一个世界，10k 实体从 {Pos} 迁到 {Pos,Vel,Hp}（去重 + 迁移全路径）。
    let (mut batch, bpos, bvel, bhp) = world();
    let batch_ids = batch
        .world_mut()
        .spawn_columns(N, &[(bpos, ramp(1))], &[])
        .unwrap();
    let (vel_col, hp_col) = (ramp(2), ramp(3));
    let (inserted, insert_us) = micros(|| {
        batch
            .world_mut()
            .insert_columns(&batch_ids, &[(bvel, vel_col), (bhp, hp_col)], &[])
    });
    let values = inserted.unwrap();
    assert_eq!(
        batch.world().entity_count(),
        N,
        "计时区必须真的搬了 10k 实体"
    );
    println!("op=insert_columns rows={N} cols=2 us={insert_us} budget_us=2000 values={values}");
}
