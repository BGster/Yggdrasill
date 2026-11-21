//! 实体即席状态写面:`ygg.entity.*` 注册(单面写/批式五面/控制两键/S24 查询薄壳)
//! 与实体错误 helper(S-25 批三自 host_api 单文件迁出)。
use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Error, Lua, Table, Value as LuaValue};
use ygg_core::entity::{EntityStore, Value};

use super::entity_decode::{entity_attr_ops, entity_fetch_queries, entity_rows, entity_tag_ops};
use super::selector::entity_query_selector;
use super::{value_from_lua, value_to_lua, HostState};

/// 形状不对（非数组 / 元素不是表 / 字段类型错 / 值非标量）。
pub const ENTITY_FRAME: &str = "YGG-ENTITY-FRAME";
/// 值不在值域（非有限浮点）。
pub const ENTITY_VALUE: &str = "YGG-ENTITY-VALUE";
/// 未知实体 id。
pub const ENTITY_ID: &str = "YGG-ENTITY-ID";

/// 取实体句柄（批式面的统一入口；`HostState` 借用只在调用瞬间）。
fn entities_of(state: &Rc<RefCell<HostState>>) -> Rc<RefCell<EntityStore>> {
    Rc::clone(&state.borrow().handles.entities)
}

pub(super) fn entity_frame(detail: impl std::fmt::Display) -> Error {
    Error::runtime(format!("{ENTITY_FRAME}: {detail}"))
}

pub(super) fn entity_value(detail: impl std::fmt::Display) -> Error {
    Error::runtime(format!("{ENTITY_VALUE}: {detail}"))
}

pub(super) fn entity_id_error(id: u64) -> Error {
    Error::runtime(format!("{ENTITY_ID}: 未知实体 id {id}"))
}

/// `has_tag` 的 store 层读取（store 无 has_tag 时经 Entity 只读面；miss = false）。
fn ent_get_tag(entities: &Rc<RefCell<EntityStore>>, id: u64, tag: &str) -> bool {
    entities
        .borrow()
        .get(id)
        .map(|e| e.has_tag(tag))
        .unwrap_or(false)
}

