use super::*;

/// 进程内同步工具执行器（挂载到 `KernelFacade`，实现 `ygg.tool.call` 同步桥）。
///
/// 强制校验两层白名单：Layer 1（`pack.json` 的 `tools`）与 Layer 2（`data/config.json` 的 `allowedTools`）。
/// 六个内置工具直接在当前进程内调用 `*_fields_with`，不派生子进程；外部工具走 `execute_tool`。
pub(crate) struct InProcessToolRunner {
    pub(crate) tools: ToolRegistry,
    pub(crate) tool_access: ToolAccess,
    pub(crate) mod_dir: PathBuf,
    pub(crate) run_dir: PathBuf,
    pub(crate) runs_root: PathBuf,
    pub(crate) run_id: String,
    pub(crate) harness: Option<String>,
}

impl InProcessToolRunner {
    fn parse_kv_output(raw: &str) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return map;
        }
        let mut any_pair = false;
        for segment in trimmed.split(';') {
            if let Some((k, v)) = segment.split_once('=') {
                let key = k.trim();
                if !key.is_empty() {
                    map.insert(key.to_owned(), v.trim().to_owned());
                    any_pair = true;
                }
            }
        }
        if !any_pair {
            map.insert("output".to_owned(), trimmed.to_owned());
        }
        map
    }

    fn fields_to_map(fields: Vec<(&'static str, String)>) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        for (k, v) in fields {
            map.insert(k.to_owned(), wire_sanitize(&v));
        }
        map
    }

    fn op_failure_to_err(op: &str, fail: OpFailure) -> String {
        match fail {
            OpFailure::Line(line) => line,
            OpFailure::Detail { slug, detail } => {
                format!(
                    "op={};error={};detail={}",
                    wire_sanitize(op),
                    wire_sanitize(slug),
                    wire_sanitize(&detail)
                )
            }
            OpFailure::Reported => format!("op={};error=reported", wire_sanitize(op)),
        }
    }

    fn try_run_builtin(
        &self,
        expanded_args: &[String],
    ) -> Option<Result<BTreeMap<String, String>, String>> {
        if expanded_args.len() < 2 || expanded_args[0] != "--builtin-tool" {
            return None;
        }
        let tool = BuiltinTool::parse(&expanded_args[1])?;
        let mut values: Vec<(String, String)> = Vec::new();
        let mut idx = 2;
        while idx < expanded_args.len() {
            let key = &expanded_args[idx];
            if key.starts_with("--") && idx + 1 < expanded_args.len() {
                values.push((key.clone(), expanded_args[idx + 1].clone()));
                idx += 2;
            } else {
                idx += 1;
            }
        }
        if tool == BuiltinTool::ArtifactProbe && !values.iter().any(|(k, _)| k == "--run-dir") {
            values.push(("--run-dir".to_owned(), self.run_dir.display().to_string()));
        }
        let flags = OpFlags { values };
        let op_name = tool.as_str();
        let outcome = match tool {
            BuiltinTool::WorktreeClaim => {
                if flags.get("--lane").is_none()
                    || flags.get("--req").is_none()
                    || flags.get("--branch").is_none()
                {
                    return Some(Err("missing required worktree-claim flags".to_owned()));
                }
                claim_fields_with(&flags)
            }
            BuiltinTool::WorktreeRelease => {
                if flags.get("--lane").is_none() {
                    return Some(Err("missing required worktree-release --lane".to_owned()));
                }
                release_fields_with(&flags)
            }
            BuiltinTool::LaneProbe => {
                if flags.get("--lane").is_none()
                    || flags.get("--base").is_none()
                    || flags.get("--branch").is_none()
                {
                    return Some(Err("missing required lane-probe flags".to_owned()));
                }
                lane_probe_fields_with(&flags)
            }
            BuiltinTool::ArtifactProbe => {
                if flags.get("--path").is_none() {
                    return Some(Err("missing required artifact-probe --path".to_owned()));
                }
                artifact_probe_fields_with(&flags)
            }
            BuiltinTool::VerifySuite => verify_suite_fields_with(&flags),
            BuiltinTool::ArchiveSessions => {
                if flags.get("--parent").is_none() || flags.get("--batch").is_none() {
                    return Some(Err("missing required archive-sessions flags".to_owned()));
                }
                let inputs = ArchiveInputs {
                    harness: self.harness.clone(),
                    mod_dir: Some(self.mod_dir.clone()),
                    sessions_dir: std::env::var("YGG_SESSIONS_DIR")
                        .ok()
                        .filter(|v| !v.trim().is_empty())
                        .map(PathBuf::from),
                    home: home_dir(),
                    cwd: std::env::current_dir().ok(),
                    env_harness: std::env::var("YGG_HARNESS")
                        .ok()
                        .filter(|v| !v.trim().is_empty()),
                };
                archive_sessions_fields_with(&flags, &inputs)
            }
        };
        Some(
            outcome
                .map(Self::fields_to_map)
                .map_err(|fail| Self::op_failure_to_err(op_name, fail)),
        )
    }
}

