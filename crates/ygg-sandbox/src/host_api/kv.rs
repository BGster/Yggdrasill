//! `ygg.kv.*` 即席状态写面:core 把关方法(`KvStore::get/set`),门拒绝为可读
//! mlua 错误(S-25 批三自 host_api 单文件迁出;原文居 install() 装配体)。
use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Error, Lua, Table, Value as LuaValue};

use super::{value_from_lua, value_to_lua, HostState};

/// `ygg.kv.*` 注册(即席状态写;原文迁入)。
pub(super) fn install_kv(
    lua: &Lua,
    state: &Rc<RefCell<HostState>>,
    ygg: &Table,
) -> Result<(), Error> {
    // ---- ygg.kv.*（即席状态写：core 把关方法，门拒绝为可读 mlua 错误；
    //      句柄经 HostState.handles 现取现用，字段即真实读取面）----
    let kv_table = lua.create_table()?;
    let kv_get = Rc::clone(state);
    kv_table.raw_set(
        "get",
        lua.create_function(move |lua, key: String| -> Result<LuaValue, Error> {
            let kv = Rc::clone(&kv_get.borrow().handles.kv);
            let found = kv.borrow().get(&key).cloned();
            Ok(match found {
                Some(v) => value_to_lua(lua, &v)?,
                None => LuaValue::Nil, // get-miss = nil（D-S9-2 marshal 规则）
            })
        })?,
    )?;
    let kv_set = Rc::clone(state);
    kv_table.raw_set(
        "set",
        lua.create_function(
            move |_, (key, payload): (String, LuaValue)| -> Result<(), Error> {
                let value = value_from_lua(&payload)?;
                let kv = Rc::clone(&kv_set.borrow().handles.kv);
                let outcome = kv
                    .borrow_mut()
                    .set(key.clone(), value)
                    .map_err(|e| Error::runtime(e.to_string()));
                outcome
            },
        )?,
    )?;
    ygg.raw_set("kv", kv_table)?;

    Ok(())
}