/// `ygg.entity.*` 注册(单面+批式五面+控制两键+S24 查询;原文迁入)。
pub(super) fn install_entity(
    lua: &Lua,
    state: &Rc<RefCell<HostState>>,
    ygg: &Table,
) -> Result<(), Error> {
    // ---- ygg.entity.*（即席状态写；get 类 miss = nil / false，写类 miss = 可读错误）----
    let entity_table = lua.create_table()?;
    let ent_spawn = Rc::clone(&state);
    entity_table.raw_set(
        "spawn",
        lua.create_function(move |_, ()| -> Result<i64, Error> {
            let entities = Rc::clone(&ent_spawn.borrow().handles.entities);
            let id = entities
                .borrow_mut()
                .spawn()
                .map_err(|e| Error::runtime(e.to_string()))?;
            i64::try_from(id).map_err(|_| Error::runtime("entity id overflow"))
        })?,
    )?;
    let ent_get = Rc::clone(&state);
    entity_table.raw_set(
        "get_attribute",
        lua.create_function(
            move |lua, (id, name): (u64, String)| -> Result<LuaValue, Error> {
                let entities = Rc::clone(&ent_get.borrow().handles.entities);
                let found = entities
                    .borrow()
                    .get(id)
                    .and_then(|entity| entity.attribute(&name).cloned());
                Ok(match found {
                    Some(v) => value_to_lua(lua, &v)?,
                    None => LuaValue::Nil,
                })
            },
        )?,
    )?;
    let ent_set = Rc::clone(&state);
    entity_table.raw_set(
        "set_attribute",
        lua.create_function(
            move |_, (id, name, payload): (u64, String, LuaValue)| -> Result<(), Error> {
                let value = value_from_lua(&payload)?;
                let entities = Rc::clone(&ent_set.borrow().handles.entities);
                let outcome = entities
                    .borrow_mut()
                    .set_attribute(id, name, value)
                    .map_err(|e| Error::runtime(e.to_string()));
                outcome
            },
        )?,
    )?;
    let ent_has = Rc::clone(&state);
    entity_table.raw_set(
        "has_tag",
        lua.create_function(move |_, (id, tag): (u64, String)| -> Result<bool, Error> {
            let entities = Rc::clone(&ent_has.borrow().handles.entities);
            Ok(ent_get_tag(&entities, id, &tag))
        })?,
    )?;
    let ent_add = Rc::clone(&state);
    entity_table.raw_set(
        "add_tag",
        lua.create_function(move |_, (id, tag): (u64, String)| -> Result<bool, Error> {
            let entities = Rc::clone(&ent_add.borrow().handles.entities);
            let changed = entities
                .borrow_mut()
                .add_tag(id, tag)
                .map_err(|e| Error::runtime(e.to_string()))?;
            Ok(changed)
        })?,
    )?;
    let ent_remove = Rc::clone(&state);
    entity_table.raw_set(
        "remove_tag",
        lua.create_function(move |_, (id, tag): (u64, String)| -> Result<bool, Error> {
            let entities = Rc::clone(&ent_remove.borrow().handles.entities);
            let removed = entities
                .borrow_mut()
                .remove_tag(id, &tag)
                .map_err(|e| Error::runtime(e.to_string()))?;
            Ok(removed)
        })?,
    )?;
    // ---- 批式五面（P-K A1；一个逐实体 getter 都没有）------------------------
    //
    // 入参是 Lua 数组（铁律② 原文「bytes/数组进出」）：n=1 的热路径不必付 pack 成本，
    // 测试夹具也不必写第二份编解码器。五面一律**先校验后改**。
    let ent_spawn_batch = Rc::clone(&state);
    entity_table.raw_set(
        "spawn_batch",
        lua.create_function(move |lua, rows: Table| -> Result<Table, Error> {
            let specs = entity_rows(&rows)?;
            let entities = Rc::clone(&ent_spawn_batch.borrow().handles.entities);
            let (ids, next_id) = {
                let mut store = entities.borrow_mut();
                let mut ids = Vec::with_capacity(specs.len());
                for spec in &specs {
                    let id = store.spawn().map_err(|e| Error::runtime(e.to_string()))?;
                    for (name, value) in &spec.attrs {
                        store
                            .set_attribute(id, name.clone(), value.clone())
                            .map_err(|e| Error::runtime(e.to_string()))?;
                    }
                    for tag in &spec.tags {
                        store
                            .add_tag(id, tag.clone())
                            .map_err(|e| Error::runtime(e.to_string()))?;
                    }
                    ids.push(id);
                }
                (ids, store.next_id())
            };
            let out = lua.create_table()?;
            out.raw_set("spawned", ids.len())?;
            out.raw_set("next_id", next_id)?;
            let list = lua.create_table()?;
            for (index, id) in ids.iter().enumerate() {
                list.raw_set(index + 1, *id)?;
            }
            out.raw_set("ids", list)?;
            Ok(out)
        })?,
    )?;

    let ent_attr_apply = Rc::clone(&state);
    entity_table.raw_set(
        "attr_apply",
        lua.create_function(move |lua, ops: Table| -> Result<Table, Error> {
            let parsed = entity_attr_ops(&ops)?;
            let entities = Rc::clone(&ent_attr_apply.borrow().handles.entities);
            let (applied, changed) = {
                let mut store = entities.borrow_mut();
                // 先校验后改：整批 id 全部在场，否则零改（错误带首个坏 id）。
                for (id, _) in &parsed {
                    if store.get(*id).is_none() {
                        return Err(entity_id_error(*id));
                    }
                }
                let mut applied = 0usize;
                let mut changed = 0usize;
                for (id, kvs) in &parsed {
                    for (name, value) in kvs {
                        // 「值是否真的变了」由内核判（含整/浮身份：`Value::Int(2) != Value::Float(2.0)`），
                        // Tier 2 据 `changed` 决定发不发事件（entity.md I6）。
                        let before = store.get(*id).and_then(|e| e.attribute(name)).cloned();
                        if before.as_ref() != Some(value) {
                            changed += 1;
                        }
                        store
                            .set_attribute(*id, name.clone(), value.clone())
                            .map_err(|e| Error::runtime(e.to_string()))?;
                        applied += 1;
                    }
                }
                (applied, changed)
            };
            let out = lua.create_table()?;
            out.raw_set("applied", applied)?;
            out.raw_set("changed", changed)?;
            Ok(out)
        })?,
    )?;

    let ent_attr_fetch = Rc::clone(&state);
    entity_table.raw_set(
        "attr_fetch",
        lua.create_function(move |lua, queries: Table| -> Result<Table, Error> {
            let parsed = entity_fetch_queries(&queries)?;
            // 先把数据拷出来再建表：不在 create_* 期间持 RefCell 借用（借用纪律同 ygg.sim）。
            let rows: Vec<(Vec<(String, Value)>, Vec<String>)> = {
                let store = entities_of(&ent_attr_fetch);
                let store = store.borrow();
                parsed
                    .iter()
                    .map(|(id, keys)| {
                        let entity = store.get(*id);
                        let values = keys
                            .iter()
                            .filter_map(|key| {
                                entity
                                    .and_then(|e| e.attribute(key))
                                    .map(|v| (key.clone(), v.clone()))
                            })
                            .collect::<Vec<_>>();
                        let tags = entity
                            .map(|e| e.tags().map(str::to_owned).collect::<Vec<_>>())
                            .unwrap_or_default();
                        (values, tags)
                    })
                    .collect()
            };
            let values_table = lua.create_table()?;
            let tags_table = lua.create_table()?;
            for (index, (values, tags)) in rows.iter().enumerate() {
                let row = lua.create_table()?;
                for (key, value) in values {
                    row.raw_set(key.as_str(), value_to_lua(lua, value)?)?;
                }
                values_table.raw_set(index + 1, row)?;
                let list = lua.create_table()?;
                for (position, tag) in tags.iter().enumerate() {
                    list.raw_set(position + 1, tag.as_str())?;
                }
                tags_table.raw_set(index + 1, list)?;
            }
            let out = lua.create_table()?;
            out.raw_set("values", values_table)?;
            out.raw_set("tags", tags_table)?;
            Ok(out)
        })?,
    )?;

    let ent_tag_batch = Rc::clone(&state);
    entity_table.raw_set(
        "tag_batch",
        lua.create_function(move |_, ops: Table| -> Result<usize, Error> {
            let parsed = entity_tag_ops(&ops)?;
            let entities = Rc::clone(&ent_tag_batch.borrow().handles.entities);
            let mut store = entities.borrow_mut();
            for (id, _, _) in &parsed {
                if store.get(*id).is_none() {
                    return Err(entity_id_error(*id));
                }
            }
            let mut changed = 0usize;
            for (id, tag, add) in &parsed {
                let outcome = if *add {
                    store.add_tag(*id, tag.clone())
                } else {
                    store.remove_tag(*id, tag.as_str())
                }
                .map_err(|e| Error::runtime(e.to_string()))?;
                if outcome {
                    changed += 1;
                }
            }
            Ok(changed)
        })?,
    )?;

    let ent_scan = Rc::clone(&state);
    entity_table.raw_set(
        "scan",
        lua.create_function(
            move |lua, (lo, hi, tag): (u64, u64, Option<String>)| -> Result<Table, Error> {
                if hi < lo {
                    return Err(entity_frame(format!("scan 的区间非法：lo={lo} > hi={hi}")));
                }
                let entities = Rc::clone(&ent_scan.borrow().handles.entities);
                // **吃调用方给的 [lo, hi]，绝不读 next_id**：上界是 Tier 2 的 `seq` 水位，
                // 用它挡「本包 before 出生 / 他处直接 spawn」的实体（entity.md §2 冷启动边界）。
                let ids: Vec<u64> = {
                    let store = entities.borrow();
                    store
                        .iter()
                        .filter(|e| e.id >= lo && e.id <= hi)
                        .filter(|e| match &tag {
                            Some(needle) => e.has_tag(needle.as_str()),
                            None => true,
                        })
                        .map(|e| e.id)
                        .collect()
                };
                let out = lua.create_table()?;
                let list = lua.create_table()?;
                for (index, id) in ids.iter().enumerate() {
                    list.raw_set(index + 1, *id)?;
                }
                out.raw_set("ids", list)?;
                Ok(out)
            },
        )?,
    )?;

    // ---- 控制面两键（S9）：存在性 + 真删 ------------------------------------
    //
    // 形状是「单标量进出」：各吃一个 u64、回一个 bool，n=1 热路径不必构造帧
    // （批式化它按 packages/entity.md §9.7 实测是负收益：表形 485 ns/8 值贵过
    // 8 次逐项穿越 ≈320 ns）。两键**不新增错误码**：三族（FRAME / VALUE / ID）
    // 只属批式面——批式拒绝是为了「整批零副作用」，单条删除没有半截状态，
    // 缺失 id 是正常答案（`false`），不是错误（契约：kernel/lua-api.md §3.1）。
    let ent_exists = Rc::clone(&state);
    entity_table.raw_set(
        "exists",
        lua.create_function(move |_, id: u64| -> Result<bool, Error> {
            let entities = entities_of(&ent_exists);
            let present = entities.borrow().contains(id);
            Ok(present)
        })?,
    )?;
    let ent_destroy = Rc::clone(&state);
    entity_table.raw_set(
        "destroy",
        lua.create_function(move |_, id: u64| -> Result<bool, Error> {
            let entities = entities_of(&ent_destroy);
            // 真删：属性 / tag / 记录一并回收；`next_id` 不动 ⇒ id 永不复用（I1）。
            let removed = entities.borrow_mut().remove(id).is_some();
            Ok(removed)
        })?,
    )?;

    // ---- 查询面（S24）：一次跨界、内核全扫（可走索引时走索引）、id 升序 --------
    //
    // 两阶段材质化（借用纪律同 `attr_fetch`）：阶段 1 持借用、零 Lua 调用（拷 id 与
    // 投影值的连续 Vec）；阶段 2 无借用、只建表。`count` 复用阶段 1 长度，不重数。
    let ent_query = Rc::clone(&state);
    entity_table.raw_set(
        "query",
        lua.create_function(move |lua, selector: Table| -> Result<Table, Error> {
            let selector = entity_query_selector(&selector)?;
            let entities = entities_of(&ent_query);
            let (count, ids, values) = {
                let mut store = entities.borrow_mut();
                let rows = store.select(&selector);
                let count = rows.len();
                let stride = selector.select.len();
                let mut ids: Vec<u64> = Vec::with_capacity(count);
                let mut values: Vec<Option<Value>> = Vec::with_capacity(count * stride);
                for entity in &rows {
                    ids.push(entity.id);
                    for key in &selector.select {
                        values.push(entity.attribute(key).cloned());
                    }
                }
                (count, ids, values)
            };
            let out = lua.create_table()?;
            out.raw_set("count", count)?;
            if selector.select.is_empty() {
                let list = lua.create_sequence_from(ids.iter().copied())?;
                out.raw_set("ids", list)?;
            } else {
                let stride = selector.select.len();
                let list = lua.create_table_with_capacity(count, 0)?;
                for (row, id) in ids.iter().enumerate() {
                    let record = lua.create_table_with_capacity(0, 1 + stride)?;
                    record.raw_set("id", *id)?;
                    for (slot, key) in selector.select.iter().enumerate() {
                        // 缺属性 ⇒ 行上**不设键**（`row.hp == nil`，与 `attr_fetch` 同一条纪律）
                        if let Some(value) = &values[row * stride + slot] {
                            record.raw_set(key.as_str(), value_to_lua(lua, value)?)?;
                        }
                    }
                    list.raw_set(row + 1, record)?;
                }
                out.raw_set("rows", list)?;
            }
            Ok(out)
        })?,
    )?;

    ygg.raw_set("entity", entity_table)?;
    Ok(())
}
