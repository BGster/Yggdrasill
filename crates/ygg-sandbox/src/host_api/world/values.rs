use mlua::{Error, Lua, Table, Value as LuaValue};
use ygg_sim::archetype::{Column, ValueRef};
use ygg_sim::component::{ComponentId, EntityId, Registry};
use ygg_sim::error::SimError;
use ygg_sim::query::RowSet;

use super::columns::{slot_of, Kind, Slot, WorldState};
use super::WORLD_BUSY;
use crate::host_api::marshal::{sim_fixed_arg, sim_unmounted};
use crate::sandbox::StepError;

pub(super) fn world_busy(what: &str) -> Error {
    Error::runtime(format!(
        "{WORLD_BUSY}: `{what}` 在系统执行期间被调用（内核槽已被 `step` 独占借用）；\
         系统体内请用 `run(dt, ctx)` 的 `ctx.*`"
    ))
}

pub(super) fn world_unmounted(what: &str) -> Error {
    Error::runtime(sim_unmounted(what))
}

/// `SimError` → Lua 错误：**码在首段**（与 `ygg` 的 `sim.*` 同一口径，外部可 grep 可断言）。
pub(super) fn sim_to_lua(e: SimError) -> Error {
    Error::runtime(e.to_string())
}

/// `StepError` → Lua 错误：内核码（`YGG-SIM-*`）原样上浮，本面自有码（`E_STEP_*`）也带首段。
pub(super) fn step_to_lua(e: StepError) -> Error {
    Error::runtime(e.to_string())
}

// ---------------------------------------------------------------------------
// 值编解码（四个 kind 一一对应；浮点无入口）
// ---------------------------------------------------------------------------

/// Lua 整数 → `i64`（过定点闸门：非整数值的 `number` 一律 `YGG-SIM-FIXED`）。
pub(super) fn int_of(slot: &Slot, value: &LuaValue) -> Result<i64, Error> {
    let raw = sim_fixed_arg(value.clone(), &slot.name)?;
    match slot.kind {
        Kind::U8 if !(0..=255).contains(&raw) => Err(sim_to_lua(SimError::fixed(format!(
            "组件 `{}` 是 u8（0..=255），得到 {raw}",
            slot.name
        )))),
        Kind::U64 if raw < 0 => Err(sim_to_lua(SimError::fixed(format!(
            "组件 `{}` 是 u64（0..=i64::MAX），得到 {raw}",
            slot.name
        )))),
        _ => Ok(raw),
    }
}

/// 单元格 Lua 值 → 一行的列载荷（`ValueRef` + 以它建列）。
fn column_of(slot: &Slot, value: &LuaValue) -> Result<Column, Error> {
    Ok(match slot.kind {
        Kind::I64 | Kind::U8 => {
            let raw = int_of(slot, value)?;
            let mut column = Column::I64(Vec::with_capacity(1));
            column
                .push_value(&ValueRef::I64(raw), slot.comp)
                .map_err(sim_to_lua)?;
            column
        }
        Kind::U64 => {
            let raw = int_of(slot, value)?;
            let raw = u64::try_from(raw).map_err(|_| {
                sim_to_lua(SimError::fixed(format!(
                    "组件 `{}` 是 u64，得到 {raw}",
                    slot.name
                )))
            })?;
            let mut column = Column::U64(Vec::with_capacity(1));
            column
                .push_value(&ValueRef::U64(raw), slot.comp)
                .map_err(sim_to_lua)?;
            column
        }
        Kind::Bytes => {
            let LuaValue::String(text) = value else {
                return Err(sim_to_lua(SimError::schema(format!(
                    "组件 `{}` 是 bytes（字符串），得到 {}",
                    slot.name,
                    value.type_name()
                ))));
            };
            let mut column = Column::Bytes {
                off: vec![0],
                buf: Vec::new(),
            };
            let bytes = text.as_bytes();
            column
                .push_value(&ValueRef::Bytes(&bytes), slot.comp)
                .map_err(sim_to_lua)?;
            column
        }
        Kind::Tag => {
            return Err(sim_to_lua(SimError::schema(format!(
                "组件 `{}` 是零宽 tag（没有列），不能带值；用 `{{ {} = true }}` 表示成员",
                slot.name, slot.name
            ))))
        }
    })
}

