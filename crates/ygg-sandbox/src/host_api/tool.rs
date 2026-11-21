use std::collections::BTreeMap;

use super::lua_type_name;
use mlua::{Error, Table, Value as LuaValue};

/// `ygg.tool.call` 的工具 ID 字节上限。
pub(super) const TOOL_ID_MAX_BYTES: usize = 64;

/// 宿主侧同步工具执行器注入面（Mod 的 `ygg.tool.call(tool_id, args_table)` 桥接）。
pub trait HostToolRunner {
    fn call(
        &mut self,
        tool_id: &str,
        args: &BTreeMap<String, String>,
    ) -> Result<BTreeMap<String, String>, String>;
}

// ============================================================================
// §文件挂载桥的注入面（`ygg.fs.*` 的 `mounts/` 根）
// ============================================================================

/// `ygg.tool.call` 的可选参数表 → `BTreeMap<String, String>`（标量转字符串）。
pub(super) fn tool_args_from_lua(
    value: Option<&LuaValue>,
) -> Result<BTreeMap<String, String>, Error> {
    let mut out = BTreeMap::new();
    let Some(val) = value else {
        return Ok(out);
    };
    let table = match val {
        LuaValue::Nil => return Ok(out),
        LuaValue::Table(t) => t,
        other => {
            return Err(Error::runtime(format!(
                "ygg.tool.call args must be a table or nil, got {}",
                lua_type_name(other)
            )))
        }
    };
    for pair in table.pairs::<LuaValue, LuaValue>() {
        let (k, v) = pair?;
        let key = match &k {
            LuaValue::String(s) => s.to_str()?.to_owned(),
            LuaValue::Integer(i) => i.to_string(),
            other => {
                return Err(Error::runtime(format!(
                    "ygg.tool.call args key must be a string, got {}",
                    lua_type_name(other)
                )))
            }
        };
        let val_str = match &v {
            LuaValue::String(s) => s.to_str()?.to_owned(),
            LuaValue::Integer(i) => i.to_string(),
            LuaValue::Number(n) => {
                if !n.is_finite() {
                    return Err(Error::runtime("ygg.tool.call args value must be finite"));
                }
                n.to_string()
            }
            LuaValue::Boolean(b) => b.to_string(),
            LuaValue::Nil => continue,
            other => {
                return Err(Error::runtime(format!(
                    "ygg.tool.call args[`{key}`] must be a scalar, got {}",
                    lua_type_name(other)
                )))
            }
        };
        out.insert(key, val_str);
    }
    Ok(out)
}
