//! S11 ygg.data 资源面集成测试（9 测，经 `Loaded` 全链路）。
//!
//! 验证面：JSON→Lua 全类型往返（object/array/string/number/bool/null→nil）、
//! 可读错误面（缺失 id / manifest 违规带 pointer / sha 篡改 / 字节数不符 /
//! payload 违规带 file+pointer+code）、预算门（`BudgetExceeded`）、装载期
//! 可见性（Mod 顶层即读资源）、非 string / 空 id 可读拒绝（代决 (c)）、
//! 确定性（资源是数据不是状态：同资源与不同资源内容下世界帧字节 / 状态哈希
//! 逐字节相同）。
//!
//! **口径（M1-E）**：事件层退场后，`ygg.data` 的消费面从 handler 移到了 Mod
//! 入口顶层——资源表在入口 chunk 之前安装（接线序仍然是被测面），观测值经
//! `ygg.kv` 写回、由 `Loaded::kv` 读回。错误面也随之从信封码变成
//! `LoadError::Resources(ResourceError)`：`code::VALIDATION` / `code::KV_LIMIT`
//! 这套稳定码随信封层一起退役，用例改为直接断言错误**变体**与其中的
//! file / pointer / code 字段（证据更贴近契约，比枚举码更严）。
//!
//! 测试侧 schema 文本必须携带 2020-12 `$schema` 元锚（ValidatorSet 构造期
//! 强制）；资源 schema URI 须匹配 manifest pattern
//! `https://yggdrasill.dev/schemas/v1/[a-z0-9-]+[.]json`；资源 id 须匹配
//! `^ygg:[a-z0-9][a-z0-9/._-]*$`；path 须匹配 `^[A-Za-z0-9][A-Za-z0-9/._-]*$`；
//! manifest schemaVersion 恒为 1。

use serde_json::json;
use ygg_sandbox::modload::{LoadError, Loaded, ModBundle};
use ygg_sandbox::resources::{ResourceError, RESOURCES_MAX_BYTES};
use ygg_schema::validate::{sha256_hex, SchemaError};

mod support;

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

/// 合法但内容不同的替代 payload（确定性测试用：证明资源内容不影响世界帧）。
const LOOT_PAYLOAD_ALT: &str = r#"{"name":"axe","count":99,"enabled":false,"tags":["steel"],"meta":{"origin":{"depth":7},"nothing":null}}"#;

/// 全类型读取 Mod：入口顶层读 `ygg.data`，按序把各类型观察值写进 KV。
///
/// 观测面是 KV 而不是叙事：入口 chunk 豁免 MOD_EFFECT，是事件层退场后唯一
/// 仍能在装载期留下痕迹的通道。
const READER_MOD: &str = r#"
local d = ygg.data("ygg:data/act1.loot")
ygg.kv.set("probe",
  "name=" .. d.name
  .. "\ncount=" .. tostring(d.count)
  .. "\nenabled=" .. tostring(d.enabled)
  .. "\ntag2=" .. d.tags[2]
  .. "\ndepth=" .. tostring(d.meta.origin.depth)
  .. "\nnothing=" .. type(d.nothing))
"#;

/// 装载期快照 Mod：Mod **顶层**读 `ygg.data` 进 local，再经 KV 浮出——资源在
/// Mod chunk 之前安装即本测试的接线序证据。
const LOADTIME_MOD: &str = r#"
local loot = ygg.data("ygg:data/act1.loot")
ygg.kv.set("probe", "loadtime=" .. loot.name)
"#;

/// 缺失 id Mod：顶层读不存在的资源 id（加载期失败，文案带 id）。
const MISSING_ID_MOD: &str = r#"
ygg.data("ygg:data/missing.thing")
"#;

/// 不读资源内容的 Mod（确定性测试用：观测值与资源内容无关）。
const CONTENT_FREE_MOD: &str = r#"
local d = ygg.data("ygg:data/act1.loot")
ygg.kv.set("probe", "got=" .. type(d))
"#;

