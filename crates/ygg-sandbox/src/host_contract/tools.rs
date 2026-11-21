use super::{json_kind, read_json, CONTRACT_DOC};
use serde_json::Value as Json;
use std::path::Path;

// ============================================================================
// §工具两层白名单（`pack.json` 的 `tools` 段 + `data/config.json` 的 `allowedTools`）
//
// 第一层定**形状与能力**（kind / 程序名 / 参数模板 / 超时 / 输出上限 / 只读 /
// 结果事件），第二层定**本局放行清单**。两层都缺失 = 本 Mod 不请求任何外部工具
// （既有 Mod 零改动照常工作）。
// ============================================================================

/// `tools` 段的应有形状（错误文案里给全，抄改友好）。
const TOOLS_SHAPE: &str = r#"{
  "tools": {
    "<tool_id>": {
      "kind": "cli" | "http",
      "program": "<可执行文件名>",
      "args": ["<参数>", "{{占位}}", "$ENV{变量名}"],
      "method": "GET" | "POST" | "PUT" | "DELETE",
      "url": "https://<主机>/<路径>",
      "headers": {"Authorization": "Bearer $ENV{TOKEN}"},
      "bodyTemplate": "<请求体，支持 {{占位}}>",
      "timeoutMs": 120000,
      "inlineWaitMs": 5000,
      "maxOutputBytes": 65536,
      "readOnly": true,
      "resultEvent": "tool.result"
    }
  }
}"#;

/// `allowedTools` 的应有形状。
const ALLOWED_TOOLS_SHAPE: &str = r#"{ "allowedTools": ["<tool_id>", "<tool_id>"] }"#;

/// 工具 id / 放行条目的字节上限（也是 `tool_results/<call_id>.json` 的文件名上限）。
const TOOL_ID_MAX_BYTES: usize = 64;
/// 单次工具执行的超时上限（1 小时）。
const TOOL_TIMEOUT_MAX_MS: u64 = 3_600_000;
/// 单次工具输出的字节上限。
const TOOL_OUTPUT_MAX_BYTES: u64 = 65_536;
/// cli 类**不得**声明的 shell 基名（「不经 shell」是这一层的核心边界）。
const SHELL_BASENAMES: [&str; 12] = [
    "sh",
    "bash",
    "zsh",
    "dash",
    "fish",
    "ksh",
    "csh",
    "tcsh",
    "cmd",
    "cmd.exe",
    "powershell",
    "pwsh",
];

/// 工具类别。`cli` = 直接 spawn 一个可执行文件（**不经 shell**）；`http` = 经
/// `curl -sS -X <method>` 发起一次请求（宿主零 HTTP 依赖）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    /// 直接 spawn 可执行文件（无 shell 解释）。
    Cli,
    /// 经 `curl` 发起一次 HTTP 请求。
    Http,
}

impl ToolKind {
    /// 契约字面量（`pack.json` 与宿主 run 目录的 `pending_tools.json` 共用）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Http => "http",
        }
    }

    /// 从契约字面量解析（未知值 → `None`）。
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "cli" => Some(Self::Cli),
            "http" => Some(Self::Http),
            _ => None,
        }
    }
}

