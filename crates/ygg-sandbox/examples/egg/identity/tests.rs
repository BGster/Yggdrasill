use super::*;

/// 用例专属临时目录（互不共享；先清空再建）。
fn case_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ygg-identity-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 最小合成 Mod：proposal 路由声明两布尔 + 一个透传事件（未声明身份段）+
/// 一个必败事件（mod 侧拒绝路径）。
fn write_identity_mod(dir: &Path) {
    fs::create_dir_all(dir.join("data")).expect("建 data 目录");
    let pack = json!({
        "schemaVersion": 1,
        "entries": [],
        "schemaFiles": [],
        "agent": {
            "entryEvent": "event.start",
            "choiceEvent": "ygg.choice_selected",
            "summaryEvent": "agent.summary",
            "endingPrefix": "【结局",
            "metaKey": "summary",
            "rejectionPrefixes": ["【拒绝】"],
            "replayPrefix": "【重放】",
            "defaultPayload": "0"
        },
        "inboxRoutes": {
            "externalIdField": "external_id",
            "payloadField": "payload",
            "extMarker": "@ext=",
            "proposal": {
                "action": "proposal.submit",
                "fields": ["req", "rev", "stage", "summary", "artifacts", "changes", "external_id"],
                "actor": true,
                "actor_session": true,
                "write": {"flag": "proposal", "shape": "payload"}
            },
            "tool_result": {
                "action": "tool.result",
                "fields": ["call_id", "tool_id", "status", "code", "output", "external_id"],
                "write": {"flag": "tool-result", "shape": "payload"}
            }
        }
    });
    write_json_pretty(&dir.join(PACK_FILE), &pack).expect("写 pack.json");
    fs::write(
        dir.join(MANIFEST_FILE),
        r#"{"schemaVersion": 1, "entries": [], "code": []}"#,
    )
    .expect("写 manifest.json");
    fs::write(
        dir.join("mod.lua"),
        r#"ygg.on("event.start", function()
  ygg.narrate("开局")
end)
ygg.on("event.echo", function(payload)
  ygg.kv.set("echoed", tostring(payload))
  ygg.narrate("回显：" .. tostring(payload))
end)
ygg.on("event.boom", function()
  error("boom")
end)
ygg.on("proposal.submit", function(payload)
  ygg.kv.set("proposal_payload", tostring(payload))
  ygg.narrate("提案载荷：" .. tostring(payload))
end)
ygg.on("tool.result", function(payload)
  ygg.kv.set("last_tool_payload", tostring(payload))
  ygg.narrate("工具结果：" .. tostring(payload))
end)
"#,
    )
    .expect("写 mod.lua");
    let config = json!({
        "drive": {"maxSteps": 50, "stallLimit": 5, "stepsPerRequirement": 20},
        "settlement": {
            "requireEvents": true,
            "requireNoFailedSteps": true,
            "requireEnding": true,
            "requireInboxConsumed": true
        },
    });
    write_json_pretty(&dir.join("data").join("config.json"), &config).expect("写 config.json");
}

/// 本模块的 Args 基底（其余字段钉成「单发新局」；身份两字段按调用方给）。
fn base_args(
    mod_dir: &Path,
    runs_root: &Path,
    run_id: &str,
    actor: Option<String>,
    session: Option<String>,
) -> Args {
    Args {
        packages: None,
        mod_dir: Some(mod_dir.to_path_buf()),
        open_style: Some(crate::OpenStyle::New),
        run_root: runs_root.join(run_id),
        runs_parent: runs_root.to_path_buf(),
        force: false,
        at: None,
        settle: false,
        watch: false,
        event: None,
        payload: None,
        drive: Drive::None,
        inbox: None,
        store: Some(ygg_store::Backend::Json),
        wait_tools: false,
        tool_timeout_ms: None,
        run_tool_worker: None,
        artifact_put: None,
        artifact_get: None,
        builtin_tool: None,
        daemon: false,
        harness: None,
        actor,
        actor_session: session,
        output: crate::OutputFormat::Text,
        serve: false,
        port: None,
        panel_dir: None,
        poll_ms: None,
    }
}