/// 实体 id → Lua 整数（越界即错：id 是 `u64`，Lua 5.4 整数是 `i64`）。
pub(super) fn entity_to_lua(id: EntityId) -> Result<i64, Error> {
    i64::try_from(id)
        .map_err(|_| sim_to_lua(SimError::entity(format!("实体 id {id} 超出 Lua 整数范围"))))
}

/// Lua 值 → 实体 id（≥ 1 的整数；不静默取整）。
pub(super) fn entity_of(value: LuaValue, what: &str) -> Result<EntityId, Error> {
    let raw = sim_fixed_arg(value, what)?;
    u64::try_from(raw)
        .ok()
        .filter(|id| *id != 0)
        .ok_or_else(|| {
            sim_to_lua(SimError::entity(format!(
                "`{what}` 必须是实体 id（≥ 1 的整数），得到 {raw}"
            )))
        })
}

/// 一列的一行 → Lua 值。
pub(super) fn cell_to_lua(
    lua: &Lua,
    slot: &Slot,
    rs: &RowSet<'_>,
    k: u8,
) -> Result<LuaValue, SimError> {
    let missing = || {
        SimError::state(format!(
            "组件 `{}` 在本次遍历的桶里读不到值（列型与声明不符）",
            slot.name
        ))
    };
    Ok(match slot.kind {
        Kind::Tag => LuaValue::Boolean(true),
        Kind::I64 | Kind::U8 => LuaValue::Integer(rs.fixed(k).ok_or_else(missing)?),
        Kind::U64 => {
            let raw = rs.u64v(k).ok_or_else(missing)?;
            let raw = i64::try_from(raw).map_err(|_| {
                SimError::fixed(format!(
                    "组件 `{}` 的 u64 值 {raw} 超出 Lua 整数范围",
                    slot.name
                ))
            })?;
            LuaValue::Integer(raw)
        }
        Kind::Bytes => {
            let bytes = rs.bytes(k).ok_or_else(missing)?;
            LuaValue::String(
                lua.create_string(bytes)
                    .map_err(|e| SimError::state(format!("字符串进 Lua 失败：{e}")))?,
            )
        }
    })
}

/// 载荷表的落地形态：（有列组件的列（值待填）, 要挂的零宽成员）。
///
/// 列按**组件 id 升序**——载荷表的书写序不参与（Lua 表无序，序必须来自注册表）。
type Payload = (Vec<(ComponentId, Column)>, Vec<ComponentId>);

/// 载荷表 →（有列组件的列, 零宽成员）；列按**组件 id 升序**（确定序，载荷书写序不参与）。
pub(super) fn payload_of(
    registry: &Registry,
    world: &WorldState,
    payload: &Table,
    what: &str,
    allow_tags: bool,
) -> Result<Payload, Error> {
    let mut entries: Vec<(ComponentId, Slot, LuaValue)> = Vec::new();
    for pair in payload.pairs::<LuaValue, LuaValue>() {
        let (key, value) = pair?;
        let name = match key {
            LuaValue::String(s) => s.to_str()?.to_owned(),
            other => {
                return Err(sim_to_lua(SimError::schema(format!(
                    "`{what}` 的载荷键必须是组件名（字符串），遇到 {}",
                    other.type_name()
                ))))
            }
        };
        let slot = slot_of(registry, world, &name)?;
        entries.push((slot.comp, slot, value));
    }
    if entries.is_empty() {
        return Err(sim_to_lua(SimError::schema(format!(
            "`{what}` 的载荷表不得为空"
        ))));
    }
    entries.sort_by_key(|(comp, ..)| *comp);

    let mut cols: Vec<(ComponentId, Column)> = Vec::with_capacity(entries.len());
    let mut tags: Vec<ComponentId> = Vec::new();
    for (comp, slot, value) in entries {
        if slot.kind == Kind::Tag {
            if !allow_tags {
                return Err(sim_to_lua(SimError::schema(format!(
                    "`{what}` 不能改零宽成员 `{}`（tag 没有列；用 world.insert / world.remove）",
                    slot.name
                ))));
            }
            if !matches!(value, LuaValue::Boolean(true)) {
                return Err(sim_to_lua(SimError::schema(format!(
                    "零宽成员 `{}` 的值必须是 `true`，得到 {}",
                    slot.name,
                    value.type_name()
                ))));
            }
            tags.push(comp);
            continue;
        }
        cols.push((comp, column_of(&slot, &value)?));
    }
    Ok((cols, tags))
}
