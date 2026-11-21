use super::{json_kind, read_json, CONTRACT_DOC};
use serde_json::Value as Json;
use std::path::Path;

/// `agent` 段的应有形状（错误文案里给全，便于直接抄改）。
const AGENT_SHAPE: &str = r#"{
  "agent": {
    "entryEvent": "<新局入口事件名>",
    "choiceEvent": "<玩家选择事件名>",
    "summaryEvent": "<Mod 自报摘要的只读动作名>",
    "endingPrefix": "<结局叙事行前缀>",
    "metaKey": "<回执信封 meta 里摘要的键名>",
    "rejectionPrefixes": ["<拒绝叙事行前缀>", "<另一条拒绝前缀>"],
    "replayPrefix": "<幂等重放的叙事行前缀>",
    "defaultPayload": "<入口事件首次派发的载荷（JSON 标量文本）>"
  }
}"#;

/// `agent` 段的八个字段名（错误文案逐字段点名 + 校验序）。
const AGENT_FIELDS: [&str; 8] = [
    "entryEvent",
    "choiceEvent",
    "summaryEvent",
    "endingPrefix",
    "metaKey",
    "rejectionPrefixes",
    "replayPrefix",
    "defaultPayload",
];

// ============================================================================
// §事件名 + 叙事约定（pack.json 的 `agent` 段，八个字段全部必填）
// ============================================================================

/// 事件名与叙事约定：入口 / 选择 / 摘要动作 / 结局前缀 / 摘要键名 /
/// 拒绝前缀集合 / 重放前缀 / 入口载荷。
///
/// 宿主所有事件名、叙事判据与摘要读点都从这里取，**代码里不出现这些字面量**
/// （换 Mod 只改 Mod 的 `pack.json`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentContract {
    /// 新局自动派发的入口事件名。
    pub entry_event: String,
    /// 玩家选择事件名（内核保留路由）。
    pub choice_event: String,
    /// Mod 自报摘要的只读动作名。
    pub summary_event: String,
    /// 结局叙事行前缀（收口判据）。
    pub ending_prefix: String,
    /// 摘要结构化字段在回执信封 `meta` 里的键名（宿主只读 `meta.<metaKey>`）。
    pub meta_key: String,
    /// Mod 侧「动作未被接受」的叙事行前缀集合（≥1 条；宿主据此把被拒的派发
    /// 算作**未成功**，而不是当成已生效）。
    pub rejection_prefixes: Vec<String>,
    /// Mod 侧「幂等重放」的叙事行前缀（同幂等键重放：算成功消费，但没新增）。
    pub replay_prefix: String,
    /// 入口事件首次派发的载荷（JSON 标量文本）。允许空串，但必须显式声明。
    pub default_payload: String,
}

impl AgentContract {
    /// 从已解析的 `pack.json` 取 `agent` 段（段缺失 / 字段缺失 / 类型不符 → 错误）。
    pub fn from_pack(pack: &Json) -> Result<Self, String> {
        let Some(section) = pack.get("agent") else {
            return Err(format!(
                "pack.json 缺少 `agent` 段（host 契约必需）。该段的完整形状：\n{AGENT_SHAPE}\n{CONTRACT_DOC}"
            ));
        };
        if !section.is_object() {
            return Err(format!(
                "pack.json 的 `agent` 段必须是对象（当前是 {}）。该段的完整形状：\n{AGENT_SHAPE}\n{CONTRACT_DOC}",
                json_kind(section)
            ));
        }
        // ① 存在性：按 AGENT_FIELDS 顺序一次性列出所有缺失字段（不逐个撞墙）。
        let missing: Vec<&str> = AGENT_FIELDS
            .iter()
            .copied()
            .filter(|field| section.get(*field).is_none())
            .collect();
        if !missing.is_empty() {
            let names = missing
                .iter()
                .map(|field| format!("`{field}`"))
                .collect::<Vec<_>>()
                .join(" / ");
            return Err(format!(
                "pack.json 的 `agent` 段缺少字段 {names}（八个字段全部必填）。该段的完整形状：\n{AGENT_SHAPE}\n{CONTRACT_DOC}"
            ));
        }
        // ② 逐字段类型 / 空值校验（错一个就报一个，文案点名该字段）。
        Ok(Self {
            entry_event: agent_text(section, "entryEvent", "非空字符串")?,
            choice_event: agent_text(section, "choiceEvent", "非空字符串")?,
            summary_event: agent_text(section, "summaryEvent", "非空字符串")?,
            ending_prefix: agent_text(section, "endingPrefix", "非空字符串")?,
            meta_key: agent_text(section, "metaKey", "非空字符串")?,
            rejection_prefixes: agent_prefixes(section, "rejectionPrefixes")?,
            replay_prefix: agent_text(section, "replayPrefix", "非空字符串")?,
            // 载荷允许空串（「空载荷」是合法意图），但必须是显式声明的字符串。
            default_payload: agent_raw_text(section, "defaultPayload")?,
        })
    }

