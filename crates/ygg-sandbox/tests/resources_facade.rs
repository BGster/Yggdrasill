//! S11 ygg.data 资源面集成测试（10 测，经 KernelFacade 全链路）。
//!
//! 验证面：JSON→Lua 全类型往返（object/array/string/number/bool/null→nil）、
//! 可读错误面（缺失 id / manifest 违规带 pointer / sha 篡改 / 字节数不符 /
//! payload 违规带 file+pointer+code）、KV_LIMIT 预留码激活（预算超限）、
//! 装载期可见性（Mod 顶层读资源）、非 string / 空 id 可读拒绝（代决 (c)）、
//! 确定性（双 facade 同事件序 reply 字节
//! 一致；不同资源内容下 save/state/canonical 字节不变——资源是数据不是状态）。
//!
//! 测试侧 schema 文本必须携带 2020-12 `$schema` 元锚（ValidatorSet 构造期
//! 强制）；资源 schema URI 须匹配 manifest pattern
//! `https://yggdrasill.dev/schemas/v1/[a-z0-9-]+[.]json`；资源 id 须匹配
//! `^ygg:[a-z0-9][a-z0-9/._-]*$`；path 须匹配 `^[A-Za-z0-9][A-Za-z0-9/._-]*$`；
//! manifest schemaVersion 恒为 1。

use serde_json::{json, Value as Json};
use ygg_sandbox::driver::{code, KernelError, KernelFacade};
use ygg_sandbox::resources::RESOURCES_MAX_BYTES;
use ygg_schema::validate::sha256_hex;

/// 测试资源 schema（2020-12 锚 + 覆盖五种 JSON 类型的字段面）。
const LOOT_SCHEMA_ID: &str = "https://yggdrasill.dev/schemas/v1/loot.json";
const LOOT_SCHEMA: &str = r#"{
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "$id": "https://yggdrasill.dev/schemas/v1/loot.json",
    "type": "object",
    "additionalProperties": false,
    "required": ["name", "count", "enabled", "tags", "meta"],
    "properties": {
        "name": { "type": "string" },
        "count": { "type": "integer" },
        "enabled": { "type": "boolean" },
        "tags": { "type": "array", "items": { "type": "string" } },
        "meta": { "type": "object" }
    }
}"#;

/// 资源 id / path（均匹配 manifest pattern）。
const RES_ID: &str = "ygg:data/act1.loot";
const RES_PATH: &str = "data/act1.loot.json";

/// 全类型覆盖 payload：object（嵌套 object）/ array / string / number（整数）
/// / bool / null。
const LOOT_PAYLOAD: &str = r#"{"name":"sword","count":3,"enabled":true,"tags":["iron","fire"],"meta":{"origin":{"depth":2},"nothing":null}}"#;

/// 合法但内容不同的替代 payload（确定性测试用：证明资源内容不影响 reply）。
const LOOT_PAYLOAD_ALT: &str = r#"{"name":"axe","count":99,"enabled":false,"tags":["steel"],"meta":{"origin":{"depth":7},"nothing":null}}"#;

/// 全类型读取 Mod：handler 内读 ygg.data 并按序叙述各类型观察值。
const READER_MOD: &str = r#"
ygg.on("event.peek", function()
  local d = ygg.data("ygg:data/act1.loot")
  ygg.narrate("name=" .. d.name)
  ygg.narrate("count=" .. tostring(d.count))
  ygg.narrate("enabled=" .. tostring(d.enabled))
  ygg.narrate("tag2=" .. d.tags[2])
  ygg.narrate("depth=" .. tostring(d.meta.origin.depth))
  ygg.narrate("nothing=" .. type(d.nothing))
end)
"#;

/// 装载期快照 Mod：Mod **顶层**读 ygg.data 进 local（资源在 Mod chunk 前
/// 安装即本测试的接线序证据），handler 消费 local。
const LOADTIME_MOD: &str = r#"
local loot = ygg.data("ygg:data/act1.loot")
ygg.on("event.peek", function()
  ygg.narrate("loadtime=" .. loot.name)
end)
"#;

/// 缺失 id Mod：读不存在的资源 id（可读错误文案带 id）。
const MISSING_ID_MOD: &str = r#"
ygg.on("event.peek", function()
  ygg.data("ygg:data/missing.thing")
end)
"#;

