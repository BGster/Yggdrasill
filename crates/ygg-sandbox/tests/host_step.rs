//! M1-D 宿主推进入口 `Sandbox::step` 的门禁：闸门（0 / 超预算）、时钟与哈希、
//! 快照往返、坏帧 fail-closed，以及「Lua 侧不再有任何时钟、实体面或事件面」
//! 这条结构性断言。
//!
//! 原 `tests/sim_face.rs` 测的是 `ygg` 的 `sim.*` 这 10 个 Lua 键——那个面随 M1-D
//! 整体删除（时间只能由宿主推），所以本文件把其中仍然成立的**行为**（推进、
//! 哈希口径、快照往返、坏帧拒绝）迁到宿主侧，把已经消失的**接口形态**
//! （键集、`pcall` 文案、逐键 UNMOUNTED）连同被测接口一起删除。

use std::cell::RefCell;
use std::rc::Rc;

use mlua::Value as LuaValue;
use ygg_core::kv::KvStore;
use ygg_sandbox::host_api::{self, HostHandles};
use ygg_sandbox::modload::{Loaded, ModBundle};
use ygg_sandbox::sandbox::{Sandbox, StepError, STEP_TICK_BUDGET};
use ygg_sim::canonical::FrameWriter;
use ygg_sim::component::ComponentDef;
use ygg_sim::fixed::ONE;
use ygg_sim::sim::Sim;

/// 三个实体两个数值组件的最小世界（走批量帧，不经 Lua）。
fn sim_with_entities() -> Sim {
    let mut sim = Sim::new(4_242);
    sim.register("Pos", ComponentDef::number()).unwrap();
    sim.register("Vel", ComponentDef::number()).unwrap();
    let mut writer = FrameWriter::new();
    writer.u32(3);
    writer.u16(2);
    for name in ["Pos", "Vel"] {
        writer.u16(name.len() as u16);
        writer.raw(name.as_bytes());
    }
    for value in [0i64, ONE, 2 * ONE] {
        writer.i64(value);
    }
    for value in [ONE, ONE, ONE] {
        writer.i64(value);
    }
    sim.spawn_batch(&writer.finish()).unwrap();
    sim
}

/// 只挂世界、不装宿主面的沙箱（推进不需要 Lua 参与）。
fn mounted() -> Sandbox {
    let mut sandbox = Sandbox::new().expect("沙箱构造");
    sandbox.set_sim(Some(Box::new(sim_with_entities())));
    sandbox
}

#[test]
fn step_advances_the_tick_and_reports_the_same_hash() {
    let mut sandbox = mounted();
    let report = sandbox.step(3, 1).expect("推进必须成功");
    assert_eq!(report.tick, 3, "ticks 次推进后内核时钟 = 3");
    assert_eq!(sandbox.tick().expect("已挂载"), 3);
    assert_eq!(
        report.hash,
        sandbox.hash().expect("已挂载"),
        "回执 hash 与推进后的世界哈希同源"
    );
    // 幂等性口径：再推 0 次不行，推 1 次必须与上一次不同（世界真的在动）。
    let next = sandbox.step(1, 1).expect("推进必须成功");
    assert_eq!(next.tick, 4);
    assert_ne!(next.hash, report.hash, "tick 变了，哈希必须跟着变");
}

#[test]
fn zero_ticks_is_rejected_with_its_own_code() {
    let mut sandbox = mounted();
    let error = sandbox.step(0, 1).expect_err("0 tick 无意义，必须拒");
    assert_eq!(error.code(), "E_STEP_TICKS");
    assert!(
        matches!(error, StepError::ZeroTicks),
        "必须是 ZeroTicks 变体：{error:?}"
    );
    assert_eq!(sandbox.tick().expect("已挂载"), 0, "拒绝不得推进时钟");
}

