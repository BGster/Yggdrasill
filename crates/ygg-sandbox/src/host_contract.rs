//! host ↔ Mod 的**显式契约**（通用 Mod 工具的零硬编码面）。
//!
//! 四样以前靠宿主字面量硬编码的东西——事件名与叙事约定、inbox 记录字段与
//! kind → 动作、结算判定项、自动推进阈值——加载期从 Mod 自己的声明里读：
//!
//! | 声明位置 | 段 | 消费方 |
//! | --- | --- | --- |
//! | `pack.json` | `agent`（**必填**，八个字段） | `examples/agent.rs`、`src/playtest.rs` |
//! | `pack.json` | `inboxRoutes`（**必填**，三个保留键 + 若干 kind） | `examples/agent.rs` |
//! | `data/config.json` | `settlement`（**必填**） | `examples/agent.rs --settle` |
//! | `data/config.json` | `drive`（**必填**） | `examples/agent.rs` |
//!
//! **无默认值纪律**：所有段都是必填，缺失 / 字段缺失 / 类型不符 / 非正数一律
//! 返回可读错误（带缺失字段名 + 应有形状 + 文档定位），由调用方 fail-closed
//! （报错退出或 `ok=false` 报告）——宿主**不**提供任何回退默认值。旧 Mod 因此
//! 必须在升级时补齐这四段（见 `docs/mod-authoring.md` 与
//! `.ygg/mods/agent-seed/README.md` 的「host 契约」章节）。
//!
//! 绑定特定 Mod 的应用（`examples/adventure.rs`、`clients/godot`、
//! `packages/client-web`…）不属本模块的消费方：它们各自服务固定 Mod，文件头
//! 注明绑定对象即可（归属边界见 `guide.show harness` 的 `literal-ownership`）。

use std::fs;
use std::path::Path;

use serde_json::Value as Json;

/// 契约文档定位（所有契约错误文案的统一尾巴）。
pub const CONTRACT_DOC: &str = "见 .ygg/mods/agent-seed/README.md 的「host 契约」章节";

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

/// `inboxRoutes` 段的应有形状（三个保留键 + 逐 kind 路由）。
const INBOX_ROUTES_SHAPE: &str = r#"{
  "inboxRoutes": {
    "externalIdField": "<记录里幂等键的字段名>",
    "payloadField": "<记录里预成形载荷的字段名>",
    "extMarker": "<幂等键注入载荷时的后缀标记>",
    "<kind>": {
      "action": "<内核动作名>",
      "fields": ["<段1>", "<段2>", "<幂等键字段名>"],
      "write": { "flag": "<宿主 CLI 旗标名>", "shape": "named|payload" }
    }
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

/// `inboxRoutes` 段的三个保留键：与 `<kind>` **同层**，任何 kind 都不得占用这些名字。
const INBOX_RESERVED_KEYS: [&str; 3] = ["externalIdField", "payloadField", "extMarker"];

/// 读一个 JSON 文件（缺失 / 非法 → 可读错误，不做任何兜底）。
pub fn read_json(path: &Path) -> Result<Json, String> {
    let text =
        fs::read_to_string(path).map_err(|err| format!("读取 {} 失败：{err}", path.display()))?;
    serde_json::from_str(&text).map_err(|err| format!("{} 不是合法 JSON：{err}", path.display()))
}

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

// ============================================================================
// §inbox 路由表（pack.json 的 `inboxRoutes` 段：kind → 动作 + 段序 + 写命令绑定）
// ============================================================================

/// 宿主写一行 inbox 记录时的记录形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteShape {
    /// 命名段形态：`--<flag> '<第一段>|<第二段>'`，分别落在路由声明的
    /// 前两个非幂等键段（典型就是标题 / 描述）。
    Named,
    /// 预成形 wire 形态：整串载荷进 `payloadField` 声明的字段，宿主不解析、不重排。
    Payload,
}

/// 路由对宿主 CLI 写命令的绑定（`--<flag> '值'` 能写这种记录）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxWriteBinding {
    /// 宿主 CLI 旗标名（不带 `--`）：`requirement` / `proposal` / …。
    pub flag: String,
    /// 记录形态（决定写命名段还是写预成形载荷）。
    pub shape: WriteShape,
}

