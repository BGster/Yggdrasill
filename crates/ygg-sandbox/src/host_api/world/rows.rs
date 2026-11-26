use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Error, Function, Lua, MultiValue, Value as LuaValue};
use ygg_sim::error::SimError;
use ygg_sim::query::RowSet;
use ygg_sim::sim::Sim;

use super::columns::{Kind, Slot};
use super::values::{cell_to_lua, entity_to_lua, int_of, sim_to_lua, world_busy, world_unmounted};
use crate::host_api::HostState;

// ---------------------------------------------------------------------------
// 内核槽访问（重入与未挂载都是可读错误，永不 panic）
// ---------------------------------------------------------------------------

pub(super) fn sim_slot(state: &Rc<RefCell<HostState>>) -> Rc<RefCell<Option<Box<Sim>>>> {
    Rc::clone(&state.borrow().sim)
}

pub(super) fn with_sim_mut<R>(
    state: &Rc<RefCell<HostState>>,
    what: &str,
    f: impl FnOnce(&mut Sim) -> Result<R, Error>,
) -> Result<R, Error> {
    let slot = sim_slot(state);
    let mut guard = slot.try_borrow_mut().map_err(|_| world_busy(what))?;
    let sim: &mut Sim = guard.as_mut().ok_or_else(|| world_unmounted(what))?;
    f(sim)
}

// ---------------------------------------------------------------------------
// 迭代：单行喂参 / 写回 / 错误透传
// ---------------------------------------------------------------------------

/// 迭代内 Lua 错误的透传槽：内核只认 `SimError`，Lua 错误先存这里、用哨兵中止内核迭代，
/// 出迭代后**原样**抛出（用户错误不被降级成 `YGG-SIM-STATE` 文本，码与消息都保住）。
const ABORT_SENTINEL: &str = "__world_iteration_aborted__";

#[derive(Default)]
struct Abort(Option<Error>);

impl Abort {
    fn trip(&mut self, e: Error) -> SimError {
        self.0 = Some(e);
        SimError::state(ABORT_SENTINEL)
    }

    fn take(&mut self) -> Option<Error> {
        self.0.take()
    }
}

/// 一行 → Lua 实参（`id, v1, v2, …`，按调用方声明序）并调用回调。
///
/// **性能契约**：每行的值作为**多个返回值**逐个进实参缓冲，**不建行表**——逐行
/// `lua.create_table()` 会给每一行分配一个 Lua 对象并把 GC 拖进内循环。
#[allow(clippy::too_many_arguments)]
fn call_row(
    lua: &Lua,
    slots: &[Slot],
    rs: &mut RowSet<'_>,
    f: &Function,
    mutate: bool,
    args: &mut MultiValue,
) -> Result<(), Error> {
    args.clear();
    args.push_back(LuaValue::Integer(entity_to_lua(rs.id())?));
    for (k, slot) in slots.iter().enumerate() {
        args.push_back(cell_to_lua(lua, slot, rs, k as u8).map_err(sim_to_lua)?);
    }
    if !mutate {
        f.call::<()>(&*args)?;
        return Ok(());
    }
    let returns: MultiValue = f.call(&*args)?;
    if returns.len() != slots.len() {
        return Err(sim_to_lua(SimError::schema(format!(
            "each_mut 的回调必须按声明序返回 {} 个值，得到 {}",
            slots.len(),
            returns.len()
        ))));
    }
    for (k, slot) in slots.iter().enumerate() {
        let Some(value) = returns.get(k) else {
            continue;
        };
        match slot.kind {
            Kind::I64 | Kind::U8 => {
                let raw = int_of(slot, value)?;
                rs.set_fixed(k as u8, raw).map_err(sim_to_lua)?;
            }
            Kind::U64 => {
                let raw = int_of(slot, value)?;
                let raw = u64::try_from(raw).map_err(|_| {
                    sim_to_lua(SimError::fixed(format!(
                        "组件 `{}` 是 u64，得到 {raw}",
                        slot.name
                    )))
                })?;
                rs.set_u64(k as u8, raw).map_err(sim_to_lua)?;
            }
            // `slots_of` + 入参只读检查已把这两种挡在迭代之外。
            Kind::Bytes | Kind::Tag => {
                return Err(sim_to_lua(SimError::schema(format!(
                    "组件 `{}` 不能经 each_mut 写回",
                    slot.name
                ))))
            }
        }
    }
    Ok(())
}

/// 跑一遍迭代：`run` 决定从哪个面取行（`ArchetypeStore::for_each_mut` 或
/// `SystemCtx::query_mut`），本函数只负责逐行喂参、写回与错误透传。
pub(super) fn iterate<F>(
    lua: &Lua,
    slots: &[Slot],
    f: &Function,
    mutate: bool,
    run: F,
) -> Result<u64, Error>
where
    F: FnOnce(&mut dyn FnMut(&mut RowSet<'_>) -> Result<(), SimError>) -> Result<u64, SimError>,
{
    let mut abort = Abort::default();
    let mut args = MultiValue::with_capacity(slots.len() + 1);
    let mut body = |rs: &mut RowSet<'_>| -> Result<(), SimError> {
        match call_row(lua, slots, rs, f, mutate, &mut args) {
            Ok(()) => Ok(()),
            Err(e) => Err(abort.trip(e)),
        }
    };
    let visited = run(&mut body);
    if let Some(e) = abort.take() {
        // 用户错误优先于内核的哨兵错：透传原始码与消息。
        return Err(e);
    }
    visited.map_err(sim_to_lua)
}

/// `each_mut` 的入参只读检查：`bytes`（缺 `set_bytes`）与 tag（无列）不可能写回，
/// 在迭代**开始前**拒绝，避免写了一半才发现。
pub(super) fn check_writable(slots: &[Slot], what: &str) -> Result<(), Error> {
    for slot in slots {
        if matches!(slot.kind, Kind::I64 | Kind::U64 | Kind::U8) {
            continue;
        }
        return Err(sim_to_lua(SimError::schema(format!(
            "`{what}` 不能写 `{}`（{}）：tag 没有列，bytes 列在内核查询面上没有写回口；\
             用 `world.set` 逐实体写",
            slot.name,
            slot.kind.label()
        ))));
    }
    Ok(())
}
