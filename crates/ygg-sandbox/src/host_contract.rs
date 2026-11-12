//! host ↔ Mod 的**显式契约**（通用 Mod 工具的零硬编码面）。
//!
//! 四样以前靠宿主字面量硬编码的东西——事件名与叙事约定、inbox 记录字段与
//! kind → 动作、结算判定项、自动推进阈值——加载期从 Mod 自己的声明里读：
//!
//! | 声明位置 | 段 | 消费方 |
//! | --- | --- | --- |
//! | `pack.json` | `agent`（**必填**，八个字段） | `examples/egg.rs`、`src/playtest.rs` |
//! | `pack.json` | `inboxRoutes`（**必填**，三个保留键 + 若干 kind） | `examples/egg.rs` |
//! | `data/config.json` | `settlement`（**必填**） | `examples/egg.rs --settle` |
//! | `data/config.json` | `drive`（**必填**） | `examples/egg.rs` |
//! | `pack.json` | `tools`（**可选**，缺省 / 空对象 = 本 Mod 不请求外部工具） | `examples/egg.rs` |
//! | `data/config.json` | `allowedTools`（**可选**，缺省 = 不允许任何工具） | `examples/egg.rs` |
//!
//! **无默认值纪律**：所有段都是必填，缺失 / 字段缺失 / 类型不符 / 非正数一律
//! 返回可读错误（带缺失字段名 + 应有形状 + 文档定位），由调用方 fail-closed
//! （报错退出或 `ok=false` 报告）——宿主**不**提供任何回退默认值。旧 Mod 因此
//! 必须在升级时补齐这四段（见 `docs/DESIGN.md` 与
//! `.ygg/mods/agent-seed/README.md` 的「host 契约」章节）。
//!
//! # 两层工具白名单（`tools` / `allowedTools`，两者都可选）
//!
//! 第一层（`pack.json` 的 `tools`）定**形状与能力**：每个工具声明 kind（`cli`
//! 直接 spawn、不经 shell；`http` 经 `curl`）、超时、输出上限、是否只读、结果
//! 事件名。宿主逐条校验，**半声明**（cli 带 `url`、http 带 `program`）一律拒绝。
//! 第二层（`data/config.json` 的 `allowedTools`）定**本局放行清单**：`tools` 里
//! 声明了但没放行的 id 在**装载期**即失败（[`require_tool_allowlist`]），不推迟到
//! 执行时。两层都缺失 = 工具能力整体不可用（既有 Mod 零改动照常工作）。
//!
//! **诚实声明**：`allowedTools` 与 `pack.json` 同在 Mod 目录、同一个作者——它是
//! **防漂移 / 防半声明**的门禁，**不是**防恶意 Mod 的安全边界。真正的边界是第一层
//! 的形状限制（无 shell、无绝对路径、无 `..` 路径段、`$ENV{}` 间接取凭据）+ `readOnly`
//! + 审计面。若将来要真正的边界，放行清单应搬出 Mod 目录（运维侧持有）。
//!
//! # 代码面契约（`pack.json` 的 `code` 段，可选）
//!
//! 同一份 `pack.json` 上还有第五个声明面，但它**不由本模块解析**（消费方是
//! [`crate::pack`] 与 [`crate::modload`]）——列在这里是因为它与上面四段同属
//! 「宿主不猜、出事即失败」的契约面，且两者容易漂移：
//!
//! | 面 | 口径 | 消费方 |
//! | --- | --- | --- |
//! | 登记面 | `code.{entry, modules[]}`：入口**不**登记为模块（同一源码两个装载身份让缓存与定位歧义）；模块**逐条枚举**，无 glob；`entry ↔ modules` 与 `manifest.json` 的 `code` 数组双向相等 | [`crate::pack::CodeSpec`]、[`crate::modload::ModBundle`] |
//! | 装载期边界 | **只有注册面有边界**：`host.seal` 封 `ygg.on`（运行期注册禁令）。`require` **不**在边界上——装载期与 handler 内部一视同仁（懒加载成立） | [`crate::modules`]、[`crate::host_api::HostApi::seal`] |
//! | 副作用边界 | **入口可注册，模块只导出**：入口 chunk 顶层就是装配清单（注册 / 写状态 / 输出都是它的职责）；模块 chunk 顶层只做纯导出（定义函数与常量），注册 / 写状态 / 输出一律搬进模块函数体，由入口显式调用（`M.install(deps)`）。理由：`ygg.on` 的注册序就是派发序——模块自己注册，等于让「文件怎么拆」决定「运行时行为」，拆文件或调 require 顺序就改行为。禁止面清单在 `ygg_modules::effects`，运行期由影子表执行、静态由 `ygg-lint --pack` 的 `YGG-MOD-EFFECT` 报 | [`crate::modules`]、[`ygg_modules::effects`] |
//! | 错误前缀口径 | 装载器自判的拒绝带**调用点**（`src/a.lua:3: <原因>`）；模块 chunk 自身的错误按 Lua 原生错误链浮出（每层 `path:line:` + traceback）——两类同形，都带真实文件与行号，禁止「拼回单文件再报错」 | [`crate::modules`] |
//!
//! 与四段 host 契约的差别：`code` 段可以整段缺失（缺 = 单文件 Mod，行为逐字节
//! 不变）；存在则**全量校验**，不允许半声明。边界细节见 `docs/DESIGN.md`
//! §3.2「Mod 包与多文件纯度契约」。
//!
//! 绑定特定 Mod 的应用（`examples/adventure.rs`、`clients/godot`、
//! `packages/client-web`…）不属本模块的消费方：它们各自服务固定 Mod，文件头
//! 注明绑定对象即可（归属边界见 `data/playbook/ch4-mod-host.json` 的 `contract.literal-ownership`）。

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
      "actor": <可选 bool：宿主为该动作的载荷注入 `|actor=<提交者>` 段，并拦截调用方自带的同名段>,
      "actor_session": <可选 bool：同上，注入 `|actor_session=<会话id>`（相对顺序固定在 actor 之后）>,
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

