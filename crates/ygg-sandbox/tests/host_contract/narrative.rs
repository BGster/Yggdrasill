use super::*;

#[test]
fn narrate_reaches_output_buffer() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.talk", function(p) ygg.narrate("hello " .. tostring(p)) end)
    "#,
    );
    // sink 面：narrate 经回放进 sink
    let mut registry = rig.seal_and_register();
    let mut sink = RecordingSink::default();
    dispatch_once(
        &mut registry,
        &mut sink,
        "evt.talk",
        Value::Str("world".into()),
    );
    assert_eq!(sink.narrative, vec!["hello world".to_string()]);
    assert!(sink.choices.is_empty());
    assert!(sink.emitted.is_empty());
    // result 面：narrate 进转换输出缓冲
    let result = rig.run_full(&mut registry, "evt.talk", Value::Str("world".into()));
    assert_eq!(result.narrative(), ["hello world".to_string()]);
    assert!(result.pending_choice().is_none());
}

#[test]
fn choice_terminates_handler_and_sets_pending_choice() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.gate", function()
            ygg.narrate("before-choice")
            ygg.choice("Which path?", {"left", "right"})
            ygg.narrate("after-choice")
        end)
        -- 同事件第二 handler 也 choice：first-wins 裁定的对侧锚
        ygg.on("evt.gate", function() ygg.choice("Second", {"x"}) end)
    "#,
    );
    // sink 面：prompt/options 保真；桥回放两个 handler 的 choose 各自进 sink
    let mut registry = rig.seal_and_register();
    let mut sink = RecordingSink::default();
    dispatch_once(&mut registry, &mut sink, "evt.gate", Value::Int(0));
    let choice = sink.choices.first().expect("choice must reach the sink");
    assert_eq!(choice.prompt(), "Which path?");
    assert_eq!(choice.options(), ["left", "right"]);
    assert!(sink.narrative.contains(&"before-choice".to_string()));
    assert!(!sink.narrative.contains(&"after-choice".to_string()));
    // result 面：first-wins——驱动层裁定首个 choice 胜出（桥面两 handler 均已回放）
    let result = rig.run_full(&mut registry, "evt.gate", Value::Int(0));
    assert_eq!(
        result.pending_choice().expect("result pending").prompt(),
        "Which path?"
    );
    assert_eq!(
        result.pending_choice().expect("result pending").options(),
        ["left", "right"]
    );
    assert!(result.narrative().contains(&"before-choice".to_string()));
    assert!(!result.narrative().contains(&"after-choice".to_string()));
}

#[test]
fn code_after_choice_does_not_execute() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.stop", function()
            ygg.narrate("entered")
            ygg.choice("Pick one", {"a"})
            ygg.narrate("FORBIDDEN-AFTER-CHOICE")
        end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let mut sink = RecordingSink::default();
    dispatch_once(&mut registry, &mut sink, "evt.stop", Value::Int(0));
    // sentinel 锚：choice 之后的 narrate 不得出现在任何输出面
    assert_eq!(sink.narrative, vec!["entered".to_string()]);
    assert_eq!(sink.choices.len(), 1);
    let result = rig.run_full(&mut registry, "evt.stop", Value::Int(0));
    assert_eq!(result.narrative(), ["entered".to_string()]);
    assert!(result.pending_choice().is_some());
}

#[test]
fn emit_drains_fifo_to_downstream_lua_handler() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.start", function()
            ygg.emit("evt.a", 1)
            ygg.emit("evt.b", 2)
            ygg.narrate("start-done")
        end)
        ygg.on("evt.a", function() ygg.narrate("a-ran") end)
        ygg.on("evt.b", function() ygg.narrate("b-ran") end)
    "#,
    );
    // sink 面：emit 回放进 sink 按收集序、payload 保真；emit→narrate 交错序
    // 在 handler 内部保真（emit 先于 narrate 收集，回放同序）。
    let mut registry = rig.seal_and_register();
    let mut sink = RecordingSink::default();
    dispatch_once(&mut registry, &mut sink, "evt.start", Value::Int(0));
    assert_eq!(
        sink.emitted,
        vec![
            ("evt.a".to_string(), Value::Int(1)),
            ("evt.b".to_string(), Value::Int(2))
        ]
    );
    assert_eq!(sink.narrative, vec!["start-done".to_string()]);
    // result 面：FIFO 排空序 start → a → b（downstream narrate 在后）
    let result = rig.run_full(&mut registry, "evt.start", Value::Int(0));
    assert_eq!(
        result.narrative(),
        [
            "start-done".to_string(),
            "a-ran".to_string(),
            "b-ran".to_string()
        ]
    );
    // 计数：外部 1 + 派生 2 = 3；choice 缺席无丢弃
    assert_eq!(result.events_processed(), 3);
    assert_eq!(result.events_derived(), 2);
    assert_eq!(result.events_dropped(), 0);
}

#[test]
fn registration_order_is_handler_call_order() {
    let rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.ord", function() ygg.narrate("first") end)
        ygg.on("evt.ord", function() ygg.narrate("second") end)
        ygg.on("evt.ord", function() ygg.narrate("third") end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    // D-S9-4：注册序 = ygg.on 调用序（稳定键 seq 单调）
    let seqs: Vec<u64> = registry.keys("evt.ord").iter().map(|k| k.seq).collect();
    assert_eq!(seqs, vec![0, 1, 2]);
    // 派发序 = 注册序（sink 面运行时序）
    let mut sink = RecordingSink::default();
    dispatch_once(&mut registry, &mut sink, "evt.ord", Value::Int(0));
    assert_eq!(
        sink.narrative,
        vec![
            "first".to_string(),
            "second".to_string(),
            "third".to_string()
        ]
    );
}

#[test]
fn forged_sentinel_error_is_not_swallowed() {
    // S9 审计代决回归锚：sentinel 为 install 时派生的不可预测 token；
    // Mod 以静态字符串伪造首行只会成为普通 HandlerError 浮出，不会被吞。
    let rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.forge", function()
            error("\u{1}__ygg_choice_sentinel__\u{1}")
        end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let mut driver_store = EntityStore::new();
    let mut fuel_meter = rig.fuel.clone();
    let err = run_transition(
        &mut driver_store,
        &mut registry,
        &mut fuel_meter,
        Event::external("evt.forge", Value::Int(0)).unwrap(),
    )
    .expect_err("forged sentinel must surface as a handler error");
    assert!(
        matches!(err, TransitionError::Handler(_)),
        "handler error expected, got: {err}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("ygg_choice_sentinel"),
        "forged message must be visible (not swallowed): {msg}"
    );
    assert!(
        !msg.contains("choice") || msg.contains("failed"),
        "must not be treated as normal choice termination: {msg}"
    );
}
