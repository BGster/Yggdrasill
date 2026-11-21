use super::*;

/// `--builtin-tool` 的操作枚举（解析即校验；未知值 = 用法错误，拼出全部合法值）。
///
/// 这六个子命令是**可移植的内置工具**：`pack.json` 里写 `"program": "@self"`
/// 即可调用（`@self` = 本宿主可执行文件），部署方不必自己写 shell wrapper。
/// 输出契约见 `run_builtin_tool` 的文档（单行 `k=v;k=v`，失败 = stdout 空 + 退出 1）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BuiltinTool {
    WorktreeClaim,
    WorktreeRelease,
    LaneProbe,
    ArtifactProbe,
    VerifySuite,
    ArchiveSessions,
}

impl BuiltinTool {
    /// 全部取值（用法提示 / 错误文案共用一份，避免各处硬编码）。
    pub(crate) const NAMES: &'static [&'static str] = &[
        "worktree-claim",
        "worktree-release",
        "lane-probe",
        "artifact-probe",
        "verify-suite",
        "archive-sessions",
    ];

    /// `--builtin-tool` 的取值解析（未知 = `None`，由调用方拼出合法表）。
    pub(crate) fn parse(text: &str) -> Option<Self> {
        match text {
            "worktree-claim" => Some(Self::WorktreeClaim),
            "worktree-release" => Some(Self::WorktreeRelease),
            "lane-probe" => Some(Self::LaneProbe),
            "artifact-probe" => Some(Self::ArtifactProbe),
            "verify-suite" => Some(Self::VerifySuite),
            "archive-sessions" => Some(Self::ArchiveSessions),
            _ => None,
        }
    }

    /// 回写用法提示时用的名字（与 [`BuiltinTool::parse`] 同源）。
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::WorktreeClaim => "worktree-claim",
            Self::WorktreeRelease => "worktree-release",
            Self::LaneProbe => "lane-probe",
            Self::ArtifactProbe => "artifact-probe",
            Self::VerifySuite => "verify-suite",
            Self::ArchiveSessions => "archive-sessions",
        }
    }
}

// ============================================================================
// §工具执行器（场景 1 同进程回投 / 场景 2 脱体 worker + inbox 回写）
//
// 设计要点（实现计划 §5.1 的 DC-1 修正）：
//   ① 真实子进程**只在 worker 里 spawn 一次**——父进程超时后不 kill、不重跑
//      （I3：非幂等工具的单次执行语义不能被「等不及」打破）；
//   ② `pending_tools.json` 只落**未解析模板**（I4：`$ENV{}` 的值绝不落盘）；
//   ③ 两条路径产出**同一 wire 载荷**（I5：Mod 侧只有一套解析）：
//      `<call_id>|<tool_id>|<status>|<code>|<output>|<external_id>`；
//   ④ 工具进程的启动与等待**全部在 `coordination.lock` 锁外**（I2 / R1——锁内
//      只做「读一个 json + 写一个 json」）；
//   ⑤ 交付责任用 `tool_results/<call_id>.claim`（`O_EXCL`）只认领一次：
//      父进程认领 = 就地 `dispatch_event`（场景 1）；worker 认领 = 写 inbox（场景 2）。
// ============================================================================

/// 一条 `meta.tool_calls` 请求（Mod 声明 → 宿主执行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolRequest {
    /// 调用 id（缺省 = `<run_id>-<seq>-<index>`；它同时是结果文件名）。
    pub(crate) id: String,
    /// 工具 id（`pack.json` 的 `tools` 段的键）。
    pub(crate) tool: String,
    /// `{{key}}` 占位值（只放 Mod 给的标量，非凭据）。
    pub(crate) args: Vec<(String, String)>,
}