/// 一条 inbox 路由：`kind` → 内核动作 + 段序 + 可选写命令绑定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxRoute {
    /// 派发的内核动作名。
    pub action: String,
    /// wire format 的段序：按此顺序从 inbox 记录取值，用 `|` 连接。
    ///
    /// 等于路由表 `externalIdField` 声明值的段取记录的幂等键；其余段取
    /// 同名字段，缺失即空串占位（保持位置对齐）。记录里只有预成形载荷时
    /// 整个载荷原样透传。
    pub fields: Vec<String>,
    /// 宿主 CLI 写命令绑定；`None` = 该 kind 只能被外部写入（宿主不提供旗标）。
    pub write: Option<InboxWriteBinding>,
}

impl InboxRoute {
    /// 该路由是否把幂等键作为**段**声明（否则按加后缀的方式注入）。
    pub fn declares_external(&self, external_field: &str) -> bool {
        self.fields.iter().any(|field| field == external_field)
    }

    /// 除幂等键以外的具名段（段序即写入时的字段顺序）。
    pub fn named_fields<'a>(&'a self, external_field: &'a str) -> impl Iterator<Item = &'a String> {
        self.fields
            .iter()
            .filter(move |field| field.as_str() != external_field)
    }
}

/// inbox 路由表（`kind` → 路由）+ 三个保留键（幂等键字段名 / 预成形载荷字段名 /
/// 幂等后缀标记）。
///
/// 宿主不枚举 kind：读表 → 按 `fields` 从记录取值 → `|` 连接 → 派发 `action`。
/// 表里没有的 kind 一律计入坏行（前移 cursor，不阻塞队列）。三个保留键必填，
/// 但**允许一个 kind 都不声明**（= 本 Mod 不支持异步 inbox，此时宿主写命令明确
/// 拒绝）。保留键与 kind 同层，所以 kind 名不得与它们相同。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxRoutes {
    /// inbox 记录里幂等键的字段名。
    external_id_field: String,
    /// inbox 记录里预成形载荷的字段名。
    payload_field: String,
    /// 幂等键拼接到载荷末尾时的后缀标记。
    ext_marker: String,
    routes: Vec<(String, InboxRoute)>,
}

impl InboxRoutes {
    /// 从已解析的 `pack.json` 取 `inboxRoutes` 段（缺段 / 缺保留键 / 单条非法 → 错误）。
    pub fn from_pack(pack: &Json) -> Result<Self, String> {
        let Some(section) = pack.get("inboxRoutes") else {
            return Err(format!(
                "pack.json 缺少 `inboxRoutes` 段（host 契约必需；本 Mod 不接受异步 inbox 时声明三个保留键 + 不写任何 kind）。该段的完整形状：\n{INBOX_ROUTES_SHAPE}\n{CONTRACT_DOC}"
            ));
        };
        let Some(table) = section.as_object() else {
            return Err(format!(
                "pack.json 的 `inboxRoutes` 段必须是对象（当前是 {}）。该段的完整形状：\n{INBOX_ROUTES_SHAPE}\n{CONTRACT_DOC}",
                json_kind(section)
            ));
        };
        let external_id_field = reserved_text(section, "externalIdField", "幂等键字段名")?;
        let payload_field = reserved_text(section, "payloadField", "预成形载荷字段名")?;
        let ext_marker = reserved_text(section, "extMarker", "幂等后缀标记")?;
        let mut routes = Vec::with_capacity(table.len());
        for (kind, spec) in table {
            if INBOX_RESERVED_KEYS.contains(&kind.as_str()) {
                continue; // 保留键已在上面消费，不是 kind
            }
            let route = Self::parse_route(kind, spec, &external_id_field)?;
            routes.push((kind.clone(), route));
        }
        Ok(Self {
            external_id_field,
            payload_field,
            ext_marker,
            routes,
        })
    }

