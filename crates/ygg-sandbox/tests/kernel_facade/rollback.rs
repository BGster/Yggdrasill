//! 用例 25-34：dispatch 原子性——失败步整体回滚与成功步提交面。
use super::*;

/// 半写 Mod：handler 先写实体（spawn + set_attribute）与 KV，再显式报错。
const DIRTY_THEN_ERROR_MOD: &str = r#"
ygg.on("event.bad", function()
  local id = ygg.entity.spawn()
  ygg.entity.set_attribute(id, "hp", 1)
  ygg.kv.set("dirty", "yes")
  error("boom after writes")
end)
ygg.on("event.good", function()
  ygg.kv.set("kept", "yes")
end)
"#;

#[test]
fn failed_dispatch_rolls_back_entity_and_kv_writes() {
    // 红绿锚（本次改造的回归点）：handler 写实体 + KV 后报错，canonical
    // store 必须回到 dispatch 之前的基线——实体一个都不能留、两个 revision
    // 计数器都不许涨。
    let mut kernel = KernelFacade::new(DIRTY_THEN_ERROR_MOD).expect("kernel construction");
    let reply = kernel.dispatch("event.bad", "0");
    assert_eq!(reply_error_code(&reply), "VALIDATION");
    assert_eq!(
        saved_entities(&mut kernel),
        json!({}),
        "failed dispatch must not leave entities behind"
    );
    assert_eq!(
        saved_kv(&mut kernel),
        json!({}),
        "failed dispatch must not leave kv writes behind"
    );
    let state = reply_ok(&kernel.dispatch("state", ""));
    assert_eq!(
        state["revision"],
        json!(0),
        "entity revision must roll back"
    );
    assert_eq!(state["kvRevision"], json!(0), "kv revision must roll back");
}

#[test]
fn failed_dispatch_rolls_back_kv_only_writes() {
    // 矩阵 #1：纯 KV 写 + 报错。实体面为零，只验 kv 基线独立成立（与实体
    // 基线互不牵连：不是「因为实体克隆救了场」）。
    const KV_ONLY_MOD: &str = r#"
ygg.on("event.bad", function()
  ygg.kv.set("a", 1)
  ygg.kv.set("b", 2)
  error("boom")
end)
"#;
    let mut kernel = KernelFacade::new(KV_ONLY_MOD).expect("kernel construction");
    assert_eq!(
        reply_error_code(&kernel.dispatch("event.bad", "0")),
        "VALIDATION"
    );
    assert_eq!(saved_kv(&mut kernel), json!({}), "kv writes must roll back");
    assert_eq!(
        reply_ok(&kernel.dispatch("state", ""))["kvRevision"],
        json!(0)
    );
}

#[test]
fn fuel_exhaustion_rolls_back_writes() {
    // 矩阵 #3：燃料耗尽（VM 指令计量面）与 handler 报错同等对待——写过的
    // KV 与实体全部回滚。
    const BURN_MOD: &str = r#"
ygg.on("event.burn", function()
  ygg.kv.set("burned", 1)
  local id = ygg.entity.spawn()
  ygg.entity.set_attribute(id, "hp", 1)
  while true do end
end)
"#;
    let mut kernel = KernelFacade::new(BURN_MOD).expect("kernel construction");
    assert_eq!(
        reply_error_code(&kernel.dispatch("event.burn", "0")),
        "FUEL_EXHAUSTED"
    );
    assert_eq!(
        saved_entities(&mut kernel),
        json!({}),
        "entities rolled back"
    );
    assert_eq!(saved_kv(&mut kernel), json!({}), "kv rolled back");
    let state = reply_ok(&kernel.dispatch("state", ""));
    assert_eq!(state["revision"], json!(0));
    assert_eq!(state["kvRevision"], json!(0));
}