/// 单个工具声明（`pack.json` 的 `tools.<tool_id>`）。
///
/// 字段按 kind 分组：`program` / `args` 只对 `cli` 有效，`method` / `url` /
/// `headers` / `body_template` 只对 `http` 有效；跨 kind 声明一律在校验期拒绝
/// （半声明禁令，与 `code` 段同源）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSpec {
    /// 工具 id（= `tools` 段的键）。
    pub id: String,
    /// 工具类别。
    pub kind: ToolKind,
    /// `cli`：可执行文件名（裸名或相对路径；**不得**绝对路径 / 含 `..` / 是 shell）。
    /// 唯一例外是自指占位 [`SELF_PROGRAM`]（`@self` = 本宿主可执行文件）。
    pub program: Option<String>,
    /// `cli`：参数模板（可空数组；支持 `{{key}}` 与 `$ENV{VAR}`）。
    pub args: Vec<String>,
    /// `http`：HTTP 方法（`GET` / `POST` / `PUT` / `DELETE`）。
    pub method: Option<String>,
    /// `http`：URL（`http://` 或 `https://` 开头）。
    pub url: Option<String>,
    /// `http`：请求头（保 Mod 的书写序）。
    pub headers: Vec<(String, String)>,
    /// `http`：请求体模板（支持 `{{key}}`）。
    pub body_template: Option<String>,
    /// 单次执行超时（毫秒，1..=3_600_000）。
    pub timeout_ms: u64,
    /// 场景 1 的就地等待窗口（毫秒，0..=timeout_ms；0 = 恒走脱体路径）。
    pub inline_wait_ms: u64,
    /// 输出截断上限（字节，1..=65_536）。
    pub max_output_bytes: usize,
    /// 是否只读（`false` 时禁止脱体：`inline_wait_ms` 必须 > 0）。
    pub read_only: bool,
    /// 结果回投的事件名（宿主以普通事件派发，载荷是 wire format）。
    pub result_event: String,
}

impl ToolSpec {
    /// 单条工具声明解析（先一次列全缺失字段，再逐字段校验）。
    fn from_json(id: &str, spec: &Json) -> Result<Self, String> {
        if !spec.is_object() {
            return Err(tools_error(
                id,
                &format!("必须是对象（当前是 {}）", json_kind(spec)),
            ));
        }
        let kind = match spec.get("kind") {
            Some(Json::String(text)) => match ToolKind::parse(text) {
                Some(kind) => kind,
                None => return Err(tool_field_error(id, "kind", "必须是 `cli` 或 `http`")),
            },
            Some(other) => {
                return Err(tool_field_error(
                    id,
                    "kind",
                    &format!(
                        "必须是字符串 `cli` 或 `http`（当前是 {}）",
                        json_kind(other)
                    ),
                ))
            }
            None => {
                return Err(tool_field_error(
                    id,
                    "kind",
                    "缺失（必需，`cli` 或 `http`）",
                ))
            }
        };
        // ① 存在性 + 半声明：一次列全该条目的问题（不逐个撞墙）。
        let (required, forbidden): (&[&str], &[&str]) = match kind {
            ToolKind::Cli => (
                &[
                    "program",
                    "args",
                    "timeoutMs",
                    "inlineWaitMs",
                    "maxOutputBytes",
                    "readOnly",
                    "resultEvent",
                ],
                &["method", "url", "headers", "bodyTemplate"],
            ),
            ToolKind::Http => (
                &[
                    "method",
                    "url",
                    "timeoutMs",
                    "inlineWaitMs",
                    "maxOutputBytes",
                    "readOnly",
                    "resultEvent",
                ],
                &["program", "args"],
            ),
        };
        let missing: Vec<&str> = required
            .iter()
            .copied()
            .filter(|field| spec.get(*field).is_none())
            .collect();
        if !missing.is_empty() {
            let names = missing
                .iter()
                .map(|field| format!("`{field}`"))
                .collect::<Vec<_>>()
                .join(" / ");
            return Err(tools_error(
                id,
                &format!(
                    "缺少字段 {names}（kind = `{}` 时这 {} 个字段全部必填）",
                    kind.as_str(),
                    required.len()
                ),
            ));
        }
        let declared: Vec<&str> = forbidden
            .iter()
            .copied()
            .filter(|field| spec.get(*field).is_some())
            .collect();
        if !declared.is_empty() {
            let names = declared
                .iter()
                .map(|field| format!("`{field}`"))
                .collect::<Vec<_>>()
                .join(" / ");
            return Err(tools_error(
                id,
                &format!(
                    "声明了与 kind = `{}` 不相干的字段 {names}（半声明一律拒绝：cli 只认 program / args，http 只认 method / url / headers / bodyTemplate）",
                    kind.as_str()
                ),
            ));
        }
        // ② 逐字段校验（类型 / 范围 / 形状）。
        let timeout_ms = tool_positive_uint(spec, id, "timeoutMs", TOOL_TIMEOUT_MAX_MS)?;
        let inline_wait_ms = tool_uint(spec, id, "inlineWaitMs")?;
        if inline_wait_ms > timeout_ms {
            return Err(tool_field_error(
                id,
                "inlineWaitMs",
                &format!(
                    "不得大于 timeoutMs（{inline_wait_ms} > {timeout_ms}）——场景 1 的等待必须在工具超时之内"
                ),
            ));
        }
        let max_output_bytes =
            tool_positive_uint(spec, id, "maxOutputBytes", TOOL_OUTPUT_MAX_BYTES)?;
        let read_only = tool_bool(spec, id, "readOnly")?;
        if !read_only && inline_wait_ms == 0 {
            return Err(tool_field_error(
                id,
                "inlineWaitMs",
                "非只读工具（`readOnly: false`）不得脱体：inlineWaitMs 必须 > 0（脱体只允许只读工具，防与收口 / 其他进程写冲突）",
            ));
        }
        let result_event = tool_text(spec, id, "resultEvent")?;
        if result_event.chars().any(char::is_control) {
            return Err(tool_field_error(id, "resultEvent", "不得含控制字符"));
        }
        let (program, args) = match kind {
            ToolKind::Cli => (Some(tool_program(spec, id)?), tool_args(spec, id)?),
            ToolKind::Http => (None, Vec::new()),
        };
        let (method, url, headers, body_template) = match kind {
            ToolKind::Http => (
                Some(tool_method(spec, id)?),
                Some(tool_url(spec, id)?),
                tool_headers(spec, id)?,
                tool_body_template(spec, id)?,
            ),
            ToolKind::Cli => (None, None, Vec::new(), None),
        };
        Ok(Self {
            id: id.to_owned(),
            kind,
            program,
            args,
            method,
            url,
            headers,
            body_template,
            timeout_ms,
            inline_wait_ms,
            max_output_bytes: max_output_bytes as usize,
            read_only,
            result_event,
        })
    }
}

