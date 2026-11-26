//! `seal` 的校验面（sim-kernel §3 / 计划 M1.3–1.4）。
//!
//! # 契约
//!
//! `seal` **只校验、不重排**：跑序恒为「阶段序（Pre → Rules → Post）→ 阶段内注册序」，
//! 声明表在 seal 前后逐条同序。校验项就是这六条，全部 fail-closed 并点名肇事系统：
//!
//! 1. 重名（跨阶段也算）；
//! 2. 未注册的组件 id（`reads` / `writes` 里）；
//! 3. 同阶段写-写（两系统写同一组件）；
//! 4. `after` 指到不存在的系统；
//! 5. `after` 指到**另一阶段**的系统（跨阶段先后靠 `Stage` 表达，不靠字符串）；
//! 6. 同阶段 `after` 成环。
//!
//! 同阶段「读 vs 写」不在拒收之列：读方看到的是同阶段前序系统写下的值（登记顺序即语义），
//! 这是积分式系统的常态，不是冲突。seal 之后再 `add_system` 一律 `YGG-SIM-SEALED`。
//!
//! # 为什么每条都要有独立用例
//!
//! 校验器最容易的失效形态不是「拒错了」，而是**根本走不到那一条**（前面一条先命中就返回）。
//! 所以每条用例都只制造一种缺陷，报错文案里必须出现肇事名——否则「拒收」可能来自别处。

use ygg_sim::archetype::Column;
use ygg_sim::component::{ComponentDef, ComponentId};
use ygg_sim::error::{codes, SimResult};
use ygg_sim::fixed::{self, ONE};
use ygg_sim::sim::Sim;
use ygg_sim::system::{Stage, SystemCtx, SystemDecl};

/// 什么都不做的系统体：本文件只测声明面，不测执行面（执行面在 `two_system_run_*`）。
fn noop(_ctx: &mut SystemCtx) -> SimResult<()> {
    Ok(())
}

/// 注册 Pos / Vel 两列（`Pos` 可写、`Vel` 只读），返回它们的 id。
fn world() -> (Sim, ComponentId, ComponentId) {
    let mut sim = Sim::new(20_261_120);
    let pos = sim.register("Pos", ComponentDef::number()).unwrap();
    let vel = sim.register("Vel", ComponentDef::number()).unwrap();
    (sim, pos, vel)
}

/// seal 必须拒收，且报错是第一段 `YGG-SIM-SEALED`、文案点名 `needle`。
fn assert_sealed(sim: &mut Sim, needle: &str) {
    let err = sim.seal().expect_err("seal 必须拒收");
    assert_eq!(err.code(), codes::SEALED, "拒收码必须是 seal：{err}");
    assert!(
        err.to_string().contains(needle),
        "报错必须点名 `{needle}`（否则拒收可能来自别的规则）：{err}"
    );
}

/// 1. 重名：同一 `name` 出现两次（这里跨阶段，证明判重不看阶段）。
#[test]
fn duplicate_name_is_rejected() {
    let (mut sim, pos, vel) = world();
    sim.add_system(SystemDecl::new("twin").writes(&[pos]), Stage::Rules, noop)
        .unwrap();
    sim.add_system(SystemDecl::new("twin").reads(&[vel]), Stage::Post, noop)
        .unwrap();
    assert_sealed(&mut sim, "twin");
}

/// 2. 未注册组件 id：声明里写了注册表里没有的列。
#[test]
fn unregistered_component_is_rejected() {
    let (mut sim, pos, _) = world();
    let ghost = ComponentId::from(9u32);
    sim.add_system(
        SystemDecl::new("reader").reads(&[ghost]),
        Stage::Rules,
        noop,
    )
    .unwrap();
    // 先把 Pos 用起来，免得「没有任何系统」这类无关形状干扰判读
    sim.add_system(SystemDecl::new("writer").writes(&[pos]), Stage::Post, noop)
        .unwrap();
    assert_sealed(&mut sim, "9");
}

