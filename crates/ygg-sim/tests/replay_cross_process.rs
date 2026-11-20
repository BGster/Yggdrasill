//! 跨进程 digest：父进程与子进程各跑同一份 fixture，逐 tick trace 必须逐字节相同。
//!
//! 契约：sim-kernel I1（同 seed + 同输入 ⇒ 跨进程一致）。这是「确定性」唯一的**硬证据**：
//! 同进程内的两次比较只能证明「这段代码没有隐藏的全局态」，跨进程才能证伪
//! 「某个值来自进程地址 / 环境 / 未初始化内存 / 随机哈希种子」。
//!
//! 做法：父进程算出 trace 字符串；再用 `current_exe()` 起一个子进程（同一个测试二进制，
//! 用 `YGG_SIM_CHILD=1` 切换行为、`--exact` 只跑本用例），子进程把 trace 写到临时文件；
//! 父进程逐字节比对。父/子都用**固定 fixture**（无共享 RNG、无时钟、无网络）。

use std::path::PathBuf;
use std::process::Command;

use ygg_sim::canonical::FrameWriter;
use ygg_sim::component::ComponentDef;
use ygg_sim::fixed::{self, ONE};
use ygg_sim::hash::fnv1a64;
use ygg_sim::sim::Sim;

const CHILD_ENV: &str = "YGG_SIM_CHILD";
const OUT_ENV: &str = "YGG_SIM_OUT";

/// 固定 fixture：60 tick、两系统、一次批量生成 + 一次批量删除。
///
/// 全字面量、无随机来源 ⇒ 同一份代码在任何进程里都给出同一串 `tick=.. hash=..`。
fn fixture_trace() -> String {
    let mut sim = Sim::new(4_242);
    let pos = sim.register("Pos", ComponentDef::number()).unwrap();
    let vel = sim.register("Vel", ComponentDef::number()).unwrap();
    let label = sim.register("Label", ComponentDef::string()).unwrap();

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
    let _ = label;

    // 初始 4 个实体（Pos / Vel / Label）
    let mut writer = FrameWriter::new();
    writer.u32(4);
    writer.u16(3);
    for name in ["Pos", "Vel", "Label"] {
        writer.u16(name.len() as u16);
        writer.raw(name.as_bytes());
    }
    for value in [0i64, ONE, 2 * ONE, 3 * ONE] {
        writer.i64(value);
    }
    for value in [ONE, -ONE, ONE / 2, 0] {
        writer.i64(value);
    }
    for text in ["alpha", "beta", "gamma", "种子"] {
        let bytes = text.as_bytes();
        writer.u32(bytes.len() as u32);
        writer.raw(bytes);
    }
    sim.spawn_batch(&writer.finish()).unwrap();

    let mut trace = String::new();
    for tick in 1..=60u64 {
        if tick == 20 {
            // 中途一次批量删除（结构操作必须影响后续每个 tick 的 hash）
            let mut writer = FrameWriter::new();
            writer.u32(1);
            writer.u64(2);
            sim.despawn_batch(&writer.finish()).unwrap();
        }
        let digest = sim.step(ONE / 30).unwrap();
        trace.push_str(&format!("tick={tick:03} hash={digest:016x}\n"));
    }
    trace
}

#[test]
fn digest_equal_across_processes() {
    // 子进程分支：只写 trace 就返回（不递归起进程）。
    if let Ok(out) = std::env::var(OUT_ENV) {
        if std::env::var(CHILD_ENV).is_ok() {
            let parent = PathBuf::from(out);
            std::fs::write(&parent, fixture_trace()).expect("子进程写 trace");
            return;
        }
    }

    let local = fixture_trace();
    let exe = std::env::current_exe().expect("current_exe");
    let out = std::env::temp_dir().join(format!("ygg-sim-child-{}.txt", std::process::id()));
    let status = Command::new(&exe)
        .env(CHILD_ENV, "1")
        .env(OUT_ENV, &out)
        .arg("--exact")
        .arg("digest_equal_across_processes")
        .arg("--nocapture")
        .status()
        .expect("启动子进程");
    assert!(status.success(), "子进程退出码非 0：{status:?}");
    let remote = std::fs::read_to_string(&out).expect("读子进程 trace");
    std::fs::remove_file(&out).ok();

    assert_eq!(
        local.lines().count(),
        60,
        "fixture 必须正好产出 60 行 trace"
    );
    assert_eq!(local, remote, "跨进程 trace 必须逐字节相同");
    let digest = fnv1a64(local.as_bytes());
    println!(
        "replay_cross_process lines={} parent_digest={digest:016x} child_digest={:016x} equal=true",
        local.lines().count(),
        fnv1a64(remote.as_bytes())
    );
}