    /// 单条路由解析（`action` 非空 + `fields` 非空字符串数组 + 可选 `write`）。
    fn parse_route(kind: &str, spec: &Json, external_field: &str) -> Result<InboxRoute, String> {
        let action = spec
            .get("action")
            .and_then(Json::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .ok_or_else(|| {
                format!(
                    "pack.json 的 `inboxRoutes.{kind}` 缺少字段 `action`（必需，非空字符串 = 内核动作名）。完整形状：\n{INBOX_ROUTES_SHAPE}\n{CONTRACT_DOC}"
                )
            })?;
        let raw_fields = spec.get("fields").and_then(Json::as_array).ok_or_else(|| {
            format!(
                "pack.json 的 `inboxRoutes.{kind}` 缺少字段 `fields`（必需，非空字符串数组 = wire format 段序）。完整形状：\n{INBOX_ROUTES_SHAPE}\n{CONTRACT_DOC}"
            )
        })?;
        let fields: Vec<String> = raw_fields
            .iter()
            .map(|field| {
                field
                    .as_str()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        format!(
                            "pack.json 的 `inboxRoutes.{kind}.fields` 必须是非空字符串数组（段名不得为空）。完整形状：\n{INBOX_ROUTES_SHAPE}\n{CONTRACT_DOC}"
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if fields.is_empty() {
            return Err(format!(
                "pack.json 的 `inboxRoutes.{kind}.fields` 不得为空数组。完整形状：\n{INBOX_ROUTES_SHAPE}\n{CONTRACT_DOC}"
            ));
        }
        let write = match spec.get("write") {
            None => None,
            Some(raw) => Some(Self::parse_write(kind, raw)?),
        };
        // 命名段形态必须至少有一个可写的段（除幂等键）：否则写出来的记录
        // 无法构成载荷（`payload_for` 会把它当坏行）。
        if matches!(
            write.as_ref().map(|binding| binding.shape),
            Some(WriteShape::Named)
        ) && fields.iter().all(|field| field.as_str() == external_field)
        {
            return Err(format!(
                "pack.json 的 `inboxRoutes.{kind}` 声明了 `write.shape = \"named\"`，但 `fields` 里除幂等键（`{external_field}`）之外没有可写段。完整形状：\n{INBOX_ROUTES_SHAPE}\n{CONTRACT_DOC}"
            ));
        }
        Ok(InboxRoute {
            action: action.to_owned(),
            fields,
            write,
        })
    }

    /// 写命令绑定解析（`flag` 非空 + `shape` ∈ {named, payload}）。
    fn parse_write(kind: &str, spec: &Json) -> Result<InboxWriteBinding, String> {
        let flag = spec
            .get("flag")
            .and_then(Json::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .ok_or_else(|| {
                format!(
                    "pack.json 的 `inboxRoutes.{kind}.write` 缺少字段 `flag`（必需，宿主 CLI 旗标名）。完整形状：\n{INBOX_ROUTES_SHAPE}\n{CONTRACT_DOC}"
                )
            })?;
        let shape = match spec.get("shape").and_then(Json::as_str) {
            Some("named") => WriteShape::Named,
            Some("payload") => WriteShape::Payload,
            _ => {
                return Err(format!(
                    "pack.json 的 `inboxRoutes.{kind}.write.shape` 必须是 `named` 或 `payload`。完整形状：\n{INBOX_ROUTES_SHAPE}\n{CONTRACT_DOC}"
                ))
            }
        };
        Ok(InboxWriteBinding {
            flag: flag.to_owned(),
            shape,
        })
    }

    /// 从 Mod 目录现读 `pack.json`（任何问题都以可读错误返回，不兜底）。
    pub fn load(dir: &Path) -> Result<Self, String> {
        Self::from_pack(&read_json(&dir.join("pack.json"))?)
    }

    /// inbox 记录里幂等键的字段名（保留键 `externalIdField`）。
    pub fn external_id_field(&self) -> &str {
        &self.external_id_field
    }

    /// inbox 记录里预成形载荷的字段名（保留键 `payloadField`）。
    pub fn payload_field(&self) -> &str {
        &self.payload_field
    }

    /// 幂等键拼接到预成形载荷末尾的后缀标记（保留键 `extMarker`）。
    pub fn ext_marker(&self) -> &str {
        &self.ext_marker
    }

    /// 按 kind 找路由（未声明 → `None` = 坏行）。
    pub fn get(&self, kind: &str) -> Option<&InboxRoute> {
        self.routes
            .iter()
            .find(|(declared, _)| declared == kind)
            .map(|(_, route)| route)
    }

    /// 按宿主 CLI 旗标名找 kind + 路由（未声明 → `None` = 明确拒绝写）。
    pub fn write_binding(&self, flag: &str) -> Option<(&str, &InboxRoute)> {
        self.routes
            .iter()
            .find(|(_, route)| {
                route
                    .write
                    .as_ref()
                    .is_some_and(|binding| binding.flag == flag)
            })
            .map(|(kind, route)| (kind.as_str(), route))
    }

    /// 已声明的 kind 数量（`0` = 本 Mod 不接受异步 inbox）。
    pub fn len(&self) -> usize {
        self.routes.len()
    }

    /// 是否一个 kind 都没声明。
    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    /// 一条 inbox 记录 → dispatch 载荷（`None` = 坏行：缺幂等键 / 声明的段全空）。
    ///
    /// 两种记录形态都支持（同一套声明驱动，无 kind 分支）：
    /// - **命名段形态**（记录里带同名字段）：按 `fields` 顺序取值，缺失即空串占位
    ///   （保持位置对齐），每段经 [`sanitize_field`] 清洗（段内 `|` / 换行会破坏
    ///   wire format）；尾部空段剔除（不留悬空分隔符）。
    /// - **预成形 wire 形态**（记录只有预成形载荷字段）：载荷原样透传——它已经是
    ///   `req|title|desc|depends` 这种完整载荷，宿主无权重排。
    ///
    /// 幂等键：路由把 `externalIdField` 声明为**段** → 作为普通段补齐（段数已够
    /// 则视为自带）；没声明 → 按 `<extMarker><幂等键>` 后缀注入。
    pub fn payload_for(&self, route: &InboxRoute, record: &Json) -> Option<String> {
        let external = record[self.external_id_field.as_str()]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_owned();
        if external.is_empty() {
            return None; // 幂等键是重放安全的唯一依据：缺了就是坏行
        }
        let named: Vec<(&String, String)> = route
            .named_fields(&self.external_id_field)
            .map(|field| {
                (
                    field,
                    record[field.as_str()].as_str().unwrap_or("").to_owned(),
                )
            })
            .collect();
        // ① 命名段形态：记录里确实带了本路由声明的非幂等段。
        if named.iter().any(|(_, value)| !value.trim().is_empty()) {
            let mut segments: Vec<String> = Vec::with_capacity(route.fields.len());
            for field in &route.fields {
                segments.push(if field == &self.external_id_field {
                    external.clone()
                } else {
                    sanitize_field(record[field.as_str()].as_str().unwrap_or(""))
                });
            }
            return Some(join_segments(segments));
        }
        // ② 预成形 wire 形态：载荷字段是完整载荷，原样透传。
        let raw = record[self.payload_field.as_str()]
            .as_str()
            .unwrap_or("")
            .trim();
        if raw.is_empty() {
            return None;
        }
        if route.declares_external(&self.external_id_field) {
            // 幂等键按段补：已够长（≥ 声明段数）= 该行自带，不重写。
            let segments: Vec<String> = raw.split('|').map(str::to_owned).collect();
            if segments.len() >= route.fields.len() {
                return Some(raw.to_owned());
            }
            let mut padded = segments;
            while padded.len() + 1 < route.fields.len() {
                padded.push(String::new());
            }
            padded.push(external);
            return Some(join_segments(padded));
        }
        Some(format!("{raw}|{}{external}", self.ext_marker))
    }
}

/// 取一个必填保留键（缺失 / 非字符串 / 空串 → 错误文案点名它 + 给全形状）。
fn reserved_text(section: &Json, key: &str, what: &str) -> Result<String, String> {
    match section.get(key) {
        None => Err(format!(
            "pack.json 的 `inboxRoutes` 段缺少保留键 `{key}`（必需，非空字符串 = {what}）。该段的完整形状：\n{INBOX_ROUTES_SHAPE}\n{CONTRACT_DOC}"
        )),
        Some(value) => match value.as_str().map(str::trim) {
            Some(text) if !text.is_empty() => Ok(text.to_owned()),
            _ => Err(format!(
                "pack.json 的 `inboxRoutes` 段的保留键 `{key}` 必须是非空字符串（{what}；当前是 {}）。该段的完整形状：\n{INBOX_ROUTES_SHAPE}\n{CONTRACT_DOC}",
                json_kind(value)
            )),
        },
    }
}

/// 段序拼装（尾部空段剔除：不留悬空 `|`；可选段缺失即不出现）。
pub fn join_segments(mut segments: Vec<String>) -> String {
    while segments.last().is_some_and(|last| last.trim().is_empty()) {
        segments.pop();
    }
    segments.join("|")
}

/// 把分隔符换成全角形态（保留可读性，不丢字）。
pub fn sanitize_field(text: &str) -> String {
    text.replace('|', "｜")
        .replace(['\n', '\r'], " ")
        .trim()
        .to_owned()
}

// ============================================================================
// §结算口径（data/config.json 的 `settlement` 段，四项全部必填）
// ============================================================================

/// 结算判定口径：四项都是必填布尔（无「缺省开启」的隐式行为）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettlementPolicy {
    /// 事件日志必须非空。
    pub require_events: bool,
    /// 不得有失败步。
    pub require_no_failed_steps: bool,
    /// 必须出现结局叙事行。
    pub require_ending: bool,
    /// inbox 不得有未消费行。
    pub require_inbox_consumed: bool,
}

/// `settlement` 段的应有形状。
const SETTLEMENT_SHAPE: &str = r#"{
  "settlement": {
    "requireEvents": true,
    "requireNoFailedSteps": true,
    "requireEnding": true,
    "requireInboxConsumed": true
  }
}"#;

impl SettlementPolicy {
    /// 从已解析的 `config.json` 取 `settlement` 段（缺段 / 缺键 / 非布尔 → 错误）。
    pub fn from_config(config: &Json, source: &Path) -> Result<Self, String> {
        let Some(section) = config.get("settlement") else {
            return Err(format!(
                "{} 缺少 `settlement` 段（结算口径必需）。该段的完整形状：\n{SETTLEMENT_SHAPE}\n{CONTRACT_DOC}",
                source.display()
            ));
        };
        if !section.is_object() {
            return Err(format!(
                "{} 的 `settlement` 段必须是对象（当前是 {}）。该段的完整形状：\n{SETTLEMENT_SHAPE}\n{CONTRACT_DOC}",
                source.display(),
                json_kind(section)
            ));
        }
        let flag = |key: &str| -> Result<bool, String> {
            section.get(key).and_then(Json::as_bool).ok_or_else(|| {
                format!(
                    "{} 的 `settlement` 段缺少字段 `{key}`（必需，布尔）。该段的完整形状：\n{SETTLEMENT_SHAPE}\n{CONTRACT_DOC}",
                    source.display()
                )
            })
        };
        Ok(Self {
            require_events: flag("requireEvents")?,
            require_no_failed_steps: flag("requireNoFailedSteps")?,
            require_ending: flag("requireEnding")?,
            require_inbox_consumed: flag("requireInboxConsumed")?,
        })
    }

    /// 从配置文件路径现读（任何问题都以可读错误返回）。
    pub fn load(path: &Path) -> Result<Self, String> {
        Self::from_config(&read_json(path)?, path)
    }
}

// ============================================================================
// §自动推进阈值（data/config.json 的 `drive` 段，三项全部必填正整数）
// ============================================================================

/// 自动推进阈值（宿主消费；内核不解释）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DriveLimits {
    /// 基础步数硬顶。
    pub max_steps: usize,
    /// 连续无进展判停阈值。
    pub stall_limit: usize,
    /// 每条需求额外放宽的步数。
    pub steps_per_requirement: usize,
}

/// `drive` 段的应有形状。
const DRIVE_SHAPE: &str = r#"{
  "drive": {
    "maxSteps": 64,
    "stallLimit": 12,
    "stepsPerRequirement": 16
  }
}"#;

impl DriveLimits {
    /// 从已解析的 `config.json` 取 `drive` 段（缺段 / 缺键 / 非正整数 → 错误）。
    pub fn from_config(config: &Json, source: &Path) -> Result<Self, String> {
        let Some(section) = config.get("drive") else {
            return Err(format!(
                "{} 缺少 `drive` 段（自动推进阈值必需）。该段的完整形状：\n{DRIVE_SHAPE}\n{CONTRACT_DOC}",
                source.display()
            ));
        };
        if !section.is_object() {
            return Err(format!(
                "{} 的 `drive` 段必须是对象（当前是 {}）。该段的完整形状：\n{DRIVE_SHAPE}\n{CONTRACT_DOC}",
                source.display(),
                json_kind(section)
            ));
        }
        let positive = |key: &str| -> Result<usize, String> {
            match section.get(key).and_then(Json::as_u64) {
                Some(value) if value > 0 => Ok(value as usize),
                _ => Err(format!(
                    "{} 的 `drive` 段缺少字段 `{key}`（必需，正整数）。该段的完整形状：\n{DRIVE_SHAPE}\n{CONTRACT_DOC}",
                    source.display()
                )),
            }
        };
        let limits = Self {
            max_steps: positive("maxSteps")?,
            stall_limit: positive("stallLimit")?,
            steps_per_requirement: positive("stepsPerRequirement")?,
        };
        Ok(limits)
    }

    /// 从配置文件路径现读（任何问题都以可读错误返回）。
    pub fn load(path: &Path) -> Result<Self, String> {
        Self::from_config(&read_json(path)?, path)
    }

    /// 本局步数预算：基础硬顶 + 每条需求额外步数（饱和运算，不溢出）。
    pub fn step_limit(&self, requirement_count: usize) -> usize {
        self.max_steps
            .saturating_add(self.steps_per_requirement.saturating_mul(requirement_count))
    }
}

/// JSON 值的类型名（错误文案用，避免把整段值打进日志）。
fn json_kind(value: &Json) -> &'static str {
    match value {
        Json::Null => "null",
        Json::Bool(_) => "布尔",
        Json::Number(_) => "数字",
        Json::String(_) => "字符串",
        Json::Array(_) => "数组",
        Json::Object(_) => "对象",
    }
}