/// 3. 同阶段写-写：两系统在 `Rules` 写同一组件。
#[test]
fn same_stage_write_write_is_rejected() {
    let (mut sim, pos, vel) = world();
    sim.add_system(
        SystemDecl::new("kick").writes(&[pos, vel]),
        Stage::Rules,
        noop,
    )
    .unwrap();
    sim.add_system(SystemDecl::new("thief").writes(&[pos]), Stage::Rules, noop)
        .unwrap();
    assert_sealed(&mut sim, "thief");
}

/// 4. `after` 指到不存在的系统。
#[test]
fn unknown_after_is_rejected() {
    let (mut sim, pos, _) = world();
    sim.add_system(
        SystemDecl::new("late").writes(&[pos]).after(&["ghost"]),
        Stage::Rules,
        noop,
    )
    .unwrap();
    assert_sealed(&mut sim, "ghost");
}

/// 5. 跨阶段 `after`：先后必须由 `Stage` 表达，字符串不得越阶段接线。
#[test]
fn cross_stage_after_is_rejected() {
    let (mut sim, pos, vel) = world();
    sim.add_system(SystemDecl::new("early").writes(&[pos]), Stage::Rules, noop)
        .unwrap();
    sim.add_system(
        SystemDecl::new("late").reads(&[vel]).after(&["early"]),
        Stage::Post,
        noop,
    )
    .unwrap();
    assert_sealed(&mut sim, "early");
}

/// 6. 同阶段成环：A after B、B after A。
#[test]
fn same_stage_cycle_is_rejected() {
    let (mut sim, pos, vel) = world();
    sim.add_system(
        SystemDecl::new("a").writes(&[pos]).after(&["b"]),
        Stage::Rules,
        noop,
    )
    .unwrap();
    sim.add_system(
        SystemDecl::new("b").writes(&[vel]).after(&["a"]),
        Stage::Rules,
        noop,
    )
    .unwrap();
    assert_sealed(&mut sim, "环");
}

/// 同阶段「读 vs 写」是合法的：读方读到前序系统写下的值，不构成冲突。
#[test]
fn same_stage_read_vs_write_is_accepted() {
    let (mut sim, pos, _) = world();
    sim.add_system(SystemDecl::new("writer").writes(&[pos]), Stage::Rules, noop)
        .unwrap();
    sim.add_system(
        SystemDecl::new("observer").reads(&[pos]),
        Stage::Rules,
        noop,
    )
    .unwrap();
    sim.seal().expect("同阶段读-写不拒");
    assert_eq!(
        sim.system_names(),
        vec!["writer", "observer"],
        "两条都应在册"
    );
    assert!(sim.is_sealed());
}

/// seal 可重入：第二次调用返回 `Ok` 且结果与第一次**逐字段相同**（不是「大约相同」）。
#[test]
fn seal_is_idempotent() {
    let (mut sim, pos, vel) = world();
    sim.add_system(SystemDecl::new("writer").writes(&[pos]), Stage::Rules, noop)
        .unwrap();
    sim.add_system(SystemDecl::new("observer").reads(&[vel]), Stage::Post, noop)
        .unwrap();
    let first = sim.seal();
    let second = sim.seal();
    assert!(first.is_ok(), "首次 seal 通过：{:?}", first.err());
    assert!(second.is_ok(), "重复 seal 必须仍为 Ok：{:?}", second.err());
    assert_eq!(
        sim.system_names(),
        vec!["writer", "observer"],
        "跑序不因 seal 改变"
    );
    assert!(sim.is_sealed());
}

/// seal 之后 `add_system` 一律 `YGG-SIM-SEALED`（终点线，不是建议）。
#[test]
fn add_after_seal_is_rejected() {
    let (mut sim, pos, _) = world();
    sim.add_system(SystemDecl::new("only").writes(&[pos]), Stage::Rules, noop)
        .unwrap();
    sim.seal().expect("先 seal 通过");
    let err = sim
        .add_system(SystemDecl::new("sneaky").reads(&[pos]), Stage::Rules, noop)
        .expect_err("seal 后追加必须拒");
    assert_eq!(err.code(), codes::SEALED, "拒收码必须是 seal：{err}");
    assert!(
        err.to_string().contains("sneaky"),
        "报错要点名肇事系统：{err}"
    );
}

