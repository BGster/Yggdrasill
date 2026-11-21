use super::*;
use crate::hash::state_hash;
use crate::modules::ModuleSet;
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

// ---- TD1：结构化 meta 通道（回执 `meta` 键 / 瞬态 / 不进状态哈希）----

/// 同一套状态写；`META` 版额外调 `ygg.meta`（瞬态回执面）——
/// 两版状态哈希必须一致（meta 不入 canonical snapshot）。
const PLAIN_MOD: &str = r#"
ygg.on("event.work", function()
  ygg.narrate("干活")
  local id = ygg.entity.spawn()
  ygg.entity.set_attribute(id, "hp", 7)
  ygg.kv.set("k", "v")
end)
"#;

const META_MOD: &str = r#"
ygg.on("event.work", function()
  ygg.narrate("干活")
  local id = ygg.entity.spawn()
  ygg.entity.set_attribute(id, "hp", 7)
  ygg.kv.set("k", "v")
  ygg.meta("summary", {
    stage = "review",
    ending = false,
    pendingProposals = 2,
    conflicts = { { id = "r1", peer = "r2", note = "同一文件" } },
  })
end)

-- 嵌套 / 非 ASCII / 序列 / 空表
 ygg.on("event.shapes", function()
  ygg.meta("obj", { deep = { deeper = { 1, 2.5, true, "中文" } }, empty = {}, list = { "a", "b" } })
end)

-- 两个 meta 键是同一次 dispatch 里**两个独立槽位**（互不覆盖；Mod 侧不必写任何防御代码）
ygg.on("event.two", function()
  ygg.meta("tool_calls", { { id = "t1", tool = "git.head_short", args = { rev = "HEAD" } } })
  ygg.meta("summary", { stage = "review", nextEndsRun = false })
end)

-- 校验面：空键 / 超长键 / 非有限值都应在 Lua 侧 fail-fast（→ VALIDATION）
ygg.on("event.badkey", function()
  ygg.meta("", 1)
end)
ygg.on("event.longkey", function()
  ygg.meta("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", 1)
end)
ygg.on("event.nankey", function()
  ygg.meta("x", 0 / 0)
end)
"#;

#[test]
fn meta_lands_in_the_receipt_and_only_for_the_event_that_wrote_it() {
    let mut kernel = KernelFacade::new(META_MOD).expect("kernel construction");
    // 无 meta 的事件：信封不带 meta 键（线格式与旧版一致）
    let plain = reply(&kernel.dispatch("event.plain", "0"));
    assert!(
        plain.get("meta").is_none(),
        "未调 ygg.meta 的事件不得带 meta"
    );
    // 调了 ygg.meta 的事件：结构化字段原样进 meta
    let envelope = reply(&kernel.dispatch("event.work", "0"));
    assert_eq!(envelope["ok"], Json::Bool(true));
    assert_eq!(envelope["meta"]["summary"]["stage"], json!("review"));
    assert_eq!(envelope["meta"]["summary"]["pendingProposals"], json!(2));
    assert_eq!(
        envelope["meta"]["summary"]["conflicts"][0]["peer"],
        json!("r2")
    );
    // 叙事不含任何结构化残留（meta 与 narrative 完全分离）
    let narrative = envelope["narrative"].to_string();
    assert!(
        !narrative.contains("meta"),
        "叙事不得携带 meta：{narrative}"
    );

    // 瞬态：保留命令不产生 meta（上一条的 meta 不得残留）
    for command in ["state", "save", "log", "drain"] {
        let envelope = reply(&kernel.dispatch(command, ""));
        assert!(
            envelope.get("meta").is_none(),
            "保留命令 `{command}` 不得带 meta"
        );
    }
    // 且下一条事件也没继承上一条的 meta
    let again = reply(&kernel.dispatch("event.plain", "0"));
    assert!(again.get("meta").is_none(), "meta 不得跨 dispatch 残留");
}

#[test]
fn meta_values_round_trip_as_json_structures() {
    let mut kernel = KernelFacade::new(META_MOD).expect("kernel construction");
    let envelope = reply(&kernel.dispatch("event.shapes", "0"));
    assert_eq!(
        envelope["meta"]["obj"],
        json!({
            "deep": {"deeper": [1, 2.5, true, "中文"]},
            // 空表只能读成空序列（Lua 无空对象字面形态，已在 host_api 声明）
            "empty": [],
            "list": ["a", "b"]
        })
    );
}

