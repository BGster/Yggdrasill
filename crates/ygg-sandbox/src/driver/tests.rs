use super::*;
use serde_json::json;

fn reply_code(reply: &str) -> String {
    let parsed: Json = serde_json::from_str(reply).unwrap();
    parsed["error"]["code"].as_str().unwrap_or("").to_owned()
}

fn reply(reply: &str) -> Json {
    serde_json::from_str(reply).unwrap()
}

/// 空转 Mod：无任何 Mod 注册（仅 stdlib 内置注册 `ygg.narrative.goto`）
/// ——未注册名预检的最小环境。
const IDLE_MOD: &str = "-- idle mod: no handlers";

/// 最小注册 Mod：一个无操作 handler——成功信封形态断言必须来自
/// **已注册**事件（fail-closed 后未注册名不再有成功信封）。
const NOOP_MOD: &str = r#"ygg.on("event.ok", function() end)"#;

#[test]
fn envelope_error_shape_uses_stable_codes() {
    let mut kernel = KernelFacade::new(NOOP_MOD).expect("kernel construction");
    // 非标量 payload → VALIDATION（payload 标量门；事件已注册，走的是
    // 门不是预检）。
    let reply = kernel.dispatch("event.ok", r#"{"nested":{"deep":1}}"#);
    assert_eq!(reply_code(&reply), code::VALIDATION);
    // 保留命令名 + 非法存档 → VALIDATION。
    let reply = kernel.dispatch("load", "not json");
    assert_eq!(reply_code(&reply), code::VALIDATION);
    // 未注册事件名 → VALIDATION（预检，零副作用；见下一用例）。
    let reply = kernel.dispatch("event.any", "0");
    assert_eq!(reply_code(&reply), code::VALIDATION);
    // 成功信封无 error 字段（已注册事件）。
    let reply = kernel.dispatch("event.ok", "0");
    let parsed: Json = serde_json::from_str(&reply).unwrap();
    assert_eq!(parsed["ok"], Json::Bool(true));
    assert!(parsed.get("error").is_none());
}

// ---- 未注册事件 fail-closed（门面预检 T5 / 派生面拒+回滚 T6）----

/// T5：预检拒绝零副作用——不进外部日志（回放日志零污染）、不动状态
/// 基线（revision / kvRevision / pendingChoice 原样），且文案三要素
/// 齐全（事件名 + 计数 + 字典序全集）；拒绝后 registry 未受损，已注册
/// 事件照常可用。空注册表（IDLE_MOD）文案同构（「已注册 0 个事件：」）。
#[test]
fn precheck_rejects_unknown_event_with_zero_side_effects() {
    let mut kernel = KernelFacade::new(SAVEPOINT_MOD).expect("kernel construction");
    let rejected = kernel.dispatch("event.nope", "0");
    assert_eq!(reply_code(&rejected), code::VALIDATION);
    let parsed = reply(&rejected);
    let message = parsed["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("`event.nope`"),
        "文案要点名事件：{message}"
    );
    assert!(
        message.contains("已注册 4 个事件"),
        "文案要带计数（3 个 Mod 注册 + 1 个 stdlib 内置）：{message}"
    );
    assert_eq!(
        message,
        "未注册事件 `event.nope`：本 Mod 已注册 4 个事件：event.bad、event.ok、event.spawn、ygg.narrative.goto",
        "文案三要素（名字 + 计数 + 字典序全集）必须逐字节确定（含 stdlib 内置注册 ygg.narrative.goto）：{message}"
    );
    // 零副作用 ①：外部日志（回放依据）不得被污染。
    let log = reply(&kernel.dispatch("log", ""));
    assert_eq!(log["log"], json!([]), "预检拒绝不得进回放日志：{log}");
    // 零副作用 ②：状态基线原样（revision / kvRevision 均 0，无待决选择）。
    let state = reply(&kernel.dispatch("state", ""));
    assert_eq!(state["revision"], Json::from(0));
    assert_eq!(state["kvRevision"], Json::from(0));
    assert!(state["pendingChoice"].is_null());
    // 零副作用 ③：registry 未受损，已注册事件照常可用。
    let ok = reply(&kernel.dispatch("event.ok", "0"));
    assert_eq!(ok["ok"], Json::Bool(true), "预检不得损伤已注册面：{ok}");
    // 空注册表边缘实际不可达：stdlib 恒注册 `ygg.narrative.goto`（内置面
    // 也算成员）——文案如实反映运行时 registry，这正是「registry 是唯一
    // 事实」的体现：不特判、不白名单、内置注册同样计入全集。
    let mut idle = KernelFacade::new(IDLE_MOD).expect("kernel construction");
    let idle_reply = idle.dispatch("event.any", "0");
    assert_eq!(reply_code(&idle_reply), code::VALIDATION);
    assert_eq!(
        reply(&idle_reply)["error"]["message"],
        json!("未注册事件 `event.any`：本 Mod 已注册 1 个事件：ygg.narrative.goto"),
        "仅 stdlib 内置注册的空 Mod：文案如实反映 registry：{idle_reply}"
    );
}

/// T6：派生面 fail-closed——handler `ygg.emit` 未注册名，整次转换失败
/// 回滚（bus Err → TransitionError::Handler → savepoint 还原），错误码
/// 显式分支钉在 VALIDATION，消息带 bus 稳定前缀 + 派生名。
#[test]
fn derived_unregistered_event_fails_and_rolls_back_the_transition() {
    const DERIVED_MISS_MOD: &str = r#"
ygg.on("event.ok", function()
  ygg.kv.set("clean", "yes")
end)
ygg.on("event.mis.emit", function()
  ygg.kv.set("dirty", "yes")
  ygg.emit("event.never.registered", 0)
end)
"#;
    let mut kernel = KernelFacade::new(DERIVED_MISS_MOD).expect("kernel construction");
    // 先提交一个非空基线（否则「回滚」与「清空」不可区分）。
    let ok = reply(&kernel.dispatch("event.ok", "0"));
    assert_eq!(ok["ok"], Json::Bool(true));
    let baseline_save = reply(&kernel.dispatch("save", ""))["save"].to_string();

    let failed = reply(&kernel.dispatch("event.mis.emit", "0"));
    assert_eq!(failed["ok"], Json::Bool(false), "{failed}");
    assert_eq!(failed["error"]["code"], code::VALIDATION);
    let message = failed["error"]["message"].as_str().unwrap();
    assert!(
        message.starts_with("unregistered event `event.never.registered`"),
        "派生面消息带 bus 稳定前缀 + 派生名：{message}"
    );
    // 回滚：失败步的 kv 写（dirty）不得留下，基线字节逐字节保持。
    assert_eq!(
        reply(&kernel.dispatch("save", ""))["save"].to_string(),
        baseline_save,
        "派生未注册必须整转换回滚"
    );
}

#[test]
fn drain_is_always_empty_and_state_reports_diagnostic_fields() {
    let mut kernel = KernelFacade::new(IDLE_MOD).expect("kernel construction");
    let reply = kernel.dispatch("drain", "");
    let parsed: Json = serde_json::from_str(&reply).unwrap();
    assert_eq!(parsed["drained"], Json::Array(vec![]));
    let reply = kernel.dispatch("state", "");
    let parsed: Json = serde_json::from_str(&reply).unwrap();
    assert_eq!(parsed["engineVersion"], ENGINE_VERSION);
    assert_eq!(parsed["revision"], 0);
    assert_eq!(parsed["kvRevision"], 0);
    assert!(parsed["pendingChoice"].is_null());
    assert!(parsed["snapshotBytes"].is_number());
    // log 初始为空。
    let reply = kernel.dispatch("log", "");
    let parsed: Json = serde_json::from_str(&reply).unwrap();
    assert_eq!(parsed["log"], Json::Array(vec![]));
}

#[test]
fn engine_mismatch_is_rejected_on_restore() {
    let mut kernel = KernelFacade::new(IDLE_MOD).expect("kernel construction");
    let reply = kernel.dispatch("save", "");
    // 信封内嵌 save 对象才是存档本体（信封键序 = serde_json 默认字典序）。
    let envelope: Json = serde_json::from_str(&reply).unwrap();
    let save = envelope["save"].to_string();
    // from_bytes 只校验 format_version；engine 门在 restore 的 apply_save
    // （存档 JSON 的字段值即字面替换锚点）。
    let tampered = save.replace(
        &format!("\"engine_version\":\"{ENGINE_VERSION}\""),
        "\"engine_version\":\"0.0.0-foreign\"",
    );
    let outcome = KernelFacade::restore(tampered.as_bytes(), IDLE_MOD);
    assert!(
        matches!(
            outcome,
            Err(KernelError::EngineMismatch { ref saved, .. }) if saved == "0.0.0-foreign"
        ),
        "engine mismatch must be rejected (restore must not succeed)"
    );
}

const SAVEPOINT_MOD: &str = r#"
world.component{ name = "hp", kind = "i64" }
ygg.on("event.ok", function()
  ygg.kv.set("clean", "yes")
end)
ygg.on("event.bad", function()
  world.spawn{ hp = 1 }
  ygg.kv.set("dirty", "yes")
  error("boom")
end)
ygg.on("event.spawn", function()
  ygg.narrate("id=" .. tostring(world.spawn{ hp = 0 }))
end)
"#;

#[test]
fn failed_dispatch_restores_the_canonical_baseline() {
    // 驱动内部状态机：取点冻结基线 → 失败整体还原（世界面与 KV 面）。
    let mut kernel = KernelFacade::new(SAVEPOINT_MOD).expect("kernel construction");
    // 先提交一个非空基线（否则「还原到基线」与「清空」不可区分）。
    let ok = reply(&kernel.dispatch("event.ok", "0"));
    assert_eq!(ok["ok"], Json::Bool(true));
    let baseline_bytes = kernel.canonical_state_bytes().expect("bytes");
    let baseline_save = reply(&kernel.dispatch("save", ""))["save"].to_string();
    assert_eq!(
        reply_code(&kernel.dispatch("event.bad", "0")),
        code::VALIDATION
    );
    // 世界面：canonical 字节逐字节回到基线（含 tick / next_entity / 组件段）。
    assert_eq!(
        kernel.canonical_state_bytes().expect("bytes"),
        baseline_bytes
    );
    // KV 面不在 canonical 快照里（hash.rs 覆盖面声明），读 save 回执。
    assert_eq!(
        reply(&kernel.dispatch("save", ""))["save"].to_string(),
        baseline_save,
        "failed dispatch must not change the save bytes"
    );
    let state = reply(&kernel.dispatch("state", ""));
    assert_eq!(
        state["revision"],
        Json::from(1),
        "commit counter of the committed step only"
    );
    assert_eq!(
        state["kvRevision"],
        Json::from(1),
        "kv revision keeps commit"
    );
    // 提交路径未被改动：成功步的写入留在 canonical。
    let ok_again = reply(&kernel.dispatch("event.ok", "0"));
    assert_eq!(ok_again["ok"], Json::Bool(true));
    let save = reply(&kernel.dispatch("save", ""));
    assert_eq!(save["save"]["kv"]["entries"]["clean"], json!("yes"));
}

#[test]
fn rollback_rewinds_next_entity_id() {
    // 回滚连 `next_entity_id` 一起退：失败步 spawn 掉的 id 不得被占用。
    let mut kernel = KernelFacade::new(SAVEPOINT_MOD).expect("kernel construction");
    assert_eq!(
        reply_code(&kernel.dispatch("event.bad", "0")),
        code::VALIDATION
    );
    let spawned = reply(&kernel.dispatch("event.spawn", "0"));
    assert_eq!(spawned["narrative"], json!(["id=1"]));
}

#[test]
fn handler_reads_its_own_writes_during_the_dispatch() {
    // savepoint 基线是**副本**而非换出：转换期间 canonical 保留原状态 +
    // 本次写入，`ygg.kv.get` / `world.get` 的读路径看到的是自己的写
    // （若改成 mem::swap 换出，此测必红——读路径直读 canonical）。
    const READ_BACK_MOD: &str = r#"
world.component{ name = "hp", kind = "i64" }
ygg.on("event.rb", function()
  ygg.kv.set("k", "v")
  local id = world.spawn{ hp = 5 }
  local bumped = world.get(id, "hp") + 1
  world.set(id, { hp = bumped })
  ygg.narrate("kv=" .. tostring(ygg.kv.get("k")) .. " hp=" .. tostring(world.get(id, "hp")))
end)
"#;
    let mut kernel = KernelFacade::new(READ_BACK_MOD).expect("kernel construction");
    let out = reply(&kernel.dispatch("event.rb", "0"));
    assert_eq!(out["narrative"], json!(["kv=v hp=6"]));
    // 成功步提交后同样可读（canonical 就是写入目标）。
    let save = reply(&kernel.dispatch("save", ""));
    assert_eq!(save["save"]["kv"]["entries"]["k"], json!("v"));
    assert_eq!(
        save["save"]["world"]
            .as_array()
            .map(|frame| frame.len() > 32),
        Some(true),
        "存档携带世界帧（不再有行存 entities 表）"
    );
}
