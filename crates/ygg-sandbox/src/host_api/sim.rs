use super::HostState;
use mlua::{Error, Lua, Table, Value as LuaValue};
use std::cell::RefCell;
use std::rc::Rc;

/// 装载期安装完整 `ygg` 面（D-S9-1）。必须在 `Sandbox` 构造之后调用
/// （依赖 build 创建的 `ygg` 占位表；fuel / inject 之后再装本面不冲突——
/// 各自 raw_set 不同成员）。
/// 未挂载时的统一表述（码在首段，与 `YGG-FS-UNMOUNTED` 同一处置）。
fn sim_unmounted(what: &str) -> String {
    format!(
        "{}: 宿主未注入模拟内核，`{what}` 不可用（注入面见 `KernelFacade::set_sim`）",
        SIM_UNMOUNTED
    )
}

/// `ygg.sim.*` 的未挂载码（Tier 1 面；与 `ygg_fs_code` 分开命名空间以免两族混淆）。
pub const SIM_UNMOUNTED: &str = "YGG-SIM-UNMOUNTED";
/// 定点闸门的拒绝码（非整数值过面 / 除零）。
pub const SIM_FIXED: &str = "YGG-SIM-FIXED";

/// 定点闸门：Lua number → `i64`，**只接受值上等于整数**的数。
///
/// 这是 Tier 2 双精度与 Tier 1 定点之间唯一的闸门：静默取整会把「调用方算错了」变成
/// 「内核算错了」，而后者无法在回放证据里查出来。上限 2^53 是双精度可精确表示的整数边界
/// （超过它「值上等于整数」这个判断本身就不成立了）。
fn sim_fixed_arg(value: LuaValue, what: &str) -> Result<i64, Error> {
    const MAX_EXACT: f64 = 9_007_199_254_740_992.0;
    match value {
        LuaValue::Integer(v) => Ok(v),
        LuaValue::Number(n) if n.fract() == 0.0 && n.abs() <= MAX_EXACT => Ok(n as i64),
        other => Err(Error::runtime(format!(
            "{SIM_FIXED}: `{what}` 必须是值上等于整数的 Lua number（Q32.32），得到 {}",
            other.type_name()
        ))),
    }
}