#[test]
fn meta_keys_are_independent_slots_of_one_dispatch() {
    // `ygg.meta` 是 `BTreeMap<String, MetaValue>` 的按键插入：`summary` 与 `tool_calls` 分属
    // 两个槽位，信封拼装是一次性遍历，谁也不会覆盖谁。这条把「不用写防御代码」变成断言。
    let mut kernel = KernelFacade::new(META_MOD).expect("kernel construction");
    let envelope = reply(&kernel.dispatch("event.two", "0"));
    assert_eq!(envelope["meta"]["summary"]["stage"], json!("review"));
    assert_eq!(envelope["meta"]["summary"]["nextEndsRun"], json!(false));
    assert_eq!(envelope["meta"]["tool_calls"][0]["id"], json!("t1"));
    assert_eq!(
        envelope["meta"]["tool_calls"][0]["args"]["rev"],
        json!("HEAD")
    );
}

#[test]
fn meta_key_and_value_validation_fails_fast() {
    let mut kernel = KernelFacade::new(META_MOD).expect("kernel construction");
    let empty_key = kernel.dispatch("event.badkey", "0");
    assert_eq!(reply_code(&empty_key), code::VALIDATION);
    assert!(
        empty_key.contains("ygg.meta key must not be empty"),
        "{empty_key}"
    );
    let long_key = kernel.dispatch("event.longkey", "0");
    assert_eq!(reply_code(&long_key), code::VALIDATION);
    assert!(long_key.contains("too long"), "{long_key}");
    let nan = kernel.dispatch("event.nankey", "0");
    assert_eq!(reply_code(&nan), code::VALIDATION);
    assert!(nan.contains("non-finite"), "{nan}");
    // 拒绝不污染后续：同进程再派发仍正常
    let envelope = reply(&kernel.dispatch("event.work", "0"));
    assert_eq!(envelope["ok"], Json::Bool(true));
    assert!(envelope["meta"]["summary"].is_object());
}

/// 确定性硬要求：同一事件序列，调不调 `ygg.meta` 状态哈希必须相同；
/// meta 也不得进存档（`save` 的字节里没有它）。
#[test]
fn meta_never_enters_state_hash_or_save() {
    let mut plain = KernelFacade::new(PLAIN_MOD).expect("plain kernel");
    let mut with_meta = KernelFacade::new(META_MOD).expect("meta kernel");
    for _ in 0..3 {
        assert_eq!(
            reply(&plain.dispatch("event.work", "0"))["ok"],
            Json::Bool(true)
        );
        assert_eq!(
            reply(&with_meta.dispatch("event.work", "0"))["ok"],
            Json::Bool(true)
        );
    }
    assert_eq!(
        state_hash(&plain).expect("plain hash"),
        state_hash(&with_meta).expect("meta hash"),
        "meta 调用与否不得改变 canonical 状态哈希"
    );
    let save = with_meta.dispatch("save", "");
    assert!(!save.contains("\"meta\""), "meta 不得进存档：{save}");
    // 存档往返后哈希仍一致（瞬态面不参与持久化）
    let envelope = reply(&save);
    let restored =
        KernelFacade::restore(envelope["save"].to_string().as_bytes(), META_MOD).expect("restore");
    assert_eq!(
        state_hash(&plain).expect("plain hash"),
        state_hash(&restored).expect("restored hash")
    );
}

// ---- dispatch 级 savepoint（实体面 + KV 面整体提交/回滚）----

/// 半写 Mod：handler 先写实体（spawn + set_attribute）与 KV 再报错；
/// `event.ok` 同形但不报错（提交对照组），`event.spawn` 暴露 next_id。
const SAVEPOINT_MOD: &str = r#"
ygg.on("event.ok", function()
  ygg.kv.set("clean", "yes")
end)
ygg.on("event.bad", function()
  local id = ygg.entity.spawn()
  ygg.entity.set_attribute(id, "hp", 1)
  ygg.kv.set("dirty", "yes")
  error("boom")
end)
ygg.on("event.spawn", function()
  ygg.narrate("id=" .. tostring(ygg.entity.spawn()))
end)
"#;