#[test]
fn budget_is_rejected_before_a_single_tick_runs() {
    let mut sandbox = mounted();
    let error = sandbox
        .step(STEP_TICK_BUDGET + 1, 1)
        .expect_err("超预算必须拒");
    assert_eq!(error.code(), "E_STEP_BUDGET");
    assert!(
        matches!(error, StepError::Budget { .. }),
        "必须是 Budget 变体：{error:?}"
    );
    assert_eq!(
        sandbox.tick().expect("已挂载"),
        0,
        "闸门在循环之前——拒绝不得留下半个 tick"
    );
    // 上界本身放行（边界是「>」不是「>=」）。
    let edge = sandbox.step(STEP_TICK_BUDGET, 0).expect("上界放行");
    assert_eq!(edge.tick, u64::from(STEP_TICK_BUDGET));
}

#[test]
fn step_without_a_mounted_world_fails_closed() {
    let mut sandbox = Sandbox::new().expect("沙箱构造");
    let error = sandbox.step(1, 1).expect_err("未注入内核时必须报错");
    assert!(
        error.to_string().contains("YGG-SIM-UNMOUNTED"),
        "错误必须点名未挂载：{error}"
    );
    assert!(sandbox.tick().is_err(), "未挂载时读时钟同样报错");
    assert!(sandbox.snapshot().is_err(), "未挂载时快照同样报错");
}

#[test]
fn snapshot_round_trips_across_steps() {
    let mut sandbox = mounted();
    let three = sandbox.step(3, 1).expect("推进必须成功");
    let saved = sandbox.snapshot().expect("快照");
    let ten = sandbox.step(7, 1).expect("推进必须成功");
    assert_eq!(ten.tick, 10);
    assert_ne!(ten.hash, three.hash);

    sandbox.restore(&saved).expect("恢复必须成功");
    assert_eq!(sandbox.tick().expect("已挂载"), 3, "恢复回到快照时的时钟");
    assert_eq!(
        sandbox.hash().expect("已挂载"),
        three.hash,
        "恢复后的哈希逐位回到快照时刻"
    );
    // 往返幂等：同一帧再快照一次，字节完全一致。
    assert_eq!(sandbox.snapshot().expect("快照"), saved);
}

#[test]
fn bad_frames_fail_closed_without_moving_the_world() {
    let mut sandbox = mounted();
    let before = sandbox.step(2, 1).expect("推进必须成功");
    let error = sandbox
        .restore(&[0u8, 1, 2, 3, 4, 5, 6, 7])
        .expect_err("坏帧必须拒");
    assert!(
        error.to_string().contains("YGG-SIM-FRAME"),
        "坏帧必须 YGG-SIM-FRAME：{error}"
    );
    assert_eq!(sandbox.tick().expect("已挂载"), before.tick, "先校验后改");
    assert_eq!(sandbox.hash().expect("已挂载"), before.hash);
}

#[test]
fn lua_has_no_time_or_entity_surface_left() {
    // Lua 不能驱动时间：`ygg` 的 `sim` / `ygg` 的 `entity` 两面整体消失，推进只在宿主侧。
    // 同时 `world.*` 在场（M1-C 的唯一 Lua 世界面，是**全局表**不是 `ygg.world`）。
    let sandbox = Sandbox::new().expect("沙箱构造");
    let api = host_api::install(
        sandbox.lua(),
        HostHandles::new(Rc::new(RefCell::new(KvStore::new()))),
        sandbox.sim_handle(),
    )
    .expect("host API 必须可装");
    api.set_sim(Some(Box::new(Sim::new(1))));
    let ygg = sandbox
        .lua()
        .globals()
        .get::<mlua::Table>("ygg")
        .expect("ygg 全局必须存在");
    assert!(
        matches!(ygg.get::<LuaValue>("sim").expect("sim 键"), LuaValue::Nil),
        "`ygg` 的 `sim` 面必须不存在（时间只能由宿主推）"
    );
    assert!(
        matches!(
            ygg.get::<LuaValue>("entity").expect("entity 键"),
            LuaValue::Nil
        ),
        "`ygg` 的 `entity` 面必须不存在（第二套实体模型已删除）"
    );
    for gone in ["on", "emit", "narrate", "choice", "meta", "rt"] {
        assert!(
            matches!(ygg.get::<LuaValue>(gone).expect("键"), LuaValue::Nil),
            "ygg.{gone} 必须不存在（事件 / 信封层已删除）"
        );
    }
    let world = sandbox
        .lua()
        .globals()
        .get::<mlua::Table>("world")
        .expect("world 全局必须存在");
    assert!(
        world
            .get::<LuaValue>("tick")
            .expect("tick 键")
            .as_function()
            .is_some(),
        "world.tick 必须可调用（读时钟，不推时钟）"
    );
    assert!(
        world.get::<LuaValue>("step").expect("step 键").is_nil(),
        "world.step 不存在——Lua 没有推进入口"
    );
}