/// Layer 1：`pack.json` 的 `tools` 段（空 registry = 本 Mod 不请求外部工具）。
///
/// 用 `Vec<(String, ToolSpec)>` 而不是 `BTreeMap`：与 [`InboxRoutes`] 同风格，
/// 报错与回执的枚举顺序跟着 Mod 的书写序走（诊断文案可复现）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolRegistry {
    tools: Vec<(String, ToolSpec)>,
}

impl ToolRegistry {
    /// 从已解析的 `pack.json` 取 `tools` 段（缺失 / 非对象 / 任一条目非法 → 错误）。
    pub fn from_pack(pack: &Json) -> Result<Self, String> {
        let Some(section) = pack.get("tools") else {
            return Ok(Self::default());
        };
        let Some(table) = section.as_object() else {
            return Err(format!(
                "pack.json 的 `tools` 段必须是对象（当前是 {}）。该段的完整形状：\n{TOOLS_SHAPE}\n{CONTRACT_DOC}",
                json_kind(section)
            ));
        };
        let mut tools = Vec::with_capacity(table.len());
        for (id, spec) in table {
            validate_tool_id(id)?;
            tools.push((id.clone(), ToolSpec::from_json(id, spec)?));
        }
        Ok(Self { tools })
    }

    /// 从 Mod 目录现读 `pack.json`（任何问题都以可读错误返回，不兜底）。
    pub fn load(dir: &Path) -> Result<Self, String> {
        Self::from_pack(&read_json(&dir.join("pack.json"))?)
    }

    /// 按 id 找声明（未声明 → `None`）。
    pub fn get(&self, id: &str) -> Option<&ToolSpec> {
        self.tools
            .iter()
            .find(|(declared, _)| declared == id)
            .map(|(_, spec)| spec)
    }

    /// 逐条声明（书写序）。
    pub fn iter(&self) -> impl Iterator<Item = &ToolSpec> {
        self.tools.iter().map(|(_, spec)| spec)
    }

