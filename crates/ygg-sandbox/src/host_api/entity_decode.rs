use super::lua_type_name;
use super::value_from_lua;
use super::{entity_frame, entity_id_error, entity_value};
use mlua::{Error, Table, Value as LuaValue};
use ygg_core::entity::Value;

/// 一个待造的行：属性袋 + tag 名单（都是可选的）。
#[derive(Clone, Debug, Default)]
pub struct EntityRowSpec {
    pub attrs: Vec<(String, Value)>,
    pub tags: Vec<String>,
}

/// 值域闸门：标量四型直通，非有限浮点拒（`EntityStore::set_attribute` 同样拒，
/// 但这一层先拒 = 整批零副作用）。
pub(super) fn entity_value_arg(value: &LuaValue, what: &str) -> Result<Value, Error> {
    let value = value_from_lua(value).map_err(|_| {
        entity_value(format!(
            "`{what}` 只接受标量（boolean / number / string），得到 {}",
            lua_type_name(value)
        ))
    })?;
    if let Value::Float(f) = &value {
        if !f.is_finite() {
            return Err(entity_value(format!("`{what}` 不接受 NaN/±∞")));
        }
    }
    Ok(value)
}

/// 稠密数组 → Vec（非表即拒；`raw_len` 为 0 的表按空批处理）。
pub(super) fn entity_array(table: &Table, what: &str) -> Result<Vec<LuaValue>, Error> {
    let len = table.raw_len();
    let mut out = Vec::with_capacity(len);
    for index in 1..=len {
        out.push(
            table
                .raw_get::<LuaValue>(index)
                .map_err(|_| entity_frame(format!("`{what}` 的第 {index} 项不可读")))?,
        );
    }
    Ok(out)
}

fn entity_field(row: &Table, key: &str) -> Result<Option<Table>, Error> {
    match row
        .raw_get::<LuaValue>(key)
        .map_err(|_| entity_frame("字段不可读"))?
    {
        LuaValue::Nil => Ok(None),
        LuaValue::Table(t) => Ok(Some(t)),
        other => Err(entity_frame(format!(
            "`{key}` 必须是表或 nil，得到 {}",
            lua_type_name(&other)
        ))),
    }
}

/// `spawn_batch(rows)`：每个元素 `{ attrs = {k = v}, tags = {t, …} }`。
pub fn entity_rows(rows: &Table) -> Result<Vec<EntityRowSpec>, Error> {
    let mut out = Vec::new();
    for (index, item) in entity_array(rows, "rows")?.iter().enumerate() {
        let LuaValue::Table(row) = item else {
            return Err(entity_frame(format!(
                "rows[{index}] 必须是表，得到 {}",
                lua_type_name(item)
            )));
        };
        let mut spec = EntityRowSpec::default();
        if let Some(attrs) = entity_field(row, "attrs")? {
            for pair in attrs.clone().pairs::<LuaValue, LuaValue>() {
                let (key, value) = pair.map_err(|_| entity_frame("attrs 的键值对不可读"))?;
                let LuaValue::String(key) = key else {
                    return Err(entity_frame("attrs 的键必须是字符串"));
                };
                let key = key
                    .to_str()
                    .map_err(|_| entity_frame("attrs 的键非 UTF-8"))?;
                spec.attrs
                    .push((key.to_string(), entity_value_arg(&value, key.as_ref())?));
            }
        }
        if let Some(tags) = entity_field(row, "tags")? {
            for item in entity_array(&tags, "tags")? {
                let LuaValue::String(tag) = item else {
                    return Err(entity_frame("tags 的元素必须是字符串"));
                };
                spec.tags.push(
                    tag.to_str()
                        .map_err(|_| entity_frame("tag 非 UTF-8"))?
                        .to_string(),
                );
            }
        }
        out.push(spec);
    }
    Ok(out)
}

/// `attr_apply(ops)`：每个元素 `{ id = n, attrs = {k = v} }`。
pub fn entity_attr_ops(ops: &Table) -> Result<Vec<(u64, Vec<(String, Value)>)>, Error> {
    let mut out = Vec::new();
    for (index, item) in entity_array(ops, "ops")?.iter().enumerate() {
        let LuaValue::Table(op) = item else {
            return Err(entity_frame(format!(
                "ops[{index}] 必须是表，得到 {}",
                lua_type_name(item)
            )));
        };
        let id = match op.raw_get::<LuaValue>("id") {
            Ok(LuaValue::Integer(v)) if v >= 0 => v as u64,
            _ => return Err(entity_frame(format!("ops[{index}].id 必须是非负整数"))),
        };
        let mut kvs = Vec::new();
        if let Some(attrs) = entity_field(op, "attrs")? {
            for pair in attrs.clone().pairs::<LuaValue, LuaValue>() {
                let (key, value) = pair.map_err(|_| entity_frame("attrs 的键值对不可读"))?;
                let LuaValue::String(key) = key else {
                    return Err(entity_frame("attrs 的键必须是字符串"));
                };
                let key = key
                    .to_str()
                    .map_err(|_| entity_frame("attrs 的键非 UTF-8"))?;
                kvs.push((key.to_string(), entity_value_arg(&value, key.as_ref())?));
            }
        }
        out.push((id, kvs));
    }
    Ok(out)
}

