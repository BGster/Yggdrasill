use super::{InboxRoutes, Path};
use serde_json::{json, Value as Json};

/// seed Mod 的契约目录（本仓内的真实契据，测试直接读它——避免在测试里
/// 复刻一份 kind 表，哪边改了都只有一处）。
fn seed_mod_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.ygg/mods/agent-seed")
}

/// 三个保留键（与 kind 同层）。测试里显式声明：宿主不得内置任何字段名。
/// 后缀标记故意用与真 Mod 不同的值，以证明它只认声明。
fn record_keys() -> Json {
    json!({
        "externalIdField": "external_id",
        "payloadField": "payload",
        "extMarker": "~ext~"
    })
}

/// 保留键 + 若干 kind → 一段合法声明。
fn section(kinds: Json) -> Json {
    let mut table = record_keys();
    for (kind, spec) in kinds.as_object().expect("对象") {
        table[kind] = spec.clone();
    }
    json!({"inboxRoutes": table})
}

/// 缺段 / 缺保留键 / 单条非法 → 硬错误；只声明保留键（零 kind）才合法。
#[test]
fn missing_section_or_reserved_key_is_a_hard_error() {
    let message = InboxRoutes::from_pack(&json!({"entries": []})).expect_err("必须报错");
    assert!(message.contains("`inboxRoutes` 段"), "{message}");
    assert!(message.contains("\"action\""), "应给出形状示例：{message}");
    assert!(message.contains("README.md"), "应给出文档定位：{message}");

    // 空表不再是合法声明：三个保留键必填（零 kind 仍然合法）
    let message =
        InboxRoutes::from_pack(&json!({"inboxRoutes": {}})).expect_err("缺保留键必须报错");
    assert!(message.contains("externalIdField"), "{message}");
    for key in ["externalIdField", "payloadField", "extMarker"] {
        let mut table = record_keys();
        table.as_object_mut().expect("对象").remove(key);
        let message =
            InboxRoutes::from_pack(&json!({"inboxRoutes": table})).expect_err("缺保留键必须报错");
        assert!(
            message.contains(&format!("`{key}`")),
            "必须点名 `{key}`：{message}"
        );
        assert!(message.contains("README.md"), "{message}");
    }
    // 保留键空串 / 非字符串 → 同样硬错误
    for (key, value) in [
        ("externalIdField", json!("  ")),
        ("payloadField", json!(7)),
        ("extMarker", Json::Null),
    ] {
        let mut table = record_keys();
        table[key] = value;
        assert!(
            InboxRoutes::from_pack(&json!({"inboxRoutes": table})).is_err(),
            "`{key}` 非法值必须报错"
        );
    }

    let empty = InboxRoutes::from_pack(&section(json!({}))).expect("零 kind 合法");
    assert!(empty.is_empty());
    assert_eq!(empty.len(), 0);
    assert_eq!(empty.external_id_field(), "external_id");
    assert_eq!(empty.payload_field(), "payload");
    assert_eq!(empty.ext_marker(), "~ext~");
    assert!(empty.write_binding("requirement").is_none());

    // 单条非法（保留键齐全）→ 逐项点名
    for (kinds, needle) in [
        (json!({"k": {"fields": ["a"]}}), "`action`"),
        (json!({"k": {"action": "a"}}), "`fields`"),
        (json!({"k": {"action": "a", "fields": []}}), "不得为空数组"),
        (
            json!({"k": {"action": "a", "fields": ["x"], "write": {"shape": "named"}}}),
            "`flag`",
        ),
        (
            json!({"k": {"action": "a", "fields": ["x"], "write": {"flag": "f", "shape": "guess"}}}),
            "`named` 或 `payload`",
        ),
        // 命名段形态必须至少有一个非幂等键段（否则写出来的记录无载荷）
        (
            json!({"k": {"action": "a", "fields": ["external_id"], "write": {"flag": "f", "shape": "named"}}}),
            "没有可写段",
        ),
    ] {
        let message = InboxRoutes::from_pack(&section(kinds)).expect_err("必须报错");
        assert!(message.contains(needle), "文案应点名 `{needle}`：{message}");
    }
    assert!(InboxRoutes::from_pack(&json!({"inboxRoutes": []})).is_err());
}