/// call_id 白名单：`[A-Za-z0-9._-]`、1..=64 字节（它同时是 `tool_results/` 的文件名）。
pub(crate) fn is_valid_call_id(call_id: &str) -> bool {
    !call_id.is_empty()
        && call_id.len() <= 64
        && call_id != "."
        && call_id != ".."
        && call_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// JSON 值的类型名（错误文案用；不透传原值）。
pub(crate) fn json_type_name(value: &Json) -> &'static str {
    match value {
        Json::Null => "null",
        Json::Bool(_) => "布尔",
        Json::Number(_) => "数字",
        Json::String(_) => "字符串",
        Json::Array(_) => "数组",
        Json::Object(_) => "对象",
    }
}

/// `meta.tool_calls` 解析（容错：坏条目只丢该条，绝不把整步判失败）。
///
/// 返回 `(请求清单, 警告清单)`：警告由调用方收进 `self.notices`（回执 warnings）。
pub(crate) fn parse_tool_calls(meta: &Json, prefix: &str) -> (Vec<ToolRequest>, Vec<String>) {
    let mut requests = Vec::new();
    let mut notices = Vec::new();
    let Some(value) = meta.get("tool_calls") else {
        return (requests, notices);
    };
    let Some(items) = value.as_array() else {
        notices.push(format!(
            "meta.tool_calls 必须是数组（当前是 {}），已忽略",
            json_type_name(value)
        ));
        return (requests, notices);
    };
    for (index, item) in items.iter().enumerate() {
        match parse_tool_call(item, &format!("{prefix}-{index}")) {
            Ok(request) => requests.push(request),
            Err(message) => notices.push(format!(
                "meta.tool_calls 第 {} 项：{message}（已跳过）",
                index + 1
            )),
        }
    }
    (requests, notices)
}

/// 单条工具请求解析（`args` 只接受标量；值必须 wire 安全）。
pub(crate) fn parse_tool_call(value: &Json, fallback_id: &str) -> Result<ToolRequest, String> {
    let Some(object) = value.as_object() else {
        return Err("不是对象".to_owned());
    };
    let tool = object
        .get("tool")
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| "缺 `tool`（必需，非空字符串）".to_owned())?;
    let id = match object.get("id").and_then(Json::as_str).map(str::trim) {
        Some(text) if is_valid_call_id(text) => text.to_owned(),
        Some(text) if !text.is_empty() => {
            return Err(format!(
                "`id` 不是合法 call_id（只允许 1..=64 字节 `[A-Za-z0-9._-]`，当前是 `{text}`）"
            ))
        }
        _ => fallback_id.to_owned(),
    };
    let args = match object.get("args") {
        None | Some(Json::Null) => Vec::new(),
        // Lua 空表会序列化成 `[]`（`src/host_api.rs` 的已文档化怪癖）：`[]` = 空参数。
        Some(Json::Array(items)) if items.is_empty() => Vec::new(),
        Some(Json::Object(table)) => {
            let mut args = Vec::with_capacity(table.len());
            for (key, entry) in table {
                let key = key.trim();
                if key.is_empty() || key.chars().any(char::is_control) {
                    return Err("`args` 的键不得为空或含控制字符".to_owned());
                }
                let text = match entry {
                    Json::String(text) => text.clone(),
                    Json::Number(number) => number.to_string(),
                    Json::Bool(flag) => flag.to_string(),
                    Json::Null => String::new(),
                    other => {
                        return Err(format!(
                            "`args.{key}` 只能是标量（当前是 {}：嵌套参数请拆成多个标量）",
                            json_type_name(other)
                        ))
                    }
                };
                if text.contains('|') {
                    return Err(format!(
                        "`args.{key}` 含 `|`（wire format 的段分隔符，会打碎回投载荷）"
                    ));
                }
                if text.contains(['\n', '\r', '\0']) {
                    return Err(format!(
                        "`args.{key}` 含换行或控制字符（wire format 的行边界）"
                    ));
                }
                if text.len() > MAX_TOOL_ARG_BYTES {
                    return Err(format!(
                        "`args.{key}` 超过 {MAX_TOOL_ARG_BYTES} 字节（防凭据走私）"
                    ));
                }
                args.push((key.to_owned(), text));
            }
            args
        }
        Some(other) => {
            return Err(format!(
                "`args` 必须是字符串键的对象（Lua 空表会序列化成 `[]`；当前是 {}）",
                json_type_name(other)
            ))
        }
    };
    Ok(ToolRequest {
        id,
        tool: tool.to_owned(),
        args,
    })
}

