//! 装载口回归：[`ygg_sandbox::modload`] 是 Mod 目录 → 内核的唯一装载语义。
//!
//! 本文件锁的是**没有第二条读盘路径**：多文件 Mod（`pack.json` 带 `code` 段）
//! 只经 `ModBundle` 装载，`load_mod` 与 `Loaded::from_bundle` 对外都只暴露同一份
//! 对账后的模块表——一处读盘实现、一处声明对账。
//!
//! Mod 五件套 + 模块文件全部运行期写入临时目录（manifest 现算，与 pack.rs 同法）。

use std::fs;
use std::path::PathBuf;

use serde_json::json;
use sha2::{Digest, Sha256};

use ygg_core::value::Value;
use ygg_sandbox::modload::{Loaded, ModBundle};

/// 入口：顶层即 `require`（模块表必须在入口 chunk 之前就位）。
const ENTRY: &str = r#"
local T = ygg.data("ygg:scene/texts")
local util = require("src.util")
ygg.kv.set("stage", "begin")
ygg.kv.set("scene", T.scenes.begin .. util.suffix)
"#;

/// 子模块：跨模块共享只经返回值（`local` 不跨 chunk）。
const UTIL: &str = r#"
local util = { suffix = "（模块）" }
return util
"#;

#[test]
fn restore_keeps_the_module_table() {
    // 热载/续局重建若丢模块表，入口的 `require` 会直接失败——读档重建的每一步
    // 都要再跑一遍入口，模块表必须在位。
    let dir = write_multi_file_mod("modload-restore");
    let bundle = ModBundle::load(&dir).expect("bundle loads");
    let mut loaded = Loaded::from_bundle(&bundle).expect("kernel");
    let frame = loaded.snapshot().expect("快照");
    let report = loaded.step(2, 1).expect("推进");
    assert_eq!(report.tick, 2);
    let mut restored = Loaded::restore_from_bundle(&bundle, &frame).expect("restore with modules");
    assert_eq!(
        restored.tick().expect("已挂载"),
        report.tick - 2,
        "读档回到快照时钟"
    );
    // 入口在重建时又跑了一遍：`require("src.util")` 通了才有这条观测值。
    let kv = restored.kv();
    let kv = kv.borrow();
    let Some(Value::Str(scene)) = kv.get("scene") else {
        panic!("读档重建后入口未留痕：模块表丢了");
    };
    assert!(
        scene.contains("（模块）"),
        "初跳文本里必须带模块后缀：{scene}"
    );
    assert!(
        scene.len() > "（模块）".len(),
        "资源文本必须真的读到：{scene}"
    );
    let _ = restored.step(1, 1).expect("读档后可继续推进");
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