    /// 已声明的工具数量（`0` = 本 Mod 不请求外部工具）。
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// 是否一条都没声明。
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// 已声明但**未被放行**的 id（装载期交叉校验用；返回全部，不是首个）。
    pub fn denied(&self, access: &ToolAccess) -> Vec<String> {
        self.tools
            .iter()
            .filter(|(id, _)| !access.allows(id))
            .map(|(id, _)| id.clone())
            .collect()
    }
}

/// Layer 2：`data/config.json` 的 `allowedTools`（空 = 不允许任何工具）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolAccess {
    allowed: Vec<String>,
}

impl ToolAccess {
    /// 从已解析的 `config.json` 取 `allowedTools` 段（缺失 → 默认；非数组 / 坏条目 → 错误）。
    pub fn from_config(config: &Json, source: &Path) -> Result<Self, String> {
        let Some(value) = config.get("allowedTools") else {
            return Ok(Self::default());
        };
        let Some(items) = value.as_array() else {
            return Err(format!(
                "{} 的 `allowedTools` 必须是字符串数组（当前是 {}）。放行清单形状：{ALLOWED_TOOLS_SHAPE}；工具声明形状：\n{TOOLS_SHAPE}\n{CONTRACT_DOC}",
                source.display(),
                json_kind(value)
            ));
        };
        let mut allowed = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            let spec = "非空字符串（工具 id）";
            let Some(text) = item.as_str() else {
                return Err(allowed_tools_error(
                    source,
                    index,
                    spec,
                    &format!("当前是 {}", json_kind(item)),
                ));
            };
            let text = text.trim();
            if text.is_empty() {
                return Err(allowed_tools_error(source, index, spec, "空串"));
            }
            if text.len() > TOOL_ID_MAX_BYTES {
                return Err(allowed_tools_error(
                    source,
                    index,
                    &format!("不超过 {TOOL_ID_MAX_BYTES} 字节的字符串"),
                    &format!("{} 字节", text.len()),
                ));
            }
            allowed.push(text.to_owned());
        }
        Ok(Self { allowed })
    }

    /// 从配置文件路径现读（任何问题都以可读错误返回）。
    pub fn load(path: &Path) -> Result<Self, String> {
        Self::from_config(&read_json(path)?, path)
    }

    /// 该 id 是否被本局放行。
    pub fn allows(&self, id: &str) -> bool {
        self.allowed.iter().any(|item| item == id)
    }

    /// 是否一条都没放行。
    pub fn is_empty(&self) -> bool {
        self.allowed.is_empty()
    }

    /// 放行清单（声明序，可能含重复项）。
    pub fn allowed(&self) -> &[String] {
        &self.allowed
    }
}

/// 装载期硬校验：`tools` 里声明的每个 id 都必须在 `allowedTools` 里（I6）。
///
/// 一次列全未放行的 id——不让使用者逐个撞墙。**诚实声明**：本函数是
/// 防漂移 / 防半声明的门禁，不是安全边界（两层声明同在 Mod 目录，同一作者）。
pub fn require_tool_allowlist(
    registry: &ToolRegistry,
    access: &ToolAccess,
    config_source: &Path,
) -> Result<(), String> {
    let denied = registry.denied(access);
    if denied.is_empty() {
        return Ok(());
    }
    let names = denied
        .iter()
        .map(|id| format!("`{id}`"))
        .collect::<Vec<_>>()
        .join(" / ");
    Err(format!(
        "{} 的 `allowedTools` 未放行以下工具：{names}（pack.json 的 `tools` 声明了但本局未放行）。\
         任一声明未放行即拒绝启动（两层白名单是硬门禁）。放行清单形状：{ALLOWED_TOOLS_SHAPE}；\
         工具声明形状：\n{TOOLS_SHAPE}\n{CONTRACT_DOC}",
        config_source.display()
    ))
}