/// 不读资源内容的 Mod（确定性测试用：reply 与资源内容无关）。
const CONTENT_FREE_MOD: &str = r#"
ygg.on("event.peek", function()
  local d = ygg.data("ygg:data/act1.loot")
  ygg.narrate("got=" .. type(d))
end)
"#;

fn schemas() -> Vec<(String, String)> {
    vec![(LOOT_SCHEMA_ID.to_owned(), LOOT_SCHEMA.to_owned())]
}

/// manifest JSON：单条目指向 `path` 的资源字节（sha/byteLength 按 `bytes` 现算；
/// `byte_length_override` 供字节面负例显式声明错误长度）。
fn manifest_one_entry(bytes: &[u8], byte_length_override: Option<u64>) -> String {
    let byte_length = byte_length_override.unwrap_or(bytes.len() as u64);
    format!(
        r#"{{"schemaVersion": 1, "entries": [{{"id": "{RES_ID}", "kind": "loot", "schema": "{LOOT_SCHEMA_ID}", "path": "{RES_PATH}", "sha256": "{}", "byteLength": {byte_length}}}]}}"#,
        sha256_hex(bytes)
    )
}

fn resources_with(payload: &str) -> Vec<(String, Vec<u8>)> {
    vec![(RES_PATH.to_owned(), payload.as_bytes().to_vec())]
}

/// 构造失败提取器（KernelFacade 无 Debug，`expect_err` 不可用）。
fn kernel_err(built: Result<KernelFacade, KernelError>) -> KernelError {
    match built {
        Err(err) => err,
        Ok(_) => panic!("expected construction failure"),
    }
}

fn parse(reply: &str) -> Json {
    serde_json::from_str(reply).expect("reply must be valid JSON")
}

#[test]
fn resources_load_and_ygg_data_returns_table() {
    let manifest = manifest_one_entry(LOOT_PAYLOAD.as_bytes(), None);
    let mut kernel = KernelFacade::with_resources(
        READER_MOD,
        manifest.as_bytes(),
        &schemas(),
        &resources_with(LOOT_PAYLOAD),
    )
    .expect("kernel with resources");
    let reply = kernel.dispatch("event.peek", "0");
    let parsed = parse(&reply);
    assert_eq!(parsed["ok"], json!(true), "reply: {reply}");
    assert_eq!(
        parsed["narrative"],
        json!([
            "name=sword",   // string 直通
            "count=3",      // number 整数优先 i64
            "enabled=true", // bool 直通
            "tag2=fire",    // array → 表 1..n
            "depth=2",      // 嵌套 object → 嵌套表
            "nothing=nil",  // null → Lua nil
        ]),
        "full-type coverage"
    );
}

#[test]
fn ygg_data_missing_id_errors_readably() {
    let manifest = manifest_one_entry(LOOT_PAYLOAD.as_bytes(), None);
    let mut kernel = KernelFacade::with_resources(
        MISSING_ID_MOD,
        manifest.as_bytes(),
        &schemas(),
        &resources_with(LOOT_PAYLOAD),
    )
    .expect("kernel with resources");
    let reply = kernel.dispatch("event.peek", "0");
    let parsed = parse(&reply);
    assert_eq!(parsed["ok"], json!(false), "reply: {reply}");
    assert_eq!(parsed["error"]["code"], json!(code::VALIDATION));
    let message = parsed["error"]["message"].as_str().expect("message");
    assert!(
        message.contains("ygg:data/missing.thing"),
        "error must name the missing id: {message}"
    );
}

#[test]
fn manifest_schema_violation_rejected_with_pointer() {
    // sha256 字段塞非法形态（非小写十六进制）→ manifest 自身 schema 拒绝，
    // details 携带 FR-12 pointer /entries/0/sha256。
    let bad_sha = "Z".repeat(64);
    let manifest = format!(
        r#"{{"schemaVersion": 1, "entries": [{{"id": "{RES_ID}", "kind": "loot", "schema": "{LOOT_SCHEMA_ID}", "path": "{RES_PATH}", "sha256": "{bad_sha}", "byteLength": 5}}]}}"#
    );
    let err = kernel_err(KernelFacade::with_resources(
        READER_MOD,
        manifest.as_bytes(),
        &schemas(),
        &resources_with(LOOT_PAYLOAD),
    ));
    let KernelError::InvalidManifest { details } = &err else {
        panic!("expected InvalidManifest, got {err:?}");
    };
    assert!(details.contains("/entries/0/sha256"), "details: {details}");
    assert_eq!(err.code(), code::VALIDATION);
}