/// `attr_fetch(queries)`：每个元素 `{ id = n, keys = {k, …} }`。
pub fn entity_fetch_queries(queries: &Table) -> Result<Vec<(u64, Vec<String>)>, Error> {
    let mut out = Vec::new();
    for (index, item) in entity_array(queries, "queries")?.iter().enumerate() {
        let LuaValue::Table(query) = item else {
            return Err(entity_frame(format!(
                "queries[{index}] 必须是表，得到 {}",
                lua_type_name(item)
            )));
        };
        let id = match query.raw_get::<LuaValue>("id") {
            Ok(LuaValue::Integer(v)) if v >= 0 => v as u64,
            _ => return Err(entity_frame(format!("queries[{index}].id 必须是非负整数"))),
        };
        let mut keys = Vec::new();
        if let Some(list) = entity_field(query, "keys")? {
            for item in entity_array(&list, "keys")? {
                let LuaValue::String(key) = item else {
                    return Err(entity_frame("keys 的元素必须是字符串"));
                };
                keys.push(
                    key.to_str()
                        .map_err(|_| entity_frame("key 非 UTF-8"))?
                        .to_string(),
                );
            }
        }
        out.push((id, keys));
    }
    Ok(out)
}

// ============================================================================
// `ygg.entity.query` 的文法解码（S24 v2 §1；**先校验后扫**）
// ============================================================================
//
// 语义不在这里：选择器 / 比较矩阵 / 索引判定都在 `ygg_core::entity`。本区只把
// Lua 表解码成 [`Selector`]，并把两态书写（字符串态糖 + 结构态逃生口）的**每一种**
// 越界形态拒成 `YGG-ENTITY-FRAME`（值域错仍是 `YGG-ENTITY-VALUE`，沿用 A1 两族）。

/// `tag_batch(ops)`：每个元素 `{ id = n, tag = "t", add = true|false }`。
pub fn entity_tag_ops(ops: &Table) -> Result<Vec<(u64, String, bool)>, Error> {
    let mut out = Vec::new();
    for (index, item) in entity_array(ops, "ops")?.iter().enumerate() {
        let LuaValue::Table(op) = item else {
            return Err(entity_frame(format!(
                "ops[{index}] 必须是表，得到 {}",
                lua_type_name(item)
            )));
        };
        let id = match op.raw_get::<LuaValue>("id") {
            Ok(LuaValue::Integer(v)) if v >= 0 => v as u64,
            _ => return Err(entity_frame(format!("ops[{index}].id 必须是非负整数"))),
        };
        let tag = match op.raw_get::<LuaValue>("tag") {
            Ok(LuaValue::String(tag)) => tag
                .to_str()
                .map_err(|_| entity_frame("tag 非 UTF-8"))?
                .to_string(),
            _ => return Err(entity_frame(format!("ops[{index}].tag 必须是字符串"))),
        };
        let add = match op.raw_get::<LuaValue>("add") {
            Ok(LuaValue::Boolean(v)) => v,
            _ => return Err(entity_frame(format!("ops[{index}].add 必须是布尔"))),
        };
        out.push((id, tag, add));
    }
    Ok(out)
}

/// options 必须是字符串的 Lua 序列（数组形态），否则可读报错。
pub(super) fn options_from_lua(options: &LuaValue) -> Result<Vec<String>, Error> {
    let table = match options {
        LuaValue::Table(t) => t,
        other => {
            return Err(Error::runtime(format!(
                "ygg.choice options must be a sequence of strings, got {}",
                lua_type_name(other)
            )))
        }
    };
    let len = table.raw_len();
    let mut out = Vec::with_capacity(len);
    for i in 1..=len {
        let item: LuaValue = table.raw_get(i)?;
        match &item {
            LuaValue::String(s) => out.push(s.to_str()?.to_owned()),
            other => {
                return Err(Error::runtime(format!(
                    "ygg.choice options[{i}] must be a string, got {}",
                    lua_type_name(other)
                )))
            }
        }
    }
    Ok(out)
}