/// 条目级错误文案（点名字段 + 给全形状 + 文档定位）。
fn tool_field_error(id: &str, field: &str, detail: &str) -> String {
    format!(
        "pack.json 的 `tools.{id}` 的字段 `{field}` {detail}。该段的完整形状：\n{TOOLS_SHAPE}\n{CONTRACT_DOC}"
    )
}

/// 条目整体错误文案（不点具体字段）。
fn tools_error(id: &str, detail: &str) -> String {
    format!("pack.json 的 `tools.{id}` {detail}。该段的完整形状：\n{TOOLS_SHAPE}\n{CONTRACT_DOC}")
}

/// 放行清单条目错误文案（点名第 n 项）。
fn allowed_tools_error(source: &Path, index: usize, spec: &str, actual: &str) -> String {
    format!(
        "{} 的 `allowedTools` 第 {} 项必须是{spec}（{actual}）。放行清单形状：{ALLOWED_TOOLS_SHAPE}；工具声明形状：\n{TOOLS_SHAPE}\n{CONTRACT_DOC}",
        source.display(),
        index + 1
    )
}

/// 工具 id 白名单：1..=64 字节、`[a-z0-9._-]`（也是 call_id / 文件名的字符集）。
fn validate_tool_id(id: &str) -> Result<(), String> {
    let legal = !id.is_empty()
        && id.len() <= TOOL_ID_MAX_BYTES
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'));
    if legal {
        return Ok(());
    }
    Err(format!(
        "pack.json 的 `tools` 段的键 `{id}` 必须是 1..={TOOL_ID_MAX_BYTES} 字节的 `[a-z0-9._-]`（工具 id 同时是回执字段与结果文件名的片段）。该段的完整形状：\n{TOOLS_SHAPE}\n{CONTRACT_DOC}"
    ))
}

/// 必填非空字符串字段。
fn tool_text(spec: &Json, id: &str, field: &str) -> Result<String, String> {
    match spec.get(field) {
        Some(Json::String(text)) if !text.trim().is_empty() => Ok(text.trim().to_owned()),
        Some(Json::String(_)) => Err(tool_field_error(id, field, "不得为空串")),
        Some(other) => Err(tool_field_error(
            id,
            field,
            &format!("必须是字符串（当前是 {}）", json_kind(other)),
        )),
        None => Err(tool_field_error(id, field, "缺失")),
    }
}

/// 必填布尔字段。
fn tool_bool(spec: &Json, id: &str, field: &str) -> Result<bool, String> {
    match spec.get(field) {
        Some(Json::Bool(value)) => Ok(*value),
        Some(other) => Err(tool_field_error(
            id,
            field,
            &format!("必须是布尔（当前是 {}）", json_kind(other)),
        )),
        None => Err(tool_field_error(id, field, "缺失")),
    }
}

/// 必填非负整数字段（`0` 合法）。
fn tool_uint(spec: &Json, id: &str, field: &str) -> Result<u64, String> {
    match spec.get(field) {
        Some(value) => value.as_u64().ok_or_else(|| {
            tool_field_error(
                id,
                field,
                &format!("必须是非负整数（当前是 {}）", json_kind(value)),
            )
        }),
        None => Err(tool_field_error(id, field, "缺失")),
    }
}

/// 必填正整数且不超过 `max`。
fn tool_positive_uint(spec: &Json, id: &str, field: &str, max: u64) -> Result<u64, String> {
    let value = tool_uint(spec, id, field)?;
    if value == 0 {
        return Err(tool_field_error(id, field, "必须是正整数（当前是 0）"));
    }
    if value > max {
        return Err(tool_field_error(
            id,
            field,
            &format!("不得超过 {max}（当前是 {value}）"),
        ));
    }
    Ok(value)
}

/// 自指占位：`@self` = 本宿主可执行文件（`std::env::current_exe()`）。
///
/// 用它是为了让 `pack.json` 的 `tools` 段可移植——内置子命令（`--builtin-tool`）
/// 不必再让每个部署方自己写 shell wrapper，也不必把宿主路径写死进静态配置。
///
/// `@` 只在这一处合法：其余位置由 [`tool_program`] 的字符集规则拒绍。
pub const SELF_PROGRAM: &str = "@self";