#[test]
fn failed_dispatch_restores_the_canonical_baseline() {
    // 驱动内部状态机：取点冻结基线 → 失败整体还原（实体面与 KV 面）。
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
    // 实体面：canonical 字节逐字节回到基线（含 revision / next_entity_id）。
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
        Json::from(0),
        "entity revision rolls back"
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
    // 本次写入，`ygg.kv.get` / `ygg.entity.get_attribute` 的读路径看到的
    // 是自己的写（若改成 mem::swap 换出，此测必红——读路径直读 canonical）。
    const READ_BACK_MOD: &str = r#"
ygg.on("event.rb", function()
  ygg.kv.set("k", "v")
  local id = ygg.entity.spawn()
  ygg.entity.set_attribute(id, "hp", 5)
  local bumped = ygg.entity.get_attribute(id, "hp") + 1
  ygg.entity.set_attribute(id, "hp", bumped)
  ygg.narrate("kv=" .. tostring(ygg.kv.get("k")) .. " hp=" .. tostring(ygg.entity.get_attribute(id, "hp")))
end)
"#;
    let mut kernel = KernelFacade::new(READ_BACK_MOD).expect("kernel construction");
    let out = reply(&kernel.dispatch("event.rb", "0"));
    assert_eq!(out["narrative"], json!(["kv=v hp=6"]));
    // 成功步提交后同样可读（canonical 就是写入目标）。
    let save = reply(&kernel.dispatch("save", ""));
    assert_eq!(save["save"]["kv"]["entries"]["k"], json!("v"));
    assert_eq!(
        save["save"]["entities"]["entities"]["1"]["attributes"]["hp"],
        json!(6)
    );
}

// ---- Part B：`ygg.fs` 确定性 VFS（写 / 读 / stat / list + savepoint 回滚）----

