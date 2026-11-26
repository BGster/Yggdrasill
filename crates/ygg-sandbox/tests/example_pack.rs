//! 参考包 `packs/example` 的端到端装载与确定性（M3.4a）。
//!
//! 这里跑的是**真内核**（不是 `selftest` 的普通 Lua 解释器）：`packs/example`
//! 必须作为一份合法的 v2 声明面被完整装载——数据面进资源表、代码面进模块表、
//! 规则系统在 `world.*` 面上注册。棋盘语义本身由 Lua 侧的
//! `packs/example/test/turn.lua` 覆盖，本文件只钉三件内核侧事实：
//!
//! 1. 50 tick 能跑完且 `tick == 50`（规则不越界、不在系统里摸 `world.*`）；
//! 2. **同种子两次数装载 ⇒ 同哈希**（规则里的 `rng` 只走 `world.rng(stream)`）；
//! 3. 一个 tick 的 `changed` 只报这一 tick 真写过的组件（内核不认识格子，
//!    它只认识列；这条断言就是「规则写在哪里」的可观测证据）。

use std::collections::BTreeSet;
use std::path::PathBuf;

use ygg_sandbox::modload::Loaded;

/// 组件 id 是**声明序**（`src/units.lua` 的 `declare()`）：pos=0 hp=1 team=2 moved=3。
/// 这四行是参考包对内核的契约面，改声明序就是改包契约。
const POS: u32 = 0;
const HP: u32 = 1;
const TEAM: u32 = 2;
const MOVED: u32 = 3;

fn example_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packs/example")
}

/// 装一次、按给定种子重建：种子进世界流，规则里的 `rng("ai.move")` 才可复现。
fn load(seed: u64) -> Loaded {
    let loaded = Loaded::from_dir(&example_dir()).expect("装载 packs/example");
    loaded.fork_with_seed(seed).expect("按种子重建")
}

#[test]
fn steps_fifty_ticks_and_two_loads_agree_on_the_hash() {
    let mut a = load(0x5eed);
    let mut b = load(0x5eed);
    assert_eq!(a.tick().unwrap(), 0, "装载后 tick 从 0 起算");
    assert_eq!(a.hash().unwrap(), b.hash().unwrap(), "同一种子装载即同哈希");

    let report = a.step(50, 1).expect("50 tick 跑得完");
    b.step(50, 1).expect("第二次装载同样跑得完");

    assert_eq!(report.tick, 50);
    assert_eq!(a.tick().unwrap(), 50);
    assert_eq!(report.hash, a.hash().unwrap(), "报告哈希即当前状态哈希");
    // 两次独立装载（各自一份 VM、各自一轮 spawn）必须逐字节同一世界：
    // 快照比哈希更严——哈希相同但帧不同的实现会在这里露馅。
    assert_eq!(
        a.snapshot().unwrap(),
        b.snapshot().unwrap(),
        "同种子两次装载必须产出同一世界帧"
    );
}

#[test]
fn one_tick_reports_only_the_components_that_tick_wrote() {
    let mut world = load(7);
    // 装载期排队的 spawn 在第一个 tick 边界落地（十个单位 × 四个组件），
    // 所以从第二 tick 起才是「移动」稳态：advance 清零 moved、ai.move 写 pos+moved。
    world.step(1, 1).unwrap();

    let entities: BTreeSet<u64> = world
        .changed()
        .iter()
        .map(|(entity, _component)| *entity)
        .collect();
    assert_eq!(entities.len(), 10, "棋盘上恰好十个单位（每行两端各一个）");

    for _ in 0..5 {
        let report = world.step(1, 1).unwrap();
        let written: BTreeSet<(u64, u32)> = report.changed.iter().copied().collect();
        // 稳态 tick 的写面 = 十个单位的 pos + moved 两列，一个不多一个不少。
        let expected: BTreeSet<(u64, u32)> = entities
            .iter()
            .flat_map(|entity| [(*entity, POS), (*entity, MOVED)])
            .collect();
        assert_eq!(written, expected, "tick {} 的写面", report.tick);
        // 显式再断言一次：战斗与分组这一 tick 没被碰过（它们没写就不该报）。
        assert!(!written.iter().any(|(_, c)| *c == HP || *c == TEAM));
    }
}