/// `cli.program`：裸名或相对路径，无 shell、无绝对路径、无 `..` 路径段。
fn tool_program(spec: &Json, id: &str) -> Result<String, String> {
    let program = tool_text(spec, id, "program")?;
    // **自指占位先于一切形状判定**：`@` 不在下面的字符集里（`[A-Za-z0-9._/-]`），
    // 所以 `@self` 只在这一处合法，其余位置（`@SELF` / `@self/x` / `x@self` /
    // `@self..`）仍然被后面的规则拒掉。
    if program == SELF_PROGRAM {
        return Ok(program);
    }
    if !program
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'))
    {
        return Err(tool_field_error(
            id,
            "program",
            "只允许 `[A-Za-z0-9._/-]`（不得含空白、引号或 shell 元字符）",
        ));
    }
    if program.starts_with('/') {
        return Err(tool_field_error(
            id,
            "program",
            "不得以 `/` 开头（只接受裸名或相对路径）",
        ));
    }
    if program.split('/').any(|segment| segment == "..") {
        return Err(tool_field_error(
            id,
            "program",
            "不得含 `..` 路径段（目录穿越）",
        ));
    }
    let base = program.rsplit('/').next().unwrap_or(program.as_str());
    if SHELL_BASENAMES.contains(&base) {
        return Err(tool_field_error(
            id,
            "program",
            &format!("不得是 shell（`{base}`）——cli 类工具只 spawn 可执行文件，不经 shell 解释"),
        ));
    }
    Ok(program)
}

/// `cli.args`：必填数组（可空），每项 string 且 wire 安全 + 模板语法合法。
fn tool_args(spec: &Json, id: &str) -> Result<Vec<String>, String> {
    let Some(value) = spec.get("args") else {
        return Err(tool_field_error(
            id,
            "args",
            "缺失（必需，字符串数组；可为空数组）",
        ));
    };
    let Some(items) = value.as_array() else {
        return Err(tool_field_error(
            id,
            "args",
            &format!("必须是字符串数组（当前是 {}）", json_kind(value)),
        ));
    };
    let mut args = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let label = format!("args[{index}]");
        let Some(text) = item.as_str() else {
            return Err(tool_field_error(
                id,
                &label,
                &format!("必须是字符串（当前是 {}）", json_kind(item)),
            ));
        };
        validate_wire_safe(id, &label, text)?;
        validate_placeholders(id, &label, text)?;
        args.push(text.to_owned());
    }
    Ok(args)
}

/// `http.method`：`GET` / `POST` / `PUT` / `DELETE`。
fn tool_method(spec: &Json, id: &str) -> Result<String, String> {
    let method = tool_text(spec, id, "method")?;
    match method.as_str() {
        "GET" | "POST" | "PUT" | "DELETE" => Ok(method),
        _ => Err(tool_field_error(
            id,
            "method",
            "必须是 `GET` / `POST` / `PUT` / `DELETE` 之一（大写）",
        )),
    }
}

/// `http.url`：`http://` 或 `https://` 开头，无控制字符与空白。
fn tool_url(spec: &Json, id: &str) -> Result<String, String> {
    let url = tool_text(spec, id, "url")?;
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(tool_field_error(
            id,
            "url",
            "必须以 `http://` 或 `https://` 开头",
        ));
    }
    if url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(tool_field_error(id, "url", "不得含控制字符或空白"));
    }
    validate_placeholders(id, "url", &url)?;
    Ok(url)
}