#[test]
fn step_and_snapshot_share_one_world() {
    // 宿主推进与快照是同一个世界：step 之后的帧带着新时钟，读回仍是它。
    const MOD: &str = r#"
world.component{ name = "hp", kind = "i64" }
world.spawn{ hp = 7 }
"#;
    let bundle = ModBundle::in_memory(MOD);
    let mut loaded = Loaded::from_bundle(&bundle).expect("kernel construction");
    let report = loaded.step(4, 1).expect("推进必须成功");
    assert_eq!(report.tick, 4);
    assert_eq!(loaded.tick().expect("已挂载"), 4);
    let frame = loaded.snapshot().expect("快照");
    assert!(
        frame.len() > 32,
        "快照必须携带世界帧（不是行存表）：{} 字节",
        frame.len()
    );
    let restored = Loaded::restore_from_bundle(&bundle, &frame).expect("读档重建");
    assert_eq!(restored.tick().expect("已挂载"), 4, "读档回到快照时的时钟");
    assert_eq!(
        restored.snapshot().expect("快照"),
        frame,
        "帧往返逐字节不变"
    );
}

/// M2-B：`Loaded` 只给**只读**的内核视图 + 唯一的推进入口。
///
/// 为什么要一条行为断言而不是只看签名：`Loaded.sim` 曾经是 `pub Rc<RefCell<..>>`，
/// 宿主一句 `*loaded.sim.borrow_mut() = 另一个 Sim` 就绕过了 `Sandbox::step` 的
/// `STEP_TICK_BUDGET` 与「时间只从宿主走」，而**签名看上去完全正常**。现在外部拿到的是
/// `Ref`：观察期间另一个借用方（包括宿主自己从 `Sandbox` 取到的同一句柄）拿不到 `RefMut`。
/// 这条断言把“不得中途换掉内核”钉成可执行事实，而不是注释里的承诺。
#[test]
fn loaded_hands_out_a_read_only_kernel_and_the_only_clock() {
    const MOD: &str = r#"
world.component{ name = "hp", kind = "i64" }
"#;
    let bundle = ModBundle::in_memory(MOD);
    let mut loaded = Loaded::from_bundle(&bundle).expect("kernel construction");

    // ① 时钟只能经 `Loaded::step` 前进：观察面看得到，推不动。
    assert_eq!(loaded.sim().as_ref().expect("装载即挂世界").tick(), 0);
    loaded.step(1, 1).expect("推进必须成功");
    assert_eq!(loaded.sim().as_ref().expect("装载即挂世界").tick(), 1);

    // ② 观察期间内核槽不可替换（同一个 `Rc`，只是外部只能拿 `Ref`）。
    let observed = loaded.sim();
    let handle = loaded.sandbox.sim_handle();
    assert!(
        handle.try_borrow_mut().is_err(),
        "观察期间不得换掉内核（否则 `Loaded` 不再是时间的唯一驱动面）"
    );
    drop(observed);
    assert!(
        handle.try_borrow_mut().is_ok(),
        "观察结束后句柄仍归 `Sandbox`"
    );
}
