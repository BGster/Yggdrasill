//! Lua ↔ core 的标量闸门：payload 双向 marshal、定点闸门与两面共用的拒绝码。
//!
//! M1-D 把第二套实体模型（`ygg-core` 行存 + 选择器 DSL）连同它的 Lua 面
//! （`ygg` 的 `entity.*` / `sim.*` 两面）一起删掉；M1-E 再把事件层（`ygg` 的 `narrate` /
//! `ygg` 的 `choice`）连同选项序列校验一起删掉。留下来的是 `ygg.kv` / `world.*` 两面
//! 共用的部分：它们约束的是**值进出的形状**，与世界怎么存、时间怎么走无关。
//! 于是从原来的 `value.rs`（实体批式形状校验的宿主）与 `sim.rs`（定点闸门的宿主）
//! 收拢到本文件，只留活下来的部分。
use mlua::{Error, Lua, Value as LuaValue};
use ygg_core::value::Value;

/// 未挂载模拟内核时的统一码（码在首段，与 `YGG-FS-UNMOUNTED` 同一处置）。
pub const SIM_UNMOUNTED: &str = "YGG-SIM-UNMOUNTED";
/// 定点闸门的拒绝码（非整数值过面 / 除零）。
pub const SIM_FIXED: &str = "YGG-SIM-FIXED";

/// 未挂载时的统一表述（`why` 是能力面名，如 `world.spawn`）。
pub(super) fn sim_unmounted(what: &str) -> String {
    format!(
        "{}: 宿主未注入模拟内核，`{what}` 不可用（注入面见 `Loaded::set_sim`）",
        SIM_UNMOUNTED
    )
}

/// 定点闸门：Lua number → `i64`，**只接受值上等于整数**的数。
///
/// 这是 Tier 2 双精度与 Tier 1 定点之间唯一的闸门：静默取整会把「调用方算错了」变成
/// 「内核算错了」，而后者无法在回放证据里查出来。上限 2^53 是双精度可精确表示的整数边界
/// （超过它「值上等于整数」这个判断本身就不成立了）。
pub(super) fn sim_fixed_arg(value: LuaValue, what: &str) -> Result<i64, Error> {
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

/// mlua `Value` 的 Lua 类型名（可读报错用）。
pub(super) fn lua_type_name(value: &LuaValue) -> &'static str {
    match value {
        LuaValue::Nil => "nil",
        LuaValue::Boolean(_) => "boolean",
        LuaValue::LightUserData(_) => "light userdata",
        LuaValue::Integer(_) => "integer",
        LuaValue::Number(_) => "number",
        LuaValue::String(_) => "string",
        LuaValue::Table(_) => "table",
        LuaValue::Function(_) => "function",
        LuaValue::Thread(_) => "thread",
        LuaValue::UserData(_) => "userdata",
        _ => "unknown",
    }
}

/// core `Value` → mlua `Value`（core→Lua 方向；只映射四个标量）。
pub(super) fn value_to_lua(lua: &Lua, value: &Value) -> Result<LuaValue, Error> {
    Ok(match value {
        Value::Int(v) => LuaValue::Integer(*v),
        Value::Float(v) => LuaValue::Number(*v),
        Value::Bool(v) => LuaValue::Boolean(*v),
        Value::Str(v) => LuaValue::String(lua.create_string(v)?),
    })
}

/// mlua `Value` → core `Value`（Lua→core 方向）：四个标量直通；
/// table / function / 其他 → 可读 `unsupported payload type` 拒绝。
/// NaN / ±∞ 在此放行为 `Value::Float`，由调用点（`KvStore::set`）自己拒绝。
pub(super) fn value_from_lua(value: &LuaValue) -> Result<Value, Error> {
    Ok(match value {
        LuaValue::Integer(v) => Value::Int(*v),
        LuaValue::Number(v) => Value::Float(*v),
        LuaValue::Boolean(v) => Value::Bool(*v),
        LuaValue::String(v) => Value::Str(v.to_str()?.to_owned()),
        other => {
            return Err(Error::runtime(format!(
                "unsupported payload type: {}",
                lua_type_name(other)
            )))
        }
    })
}
