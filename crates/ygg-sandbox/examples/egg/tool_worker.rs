use super::*;

/// 内部模式：一次工具调用的执行体（不在 usage 里列出）。
///
/// 不打开 state 写面、不装内核、不派发——只做「读模板 → spawn → 等 → 写结果 →
/// 试认领 →（认领成功时）写 inbox 一条 -> 摘 pending」。这样即使父进程早已
/// 退出（场景 2），结果也能落地（比「父进程自己等」更强的崩溃语义）。
pub(crate) fn run_tool_worker(args: &Args, call_id: &str) -> i32 {
    if !is_valid_call_id(call_id) {
        eprintln!("error: 非法 call_id `{call_id}`（只允许 1..=64 字节 [A-Za-z0-9._-]）");
        return 2;
    }
    let run_dir = args.run_root.clone();
    let Some(call) = read_pending_call(&run_dir, call_id) else {
        // 父进程已收口 / 本局已重置：无活可干（不是错误）。
        return 0;
    };
    let env = load_env_table(args.mod_dir.as_deref());
    let scratch = tool_results_dir(&run_dir);
    if let Err(message) = fs::create_dir_all(&scratch) {
        eprintln!("error: 创建 {} 失败：{message}", scratch.display());
        return 1;
    }
    let outcome = execute_tool(&call, &env, &scratch);
    let external_id = new_external_id(run_basename(&args.run_root), 0);
    let result = ToolResult::from_outcome(&call, &outcome, &external_id);
    if let Err(message) = write_tool_result(&run_dir, &result) {
        eprintln!("error: {message}");
        return 1;
    }
    // 交付责任要等父进程的等待窗口过去（否则快工具永远抢不到场景 1）：
    // 结果文件已经写好，父进程在 `inlineWaitMs` 内看到就会就地认领并派发。
    let grace_until = if call.inline_wait_ms == 0 {
        0
    } else {
        call.started_at_unix_ms + u128::from(call.inline_wait_ms) + u128::from(TOOL_CLAIM_SLACK_MS)
    };
    let now = now_ms();
    if grace_until > now {
        std::thread::sleep(Duration::from_millis((grace_until - now) as u64));
    }
    // 父进程已先认领 → 派发责任在父进程，本进程只留结果文件，静默退出。
    if !claim_tool_result(&run_dir, &call.call_id) {
        return 0;
    }
    // 认领成功 = 交付责任在本进程：写 inbox 一条，成功后摘除 pending 条目。
    let delivery = (|| -> Result<(), String> {
        let mod_dir = resolve_mod_dir(args, &run_dir)?;
        let routes = InboxRoutes::load(&mod_dir)?;
        let kind = routes
            .write_binding("tool-result")
            .map(|(kind, _)| kind.to_owned())
            .ok_or_else(|| {
                format!(
                    "本 Mod 的 pack.json `inboxRoutes` 未声明 `--tool-result` 的写命令；\
                     结果留在 {} 与 {}（可人工 --tool-result 补票；{CONTRACT_DOC}）",
                    PENDING_TOOLS_FILE,
                    tool_result_path(&run_dir, &call.call_id).display()
                )
            })?;
        let mut store = open_store_resolved(&run_dir, args.store)?;
        // worker 自身环境的身份（spawn 侧注入 YGG_ACTOR / YGG_ACTOR_SESSION；
        // 未注入 = 不盖章不认领，交付行为不变）。
        let identity = resolve_identity(args)?;
        append_tool_result_record(
            &run_dir,
            store.as_mut(),
            &routes,
            &kind,
            &result.payload,
            run_basename(&args.run_root),
            &identity,
        )
        .map(|_| ())
    })();
    match delivery {
        Ok(()) => {
            let _ = pending_tools_remove(&run_dir, call_id);
            0
        }
        Err(message) => {
            // 交付失败就**保留** pending：`--wait-tools` 会报 TOOLS_PENDING，
            // 并给出人工补票入口（自动重跑会打破单次执行语义，不做）。
            eprintln!("error: {message}");
            1
        }
    }
}

/// `--wait-tools`：等本局在飞工具全部落地（结果由 worker 写 inbox）后消费一次
/// inbox 并打印回执。
///
/// **只观察**：走 `OpenMode::Attach`——读档失败就失败，绝不重置别人的局。
pub(crate) fn run_wait_tools(args: &Args) -> i32 {
    let run_dir = args.run_root.clone();
    let timeout = args.tool_timeout_ms.unwrap_or(WAIT_TOOLS_DEFAULT_MS);
    let deadline = Instant::now() + Duration::from_millis(timeout);
    let mut last_beat = Instant::now();
    loop {
        let pending = pending_calls(&run_dir);
        if pending.is_empty() {
            break;
        }
        if Instant::now() >= deadline {
            let list: Vec<Json> = pending
                .iter()
                .map(|call| {
                    json!({
                        "callId": call.call_id,
                        "tool": call.tool,
                        "deadlineUnixMs": call.deadline_unix_ms as u64,
                    })
                })
                .collect();
            let message = format!(
                "等待在飞工具超时（{timeout}ms）：{} 个调用仍未落地。worker 崩溃留下的条目\
                 不会自动重跑（重跑会破坏单次执行语义），可用 --tool-result 人工补票",
                pending.len()
            );
            eprintln!("error: {message}");
            println!(
                "{}",
                json!({"ok": false, "error": {"code": "TOOLS_PENDING", "message": message}, "pending": list})
            );
            return 1;
        }
        if last_beat.elapsed() >= WAIT_TOOLS_HEARTBEAT {
            let list = pending
                .iter()
                .map(|call| format!("{}({})", call.call_id, call.tool))
                .collect::<Vec<_>>()
                .join(" ");
            eprintln!("[wait-tools] 仍在飞：{list}");
            last_beat = Instant::now();
        }
        std::thread::sleep(TOOL_POLL_INTERVAL);
    }
    // 在飞清零 → 消费 inbox（结果行已落盘，drain 会按 Mod 的 tool_result 路由派发）
    let mod_dir = match resolve_mod_dir(args, &run_dir) {
        Ok(dir) => dir,
        Err(message) => return wait_tools_failure(&message),
    };
    let open_args = Args {
        mod_dir: Some(mod_dir),
        ..args.clone()
    };
    let mut session = match Session::open_mode(&open_args, OpenMode::Attach) {
        Ok(session) => session,
        Err(message) => return wait_tools_failure(&message),
    };
    let drain = session.drain_inbox();
    let receipt = match &drain.last {
        Some(receipt) => receipt.clone(),
        None => state_receipt(&mut session, "", Vec::new()),
    };
    // stdout 只留回执行（Agent 的解析面）；警告另走 stderr 便于人读。
    if let Some(warnings) = receipt["warnings"].as_array() {
        for warning in warnings.iter().filter_map(Json::as_str) {
            eprintln!("[警告] {warning}");
        }
    }
    println!("{receipt}");
    match &drain.error {
        Some(message) => {
            eprintln!(
                "error: inbox 未能全部并入：{message}（剩 {} 行未消费）",
                drain.remaining
            );
            1
        }
        None => 0,
    }
}

/// `--wait-tools` 的失败回执（与单发模式同一条解析路径）。
pub(crate) fn wait_tools_failure(message: &str) -> i32 {
    eprintln!("error: {message}");
    println!(
        "{}",
        json!({"ok": false, "error": {"code": "INTERNAL", "message": message}})
    );
    1
}