fn open_session(dir: &Path, tag: &str, actor: Option<&str>, session: Option<&str>) -> Session {
    write_identity_mod(dir);
    let runs_root = dir.join("runs");
    let args = base_args(
        dir,
        &runs_root,
        tag,
        actor.map(str::to_owned),
        session.map(str::to_owned),
    );
    Session::open(&args).expect("开局")
}

/// run.json 的 opener_actor（未认领 = None）。
fn opener_of(run_dir: &Path) -> Option<String> {
    read_meta(&run_dir.join(META_FILE))?["opener_actor"]["actor"]
        .as_str()
        .map(str::to_owned)
}

// ── 形状与解析链 ─────────────────────────────────────────────

#[test]
fn identity_value_shape_rejects_bad_values() {
    assert!(validate_identity_value("--actor", "agent-1", 64).is_ok());
    assert!(validate_identity_value("--actor", "  agent-1  ", 64).is_ok()); // trim 后合法
    assert!(
        validate_identity_value("--actor", "", 64).is_err(),
        "空值必拒"
    );
    assert!(
        validate_identity_value("--actor", "a b", 64).is_err(),
        "空白必拒"
    );
    assert!(
        validate_identity_value("--actor", "a|b", 64).is_err(),
        "竖线必拒"
    );
    assert!(
        validate_identity_value("--actor", &"x".repeat(65), 64).is_err(),
        "超长必拒"
    );
    assert!(validate_identity_value("--actor-session", &"x".repeat(128), 128).is_ok());
    assert!(validate_identity_value("--actor-session", &"x".repeat(129), 128).is_err());
}

#[test]
fn identity_value_flag_beats_env_and_blank_env_is_absent() {
    let flag = "flag-id".to_owned();
    // 旗标 > 环境
    assert_eq!(
        identity_value_from(Some(&flag), Some("env-id".to_owned()), "--actor", 64)
            .expect("旗标合法"),
        Some("flag-id".to_owned())
    );
    // 无旗标 → 环境
    assert_eq!(
        identity_value_from(None, Some("env-id".to_owned()), "--actor", 64).expect("环境合法"),
        Some("env-id".to_owned())
    );
    // 环境空白 = 未设置
    assert_eq!(
        identity_value_from(None, Some("   ".to_owned()), "--actor", 64).expect("空白视为缺省"),
        None
    );
    // 都缺 → None
    assert_eq!(
        identity_value_from(None, None, "--actor", 64).expect("都缺视为缺省"),
        None
    );
    // 非法环境值 = 错误（不是静默无身份）
    assert!(identity_value_from(None, Some("a b".to_owned()), "--actor", 64).is_err());
}

#[test]
fn identity_segments_inject_in_fixed_relative_order_and_independently() {
    // 载荷是 JSON 字符串标量：身份段插进闭引号**之前**（内核解析仍是合法 JSON）。
    // 双段：actor 在前
    assert_eq!(
        inject_identity_segments(
            "\"r1|1|plan|s|a.md||x\"",
            Some("agent-1"),
            Some("s-1"),
            true,
            true
        ),
        "\"r1|1|plan|s|a.md||x|actor=agent-1|actor_session=s-1\""
    );
    // 只声明 actor（未声明 session 即使携带也不注入）
    assert_eq!(
        inject_identity_segments("\"a\"", Some("agent-1"), Some("s-1"), true, false),
        "\"a|actor=agent-1\""
    );
    // 只声明 session
    assert_eq!(
        inject_identity_segments("\"a\"", Some("agent-1"), Some("s-1"), false, true),
        "\"a|actor_session=s-1\""
    );
    // 声明但未携带 → 原样
    assert_eq!(
        inject_identity_segments("\"a\"", None, None, true, true),
        "\"a\""
    );
    // 未声明 → 原样（宿主不碰载荷）
    assert_eq!(
        inject_identity_segments("\"a\"", Some("agent-1"), None, false, false),
        "\"a\""
    );
    // 非字符串标量（数字）：无处可插，原样返回
    assert_eq!(
        inject_identity_segments("0", Some("agent-1"), None, true, true),
        "0"
    );
}