impl HostToolRunner for InProcessToolRunner {
    fn call(
        &mut self,
        tool_id: &str,
        args: &BTreeMap<String, String>,
    ) -> Result<BTreeMap<String, String>, String> {
        let Some(spec) = self.tools.get(tool_id).cloned() else {
            return Err(format!("工具 `{tool_id}` 未在 pack.json 的 `tools` 段声明"));
        };
        if !self.tool_access.allows(&spec.id) {
            return Err(format!("工具 `{}` 未被本局 `allowedTools` 放行", spec.id));
        }
        let mut env = load_env_table(Some(&self.mod_dir));
        env.process
            .push(("YGG_RUN_DIR".to_owned(), self.run_dir.display().to_string()));
        env.process.push((
            "YGG_RUNS_ROOT".to_owned(),
            self.runs_root.display().to_string(),
        ));
        env.process
            .push(("YGG_RUN_ID".to_owned(), self.run_id.clone()));
        env.process
            .push(("YGG_MOD_DIR".to_owned(), self.mod_dir.display().to_string()));
        let arg_pairs: Vec<(String, String)> =
            args.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        validate_expansion(&spec, &arg_pairs, &env)?;

        if spec.kind == ToolKind::Cli {
            let is_self = spec.program.as_deref() == Some(ygg_sandbox::host_contract::SELF_PROGRAM)
                || spec.program.as_deref() == Some(SELF_ALIAS);
            if is_self {
                let mut expanded = Vec::with_capacity(spec.args.len());
                for tpl in &spec.args {
                    expanded.push(substitute(tpl, &arg_pairs, &env)?);
                }
                if let Some(res) = self.try_run_builtin(&expanded) {
                    return res;
                }
            }
        }

        let request = ToolRequest {
            id: format!("sync-{}", now_ms()),
            tool: spec.id.clone(),
            args: arg_pairs,
        };
        let call =
            PendingCall::from_spec(&spec, &request, now_ms() + u128::from(spec.timeout_ms), 1);
        let scratch = tool_results_dir(&self.run_dir);
        let _ = fs::create_dir_all(&scratch);
        let outcome = execute_tool(&call, &env, &scratch);
        if outcome.status == "ok" {
            Ok(Self::parse_kv_output(&outcome.output))
        } else {
            Err(outcome.output)
        }
    }
}

/// 子进程结局（`TimedOut` = 已被 kill 并回收）。
pub(crate) enum ChildOutcome {
    Exited(std::process::ExitStatus),
    TimedOut,
}

