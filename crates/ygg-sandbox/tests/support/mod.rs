//! 多文件 Mod 测试面的共享脚手架（r4）：fixture 定位 / 临时包生成 / manifest 现算。
//!
//! 三个测试目标（`modules_e2e` / `modules_negative` / `modules_lifecycle`）都要
//! 做同一批琐事：找到入库 fixture、把 fixture 复制到临时目录（改文件不能污染
//! 入库件）、按 `pack.json` 的声明现算 `manifest.json`（改一个模块就必须重算
//! 内容寻址，这正是 r2 的打包工具职责）。抄三份就会各自漂移，所以收在这里。
//!
//! 注意：`tests/support/mod.rs` 不会被 cargo 当成独立测试目标（只有 `tests/*.rs`
//! 是），所以各测试文件用 `mod support;` 引入即可。

#![allow(dead_code)] // 每个测试目标只用到其中一部分

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value as Json};
use sha2::{Digest, Sha256};

use ygg_sandbox::driver::KernelFacade;
use ygg_sandbox::modload::ModBundle;

/// 小写十六进制 SHA-256（与 `ygg_schema::validate::sha256_hex` / 打包工具同口径）。
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// 入库 fixture 的目录（`tests/fixtures/multi-script`）。
pub fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/multi-script")
}

/// 本用例独占的临时目录（`tag` 区分用例；先清空再建，重跑不受上次残留影响）。
pub fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ygg-r4-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// 递归复制目录（fixture → 临时目录）。
pub fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create target dir");
    for item in fs::read_dir(from).expect("read source dir") {
        let item = item.expect("dir entry");
        let source = item.path();
        let target = to.join(item.file_name());
        if source.is_dir() {
            copy_dir(&source, &target);
        } else {
            fs::copy(&source, &target).expect("copy file");
        }
    }
}

/// fixture 的临时副本（热载 / 负例要改文件，不能改入库件）。
pub fn copy_fixture(tag: &str) -> PathBuf {
    let dir = temp_dir(tag);
    copy_dir(&fixture_dir(), &dir);
    dir
}

/// 读 JSON（测试脚手架的失败都是"测试环境坏了"，直接 panic 带路径）。
pub fn read_json(path: &Path) -> Json {
    let text =
        fs::read_to_string(path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|err| panic!("parse {}: {err}", path.display()))
}

/// 写 JSON（pretty 形态，方便人读 diff）。
pub fn write_json(path: &Path, value: &Json) {
    let text = serde_json::to_string_pretty(value).expect("serialize json");
    fs::write(path, format!("{text}\n"))
        .unwrap_or_else(|err| panic!("write {}: {err}", path.display()));
}

/// 按 `pack.json` 的声明现算 `manifest.json`（`entries` 按 id 序、`code` 按路径序）。
///
/// 打包工具的测试侧等价物：改了任何模块 / 资源文件都必须重算，否则装载会以
/// 内容寻址不符拒绝——这正是"声明面是唯一事实、字节必须对得上"的回归面。
pub fn reseal(dir: &Path) {
    let pack = read_json(&dir.join("pack.json"));

    let mut resources: Vec<Json> = pack["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|entry| {
            let path = entry["path"].as_str().expect("entries[].path");
            let bytes = fs::read(dir.join(path)).unwrap_or_else(|err| panic!("read {path}: {err}"));
            json!({
                "id": entry["id"],
                "kind": entry["kind"],
                "schema": entry["schema"],
                "path": path,
                "sha256": sha256_hex(&bytes),
                "byteLength": bytes.len(),
            })
        })
        .collect();
    resources.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));

    let mut code: Vec<Json> = pack["code"]["modules"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|item| {
            let path = item.as_str().expect("code.modules[] 必须是字符串");
            let bytes = fs::read(dir.join(path)).unwrap_or_else(|err| panic!("read {path}: {err}"));
            json!({
                "path": path,
                "sha256": sha256_hex(&bytes),
                "byteLength": bytes.len(),
            })
        })
        .collect();
    code.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));

    write_json(
        &dir.join("manifest.json"),
        &json!({ "schemaVersion": 1, "entries": resources, "code": code }),
    );
}

/// `(路径, 源码)` → 拥有所有权的模块清单（调用方不必为字面量加 `to_owned()`）。
pub fn modules(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(path, source)| ((*path).to_owned(), (*source).to_owned()))
        .collect()
}

/// 写一个最小多文件 Mod 包：入口源码 + 模块文件 + `pack.json` + `manifest.json`。///
/// 声明序 = `modules` 顺序（装载序由依赖图决定，与声明序无关——这一点被 e2e
/// 的正向用例 2 显式断言）。资源面留空（本文件只管代码面；需要资源的用例走
/// fixture 副本）。
pub fn write_package(dir: &Path, entry_source: &str, modules: &[(String, String)]) {
    fs::create_dir_all(dir.join("src")).expect("create src dir");
    fs::write(dir.join("mod.lua"), entry_source).expect("write entry");
    for (path, source) in modules {
        let target = dir.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).expect("create module dir");
        }
        fs::write(&target, source).unwrap_or_else(|err| panic!("write {path}: {err}"));
    }
    let declared: Vec<&str> = modules.iter().map(|(path, _)| path.as_str()).collect();
    write_json(
        &dir.join("pack.json"),
        &json!({
            "schemaVersion": 1,
            "entries": [],
            "schemaFiles": [],
            "code": { "entry": "mod.lua", "modules": declared },
        }),
    );
    reseal(dir);
}

// ───────────────────────── 派发与观测面 ─────────────────────────

/// 装载入库 fixture 并返回内核（失败文案带上下文，比裸 unwrap 可读）。
pub fn fixture_kernel() -> KernelFacade {
    fixture_bundle().kernel().expect("fixture 内核构造")
}

/// 装载入库 fixture 的 [`ModBundle`]（读档 / 热载用例要反复重建驱动）。
pub fn fixture_bundle() -> ModBundle {
    ModBundle::load(&fixture_dir()).expect("fixture 必须可装载")
}

/// 派发一个事件并取回叙事行（信封必须 ok，叙事必须是字符串数组）。
#[track_caller]
pub fn narrate(kernel: &mut KernelFacade, event: &str) -> Vec<String> {
    let reply: Json = serde_json::from_str(&kernel.dispatch(event, "0"))
        .unwrap_or_else(|err| panic!("{event} 的回执不是 JSON：{err}"));
    assert_eq!(reply["ok"], json!(true), "{event} 派发失败：{reply}");
    reply["narrative"]
        .as_array()
        .unwrap_or_else(|| panic!("{event} 的 narrative 不是数组：{reply}"))
        .iter()
        .map(|line| {
            line.as_str()
                .unwrap_or_else(|| panic!("叙事行不是字符串：{line:?}"))
                .to_owned()
        })
        .collect()
}

/// `pack.json` 的 `code.modules` 声明序（与装载序对照用）。
pub fn declared_modules() -> Vec<String> {
    let pack = read_json(&fixture_dir().join("pack.json"));
    pack["code"]["modules"]
        .as_array()
        .expect("code.modules")
        .iter()
        .map(|item| item.as_str().expect("module path").to_owned())
        .collect()
}