#[test]
fn daemon_request_identity_fills_only_when_base_absent() {
    let base = ActorIdentity {
        actor: Some("base-actor".to_owned()),
        session: None,
    };
    let req = json!({"actor": "req-actor", "actor_session": "req-session"});
    // 链：旗标 > 环境（已折进 base） > 请求字段 —— base.actor 优先，session 由请求补
    let merged = merge_request_identity(&base, &req).expect("合并合法");
    assert_eq!(merged.actor.as_deref(), Some("base-actor"));
    assert_eq!(merged.session.as_deref(), Some("req-session"));
    // base 全缺 → 请求字段兜底
    let merged = merge_request_identity(&ActorIdentity::NONE, &req).expect("合并合法");
    assert_eq!(merged.actor.as_deref(), Some("req-actor"));
    assert_eq!(merged.session.as_deref(), Some("req-session"));
    // 请求字段形状非法 → 可读错误
    let bad = json!({"actor": "a b"});
    assert!(merge_request_identity(&ActorIdentity::NONE, &bad).is_err());
}

// ── 伪造拦截与注入（Session 级） ─────────────────────────────

#[test]
fn forged_actor_segment_is_rejected_without_dispatch_or_persist() {
    let dir = case_dir("forged");
    let mut session = open_session(&dir, "forged-run", Some("agent-1"), None);
    let before = session.count_events();
    let receipt = session.dispatch_event_as(
        "proposal.submit",
        "\"r1|1|plan|s|artifacts/p.md||x|actor=fake\"",
        Some("agent-1"),
        None,
    );
    assert_eq!(receipt["ok"], Json::Bool(false), "必须拒绝：{receipt}");
    assert_eq!(receipt["error"]["code"], json!("ACTOR_FORGED"));
    assert_eq!(session.count_events(), before, "拦截不得占 seq / 落事件行");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn forged_actor_session_segment_is_rejected_too() {
    let dir = case_dir("forged-session");
    let mut session = open_session(&dir, "forged-session-run", None, Some("s-1"));
    let receipt = session.dispatch_event_as(
        "proposal.submit",
        "\"r1|1|plan|s|artifacts/p.md||x|actor_session=fake\"",
        None,
        Some("s-1"),
    );
    assert_eq!(receipt["ok"], Json::Bool(false), "{receipt}");
    assert_eq!(receipt["error"]["code"], json!("ACTOR_FORGED"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn declared_route_injects_segments_and_receipt_echoes_identity() {
    let dir = case_dir("inject");
    let mut session = open_session(&dir, "inject-run", Some("agent-1"), Some("s-1"));
    let receipt = session.dispatch_event_as(
        "proposal.submit",
        "\"r1|1|plan|s|artifacts/p.md||x\"",
        Some("agent-1"),
        Some("s-1"),
    );
    assert_eq!(receipt["ok"], Json::Bool(true), "{receipt}");
    assert_eq!(receipt["actor"], json!("agent-1"), "回显 resolved actor");
    assert_eq!(receipt["actor_session"], json!("s-1"));
    // 事件行里的载荷带双段（声明承重：mod 收到的是宿主章，不是调用方原文）
    let payload = session.kernel.dispatch("save", "");
    let kv = parse_envelope(&payload)["save"]["kv"]["entries"]["proposal_payload"]
        .as_str()
        .expect("mod 已存提案载荷")
        .to_owned();
    assert!(kv.ends_with("|actor=agent-1|actor_session=s-1"), "{kv}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn undeclared_route_never_injects_and_passes_caller_payload_through() {
    let dir = case_dir("undeclared");
    let mut session = open_session(&dir, "undeclared-run", Some("agent-1"), None);
    let receipt = session.dispatch_event_as(
        "event.echo",
        "\"text-with-actor=inside\"",
        Some("agent-1"),
        None,
    );
    assert_eq!(receipt["ok"], Json::Bool(true), "{receipt}");
    assert_eq!(receipt["actor"], json!("agent-1"), "任意事件回显章（增强）");
    // 未声明路由：载荷原样透传（不注入、不拦截）
    let saved = parse_envelope(&session.kernel.dispatch("save", ""))["save"]["kv"]["entries"]
        ["echoed"]
        .as_str()
        .expect("mod 已存回显")
        .to_owned();
    assert_eq!(saved, "text-with-actor=inside", "{saved}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn receipt_without_identity_echoes_nulls() {
    let dir = case_dir("echo-null");
    let mut session = open_session(&dir, "echo-null-run", None, None);
    let receipt = session.dispatch_event("event.echo", "\"x\"");
    assert_eq!(receipt["ok"], Json::Bool(true));
    assert_eq!(receipt["actor"], Json::Null, "None → null");
    assert_eq!(receipt["actor_session"], Json::Null);
    let _ = fs::remove_dir_all(&dir);
}

// ── opener 认领 ─────────────────────────────────────────────

#[test]
fn opener_first_claim_wins_and_is_immutable() {
    let dir = case_dir("opener");
    let run_dir = dir.join("runs").join("opener-run");
    fs::create_dir_all(&run_dir).expect("建 run 目录");
    assert_eq!(opener_of(&run_dir), None, "未认领");
    assert!(claim_opener(&run_dir, "agent-1", Some("s-1")).expect("首认领"));
    assert_eq!(opener_of(&run_dir).as_deref(), Some("agent-1"));
    // 同 id 重复 claim → no-op（不可变，连 at 都不刷新）
    assert!(!claim_opener(&run_dir, "agent-1", Some("s-1")).expect("重 claim"));
    // 异 actor 竞争 → 先到先得，后来者不覆盖不报错
    assert!(!claim_opener(&run_dir, "agent-2", None).expect("后来者"));
    assert_eq!(opener_of(&run_dir).as_deref(), Some("agent-1"));
    let meta = read_meta(&run_dir.join(META_FILE)).expect("读 run.json");
    assert_eq!(
        meta["opener_actor"]["session"],
        json!("s-1"),
        "session 随首认领落盘"
    );
    assert!(
        meta["opener_actor"]["at"].as_u64().is_some(),
        "开局时刻是事实"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn opener_claimed_by_successful_identity_dispatch_only() {
    let dir = case_dir("opener-dispatch");
    let run_dir = dir.join("runs").join("opener-dispatch-run");
    let mut session = open_session(&dir, "opener-dispatch-run", None, None);
    // 未携带身份的成功派发不认领
    session.dispatch_event("event.echo", "\"x\"");
    assert_eq!(opener_of(&run_dir), None);
    // 携带身份的成功派发认领
    session.dispatch_event_as("event.echo", "\"x\"", Some("agent-9"), None);
    assert_eq!(opener_of(&run_dir).as_deref(), Some("agent-9"));
    // mod 侧拒绝（ok=false）的成功送达不认领：失败步不配当开局者
    let receipt = session.dispatch_event_as("event.boom", "\"0\"", Some("agent-10"), None);
    assert_eq!(receipt["ok"], Json::Bool(false));
    assert_eq!(
        opener_of(&run_dir).as_deref(),
        Some("agent-9"),
        "被拒派发不认领"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn write_meta_preserves_opener_across_reset_and_rewrite() {
    let dir = case_dir("opener-meta");
    let runs_root = dir.join("runs");
    write_identity_mod(&dir);
    // 旧局盘面：run.json 已有 opener_actor（升级后的旧局补认领形态）
    let run_dir = runs_root.join("opener-meta-run");
    fs::create_dir_all(&run_dir).expect("建 run 目录");
    fs::write(
            run_dir.join(META_FILE),
            r#"{"run_id":"opener-meta-run","started_at":1,"opener_actor":{"actor":"old-agent","at":42}}"#,
        )
        .expect("写旧 run.json");
    // Session::open（新局）会重写 run.json（write_meta）——opener 必须原样带回
    let args = base_args(&dir, &runs_root, "opener-meta-run", None, None);
    let session = Session::open(&args).expect("开局");
    assert_eq!(
        opener_of(&run_dir).as_deref(),
        Some("old-agent"),
        "write_meta 保留 opener"
    );
    // 显式再写一次 meta（热载路径同函数）也不丢
    session.write_meta();
    assert_eq!(opener_of(&run_dir).as_deref(), Some("old-agent"));
    let _ = fs::remove_dir_all(&dir);
}

/// 声明承重锁（r3 变异探针 4 的红面）：真 Mod 的 proposal 路由必须声明双布尔——
/// 删掉任一布尔，此测试与宿主注入路径一起变红（声明是注入/拦截的唯一依据）。
/// r9 追加：三个**推进/配置动作**各有一条 stamp-only 路由（只盖 actor，不声明
/// actor_session、不带 write）——宿主据此为直接 `--event` 通道盖章，人机门禁才"按人"。
#[test]
fn project_pack_declares_actor_bools_on_the_proposal_route() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let routes = InboxRoutes::load(&repo.join(".ygg/mods/agent-seed")).expect("读真 Mod 路由表");
    assert_eq!(routes.actor_flags_for("proposal.submit"), (true, true));
    // r9：签名与推进动作必须盖章（否则按人门禁无身份来源）。
    assert_eq!(routes.actor_flags_for("config.set"), (true, false));
    assert_eq!(routes.actor_flags_for("gate.approve"), (true, false));
    assert_eq!(routes.actor_flags_for("coordinate.submit"), (true, false));
    // stamp-only 的负面契约：这三条不得长出任何写命令（inbox 不是它们的旁路）。
    for kind in ["config", "gate", "coordinate"] {
        assert!(
            routes.get(kind).is_some_and(|route| route.write.is_none()),
            "`{kind}` 必须是 stamp-only（不带 write）"
        );
    }
    // 其余动作不声明身份段（宿主不注入也不拦截）。
    assert_eq!(routes.actor_flags_for("tool.result"), (false, false));
    assert_eq!(routes.actor_flags_for("requirement.add"), (false, false));
}

#[test]
fn inbox_record_stamps_identity_and_claims_opener() {
    let dir = case_dir("inbox-stamp");
    let run_dir = dir.join("runs").join("inbox-stamp-run");
    fs::create_dir_all(&run_dir).expect("建 run 目录");
    write_identity_mod(&dir);
    let routes = InboxRoutes::load(&dir).expect("读路由表");
    let mut store = open_store(Backend::Json, &run_dir).expect("开 json 后端");
    let identity = ActorIdentity {
        actor: Some("worker-1".to_owned()),
        session: Some("s-worker".to_owned()),
    };
    let record = append_tool_result_record(
        &run_dir,
        store.as_mut(),
        &routes,
        "tool_result",
        "c1|probe.echo|ok|0|hello|ext-1",
        "inbox-stamp-run",
        &identity,
    )
    .expect("追加成功");
    assert_eq!(record["actor"], json!("worker-1"), "记录盖宿主保留键");
    assert_eq!(record["actor_session"], json!("s-worker"));
    assert_eq!(
        opener_of(&run_dir).as_deref(),
        Some("worker-1"),
        "成功落行认领 opener"
    );
    // 无身份：不盖章、不认领
    let record = append_tool_result_record(
        &run_dir,
        store.as_mut(),
        &routes,
        "tool_result",
        "c2|probe.echo|ok|0|hi|ext-2",
        "inbox-stamp-run",
        &ActorIdentity::NONE,
    )
    .expect("追加成功");
    assert!(record.get("actor").is_none(), "None 不写键");
    assert_eq!(opener_of(&run_dir).as_deref(), Some("worker-1"));
    // drain 消费：记录自带身份经 dispatch_event_as 注入声明路由（tool_result
    // 路由未声明 → 不注入，但 mod 收到的载荷不含身份段）
    let mut session = {
        let args = base_args(&dir, &dir.join("runs"), "inbox-stamp-run", None, None);
        Session::open(&args).expect("开局")
    };
    session.load_autosave().expect("读档");
    let drain = session.drain_inbox();
    assert_eq!(drain.applied, 2, "两行都被消费：{:?}", drain.error);
    let _ = fs::remove_dir_all(&dir);
}