/// 一次工具执行的结局（status ∈ `ok` | `err`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolOutcome {
    pub(crate) status: &'static str,
    pub(crate) code: String,
    pub(crate) output: String,
}

impl ToolOutcome {
    pub(crate) fn error(code: &str, message: String, max_output_bytes: usize) -> Self {
        Self {
            status: "err",
            code: code.to_owned(),
            output: sanitize_tool_output(&message, max_output_bytes),
        }
    }
}

/// 一条在飞调用的**完整模板**（落 `pending_tools.json`；不落解析值）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingCall {
    pub(crate) call_id: String,
    pub(crate) tool: String,
    pub(crate) kind: ToolKind,
    pub(crate) program: Option<String>,
    pub(crate) args: Vec<String>,
    pub(crate) method: Option<String>,
    pub(crate) url: Option<String>,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body_template: Option<String>,
    pub(crate) placeholders: Vec<(String, String)>,
    pub(crate) timeout_ms: u64,
    pub(crate) max_output_bytes: usize,
    pub(crate) result_event: String,
    pub(crate) deadline_unix_ms: u128,
    pub(crate) started_at_unix_ms: u128,
    /// 场景 1 的等待窗口（worker 认领前的宽限基准）。
    pub(crate) inline_wait_ms: u64,
    pub(crate) root: usize,
}

impl PendingCall {
    pub(crate) fn from_spec(
        spec: &ToolSpec,
        request: &ToolRequest,
        deadline_unix_ms: u128,
        root: usize,
    ) -> Self {
        Self {
            call_id: request.id.clone(),
            tool: spec.id.clone(),
            kind: spec.kind,
            program: spec.program.clone(),
            args: spec.args.clone(),
            method: spec.method.clone(),
            url: spec.url.clone(),
            headers: spec.headers.clone(),
            body_template: spec.body_template.clone(),
            placeholders: request.args.clone(),
            timeout_ms: spec.timeout_ms,
            max_output_bytes: spec.max_output_bytes,
            result_event: spec.result_event.clone(),
            deadline_unix_ms,
            started_at_unix_ms: now_ms(),
            inline_wait_ms: spec.inline_wait_ms,
            root,
        }
    }

    pub(crate) fn to_json(&self) -> Json {
        let placeholders: serde_json::Map<String, Json> = self
            .placeholders
            .iter()
            .map(|(key, value)| (key.clone(), json!(value)))
            .collect();
        let headers: Vec<Json> = self
            .headers
            .iter()
            .map(|(key, value)| json!([key, value]))
            .collect();
        json!({
            "tool": self.tool,
            "kind": self.kind.as_str(),
            "program": self.program,
            "args": self.args,
            "method": self.method,
            "url": self.url,
            "headers": headers,
            "bodyTemplate": self.body_template,
            "placeholders": placeholders,
            "timeoutMs": self.timeout_ms,
            "maxOutputBytes": self.max_output_bytes,
            "resultEvent": self.result_event,
            "deadlineUnixMs": self.deadline_unix_ms as u64,
            "startedAtUnixMs": self.started_at_unix_ms as u64,
            "inlineWaitMs": self.inline_wait_ms,
            "root": self.root,
        })
    }