/// spawn 一个子进程并轮询到退出或超时（stdout / stderr 落临时文件，避开管道死锁）。
pub(crate) fn run_child(
    mut command: Command,
    label: &str,
    timeout_ms: u64,
    scratch: &Path,
    tag: &str,
) -> Result<ChildOutcome, String> {
    let stdout_path = scratch.join(format!("{tag}.stdout"));
    let stderr_path = scratch.join(format!("{tag}.stderr"));
    let stdout = File::create(&stdout_path)
        .map_err(|err| format!("创建 {} 失败：{err}", stdout_path.display()))?;
    let stderr = File::create(&stderr_path)
        .map_err(|err| format!("创建 {} 失败：{err}", stderr_path.display()))?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    let mut child = command
        .spawn()
        .map_err(|err| format!("无法启动 `{label}`：{err}"))?;
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(ChildOutcome::Exited(status)),
            Ok(None) => {}
            Err(err) => return Err(format!("等待 `{label}` 失败：{err}")),
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(ChildOutcome::TimedOut);
        }
        std::thread::sleep(TOOL_POLL_INTERVAL);
    }
}

/// 读一个临时产物文件（最多 `limit` 字节，非法 UTF-8 用替换符兵底）。
pub(crate) fn read_limited(path: &Path, limit: usize) -> String {
    let Ok(file) = File::open(path) else {
        return String::new();
    };
    let mut buffer = Vec::new();
    let _ = file.take(limit as u64).read_to_end(&mut buffer);
    String::from_utf8_lossy(&buffer).into_owned()
}

/// 执行一次工具（worker 侧；每轮只调一次）。
pub(crate) fn execute_tool(call: &PendingCall, env: &EnvTable, scratch: &Path) -> ToolOutcome {
    let outcome = match call.kind {
        ToolKind::Cli => execute_cli(call, env, scratch),
        ToolKind::Http => execute_http(call, env, scratch),
    };
    // 暂存文件只服务本次执行：结局已经进内存 / 结果文件，删掉（不把工具输出
    // 以原形留在 run 目录里等下一次 `git add`）。
    for suffix in ["stdout", "stderr"] {
        let _ = fs::remove_file(scratch.join(format!("{}.{suffix}", call.call_id)));
    }
    let _ = fs::remove_file(scratch.join(format!("{}-body.out", call.call_id)));
    outcome
}

/// `cli`：直接 spawn（**无 shell**），`status.success()` 即 `ok`。
pub(crate) fn execute_cli(call: &PendingCall, env: &EnvTable, scratch: &Path) -> ToolOutcome {
    let Some(program) = call.program.clone() else {
        return ToolOutcome::error(
            "000",
            format!("工具 `{}` 的声明缺少 program", call.tool),
            call.max_output_bytes,
        );
    };
    // `@self` / `egg` → 本可执行文件。两个名字都解析到 `current_exe()`：
    // 内置子命令不能依赖 `$PATH`（部署方可能根本没把 `egg` 装进 PATH，
    // 而 `pack.json` 是静态文件、不能内插绝对路径）。
    let executable = if program == ygg_sandbox::host_contract::SELF_PROGRAM || program == SELF_ALIAS
    {
        match std::env::current_exe() {
            Ok(path) => path.display().to_string(),
            Err(err) => {
                return ToolOutcome::error(
                    "127",
                    format!("工具 `{}` 的 @self 解析失败：{err}", call.tool),
                    call.max_output_bytes,
                )
            }
        }
    } else {
        program.clone()
    };
    let mut command = Command::new(&executable);
    for template in &call.args {
        match substitute(template, &call.placeholders, env) {
            Ok(value) => {
                command.arg(value);
            }
            Err(message) => {
                return ToolOutcome::error(
                    "126",
                    format!("工具 `{}` 的{message}", call.tool),
                    call.max_output_bytes,
                )
            }
        }
    }
    match run_child(command, &program, call.timeout_ms, scratch, &call.call_id) {
        Ok(ChildOutcome::Exited(status)) => {
            let limit = call.max_output_bytes.saturating_add(64);
            let stdout = read_limited(&scratch.join(format!("{}.stdout", call.call_id)), limit);
            let stderr = read_limited(&scratch.join(format!("{}.stderr", call.call_id)), limit);
            let output = if stdout.trim().is_empty() {
                stderr
            } else {
                stdout
            };
            let code = match status.code() {
                Some(code) => code.to_string(),
                None => "signal".to_owned(),
            };
            ToolOutcome {
                status: if status.success() { "ok" } else { "err" },
                code,
                output: sanitize_tool_output(&output, call.max_output_bytes),
            }
        }
        Ok(ChildOutcome::TimedOut) => ToolOutcome::error(
            "timeout",
            format!(
                "工具 `{}` 执行超时（{}ms），已终止",
                call.tool, call.timeout_ms
            ),
            call.max_output_bytes,
        ),
        Err(message) => ToolOutcome::error("127", message, call.max_output_bytes),
    }
}