/// 一条 inbox 路由：`kind` → 内核动作 + 段序 + 提交者身份声明 + 可选写命令绑定。
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
    /// 是否声明宿主身份段 `actor`：声明后宿主为该动作的载荷注入 `|actor=<提交者>`
    /// 段，并把调用方自带的同名段判为伪造（ACTOR_FORGED，不派发不落盘）。
    pub actor: bool,
    /// 是否声明宿主身份段 `actor_session`：同 [`InboxRoute::actor`]，注入的是
    /// `|actor_session=<会话id>`（不透明审计字段；相对顺序固定在 actor 之后）。
    pub actor_session: bool,
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
/// 工具结果也走**同一条通道**（`kind` 由 Mod 声明，宿主不枚举）：`tools.<id>` 的
/// `resultEvent` 是派发的动作名，inbox 只是一个可选的、可跨进程的投递载体。
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
        // 身份段声明：可选布尔；**非 bool 一律 fail-closed**（缺省 = false，功能关闭，
        // 旧 pack 前向兼容；写错类型比静默按 false 处理更诚实）。
        let actor = Self::parse_route_bool(kind, spec, "actor")?;
        let actor_session = Self::parse_route_bool(kind, spec, "actor_session")?;
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
            actor,
            actor_session,
            write,
        })
    }

    /// 单条路由的可选布尔读取（缺省 `false`；非 bool = fail-closed 可读错误）。
    fn parse_route_bool(kind: &str, spec: &Json, key: &str) -> Result<bool, String> {
        match spec.get(key) {
            None => Ok(false),
            Some(Json::Bool(value)) => Ok(*value),
            Some(other) => Err(format!(
                "pack.json 的 `inboxRoutes.{kind}.{key}` 必须是布尔（缺省 false；当前是 {}）。完整形状：\n{INBOX_ROUTES_SHAPE}\n{CONTRACT_DOC}",
                json_kind(other)
            )),
        }
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

    /// 按内核动作反查该动作的身份段声明：`(声明注入 actor, 声明注入 actor_session)`。
    ///
    /// 动作不在表内 / 两布尔都没声明 → `(false, false)`：宿主不注入也不拦截
    /// （声明承重——**没声明的动作没有身份段**，这是旧 pack 前向兼容的根据）。
    pub fn actor_flags_for(&self, action: &str) -> (bool, bool) {
        self.routes
            .iter()
            .find(|(_, route)| route.action == action)
            .map(|(_, route)| (route.actor, route.actor_session))
            .unwrap_or((false, false))
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

// ============================================================================
// §工具两层白名单的装载期校验（`tools` / `allowedTools`）
//
// 这些用例只碰纯 JSON：不建 Mod 目录、不 spawn 进程、不读环境——校验规则是
// 装载期门禁的全部事实面（运行期执行模型在 `examples/egg.rs` 的
// `tool_runner_tests` 里测）。
// ============================================================================
#[cfg(test)]
mod tool_contract_tests {
    use super::*;
    use serde_json::json;

    /// 一条形状完全合法的 cli 工具（各用例只改自己关心的字段）。
    fn cli_spec() -> Json {
        json!({
            "kind": "cli",
            "program": "git",
            "args": ["status", "--porcelain"],
            "timeoutMs": 120_000u64,
            "inlineWaitMs": 5_000u64,
            "maxOutputBytes": 65_536u64,
            "readOnly": true,
            "resultEvent": "tool.result",
        })
    }

    /// 一条形状完全合法的 http 工具。
    fn http_spec() -> Json {
        json!({
            "kind": "http",
            "method": "POST",
            "url": "https://example.test/api",
            "headers": {"Authorization": "Bearer $ENV{TOKEN}"},
            "bodyTemplate": "{\"repo\":\"{{repo}}\"}",
            "timeoutMs": 30_000u64,
            "inlineWaitMs": 1_000u64,
            "maxOutputBytes": 4_096u64,
            "readOnly": true,
            "resultEvent": "tool.result",
        })
    }

    /// 只含一条声明的 `pack.json`（键名可变，用于断言错误文案里的 id）。
    fn pack_one(id: &str, spec: Json) -> Json {
        let mut table = serde_json::Map::new();
        table.insert(id.to_owned(), spec);
        json!({ "tools": Json::Object(table) })
    }

    /// 解析一条声明（id 固定为 `probe`）。
    fn parse_one(spec: Json) -> Result<ToolRegistry, String> {
        ToolRegistry::from_pack(&pack_one("probe", spec))
    }

    /// 合成配置路径（只用于错误文案，不读盘）。
    fn config_source() -> &'static Path {
        Path::new("/tmp/synthetic-mod/data/config.json")
    }

    /// 放行清单的合成 `config.json`。
    fn config_with(allowed: Json) -> Json {
        json!({ "allowedTools": allowed })
    }

    #[test]
    fn tools_absent_is_backward_compatible() {
        let registry = ToolRegistry::from_pack(&json!({"agent": {}})).expect("缺段必须放行");
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
        let access = ToolAccess::from_config(&json!({}), config_source()).expect("缺段必须放行");
        assert!(access.is_empty());
        assert!(registry.denied(&access).is_empty());
        assert!(require_tool_allowlist(&registry, &access, config_source()).is_ok());
    }

    #[test]
    fn tools_empty_object_is_backward_compatible() {
        let registry = ToolRegistry::from_pack(&json!({"tools": {}})).expect("空对象必须放行");
        assert!(registry.is_empty());
        assert!(registry.iter().next().is_none());
        let access = ToolAccess::from_config(&config_with(json!([])), config_source())
            .expect("空数组必须放行");
        assert!(access.is_empty());
        assert!(require_tool_allowlist(&registry, &access, config_source()).is_ok());
    }

    #[test]
    fn tools_rejects_non_object() {
        let error = ToolRegistry::from_pack(&json!({"tools": []})).expect_err("数组必须被拒");
        assert!(error.contains(TOOLS_SHAPE), "{error}");
        assert!(error.contains(CONTRACT_DOC), "{error}");
        assert!(error.contains("必须是对象"), "{error}");
    }

    #[test]
    fn tool_ids_must_be_lowercase_wire_safe() {
        let error = ToolRegistry::from_pack(&pack_one("Git Status!", cli_spec()))
            .expect_err("非法 id 必须被拒");
        assert!(error.contains("Git Status!"), "{error}");
        assert!(error.contains("[a-z0-9._-]"), "{error}");
        // 合法 id：小写字母 / 数字 / `.` `_` `-`
        assert!(ToolRegistry::from_pack(&pack_one("git.status_v2-1", cli_spec())).is_ok());
    }

    #[test]
    fn cli_happy_path_roundtrips() {
        let registry = parse_one(cli_spec()).expect("合法 cli 声明必须通过");
        let spec = registry.get("probe").expect("按 id 取到声明");
        assert_eq!(spec.id, "probe");
        assert_eq!(spec.kind, ToolKind::Cli);
        assert_eq!(spec.program.as_deref(), Some("git"));
        assert_eq!(
            spec.args,
            vec!["status".to_owned(), "--porcelain".to_owned()]
        );
        assert_eq!(spec.method, None);
        assert_eq!(spec.url, None);
        assert!(spec.headers.is_empty());
        assert_eq!(spec.body_template, None);
        assert_eq!(spec.timeout_ms, 120_000);
        assert_eq!(spec.inline_wait_ms, 5_000);
        assert_eq!(spec.max_output_bytes, 65_536);
        assert!(spec.read_only);
        assert_eq!(spec.result_event, "tool.result");
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn http_happy_path_roundtrips() {
        let registry = parse_one(http_spec()).expect("合法 http 声明必须通过");
        let spec = registry.get("probe").expect("按 id 取到声明");
        assert_eq!(spec.kind, ToolKind::Http);
        assert_eq!(spec.method.as_deref(), Some("POST"));
        assert_eq!(spec.url.as_deref(), Some("https://example.test/api"));
        assert_eq!(
            spec.headers,
            vec![("Authorization".to_owned(), "Bearer $ENV{TOKEN}".to_owned())]
        );
        assert_eq!(
            spec.body_template.as_deref(),
            Some("{\"repo\":\"{{repo}}\"}")
        );
        assert_eq!(spec.program, None);
        assert!(spec.args.is_empty());
    }

    #[test]
    fn cli_rejects_shell_basenames() {
        for program in [
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
            "pwsh",
            "/bin/sh",
            "tools/bash",
        ] {
            let mut spec = cli_spec();
            spec["program"] = json!(program);
            let error = parse_one(spec).expect_err(&format!("`{program}` 必须被拒"));
            assert!(
                error.contains("program"),
                "错误文案必须点名 program（{program}）：{error}"
            );
        }
    }

    #[test]
    fn cli_rejects_absolute_and_traversal() {
        for program in ["/usr/bin/git", "../git", "a/../b", "a b", "git;rm", "$(x)"] {
            let mut spec = cli_spec();
            spec["program"] = json!(program);
            let error = parse_one(spec).expect_err(&format!("`{program}` 必须被拒"));
            assert!(error.contains("program"), "{error}");
        }
    }

    #[test]
    fn cli_allows_relative_subpath() {
        let mut spec = cli_spec();
        spec["program"] = json!("./scripts/probe.sh");
        let registry = parse_one(spec).expect("有效相对路径必须放行（否则规则过紧）");
        assert_eq!(
            registry
                .get("probe")
                .and_then(|spec| spec.program.as_deref()),
            Some("./scripts/probe.sh")
        );
    }

    /// `@self`：**只此一处**合法（内置子命令的可移植入口），其余 `@` 形态照旧被拒。
    #[test]
    fn cli_accepts_self_program_and_rejects_look_alikes() {
        let mut spec = cli_spec();
        spec["program"] = json!(SELF_PROGRAM);
        let registry = parse_one(spec).expect("`@self` 必须放行（否则内置子命令不可携带）");
        assert_eq!(
            registry
                .get("probe")
                .and_then(|spec| spec.program.as_deref()),
            Some(SELF_PROGRAM)
        );

        // 长像的都不行：`@` 不在常规字符集里，只有精确的 `@self` 走特例。
        for program in ["@SELF", "@self/x", "x@self", "@self..", "@", "@self."] {
            let mut spec = cli_spec();
            spec["program"] = json!(program);
            let error = parse_one(spec).expect_err(&format!("`{program}` 必须被拒"));
            assert!(error.contains("program"), "{error}");
        }

        // 裸名 `egg` 本来就是合法形态（`execute_cli` 会把它解析到 current_exe）。
        let mut spec = cli_spec();
        spec["program"] = json!("egg");
        assert!(parse_one(spec).is_ok());
    }

    #[test]
    fn cli_rejects_pipe_in_args() {
        for bad in ["a|b", "a\nb", "a\rb", "a\0b"] {
            let mut spec = cli_spec();
            spec["args"] = json!([bad]);
            let error = parse_one(spec).expect_err(&format!("`{bad:?}` 必须被拒"));
            assert!(error.contains("args[0]"), "{error}");
        }
    }

    #[test]
    fn cli_rejects_bad_env_syntax() {
        for bad in ["$ENV{1BAD}", "$ENV{X", "$ENV{}", "{{unclosed"] {
            let mut spec = cli_spec();
            spec["args"] = json!([bad]);
            let error = parse_one(spec).expect_err(&format!("`{bad}` 必须被拒"));
            assert!(error.contains("args[0]"), "{error}");
        }
        // 合法形态必须放行
        let mut spec = cli_spec();
        spec["args"] = json!(["--token", "$ENV{GH_TOKEN}", "{{repo}}"]);
        assert!(parse_one(spec).is_ok());
    }

    #[test]
    fn http_rejects_non_http_scheme() {
        for url in ["file:///etc/passwd", "ftp://x", "//x", "example.test/api"] {
            let mut spec = http_spec();
            spec["url"] = json!(url);
            let error = parse_one(spec).expect_err(&format!("`{url}` 必须被拒"));
            assert!(error.contains("url"), "{error}");
        }
    }

    #[test]
    fn http_rejects_unknown_method() {
        for method in ["PATCH", "get", "HEAD", ""] {
            let mut spec = http_spec();
            spec["method"] = json!(method);
            let error = parse_one(spec).expect_err(&format!("`{method}` 必须被拒"));
            assert!(error.contains("method"), "{error}");
        }
    }

    #[test]
    fn cross_kind_fields_rejected() {
        let mut cli = cli_spec();
        cli["url"] = json!("https://example.test/api");
        let error = parse_one(cli).expect_err("cli 带 url 必须被拒");
        assert!(error.contains("半声明"), "{error}");

        let mut http = http_spec();
        http["args"] = json!(["--verbose"]);
        let error = parse_one(http).expect_err("http 带 args 必须被拒");
        assert!(error.contains("半声明"), "{error}");

        let mut cli_program = cli_spec();
        cli_program["headers"] = json!({"X-A": "b"});
        assert!(parse_one(cli_program).is_err(), "cli 带 headers 必须被拒");
    }

    #[test]
    fn missing_fields_are_listed_in_one_shot() {
        let mut spec = cli_spec();
        for field in ["program", "args", "timeoutMs", "resultEvent"] {
            spec.as_object_mut().expect("对象").remove(field);
        }
        let error = parse_one(spec).expect_err("缺字段必须被拒");
        for field in ["program", "args", "timeoutMs", "resultEvent"] {
            assert!(error.contains(field), "错误文案必须列出 `{field}`：{error}");
        }
        assert!(error.contains("必填"), "{error}");
    }

    #[test]
    fn inline_wait_must_not_exceed_timeout() {
        let mut spec = cli_spec();
        spec["timeoutMs"] = json!(1_000u64);
        spec["inlineWaitMs"] = json!(5_000u64);
        let error = parse_one(spec).expect_err("inlineWaitMs > timeoutMs 必须被拒");
        assert!(error.contains("inlineWaitMs"), "{error}");
        assert!(error.contains("timeoutMs"), "{error}");
    }

    #[test]
    fn mutating_tool_must_be_inline() {
        let mut spec = cli_spec();
        spec["readOnly"] = json!(false);
        spec["inlineWaitMs"] = json!(0u64);
        let error = parse_one(spec).expect_err("非只读工具不得脱体");
        assert!(error.contains("inlineWaitMs"), "{error}");
        assert!(error.contains("readOnly"), "{error}");
        // 只读工具脱体是合法的（inlineWaitMs = 0 = 恒走场景 2）
        let mut readonly = cli_spec();
        readonly["inlineWaitMs"] = json!(0u64);
        let registry = parse_one(readonly).expect("只读工具允许 inlineWaitMs = 0");
        assert_eq!(
            registry.get("probe").map(|spec| spec.inline_wait_ms),
            Some(0)
        );
    }

    #[test]
    fn max_output_bytes_bounds() {
        for (value, ok) in [(0i64, false), (65_537, false), (65_536, true), (1, true)] {
            let mut spec = cli_spec();
            spec["maxOutputBytes"] = json!(value);
            assert_eq!(
                parse_one(spec).is_ok(),
                ok,
                "maxOutputBytes = {value} 的判定不符合预期"
            );
        }
    }

    #[test]
    fn timeout_bounds() {
        for (value, ok) in [(0i64, false), (3_600_001, false), (3_600_000, true)] {
            let mut spec = cli_spec();
            spec["timeoutMs"] = json!(value);
            spec["inlineWaitMs"] = json!(0u64);
            assert_eq!(
                parse_one(spec).is_ok(),
                ok,
                "timeoutMs = {value} 的判定不符合预期"
            );
        }
    }

    #[test]
    fn allowlist_absent_allows_nothing() {
        let access = ToolAccess::from_config(&json!({}), config_source()).expect("缺段必须放行");
        assert!(access.is_empty());
        assert!(!access.allows("git.status"));
        assert!(access.allowed().is_empty());
        // tools 也空 → 交叉校验通过（两层都缺 = 工具能力不可用，但那不是错误）
        assert!(require_tool_allowlist(&ToolRegistry::default(), &access, config_source()).is_ok());
    }

    #[test]
    fn allowlist_missing_entry_fails_closed() {
        let mut table = serde_json::Map::new();
        table.insert("a".to_owned(), cli_spec());
        table.insert("b".to_owned(), cli_spec());
        let registry = ToolRegistry::from_pack(&json!({ "tools": Json::Object(table) }))
            .expect("两条声明都合法");
        let access = ToolAccess::from_config(&config_with(json!(["a"])), config_source())
            .expect("放行清单合法");
        assert_eq!(registry.denied(&access), vec!["b".to_owned()]);
        let error = require_tool_allowlist(&registry, &access, config_source())
            .expect_err("未放行的声明必须拒绝启动");
        assert!(error.contains("`b`"), "必须列出未放行的 id：{error}");
        assert!(
            !error.contains("`a`（"),
            "已放行的 id 不该进缺失清单：{error}"
        );
        assert!(error.contains("allowedTools"), "{error}");
        assert!(error.contains(ALLOWED_TOOLS_SHAPE), "{error}");
        assert!(error.contains(TOOLS_SHAPE), "{error}");
    }

    #[test]
    fn allowlist_extra_entry_is_allowed() {
        let registry = ToolRegistry::from_pack(&pack_one("a", cli_spec())).expect("声明合法");
        let access = ToolAccess::from_config(&config_with(json!(["a", "zzz"])), config_source())
            .expect("多放行不是错误");
        assert!(access.allows("zzz"));
        assert!(require_tool_allowlist(&registry, &access, config_source()).is_ok());
    }

    #[test]
    fn allowlist_rejects_non_string_entries() {
        for bad in [json!([1]), json!([""]), json!(["   "]), json!([null])] {
            let error = ToolAccess::from_config(&config_with(bad.clone()), config_source())
                .expect_err(&format!("{bad} 必须被拒"));
            assert!(error.contains("第 1 项"), "{error}");
            assert!(error.contains(ALLOWED_TOOLS_SHAPE), "{error}");
        }
        // 非数组整段拒绝
        let error = ToolAccess::from_config(&json!({"allowedTools": "a"}), config_source())
            .expect_err("非数组必须被拒");
        assert!(error.contains("必须是字符串数组"), "{error}");
        // 重复项按集合语义允许
        let access = ToolAccess::from_config(&config_with(json!(["a", "a"])), config_source())
            .expect("重复项允许");
        assert!(access.allows("a"));
        assert_eq!(access.allowed().len(), 2);
    }
}

// ============================================================================
// §inbox 路由的身份段声明（`actor` / `actor_session` 两布尔）
//
// 声明承重：没声明的动作没有身份段（宿主不注入也不拦截）。这里只碰纯 JSON，
// 不建 Mod 目录（运行期注入/拦截在 `examples/egg.rs` 的宿主通道用例里测）。
// ============================================================================
#[cfg(test)]
mod inbox_route_actor_tests {
    use super::*;
    use serde_json::json;

    /// 合成 `inboxRoutes` 段：三保留键 + 一条路由（可覆盖字段）。
    fn routes_pack(route: Json) -> Json {
        json!({
            "inboxRoutes": {
                "externalIdField": "external_id",
                "payloadField": "payload",
                "extMarker": "@ext=",
                "proposal": route,
            }
        })
    }

    /// 一条不带身份布尔的基础路由（旧 pack 形态）。
    fn legacy_route() -> Json {
        json!({
            "action": "proposal.submit",
            "fields": ["req", "rev", "stage", "summary", "artifacts", "changes", "external_id"],
            "write": {"flag": "proposal", "shape": "payload"},
        })
    }

    #[test]
    fn actor_bools_default_false_and_old_pack_is_unchanged() {
        let routes =
            InboxRoutes::from_pack(&routes_pack(legacy_route())).expect("旧 pack 必须放行");
        let route = routes.get("proposal").expect("按 kind 取路由");
        assert!(
            !route.actor && !route.actor_session,
            "缺省 = false（功能关闭）"
        );
        // 反查：未声明布尔的动作 → (false, false)
        assert_eq!(routes.actor_flags_for("proposal.submit"), (false, false));
        // 反查：不在表内的动作 → (false, false)（宿主不注入也不拦截）
        assert_eq!(routes.actor_flags_for("config.set"), (false, false));
    }

    #[test]
    fn actor_bools_roundtrip_and_reverse_lookup() {
        let mut route = legacy_route();
        route["actor"] = json!(true);
        route["actor_session"] = json!(true);
        let routes = InboxRoutes::from_pack(&routes_pack(route)).expect("双布尔声明必须放行");
        let parsed = routes.get("proposal").expect("按 kind 取路由");
        assert!(parsed.actor && parsed.actor_session);
        assert_eq!(routes.actor_flags_for("proposal.submit"), (true, true));
    }

    #[test]
    fn actor_bools_are_independent() {
        let mut route = legacy_route();
        route["actor"] = json!(true);
        let routes = InboxRoutes::from_pack(&routes_pack(route)).expect("单布尔声明必须放行");
        assert_eq!(routes.actor_flags_for("proposal.submit"), (true, false));
    }

    #[test]
    fn actor_bools_fail_closed_on_non_bool() {
        for (key, bad) in [
            ("actor", json!("true")),
            ("actor", json!(1)),
            ("actor_session", json!("yes")),
            ("actor_session", json!(null)),
        ] {
            let mut route = legacy_route();
            route[key] = bad.clone();
            let error = InboxRoutes::from_pack(&routes_pack(route))
                .expect_err(&format!("{key}={bad} 非 bool 必须被拒"));
            assert!(error.contains(key), "错误必须点名字段：{error}");
            assert!(error.contains("必须是布尔"), "{error}");
            assert!(error.contains(INBOX_ROUTES_SHAPE), "{error}");
            assert!(error.contains(CONTRACT_DOC), "{error}");
        }
    }
}