/// 非 string / 空 id Mod：pcall 捕获两处可读错误并逐条写回（S11 审计代决 (c)：
/// 补 non-string / empty id 两断言）。
const BAD_ID_MOD: &str = r#"
local _, err_nonstring = pcall(function() ygg.data(42) end)
local _, err_empty = pcall(function() ygg.data("") end)
ygg.kv.set("probe",
  "nonstring=" .. tostring(err_nonstring)
  .. "\nempty=" .. tostring(err_empty))
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

/// 内存态装载参数：`mod_source` + 一份单条目 manifest + 一份资源。
fn bundle(mod_source: &str, manifest: &str, resources: Vec<(String, Vec<u8>)>) -> ModBundle {
    let mut bundle = ModBundle::in_memory(mod_source);
    bundle.manifest_bytes = manifest.as_bytes().to_vec();
    bundle.schemas = schemas();
    bundle.resources = resources;
    bundle
}

/// 正常装载（失败即 panic，带 `LoadError` 原文）。
#[track_caller]
fn load(mod_source: &str, manifest: &str, resources: Vec<(String, Vec<u8>)>) -> Loaded {
    Loaded::from_bundle(&bundle(mod_source, manifest, resources))
        .unwrap_or_else(|err| panic!("装裁应成功：{err}"))
}

/// 构造失败提取器（`Loaded` 无 Debug，`expect_err` 不可用）。
#[track_caller]
fn load_err(mod_source: &str, manifest: &str, resources: Vec<(String, Vec<u8>)>) -> LoadError {
    match Loaded::from_bundle(&bundle(mod_source, manifest, resources)) {
        Err(err) => err,
        Ok(_) => panic!("expected load failure"),
    }
}

/// 取出资源层错误（其余变体 = 被测面错位，直接失败）。
#[track_caller]
fn resources_err(err: LoadError) -> ResourceError {
    match err {
        LoadError::Resources(err) => err,
        other => panic!("expected LoadError::Resources, got {other:?}"),
    }
}

/// 取出 schema 层错误（manifest 自身校验 / 内容寻址两个入口都归这一层）。
#[track_caller]
fn schema_err(err: ResourceError) -> SchemaError {
    match err {
        ResourceError::Manifest(err) | ResourceError::Content(err) => err,
        other => panic!("expected a schema-layer rejection, got {other:?}"),
    }
}

#[test]
fn ygg_data_returns_every_json_type() {
    let manifest = manifest_one_entry(LOOT_PAYLOAD.as_bytes(), None);
    let loaded = load(READER_MOD, &manifest, resources_with(LOOT_PAYLOAD));
    assert_eq!(
        support::narrate(&loaded, "probe"),
        vec![
            "name=sword",   // string 直通
            "count=3",      // number 整数优先 i64
            "enabled=true", // bool 直通
            "tag2=fire",    // array → 表 1..n
            "depth=2",      // 嵌套 object → 嵌套表
            "nothing=nil",  // null → Lua nil
        ],
        "full-type coverage"
    );
}

#[test]
fn ygg_data_missing_id_fails_the_load_readably() {
    // 事件层退场后，handler 里的运行期错误路径不存在了：顶层读失败即装载失败，
    // 文案仍须点名缺失 id（可读错误面不变）。
    let manifest = manifest_one_entry(LOOT_PAYLOAD.as_bytes(), None);
    let err = load_err(MISSING_ID_MOD, &manifest, resources_with(LOOT_PAYLOAD));
    let LoadError::ModChunk(message) = &err else {
        panic!("expected ModChunk, got {err:?}");
    };
    assert!(
        message.contains("ygg:data/missing.thing"),
        "error must name the missing id: {message}"
    );
}