/// `http`：经 `curl` 发一次请求（`code` = `%{http_code}`；2xx 即 `ok`）。
pub(crate) fn execute_http(call: &PendingCall, env: &EnvTable, scratch: &Path) -> ToolOutcome {
    let (Some(method), Some(url_template)) = (call.method.clone(), call.url.clone()) else {
        return ToolOutcome::error(
            "000",
            format!("工具 `{}` 的声明缺少 method / url", call.tool),
            call.max_output_bytes,
        );
    };
    let url = match substitute(&url_template, &call.placeholders, env) {
        Ok(value) => value,
        Err(message) => {
            return ToolOutcome::error(
                "126",
                format!("工具 `{}` 的{message}", call.tool),
                call.max_output_bytes,
            )
        }
    };
    let body_path = scratch.join(format!("{}-body.out", call.call_id));
    let mut command = Command::new("curl");
    command
        .arg("-sS")
        .arg("-X")
        .arg(&method)
        .arg("--max-time")
        .arg(call.timeout_ms.div_ceil(1000).max(1).to_string())
        .arg("-o")
        .arg(&body_path)
        .arg("-w")
        .arg("%{http_code}");
    for (key, template) in &call.headers {
        match substitute(template, &call.placeholders, env) {
            Ok(value) => {
                command.arg("-H").arg(format!("{key}: {value}"));
            }
            Err(message) => {
                return ToolOutcome::error(
                    "126",
                    format!("工具 `{}` 的{message}", call.tool),
                    call.max_output_bytes,
                )
            }
        }
    }
    if let Some(template) = &call.body_template {
        match substitute(template, &call.placeholders, env) {
            Ok(body) => {
                command.arg("--data-binary").arg(body);
            }
            Err(message) => {
                return ToolOutcome::error(
                    "126",
                    format!("工具 `{}` 的{message}", call.tool),
                    call.max_output_bytes,
                )
            }
        }
    }
    command.arg(&url);
    match run_child(command, "curl", call.timeout_ms, scratch, &call.call_id) {
        Ok(ChildOutcome::Exited(status)) => {
            let limit = call.max_output_bytes.saturating_add(64);
            let code = {
                let raw = read_limited(&scratch.join(format!("{}.stdout", call.call_id)), 16);
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    "000".to_owned()
                } else {
                    sanitize_tool_output(trimmed, 16)
                }
            };
            let body = read_limited(&body_path, limit);
            let stderr = read_limited(&scratch.join(format!("{}.stderr", call.call_id)), limit);
            let output = if body.trim().is_empty() { stderr } else { body };
            let http_ok = code
                .parse::<u16>()
                .is_ok_and(|code| (200..=299).contains(&code));
            let _ = status; // curl 的退出码不进 wire：`code` 是 HTTP 状态码
            ToolOutcome {
                status: if http_ok { "ok" } else { "err" },
                code,
                output: sanitize_tool_output(&output, call.max_output_bytes),
            }
        }
        Ok(ChildOutcome::TimedOut) => ToolOutcome::error(
            "timeout",
            format!(
                "工具 `{}` 执行超时（{}ms），已终止",
                call.tool, call.timeout_ms
            ),
            call.max_output_bytes,
        ),
        Err(message) => ToolOutcome::error("127", message, call.max_output_bytes),
    }
}