/// VFS 全集 Mod：四个事件分别打写 / 读 / stat / list 面，叙事即断言载体。
///
/// 沙箱裁掉了 `string` / `table` / `math`（`BASE_WHITELIST` 只留 base 纯计算子集），
/// 所以这里只能用 `#` / `..` / `tostring` / `type`——别顺手写 `string.len`。
const VFS_MOD: &str = r#"
ygg.on("event.write", function()
  local b = ygg.fs.write("artifacts/b.md", "second")
  local a = ygg.fs.write("artifacts/a.md", "first")
  local sb, sa = b.sha256, a.sha256
  ygg.narrate("w=" .. b.path .. ":" .. tostring(b.bytes) .. ":" .. tostring(#sb))
  ygg.narrate("w=" .. a.path .. ":" .. tostring(a.bytes) .. ":" .. tostring(#sa))
end)
ygg.on("event.read", function()
  ygg.narrate("hit=" .. tostring(ygg.fs.read("artifacts/a.md")))
  ygg.narrate("miss=" .. tostring(ygg.fs.read("artifacts/none.md")))
  ygg.narrate("misstype=" .. type(ygg.fs.read("artifacts/none.md")))
end)
ygg.on("event.stat", function()
  local hit = ygg.fs.stat("artifacts/a.md")
  local miss = ygg.fs.stat("artifacts/none.md")
  local hs, ms = hit.sha256, miss.sha256
  ygg.narrate("hit=" .. tostring(hit.exists) .. ":" .. tostring(hit.bytes)
    .. ":" .. tostring(#hs))
  ygg.narrate("miss=" .. tostring(miss.exists) .. ":" .. tostring(miss.bytes)
    .. ":" .. tostring(#ms) .. ":" .. type(miss))
end)
ygg.on("event.list", function()
  local rows = ygg.fs.list()
  local acc = ""
  for i = 1, #rows do
    local sha = rows[i].sha256
    if i > 1 then acc = acc .. "," end
    acc = acc .. rows[i].path .. "@" .. tostring(rows[i].bytes) .. ":" .. tostring(#sha)
  end
  ygg.narrate("list=" .. tostring(#rows) .. ":" .. acc)
end)
"#;

#[test]
fn fs_write_read_stat_list_roundtrip() {
    let mut kernel = KernelFacade::new(VFS_MOD).expect("kernel construction");
    let written = reply(&kernel.dispatch("event.write", "0"));
    assert_eq!(written["ok"], Json::Bool(true), "envelope={written}");
    // write 回执：path / bytes / sha256（64 字符）——长度在叙事里断言。
    assert_eq!(
        written["narrative"],
        json!(["w=artifacts/b.md:6:64", "w=artifacts/a.md:5:64"])
    );

    // read：命中回原文，miss 回 nil（不 throw）。
    let read = reply(&kernel.dispatch("event.read", "0"));
    assert_eq!(
        read["narrative"],
        json!(["hit=first", "miss=nil", "misstype=nil"])
    );

    // stat：**恒回表**（miss 也是表，字段齐全）。
    let stat = reply(&kernel.dispatch("event.stat", "0"));
    assert_eq!(
        stat["narrative"],
        json!(["hit=true:5:64", "miss=false:0:0:table"])
    );

    // list：稠密数组（`#rows` 可数），按 path 升序。
    let list = reply(&kernel.dispatch("event.list", "0"));
    assert_eq!(
        list["narrative"],
        json!(["list=2:artifacts/a.md@5:64,artifacts/b.md@6:64"])
    );
}

/// 同序列两次 → 同字节（确定性锤子：`BTreeMap` 序 + `sha2` 确定性）。
#[test]
fn fs_list_is_deterministic_across_repeated_dispatch() {
    let mut first = KernelFacade::new(VFS_MOD).expect("kernel construction");
    let mut second = KernelFacade::new(VFS_MOD).expect("kernel construction");
    for kernel in [&mut first, &mut second] {
        reply(&kernel.dispatch("event.write", "0"));
        reply(&kernel.dispatch("event.list", "0"));
    }
    let a = reply(&first.dispatch("event.list", "0"));
    let b = reply(&second.dispatch("event.list", "0"));
    assert_eq!(a["narrative"], b["narrative"], "list 回执必须同字节");
    assert_eq!(first.vfs_entries(), second.vfs_entries());
    assert_eq!(first.vfs_revision(), second.vfs_revision());
}

/// 成功派发：写入留在 VFS 且入 dirty 水位；排空后二次排空为空。
#[test]
fn successful_dispatch_keeps_vfs_and_marks_dirty() {
    let mut kernel = KernelFacade::new(VFS_MOD).expect("kernel construction");
    reply(&kernel.dispatch("event.write", "0"));
    assert_eq!(kernel.vfs_len(), 2);
    assert_eq!(kernel.vfs_revision(), 2, "两次成功 put 各 +1");

    let dirty = kernel.vfs_take_dirty();
    assert_eq!(
        dirty.iter().map(|(p, _, _)| p.as_str()).collect::<Vec<_>>(),
        vec!["artifacts/a.md", "artifacts/b.md"],
        "dirty 必须按 BTreeMap 序排空"
    );
    assert_eq!(dirty[0].1, b"first".to_vec(), "带出的是内容字节");
    assert!(kernel.vfs_take_dirty().is_empty(), "排空后不得重复上报");
    assert_eq!(kernel.vfs_len(), 2, "排空 dirty 不删文件");
}

/// **savepoint 回滚的 VFS 面**：失败步里写的文件必须一起回滚，且不得入
/// dirty（否则一次失败会在下一轮把半个状态落盘）。
#[test]
fn failed_dispatch_rolls_back_vfs() {
    const HALF_WRITE_MOD: &str = r#"
ygg.on("event.ok", function()
  ygg.fs.write("artifacts/clean.md", "keep")
end)
ygg.on("event.bad", function()
  ygg.fs.write("artifacts/dirty.md", "leak")
  error("boom")
end)
"#;
    let mut kernel = KernelFacade::new(HALF_WRITE_MOD).expect("kernel construction");
    // 先提交一个非空基线（否则「回滚到基线」与「清空」不可区分）。
    let ok = reply(&kernel.dispatch("event.ok", "0"));
    assert_eq!(ok["ok"], Json::Bool(true));
    let _ = kernel.vfs_take_dirty(); // 排空基线写入，只观察失败步的增量
    let baseline_revision = kernel.vfs_revision();

    assert_eq!(
        reply_code(&kernel.dispatch("event.bad", "0")),
        code::VALIDATION
    );
    // 文件面：失败步写的键不在表里，基线件还在。
    assert_eq!(kernel.vfs_len(), 1, "失败步的文件不得留下");
    assert_eq!(
        kernel
            .vfs_entries()
            .iter()
            .map(|(path, _, _)| path.as_str())
            .collect::<Vec<_>>(),
        vec!["artifacts/clean.md"]
    );
    // 水位面：dirty 与 revision 一起回滚。
    assert!(
        kernel.vfs_take_dirty().is_empty(),
        "失败步不得留下待落盘条目"
    );
    assert_eq!(
        kernel.vfs_revision(),
        baseline_revision,
        "revision 随基线回滚"
    );
}

/// 路径 / 内容门在 Lua 调用点 fail-fast（`ygg.fs.write` 当场报错，
/// 不进 `files`、不动 revision）——与 `ygg_store` 的端口门同规则。
#[test]
fn fs_write_path_gate_rejects_at_the_call_site() {
    let mut kernel = KernelFacade::new(IDLE_MOD).expect("kernel construction");
    for bad in [
        "artifact/x.md",
        "artifacts/",
        "artifacts//x",
        "artifacts/../x",
        "/artifacts/x",
        "artifacts/a b.md",
        "artifacts/a|b.md",
        "artifacts/a@b.md",
    ] {
        let err = kernel
            .vfs_put(bad, b"x")
            .expect_err(&format!("`{bad}` 必须被拒"));
        assert!(err.contains("ygg.fs.write 拒绝："), "必须点名写面：{err}");
    }
    // 内容上限（256 KiB）：恰好上限放行，多一字节即拒。
    let exactly = vec![0u8; 256 * 1024];
    assert!(kernel.vfs_put("artifacts/max.md", &exactly).is_ok());
    let over = vec![0u8; 256 * 1024 + 1];
    assert!(kernel.vfs_put("artifacts/over.md", &over).is_err());
    // 被拒的写不留下任何痕迹（revision 只记成功的两次）。
    assert_eq!(kernel.vfs_len(), 1);
    assert_eq!(kernel.vfs_revision(), 1);
}

/// 宿主预载：`vfs_load` **不**入 dirty、**不**动 revision（不是本会话的写）。
#[test]
fn vfs_load_does_not_dirty_or_bump_revision() {
    let mut kernel = KernelFacade::new(IDLE_MOD).expect("kernel construction");
    kernel.vfs_load("artifacts/pre.md", b"preset", "deadbeef");
    assert_eq!(kernel.vfs_len(), 1);
    assert_eq!(kernel.vfs_revision(), 0, "预载不得当成本次写入");
    assert!(kernel.vfs_take_dirty().is_empty(), "预载不得入 dirty");
    // 预载件参与 list / 诊断面。
    assert_eq!(
        kernel.vfs_entries()[0],
        (
            "artifacts/pre.md".to_owned(),
            b"preset".to_vec(),
            "deadbeef".to_owned()
        )
    );
}

/// 多文件 Mod 支持面：`ModuleSet` 由 `(规范名, 源码)` 构造——名字是点分
/// 规范名（`src.writer`），不是路径（`src/writer.lua` 由 `path_of` 派生）。
/// 下面两个用例共用。
fn module_set(pairs: &[(&str, &str)]) -> ModuleSet {
    ModuleSet::from_pairs(
        pairs
            .iter()
            .map(|(name, code)| ((*name).to_owned(), code.as_bytes().to_vec())),
    )
    .expect("登记模块")
}

/// **模块顶层禁令**：`ygg.fs.write` 与 `ygg.kv.set` 同类（写宿主状态），
/// 模块 chunk 顶层就被影子表拒绍，报错带调用点与合法写法。
#[test]
fn fs_write_is_banned_at_module_top_level() {
    let modules = module_set(&[("src.writer", "ygg.fs.write(\"artifacts/x.md\", \"y\")\n")]);
    let err = match KernelFacade::with_modules("require(\"src.writer\")", modules) {
        Ok(_) => panic!("模块顶层写 ygg.fs.write 必须被拒"),
        Err(err) => err,
    };
    let text = err.to_string();
    for fragment in ["ygg.fs.write", "src/writer.lua", "模块顶层"] {
        assert!(
            text.contains(fragment),
            "错误文本缺少「{fragment}」：{text}"
        );
    }
}

/// 纯读面（`fs.read` / `fs.stat` / `fs.list`）在模块顶层**合法**——
/// 禁令只压写面，否则「模块顶层读一下自己上次的产物」这种正当用法会被误杀。
#[test]
fn fs_reads_are_allowed_at_module_top_level() {
    let modules = module_set(&[(
        "src.reader",
        "local v = ygg.fs.read(\"artifacts/x.md\")\n\
         local s = ygg.fs.stat(\"artifacts/x.md\")\n\
         return { n = #ygg.fs.list(), nil_read = (v == nil), exists = s.exists }\n",
    )]);
    let mut kernel = KernelFacade::with_modules(
        "local r = require(\"src.reader\")\n\
         ygg.on(\"event.peek\", function()\n\
           ygg.narrate(\"n=\" .. tostring(r.n) .. \" nil=\" .. tostring(r.nil_read) .. \" exists=\" .. tostring(r.exists))\n\
         end)",
        modules,
    )
    .expect("纯读面必须放行");
    let out = reply(&kernel.dispatch("event.peek", "0"));
    assert_eq!(out["narrative"], json!(["n=0 nil=true exists=false"]));
}