/// 声明表真的驱动派发：kind → action + 段序 + 写命令绑定，宿主零 kind 字面量。
#[test]
fn declared_routes_drive_the_dispatch() {
    let routes = InboxRoutes::from_pack(&section(json!({
        "mine": {
            "action": "requirement.add",
            "fields": ["title", "description", "external_id"],
            "write": {"flag": "requirement", "shape": "named"}
        },
        "raw": {
            "action": "proposal.submit",
            "fields": ["req", "rev", "stage", "summary", "artifacts", "external_id"],
            "write": {"flag": "proposal", "shape": "payload"}
        },
        "note": {
            "action": "requirement.conflict",
            "fields": ["reqs", "note"],
            "write": {"flag": "conflict", "shape": "payload"}
        }
    })))
    .expect("合法 inboxRoutes");
    assert_eq!(routes.len(), 3);
    let (kind, route) = routes.write_binding("requirement").expect("声明了写命令");
    assert_eq!(kind, "mine");
    assert_eq!(route.action, "requirement.add");
    assert!(route.declares_external(routes.external_id_field()));
    assert_eq!(
        routes
            .payload_for(
                route,
                &json!({"title": "t", "description": "d", "external_id": "e"})
            )
            .expect("命名段形态"),
        "t|d|e"
    );
    // 把幂等键声明成段的路径：按段补齐（不拼后缀）
    let (_, raw) = routes.write_binding("proposal").expect("声明了写命令");
    assert_eq!(
        routes
            .payload_for(
                &raw,
                &json!({"payload": "r1|1|analysis", "external_id": "e"})
            )
            .expect("预成形形态"),
        "r1|1|analysis|||e"
    );
    // 没把幂等键声明成段的路径：按契约声明的后缀标记注入
    let (_, note) = routes.write_binding("conflict").expect("声明了写命令");
    assert!(!note.declares_external(routes.external_id_field()));
    assert_eq!(
        routes
            .payload_for(&note, &json!({"payload": "r1,r2|说明", "external_id": "e"}))
            .expect("后缀形态"),
        "r1,r2|说明|~ext~e"
    );
    // 未声明的旗标 → None（调用方明确拒绝，不猜）
    assert!(routes.write_binding("amend").is_none());
    // 未声明的 kind 与保留键本身都不被当成 kind（= 坏行）
    assert!(routes.get("add").is_none());
    assert!(routes.get("externalIdField").is_none());
    assert!(routes.get("extMarker").is_none());
}

/// **向后兼容的字节面**：真实 seed Mod 契约对旧形态记录产出的载荷与改造前一致。
#[test]
fn seed_contract_reproduces_the_legacy_payloads() {
    let routes = InboxRoutes::load(&seed_mod_dir()).expect("seed Mod 的契约必须合法");
    let marker = routes.ext_marker();
    let payload = |flag: &str, record: Json| {
        let (_, route) = routes.write_binding(flag).expect("声明了写命令");
        routes.payload_for(route, &record)
    };

    // 命名段形态（`--requirement`）：title|description|external_id
    assert_eq!(
        payload(
            "requirement",
            json!({"title": "标题", "description": "描述", "external_id": "inbox-1"})
        ),
        Some("标题|描述|inbox-1".to_owned())
    );
    // 段内 `|` / 换行必须被清洗（否则破坏 wire format）
    assert_eq!(
        payload(
            "requirement",
            json!({"title": "a|b", "description": "c\nd", "external_id": "inbox-2"})
        ),
        Some("a｜b|c d|inbox-2".to_owned())
    );
    // 描述为空 → 中间段留空但位置对齐；幂等键缺失 → 坏行
    assert_eq!(
        payload(
            "requirement",
            json!({"title": "只有标题", "external_id": "inbox-3"})
        ),
        Some("只有标题||inbox-3".to_owned())
    );
    assert_eq!(payload("requirement", json!({"title": "t"})), None);

    // 预成形 wire 形态：proposal 的段数补齐（5 段补空 changes，6 段直接补，7 段原样）
    assert_eq!(
        payload(
            "proposal",
            json!({"payload": "r1|1|analysis|摘要|notes/a.md", "external_id": "inbox-5"})
        ),
        Some("r1|1|analysis|摘要|notes/a.md||inbox-5".to_owned())
    );
    assert_eq!(
        payload(
            "proposal",
            json!({"payload": "r1|1|analysis|摘要|notes/a.md|split=a,b", "external_id": "inbox-6"})
        ),
        Some("r1|1|analysis|摘要|notes/a.md|split=a,b|inbox-6".to_owned())
    );
    assert_eq!(
        payload(
            "proposal",
            json!({"payload": "r1|1|analysis|摘要|notes/a.md|split=a,b|my-ext", "external_id": "inbox-7"})
        ),
        Some("r1|1|analysis|摘要|notes/a.md|split=a,b|my-ext".to_owned())
    );

    // 其余写命令：载荷原样透传 + 契约声明的幂等后缀
    for (flag, raw) in [
        ("amend", "r1|新标题|新描述|依赖"),
        ("split", "r1|子A|子B"),
        ("merge", "r2,r3|合并标题|合并描述"),
        ("conflict", "r1,r2|说明"),
        ("resolve", "r1,r2|keep_both"),
    ] {
        assert_eq!(
            payload(flag, json!({"payload": raw, "external_id": "inbox-8"})),
            Some(format!("{raw}|{marker}inbox-8")),
            "`--{flag}` 的载荷必须保持改造前形态"
        );
    }
    // 空载荷 → 坏行
    assert_eq!(
        payload("split", json!({"payload": "  ", "external_id": "inbox-9"})),
        None
    );
}