/// 写一条工具结果 inbox 记录（worker 侧；与 `--tool-result` 同一条通道）。
///
/// 记录字段名 / 写入形态全部来自 Mod 的 `inboxRoutes` 声明；幂等键取 wire 载荷
/// 的末段（同一值，Mod 侧去重与审计看同一处）。
pub(crate) fn append_tool_result_record(
    run_dir: &Path,
    store: &mut dyn RunStore,
    routes: &InboxRoutes,
    kind: &str,
    payload: &str,
    run_id: &str,
    identity: &ActorIdentity,
) -> Result<Json, String> {
    let route = routes
        .get(kind)
        .ok_or_else(|| format!("inboxRoutes 未声明 kind `{kind}`"))?;
    let Some(shape) = route.write.as_ref().map(|binding| binding.shape) else {
        return Err(format!(
            "inboxRoutes 的 `{kind}` 没有 write 绑定（worker 无处写结果）"
        ));
    };
    let external_field = routes.external_id_field();
    let payload_field = routes.payload_field();
    let mut record = json!({"kind": kind, "created_at": now_secs()});
    let external_id = payload.rsplit('|').next().unwrap_or("").trim();
    record[external_field] = json!(if external_id.is_empty() {
        new_external_id(run_id, store.inbox_len().unwrap_or(0))
    } else {
        external_id.to_owned()
    });
    match shape {
        WriteShape::Payload => record[payload_field] = json!(payload.trim()),
        WriteShape::Named => {
            let (first, rest) = split_requirement(payload);
            let mut named = route.named_fields(external_field);
            if let Some(field) = named.next() {
                record[field.as_str()] = json!(first);
            }
            if let Some(field) = named.next() {
                record[field.as_str()] = json!(rest);
            }
        }
    }
    // 提交者章（worker 自身环境的身份解析产物；None 不写键）：与 `--requirement`
    // 写命令同一套保留键，drain 消费时按记录取回注入声明路由。
    if let Some(actor) = &identity.actor {
        record["actor"] = json!(actor);
    }
    if let Some(session) = &identity.session {
        record["actor_session"] = json!(session);
    }
    // 写前登记（与 `--requirement` 同一准入门禁）：closing / finalized 一律拒绝。
    let token = begin_write(run_dir).map_err(|gate| match gate {
        WriteGate::Closing => {
            "本局正在收口（closing）——工具结果未写入，稍后重试或开新局".to_owned()
        }
        WriteGate::Finalized => "本局已 finalized——工具结果未写入，需开新局".to_owned(),
        WriteGate::Internal(message) => message,
    })?;
    let pushed = store.inbox_push(&record, clock());
    if pushed.is_ok() {
        // opener 认领（成功落行 + 携带身份；仍在 token 临界区内，与写命令同位）。
        if let Some(actor) = &identity.actor {
            if let Err(message) = claim_opener(run_dir, actor, identity.session.as_deref()) {
                eprintln!("[警告] opener 认领失败（不影响本条写入）：{message}");
            }
        }
    }
    end_write(run_dir, &token);
    pushed.map_err(|err| err.to_string())?;
    Ok(record)
}

/// 工具回投深度的归零守卫（panic 展开也归零）。
pub(crate) struct ToolDepthGuard(pub(crate) Rc<Cell<usize>>);

impl Drop for ToolDepthGuard {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(1));
    }
}

// ============================================================================
// §工具执行器（r1）：契约装载 / 场景 1 就地回投 / 场景 2 脱体 worker + inbox
//
// 全部用例都用**合成 Mod**（本节自己的 `write_tool_mod`）：真实 `.ygg/mods/**`
// 的 `tools` / `allowedTools` 声明由后续迭代落地，本节的断言不依赖它。
// ============================================================================
#[cfg(test)]
mod tests;
