use super::{AgentContract, Path};
use serde_json::json;

/// `agent` 段的八个字段名（与本模块上一层的 `AGENT_FIELDS` 同集合；这里显式列
/// 出来是为了让「逐个缺字段」用例覆盖完整形状）。
const DECLARED_FIELDS: [&str; 8] = [
    "entryEvent",
    "choiceEvent",
    "summaryEvent",
    "endingPrefix",
    "metaKey",
    "rejectionPrefixes",
    "replayPrefix",
    "defaultPayload",
];

/// 完整合法段（错误用例在此基础上逐字段挖掉）。
fn full_section() -> serde_json::Value {
    json!({
        "entryEvent": "mod.entry",
        "choiceEvent": "mod.choice",
        "summaryEvent": "mod.summary",
        "endingPrefix": "END:",
        "metaKey": "digest",
        "rejectionPrefixes": ["NOPE:", "BAD:"],
        "replayPrefix": "AGAIN:",
        "defaultPayload": "-"
    })
}

/// 段缺失 / 八个字段缺任一 → 硬错误（点名缺失字段 + 形状示例 + 文档定位）。
#[test]
fn missing_section_or_field_is_a_hard_error() {
    let message = AgentContract::from_pack(&json!({"entries": []})).expect_err("必须报错");
    assert!(message.contains("`agent` 段"), "{message}");
    assert!(
        message.contains("\"entryEvent\"") && message.contains("\"defaultPayload\""),
        "应给出完整形状示例：{message}"
    );
    assert!(message.contains("README.md"), "应给出文档定位：{message}");
    // 读不到 pack.json 也是硬错误（不兜底）
    assert!(AgentContract::load(Path::new("/nonexistent/ygg-mod")).is_err());

    for field in DECLARED_FIELDS {
        let mut section = full_section();
        section.as_object_mut().expect("对象").remove(field);
        let message =
            AgentContract::from_pack(&json!({"agent": section})).expect_err("缺字段必须报错");
        assert!(
            message.contains(&format!("`{field}`")),
            "必须点名缺失字段 `{field}`：{message}"
        );
        assert!(message.contains("README.md"), "{message}");
    }
    // 空串 / 非字符串 / 段不是对象 → 同样硬错误
    for broken in [
        json!({"agent": []}),
        json!({"agent": "x"}),
        json!({"agent": {"entryEvent": 7, "choiceEvent": "c", "summaryEvent": "s", "endingPrefix": "E", "metaKey": "m", "rejectionPrefixes": ["R"], "replayPrefix": "P", "defaultPayload": "0"}}),
    ] {
        assert!(AgentContract::from_pack(&broken).is_err(), "{broken}");
    }
}

/// 每个字段的类型 / 空值约束都单独成立（含拒绝前缀数组的三道检查）。
#[test]
fn field_types_and_emptiness_are_enforced() {
    let broken = |field: &str, value: serde_json::Value| {
        let mut section = full_section();
        section[field] = value;
        AgentContract::from_pack(&json!({"agent": section})).expect_err("非法字段必须报错")
    };
    for field in [
        "entryEvent",
        "choiceEvent",
        "summaryEvent",
        "endingPrefix",
        "metaKey",
        "replayPrefix",
    ] {
        let message = broken(field, json!("   "));
        assert!(
            message.contains(&format!("`{field}`")) && message.contains("空串"),
            "空串必须被拦：{message}"
        );
        assert!(broken(field, json!(42)).contains(&format!("`{field}`")));
    }
    // 拒绝前缀：空数组 / 含空串 / 非数组 → 三道都拦
    for value in [json!([]), json!(["OK:", ""]), json!("OK:")] {
        let message = broken("rejectionPrefixes", value);
        assert!(message.contains("`rejectionPrefixes`"), "{message}");
    }
    // 入口载荷允许空串（显式声明「空载荷」是合法意图），但不是字符串即错
    let mut section = full_section();
    section["defaultPayload"] = json!("");
    let contract = AgentContract::from_pack(&json!({"agent": section})).expect("空载荷合法");
    assert_eq!(contract.default_payload, "");
    assert!(broken("defaultPayload", json!(null)).contains("`defaultPayload`"));
}

/// 声明了就用声明值（换 Mod 只改 pack.json，宿主不认字面量）。
#[test]
fn reads_the_eight_declared_fields() {
    let contract =
        AgentContract::from_pack(&json!({"agent": full_section()})).expect("合法 agent 段");
    assert_eq!(contract.entry_event, "mod.entry");
    assert_eq!(contract.choice_event, "mod.choice");
    assert_eq!(contract.summary_event, "mod.summary");
    assert_eq!(contract.ending_prefix, "END:");
    assert_eq!(contract.meta_key, "digest");
    assert_eq!(contract.rejection_prefixes, ["NOPE:", "BAD:"]);
    assert_eq!(contract.replay_prefix, "AGAIN:");
    assert_eq!(contract.default_payload, "-");
    // 判定面也只认声明值
    assert!(contract.is_rejection("NOPE: 动作越阶"));
    assert!(contract.is_rejection("BAD: 载荷非法"));
    assert!(!contract.is_rejection("动作越阶"));
    assert!(contract.is_replay("AGAIN: 同键重放"));
    assert!(!contract.is_replay("NOPE: 动作越阶"));
}