    pub(crate) fn from_json(call_id: &str, value: &Json) -> Result<Self, String> {
        let kind_text = value["kind"].as_str().unwrap_or_default();
        let kind = ToolKind::parse(kind_text)
            .ok_or_else(|| format!("{PENDING_TOOLS_FILE} 里 {call_id} 的 kind 非法"))?;
        let strings = |key: &str| -> Vec<String> {
            value[key]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Json::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        };
        let headers = value["headers"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|pair| {
                        let pair = pair.as_array()?;
                        Some((
                            pair.first()?.as_str()?.to_owned(),
                            pair.get(1)?.as_str()?.to_owned(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let placeholders = value["placeholders"]
            .as_object()
            .map(|table| {
                table
                    .iter()
                    .map(|(key, value)| (key.clone(), value.as_str().unwrap_or("").to_owned()))
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            call_id: call_id.to_owned(),
            tool: value["tool"].as_str().unwrap_or(call_id).to_owned(),
            kind,
            program: value["program"].as_str().map(str::to_owned),
            args: strings("args"),
            method: value["method"].as_str().map(str::to_owned),
            url: value["url"].as_str().map(str::to_owned),
            headers,
            body_template: value["bodyTemplate"].as_str().map(str::to_owned),
            placeholders,
            timeout_ms: value["timeoutMs"].as_u64().unwrap_or(120_000),
            max_output_bytes: value["maxOutputBytes"].as_u64().unwrap_or(65_536) as usize,
            result_event: value["resultEvent"].as_str().unwrap_or("").to_owned(),
            deadline_unix_ms: value["deadlineUnixMs"].as_u64().map_or(0, u128::from),
            started_at_unix_ms: value["startedAtUnixMs"].as_u64().map_or(0, u128::from),
            inline_wait_ms: value["inlineWaitMs"].as_u64().unwrap_or(0),
            root: value["root"].as_u64().unwrap_or(1) as usize,
        })
    }
}

/// 一次工具调用的结局记录（`tool_results/<call_id>.json`，两条路径共用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolResult {
    pub(crate) call_id: String,
    pub(crate) tool: String,
    pub(crate) status: String,
    pub(crate) code: String,
    pub(crate) output: String,
    pub(crate) payload: String,
    pub(crate) external_id: String,
}

impl ToolResult {
    pub(crate) fn from_outcome(
        call: &PendingCall,
        outcome: &ToolOutcome,
        external_id: &str,
    ) -> Self {
        Self {
            call_id: call.call_id.clone(),
            tool: call.tool.clone(),
            status: outcome.status.to_owned(),
            code: outcome.code.clone(),
            output: outcome.output.clone(),
            payload: wire_payload(call, outcome, external_id),
            external_id: external_id.to_owned(),
        }
    }

    pub(crate) fn to_json(&self) -> Json {
        json!({
            "call_id": self.call_id,
            "tool": self.tool,
            "status": self.status,
            "code": self.code,
            "output": self.output,
            "payload": self.payload,
            "external_id": self.external_id,
            "finished_at": now_secs(),
        })
    }

    pub(crate) fn from_json(value: &Json) -> Option<Self> {
        Some(Self {
            call_id: value["call_id"].as_str()?.to_owned(),
            tool: value["tool"].as_str()?.to_owned(),
            status: value["status"].as_str()?.to_owned(),
            code: value["code"].as_str().unwrap_or("").to_owned(),
            output: value["output"].as_str().unwrap_or("").to_owned(),
            payload: value["payload"].as_str()?.to_owned(),
            external_id: value["external_id"].as_str().unwrap_or("").to_owned(),
        })
    }
}

/// 两条路径共用的 wire 载荷（I5）。
pub(crate) fn wire_payload(call: &PendingCall, outcome: &ToolOutcome, external_id: &str) -> String {
    join_segments(vec![
        sanitize_field(&call.call_id),
        sanitize_field(&call.tool),
        outcome.status.to_owned(),
        sanitize_field(&outcome.code),
        outcome.output.clone(),
        external_id.to_owned(),
    ])
}