#[test]
fn derived_drain_failure_rolls_back_the_whole_dispatch() {
    // 矩阵 #4：派生事件排空中途失败——失败点之前的写（外部事件的 handler）
    // 同样回滚：savepoint 覆盖整次 dispatch，不是单个 handler。
    const DRAIN_MOD: &str = r#"
ygg.on("event.ext", function()
  ygg.kv.set("ext_wrote", 1)
  ygg.emit("evt.d", 0)
end)
ygg.on("evt.d", function()
  ygg.kv.set("derived_wrote", 2)
  error("derived handler boom")
end)
"#;
    let mut kernel = KernelFacade::new(DRAIN_MOD).expect("kernel construction");
    let reply = kernel.dispatch("event.ext", "0");
    assert_eq!(reply_error_code(&reply), "VALIDATION");
    let parsed = parse(&reply);
    assert!(
        parsed.get("eventsProcessed").is_none(),
        "error envelope carries no counters: {reply}"
    );
    assert_eq!(
        saved_kv(&mut kernel),
        json!({}),
        "external handler write must roll back with the failed dispatch"
    );
    assert_eq!(
        reply_ok(&kernel.dispatch("state", ""))["kvRevision"],
        json!(0)
    );
}

#[test]
fn committed_dispatch_survives_a_later_failed_step() {
    // 矩阵 #5（对照组）：成功步 = 提交。失败步不得吞掉它之前的成功步写入
    // ——「一律回滚」的错实现在这里会红。
    let mut kernel = KernelFacade::new(DIRTY_THEN_ERROR_MOD).expect("kernel construction");
    reply_ok(&kernel.dispatch("event.good", "0"));
    assert_eq!(saved_kv(&mut kernel), json!({"kept": "yes"}));
    assert_eq!(
        reply_error_code(&kernel.dispatch("event.bad", "0")),
        "VALIDATION"
    );
    assert_eq!(
        saved_kv(&mut kernel),
        json!({"kept": "yes"}),
        "rollback must target the failed step only, not the committed baseline"
    );
    let state = reply_ok(&kernel.dispatch("state", ""));
    assert_eq!(state["revision"], json!(0));
    assert_eq!(
        state["kvRevision"],
        json!(1),
        "kv revision of committed step"
    );
    // 失败步之后的成功步照常续接（基线仍是 {kept:yes}）。
    reply_ok(&kernel.dispatch("event.good", "0"));
    assert_eq!(saved_kv(&mut kernel), json!({"kept": "yes"}));
}

#[test]
fn rollback_baseline_follows_a_load_overwrite() {
    // 矩阵 #6：load 覆写 canonical 之后再 dispatch，基线必须是 load 之后
    // 的状态——既不是空（未刷新基线），也不是 {x:1,y:2}（半写）。
    const LOAD_MOD: &str = r#"
ygg.on("event.seed", function() ygg.kv.set("x", 1) end)
ygg.on("event.bad", function()
  ygg.kv.set("y", 2)
  error("boom")
end)
ygg.on("event.after", function() ygg.kv.set("z", 3) end)
"#;
    let mut source = KernelFacade::new(LOAD_MOD).expect("kernel construction");
    reply_ok(&source.dispatch("event.seed", "0"));
    let save_str = reply_ok(&source.dispatch("save", ""))["save"].to_string();
    let mut target = KernelFacade::new(LOAD_MOD).expect("kernel construction");
    reply_ok(&target.dispatch("load", &save_str));
    assert_eq!(
        reply_error_code(&target.dispatch("event.bad", "0")),
        "VALIDATION"
    );
    assert_eq!(
        saved_kv(&mut target),
        json!({"x": 1}),
        "baseline must be the loaded state"
    );
    reply_ok(&target.dispatch("event.after", "0"));
    assert_eq!(saved_kv(&mut target), json!({"x": 1, "z": 3}));
}

#[test]
fn failed_dispatch_still_lands_in_external_log() {
    // 矩阵 #7：旁路面口径——失败步**照记**日志（尝试日志 = 回放前提），
    // 条目形态与成功步同形。
    let mut kernel = KernelFacade::new(DIRTY_THEN_ERROR_MOD).expect("kernel construction");
    reply_ok(&kernel.dispatch("event.good", "7"));
    let _ = kernel.dispatch("event.bad", "9");
    let log = reply_ok(&kernel.dispatch("log", ""))["log"].clone();
    assert_eq!(
        log,
        json!([
            {"name": "event.good", "payload": 7},
            {"name": "event.bad", "payload": 9},
        ]),
        "failed step is still an attempted external event"
    );
}

