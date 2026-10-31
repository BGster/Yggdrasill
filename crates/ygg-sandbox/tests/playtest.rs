//! playtest 回归：golden 走查至结局（贪婪与脚本两路）+ 脚本耗尽停选择点 +
//! 死路失败 + 步数上限失败。迷你 Mod 五件套运行期写入临时目录
//! （manifest 现算对齐 pack.rs 语义——serde_json 默认 BTreeMap 键序即字典序）。

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use sha2::{Digest, Sha256};

use ygg_sandbox::playtest::{playtest, PlaytestOptions, Reached};

/// golden few-shot 种子（仓库路径，绝对定位不依赖 cwd）。
/// （CARGO_MANIFEST_DIR = crates/ygg-sandbox：两级上行到仓库根——与
/// examples/ 内 include_str! 的三级上行不同基，勿照抄。）
fn repo_mod_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/mods/test-adventure")
}

/// 迷你 Mod：单选择点双结局（走查正路径的最小完备素材）。
const MINI_MOD: &str = r#"
local T = ygg.data("ygg:scene/texts")
ygg.narrative.node("node:begin", function()
  ygg.narrate(T.scenes.begin)
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

/// 迷你 Mod：两个选择点串联（脚本耗尽停选择点语义的素材）。
const TWO_CHOICE_MOD: &str = r#"
local T = ygg.data("ygg:scene/texts")
ygg.narrative.node("node:first", function()
  ygg.narrate(T.scenes.begin)
  ygg.kv.set("stage", "first")
  ygg.choice(T.prompts.door, { T.options.enter, T.options.leave })
end)
ygg.narrative.node("node:second", function()
  ygg.narrate(T.scenes.begin)
  ygg.choice(T.prompts.door, { T.options.enter, T.options.leave })
end)
ygg.on("ygg.choice_selected", function(index)
  if ygg.kv.get("stage") == "first" then
    ygg.kv.set("stage", "second")
    ygg.narrative["goto"]("node:second")
  else
    ygg.narrate(T.endings.brave)
  end
end)
ygg.on("event.start", function()
  ygg.narrative["goto"]("node:first")
end)
"#;

/// 死路 Mod：无选择、无结局。
const DEAD_END_MOD: &str = r#"
ygg.on("event.start", function()
  ygg.narrate("路到此为止。")
end)
"#;

/// 循环选择 Mod：choice_selected 永远回到同一选择点（步数上限素材）。
const LOOP_MOD: &str = r#"
local T = ygg.data("ygg:scene/texts")
ygg.narrative.node("node:ask", function()
  ygg.narrate("……")
  ygg.choice(T.prompts.door, { T.options.enter, T.options.leave })
end)
ygg.on("ygg.choice_selected", function(_)
  ygg.narrative["goto"]("node:ask")
end)
ygg.on("event.start", function()
  ygg.narrative["goto"]("node:ask")
end)
"#;

#[test]
fn playtest_golden_reaches_ending_with_default_greedy() {
    let report = playtest(&repo_mod_dir(), &PlaytestOptions::default());
    assert!(
        report.ok,
        "golden 走查应成功：errors={:?} reached={:?}",
        report.errors, report.reached
    );
    assert_eq!(report.reached, Some(Reached::Ending));
    assert!(!report.narrative_sample.is_empty());
    assert!(report.errors.is_empty());
}

#[test]
fn playtest_mini_mod_reaches_ending_on_both_paths() {
    let dir = write_mini_mod("ending", MINI_MOD);
    let greedy = playtest(&dir, &PlaytestOptions::default());
    assert!(
        greedy.ok && greedy.reached == Some(Reached::Ending),
        "errors={:?}",
        greedy.errors
    );
    let scripted = playtest(
        &dir,
        &PlaytestOptions {
            max_steps: 16,
            choices: vec![2],
        },
    );
    assert!(scripted.ok && scripted.reached == Some(Reached::Ending));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn playtest_script_exhaustion_stops_at_choice() {
    let dir = write_mini_mod("stop", TWO_CHOICE_MOD);
    let report = playtest(
        &dir,
        &PlaytestOptions {
            max_steps: 16,
            choices: vec![1], // 脚本长度 1 < 选择点数 2。
        },
    );
    assert!(
        report.ok && report.reached == Some(Reached::Choice),
        "errors={:?} reached={:?}",
        report.errors,
        report.reached
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn playtest_dead_end_fails() {
    let dir = write_mini_mod("deadend", DEAD_END_MOD);
    let report = playtest(&dir, &PlaytestOptions::default());
    assert!(!report.ok);
    assert_eq!(report.reached, Some(Reached::DeadEnd));
    assert!(report.errors.is_empty()); // 死路不是错误信封，是收口判定。
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn playtest_max_steps_limit_fails() {
    let dir = write_mini_mod("loop", LOOP_MOD);
    let report = playtest(
        &dir,
        &PlaytestOptions {
            max_steps: 4,
            choices: Vec::new(),
        },
    );
    assert!(!report.ok);
    assert_eq!(report.reached, Some(Reached::Limit));
    assert_eq!(report.steps, 4);
    let _ = fs::remove_dir_all(&dir);
}

/// 五件套写入临时目录（manifest 现算；serde_json 默认 BTreeMap = 字典序键序）。
fn write_mini_mod(tag: &str, mod_lua: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ygg-playtest-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("data")).expect("mkdir data");
    fs::create_dir_all(dir.join("schemas")).expect("mkdir schemas");

    let scenes = json!({
        "scenes": { "begin": "起点。" },
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
    let schema_text = format!(
        "{}\n",
        serde_json::to_string_pretty(&schema).expect("schema")
    );
    fs::write(dir.join("schemas/scene-texts.json"), &schema_text).expect("write schema");
    fs::write(dir.join("mod.lua"), mod_lua).expect("write mod");

    // manifest 现算（pack.rs 语义：sha256 小写 hex + byteLength，键序 = 字典序）。
    let digest = Sha256::digest(scenes_text.as_bytes());
    let mut sha = String::with_capacity(digest.len() * 2);
    for byte in digest {
        sha.push_str(&format!("{byte:02x}"));
    }
    let manifest = json!({
        "entries": [{
            "byteLength": scenes_text.len(),
            "id": "ygg:scene/texts",
            "kind": "scene-texts",
            "path": "data/scenes.json",
            "schema": "https://yggdrasill.dev/schemas/v1/scene-texts.json",
            "sha256": sha,
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
            // host 契约（必填）：事件名 / 结局前缀 / 叙事约定由 Mod 自己声明。
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
            // 三个保留键必填；一个 kind 都不声明 = 不支持异步 inbox。
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