    /// 从 Mod 目录现读 `pack.json`（任何问题都以可读错误返回，不兜底）。
    pub fn load(dir: &Path) -> Result<Self, String> {
        Self::from_pack(&read_json(&dir.join("pack.json"))?)
    }

    /// 一条回执叙事行是否 Mod 侧「拒绝」（= 本步未生效）。
    pub fn is_rejection(&self, line: &str) -> bool {
        self.rejection_prefixes
            .iter()
            .any(|prefix| line.starts_with(prefix.as_str()))
    }

    /// 一条回执叙事行是否 Mod 侧「幂等重放」。
    pub fn is_replay(&self, line: &str) -> bool {
        line.starts_with(self.replay_prefix.as_str())
    }
}

/// `agent` 段「必填非空字符串」字段（缺失已在存在性检查里拦下，这里管类型与空值）。
fn agent_text(section: &Json, field: &str, spec: &str) -> Result<String, String> {
    match section.get(field) {
        Some(value) => match value.as_str() {
            Some(text) if !text.trim().is_empty() => Ok(text.trim().to_owned()),
            Some(_) => Err(agent_field_error(field, spec, "空串")),
            None => Err(agent_field_error(field, spec, json_kind(value))),
        },
        None => Err(agent_field_error(field, spec, "缺失")),
    }
}

/// `agent` 段「必填字符串」字段（允许空串：内容原样保留，不 trim）。
fn agent_raw_text(section: &Json, field: &str) -> Result<String, String> {
    match section.get(field) {
        Some(value) => match value.as_str() {
            Some(text) => Ok(text.to_owned()),
            None => Err(agent_field_error(
                field,
                "字符串（可为空串）",
                json_kind(value),
            )),
        },
        None => Err(agent_field_error(field, "字符串（可为空串）", "缺失")),
    }
}

/// `agent` 段「非空字符串数组，≥1 条」字段。
fn agent_prefixes(section: &Json, field: &str) -> Result<Vec<String>, String> {
    let Some(value) = section.get(field) else {
        return Err(agent_field_error(field, "非空字符串数组（≥1 条）", "缺失"));
    };
    let spec = "非空字符串数组（≥1 条；元素不得为空）";
    let Some(items) = value.as_array() else {
        return Err(agent_field_error(field, spec, json_kind(value)));
    };
    let mut prefixes = Vec::with_capacity(items.len());
    for item in items {
        match item.as_str().map(str::trim) {
            Some(text) if !text.is_empty() => prefixes.push(text.to_owned()),
            _ => return Err(agent_field_error(field, spec, "含空串或非字符串元素")),
        }
    }
    if prefixes.is_empty() {
        return Err(agent_field_error(field, spec, "空数组"));
    }
    Ok(prefixes)
}

/// `agent` 段字段错误的统一文案（点名字段 + 应有的类型 + 全形状 + 文档定位）。
fn agent_field_error(field: &str, spec: &str, actual: &str) -> String {
    format!(
        "pack.json 的 `agent` 段字段 `{field}` 必须是{spec}（当前是{actual}）。该段的完整形状：\n{AGENT_SHAPE}\n{CONTRACT_DOC}"
    )
}
