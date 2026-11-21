use super::*;

pub(crate) fn pending_tools_path(run_dir: &Path) -> PathBuf {
    run_dir.join(PENDING_TOOLS_FILE)
}

pub(crate) fn tool_results_dir(run_dir: &Path) -> PathBuf {
    run_dir.join(TOOL_RESULTS_DIR)
}

pub(crate) fn tool_result_path(run_dir: &Path, call_id: &str) -> PathBuf {
    tool_results_dir(run_dir).join(format!("{call_id}.json"))
}

pub(crate) fn tool_claim_path(run_dir: &Path, call_id: &str) -> PathBuf {
    tool_results_dir(run_dir).join(format!("{call_id}.claim"))
}

/// 在飞调用摘要（`--wait-tools` / `--settle` / 心跳文案共用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingSummary {
    pub(crate) call_id: String,
    pub(crate) tool: String,
    pub(crate) deadline_unix_ms: u128,
}

/// 无锁读在飞清单（`atomic_write` 保证读者看到旧全文或新全文）。
///
/// 轮询路径**不取锁**：取锁会让 `--wait-tools` 与运行中的局互相拖慢。
pub(crate) fn pending_calls(run_dir: &Path) -> Vec<PendingSummary> {
    let Some(record) = read_json(&pending_tools_path(run_dir)) else {
        return Vec::new();
    };
    let Some(calls) = record.get("calls").and_then(Json::as_object) else {
        return Vec::new();
    };
    calls
        .iter()
        .map(|(call_id, value)| PendingSummary {
            call_id: call_id.clone(),
            tool: value["tool"].as_str().unwrap_or("?").to_owned(),
            deadline_unix_ms: value["deadlineUnixMs"].as_u64().map_or(0, u128::from),
        })
        .collect()
}

/// 锁内读—改—写 `pending_tools.json`（与 `coordination_update` 同一把锁、同风格）。
///
/// 临界区只做「读一个文件 + 写一个文件」：工具进程的启动与等待**绝不**在锁内（I2）。
pub(crate) fn pending_tools_update<T>(
    run_dir: &Path,
    f: impl FnOnce(&mut Json) -> T,
) -> Result<T, String> {
    let _guard = acquire_run_lock(run_dir)?;
    let path = pending_tools_path(run_dir);
    let mut record = read_json(&path).unwrap_or_else(|| json!({"version": 1, "calls": {}}));
    if !record.get("calls").is_some_and(Json::is_object) {
        record["calls"] = json!({});
    }
    record["version"] = json!(1);
    let out = f(&mut record);
    atomic_write(&path, record.to_string().as_bytes())?;
    Ok(out)
}

/// 登记一条在飞调用（锁内）。返回 `Ok(true)` 表示首次登记（调用方负责 spawn worker），
/// `Ok(false)` 表示该 `call_id` 已在飞或已被认领（幂等跳过，绝不二次 spawn —— I3）。
pub(crate) fn pending_tools_insert(run_dir: &Path, call: &PendingCall) -> Result<bool, String> {
    if tool_claim_path(run_dir, &call.call_id).exists() {
        return Ok(false);
    }
    pending_tools_update(run_dir, |record| {
        if let Some(calls) = record["calls"].as_object_mut() {
            if calls.contains_key(&call.call_id) {
                return false;
            }
            calls.insert(call.call_id.clone(), call.to_json());
            return true;
        }
        false
    })
}

/// 摘除一条在飞调用（锁内；已不存在 = 幂等成功）。
pub(crate) fn pending_tools_remove(run_dir: &Path, call_id: &str) -> Result<(), String> {
    pending_tools_update(run_dir, |record| {
        if let Some(calls) = record["calls"].as_object_mut() {
            calls.remove(call_id);
        }
    })
}

/// 读一条在飞调用的模板（worker 侧；无锁读）。
pub(crate) fn read_pending_call(run_dir: &Path, call_id: &str) -> Option<PendingCall> {
    let record = read_json(&pending_tools_path(run_dir))?;
    let value = record.get("calls")?.get(call_id)?;
    PendingCall::from_json(call_id, value).ok()
}

/// 原子写结果文件（读者不会看到半截 JSON）。
pub(crate) fn write_tool_result(run_dir: &Path, result: &ToolResult) -> Result<(), String> {
    let dir = tool_results_dir(run_dir);
    fs::create_dir_all(&dir).map_err(|err| format!("创建 {} 失败：{err}", dir.display()))?;
    atomic_write(
        &tool_result_path(run_dir, &result.call_id),
        result.to_json().to_string().as_bytes(),
    )
}

/// 无锁读结果文件（父进程轮询路径）。
pub(crate) fn read_tool_result(run_dir: &Path, call_id: &str) -> Option<ToolResult> {
    let value = read_json(&tool_result_path(run_dir, call_id))?;
    ToolResult::from_json(&value)
}

/// 交付认领（`O_EXCL`）：**只有一个**进程能认领成功（I3 的交付侧保证）。
pub(crate) fn claim_tool_result(run_dir: &Path, call_id: &str) -> bool {
    let dir = tool_results_dir(run_dir);
    let _ = fs::create_dir_all(&dir);
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(tool_claim_path(run_dir, call_id))
    {
        Ok(mut file) => {
            let _ = writeln!(file, "{}", std::process::id());
            true
        }
        Err(_) => false,
    }
}

/// 从 `summary` 对象提取 `jobs[].subagentAction` 列表。
pub(crate) fn extract_harness_actions(summary: &Json) -> Vec<Json> {
    summary["jobs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|job| {
            let action = job.get("subagentAction")?;
            action.is_object().then(|| action.clone())
        })
        .collect()
}

/// 从回执信封提取 `harnessActions`（优先 `meta.harness_actions`，其次 `meta.<metaKey>.jobs[].subagentAction`）。
pub(crate) fn envelope_harness_actions(envelope: &Json, meta_key: &str) -> Vec<Json> {
    if let Some(items) = envelope["meta"]["harness_actions"].as_array() {
        let out: Vec<Json> = items
            .iter()
            .filter(|item| item.is_object())
            .cloned()
            .collect();
        if !out.is_empty() {
            return out;
        }
    }
    extract_harness_actions(&envelope["meta"][meta_key])
}

/// 渲染通用 CLI 模式下的 `[HARNESS ACTION]` 摘要行。
pub(crate) fn format_harness_action_lines(receipt: &Json) -> Vec<String> {
    let mut lines = Vec::new();
    for action in receipt["harnessActions"].as_array().into_iter().flatten() {
        let kind = action["action"].as_str().unwrap_or("subagent.spawn");
        let req_id = action["reqId"].as_str().unwrap_or("?");
        let stage = action["stage"].as_str().unwrap_or("?");
        let rev = action["rev"].as_u64().unwrap_or(1);
        let lane = action["lane"].as_u64().unwrap_or(0);
        let wt = action["worktreePath"].as_str().unwrap_or("");
        let artifact = action["artifactPath"].as_str().unwrap_or("");
        let wt_display = if wt.is_empty() { "." } else { wt };
        lines.push(format!(
            "[HARNESS ACTION] {kind} {req_id} (rev {rev}, stage={stage}, lane={lane}, worktree={wt_display}, artifact={artifact})"
        ));
    }
    lines
}