#[test]
fn choice_and_error_in_one_dispatch_roll_back_together() {
    // 矩阵 #8：同 dispatch 内 choice（handler1）与 error（handler2）——
    // pending_choice 本就不写（错误分支早于赋值），两个 kv 键都回滚。
    const BOTH_MOD: &str = r#"
ygg.on("event.both", function()
  ygg.kv.set("first_wrote", 1)
  ygg.choice("继续？", { "go" })
end)
ygg.on("event.both", function()
  ygg.kv.set("second_wrote", 2)
  error("second handler boom")
end)
"#;
    let mut kernel = KernelFacade::new(BOTH_MOD).expect("kernel construction");
    assert_eq!(
        reply_error_code(&kernel.dispatch("event.both", "0")),
        "VALIDATION"
    );
    let state = reply_ok(&kernel.dispatch("state", ""));
    assert!(
        state["pendingChoice"].is_null(),
        "choice from failed step drops"
    );
    assert_eq!(saved_kv(&mut kernel), json!({}));
}

#[test]
fn failed_dispatch_leaves_state_hash_and_save_bytes_unchanged() {
    // 矩阵 #9：失败步对持久面零影响——状态哈希（实体面）与 save 字节
    // （含 kv 面）都不变，连续两次失败后仍不变。
    use ygg_sandbox::hash::state_hash;

    let mut kernel = KernelFacade::new(STATE_MOD).expect("kernel construction");
    reply_ok(&kernel.dispatch("event.hit", "0"));
    let hash_before = state_hash(&kernel).expect("hash");
    let bytes_before = reply_ok(&kernel.dispatch("save", ""))["save"].to_string();
    // 失败步：写实体属性 + 写 kv 后报错（同一 Mod 的命中面被包一层错误）。
    const FAILING_HIT_MOD: &str = r#"
local id = ygg.entity.spawn()
ygg.entity.set_attribute(id, "hp", 10)
ygg.on("event.hit", function()
  ygg.entity.set_attribute(id, "hp", 99)
  ygg.kv.set("hits", 99)
  error("boom")
end)
"#;
    let mut failing = KernelFacade::new(FAILING_HIT_MOD).expect("kernel construction");
    let failing_hash = state_hash(&failing).expect("hash");
    let failing_bytes = reply_ok(&failing.dispatch("save", ""))["save"].to_string();
    for _ in 0..2 {
        assert_eq!(
            reply_error_code(&failing.dispatch("event.hit", "0")),
            "VALIDATION"
        );
    }
    assert_eq!(
        state_hash(&failing).expect("hash"),
        failing_hash,
        "entity hash must be unchanged by failed dispatches"
    );
    assert_eq!(
        reply_ok(&failing.dispatch("save", ""))["save"].to_string(),
        failing_bytes,
        "save bytes (entities + kv) must be unchanged by failed dispatches"
    );
    // 参考内核未被扰动（对照：hash/save 抓取本身不改变状态）。
    assert_eq!(state_hash(&kernel).expect("hash"), hash_before);
    assert_eq!(
        reply_ok(&kernel.dispatch("save", ""))["save"].to_string(),
        bytes_before
    );
}

#[test]
fn domain_gate_rejection_does_not_touch_committed_state() {
    // 矩阵 #11（防误伤）：`ygg.choice_selected` 序号域校验门在 savepoint
    // **之前**早退——它既不该回滚，也不该被当成失败步而抹掉已提交状态。
    const GATE_KV_MOD: &str = r#"
ygg.on("event.start", function()
  ygg.kv.set("stage", "gate")
  ygg.choice("继续？", { "go", "stay" })
end)
ygg.on("ygg.choice_selected", function(index)
  ygg.kv.set("picked", index)
end)
"#;
    let mut kernel = KernelFacade::new(GATE_KV_MOD).expect("kernel construction");
    reply_ok(&kernel.dispatch("event.start", "0"));
    assert_eq!(
        reply_error_code(&kernel.dispatch("ygg.choice_selected", "9")),
        "VALIDATION"
    );
    let state = reply_ok(&kernel.dispatch("state", ""));
    assert!(
        state["pendingChoice"].is_object(),
        "pending must survive the gate"
    );
    assert_eq!(
        state["kvRevision"],
        json!(1),
        "committed step keeps its revision"
    );
    assert_eq!(saved_kv(&mut kernel), json!({"stage": "gate"}));
    // 合法序号照常落盘（域门不是「一律拒绝」）。
    reply_ok(&kernel.dispatch("ygg.choice_selected", "2"));
    assert_eq!(saved_kv(&mut kernel), json!({"picked": 2, "stage": "gate"}));
}
