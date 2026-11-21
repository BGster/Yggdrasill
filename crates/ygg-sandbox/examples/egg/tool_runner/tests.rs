use super::*;

/// 用例专属临时目录（互不共享；先清空再建）。
fn case_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ygg-tools-r1-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 最小合成 Mod：两条只读 cli 工具 + `tool_result` inbox 路由 + 三个触发事件。
///
/// `probe.echo` 走场景 1（`inlineWaitMs > 0`），`probe.detach` 恒走场景 2
/// （`inlineWaitMs = 0`，参数里的 `$ENV{YGG_TOOL_TEST}` 用于验 I4）。
fn write_tool_mod(dir: &Path, allowed: &[&str]) {
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
            "tool_result": {
                "action": "tool.result",
                "fields": ["call_id", "tool_id", "status", "code", "output", "external_id"],
                "write": {"flag": "tool-result", "shape": "payload"}
            }
        },
        "tools": {
            "probe.echo": {
                "kind": "cli",
                "program": "printf",
                "args": ["%s", "{{text}}"],
                "timeoutMs": 5_000u64,
                "inlineWaitMs": 2_000u64,
                "maxOutputBytes": 1_024u64,
                "readOnly": true,
                "resultEvent": "tool.result"
            },
            "probe.detach": {
                "kind": "cli",
                "program": "printf",
                "args": ["%s", "$ENV{YGG_TOOL_TEST}"],
                "timeoutMs": 5_000u64,
                "inlineWaitMs": 0u64,
                "maxOutputBytes": 1_024u64,
                "readOnly": true,
                "resultEvent": "tool.result"
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
        r#"ygg.on("event.plain", function()
  ygg.narrate("普通一步")
end)
ygg.on("event.start", function()
  ygg.meta("tool_calls", { { id = "c1", tool = "probe.echo", args = { text = "hello" } } })
  ygg.narrate("已请求工具")
end)
ygg.on("event.detach", function()
  ygg.meta("tool_calls", { { id = "d1", tool = "probe.detach", args = {} } })
  ygg.narrate("已请求脱体工具")
end)
ygg.on("event.unknown", function()
  ygg.meta("tool_calls", { { id = "u1", tool = "probe.missing", args = {} } })
  ygg.narrate("已请求未声明工具")
end)
ygg.on("tool.result", function(payload)
  ygg.kv.set("last_tool_payload", tostring(payload))
  ygg.narrate("工具结果：" .. tostring(payload))
end)
ygg.on("agent.summary", function()
  ygg.meta("summary", { open = true })
  ygg.narrate("摘要就绪")
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
        "allowedTools": allowed,
    });
    write_json_pretty(&dir.join("data/config.json"), &config).expect("写 config.json");
}

/// 单发 / 只观察共用的宿主参数（`store` 钉住 JSON：本节断言都在文件面）。
fn tool_args(mod_dir: &Path, runs_root: &Path, run_id: &str) -> Args {
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
        output: crate::OutputFormat::Text,
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
        actor: None,
        actor_session: None,
        serve: false,
        port: None,
        panel_dir: None,
        poll_ms: None,
    }
}

/// 一步派发 + 回执（断言用）。
#[track_caller]
fn dispatch(session: &mut Session, event: &str) -> Json {
    let receipt = session.dispatch_event(event, "0");
    assert_eq!(
        receipt["ok"],
        Json::Bool(true),
        "{event} 必须成功：{receipt}"
    );
    receipt
}

/// 取一条声明 + 一条请求 → 造出待登记的调用（不 spawn，纯内存）。
fn pending_call(
    session: &Session,
    tool: &str,
    call_id: &str,
    args: &[(&str, &str)],
) -> (ToolSpec, PendingCall) {
    let spec = session.tools.get(tool).cloned().expect("声明必须存在");
    let request = ToolRequest {
        id: call_id.to_owned(),
        tool: tool.to_owned(),
        args: args
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect(),
    };
    let call = PendingCall::from_spec(&spec, &request, now_ms() + 5_000, 1);
    (spec, call)
}

// ---- 解析面（§6.1） ----

