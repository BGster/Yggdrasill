//! 用例 11-14：存读档往返、统一 store 代决证据与外部事件日志面。
use super::*;

/// 事件日志 Mod：handler 发派生事件——日志面只记外部事件。
/// 派生名 `evt.derived` 无操作注册（fail-closed：派生面同样要求注册）。
const LOG_MOD: &str = r#"
ygg.on("event.log", function()
  ygg.emit("evt.derived", 1)
  ygg.narrate("logged")
end)
ygg.on("evt.derived", function() end)
"#;

#[test]
fn handler_state_reaches_driver_save_and_restore_continues() {
    // 统一 store 代决证据：handler 的实体/KV 写落 canonical store，
    // save 快照携带之；restore 后 handler 在恢复态上续接。
    let mut kernel = KernelFacade::new(STATE_MOD).expect("kernel construction");
    let parsed = reply_ok(&kernel.dispatch("event.hit", "0"));
    assert!(
        parsed["revision"].as_u64().unwrap_or(0) >= 1,
        "handler write bumps revision"
    );
    let save_reply = reply_ok(&kernel.dispatch("save", ""));
    let mut restored = KernelFacade::restore(
        serde_json::to_vec(&save_reply["save"]).unwrap().as_slice(),
        STATE_MOD,
    )
    .expect("restore");
    // 恢复态续接：hp=9 起点，再 hit 一次 → hp=8、hits=2（读档前累计不丢）。
    let _ = restored.dispatch("event.hit", "0");
    let restored_save = reply_ok(&restored.dispatch("save", ""));
    assert_eq!(restored_save["save"]["kv"]["entries"]["hits"], json!(2));
    // 实体表序列化为 id 键控对象（BTreeMap<EntityId, Entity>）。
    let entity = &restored_save["save"]["entities"]["entities"]["1"];
    assert_eq!(entity["attributes"]["hp"], json!(8));
}

#[test]
fn pending_choice_persisted_and_informational_after_restore() {
    let mut kernel = KernelFacade::new(GATE_MOD).expect("kernel construction");
    let _ = reply_ok(&kernel.dispatch("event.start", "0"));
    let save_reply = reply_ok(&kernel.dispatch("save", ""));
    assert!(
        save_reply["save"]["pending_choice"].is_object(),
        "pending choice is kernel state (B3 snapshot)"
    );
    let mut restored = KernelFacade::restore(
        serde_json::to_vec(&save_reply["save"]).unwrap().as_slice(),
        GATE_MOD,
    )
    .expect("restore");
    // 挂起选择原样重现（informational：状态数据，读档后玩家仍面对待决项）。
    let state = reply_ok(&restored.dispatch("state", ""));
    assert_eq!(state["pendingChoice"]["prompt"], json!("继续前行？"));
    assert_eq!(state["pendingChoice"]["options"], json!(["go", "stay"]));
    // respond 在恢复态上照常工作（选择 = 新外部事件，不依赖未读档前的栈）。
    let parsed = reply_ok(&restored.dispatch("ygg.choice_selected", "1"));
    assert_eq!(parsed["narrative"], json!(["picked=1"]));
}

#[test]
fn load_command_restores_state_in_place() {
    let mut source = KernelFacade::new(STATE_MOD).expect("kernel construction");
    let _ = reply_ok(&source.dispatch("event.hit", "0"));
    let save_reply = reply_ok(&source.dispatch("save", ""));
    let save_str = save_reply["save"].to_string();
    // 目标内核（同 Mod 重装载）经 dispatch("load") 原地恢复——不走 restore 构造器。
    let mut target = KernelFacade::new(STATE_MOD).expect("kernel construction");
    let parsed = reply_ok(&target.dispatch("load", &save_str));
    assert_eq!(parsed["restored"], json!(true));
    // 原地恢复后 handler 续接：hits 计数从 1 起再 +1（新内核装载期未跑过 hit）。
    let _ = reply_ok(&target.dispatch("event.hit", "0"));
    let final_save = reply_ok(&target.dispatch("save", ""));
    assert_eq!(final_save["save"]["kv"]["entries"]["hits"], json!(2));
    assert_eq!(final_save["save"]["engine_version"], json!(ENGINE_VERSION));
    // 坏存档 → VALIDATION 信封（域错误不 throw）。
    let reply = target.dispatch("load", r#"{"format_version":1,"engine_version":"0.0.0-x"}"#);
    assert_eq!(reply_error_code(&reply), "VALIDATION");
}

#[test]
fn event_log_records_external_events_only() {
    let mut kernel = KernelFacade::new(LOG_MOD).expect("kernel construction");
    let _ = reply_ok(&kernel.dispatch("event.log", "7"));
    let _ = reply_ok(&kernel.dispatch("event.log", "9"));
    let log_reply = reply_ok(&kernel.dispatch("log", ""));
    let log = log_reply["log"].as_array().expect("log array");
    assert_eq!(
        log,
        &vec![
            json!({"name": "event.log", "payload": 7}),
            json!({"name": "event.log", "payload": 9}),
        ],
        "derived evt.derived must not enter the external log (B1)"
    );
}
