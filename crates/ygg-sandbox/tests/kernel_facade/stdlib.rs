//! 用例 15-20：确定性 canonical 字节与 stdlib（narrative/fsm/when）全链路面。
use super::*;

/// 叙事流 Mod：node 声明 + goto 路由（stdlib narrative 全链路）。
const NARRATIVE_MOD: &str = r#"
ygg.narrative.node("node:intro", function()
  ygg.narrate("node:intro")
  ygg.narrative["goto"]("node:scene")
end)
ygg.narrative.node("node:scene", function()
  ygg.narrate("node:scene")
end)
ygg.on("event.enter", function()
  ygg.narrative["goto"]("node:intro")
end)
"#;

/// 状态机 Mod：门状态机（stdlib fsm 全链路；状态持久化在 KV）。
const FSM_MOD: &str = r#"
ygg.fsm.define{
  name = "door",
  initial = "closed",
  event = "event.toggle",
  transitions = {
    { from = "closed", to = "open" },
    { from = "open", to = "closed" },
  },
}
"#;

/// 条件边状态机 Mod（B1）：同 from 双出边，声明序首个 when 成立者胜出。
const FSM_WHEN_MOD: &str = r#"
ygg.fsm.define{
  name = "router",
  initial = "road",
  event = "event.go",
  transitions = {
    { from = "road", to = "cellar", when = function(p) return p == 1 end },
    { from = "road", to = "inn" },
  },
}
"#;

#[test]
fn determinism_same_sequence_same_canonical_bytes() {
    let mut a = KernelFacade::new(STATE_MOD).expect("kernel construction");
    let mut b = KernelFacade::new(STATE_MOD).expect("kernel construction");
    for event in ["event.hit", "event.hit", "event.hit"] {
        assert_eq!(
            reply_ok(&a.dispatch(event, "0")),
            reply_ok(&b.dispatch(event, "0"))
        );
    }
    assert_eq!(
        a.canonical_state_bytes().unwrap(),
        b.canonical_state_bytes().unwrap(),
        "same sequence must reach byte-identical canonical state (FR9 native)"
    );
}

#[test]
fn stdlib_narrative_flow_through_facade() {
    let mut kernel = KernelFacade::new(NARRATIVE_MOD).expect("kernel construction");
    let parsed = reply_ok(&kernel.dispatch("event.enter", "0"));
    assert_eq!(parsed["narrative"], json!(["node:intro", "node:scene"]));
    assert_eq!(parsed["eventsDerived"], json!(2));
    assert_eq!(parsed["eventsProcessed"], json!(3));
    // 未知节点 = 可读错误（经 VALIDATION 码浮出，不静默）。
    let mut kernel2 = KernelFacade::new(
        r#"ygg.on("event.enter", function() ygg.narrative["goto"]("node:ghost") end)"#,
    )
    .expect("kernel construction");
    let reply = kernel2.dispatch("event.enter", "0");
    assert_eq!(reply_error_code(&reply), "VALIDATION");
}

#[test]
fn stdlib_fsm_drives_state_through_facade() {
    let mut kernel = KernelFacade::new(FSM_MOD).expect("kernel construction");
    // 第一次 toggle：状态未初始化 → initial "closed" 匹配 → "open"。
    let _ = reply_ok(&kernel.dispatch("event.toggle", "0"));
    let save_reply = reply_ok(&kernel.dispatch("save", ""));
    assert_eq!(
        save_reply["save"]["kv"]["entries"]["fsm.door"],
        json!("open")
    );
    // 第二次 toggle："open" → "closed"。
    let _ = reply_ok(&kernel.dispatch("event.toggle", "0"));
    let save_reply = reply_ok(&kernel.dispatch("save", ""));
    assert_eq!(
        save_reply["save"]["kv"]["entries"]["fsm.door"],
        json!("closed")
    );
    // 状态随 KV 存档往返：restore 后第三次 toggle 续接（closed → open）。
    let mut restored = KernelFacade::restore(
        serde_json::to_vec(&save_reply["save"]).unwrap().as_slice(),
        FSM_MOD,
    )
    .expect("restore");
    let _ = reply_ok(&restored.dispatch("event.toggle", "0"));
    let final_save = reply_ok(&restored.dispatch("save", ""));
    assert_eq!(
        final_save["save"]["kv"]["entries"]["fsm.door"],
        json!("open")
    );
}