/// `http.headers`：可选；键非空无控制字符，值无 `\n` `\r` `\0`。
fn tool_headers(spec: &Json, id: &str) -> Result<Vec<(String, String)>, String> {
    let Some(value) = spec.get("headers") else {
        return Ok(Vec::new());
    };
    let Some(table) = value.as_object() else {
        return Err(tool_field_error(
            id,
            "headers",
            &format!("必须是对象（当前是 {}）", json_kind(value)),
        ));
    };
    let mut headers = Vec::with_capacity(table.len());
    for (key, entry) in table {
        if key.trim().is_empty() || key.chars().any(char::is_control) {
            return Err(tool_field_error(
                id,
                "headers",
                &format!("的键 `{key}` 不得为空或含控制字符"),
            ));
        }
        let Some(text) = entry.as_str() else {
            return Err(tool_field_error(
                id,
                "headers",
                &format!(
                    "的键 `{key}` 的值必须是字符串（当前是 {}）",
                    json_kind(entry)
                ),
            ));
        };
        if text.contains(['\n', '\r', '\0']) {
            return Err(tool_field_error(
                id,
                "headers",
                &format!("的键 `{key}` 的值不得含换行或 NUL"),
            ));
        }
        validate_placeholders(id, "headers", text)?;
        headers.push((key.clone(), text.to_owned()));
    }
    Ok(headers)
}

/// `http.bodyTemplate`：可选字符串，不得含 NUL；模板语法必须合法。
fn tool_body_template(spec: &Json, id: &str) -> Result<Option<String>, String> {
    let Some(value) = spec.get("bodyTemplate") else {
        return Ok(None);
    };
    let Some(text) = value.as_str() else {
        return Err(tool_field_error(
            id,
            "bodyTemplate",
            &format!("必须是字符串（当前是 {}）", json_kind(value)),
        ));
    };
    if text.contains('\0') {
        return Err(tool_field_error(id, "bodyTemplate", "不得含 NUL"));
    }
    validate_placeholders(id, "bodyTemplate", text)?;
    Ok(Some(text.to_owned()))
}

/// wire format 安全：不得含 `|` 与换行 / NUL（它们是载荷分隔符与行边界）。
fn validate_wire_safe(id: &str, label: &str, text: &str) -> Result<(), String> {
    if text.contains('|') {
        return Err(tool_field_error(
            id,
            label,
            "不得含 `|`（wire format 的段分隔符）",
        ));
    }
    if text.contains(['\n', '\r', '\0']) {
        return Err(tool_field_error(
            id,
            label,
            "不得含换行或 NUL（wire format 的行边界）",
        ));
    }
    Ok(())
}

/// 模板语法：`{{key}}` 成对闭合、`$ENV{VAR}` 的 `VAR` 是合法标识符。
fn validate_placeholders(id: &str, label: &str, text: &str) -> Result<(), String> {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"{{") {
            let rest = &text[index + 2..];
            let Some(end) = rest.find("}}") else {
                return Err(tool_field_error(
                    id,
                    label,
                    "的 `{{` 未闭合（占位符必须写成 `{{key}}`）",
                ));
            };
            let key = rest[..end].trim();
            if key.is_empty() || key.contains(['{', '}']) {
                return Err(tool_field_error(
                    id,
                    label,
                    "的占位符必须是 `{{key}}`（key 非空、不含花括号）",
                ));
            }
            index += 2 + end + 2;
            continue;
        }
        if bytes[index..].starts_with(b"$ENV{") {
            let rest = &text[index + 5..];
            let Some(end) = rest.find('}') else {
                return Err(tool_field_error(
                    id,
                    label,
                    "的 `$ENV{` 未闭合（必须写成 `$ENV{VAR}`）",
                ));
            };
            let name = &rest[..end];
            let legal = !name.is_empty()
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !name.starts_with(|c: char| c.is_ascii_digit());
            if !legal {
                return Err(tool_field_error(
                    id,
                    label,
                    "的 `$ENV{VAR}` 里 VAR 必须是 `[A-Za-z_][A-Za-z0-9_]*`",
                ));
            }
            index += 5 + end + 1;
            continue;
        }
        if bytes[index..].starts_with(b"}}") {
            return Err(tool_field_error(
                id,
                label,
                "有未配对的 `}}`（占位符必须写成 `{{key}}`）",
            ));
        }
        index += text[index..].chars().next().map_or(1, char::len_utf8);
    }
    Ok(())
}

#[cfg(test)]
mod tool_contract_tests;
