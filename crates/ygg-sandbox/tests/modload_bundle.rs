//! r3 统一装载口回归：[`ygg_sandbox::modload`] 是走查与宿主实跑共用的唯一装载语义。
//!
//! 本文件锁的是**分叉本身**：多文件 Mod（`pack.json` 带 `code` 段）在
//! `playtest` 里曾经不带模块表装载（走查自己读盘、不看 `code`），于是入口的
//! `require("src.util")` 对空模块表得到可读拒绝——「playtest 红、实跑绿」
//! 或者反过来，而两份日志都不指向根因。现在两者都走 `ModBundle`：
//! playtest 能跑到结局、`load_mod` 能派发，就是这条语义真的统一了的证据。
//!
//! Mod 五件套 + 模块文件全部运行期写入临时目录（manifest 现算，与 pack.rs 同法）。

use std::fs;
use std::path::PathBuf;

use serde_json::json;
use sha2::{Digest, Sha256};

use ygg_sandbox::modload::{load_mod, ModBundle};
use ygg_sandbox::playtest::{playtest, PlaytestOptions, Reached};

/// 入口：顶层即 `require`（模块表必须在入口 chunk 之前就位）。
const ENTRY: &str = r#"
local T = ygg.data("ygg:scene/texts")
local util = require("src.util")
ygg.narrative.node("node:begin", function()
  ygg.narrate(T.scenes.begin .. util.suffix)
  ygg.kv.set("stage", "begin")
  ygg.choice(T.prompts.door, { T.options.enter, T.options.leave })
end)
ygg.on("ygg.choice_selected", function(index)
  if index == 1 then
    ygg.narrate(T.endings.brave)
  else
    ygg.narrate(T.endings.cautious)
  end
end)
ygg.on("event.start", function()
  ygg.narrative["goto"]("node:begin")
end)
"#;

/// 子模块：跨模块共享只经返回值（`local` 不跨 chunk）。
const UTIL: &str = r#"
local util = { suffix = "（模块）" }
return util
"#;

