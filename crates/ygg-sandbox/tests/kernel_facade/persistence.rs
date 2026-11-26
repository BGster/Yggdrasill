//! 用例 11-14：存读档往返、一字宙代决证据与外部事件日志面。
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
    // 一字宙代决证据：handler 的世界/KV 写落同一份内核状态，save 快照
    // 携带它的帧字节；restore 后 handler 在恢复态上续接。
    use ygg_sandbox::hash::state_hash;

    let mut kernel = KernelFacade::new(STATE_MOD).expect("kernel construction");
    let parsed = reply_ok(&kernel.dispatch("event.hit", "0"));
    assert!(
        parsed["revision"].as_u64().unwrap_or(0) >= 1,
        "handler write bumps revision"
    );
    let hash_before = state_hash(&kernel).expect("hash");
    let save_reply = reply_ok(&kernel.dispatch("save", ""));
    let mut restored = KernelFacade::restore(
        serde_json::to_vec(&save_reply["save"]).unwrap().as_slice(),
        STATE_MOD,
    )
    .expect("restore");
    // 读档后的世界与读档前**逐字节**同源（快照帧是世界的唯一表示）：
    // 这比断言某个字段的 JSON 形状更强——hp=9 的任何丢失都会改帧。
    assert_eq!(
        state_hash(&restored).expect("hash"),
        hash_before,
        "restore must reproduce the saved world bit-for-bit"
    );
    // 恢复态续接：hp=9 起点，再 hit 一次 → hp=8、hits=2（读档前累计不丢）。
    let _ = restored.dispatch("event.hit", "0");
    let restored_save = reply_ok(&restored.dispatch("save", ""));
    assert_eq!(restored_save["save"]["kv"]["entries"]["hits"], json!(2));
    // 存档里的世界是内核快照帧字节（`save.world` = u8 数组），不是行存表。
    let world = restored_save["save"]["world"]
        .as_array()
        .expect("save.world must be a byte array");
    assert!(world.len() > 32, "帧至少包含魔数与尾 digest");
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
