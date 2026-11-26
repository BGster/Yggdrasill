use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Error, Function, Lua, Scope, Table, Value as LuaValue};
use ygg_sim::component::{ComponentId, EntityId};
use ygg_sim::system::SystemCtx;

use super::columns::slots_of;
use super::rows::{check_writable, iterate};
use super::values::{entity_of, entity_to_lua, payload_of};
use super::WORLD_BUSY;
use crate::host_api::HostState;

// ---------------------------------------------------------------------------
// 系统面：`run(dt, ctx)`
// ---------------------------------------------------------------------------

/// `ctx` 函数表：与 `world.*` 同名同义，但走 `SystemCtx` 的能力面。
///
/// 借用纪律：`SystemCtx` 由 `RefCell` 暂存，每个闭包进出各取一次 `try_borrow_mut`；
/// 借用失败（理论上不可达——同一时刻只有一个闭包在跑）给可读错误而不是 panic。
///
/// 生命周期：`'a` = `lua.scope(...)` 体这一小段（四个入参共用一个区间），
/// `'b` / `'c` = `SystemCtx` 自身的借用与内部生命周期（不变，与会话区间解耦）。
#[allow(clippy::needless_lifetimes)]
pub(super) fn ctx_table<'a, 'env, 'b, 'c>(
    lua: &'a Lua,
    scope: &'a Scope<'a, 'env>,
    state: &'a Rc<RefCell<HostState>>,
    cell: &'a RefCell<&'b mut SystemCtx<'c>>,
) -> Result<Table, Error> {
    let table = lua.create_table()?;

    table.raw_set(
        "tick",
        scope.create_function(|_, ()| -> Result<i64, Error> {
            let guard = cell
                .try_borrow()
                .map_err(|_| Error::runtime(format!("{WORLD_BUSY}: ctx 上下文已被占用")))?;
            Ok(i64::try_from(guard.tick()).unwrap_or(i64::MAX))
        })?,
    )?;

    table.raw_set(
        "dt",
        scope.create_function(|_, ()| -> Result<i64, Error> {
            let guard = cell
                .try_borrow()
                .map_err(|_| Error::runtime(format!("{WORLD_BUSY}: ctx 上下文已被占用")))?;
            Ok(guard.dt())
        })?,
    )?;

    let what = "ctx.each";
    table.raw_set(
        "each",
        scope.create_function(
            move |lua, (names, f): (Table, Function)| -> Result<i64, Error> {
                ctx_each(lua, state, cell, names, f, false, what)
            },
        )?,
    )?;
    let what = "ctx.each_mut";
    table.raw_set(
        "each_mut",
        scope.create_function(
            move |lua, (names, f): (Table, Function)| -> Result<i64, Error> {
                ctx_each(lua, state, cell, names, f, true, what)
            },
        )?,
    )?;

    table.raw_set(
        "ids",
        scope.create_function(|lua, names: Table| -> Result<Table, Error> {
            let mut guard = cell
                .try_borrow_mut()
                .map_err(|_| Error::runtime(format!("{WORLD_BUSY}: ctx 上下文已被占用")))?;
            let ctx = &mut **guard;
            let slots = {
                let world = state.borrow();
                slots_of(ctx.registry(), &world.world, &names, "ctx.ids")?
            };
            let comps: Vec<ComponentId> = slots.iter().map(|s| s.comp).collect();
            // `SystemCtx` 不暴露 `collect_ids`：用遍历收 id，序与 `each` 逐项相同。
            let mut ids: Vec<EntityId> = Vec::new();
            ctx.query(&comps, |rs| ids.push(rs.id()));
            let out = lua.create_table_with_capacity(ids.len(), 0)?;
            for (i, id) in ids.iter().enumerate() {
                out.raw_set(i + 1, entity_to_lua(*id)?)?;
            }
            Ok(out)
        })?,
    )?;

    table.raw_set(
        "rng",
        scope.create_function(|_, stream: String| -> Result<i64, Error> {
            let mut guard = cell
                .try_borrow_mut()
                .map_err(|_| Error::runtime(format!("{WORLD_BUSY}: ctx 上下文已被占用")))?;
            Ok(i64::from(guard.rng32(&stream)))
        })?,
    )?;

    table.raw_set(
        "spawn",
        scope.create_function(|_, payload: Table| -> Result<(), Error> {
            let mut guard = cell
                .try_borrow_mut()
                .map_err(|_| Error::runtime(format!("{WORLD_BUSY}: ctx 上下文已被占用")))?;
            let ctx = &mut **guard;
            let (cols, tags) = {
                let world = state.borrow();
                payload_of(ctx.registry(), &world.world, &payload, "ctx.spawn", true)?
            };
            ctx.defer_spawn(1, cols, tags);
            Ok(())
        })?,
    )?;

    table.raw_set(
        "despawn",
        scope.create_function(|_, id: LuaValue| -> Result<(), Error> {
            let id = entity_of(id, "ctx.despawn")?;
            let mut guard = cell
                .try_borrow_mut()
                .map_err(|_| Error::runtime(format!("{WORLD_BUSY}: ctx 上下文已被占用")))?;
            guard.defer_despawn(vec![id]);
            Ok(())
        })?,
    )?;

    table.raw_set(
        "insert",
        scope.create_function(|_, (id, payload): (LuaValue, Table)| -> Result<(), Error> {
            let id = entity_of(id, "ctx.insert")?;
            let mut guard = cell
                .try_borrow_mut()
                .map_err(|_| Error::runtime(format!("{WORLD_BUSY}: ctx 上下文已被占用")))?;
            let ctx = &mut **guard;
            let (cols, tags) = {
                let world = state.borrow();
                payload_of(ctx.registry(), &world.world, &payload, "ctx.insert", true)?
            };
            ctx.defer_insert(vec![id], cols, tags);
            Ok(())
        })?,
    )?;

    Ok(table)
}

fn ctx_each(
    lua: &Lua,
    state: &Rc<RefCell<HostState>>,
    cell: &RefCell<&mut SystemCtx<'_>>,
    names: Table,
    f: Function,
    mutate: bool,
    what: &str,
) -> Result<i64, Error> {
    let mut guard = cell
        .try_borrow_mut()
        .map_err(|_| Error::runtime(format!("{WORLD_BUSY}: ctx 上下文已被占用")))?;
    let ctx = &mut **guard;
    let slots = {
        let world = state.borrow();
        slots_of(ctx.registry(), &world.world, &names, what)?
    };
    if mutate {
        check_writable(&slots, what)?;
    }
    let comps: Vec<ComponentId> = slots.iter().map(|s| s.comp).collect();
    let visited = iterate(lua, &slots, &f, mutate, |body| ctx.query_mut(&comps, body))?;
    Ok(i64::try_from(visited).unwrap_or(i64::MAX))
}
