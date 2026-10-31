//! r2 打包面：`pack.json` 的 `code` 段 + `manifest.json` 的 `code` 数组。
//!
//! 覆盖三件事：① 声明校验（路径规则 / 形状 / 规模）② 三方对账（声明 ↔ manifest
//! 登记 ↔ 实际字节）③ 装载前失败（内容寻址、总量预算先于装载）。正例走
//! `KernelFacade::with_resources_and_modules` 端到端——模块表真的能被入口
//! `require` 到，而不是「校验过了但装不进去」。
//!
//! 缺省兼容（旧单文件 Mod）另有一组：`code` 缺失即 `Ok(None)`，manifest 无
//! `code` 数组时 schema 照过——现有三个 Mod 零迁移的回归锚。

use serde_json::{json, Value as Json};
use ygg_sandbox::driver::KernelFacade;
use ygg_sandbox::modules::{MAX_MODULES, MODULE_EXTENSION};
use ygg_sandbox::pack::{assemble, collect_bytes, entries_for, CodeSpec, MAX_TOTAL_CODE_BYTES};
use ygg_schema::manifest::{parse_manifest, ManifestCodeEntry};
use ygg_schema::validate::sha256_hex;

/// 由 `{entry, modules}` 字面量构造 pack.json 片段（测试便捷面）。
fn code(entry: &str, modules: &[&str]) -> Json {
    json!({ "code": { "entry": entry, "modules": modules } })
}

/// 解析成功路径（断言必须成立）。
fn spec(pack: &Json) -> CodeSpec {
    CodeSpec::parse(pack)
        .expect("code section should parse")
        .expect("code section should be present")
}

/// 解析失败文本（断言可读文案里带路径/字段）。
fn parse_err(pack: &Json) -> String {
    match CodeSpec::parse(pack) {
        Ok(_) => panic!("expected the code section to be rejected"),
        Err(err) => err.to_string(),
    }
}

/// `(path, bytes)` 便捷构造。
fn module(path: &str, source: &str) -> (String, Vec<u8>) {
    (path.to_owned(), source.as_bytes().to_vec())
}

/// 由字节现算 manifest 条目（与打包工具同法）。
fn entries(module_bytes: &[(String, Vec<u8>)]) -> Vec<ManifestCodeEntry> {
    entries_for(module_bytes)
}