#[test]
fn parses_meta_tool_calls() {
    let meta = json!({"tool_calls": [{"tool": "t", "args": {"a": "1"}}]});
    let (requests, notices) = parse_tool_calls(&meta, "run-0");
    assert!(notices.is_empty(), "{notices:?}");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].tool, "t");
    assert_eq!(requests[0].args, vec![("a".to_owned(), "1".to_owned())]);
    // 缺 id → 缺省 `<prefix>-<index>`
    assert_eq!(requests[0].id, "run-0-0");
    // 无该键 / 非数组 / 空数组 → 零请求
    assert!(parse_tool_calls(&json!({}), "run-0").0.is_empty());
    assert!(parse_tool_calls(&json!({"tool_calls": []}), "run-0")
        .0
        .is_empty());
    let (requests, notices) = parse_tool_calls(&json!({"tool_calls": {}}), "run-0");
    assert!(requests.is_empty());
    assert_eq!(notices.len(), 1);
}

#[test]
fn empty_args_array_is_empty_map() {
    // Lua 空表 `{}` 会序列化成 `[]`（host_api 已文档化的怪癖）：等价于无参数。
    let meta = json!({"tool_calls": [{"id": "c1", "tool": "probe.echo", "args": []}]});
    let (requests, notices) = parse_tool_calls(&meta, "run-0");
    assert!(notices.is_empty(), "{notices:?}");
    assert_eq!(requests.len(), 1);
    assert!(requests[0].args.is_empty());
    assert_eq!(requests[0].id, "c1");
}

#[test]
fn rejects_args_with_pipe() {
    let meta = json!({"tool_calls": [{"tool": "t", "args": {"a": "x|y"}}]});
    let (requests, notices) = parse_tool_calls(&meta, "run-0");
    assert!(requests.is_empty(), "含 `|` 的条目必须被跳过");
    assert_eq!(notices.len(), 1);
    assert!(notices[0].contains('|'), "{notices:?}");
    // 嵌套值 / 非对象 args 也各自只丢该条
    for bad in [
        json!({"tool": "t", "args": {"a": ["x"]}}),
        json!({"tool": "t", "args": 1}),
    ] {
        let (requests, notices) = parse_tool_calls(&json!({"tool_calls": [bad]}), "run-0");
        assert!(requests.is_empty());
        assert_eq!(notices.len(), 1);
    }
    // 缺 tool 的条目同样只丢自己
    let (requests, notices) = parse_tool_calls(&json!({"tool_calls": [{"args": {}}]}), "run-0");
    assert!(requests.is_empty());
    assert_eq!(notices.len(), 1);
    // id 非法 = 拒绝（不静默换成 fallback，因为它同时是结果文件名）
    let (requests, notices) = parse_tool_calls(
        &json!({"tool_calls": [{"id": "../x", "tool": "t"}]}),
        "run-0",
    );
    assert!(requests.is_empty());
    assert_eq!(notices.len(), 1);
}

// ---- 替换面（§6.2 / §6.3） ----

#[test]
fn substitutes_placeholders() {
    let args = vec![("repo".to_owned(), ".".to_owned())];
    let env = EnvTable {
        process: Vec::new(),
        env_local: vec![("TOKEN".to_owned(), "secret".to_owned())],
        env: Vec::new(),
    };
    assert_eq!(
        substitute("{{repo}}/src", &args, &env).expect("替换成功"),
        "./src"
    );
    assert_eq!(
        substitute("Bearer $ENV{TOKEN}", &args, &env).expect("替换成功"),
        "Bearer secret"
    );
    assert_eq!(
        substitute("{{ repo }}", &args, &env).expect("键两侧空白容忍"),
        "."
    );
    assert_eq!(
        substitute("no-templates", &args, &env).expect("原样"),
        "no-templates"
    );
}

#[test]
fn missing_placeholder_is_per_tool_error() {
    let args: Vec<(String, String)> = Vec::new();
    let env = EnvTable {
        process: Vec::new(),
        env_local: Vec::new(),
        env: Vec::new(),
    };
    let error = substitute("--repo={{repo}}", &args, &env).expect_err("缺参数必须报错");
    assert!(error.contains("repo"), "{error}");
    assert!(error.contains("未提供"), "{error}");
}

