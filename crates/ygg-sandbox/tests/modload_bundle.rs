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
local T = ygg.data("ygg:probe/state")
local util = require("src.util")
ygg.kv.set("stage", "begin")
ygg.kv.set("scene", T.tag .. util.suffix)
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

#[test]
fn restore_does_not_carry_kv_across() {
    // ADR-S3-3：帧里只有世界面，KV 不在帧内。读档重建 = 重跑一遍装载，所以 KV 是
    // **装载期写下的值**——既不是拍快照那刻的 7，也不是改完的 99。跨存档的状态
    // 只能写世界组件；把 KV 当存档面用，读档后会静默丢掉一切。
    let dir = write_multi_file_mod("kv-not-persisted");
    let bundle = ModBundle::load(&dir).expect("bundle loads");
    let loaded = Loaded::from_bundle(&bundle).expect("kernel");
    let kv = loaded.kv();
    kv.borrow_mut().set("k", Value::Int(7)).expect("kv set");
    let frame = loaded.snapshot().expect("快照");
    kv.borrow_mut().set("k", Value::Int(99)).expect("kv set");

    let restored = Loaded::restore_from_bundle(&bundle, &frame).expect("restore");
    let kv = restored.kv();
    let kv = kv.borrow();
    assert_eq!(kv.get("k"), None, "帧不含 KV：读档后 7 / 99 都不该在");
    assert_eq!(
        kv.get("stage"),
        Some(&Value::Str(String::from("begin"))),
        "KV 只能是装载期写下的值"
    );
    assert_eq!(
        kv.len(),
        2,
        "帧没把任何键带过来：只剩入口写的 stage / scene"
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

    let probe = json!({ "tag": "multi-script" });
    let probe_text = format!("{}\n", serde_json::to_string_pretty(&probe).expect("probe"));
    fs::write(dir.join("data/probe.json"), &probe_text).expect("write probe");

    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://yggdrasill.dev/schemas/v1/probe.json",
        "type": "object",
        "additionalProperties": false,
        "required": ["tag"],
        "properties": {
            "tag": { "type": "string", "minLength": 1 },
        },
    });
    fs::write(
        dir.join("schemas/probe.json"),
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
            "byteLength": probe_text.len(),
            "id": "ygg:probe/state",
            "kind": "probe",
            "path": "data/probe.json",
            "schema": "https://yggdrasill.dev/schemas/v1/probe.json",
            "sha256": sha256_hex(probe_text.as_bytes()),
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
                "id": "ygg:probe/state",
                "kind": "probe",
                "schema": "https://yggdrasill.dev/schemas/v1/probe.json",
                "path": "data/probe.json",
            }],
            "schemaFiles": ["schemas/probe.json"],
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