#[test]
fn code_section_is_optional_so_single_file_mods_need_no_migration() {
    // 旧 Mod 的 pack.json：没有 code 段 → None（走单文件路径，行为逐字节不变）。
    let legacy = json!({
        "schemaVersion": 1,
        "entries": [{ "id": "ygg:scene/texts", "kind": "scene-texts",
                      "schema": "https://yggdrasill.dev/schemas/v1/scene-texts.json",
                      "path": "data/scenes.json" }],
        "schemaFiles": ["schemas/scene-texts.json"]
    });
    assert!(CodeSpec::parse(&legacy).expect("parse").is_none());

    // 旧 manifest 也没有 code 数组：schema 照过，code() 为空。
    let manifest = parse_manifest(br#"{"schemaVersion": 1, "entries": []}"#)
        .expect("legacy manifest must still parse");
    assert!(manifest.code().is_empty());
}

#[test]
fn code_section_declares_entry_and_modules_verbatim() {
    let parsed = spec(&code("mod.lua", &["src/util.lua", "src/deep/leaf.lua"]));
    assert_eq!(parsed.entry(), "mod.lua");
    assert_eq!(parsed.modules(), ["src/util.lua", "src/deep/leaf.lua"]);

    // 空模块表是合法声明（只有入口即可 require 的范围为零），不是「半声明」。
    let parsed = spec(&code("mod.lua", &[]));
    assert!(parsed.modules().is_empty());
}

#[test]
fn code_section_rejects_unsafe_paths() {
    // 越界 / 通配 / 反斜杠 / 绝对 / 非 .lua / 段含点（与 src/a/b.lua 撞名）/
    // 空段——每条都给出可读错误，且文案里带原始路径。
    let cases: &[(&str, &str)] = &[
        ("../evil.lua", ".."),
        ("src/../evil.lua", ".."),
        ("./src/util.lua", "`.`"),
        ("/abs/util.lua", "relative"),
        ("src\\util.lua", "backslash"),
        ("src/*.lua", "glob"),
        ("src/ut?l.lua", "glob"),
        ("src/util.txt", "module path"),
        ("src/a.b.lua", "module path"),
        ("src//util.lua", "module path"),
        ("", "module path"),
    ];
    for (path, needle) in cases {
        let err = parse_err(&code("mod.lua", &[path]));
        assert!(
            err.contains(path) || err.contains(needle),
            "error for `{path}` must name the path or the rule, got: {err}"
        );
        assert!(
            err.to_lowercase().contains(&needle.to_lowercase()),
            "error for `{path}` must explain the rule `{needle}`, got: {err}"
        );
    }
    // 入口走同一套规则（不能从 entry 侧绕过）。
    let err = parse_err(&code("../mod.lua", &[]));
    assert!(err.contains(".."), "entry path rule, got: {err}");
}

#[test]
fn code_section_rejects_bad_shapes_and_duplicates() {
    // code 不是对象。
    let err = parse_err(&json!({ "code": ["src/util.lua"] }));
    assert!(err.contains("must be an object"), "got: {err}");
    // 缺 entry / entry 非字符串。
    let err = parse_err(&json!({ "code": { "modules": [] } }));
    assert!(err.contains("code.entry"), "got: {err}");
    let err = parse_err(&json!({ "code": { "entry": 1, "modules": [] } }));
    assert!(err.contains("must be a string"), "got: {err}");
    // 缺 modules / modules 非数组 / 元素非字符串。
    let err = parse_err(&json!({ "code": { "entry": "mod.lua" } }));
    assert!(err.contains("code.modules"), "got: {err}");
    let err = parse_err(&json!({ "code": { "entry": "mod.lua", "modules": "src/util.lua" } }));
    assert!(err.contains("array of strings"), "got: {err}");
    let err = parse_err(&json!({ "code": { "entry": "mod.lua", "modules": ["src/util.lua", 7] } }));
    assert!(err.contains("modules[1]"), "got: {err}");
    // 重复声明（同名两份 = 装载序决定谁生效，不静默取后者）。
    let err = parse_err(&code("mod.lua", &["src/util.lua", "src/util.lua"]));
    assert!(err.contains("more than once"), "got: {err}");
    // 入口不得同时登记为模块。
    let err = parse_err(&code("mod.lua", &["mod.lua"]));
    assert!(err.contains("must not also be listed"), "got: {err}");
}

#[test]
fn code_section_rejects_more_modules_than_the_loader_can_register() {
    let paths: Vec<String> = (0..=MAX_MODULES)
        .map(|index| format!("src/m{index}{MODULE_EXTENSION}"))
        .collect();
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    let err = parse_err(&code("mod.lua", &refs));
    assert!(err.contains("code modules"), "got: {err}");
    assert!(err.contains(&MAX_MODULES.to_string()), "got: {err}");
}

#[test]
fn manifest_registry_must_match_the_declaration_both_ways() {
    let spec = spec(&code("mod.lua", &["src/util.lua", "src/extra.lua"]));
    let bytes = vec![
        module("src/util.lua", "return {}"),
        module("src/extra.lua", "return {}"),
    ];

    // ① 忘了跑打包工具：manifest 没有 code 数组。
    let err = assemble(&spec, &[], &bytes).unwrap_err().to_string();
    assert!(err.contains("no `code` array"), "got: {err}");

    // ② 部分登记（manifest 少一条）。
    let partial = entries(&bytes[..1]);
    let err = assemble(&spec, &partial, &bytes).unwrap_err().to_string();
    assert!(err.contains("src/extra.lua"), "got: {err}");

    // ③ 登记了未声明的模块（改了声明没重跑打包）。
    let mut extra = entries(&bytes);
    extra.push(ManifestCodeEntry::new("src/ghost.lua", sha256_hex(b""), 1));
    let err = assemble(&spec, &extra, &bytes).unwrap_err().to_string();
    assert!(err.contains("src/ghost.lua"), "got: {err}");

    // ④ manifest 同一路径登记两次（手改 manifest 的退化路径）。
    let mut duplicated = entries(&bytes);
    duplicated.push(ManifestCodeEntry::new(
        "src/util.lua",
        sha256_hex(b"return {}"),
        b"return {}".len() as u64,
    ));
    let err = assemble(&spec, &duplicated, &bytes)
        .unwrap_err()
        .to_string();
    assert!(err.contains("more than once"), "got: {err}");

    // ⑤ 声明了但调用方没给字节（装载器只认声明清单的字节）。
    let err = assemble(&spec, &entries(&bytes), &bytes[..1])
        .unwrap_err()
        .to_string();
    assert!(err.contains("src/extra.lua"), "got: {err}");

    // ⑥ 给了未声明的字节（不解静默采信）。
    let mut stranger = bytes.clone();
    stranger.push(module("src/stranger.lua", "return {}"));
    let err = assemble(&spec, &entries(&bytes), &stranger)
        .unwrap_err()
        .to_string();
    assert!(err.contains("src/stranger.lua"), "got: {err}");
}

#[test]
fn content_addressing_is_checked_per_module_and_names_the_path() {
    let spec = spec(&code("mod.lua", &["src/util.lua"]));
    let bytes = vec![module("src/util.lua", "return {}")];
    let registered = entries(&bytes);
    assert!(assemble(&spec, &registered, &bytes).is_ok());

    // 字节数不符：先于哈希（廉价预检）。
    let err = assemble(&spec, &registered, &[module("src/util.lua", "return {} ")])
        .unwrap_err()
        .to_string();
    assert!(err.contains("src/util.lua"), "got: {err}");
    assert!(
        err.contains("byteLength") || err.contains("bytes"),
        "got: {err}"
    );

    // 字节数同、内容换：sha256 不符，文案带路径。
    let err = assemble(&spec, &registered, &[module("src/util.lua", "return []")])
        .unwrap_err()
        .to_string();
    assert!(err.contains("src/util.lua"), "got: {err}");
    assert!(err.contains("hash"), "got: {err}");
}

#[test]
fn total_code_budget_fires_before_any_hashing() {
    // 两块各 600 KiB（单块未超 r1 的单模块上限），合计 1.2 MiB > 1 MiB。
    // 断言文案是总量超限而非 sha256 不符——证「先量体积、后算哈希」的顺序。
    let chunk_a = "a".repeat(600 * 1024);
    let chunk_b = "b".repeat(600 * 1024);
    let spec = spec(&code("mod.lua", &["src/a.lua", "src/b.lua"]));
    let bytes = vec![module("src/a.lua", &chunk_a), module("src/b.lua", &chunk_b)];
    // 故意给一份错哈希的登记：若哈希先跑，报的会是内容不符。
    let bogus = vec![
        ManifestCodeEntry::new("src/a.lua", "0".repeat(64), chunk_a.len() as u64),
        ManifestCodeEntry::new("src/b.lua", "0".repeat(64), chunk_b.len() as u64),
    ];
    let err = assemble(&spec, &bogus, &bytes).unwrap_err().to_string();
    assert!(err.contains("totals"), "got: {err}");
    assert!(
        err.contains(&MAX_TOTAL_CODE_BYTES.to_string()),
        "budget limit must be named, got: {err}"
    );
}

#[test]
fn packing_names_missing_module_files_and_hashes_content() {
    let spec = spec(&code("mod.lua", &["src/util.lua", "src/leaf.lua"]));

    // 缺文件：IO 由调用方注入，错误文案带路径（打包工具直接可读呈现）。
    let err = collect_bytes(&spec, |rel| {
        if rel == "src/util.lua" {
            Ok(b"return {}".to_vec())
        } else {
            Err(std::io::Error::from(std::io::ErrorKind::NotFound))
        }
    })
    .unwrap_err()
    .to_string();
    assert!(err.contains("src/leaf.lua"), "got: {err}");

    // 现算条目：路径序（manifest 确定性），哈希与字节数同口径。
    let bytes = collect_bytes(&spec, |rel| {
        Ok(if rel == "src/util.lua" {
            b"return 1".to_vec()
        } else {
            b"return 2".to_vec()
        })
    })
    .expect("collect");
    let registered = entries_for(&bytes);
    assert_eq!(
        registered
            .iter()
            .map(ManifestCodeEntry::path)
            .collect::<Vec<_>>(),
        ["src/leaf.lua", "src/util.lua"]
    );
    assert_eq!(registered[0].sha256(), sha256_hex(b"return 2"));
    assert_eq!(registered[0].byte_length(), 8);
}

#[test]
fn manifest_code_array_survives_parse_and_feeds_assemble() {
    // 打包工具写出的形态 → parse_manifest（schema 校验）→ assemble 直通。
    let spec = spec(&code("mod.lua", &["src/util.lua"]));
    let bytes = vec![module("src/util.lua", "return { count = 0 }")];
    let registered = entries(&bytes);
    let manifest_bytes = serde_json::to_vec(&json!({
        "schemaVersion": 1,
        "entries": [],
        "code": registered
            .iter()
            .map(|entry| json!({
                "path": entry.path(),
                "sha256": entry.sha256(),
                "byteLength": entry.byte_length(),
            }))
            .collect::<Vec<_>>(),
    }))
    .expect("manifest json");
    let manifest = parse_manifest(&manifest_bytes).expect("manifest with code must validate");
    assert_eq!(manifest.code().len(), 1);
    assert!(assemble(&spec, manifest.code(), &bytes).is_ok());
}

#[test]
fn code_mod_boots_and_requires_its_modules() {
    // 端到端：入口 require 模块 → 模块表真的注入了内核（不只是「校验通过」）。
    let mod_source = r#"
local util = require("src.util")
ygg.on("probe", function() ygg.narrate("count=" .. tostring(util.count)) end)
"#;
    let bytes = vec![module("src/util.lua", "return { count = 42 }")];
    let spec = spec(&code("mod.lua", &["src/util.lua"]));
    let registered = entries(&bytes);
    let manifest_bytes = serde_json::to_vec(&json!({
        "schemaVersion": 1,
        "entries": [],
        "code": registered
            .iter()
            .map(|entry| json!({
                "path": entry.path(),
                "sha256": entry.sha256(),
                "byteLength": entry.byte_length(),
            }))
            .collect::<Vec<_>>(),
    }))
    .expect("manifest json");
    let manifest = parse_manifest(&manifest_bytes).expect("manifest");
    let modules = assemble(&spec, manifest.code(), &bytes).expect("assemble");

    let mut kernel =
        KernelFacade::with_resources_and_modules(mod_source, modules, &manifest_bytes, &[], &[])
            .expect("code mod must load");
    let reply: Json = serde_json::from_str(&kernel.dispatch("probe", "0")).expect("json envelope");
    assert_eq!(reply["ok"], json!(true), "dispatch failed: {reply}");
    assert_eq!(reply["narrative"][0], json!("count=42"), "reply: {reply}");
}

#[test]
fn code_mod_with_unregistered_module_is_denied_before_it_runs() {
    // 入口 require 一个没登记的模块：装载期可读拒绝（不是运行期 nil-call）。
    let mod_source = r#"require("src.ghost")"#;
    let bytes = vec![module("src/util.lua", "return {}")];
    let spec = spec(&code("mod.lua", &["src/util.lua"]));
    let modules = assemble(&spec, &entries(&bytes), &bytes).expect("assemble");
    let err = KernelFacade::with_resources_and_modules(
        mod_source,
        modules,
        br#"{"schemaVersion": 1, "entries": []}"#,
        &[],
        &[],
    )
    .err()
    .expect("unregistered module must fail loading")
    .to_string();
    assert!(err.contains("src.ghost"), "got: {err}");
    assert!(err.contains("sandbox denied"), "got: {err}");
}