#[test]
fn sha_mismatch_tampered_bytes_rejected() {
    // manifest 声明原 payload 摘要，字节被同长篡改 → ContentMismatch 带 path。
    let mut tampered = LOOT_PAYLOAD.to_owned();
    tampered.replace_range(2..3, "X"); // "sword" → "sXord"，等长篡改
    assert_eq!(tampered.len(), LOOT_PAYLOAD.len());
    let manifest = manifest_one_entry(LOOT_PAYLOAD.as_bytes(), None);
    let err = kernel_err(KernelFacade::with_resources(
        READER_MOD,
        manifest.as_bytes(),
        &schemas(),
        &resources_with(&tampered),
    ));
    let KernelError::ContentMismatch { details } = &err else {
        panic!("expected ContentMismatch, got {err:?}");
    };
    assert!(
        details.contains(RES_PATH),
        "details must name path: {details}"
    );
    assert_eq!(err.code(), code::VALIDATION);
}

#[test]
fn byte_length_mismatch_rejected() {
    // byteLength 声明比实际多 5 字节（sha 也对不上）→ ByteLengthMismatch
    // 先于哈希被报告（廉价预检优先的字节面证据）。
    let manifest = manifest_one_entry(LOOT_PAYLOAD.as_bytes(), Some(LOOT_PAYLOAD.len() as u64 + 5));
    let err = kernel_err(KernelFacade::with_resources(
        READER_MOD,
        manifest.as_bytes(),
        &schemas(),
        &resources_with(LOOT_PAYLOAD),
    ));
    let KernelError::ContentMismatch { details } = &err else {
        panic!("expected ContentMismatch, got {err:?}");
    };
    assert!(
        details.contains("expected") && details.contains("bytes"),
        "byte-level mismatch message: {details}"
    );
    assert!(
        details.contains(RES_PATH),
        "details must name path: {details}"
    );
}

#[test]
fn payload_schema_violation_carries_issues() {
    // count 传字符串 → Type 违规；details 携带 file / pointer / code。
    let bad = r#"{"name":"sword","count":"three","enabled":true,"tags":[],"meta":{}}"#;
    let manifest = manifest_one_entry(bad.as_bytes(), None);
    let err = kernel_err(KernelFacade::with_resources(
        READER_MOD,
        manifest.as_bytes(),
        &schemas(),
        &resources_with(bad),
    ));
    let KernelError::InvalidManifest { details } = &err else {
        panic!("expected InvalidManifest, got {err:?}");
    };
    assert!(
        details.contains(RES_PATH),
        "details must carry file: {details}"
    );
    assert!(
        details.contains("/count"),
        "details must carry pointer: {details}"
    );
    assert!(
        details.contains("Type"),
        "details must carry code: {details}"
    );
    assert_eq!(err.code(), code::VALIDATION);
}

#[test]
fn budget_exceeded_activates_kv_limit() {
    // manifest + 资源字节总量 > 1 MiB → 预算门（最廉价失败优先，先于一切解析）
    // → KernelError::ResourceBudget → KV_LIMIT 预留码激活。fixture 用**非法
    // JSON manifest**：若预算门不在解析之前，本案会先以 Manifest(Json) 失败；
    // 实测报 ResourceBudget 即证序（S11 审计代决 (a) 补强，与 resources.rs
    // 单元侧 budget_gate_fires_before_any_parsing 同口径）。
    let big = vec![b'x'; RESOURCES_MAX_BYTES]; // 单资源即顶满预算，加 manifest 必超
    let manifest = "not json at all";
    let err = kernel_err(KernelFacade::with_resources(
        READER_MOD,
        manifest.as_bytes(),
        &schemas(),
        &[(RES_PATH.to_owned(), big)],
    ));
    let KernelError::ResourceBudget { total, limit } = &err else {
        panic!("expected ResourceBudget, got {err:?}");
    };
    assert_eq!(*limit, RESOURCES_MAX_BYTES);
    assert!(*total > *limit);
    assert_eq!(err.code(), code::KV_LIMIT);
}