#[test]
fn playtest_and_load_mod_share_one_multi_file_loading_path() {
    let dir = write_multi_file_mod("modload-shared");
    let bundle = ModBundle::load(&dir).expect("bundle loads");
    assert_eq!(bundle.modules.len(), 1, "code 段的模块必须进模块表");
    assert!(bundle.mod_source.contains("src.util"), "入口取 code.entry");

    // ① 宿主实跑路径：装载后派发（require 在入口顶层即被解析）。
    let mut kernel = load_mod(&dir).expect("load_mod");
    let reply: serde_json::Value =
        serde_json::from_str(&kernel.dispatch("event.start", "0")).expect("json envelope");
    assert_eq!(reply["ok"], json!(true), "{reply}");
    let narrative = reply["narrative"].to_string();
    assert!(
        narrative.contains("（模块）"),
        "入口必须真的 require 到模块里的值：{narrative}"
    );

    // ② 走查路径：同一份装载语义 → 走到结局（不是「入口 require 失败」）。
    let report = playtest(&dir, &PlaytestOptions::default());
    assert!(
        report.ok && report.reached == Some(Reached::Ending),
        "走查必须与实跑同样装载模块：errors={:?} reached={:?}",
        report.errors,
        report.reached
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn restore_keeps_the_module_table() {
    // 热载/续局重建若丢模块表，跑起来的仍是旧代码或入口 require 直接失败。
    let dir = write_multi_file_mod("modload-restore");
    let bundle = ModBundle::load(&dir).expect("bundle loads");
    let mut kernel = bundle.kernel().expect("kernel");
    let save: serde_json::Value =
        serde_json::from_str(&kernel.dispatch("save", "")).expect("save envelope");
    let save_text = save["save"].to_string();
    let mut restored = bundle
        .restore(save_text.as_bytes())
        .expect("restore with modules");
    let reply: serde_json::Value =
        serde_json::from_str(&restored.dispatch("event.start", "0")).expect("json envelope");
    assert_eq!(reply["ok"], json!(true), "{reply}");
    assert!(
        reply["narrative"].to_string().contains("（模块）"),
        "读档重建后模块表仍须在位：{reply}"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// 多文件 Mod 五件套 + `src/util.lua`（manifest 现算：entries 与 code 两段）。
fn write_multi_file_mod(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ygg-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("data")).expect("mkdir data");
    fs::create_dir_all(dir.join("schemas")).expect("mkdir schemas");
    fs::create_dir_all(dir.join("src")).expect("mkdir src");

    let scenes = json!({
        "scenes": { "begin": "起点" },
        "endings": { "brave": "【结局X】勇闯。", "cautious": "【结局Y】折返。" },
        "prompts": { "door": "推门吗？" },
        "options": { "enter": "推门", "leave": "离开" },
        "beats": {},
        "tuning": {},
    });
    let scenes_text = format!(
        "{}\n",
        serde_json::to_string_pretty(&scenes).expect("scenes")
    );
    fs::write(dir.join("data/scenes.json"), &scenes_text).expect("write scenes");

    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://yggdrasill.dev/schemas/v1/scene-texts.json",
        "type": "object",
        "additionalProperties": false,
        "required": ["scenes", "endings", "prompts", "options", "beats", "tuning"],
        "properties": {
            "scenes": { "type": "object", "additionalProperties": { "type": "string", "minLength": 1 } },
            "endings": { "type": "object", "additionalProperties": { "type": "string", "minLength": 1 } },
            "prompts": { "type": "object", "additionalProperties": { "type": "string", "minLength": 1 } },
            "options": { "type": "object", "additionalProperties": { "type": "string", "minLength": 1 } },
            "beats": { "type": "object", "additionalProperties": { "type": "string", "minLength": 1 } },
            "tuning": { "type": "object", "additionalProperties": { "type": "integer" } },
        },
    });
    fs::write(
        dir.join("schemas/scene-texts.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&schema).expect("schema")
        ),
    )
    .expect("write schema");

    fs::write(dir.join("mod.lua"), ENTRY).expect("write entry");
    fs::write(dir.join("src/util.lua"), UTIL).expect("write module");

    let manifest = json!({
        "entries": [{
            "byteLength": scenes_text.len(),
            "id": "ygg:scene/texts",
            "kind": "scene-texts",
            "path": "data/scenes.json",
            "schema": "https://yggdrasill.dev/schemas/v1/scene-texts.json",
            "sha256": sha256_hex(scenes_text.as_bytes()),
        }],
        // r2：模块的内容寻址（路径序；此处仅一条）。
        "code": [{
            "byteLength": UTIL.len(),
            "path": "src/util.lua",
            "sha256": sha256_hex(UTIL.as_bytes()),
        }],
        "schemaVersion": 1,
    });
    fs::write(
        dir.join("manifest.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&manifest).expect("manifest")
        ),
    )
    .expect("write manifest");

    fs::write(
        dir.join("pack.json"),
        serde_json::to_string_pretty(&json!({
            "schemaVersion": 1,
            "entries": [{
                "id": "ygg:scene/texts",
                "kind": "scene-texts",
                "schema": "https://yggdrasill.dev/schemas/v1/scene-texts.json",
                "path": "data/scenes.json",
            }],
            "schemaFiles": ["schemas/scene-texts.json"],
            // r2：多文件 Mod 的代码声明面（入口不是模块）。
            "code": { "entry": "mod.lua", "modules": ["src/util.lua"] },
            "agent": {
                "entryEvent": "event.start",
                "choiceEvent": "ygg.choice_selected",
                "summaryEvent": "agent.summary",
                "endingPrefix": "【结局",
                "metaKey": "summary",
                "rejectionPrefixes": ["【拒绝】"],
                "replayPrefix": "【重放】",
                "defaultPayload": "0",
            },
            "inboxRoutes": {
                "externalIdField": "external_id",
                "payloadField": "payload",
                "extMarker": "@ext=",
            },
        }))
        .expect("pack"),
    )
    .expect("write pack");

    dir
}

/// 小写十六进制 SHA-256（与 src/hash.rs 同口径；测试侧不引入 hex crate）。
fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
