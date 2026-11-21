use super::*;

fn case_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ygg-daemon-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 最小合成 Mod：一个累加 kv 的 handler、一个**读** `ygg.fs.stat` 的 handler，
/// 以及一个把 `jobs` / `gates` 写进 `meta.summary` 的只读摘要动作。
///
/// 沙箱裁掉了 `string` / `table` / `math`：只能用 `#` / `..` / `tostring` / `type`。
fn write_daemon_mod(dir: &Path) {
    fs::create_dir_all(dir.join("data")).expect("建 data 目录");
    fs::write(
        dir.join("pack.json"),
        r#"{
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
    "add": {
      "action": "inbox.add",
      "fields": ["title", "external_id"],
      "write": {"flag": "requirement", "shape": "named"}
    }
  }
}"#,
    )
    .expect("写 pack.json");
    fs::write(
        dir.join("manifest.json"),
        r#"{"schemaVersion": 1, "entries": [], "code": []}"#,
    )
    .expect("写 manifest.json");
    fs::write(
            dir.join("mod.lua"),
            r#"ygg.on("event.start", function()
  ygg.narrate("开局")
end)
ygg.on("event.tick", function(payload)
  ygg.kv.set("ticks", (ygg.kv.get("ticks") or 0) + 1)
  ygg.narrate("tick=" .. tostring(ygg.kv.get("ticks")) .. " payload=" .. tostring(payload))
end)
ygg.on("event.probe_vfs", function()
  local hit = ygg.fs.stat("artifacts/note.md")
  local sha = hit.sha256
  ygg.narrate("vfs=" .. tostring(hit.exists) .. ":" .. tostring(hit.bytes) .. ":" .. tostring(#sha))
end)
ygg.on("agent.summary", function()
  ygg.meta("summary", {
    stage = "coordinate",
    jobs = {
      { id = "r1", rev = 3, stage = "development", lane = 1, branch = "feat-x", base = "abc1234", title = "切片一", description = "描述一" },
    },
    gates = {
      { id = "r2", gate = "plan", rev = 2, title = "切片二" },
    },
  })
  ygg.narrate("摘要就绪")
end)
"#,
        )
        .expect("写 mod.lua");
    fs::write(
        dir.join("data/config.json"),
        r#"{
  "drive": {"maxSteps": 50, "stallLimit": 5, "stepsPerRequirement": 20},
  "settlement": {
    "requireEvents": true,
    "requireNoFailedSteps": true,
    "requireEnding": true,
    "requireInboxConsumed": true
  }
}"#,
    )
    .expect("写 config.json");
}

fn daemon_args(mod_dir: &Path, runs_root: &Path, run_id: &str) -> Args {
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
        daemon: true,
        harness: None,
        actor: None,
        actor_session: None,
        serve: false,
        port: None,
        panel_dir: None,
        poll_ms: None,
    }
}

/// 发一条请求并断言「不退出」（只有 `shutdown` 能把第二项置 true）。
#[track_caller]
fn call(session: &mut Session, args: &Args, req: Json) -> Json {
    let (reply, quit) = handle_daemon_request(session, args, &req);
    assert!(!quit, "只有 shutdown 能让守护进程退出：{reply}");
    assert!(reply.is_object(), "应答恒为 JSON 对象：{reply}");
    reply
}

fn narrative_text(reply: &Json) -> String {
    reply["narrative"].to_string()
}

