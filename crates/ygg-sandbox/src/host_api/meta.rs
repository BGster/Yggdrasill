use super::lua_type_name;
use super::value_from_lua;
use mlua::{Error, Lua, Table, Value as LuaValue};
use std::cell::RefCell;
use std::rc::Rc;

use super::HostState;
use serde_json::{Map as JsonMap, Value as MetaValue};
use ygg_core::entity::{normalize_f64, Value};

/// `ygg.meta` 键长上限（字节）。键是**回执字段名**，不是数据面通道——
/// 上限防「把人读内容塞键名」的滥用回流。
const META_KEY_MAX_BYTES: usize = 64;

/// `ygg.meta` 值嵌套深度上限（含顶层表：1 层）。超限可读报错而非爆栈。
const META_MAX_DEPTH: usize = 8;

/// `ygg.meta` 的键校验：必须是长度 1..=`META_KEY_MAX_BYTES` 的字符串
/// （非字符串 / 空串 / 超长均为可读报错；键是回执字段名，不承载数据）。
pub(super) fn meta_key_from_lua(key: &LuaValue) -> Result<String, Error> {
    let name = match key {
        LuaValue::String(s) => s.to_str()?.to_owned(),
        other => {
            return Err(Error::runtime(format!(
                "ygg.meta key must be a string, got {}",
                lua_type_name(other)
            )))
        }
    };
    if name.is_empty() {
        return Err(Error::runtime("ygg.meta key must not be empty"));
    }
    if name.len() > META_KEY_MAX_BYTES {
        return Err(Error::runtime(format!(
            "ygg.meta key `{name}` is too long: {} bytes exceeds limit {META_KEY_MAX_BYTES}",
            name.len()
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(Error::runtime(format!(
            "ygg.meta key `{name}` must not contain control characters"
        )));
    }
    Ok(name)
}

/// `ygg.meta` 的值转换：标量走 [`value_from_lua`]（同一套四标量校验），
/// 表递归成 JSON 序列/对象（深度封顶 [`META_MAX_DEPTH`]）。
/// 数值纪律与 core 写入门同源：非有限值拒绝（`-0.0` 经 `normalize_f64`
/// 规范化——meta 是回执面，跨进程字节稳定由规范形承担）。
pub(super) fn meta_value_from_lua(value: &LuaValue, depth: usize) -> Result<MetaValue, Error> {
    if let LuaValue::Table(table) = value {
        if depth >= META_MAX_DEPTH {
            return Err(Error::runtime(format!(
                "ygg.meta value nesting exceeds {META_MAX_DEPTH} levels"
            )));
        }
        return meta_table_to_json(table, depth + 1);
    }
    meta_scalar_from_lua(value)
}

/// 标量 leaf：`value_from_lua` 把关类型，此处补浮点纪律（非有限值拒绝 +
/// 规范形）。
fn meta_scalar_from_lua(value: &LuaValue) -> Result<MetaValue, Error> {
    Ok(match value_from_lua(value)? {
        Value::Int(v) => MetaValue::from(v),
        Value::Float(v) => {
            if !v.is_finite() {
                return Err(Error::runtime(
                    "ygg.meta value is non-finite (NaN / +inf / -inf are rejected)",
                ));
            }
            match serde_json::Number::from_f64(normalize_f64(v)) {
                Some(number) => MetaValue::Number(number),
                None => {
                    return Err(Error::runtime(
                        "ygg.meta value is non-finite (NaN / +inf / -inf are rejected)",
                    ))
                }
            }
        }
        Value::Bool(v) => MetaValue::Bool(v),
        Value::Str(v) => MetaValue::String(v),
    })
}

/// Lua 表 → JSON：稠密 `1..n` 数值键 = 序列；纯字符串键 = 对象；
/// 混合 / 稀疏数值键 / 其他键型都是可读报错（**不做静默猜测**：
/// 猜测会让同一 Mod 在不同输入下产出不同形状的回执）。
///
/// **空表 = 空序列 `[]`**：Lua 无法区分空表 / 空序列，取「空列表」这一更常用
/// 语义（`conflicts = {}` 读成 `[]`）；需要空对象时给一个键（或读侧按缺键处理）。
fn meta_table_to_json(table: &Table, depth: usize) -> Result<MetaValue, Error> {
    let len = table.raw_len();
    let mut int_keys: Vec<i64> = Vec::new();
    let mut str_keys: Vec<String> = Vec::new();
    for pair in table.pairs::<LuaValue, LuaValue>() {
        let (key, _) = pair?;
        match &key {
            LuaValue::Integer(i) => int_keys.push(*i),
            LuaValue::Number(n) if n.fract() == 0.0 && n.is_finite() => {
                int_keys.push(*n as i64);
            }
            LuaValue::String(s) => str_keys.push(s.to_str()?.to_owned()),
            other => {
                return Err(Error::runtime(format!(
                    "ygg.meta table keys must be integers or strings, got {}",
                    lua_type_name(other)
                )))
            }
        }
    }
    if str_keys.is_empty() {
        int_keys.sort_unstable();
        let dense = int_keys.len() == len
            && int_keys
                .iter()
                .enumerate()
                .all(|(position, key)| *key == position as i64 + 1);
        if !dense {
            return Err(Error::runtime(
                "ygg.meta table is not a dense 1..n sequence (numeric keys must be exactly 1..n)",
            ));
        }
        let mut out = Vec::with_capacity(len);
        for index in 1..=len {
            let item: LuaValue = table.raw_get(index)?;
            out.push(meta_value_from_lua(&item, depth)?);
        }
        return Ok(MetaValue::Array(out));
    }
    if !int_keys.is_empty() {
        return Err(Error::runtime(
            "ygg.meta table mixes array indices and string keys",
        ));
    }
    let mut map = JsonMap::new();
    for key in str_keys {
        let item: LuaValue = table.raw_get(key.as_str())?;
        map.insert(key, meta_value_from_lua(&item, depth)?);
    }
    Ok(MetaValue::Object(map))
}

/// `ygg.meta` 注册(TD1;原文迁入)。
pub(super) fn install_meta(
    lua: &Lua,
    state: &Rc<RefCell<HostState>>,
    ygg: &Table,
) -> Result<(), Error> {
    // ---- ygg.meta（结构化回执面，TD1）：不进 calls 缓冲、不回放、不改 core
    //      trait；驱动转换结束后 take_meta 拼进信封。key 长度/类型与非有限值
    //      在调用点 fail-fast 拒绝（与 ygg.emit 同纪律）。----
    let st = Rc::clone(state);
    ygg.raw_set(
        "meta",
        lua.create_function(
            move |_, (key, payload): (LuaValue, LuaValue)| -> Result<(), Error> {
                let name = meta_key_from_lua(&key)?;
                let value = meta_value_from_lua(&payload, 0)?;
                st.borrow_mut().meta.insert(name, value);
                Ok(())
            },
        )?,
    )?;

    Ok(())
}
