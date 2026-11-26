use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Error, Function, Lua, MultiValue, Table, Value as LuaValue};
use ygg_sim::component::ComponentId;
use ygg_sim::error::SimError;
use ygg_sim::sim::Sim;
use ygg_sim::system::SystemCtx;

use super::columns::{slot_of, slots_of};
use super::ctx::ctx_table;
use super::rows::{check_writable, iterate, sim_slot};
use super::values::{sim_to_lua, world_busy, world_unmounted};
use crate::host_api::HostState;

/// `world.system` 的可选字符串数组字段（缺项 = 空表）。
pub(super) fn names_field(spec: &Table, key: &str) -> Result<Vec<String>, Error> {
    let Some(list) = spec.raw_get::<Option<Table>>(key)? else {
        return Ok(Vec::new());
    };
    let mut out: Vec<String> = Vec::new();
    for item in list.sequence_values::<LuaValue>() {
        match item? {
            LuaValue::String(s) => out.push(s.to_str()?.to_owned()),
            other => {
                return Err(sim_to_lua(SimError::schema(format!(
                    "系统声明的 `{key}` 必须是字符串数组，遇到 {}",
                    other.type_name()
                ))))
            }
        }
    }
    Ok(out)
}

pub(super) fn resolve_ids(
    sim: &Sim,
    names: &[String],
    state: &Rc<RefCell<HostState>>,
) -> Result<Vec<ComponentId>, Error> {
    let world = state.borrow();
    let registry = sim.world().registry();
    names
        .iter()
        .map(|name| Ok(slot_of(registry, &world.world, name)?.comp))
        .collect()
}

/// 系统体：把 `SystemCtx` 暂存进 `RefCell`，为本次调用造一张 `ctx` 表并调 `run(dt, ctx)`。
/// `run(dt, ctx)` 的 `dt` 与 `ctx.dt()` 都是 **Q32.32 裸值**——与 `ygg` 的 `sim.step(dt)`
/// 的入参**同一单位**。世界面的组件值是 `i64` 纯整数（schema 不带 `frac_bits`），
/// 所以面内没有第二套换算；擅自按 `to_int` 截断会把 60Hz 的 tick（dt ≈ 2^25）全截成 0，
/// 而擅自右移 32 位又与「调用方传给 `step` 的那个数」差 2³² 倍——两个都是静默错。
/// 要整数秒就在作者侧写 `dt >> 32`（`ygg-sim::fixed::ONE = 1 << 32`）。
pub(super) fn run_system(
    lua: &Lua,
    state: &Rc<RefCell<HostState>>,
    run: &Function,
    name: &str,
    ctx: &mut SystemCtx<'_>,
) -> Result<(), SimError> {
    let dt = ctx.dt();
    let cell = RefCell::new(ctx);
    let outcome = lua.scope(|scope| {
        let view = ctx_table(lua, scope, state, &cell)?;
        run.call::<()>((dt, view))
    });
    outcome.map_err(|e| SimError::state(format!("系统 `{name}` 执行失败：{e}")))
}

/// `world.each` / `world.each_mut` 的共同实现（返回值 = 访问的行数）。
pub(super) fn top_each(
    lua: &Lua,
    state: &Rc<RefCell<HostState>>,
    names: &Table,
    f: Function,
    mutate: bool,
) -> Result<i64, Error> {
    let what = if mutate {
        "world.each_mut"
    } else {
        "world.each"
    };
    let slots = {
        let slot = sim_slot(state);
        let guard = slot.try_borrow().map_err(|_| world_busy(what))?;
        let sim = guard.as_ref().ok_or_else(|| world_unmounted(what))?;
        let world = state.borrow();
        slots_of(sim.world().registry(), &world.world, names, what)?
    };
    if mutate {
        check_writable(&slots, what)?;
    }
    let comps: Vec<ComponentId> = slots.iter().map(|s| s.comp).collect();
    let slot = sim_slot(state);
    let mut guard = slot.try_borrow_mut().map_err(|_| world_busy(what))?;
    let sim: &mut Sim = guard.as_mut().ok_or_else(|| world_unmounted(what))?;
    let visited = iterate(lua, &slots, &f, mutate, |body| {
        sim.world_mut().for_each_mut(&comps, body)
    })?;
    Ok(i64::try_from(visited).unwrap_or(i64::MAX))
}
/// `world.log(fmt, ...)` 的排版器：**经字符串元表**调 `string.format`。
///
/// M2 起 `string` 表已是沙箱全局（[`crate::sandbox`] 的 `LIBRARY_WHITELIST`），
/// 但这个一次性编译的包装器留着：它走的是**字符串值元表**那条路（`("x"):format()`），
/// 于是排版能力与「库表是否露头」脱钩——以后收紧或放宽库表白名单不必再动日志面。
/// 排版语义与 `string.format` 同一实现，**不另造格式化器**。
pub(super) const LOG_FORMAT_SRC: &str =
    "return function(fmt, ...) return tostring(fmt):format(...) end";

/// `world.log(fmt, ...)`：结果进环形缓冲（[`WORLD_LOG_CAP`](super::WORLD_LOG_CAP)），由宿主经
/// [`super::HostApi::take_world_log`](crate::host_api::HostApi::take_world_log) 取走。**不进**叙述流、**不进**事件流：
/// 日志是诊断面，混进那两条流会让「输出 = 状态」的对账多出一类噪声。
///
/// 排版失败（格式串里有多余 `%`、`%d` 配了字符串等）**不抛给游戏回路**：诊断面
/// 不许打断确定序，退化成各实参 `tostring` 以空格拼接。
pub(super) fn format_line(
    format: &Function,
    tostring_fn: &Function,
    fmt: LuaValue,
    rest: MultiValue,
) -> Result<String, Error> {
    let mut argv = MultiValue::with_capacity(rest.len() + 1);
    argv.push_back(fmt.clone());
    for value in &rest {
        argv.push_back(value.clone());
    }
    if let Ok(line) = format.call::<String>(argv) {
        return Ok(line);
    }
    let mut parts = Vec::with_capacity(rest.len() + 1);
    parts.push(tostring_fn.call::<String>(fmt)?);
    for value in rest {
        parts.push(tostring_fn.call::<String>(value)?);
    }
    Ok(parts.join(" "))
}