/// 装 `ygg.sim.*`（10 个键，**一个逐实体 getter 都没有**——批式铁律）。
///
/// 表**始终**注册：未注入实例时每个函数抛 [`SIM_UNMOUNTED`] 而不是返回 `nil` / `false`。
/// `nil` 与「世界是空的」不可分辨，静默降级会让 Mod 把「宿主不支持」读成「世界为空」并据此
/// 写下错误状态。
pub(super) fn install_sim(lua: &Lua, state: &Rc<RefCell<HostState>>) -> Result<(), Error> {
    let ygg: Table = lua.globals().raw_get("ygg")?;
    let table = lua.create_table()?;

    let st = Rc::clone(state);
    table.raw_set(
        "step",
        lua.create_function(move |lua, dt: LuaValue| -> Result<Table, Error> {
            let dt = sim_fixed_arg(dt, "dt")?;
            let slot = Rc::clone(&st.borrow().sim);
            let (tick, hash) = {
                let mut guard = slot.borrow_mut();
                let sim = guard
                    .as_mut()
                    .ok_or_else(|| Error::runtime(sim_unmounted("sim.step")))?;
                let hash = sim.step(dt).map_err(Error::runtime)?;
                (sim.tick(), hash)
            };
            let out = lua.create_table()?;
            out.raw_set("tick", tick)?;
            out.raw_set("hash", format!("{hash:016x}"))?;
            Ok(out)
        })?,
    )?;

    let st = Rc::clone(state);
    table.raw_set(
        "hash",
        lua.create_function(move |_, ()| -> Result<String, Error> {
            let slot = Rc::clone(&st.borrow().sim);
            let guard = slot.borrow();
            let sim = guard
                .as_ref()
                .ok_or_else(|| Error::runtime(sim_unmounted("sim.hash")))?;
            Ok(format!("{:016x}", sim.hash()))
        })?,
    )?;

    let st = Rc::clone(state);
    table.raw_set(
        "snapshot",
        lua.create_function(move |lua, ()| -> Result<LuaValue, Error> {
            let slot = Rc::clone(&st.borrow().sim);
            let bytes = {
                let guard = slot.borrow();
                let sim = guard
                    .as_ref()
                    .ok_or_else(|| Error::runtime(sim_unmounted("sim.snapshot")))?;
                sim.snapshot()
            };
            Ok(LuaValue::String(lua.create_string(&bytes)?))
        })?,
    )?;

    let st = Rc::clone(state);
    table.raw_set(
        "restore",
        lua.create_function(move |lua, bytes: mlua::String| -> Result<Table, Error> {
            let slot = Rc::clone(&st.borrow().sim);
            let (tick, len, hash) = {
                let mut guard = slot.borrow_mut();
                let sim = guard
                    .as_mut()
                    .ok_or_else(|| Error::runtime(sim_unmounted("sim.restore")))?;
                sim.restore(&bytes.as_bytes()).map_err(Error::runtime)?;
                (sim.tick(), bytes.as_bytes().len(), sim.hash())
            };
            let out = lua.create_table()?;
            out.raw_set("tick", tick)?;
            out.raw_set("bytes", len)?;
            out.raw_set("hash", format!("{hash:016x}"))?;
            Ok(out)
        })?,
    )?;

    let st = Rc::clone(state);
    table.raw_set(
        "spawn_batch",
        lua.create_function(move |lua, frame: mlua::String| -> Result<Table, Error> {
            let slot = Rc::clone(&st.borrow().sim);
            let (spawned, next_id) = {
                let mut guard = slot.borrow_mut();
                let sim = guard
                    .as_mut()
                    .ok_or_else(|| Error::runtime(sim_unmounted("sim.spawn_batch")))?;
                let report = sim.spawn_batch(&frame.as_bytes()).map_err(Error::runtime)?;
                (report.spawned, report.next_id)
            };
            let out = lua.create_table()?;
            out.raw_set("spawned", spawned)?;
            out.raw_set("next_id", next_id)?;
            Ok(out)
        })?,
    )?;

    let st = Rc::clone(state);
    table.raw_set(
        "despawn_batch",
        lua.create_function(move |lua, frame: mlua::String| -> Result<Table, Error> {
            let slot = Rc::clone(&st.borrow().sim);
            let removed = {
                let mut guard = slot.borrow_mut();
                let sim = guard
                    .as_mut()
                    .ok_or_else(|| Error::runtime(sim_unmounted("sim.despawn_batch")))?;
                sim.despawn_batch(&frame.as_bytes())
                    .map_err(Error::runtime)?
            };
            let out = lua.create_table()?;
            out.raw_set("despawned", removed)?;
            Ok(out)
        })?,
    )?;

    let st = Rc::clone(state);
    table.raw_set(
        "insert_batch",
        lua.create_function(move |lua, frame: mlua::String| -> Result<Table, Error> {
            let slot = Rc::clone(&st.borrow().sim);
            let (entities, values) = {
                let mut guard = slot.borrow_mut();
                let sim = guard
                    .as_mut()
                    .ok_or_else(|| Error::runtime(sim_unmounted("sim.insert_batch")))?;
                let report = sim
                    .insert_batch(&frame.as_bytes())
                    .map_err(Error::runtime)?;
                (report.entities, report.values)
            };
            let out = lua.create_table()?;
            out.raw_set("entities", entities)?;
            out.raw_set("values", values)?;
            Ok(out)
        })?,
    )?;

    let st = Rc::clone(state);
    table.raw_set(
        "status",
        lua.create_function(move |lua, ()| -> Result<Table, Error> {
            let slot = Rc::clone(&st.borrow().sim);
            let (tick, entities, archetypes, hash) = {
                let guard = slot.borrow();
                let sim = guard
                    .as_ref()
                    .ok_or_else(|| Error::runtime(sim_unmounted("sim.status")))?;
                (
                    sim.tick(),
                    sim.world().entity_count(),
                    sim.world().archetype_count(),
                    sim.hash(),
                )
            };
            let out = lua.create_table()?;
            out.raw_set("mounted", true)?;
            out.raw_set("tick", tick)?;
            out.raw_set("entities", entities)?;
            out.raw_set("archetypes", archetypes)?;
            out.raw_set("hash", format!("{hash:016x}"))?;
            Ok(out)
        })?,
    )?;

    let st = Rc::clone(state);
    table.raw_set(
        "trace",
        lua.create_function(move |lua, on: bool| -> Result<Table, Error> {
            let slot = Rc::clone(&st.borrow().sim);
            {
                let mut guard = slot.borrow_mut();
                let sim = guard
                    .as_mut()
                    .ok_or_else(|| Error::runtime(sim_unmounted("sim.trace")))?;
                sim.trace(on);
            }
            let out = lua.create_table()?;
            out.raw_set("tracing", on)?;
            Ok(out)
        })?,
    )?;

    let st = Rc::clone(state);
    table.raw_set(
        "replay",
        lua.create_function(
            move |lua, (snapshot, log): (mlua::String, mlua::String)| -> Result<Table, Error> {
                let slot = Rc::clone(&st.borrow().sim);
                let report = {
                    let guard = slot.borrow();
                    let sim = guard
                        .as_ref()
                        .ok_or_else(|| Error::runtime(sim_unmounted("sim.replay")))?;
                    sim.replay(&snapshot.as_bytes(), &log.as_bytes())
                        .map_err(Error::runtime)?
                };
                let out = lua.create_table()?;
                out.raw_set("ticks", report.ticks)?;
                out.raw_set("matched", report.matched)?;
                out.raw_set(
                    "first_mismatch",
                    match report.first_mismatch {
                        Some(tick) => LuaValue::Integer(tick as i64),
                        None => LuaValue::Nil,
                    },
                )?;
                out.raw_set("final_hash", format!("{:016x}", report.final_hash))?;
                Ok(out)
            },
        )?,
    )?;

    ygg.raw_set("sim", table)?;
    Ok(())
}
