use mlua::{Error, Lua, Value as LuaValue};
use ygg_core::entity::Value;

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
/// NaN / ±∞ 在此放行为 `Value::Float`，由调用点的 core 门拒绝（事件构造 /
/// KvStore::set / EntityStore::set_attribute 均带定位）。
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

// ============================================================================
// `ygg.entity.*` 批式面的形状校验（P-K A1；铁律②「数组进出、单次调用承载整批」）
// ============================================================================
