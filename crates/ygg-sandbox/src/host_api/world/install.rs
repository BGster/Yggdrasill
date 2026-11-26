use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Error, Function, Lua, MultiValue, Table, Value as LuaValue};
use ygg_sim::archetype::ValueRef;
use ygg_sim::component::{ComponentId, EntityId};
use ygg_sim::error::SimError;
use ygg_sim::system::{Stage, SystemCtx, SystemDecl};

use super::columns::{slot_of, slots_of, Kind};
use super::rows::with_sim_mut;
use super::systems::{format_line, names_field, resolve_ids, run_system, top_each, LOG_FORMAT_SRC};
use super::values::{entity_of, entity_to_lua, payload_of, sim_to_lua, step_to_lua};
use crate::host_api::marshal::sim_fixed_arg;
use crate::host_api::HostState;
use crate::world_ops::{self, WorldOp};

// ---------------------------------------------------------------------------
// 安装
// ---------------------------------------------------------------------------

/// 装 `world` 面（全局表 `world`；须在 `ygg.kv` 之后——`world.kv` 直接复用那张表）——
/// 一块表里 17 个键，逐个 `raw_set`。
pub(crate) fn install_world(lua: &Lua, state: &Rc<RefCell<HostState>>) -> Result<(), Error> {
    let ygg: Table = lua.globals().raw_get("ygg")?;
    let world = lua.create_table()?;

    // ---- world.component{ name, kind } ----
    let st = Rc::clone(state);
    world.raw_set(
        "component",
        lua.create_function(move |_, spec: Table| -> Result<i64, Error> {
            let name: String = spec.raw_get("name")?;
            let kind_name: String = spec.raw_get("kind")?;
            let Some(kind) = Kind::parse(&kind_name) else {
                return Err(sim_to_lua(SimError::schema(format!(
                    "`world.component` 的 kind 必须是 i64|u64|u8|bytes，\
                     组件 `{name}` 得到 `{kind_name}`"
                ))));
            };
            let id = with_sim_mut(&st, "world.component", |sim| {
                if sim.world().registry().find(&name).is_some() {
                    return Err(sim_to_lua(SimError::schema(format!(
                        "组件 `{name}` 已声明过（world.component 每个组件只声明一次）"
                    ))));
                }
                sim.register(&name, kind.def()).map_err(sim_to_lua)
            })?;
            st.borrow_mut().world.kinds.insert(id, kind);
            Ok(i64::from(id))
        })?,
    )?;

    // ---- world.spawn{ ... } -> id ----
    let st = Rc::clone(state);
    world.raw_set(
        "spawn",
        lua.create_function(move |_, payload: Table| -> Result<i64, Error> {
            with_sim_mut(&st, "world.spawn", |sim| {
                let (cols, tags) = {
                    let world = st.borrow();
                    payload_of(
                        sim.world().registry(),
                        &world.world,
                        &payload,
                        "world.spawn",
                        true,
                    )?
                };
                let report = world_ops::apply_ops(
                    sim,
                    &[WorldOp::Spawn {
                        rows: 1,
                        cols,
                        tags,
                    }],
                )
                .map_err(step_to_lua)?;
                let id = report
                    .spawned
                    .first()
                    .copied()
                    .ok_or_else(|| sim_to_lua(SimError::state("spawn 未返回实体 id")))?;
                entity_to_lua(id)
            })
        })?,
    )?;

    // ---- world.despawn(id) -> n ----
    let st = Rc::clone(state);
    world.raw_set(
        "despawn",
        lua.create_function(move |_, id: LuaValue| -> Result<i64, Error> {
            let id = entity_of(id, "world.despawn")?;
            with_sim_mut(&st, "world.despawn", |sim| {
                let report = world_ops::apply_ops(sim, &[WorldOp::Despawn { ids: vec![id] }])
                    .map_err(step_to_lua)?;
                Ok(i64::try_from(report.writes).unwrap_or(i64::MAX))
            })
        })?,
    )?;

    // ---- world.insert(id, { ... }) -> n ----
    let st = Rc::clone(state);
    world.raw_set(
        "insert",
        lua.create_function(
            move |_, (id, payload): (LuaValue, Table)| -> Result<i64, Error> {
                let id = entity_of(id, "world.insert")?;
                with_sim_mut(&st, "world.insert", |sim| {
                    let (cols, tags) = {
                        let world = st.borrow();
                        payload_of(
                            sim.world().registry(),
                            &world.world,
                            &payload,
                            "world.insert",
                            true,
                        )?
                    };
                    let report = world_ops::apply_ops(
                        sim,
                        &[WorldOp::Insert {
                            ids: vec![id],
                            cols,
                            tags,
                        }],
                    )
                    .map_err(step_to_lua)?;
                    Ok(i64::try_from(report.writes).unwrap_or(i64::MAX))
                })
            },
        )?,
    )?;

    // ---- world.set(id, { ... }) -> n ----
    let st = Rc::clone(state);
    world.raw_set(
        "set",
        lua.create_function(
            move |_, (id, payload): (LuaValue, Table)| -> Result<i64, Error> {
                let id = entity_of(id, "world.set")?;
                with_sim_mut(&st, "world.set", |sim| {
                    let (cols, tags) = {
                        let world = st.borrow();
                        payload_of(
                            sim.world().registry(),
                            &world.world,
                            &payload,
                            "world.set",
                            false,
                        )?
                    };
                    debug_assert!(tags.is_empty(), "world.set 已在 payload_of 里拒绝 tag");
                    // 一个组件一条 `Set`：整批一个事务——多组件写也只有全成/全回两种结局。
                    let ops: Vec<WorldOp> = cols
                        .into_iter()
                        .map(|(col, values)| WorldOp::Set {
                            ids: vec![id],
                            col,
                            values,
                        })
                        .collect();
                    let count = ops.len();
                    world_ops::apply_ops(sim, &ops).map_err(step_to_lua)?;
                    Ok(i64::try_from(count).unwrap_or(i64::MAX))
                })
            },
        )?,
    )?;

    // ---- world.get(id, name) -> value | nil ----
    let st = Rc::clone(state);
    world.raw_set(
        "get",
        lua.create_function(
            move |lua, (id, name): (LuaValue, String)| -> Result<LuaValue, Error> {
                let id = entity_of(id, "world.get")?;
                with_sim_mut(&st, "world.get", |sim| {
                    let slot = {
                        let world = st.borrow();
                        slot_of(sim.world().registry(), &world.world, &name)?
                    };
                    let store = sim.world();
                    let (arch_idx, _) = store.locate(id).map_err(sim_to_lua)?;
                    let arch = &store.archetypes()[arch_idx as usize];
                    if arch.key & slot.bit() == 0 {
                        return Ok(LuaValue::Nil);
                    }
                    if slot.kind == Kind::Tag {
                        return Ok(LuaValue::Boolean(true));
                    }
                    let value = store.value_of(id, slot.comp).map_err(sim_to_lua)?;
                    Ok(match (slot.kind, value) {
                        (Kind::I64 | Kind::U8, ValueRef::I64(raw)) => LuaValue::Integer(raw),
                        (Kind::U64, ValueRef::U64(raw)) => {
                            let raw = i64::try_from(raw).map_err(|_| {
                                sim_to_lua(SimError::fixed(format!(
                                    "组件 `{}` 的 u64 值 {raw} 超出 Lua 整数范围",
                                    slot.name
                                )))
                            })?;
                            LuaValue::Integer(raw)
                        }
                        (Kind::Bytes, ValueRef::Bytes(bytes)) => {
                            LuaValue::String(lua.create_string(bytes)?)
                        }
                        (kind, value) => {
                            return Err(sim_to_lua(SimError::state(format!(
                                "组件 `{}` 声明为 {} 但与列里的值形状不符（{:?}）",
                                slot.name,
                                kind.label(),
                                value
                            ))))
                        }
                    })
                })
            },
        )?,
    )?;

    // ---- world.remove(id, { "hp" }) -> n ----
    //
    // 摘列的实现在内核（`ArchetypeStore::remove_components`）：`by_key` / `entity_index`
    // 是私有派生面，摘掉一列必须重建它们，而唯一的重建口是 `from_parts`——那是内核语义，
    // 不是本面的决定。本面只做「名字 → 组件 id」与批量写面的转译，校验与落笔全在
    // [`crate::world_ops`]（与宿主 `submit` 面同一份实现）。
    let st = Rc::clone(state);
    world.raw_set(
        "remove",
        lua.create_function(
            move |_, (id, names): (LuaValue, Table)| -> Result<i64, Error> {
                let id = entity_of(id, "world.remove")?;
                with_sim_mut(&st, "world.remove", |sim| {
                    let cids: Vec<ComponentId> = {
                        let world = st.borrow();
                        slots_of(sim.world().registry(), &world.world, &names, "world.remove")?
                            .iter()
                            .map(|slot| slot.comp)
                            .collect()
                    };
                    let report = world_ops::apply_ops(
                        sim,
                        &[WorldOp::Remove {
                            ids: vec![id],
                            cids,
                        }],
                    )
                    .map_err(step_to_lua)?;
                    Ok(i64::try_from(report.writes).unwrap_or(i64::MAX))
                })
            },
        )?,
    )?;

    // ---- world.each / world.each_mut / world.ids ----
    let st = Rc::clone(state);
    world.raw_set(
        "each",
        lua.create_function(
            move |lua, (names, f): (Table, Function)| -> Result<i64, Error> {
                top_each(lua, &st, &names, f, false)
            },
        )?,
    )?;
    let st = Rc::clone(state);
    world.raw_set(
        "each_mut",
        lua.create_function(
            move |lua, (names, f): (Table, Function)| -> Result<i64, Error> {
                top_each(lua, &st, &names, f, true)
            },
        )?,
    )?;
    let st = Rc::clone(state);
    world.raw_set(
        "ids",
        lua.create_function(move |lua, names: Table| -> Result<Table, Error> {
            with_sim_mut(&st, "world.ids", |sim| {
                let slots = {
                    let world = st.borrow();
                    slots_of(sim.world().registry(), &world.world, &names, "world.ids")?
                };
                let comps: Vec<ComponentId> = slots.iter().map(|s| s.comp).collect();
                let mut ids: Vec<EntityId> = Vec::new();
                sim.world().collect_ids(&comps, &mut ids);
                let out = lua.create_table_with_capacity(ids.len(), 0)?;
                for (i, id) in ids.iter().enumerate() {
                    out.raw_set(i + 1, entity_to_lua(*id)?)?;
                }
                Ok(out)
            })
        })?,
    )?;

    // ---- world.tick / world.rng ----
    let st = Rc::clone(state);
    world.raw_set(
        "tick",
        lua.create_function(move |_, ()| -> Result<i64, Error> {
            with_sim_mut(&st, "world.tick", |sim| {
                Ok(i64::try_from(sim.tick()).unwrap_or(i64::MAX))
            })
        })?,
    )?;
    let st = Rc::clone(state);
    world.raw_set(
        "rng",
        lua.create_function(move |_, stream: String| -> Result<i64, Error> {
            with_sim_mut(&st, "world.rng", |sim| Ok(i64::from(sim.rng32(&stream))))
        })?,
    )?;

    // ---- world.isqrt（定点平方根：定量距离比较的唯一入口）----
    //
    // 为什么在 world 面而不是 `math.sqrt`：定量规则必须留在 Q32.32 定点域
    // （D-B5）。内核里唯一可以接受的平方根是定点逐位法（`ygg_sim::fixed::isqrt`，
    // 只有移位/比较/减法，native/wasm 位一致），它的结果还是定点值，于是
    // 「比距离」与「算坐标」永远在同一个数域里。浮点 sqrt 不进 `math`。
    world.raw_set(
        "isqrt",
        lua.create_function(move |_, value: LuaValue| -> Result<i64, Error> {
            let x = sim_fixed_arg(value, "world.isqrt")?;
            ygg_sim::fixed::isqrt(x).map_err(sim_to_lua)
        })?,
    )?;

    // ---- world.log(fmt, ...) ----
    let log_format: Function = lua.load(LOG_FORMAT_SRC).eval()?;
    let log_tostring: Function = lua.globals().raw_get("tostring")?;
    let st = Rc::clone(state);
    world.raw_set(
        "log",
        lua.create_function(
            move |_, (fmt, rest): (LuaValue, MultiValue)| -> Result<(), Error> {
                let line = format_line(&log_format, &log_tostring, fmt, rest)?;
                st.borrow_mut().world.push_log(line);
                Ok(())
            },
        )?,
    )?;

    // ---- world.system{ ... } ----
    let st = Rc::clone(state);
    world.raw_set(
        "system",
        lua.create_function(move |lua, spec: Table| -> Result<(), Error> {
            let name: String = spec.raw_get("name")?;
            let stage_name: String = spec.raw_get("stage")?;
            let stage = match stage_name.as_str() {
                "pre" => Stage::Pre,
                "rules" => Stage::Rules,
                "post" => Stage::Post,
                other => {
                    return Err(sim_to_lua(SimError::schema(format!(
                        "系统 `{name}` 的 stage 必须是 pre|rules|post，得到 `{other}`"
                    ))))
                }
            };
            let run: Function = spec.raw_get("run")?;
            let reads = names_field(&spec, "reads")?;
            let writes = names_field(&spec, "writes")?;
            let after = names_field(&spec, "after")?;

            with_sim_mut(&st, "world.system", |sim| {
                let read_ids: Vec<ComponentId> = resolve_ids(sim, &reads, &st)?;
                let write_ids: Vec<ComponentId> = resolve_ids(sim, &writes, &st)?;
                let after_refs: Vec<&str> = after.iter().map(String::as_str).collect();
                let decl = SystemDecl::new(name.as_str())
                    .reads(&read_ids)
                    .writes(&write_ids)
                    .after(&after_refs);
                let lua_for_system = lua.clone();
                let st_for_system = Rc::clone(&st);
                let sys_name = name.clone();
                sim.add_system(decl, stage, move |ctx: &mut SystemCtx<'_>| {
                    run_system(&lua_for_system, &st_for_system, &run, &sys_name, ctx)
                })
                .map_err(sim_to_lua)
            })
        })?,
    )?;

    // ---- world.seal() ----
    let st = Rc::clone(state);
    world.raw_set(
        "seal",
        lua.create_function(move |_, ()| -> Result<(), Error> {
            with_sim_mut(&st, "world.seal", |sim| sim.seal().map_err(sim_to_lua))
        })?,
    )?;

    // ---- world.kv：与 `ygg.kv` **同一张表**（同一份实现，没有第二个 kv）----
    world.raw_set("kv", ygg.raw_get::<Table>("kv")?)?;

    lua.globals().raw_set("world", world)?;
    Ok(())
}