#[test]
fn resource_visible_at_mod_load_time() {
    // Mod 顶层读 ygg.data（接线序证据：resources::install 先于 Mod chunk）。
    let manifest = manifest_one_entry(LOOT_PAYLOAD.as_bytes(), None);
    let mut kernel = KernelFacade::with_resources(
        LOADTIME_MOD,
        manifest.as_bytes(),
        &schemas(),
        &resources_with(LOOT_PAYLOAD),
    )
    .expect("kernel with resources");
    let reply = kernel.dispatch("event.peek", "0");
    let parsed = parse(&reply);
    assert_eq!(
        parsed["narrative"],
        json!(["loadtime=sword"]),
        "reply: {reply}"
    );
}

// 非 string / 空 id Mod：pcall 捕获两处可读错误并逐条叙述（S11 审计代决 (c)：
// 补 non-string / empty id 两断言）。
const BAD_ID_MOD: &str = r#"
ygg.on("event.peek", function()
  local _, err_nonstring = pcall(function() ygg.data(42) end)
  ygg.narrate("nonstring=" .. tostring(err_nonstring))
  local _, err_empty = pcall(function() ygg.data("") end)
  ygg.narrate("empty=" .. tostring(err_empty))
end)
"#;

#[test]
fn ygg_data_non_string_and_empty_id_rejected_readably() {
    // id 非字符串 / 空串 → 可读错误（文案带实参类型 / 空串禁令），经 pcall
    // 捕获后以叙述面浮出（错误面即契约面；不触发信封错误路径）。
    let manifest = manifest_one_entry(LOOT_PAYLOAD.as_bytes(), None);
    let mut kernel = KernelFacade::with_resources(
        BAD_ID_MOD,
        manifest.as_bytes(),
        &schemas(),
        &resources_with(LOOT_PAYLOAD),
    )
    .expect("kernel with resources");
    let reply = kernel.dispatch("event.peek", "0");
    let parsed = parse(&reply);
    assert_eq!(
        parsed["ok"],
        json!(true),
        "pcall must contain both errors: {reply}"
    );
    let narrative: Vec<String> = parsed["narrative"]
        .as_array()
        .expect("narrative array")
        .iter()
        .map(|v| v.as_str().expect("string").to_owned())
        .collect();
    assert_eq!(narrative.len(), 2, "both branches narrated: {narrative:?}");
    assert!(
        narrative[0].contains("requires a string resource id")
            && narrative[0].contains("got number"),
        "non-string id rejected readably: {}",
        narrative[0]
    );
    assert!(
        narrative[1].contains("resource id must not be empty"),
        "empty id rejected readably: {}",
        narrative[1]
    );
}

#[test]
fn determinism_identical_dispatch_replies() {
    // 双 facade（同输入）+ 第三 facade（不同资源内容）：同事件序 reply
    // 字节一致；save/state/canonical 字节与资源内容无关（资源是数据不是
    // 状态——不入 canonical store/存档/状态哈希的回归证据）。
    let manifest = manifest_one_entry(LOOT_PAYLOAD.as_bytes(), None);
    let manifest_alt = manifest_one_entry(LOOT_PAYLOAD_ALT.as_bytes(), None);
    let mut a = KernelFacade::with_resources(
        CONTENT_FREE_MOD,
        manifest.as_bytes(),
        &schemas(),
        &resources_with(LOOT_PAYLOAD),
    )
    .expect("kernel a");
    let mut b = KernelFacade::with_resources(
        CONTENT_FREE_MOD,
        manifest.as_bytes(),
        &schemas(),
        &resources_with(LOOT_PAYLOAD),
    )
    .expect("kernel b");
    let mut c = KernelFacade::with_resources(
        CONTENT_FREE_MOD,
        manifest_alt.as_bytes(),
        &schemas(),
        &resources_with(LOOT_PAYLOAD_ALT),
    )
    .expect("kernel c (different resource content)");
    for (cmd, payload) in [("event.peek", "0"), ("save", ""), ("state", "")] {
        let (ra, rb, rc) = (
            a.dispatch(cmd, payload),
            b.dispatch(cmd, payload),
            c.dispatch(cmd, payload),
        );
        assert_eq!(
            ra, rb,
            "identical inputs must yield byte-identical replies ({cmd})"
        );
        assert_eq!(
            ra, rc,
            "replies must not depend on resource content ({cmd})"
        );
    }
    assert_eq!(
        a.canonical_state_bytes().expect("canonical bytes a"),
        c.canonical_state_bytes().expect("canonical bytes c"),
        "canonical bytes must not depend on resource content"
    );
}
