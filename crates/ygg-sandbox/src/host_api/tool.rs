use std::collections::BTreeMap;

use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Error, Lua, Table, Value as LuaValue};

use super::lua_type_name;
use super::HostState;
use super::Phase;

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

/// `ygg.tool.*` 注册(同步宿主工具桥;原文迁入)。
pub(super) fn install_tool(
    lua: &Lua,
    state: &Rc<RefCell<HostState>>,
    ygg: &Table,
) -> Result<(), Error> {
    // ---- ygg.tool.*（同步宿主工具桥：装载期由影子表禁止，转换期按挂载态执行或回退）----
    let tool_table = lua.create_table()?;
    let tool_call = Rc::clone(state);
    tool_table.raw_set(
        "call",
        lua.create_function(
            move |lua, (tool_id, args_val): (String, Option<LuaValue>)| -> Result<Table, Error> {
                let trimmed = tool_id.trim();
                if trimmed.is_empty() || trimmed.len() > TOOL_ID_MAX_BYTES {
                    return Err(Error::runtime(format!(
                        "ygg.tool.call tool_id must be 1..={TOOL_ID_MAX_BYTES} bytes, got `{tool_id}`"
                    )));
                }
                let args = tool_args_from_lua(args_val.as_ref())?;
                let (phase, runner_rc) = {
                    let s = tool_call.borrow();
                    (s.phase, Rc::clone(&s.tool_runner))
                };
                if phase != Phase::Transition {
                    return Err(Error::runtime(format!(
                        "ygg.tool.call is only allowed during transition phase (current phase: {phase:?})"
                    )));
                }
                let mut runner_slot = runner_rc.borrow_mut();
                let out = lua.create_table()?;
                match runner_slot.as_mut() {
                    None => {
                        out.raw_set("ok", false)?;
                        out.raw_set("unmounted", true)?;
                        out.raw_set("code", -1)?;
                        out.raw_set("error", "no host tool runner mounted")?;
                    }
                    Some(runner) => match runner.call(trimmed, &args) {
                        Ok(data_map) => {
                            let data_table = lua.create_table()?;
                            for (k, v) in data_map {
                                data_table.raw_set(k, v)?;
                            }
                            out.raw_set("ok", true)?;
                            out.raw_set("unmounted", false)?;
                            out.raw_set("code", 0)?;
                            out.raw_set("data", data_table)?;
                        }
                        Err(err_msg) => {
                            out.raw_set("ok", false)?;
                            out.raw_set("unmounted", false)?;
                            out.raw_set("code", 1)?;
                            out.raw_set("error", err_msg)?;
                        }
                    },
                }
                Ok(out)
            },
        )?,
    )?;
    ygg.raw_set("tool", tool_table)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::rc::Rc;

    use ygg_core::entity::{EntityStore, Value};
    use ygg_core::kv::KvStore;

    use ygg_core::bus::HandlerRegistry;
    use ygg_core::event::Event;

    use super::super::{install, HostHandles, HostToolRunner, Phase};

    struct MockRunner;
    impl HostToolRunner for MockRunner {
        fn call(
            &mut self,
            tool_id: &str,
            args: &BTreeMap<String, String>,
        ) -> Result<BTreeMap<String, String>, String> {
            if tool_id == "mock.fail" {
                return Err("boom".to_owned());
            }
            let mut out = BTreeMap::new();
            out.insert("tool".to_owned(), tool_id.to_owned());
            if let Some(v) = args.get("k") {
                out.insert("echo".to_owned(), v.clone());
            }
            Ok(out)
        }
    }

    #[test]
    fn ygg_tool_call_respects_phase_and_runner_mount_state() {
        let sandbox = crate::sandbox::Sandbox::new().expect("sandbox");
        let lua = sandbox.lua().clone();
        let handles = HostHandles {
            kv: Rc::new(RefCell::new(KvStore::new())),
            entities: Rc::new(RefCell::new(EntityStore::new())),
        };
        let host = install(&lua, handles).expect("install host");
        // Loading 相位下直接调用 ygg.tool.call 必须被拒。
        let loading_err = lua
            .load(r#"ygg.tool.call("mock.ok", { k = "v" })"#)
            .exec()
            .expect_err("loading phase must reject ygg.tool.call");
        assert!(
            loading_err.to_string().contains("transition phase"),
            "{loading_err}"
        );

        lua.load(
            r#"
            ygg.on("test.call", function(tool_id)
                local res = ygg.tool.call(tool_id, { k = "val", n = 42, b = true })
                ygg.kv.set("ok", res.ok == true)
                ygg.kv.set("unmounted", res.unmounted == true)
                ygg.kv.set("code", res.code or -99)
                if res.data then
                    ygg.kv.set("echo", res.data.echo or "")
                end
                if res.error then
                    ygg.kv.set("err", res.error)
                end
            end)
        "#,
        )
        .exec()
        .expect("register handler");
        host.seal(&lua).expect("seal");
        let mut registry = HandlerRegistry::new();
        host.register_into(&mut registry, "mod");
        let mut entities = EntityStore::new();
        let mut fuel = crate::fuel::install(&lua, 10_000).expect("fuel");

        // 1) 未挂载：返回 unmounted = true, ok = false, code = -1
        ygg_core::transition::run_transition(
            &mut entities,
            &mut registry,
            &mut fuel,
            Event::external("test.call", Value::Str("mock.ok".to_owned())).expect("event"),
        )
        .expect("unmounted transition");
        assert_eq!(
            host.state.borrow().handles.kv.borrow().get("unmounted"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            host.state.borrow().handles.kv.borrow().get("ok"),
            Some(&Value::Bool(false))
        );
        assert_eq!(
            host.state.borrow().handles.kv.borrow().get("code"),
            Some(&Value::Int(-1))
        );

        // 2) 挂载后成功调用：ok = true, unmounted = false, code = 0, data 映射回传
        host.set_tool_runner(Some(Box::new(MockRunner)));
        fuel.reset(10_000);
        ygg_core::transition::run_transition(
            &mut entities,
            &mut registry,
            &mut fuel,
            Event::external("test.call", Value::Str("mock.ok".to_owned())).expect("event"),
        )
        .expect("mounted ok transition");
        assert_eq!(
            host.state.borrow().handles.kv.borrow().get("unmounted"),
            Some(&Value::Bool(false))
        );
        assert_eq!(
            host.state.borrow().handles.kv.borrow().get("ok"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            host.state.borrow().handles.kv.borrow().get("echo"),
            Some(&Value::Str("val".to_owned()))
        );

        // 3) 挂载后返回 Err：ok = false, unmounted = false, code = 1, error 回传
        fuel.reset(10_000);
        ygg_core::transition::run_transition(
            &mut entities,
            &mut registry,
            &mut fuel,
            Event::external("test.call", Value::Str("mock.fail".to_owned())).expect("event"),
        )
        .expect("mounted err transition");
        assert_eq!(
            host.state.borrow().handles.kv.borrow().get("ok"),
            Some(&Value::Bool(false))
        );
        assert_eq!(
            host.state.borrow().handles.kv.borrow().get("code"),
            Some(&Value::Int(1))
        );
        assert_eq!(
            host.state.borrow().handles.kv.borrow().get("err"),
            Some(&Value::Str("boom".to_owned()))
        );
    }
}