#[test]
fn manifest_schema_violation_rejected_with_pointer() {
    // sha256 字段塞非法形态（非小写十六进制）→ manifest 自身 schema 拒绝，
    // issues 携带 FR-12 pointer /entries/0/sha256。
    let bad_sha = "Z".repeat(64);
    let manifest = format!(
        r#"{{"schemaVersion": 1, "entries": [{{"id": "{RES_ID}", "kind": "loot", "schema": "{LOOT_SCHEMA_ID}", "path": "{RES_PATH}", "sha256": "{bad_sha}", "byteLength": 5}}]}}"#
    );
    let err = resources_err(load_err(
        READER_MOD,
        &manifest,
        resources_with(LOOT_PAYLOAD),
    ));
    let SchemaError::InvalidManifest { issues, .. } = schema_err(err) else {
        panic!("expected InvalidManifest");
    };
    let pointer = &issues.first().expect("at least one issue").pointer;
    assert_eq!(pointer, "/entries/0/sha256", "details: {issues:?}");
}

#[test]
fn sha_mismatch_tampered_bytes_rejected() {
    // manifest 声明原 payload 摘要，字节被同长篡改 → ContentMismatch 带 file。
    let mut tampered = LOOT_PAYLOAD.to_owned();
    tampered.replace_range(2..3, "X"); // "sword" → "sXord"，等长篡改
    assert_eq!(tampered.len(), LOOT_PAYLOAD.len());
    let manifest = manifest_one_entry(LOOT_PAYLOAD.as_bytes(), None);
    let err = resources_err(load_err(READER_MOD, &manifest, resources_with(&tampered)));
    let SchemaError::ContentMismatch { file, .. } = schema_err(err) else {
        panic!("expected ContentMismatch");
    };
    assert_eq!(file, RES_PATH, "mismatch must name the path");
}

#[test]
fn byte_length_mismatch_rejected() {
    // byteLength 声明比实际多 5 字节（sha 也对不上）→ ByteLengthMismatch
    // 先于哈希被报告（廉价预检优先的字节面证据）。
    let manifest = manifest_one_entry(LOOT_PAYLOAD.as_bytes(), Some(LOOT_PAYLOAD.len() as u64 + 5));
    let err = resources_err(load_err(
        READER_MOD,
        &manifest,
        resources_with(LOOT_PAYLOAD),
    ));
    let SchemaError::ByteLengthMismatch {
        file,
        expected,
        actual,
    } = schema_err(err)
    else {
        panic!("expected ByteLengthMismatch");
    };
    assert_eq!(file, RES_PATH);
    assert_eq!(expected, LOOT_PAYLOAD.len() as u64 + 5, "declared length");
    assert_eq!(actual, LOOT_PAYLOAD.len(), "actual length");
}

#[test]
fn payload_schema_violation_carries_issues() {
    // count 传字符串 → Type 违规；issues 携带 file / pointer / code。
    let bad = r#"{"name":"sword","count":"three","enabled":true,"tags":[],"meta":{}}"#;
    let manifest = manifest_one_entry(bad.as_bytes(), None);
    let err = resources_err(load_err(READER_MOD, &manifest, resources_with(bad)));
    let ResourceError::Payload { file, issues } = err else {
        panic!("expected ResourceError::Payload, got {err:?}");
    };
    assert_eq!(file, RES_PATH, "payload error must carry the file");
    let issue = issues.first().expect("at least one issue");
    assert_eq!(issue.pointer, "/count", "pointer: {issue:?}");
    assert_eq!(issue.code, "Type", "code: {issue:?}");
}