/// 跑序 = 阶段序 → 阶段内注册序：`system_names()` 如实反映，不因 seal 变化。
#[test]
fn run_order_is_stage_then_registration() {
    let (mut sim, pos, vel) = world();
    // 故意乱序注册：Post 先、Rules 后——跑序必须把 Post 排到最后。
    sim.add_system(SystemDecl::new("post_a").writes(&[vel]), Stage::Post, noop)
        .unwrap();
    sim.add_system(
        SystemDecl::new("rules_a").writes(&[pos]),
        Stage::Rules,
        noop,
    )
    .unwrap();
    sim.add_system(SystemDecl::new("pre_a").reads(&[vel]), Stage::Pre, noop)
        .unwrap();
    let before: Vec<String> = sim.system_names().into_iter().map(str::to_owned).collect();
    sim.seal().expect("seal 通过");
    let after: Vec<String> = sim.system_names().into_iter().map(str::to_owned).collect();
    let expected = vec!["pre_a", "rules_a", "post_a"];
    assert_eq!(before, expected, "跑序 = 阶段序 → 阶段内注册序");
    assert_eq!(after, expected, "seal 不得重排");
}

/// 端到端：`Rules` 里的两系统（一写一读）都跑，且逐 tick 值跨两次独立运行逐位相同。
///
/// 这条是「声明面校验」之外的闭环：合法声明不但要 seal 通过，还要**跑得动且可复现**。
#[test]
fn two_system_run_is_hash_stable() {
    /// 一次完整运行：返回逐 tick 的 hash 与终态 hash。
    fn run() -> (Vec<u64>, u64) {
        let (mut sim, pos, vel) = world();
        let hp = sim.register("Hp", ComponentDef::number()).unwrap();
        // 系统 1：pos += vel * dt
        sim.add_system(
            SystemDecl::new("integrate").reads(&[vel]).writes(&[pos]),
            Stage::Rules,
            move |ctx| {
                let dt = ctx.dt();
                ctx.query_mut(&[pos, vel], |row| {
                    let (Some(position), Some(velocity)) = (row.fixed(0), row.fixed(1)) else {
                        return Ok(());
                    };
                    row.set_fixed(0, fixed::add(position, fixed::mul(velocity, dt)))
                })?;
                Ok(())
            },
        )
        .unwrap();
        // 系统 2：读同阶段前序系统写下的 pos（同阶段读-写合法），攒进 hp
        sim.add_system(
            SystemDecl::new("charge").reads(&[pos]).writes(&[hp]),
            Stage::Rules,
            move |ctx| {
                ctx.query_mut(&[hp, pos], |row| {
                    let (Some(hp), Some(position)) = (row.fixed(0), row.fixed(1)) else {
                        return Ok(());
                    };
                    row.set_fixed(0, fixed::add(hp, position))
                })?;
                Ok(())
            },
        )
        .unwrap();
        sim.seal().expect("合法声明必须 seal 通过");

        let rows = 16usize;
        sim.world_mut()
            .spawn_columns(
                rows,
                &[
                    (pos, Column::I64(vec![0; rows])),
                    (vel, Column::I64(vec![ONE; rows])),
                    (hp, Column::I64(vec![0; rows])),
                ],
                &[],
            )
            .unwrap();

        let mut hashes = Vec::with_capacity(32);
        for _ in 0..32 {
            hashes.push(sim.step(ONE).unwrap());
        }
        (hashes, sim.hash())
    }

    let (first, first_end) = run();
    let (second, second_end) = run();
    assert_eq!(first, second, "逐 tick hash 必须逐位相同");
    assert_eq!(first_end, second_end, "终态 hash 必须相同");
    // 非平凡：值真的在变（否则「相等」只是「都是 0」）
    assert!(
        first.windows(2).any(|pair| pair[0] != pair[1]),
        "逐 tick hash 不应全等——先证明状态在动，再证明它可复现"
    );
}
