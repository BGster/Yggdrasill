use super::{json_kind, read_json, CONTRACT_DOC};
use serde_json::Value as Json;
use std::path::Path;

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

/// `inboxRoutes` 段的三个保留键：与 `<kind>` **同层**，任何 kind 都不得占用这些名字。
const INBOX_RESERVED_KEYS: [&str; 3] = ["externalIdField", "payloadField", "extMarker"];

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
        assert_eq!(routes.actor_flags_for("verify.submit"), (false, false));
    }

    /// r9：**stamp-only 路由**（声明 `actor` 但不带 `write`）是合法形态——它让宿主为
    /// 非 inbox 动作盖身份章，却不长出任何 CLI 写命令（`write_binding` 只匹配带 `write` 的）。
    #[test]
    fn stamp_only_route_declares_actor_without_a_write_binding() {
        let mut route = legacy_route();
        route["action"] = json!("config.set");
        route["fields"] = json!(["key", "value", "external_id"]);
        route["actor"] = json!(true);
        route.as_object_mut().expect("route object").remove("write");
        let routes = InboxRoutes::from_pack(&routes_pack(route)).expect("stamp-only 必须放行");
        assert_eq!(routes.actor_flags_for("config.set"), (true, false));
        // 无 write 绑定 ⇒ 宿主拒绝任何以它为目标的写命令（不会冒出 `--config` 旗标）。
        assert!(routes.write_binding("config").is_none());
        assert!(routes.write_binding("proposal").is_none());
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
