//! 用例 1-5：JSON 信封形状（A1）与 choice 终结/respond（B2）面。
use super::*;

#[test]
fn envelope_ok_shape_for_entry_event() {
    let mut kernel = KernelFacade::new(GATE_MOD).expect("kernel construction");
    let reply = kernel.dispatch("event.start", "0");
    let parsed = reply_ok(&reply);
    // A1 线格式字段面：恰好八个键，无 stateHash（S12 才回填哈希面）。
    let mut keys: Vec<&str> = parsed
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "event",
            "eventsDerived",
            "eventsDropped",
            "eventsProcessed",
            "narrative",
            "ok",
            "pendingChoice",
            "revision"
        ]
    );
    assert!(parsed.get("stateHash").is_none(), "stateHash lands in S12");
    assert_eq!(parsed["event"], json!("event.start"));
    assert_eq!(parsed["narrative"], json!([]));
    assert_eq!(parsed["eventsProcessed"], json!(1));
    assert_eq!(parsed["eventsDerived"], json!(0));
    assert_eq!(parsed["eventsDropped"], json!(0));
    assert!(
        parsed["pendingChoice"].is_object(),
        "choice pending: {parsed}"
    );
    assert_eq!(parsed["pendingChoice"]["prompt"], json!("继续前行？"));
    assert_eq!(parsed["pendingChoice"]["options"], json!(["go", "stay"]));
}

#[test]
fn choice_terminates_and_respond_is_new_external_event() {
    let mut kernel = KernelFacade::new(GATE_MOD).expect("kernel construction");
    // 首事件：choice 终结转换（first-wins；FIFO 余量计 dropped）。
    let parsed = reply_ok(&kernel.dispatch("event.start", "0"));
    assert!(parsed["pendingChoice"].is_object());
    // respond = 新外部事件（B2 choice-as-state；payload 2 → picked=2）。
    let parsed = reply_ok(&kernel.dispatch("ygg.choice_selected", "2"));
    assert_eq!(parsed["narrative"], json!(["picked=2"]));
    assert!(parsed["pendingChoice"].is_null(), "respond clears pending");
}

#[test]
fn load_command_reply_carries_pending_choice() {
    // A1：load 成功回包携带 pendingChoice（同 state 形态）——读档方零额外
    // dispatch 即可恢复提示面。挂起选择随存档往返（B3）的直接证据。
    let mut source = KernelFacade::new(GATE_MOD).expect("kernel construction");
    let _ = reply_ok(&source.dispatch("event.start", "0"));
    let save_reply = reply_ok(&source.dispatch("save", ""));
    let save_str = save_reply["save"].to_string();
    let mut target = KernelFacade::new(GATE_MOD).expect("kernel construction");
    let parsed = reply_ok(&target.dispatch("load", &save_str));
    assert_eq!(parsed["restored"], json!(true));
    assert_eq!(parsed["pendingChoice"]["prompt"], json!("继续前行？"));
    assert_eq!(parsed["pendingChoice"]["options"], json!(["go", "stay"]));
    // respond 在 load 恢复态上照常工作（选择 = 新外部事件）。
    let parsed = reply_ok(&target.dispatch("ygg.choice_selected", "1"));
    assert_eq!(parsed["narrative"], json!(["picked=1"]));
    // 无待决选择的存档：load 回包 pendingChoice 为 null（与 state 同形态）。
    let mut clean = KernelFacade::new(GATE_MOD).expect("kernel construction");
    let clean_save = reply_ok(&clean.dispatch("save", ""));
    let parsed = reply_ok(&clean.dispatch("load", &clean_save["save"].to_string()));
    assert!(parsed["pendingChoice"].is_null());
}

#[test]
fn choice_selected_out_of_range_is_rejected_readably() {
    // A2：待决选择存在时序号越界 → 可读 VALIDATION（不进内核转换）。
    let mut kernel = KernelFacade::new(GATE_MOD).expect("kernel construction");
    let _ = reply_ok(&kernel.dispatch("event.start", "0"));
    let reply = kernel.dispatch("ygg.choice_selected", "9");
    let parsed: Json = parse(&reply);
    assert_eq!(parsed["ok"], json!(false), "reply: {reply}");
    assert_eq!(parsed["error"]["code"], json!("VALIDATION"));
    let message = parsed["error"]["message"].as_str().expect("message");
    assert!(message.contains("选项不存在"), "readable message: {reply}");
    assert!(message.contains("1..2"), "valid range in message: {reply}");
    // 校验失败不消费待决选择：仍挂起，随后合法序号照常工作。
    let state = reply_ok(&kernel.dispatch("state", ""));
    assert!(state["pendingChoice"].is_object(), "pending must survive");
    let parsed = reply_ok(&kernel.dispatch("ygg.choice_selected", "2"));
    assert_eq!(parsed["narrative"], json!(["picked=2"]));
}

#[test]
fn choice_selected_non_integer_is_rejected_readably() {
    // A2：非整数序号（非 JSON / 浮点 / 字符串标量）→ 同一可读 VALIDATION。
    let mut kernel = KernelFacade::new(GATE_MOD).expect("kernel construction");
    let _ = reply_ok(&kernel.dispatch("event.start", "0"));
    for bad in ["abc", "1.5", r#""2""#] {
        let reply = kernel.dispatch("ygg.choice_selected", bad);
        let parsed: Json = parse(&reply);
        assert_eq!(
            parsed["error"]["code"],
            json!("VALIDATION"),
            "payload {bad}"
        );
        let message = parsed["error"]["message"].as_str().expect("message");
        assert!(
            message.contains("选项不存在") && message.contains("1..2"),
            "non-integer {bad} must name the valid range: {reply}"
        );
    }
}