#[test]
fn rpc_covers_ping_dispatch_vfs_summary_and_shutdown() {
    let dir = case_dir("rpc");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_daemon_mod(&mod_dir);
    let args = daemon_args(&mod_dir, &runs_root, "d1");
    let mut session = Session::open(&args).expect("守护模式开局");

    // ① ping：runId 与存储后端都取自真实装载面（不是硬编码）。
    let ping = call(&mut session, &args, json!({"op": "ping", "id": 7}));
    assert_eq!(ping["ok"], json!(true), "{ping}");
    assert_eq!(ping["op"], json!("ping"));
    assert_eq!(ping["runId"], json!("d1"));
    assert_eq!(ping["store"], json!("json"));
    assert_eq!(
        ping["id"],
        json!(7),
        "id 必须原样回显（单通道配对）：{ping}"
    );

    // ② dispatch：字符串载荷要**包成 JSON 标量**再下发（裸串不是合法载荷）。
    let first = call(
        &mut session,
        &args,
        json!({"op": "dispatch", "event": "event.tick", "payload": "abc"}),
    );
    assert_eq!(first["ok"], json!(true), "{first}");
    assert_eq!(first["op"], json!("dispatch"));
    assert_eq!(first["event"], json!("event.tick"));
    let lines = narrative_text(&first);
    assert!(lines.contains("tick=1"), "{first}");
    assert!(
        lines.contains("payload=abc"),
        "载荷必须是字符串 abc：{first}"
    );

    // dispatch 的回执自带最新摘要：调度端不必再发一条 `summary`（也就不会
    // 看到「派发之后、摘要之前」的中间态）。
    let summary = &first["summary"];
    assert_eq!(summary["ok"], json!(true), "{first}");
    let jobs = summary["summary"]["jobs"].as_array().expect("summary.jobs");
    assert_eq!(jobs.len(), 1, "{first}");
    assert_eq!(jobs[0]["id"], json!("r1"));
    assert_eq!(jobs[0]["stage"], json!("development"));
    assert_eq!(jobs[0]["lane"], json!(1));
    assert_eq!(jobs[0]["branch"], json!("feat-x"));
    assert_eq!(jobs[0]["base"], json!("abc1234"));
    assert_eq!(jobs[0]["title"], json!("切片一"));
    assert_eq!(jobs[0]["description"], json!("描述一"));
    let gates = summary["summary"]["gates"]
        .as_array()
        .expect("summary.gates");
    assert_eq!(gates.len(), 1, "{first}");
    assert_eq!(gates[0]["gate"], json!("plan"));
    assert_eq!(gates[0]["rev"], json!(2));
    assert_eq!(gates[0]["title"], json!("切片二"));

    // ③ vfs_put：端口与内核 `ygg.fs` 必须**同时**可见。
    let body = "第一版";
    let sha = ygg_store::sha256_hex(body.as_bytes());
    let put = call(
        &mut session,
        &args,
        json!({"op": "vfs_put", "path": "artifacts/note.md", "content": body}),
    );
    assert_eq!(put["ok"], json!(true), "{put}");
    assert_eq!(put["path"], json!("artifacts/note.md"));
    assert_eq!(put["bytes"], json!(body.len()));
    assert_eq!(put["sha256"], json!(sha));
    assert_eq!(
        fs::read(runs_root.join("d1/artifacts/note.md")).expect("制品必须落盘"),
        body.as_bytes(),
        "vfs_put 必须写进本局后端"
    );
    // 同一局内的 `ygg.fs.stat` 立即可见（只落盘不灌 VFS 会让这里报 exists=false）。
    let probe = call(
        &mut session,
        &args,
        json!({"op": "dispatch", "event": "event.probe_vfs"}),
    );
    let probe_lines = narrative_text(&probe);
    assert!(
        probe_lines.contains(&format!("vfs=true:{}:64", body.len())),
        "vfs_put 之后内核必须立刻看得见（exists=true / bytes / 64 位 sha）：{probe}"
    );

    // ④ vfs_get / vfs_list：读面与端口同一份事实。
    let got = call(
        &mut session,
        &args,
        json!({"op": "vfs_get", "path": "artifacts/note.md"}),
    );
    assert_eq!(got["ok"], json!(true), "{got}");
    assert_eq!(got["content"], json!(body));
    assert_eq!(got["sha256"], json!(sha));
    let missing = call(
        &mut session,
        &args,
        json!({"op": "vfs_get", "path": "artifacts/none.md"}),
    );
    assert_eq!(missing["ok"], json!(false), "缺失必须是失败应答：{missing}");
    assert!(
        missing["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("不存在"),
        "{missing}"
    );
    let list = call(&mut session, &args, json!({"op": "vfs_list"}));
    let artifacts = list["artifacts"].as_array().expect("artifacts");
    assert_eq!(artifacts.len(), 1, "{list}");
    assert_eq!(artifacts[0]["path"], json!("artifacts/note.md"));
    assert_eq!(artifacts[0]["sha256"], json!(sha));

    // ⑤ summary 是**只读**面：不落 RunStore 步（否则守护进程每探一次就把日志写胖）。
    let before = session.store.steps().expect("读事件日志").records.len();
    let summary = call(&mut session, &args, json!({"op": "summary", "id": "s1"}));
    let after = session.store.steps().expect("读事件日志").records.len();
    assert_eq!(after, before, "summary 不得往事件日志加步");
    assert_eq!(summary["ok"], json!(true), "{summary}");
    assert_eq!(summary["op"], json!("summary"));
    assert_eq!(summary["id"], json!("s1"));
    assert!(summary["summary"]["jobs"].is_array(), "{summary}");

    // ⑥ shutdown：唯一把 should_quit 置 true 的动作。
    let (bye, quit) = handle_daemon_request(&mut session, &args, &json!({"op": "shutdown"}));
    assert_eq!(bye["ok"], json!(true), "{bye}");
    assert!(quit, "shutdown 必须要求退出循环");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn rpc_refuses_unknown_ops_and_non_scalar_payloads() {
    let dir = case_dir("refuse");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_daemon_mod(&mod_dir);
    let args = daemon_args(&mod_dir, &runs_root, "d2");
    let mut session = Session::open(&args).expect("守护模式开局");

    let unknown = call(&mut session, &args, json!({"op": "nope"}));
    assert_eq!(unknown["ok"], json!(false), "{unknown}");
    assert!(
        unknown["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("未知的 op"),
        "{unknown}"
    );

    // 数组 / 对象不是合法载荷（与 `--payload` 同一口径：必须是 JSON 标量）。
    let array = call(
        &mut session,
        &args,
        json!({"op": "dispatch", "event": "event.tick", "payload": [1, 2]}),
    );
    assert_eq!(array["ok"], json!(false), "{array}");
    assert!(
        array["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("JSON 标量"),
        "{array}"
    );

    // 缺 event / 空 event：明确拒绝，而不是拿入口载荷去措。
    let no_event = call(&mut session, &args, json!({"op": "dispatch"}));
    assert_eq!(no_event["ok"], json!(false), "{no_event}");
    let blank = call(
        &mut session,
        &args,
        json!({"op": "dispatch", "event": "  "}),
    );
    assert_eq!(blank["ok"], json!(false), "{blank}");

    // 缺数字载荷 = 用契约的 defaultPayload（这里 "0"），不是错误。
    let defaulted = call(
        &mut session,
        &args,
        json!({"op": "dispatch", "event": "event.tick"}),
    );
    assert_eq!(defaulted["ok"], json!(true), "{defaulted}");
    assert!(
        narrative_text(&defaulted).contains("payload=0"),
        "{defaulted}"
    );

    // 制品路径形状由端口裁决（宿主不重写一份）——穿越必须被拒。
    let escaped = call(
        &mut session,
        &args,
        json!({"op": "vfs_put", "path": "../escape.md", "content": "x"}),
    );
    assert_eq!(escaped["ok"], json!(false), "{escaped}");

    let _ = fs::remove_dir_all(&dir);
}

/// T4（r11 fail-closed）：daemon `op=dispatch` 派发未注册事件 = 同语义
/// 错误应答——`ok:false` + `error.code=VALIDATION`（文案含已注册全集）、
/// `op:"dispatch"` 与 `summary` 字段仍在（调度端形状不破，一次往返的
/// 契约不受错误步影响）。
#[test]
fn rpc_dispatch_unknown_event_returns_readable_dispatch_receipt() {
    let dir = case_dir("t4");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_daemon_mod(&mod_dir);
    let args = daemon_args(&mod_dir, &runs_root, "t4");
    let mut session = Session::open(&args).expect("守护模式开局");

    let reply = call(
        &mut session,
        &args,
        json!({"op": "dispatch", "event": "event.nope", "id": 9}),
    );
    assert_eq!(reply["ok"], json!(false), "{reply}");
    assert_eq!(reply["op"], json!("dispatch"), "op 原样回填：{reply}");
    assert_eq!(reply["id"], json!(9), "{reply}");
    assert_eq!(reply["error"]["code"], json!("VALIDATION"), "{reply}");
    let message = reply["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.starts_with("未注册事件 `event.nope`："),
        "文案要点名事件：{reply}"
    );
    assert!(
        message.contains("已注册"),
        "文案要带已注册全集（write_daemon_mod 注册 event.tick / event.probe_vfs）：{reply}"
    );
    assert!(message.contains("event.tick"), "{reply}");
    // 调度端形状不破：summary 字段仍在（一次往返 = 步回执 ⊆ 摘要）。
    assert!(reply.get("summary").is_some(), "{reply}");
    assert_eq!(reply["summary"]["ok"], json!(true), "{reply}");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn daemon_resumes_an_existing_run_instead_of_resetting_it() {
    let dir = case_dir("resume");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_daemon_mod(&mod_dir);

    // 第一段：新局跑一步（ticks=1），然后丢弃进程内状态。
    {
        let args = daemon_args(&mod_dir, &runs_root, "d3");
        let mut session = Session::open(&args).expect("新局开局");
        let step = call(
            &mut session,
            &args,
            json!({"op": "dispatch", "event": "event.tick"}),
        );
        assert!(narrative_text(&step).contains("tick=1"), "{step}");
    }

    // 第二段：重启守护进程。目录里已有后端产物 → 自动续局（不重置），
    // 所以 ticks 从 1 累到 2 而不是从头开始。
    //
    // `resume` 这一行就是 `run_daemon` 的判定式（「已有后端产物即续局」）：
    // 它在 CLI 入口，不在 `handle_daemon_request` 里，所以单测只能复刻它。
    let reopen = Args {
        open_style: Some(if ygg_store::detect(&runs_root.join("d3")).any() {
            crate::OpenStyle::Open
        } else {
            crate::OpenStyle::New
        }),
        ..daemon_args(&mod_dir, &runs_root, "d3")
    };
    let mut resumed = Session::open(&reopen).expect("续局（重启守护进程不得清局）");
    let step = call(
        &mut resumed,
        &reopen,
        json!({"op": "dispatch", "event": "event.tick"}),
    );
    assert!(
        narrative_text(&step).contains("tick=2"),
        "重启必须续局（ticks 从 1 累到 2）：{step}"
    );

    let _ = fs::remove_dir_all(&dir);
}