#[test]
fn stdlib_fsm_persists_initial_on_first_trigger_even_without_match() {
    // B2 键缺席可见化：首次触发且键缺席 → 先落 initial 再参与匹配；
    // 即使无匹配边（静默 no-op），initial 已落盘——handler 内 kv.get
    // 与存档 kv.entries 均可见（原语义：键缺失难排查的创作痛点）。
    const FSM_SEALED_MOD: &str = r#"
ygg.fsm.define{
  name = "gate",
  initial = "sealed",
  event = "event.knock",
  transitions = {
    { from = "open", to = "ajar" },
  },
}
ygg.on("event.probe", function()
  ygg.narrate("gate=" .. tostring(ygg.kv.get("fsm.gate")))
end)
"#;
    let mut kernel = KernelFacade::new(FSM_SEALED_MOD).expect("kernel construction");
    // define 后首次触发 knock：initial "sealed" 落盘；from="open" 无匹配 →
    // 不转移（静默 no-op 面不变，但键已在）。
    let _ = reply_ok(&kernel.dispatch("event.knock", "0"));
    let parsed = reply_ok(&kernel.dispatch("event.probe", "0"));
    assert_eq!(
        parsed["narrative"],
        json!(["gate=sealed"]),
        "initial must be visible via kv.get after first trigger"
    );
    let save_reply = reply_ok(&kernel.dispatch("save", ""));
    assert_eq!(
        save_reply["save"]["kv"]["entries"]["fsm.gate"],
        json!("sealed"),
        "initial persisted in kv.entries even when no edge matched"
    );
    // 幂等：再次触发同事件（键已存在）不重复写、不改值。
    let _ = reply_ok(&kernel.dispatch("event.knock", "0"));
    let save_reply = reply_ok(&kernel.dispatch("save", ""));
    assert_eq!(
        save_reply["save"]["kv"]["entries"]["fsm.gate"],
        json!("sealed")
    );
}

#[test]
fn stdlib_fsm_when_edges_dispatch_in_declaration_order() {
    // B1 条件边：同 from（road）双出边，声明序首个 when 成立者胜出——
    // payload 1 命中第一条（road → cellar）；payload 2 时 when 不成立，
    // 第二条无 when（恒真）胜出（road → inn）。无匹配仍静默 no-op。
    let mut kernel = KernelFacade::new(FSM_WHEN_MOD).expect("kernel construction");
    let _ = reply_ok(&kernel.dispatch("event.go", "1"));
    let save_reply = reply_ok(&kernel.dispatch("save", ""));
    assert_eq!(
        save_reply["save"]["kv"]["entries"]["fsm.router"],
        json!("cellar"),
        "payload 1 must take the first (when-true) edge"
    );
    let mut kernel = KernelFacade::new(FSM_WHEN_MOD).expect("kernel construction");
    let _ = reply_ok(&kernel.dispatch("event.go", "2"));
    let save_reply = reply_ok(&kernel.dispatch("save", ""));
    assert_eq!(
        save_reply["save"]["kv"]["entries"]["fsm.router"],
        json!("inn"),
        "payload 2 must fall through to the unconditional edge"
    );
}

#[test]
fn stdlib_fsm_when_must_be_function_at_define_time() {
    // B1 定义期校验：when 声明时必须为 function（或缺省）——否则装载期
    // 可读报错（ModLoad，含字段定位）。
    const BAD_WHEN_MOD: &str = r#"
ygg.fsm.define{
  name = "broken",
  initial = "a",
  event = "event.x",
  transitions = {
    { from = "a", to = "b", when = "yes" },
  },
}
"#;
    let outcome = KernelFacade::new(BAD_WHEN_MOD);
    assert!(
        matches!(
            outcome,
            Err(KernelError::ModLoad(ref msg))
                if msg.contains("transitions[1].when") && msg.contains("function")
        ),
        "non-function when must fail at define time with a readable error"
    );
}