#[test]
fn budget_exceeded_is_reported_before_any_parsing() {
    // manifest + 资源字节总量 > 1 MiB → 预算门（最廉价失败优先，先于一切解析）。
    // fixture 用**非法 JSON manifest**：若预算门不在解析之前，本案会先以
    // Manifest(Json) 失败；实测报 BudgetExceeded 即证序（S11 审计代决 (a) 补强，
    // 与 resources.rs 单元侧 budget_gate_fires_before_any_parsing 同口径）。
    let big = vec![b'x'; RESOURCES_MAX_BYTES]; // 单资源即顶满预算，加 manifest 必超
    let err = resources_err(load_err(
        READER_MOD,
        "not json at all",
        vec![(RES_PATH.to_owned(), big)],
    ));
    let ResourceError::BudgetExceeded { total, limit } = err else {
        panic!("expected BudgetExceeded, got {err:?}");
    };
    assert_eq!(limit, RESOURCES_MAX_BYTES);
    assert!(total > limit, "total {total} must exceed limit {limit}");
}

#[test]
fn resource_visible_at_mod_load_time() {
    // Mod 顶层读 ygg.data（接线序证据：resources::install 先于 Mod chunk）。
    let manifest = manifest_one_entry(LOOT_PAYLOAD.as_bytes(), None);
    let loaded = load(LOADTIME_MOD, &manifest, resources_with(LOOT_PAYLOAD));
    assert_eq!(support::narrate(&loaded, "probe"), vec!["loadtime=sword"]);
}

#[test]
fn ygg_data_non_string_and_empty_id_rejected_readably() {
    // id 非字符串 / 空串 → 可读错误（文案带实参类型 / 空串禁令），经 pcall
    // 捕获后写回 KV（错误面即契约面）。
    let manifest = manifest_one_entry(LOOT_PAYLOAD.as_bytes(), None);
    let loaded = load(BAD_ID_MOD, &manifest, resources_with(LOOT_PAYLOAD));
    // Lua 的 error 自带 traceback（多行），整段拼回一条文本比对——比逐行更稳。
    let text = support::narrate(&loaded, "probe").join("\n");
    assert!(
        text.contains("requires a string resource id") && text.contains("got number"),
        "non-string id rejected readably: {text}"
    );
    assert!(
        text.contains("resource id must not be empty"),
        "empty id rejected readably: {text}"
    );
}

#[test]
fn state_bytes_do_not_depend_on_resource_content() {
    // 三份装载（同输入 a / b，不同资源内容 c）：观测值与世界帧字节、状态哈希
    // 全部一致——资源是数据不是状态（不入世界帧 / 状态哈希的回归证据）。
    let manifest = manifest_one_entry(LOOT_PAYLOAD.as_bytes(), None);
    let manifest_alt = manifest_one_entry(LOOT_PAYLOAD_ALT.as_bytes(), None);
    let a = load(CONTENT_FREE_MOD, &manifest, resources_with(LOOT_PAYLOAD));
    let b = load(CONTENT_FREE_MOD, &manifest, resources_with(LOOT_PAYLOAD));
    let c = load(
        CONTENT_FREE_MOD,
        &manifest_alt,
        resources_with(LOOT_PAYLOAD_ALT),
    );

    let probe_a = support::narrate(&a, "probe");
    assert_eq!(probe_a, vec!["got=table"], "resource visible as a table");
    assert_eq!(
        probe_a,
        support::narrate(&b, "probe"),
        "same inputs, same rows"
    );
    assert_eq!(
        probe_a,
        support::narrate(&c, "probe"),
        "rows must not depend on resource content"
    );

    let frame_a = a.snapshot().expect("frame a");
    let frame_c = c.snapshot().expect("frame c");
    assert_eq!(
        frame_a, frame_c,
        "world frame must not depend on resource content"
    );
    assert_eq!(
        ygg_sandbox::hash::state_hash(&a).expect("hash a"),
        ygg_sandbox::hash::state_hash(&c).expect("hash c"),
        "state hash must not depend on resource content"
    );

    // 资源本身仍是可读面（内容 c 读到的是 c 的 payload）。
    let loot = c.resources().get(RES_ID).expect("resource registered");
    assert_eq!(loot["name"], json!("axe"), "content c is the alt payload");
}