#[test]
fn env_resolution_precedence() {
    let dir = case_dir("env");
    fs::write(dir.join(".env"), "YGG_TOOL_ONLY=y\nPATH=from-env\n").expect("写 .env");
    fs::write(dir.join(".env.local"), "YGG_TOOL_ONLY=x\nPATH=from-local\n").expect("写 .env.local");
    let env = load_env_table(Some(&dir));
    // 进程环境 > .env.local > .env
    assert_eq!(
        env.lookup("PATH"),
        std::env::var("PATH").ok().as_deref(),
        "进程环境必须压过两个文件"
    );
    assert_eq!(
        env.lookup("YGG_TOOL_ONLY"),
        Some("x"),
        ".env.local 压过 .env"
    );
    // 不存在的目录 = 空表（不报错）
    let empty = load_env_table(Some(&dir.join("missing")));
    assert_eq!(empty.lookup("YGG_TOOL_ONLY"), None);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn env_missing_reports_key_only() {
    let env = EnvTable {
        process: Vec::new(),
        env_local: Vec::new(),
        env: Vec::new(),
    };
    let error = resolve_env("YGG_TOOL_NOPE", &env).expect_err("未设置必须报错");
    assert!(error.contains("YGG_TOOL_NOPE"), "{error}");
    assert!(
        !error.contains('='),
        "错误文案不得出现值形态（只有变量名）：{error}"
    );
}

#[test]
fn parse_env_file_lexes_the_usual_forms() {
    let table = parse_env_file(
        "# comment\nexport A=1\nB='two'\nC=\"three\"\nD=\nnot a pair\n=bad\nE=unquoted value\n",
    );
    assert_eq!(
        table,
        vec![
            ("A".to_owned(), "1".to_owned()),
            ("B".to_owned(), "two".to_owned()),
            ("C".to_owned(), "three".to_owned()),
            ("D".to_owned(), String::new()),
            ("E".to_owned(), "unquoted value".to_owned()),
        ]
    );
}

// ---- 结果面（§7.1） ----

#[test]
fn sanitize_tool_output_wire_safe() {
    let cleaned = sanitize_tool_output("a|b\nc\0d\r\ne", 1_024);
    assert!(!cleaned.contains('|'), "{cleaned}");
    assert!(!cleaned.contains('\n'), "{cleaned}");
    assert!(!cleaned.contains('\r'), "{cleaned}");
    assert!(!cleaned.contains('\0'), "{cleaned}");
    assert_eq!(cleaned.split('|').count(), 1, "输出不得引入段分隔符");
    assert!(cleaned.contains('｜'), "分隔符换成全角形态（保留可读性）");
}

#[test]
fn truncates_on_char_boundary() {
    // 「中文」每个字 3 字节：把上限切在第 4 字节（字中间）→ 必须回退到边界。
    let text = "中文中文";
    let (truncated, hit) = truncate_bytes(text, 4);
    assert!(hit);
    assert_eq!(truncated, "中", "必须回退到字符边界");
    assert!(!truncated.contains('\u{FFFD}'));
    let cleaned = sanitize_tool_output(&"中".repeat(10), 8);
    assert!(!cleaned.contains('\u{FFFD}'), "{cleaned}");
    assert!(cleaned.ends_with("…[truncated]"), "{cleaned}");
    // 不超限时原样返回（不加标记）
    assert_eq!(sanitize_tool_output("short", 1_024), "short");
}

#[test]
fn result_event_payload_is_json_string() {
    // 场景 1 的派发载荷必须与 `drain_inbox_locked` 同形：JSON **字符串**。
    let payload = json!("c1|probe.echo|ok|0|hello|inbox-1").to_string();
    let parsed: Json = serde_json::from_str(&payload).expect("载荷必须能解析");
    assert!(matches!(parsed, Json::String(_)), "{payload}");
}

#[test]
fn wire_payload_has_six_segments() {
    let dir = case_dir("wire");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_tool_mod(&mod_dir, &["probe.echo", "probe.detach"]);
    let session = Session::open(&tool_args(&mod_dir, &runs_root, "wire")).expect("开局");
    let (_, call) = pending_call(&session, "probe.echo", "c1", &[("text", "hello")]);
    let outcome = ToolOutcome {
        status: "ok",
        code: "0".to_owned(),
        output: "hello".to_owned(),
    };
    let payload = wire_payload(&call, &outcome, "inbox-1");
    let segments: Vec<&str> = payload.split('|').collect();
    assert_eq!(segments.len(), 6, "{payload}");
    assert_eq!(segments[0], "c1");
    assert_eq!(segments[1], "probe.echo");
    assert_eq!(segments[2], "ok");
    assert_eq!(segments[3], "0");
    assert_eq!(segments[4], "hello");
    assert_eq!(segments[5], "inbox-1");
    let _ = fs::remove_dir_all(&dir);
}

// ---- 在飞事实面（§4） ----

#[test]
fn pending_snapshot_never_stores_env_values() {
    let dir = case_dir("pending-redaction");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_tool_mod(&mod_dir, &["probe.echo", "probe.detach"]);
    fs::write(mod_dir.join(".env.local"), "YGG_TOOL_TEST=tok-123\n").expect("写 .env.local");
    let session = Session::open(&tool_args(&mod_dir, &runs_root, "redaction")).expect("开局");
    let (_, call) = pending_call(&session, "probe.detach", "d1", &[]);
    pending_tools_insert(&session.run_dir, &call).expect("登记");
    let snapshot = fs::read_to_string(pending_tools_path(&session.run_dir)).expect("读 pending");
    assert!(
        snapshot.contains("$ENV{YGG_TOOL_TEST}"),
        "模板必须原样保留：{snapshot}"
    );
    assert!(
        !snapshot.contains("tok-123"),
        "I4：pending 快照绝不得出现解析后的值：{snapshot}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn pending_tools_update_is_atomic() {
    let dir = case_dir("atomic");
    let run_dir = dir.join("run");
    fs::create_dir_all(&run_dir).expect("建 run 目录");
    std::thread::scope(|scope| {
        for worker in 0..2 {
            let run_dir = run_dir.clone();
            scope.spawn(move || {
                for index in 0..20 {
                    pending_tools_update(&run_dir, |record| {
                        if let Some(calls) = record["calls"].as_object_mut() {
                            calls.insert(
                                format!("c{worker}-{index}"),
                                json!({"tool": "probe.echo"}),
                            );
                        }
                    })
                    .expect("并发登记必须成功");
                }
            });
        }
    });
    assert_eq!(pending_calls(&run_dir).len(), 40, "并发写不得丢更新");
    // 摘除是幂等的
    pending_tools_remove(&run_dir, "c0-0").expect("摘除");
    pending_tools_remove(&run_dir, "c0-0").expect("重复摘除不报错");
    assert_eq!(pending_calls(&run_dir).len(), 39);
    let _ = fs::remove_dir_all(&dir);
}

// ---- 场景 1：同进程就地回投 ----

#[test]
fn scenario1_delivers_in_process_and_clears_pending() {
    let dir = case_dir("scenario1");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_tool_mod(&mod_dir, &["probe.echo", "probe.detach"]);
    let mut session = Session::open(&tool_args(&mod_dir, &runs_root, "s1")).expect("开局");
    let (spec, call) = pending_call(&session, "probe.echo", "c1", &[("text", "hello")]);
    pending_tools_insert(&session.run_dir, &call).expect("登记");
    // 模拟 worker 已把结果写好（不 spawn：本用例只验父进程的收敛面）。
    write_tool_result(
        &session.run_dir,
        &ToolResult {
            call_id: "c1".to_owned(),
            tool: "probe.echo".to_owned(),
            status: "ok".to_owned(),
            code: "0".to_owned(),
            output: "hello".to_owned(),
            payload: "c1|probe.echo|ok|0|hello|inbox-test".to_owned(),
            external_id: "inbox-test".to_owned(),
        },
    )
    .expect("写结果");
    let run = session
        .settle_tool_call(&spec, &call)
        .expect("场景 1 必须就地回投");
    assert_eq!(run["status"], "ok");
    assert_eq!(run["tool"], "probe.echo");
    assert_eq!(run["ok"], Json::Bool(true));
    assert!(
        run["seq"].as_u64().is_some(),
        "回投事件必须有自己的 seq：{run}"
    );
    // pending 已摘除、结果文件已消费
    assert!(pending_calls(&session.run_dir).is_empty());
    assert!(!tool_result_path(&session.run_dir, "c1").exists());
    // 事件日志里出现 tool.result 一步（结果以普通事件回投）
    let events =
        fs::read_to_string(session.run_dir.join(ygg_store::EVENTS_FILE)).expect("读事件日志");
    assert!(events.contains("\"tool.result\""), "{events}");
    // 认领文件仍在：它同时是「已交付」标记（防止第二个进程重复派发）
    assert!(tool_claim_path(&session.run_dir, "c1").exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn scenario1_zero_inline_wait_never_polls() {
    let dir = case_dir("scenario1-zero");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_tool_mod(&mod_dir, &["probe.echo", "probe.detach"]);
    let mut session = Session::open(&tool_args(&mod_dir, &runs_root, "s1z")).expect("开局");
    let (spec, call) = pending_call(&session, "probe.detach", "d1", &[]);
    pending_tools_insert(&session.run_dir, &call).expect("登记");
    // 结果文件就在那里，但 inlineWaitMs = 0 → 父进程不轮询、不认领（场景 2）
    write_tool_result(
        &session.run_dir,
        &ToolResult {
            call_id: "d1".to_owned(),
            tool: "probe.detach".to_owned(),
            status: "ok".to_owned(),
            code: "0".to_owned(),
            output: "detached".to_owned(),
            payload: "d1|probe.detach|ok|0|detached|inbox-d".to_owned(),
            external_id: "inbox-d".to_owned(),
        },
    )
    .expect("写结果");
    assert!(session.settle_tool_call(&spec, &call).is_none());
    assert!(
        !tool_claim_path(&session.run_dir, "d1").exists(),
        "父进程不得认领"
    );
    assert_eq!(pending_calls(&session.run_dir).len(), 1, "条目留给 worker");
    let _ = fs::remove_dir_all(&dir);
}

// ---- 场景 2：脱体 worker + inbox 回写 + `--wait-tools` ----

#[test]
fn scenario2_worker_delivers_to_inbox_and_wait_tools_drains() {
    let dir = case_dir("scenario2");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_tool_mod(&mod_dir, &["probe.echo", "probe.detach"]);
    fs::write(mod_dir.join(".env.local"), "YGG_TOOL_TEST=tok-123\n").expect("写 .env.local");
    let args = tool_args(&mod_dir, &runs_root, "s2");
    let mut session = Session::open(&args).expect("开局");
    dispatch(&mut session, "event.plain");
    let (_, call) = pending_call(&session, "probe.detach", "d1", &[]);
    pending_tools_insert(&session.run_dir, &call).expect("登记");

    // worker 执行体（真子进程：printf，只读无害）
    assert_eq!(run_tool_worker(&args, "d1"), 0, "worker 必须成功交付");
    assert!(
        pending_calls(&session.run_dir).is_empty(),
        "交付成功后摘除 pending"
    );
    assert!(tool_claim_path(&session.run_dir, "d1").exists());

    // --wait-tools：消费 inbox（结果行走 Mod 的 tool_result 路由）
    let wait_args = Args {
        wait_tools: true,
        ..args.clone()
    };
    assert_eq!(run_wait_tools(&wait_args), 0);
    let events =
        fs::read_to_string(session.run_dir.join(ygg_store::EVENTS_FILE)).expect("读事件日志");
    assert!(events.contains("\"tool.result\""), "{events}");
    assert!(
        events.contains("tok-123"),
        "工具输出必须进回投载荷：{events}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn wait_tools_never_resets_the_run() {
    let dir = case_dir("wait-tools-no-reset");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_tool_mod(&mod_dir, &["probe.echo", "probe.detach"]);
    let args = tool_args(&mod_dir, &runs_root, "wnr");
    let mut session = Session::open(&args).expect("开局");
    dispatch(&mut session, "event.plain");
    dispatch(&mut session, "event.plain");
    let run_dir = session.run_dir.clone();
    let events_before =
        fs::read_to_string(run_dir.join(ygg_store::EVENTS_FILE)).expect("读事件日志");
    let autosave_before = fs::read(run_dir.join(AUTOSAVE_FILE)).expect("读存档");
    let wait_args = Args {
        wait_tools: true,
        tool_timeout_ms: Some(500),
        ..args.clone()
    };
    assert_eq!(run_wait_tools(&wait_args), 0);
    let events_after =
        fs::read_to_string(run_dir.join(ygg_store::EVENTS_FILE)).expect("读事件日志");
    let autosave_after = fs::read(run_dir.join(AUTOSAVE_FILE)).expect("读存档");
    assert_eq!(
        events_before, events_after,
        "--wait-tools 不得新增/重置事件行"
    );
    assert_eq!(autosave_before, autosave_after, "--wait-tools 不得重置存档");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn wait_tools_reports_pending_after_timeout() {
    let dir = case_dir("wait-tools-pending");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_tool_mod(&mod_dir, &["probe.echo", "probe.detach"]);
    let mut session = Session::open(&tool_args(&mod_dir, &runs_root, "wtp")).expect("开局");
    dispatch(&mut session, "event.plain");
    let (_, call) = pending_call(&session, "probe.detach", "stuck", &[]);
    pending_tools_insert(&session.run_dir, &call).expect("登记");
    let wait_args = Args {
        wait_tools: true,
        tool_timeout_ms: Some(150),
        ..tool_args(&mod_dir, &runs_root, "wtp")
    };
    assert_eq!(run_wait_tools(&wait_args), 1, "残留 pending 必须非零退出");
    assert_eq!(
        pending_calls(&session.run_dir).len(),
        1,
        "残留条目不被自动回收"
    );
    let _ = fs::remove_dir_all(&dir);
}

// ---- 纵深防御 / 生命周期 ----

#[test]
fn unknown_tool_is_skipped_not_fatal() {
    let dir = case_dir("unknown-tool");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_tool_mod(&mod_dir, &["probe.echo", "probe.detach"]);
    let mut session = Session::open(&tool_args(&mod_dir, &runs_root, "unk")).expect("开局");
    let receipt = dispatch(&mut session, "event.unknown");
    assert!(
        receipt.get("tools").is_none(),
        "未声明工具不得产生 runs：{receipt}"
    );
    let warnings = receipt["warnings"].as_array().cloned().unwrap_or_default();
    let text = warnings
        .iter()
        .filter_map(Json::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("未在 pack.json"), "{text}");
    assert!(pending_calls(&session.run_dir).is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn max_tool_rounds_is_bounded() {
    let dir = case_dir("rounds");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_tool_mod(&mod_dir, &["probe.echo", "probe.detach"]);
    let mut session = Session::open(&tool_args(&mod_dir, &runs_root, "rounds")).expect("开局");
    // 已达上限：再来的请求直接忽略（不递归、不 spawn）
    session.tool_depth.set(MAX_TOOL_ROUNDS);
    let receipt = dispatch(&mut session, "event.start");
    assert!(
        receipt.get("tools").is_none(),
        "超限后不得再跑工具：{receipt}"
    );
    let warnings = receipt["warnings"].as_array().cloned().unwrap_or_default();
    let text = warnings
        .iter()
        .filter_map(Json::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("轮上限"), "{text}");
    assert!(pending_calls(&session.run_dir).is_empty());
    // 归零守卫：未超限的一步把深度还回去（空数组 = 不 spawn，测试不依赖子进程）
    session.tool_depth.set(MAX_TOOL_ROUNDS - 1);
    assert!(session
        .run_tool_requests(&json!({"meta": {"tool_calls": []}}), 0)
        .is_empty());
    assert_eq!(
        session.tool_depth.get(),
        MAX_TOOL_ROUNDS - 1,
        "深度必须归零（Drop guard）"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn reset_clears_pending_tools() {
    let dir = case_dir("reset");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_tool_mod(&mod_dir, &["probe.echo", "probe.detach"]);
    let mut session = Session::open(&tool_args(&mod_dir, &runs_root, "reset")).expect("开局");
    let (_, call) = pending_call(&session, "probe.detach", "d1", &[]);
    pending_tools_insert(&session.run_dir, &call).expect("登记");
    write_tool_result(
        &session.run_dir,
        &ToolResult {
            call_id: "d1".to_owned(),
            tool: "probe.detach".to_owned(),
            status: "ok".to_owned(),
            code: "0".to_owned(),
            output: String::new(),
            payload: "d1|probe.detach|ok|0||inbox-d".to_owned(),
            external_id: "inbox-d".to_owned(),
        },
    )
    .expect("写结果");
    session.reset_artifacts(false).expect("重置必须成功");
    assert!(
        !pending_tools_path(&session.run_dir).exists(),
        "pending 必须随本代清空"
    );
    assert!(
        !tool_results_dir(&session.run_dir).exists(),
        "结果暂存必须随本代清空"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn allowlist_gate_is_enforced_at_load_time() {
    let dir = case_dir("allowlist");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    // 只放行一条：另一条声明必须让开局 fail-closed（I6）。
    write_tool_mod(&mod_dir, &["probe.echo"]);
    let error = match Session::open(&tool_args(&mod_dir, &runs_root, "gate")) {
        Ok(_) => panic!("未放行的声明必须拒绝启动"),
        Err(message) => message,
    };
    assert!(error.contains("probe.detach"), "{error}");
    assert!(error.contains("allowedTools"), "{error}");
    // 补上放行清单即可开局
    write_tool_mod(&mod_dir, &["probe.echo", "probe.detach"]);
    assert!(Session::open(&tool_args(&mod_dir, &runs_root, "gate")).is_ok());
    let _ = fs::remove_dir_all(&dir);
}
