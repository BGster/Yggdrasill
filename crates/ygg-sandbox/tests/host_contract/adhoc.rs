use super::*;

#[test]
fn kv_set_get_round_trip() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.kv", function()
            ygg.kv.set("gold", 30)
            ygg.kv.set("name", "feni")
            ygg.narrate("gold=" .. tostring(ygg.kv.get("gold")))
            ygg.narrate("name=" .. tostring(ygg.kv.get("name")))
            local miss = ygg.kv.get("missing")
            ygg.narrate(miss == nil and "MISS_NIL" or "PRESENT")
        end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let result = rig.run_full(&mut registry, "evt.kv", Value::Int(0));
    assert_eq!(
        result.narrative(),
        [
            "gold=30".to_string(),
            "name=feni".to_string(),
            "MISS_NIL".to_string()
        ]
    );
    // 即席写直达宿主句柄 store（非回放）
    assert_eq!(rig.handles.kv.borrow().get("gold"), Some(&Value::Int(30)));
    assert_eq!(
        rig.handles.kv.borrow().get("name"),
        Some(&Value::Str("feni".into()))
    );
}

#[test]
fn kv_nan_rejected_with_readable_error() {
    let rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.nan", function() ygg.kv.set("ratio", 0.0 / 0.0) end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let mut driver_store = EntityStore::new();
    let mut fuel_meter = rig.fuel.clone();
    let err = run_transition(
        &mut driver_store,
        &mut registry,
        &mut fuel_meter,
        Event::external("evt.nan", Value::Int(0)).unwrap(),
    )
    .expect_err("NaN kv set must fail the transition");
    let msg = err.to_string();
    assert!(
        matches!(err, TransitionError::Handler(_)),
        "handler error expected: {msg}"
    );
    // 可读性：消息含键名 + 非有限语义
    assert!(msg.contains("ratio"), "error must name the key: {msg}");
    assert!(
        msg.contains("non-finite"),
        "error must state non-finite: {msg}"
    );
    // 拒绝路径不落表
    assert!(rig.handles.kv.borrow().get("ratio").is_none());
}

#[test]
fn entity_attribute_set_get_round_trip() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.ent", function()
            local id = ygg.entity.spawn()
            ygg.narrate("id=" .. tostring(id))
            ygg.entity.set_attribute(id, "hp", 42)
            ygg.narrate("hp=" .. tostring(ygg.entity.get_attribute(id, "hp")))
            local miss = ygg.entity.get_attribute(id, "nope")
            ygg.narrate(miss == nil and "ATTR_NIL" or "PRESENT")
        end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let result = rig.run_full(&mut registry, "evt.ent", Value::Int(0));
    assert_eq!(
        result.narrative(),
        [
            "id=1".to_string(),
            "hp=42".to_string(),
            "ATTR_NIL".to_string()
        ]
    );
    // 即席写直达宿主句柄 store
    let entities = rig.handles.entities.borrow();
    let entity = entities.get(1).expect("entity 1 must exist");
    assert_eq!(entity.attribute("hp"), Some(&Value::Int(42)));
}

#[test]
fn entity_tags_ops_round_trip() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.tag", function()
            local id = ygg.entity.spawn()
            local added = ygg.entity.add_tag(id, "hero")
            local dup = ygg.entity.add_tag(id, "hero")
            local has = ygg.entity.has_tag(id, "hero")
            local removed = ygg.entity.remove_tag(id, "hero")
            local has_after = ygg.entity.has_tag(id, "hero")
            ygg.narrate(tostring(added) .. "," .. tostring(dup) .. ","
                .. tostring(has) .. "," .. tostring(removed) .. ","
                .. tostring(has_after))
        end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let result = rig.run_full(&mut registry, "evt.tag", Value::Int(0));
    assert_eq!(
        result.narrative(),
        ["true,false,true,true,false".to_string()]
    );
    assert!(!rig
        .handles
        .entities
        .borrow()
        .get(1)
        .expect("entity 1")
        .has_tag("hero"));
}
