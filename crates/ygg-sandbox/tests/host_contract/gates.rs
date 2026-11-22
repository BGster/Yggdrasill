use super::*;

#[test]
fn ygg_on_after_seal_errors_readably() {
    let rig = Rig::new();
    rig.host.seal(rig.lua()).expect("first seal succeeds");
    let err = rig
        .lua()
        .load(r#"ygg.on("evt.late", function() end)"#)
        .exec()
        .expect_err("sealed ygg.on must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("sealed"),
        "readable seal error expected: {msg}"
    );
    assert!(
        msg.contains("forbidden") || msg.contains("load phase"),
        "error must state the registration ban: {msg}"
    );
    // 重复 seal 同样可读拒绝
    let err2 = rig.host.seal(rig.lua()).expect_err("double seal must fail");
    assert!(
        err2.to_string().contains("sealed"),
        "double seal message: {err2}"
    );
}

#[test]
fn unsupported_payload_type_rejected_readably() {
    let rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.bad", function() ygg.emit("evt.out", {1, 2}) end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let mut driver_store = EntityStore::new();
    let err = run_transition(
        &mut driver_store,
        &mut registry,
        Event::external("evt.bad", Value::Int(0)).unwrap(),
    )
    .expect_err("table payload must be rejected");
    let msg = err.to_string();
    assert!(
        matches!(err, TransitionError::Handler(_)),
        "handler error expected: {msg}"
    );
    assert!(
        msg.contains("unsupported payload type"),
        "readable rejection: {msg}"
    );
    assert!(msg.contains("table"), "error must name the lua type: {msg}");
}

// ---- TD1：`ygg.meta` 结构化回执通道（与叙事分离、瞬态、值面把关）----

#[test]
fn meta_channel_is_separate_from_narrative() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("meta.write", function()
            ygg.narrate("人读叙事")
            ygg.meta("summary", { stage = "review", conflicts = { { id = "r1" } } })
        end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let result = rig.run_full(&mut registry, "meta.write", Value::Int(0));
    // meta 不进 calls 回放面：叙事只有那一条人读行，没有任何结构化残渣
    assert_eq!(result.narrative(), &["人读叙事".to_owned()]);
    // 结构化数据在 HostState 里（驱动 take_meta 后拼信封），与叙事完全分离
    let meta = rig.host.take_meta();
    assert_eq!(meta["summary"]["stage"], serde_json::json!("review"));
    assert_eq!(
        meta["summary"]["conflicts"][0]["id"],
        serde_json::json!("r1")
    );
    // 取走即清空；clear_meta 同样幂等清空
    assert!(rig.host.take_meta().is_empty());
    rig.load_phase(r#"ygg.meta("after", 1)"#);
    rig.host.clear_meta();
    assert!(rig.host.take_meta().is_empty());
}

#[test]
fn meta_rejects_bad_keys_and_shapes_readably() {
    let rig = Rig::new();
    let check = |chunk: &str, needle: &str| {
        let msg = rig
            .lua()
            .load(chunk)
            .exec()
            .expect_err("chunk must be rejected")
            .to_string();
        assert!(
            msg.contains(needle),
            "expected `{needle}` in the readable error, got: {msg}"
        );
    };
    check(r#"ygg.meta("", 1)"#, "must not be empty");
    check(
        r#"ygg.meta("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", 1)"#,
        "too long",
    );
    check(r#"ygg.meta(7, 1)"#, "key must be a string");
    check(r#"ygg.meta("x", 0 / 0)"#, "non-finite");
    // 稀疏 / 混合表都是可读拒绝（不静默猜形状）
    check(r#"ygg.meta("x", { [2] = 1 })"#, "dense 1..n");
    check(r#"ygg.meta("x", { 1, 2, k = 3 })"#, "mixes array indices");
    // 深度封顶（9 层起拒绝）
    check(r#"ygg.meta("x", {{{{{{{{{{1}}}}}}}}}})"#, "nesting exceeds");
}
