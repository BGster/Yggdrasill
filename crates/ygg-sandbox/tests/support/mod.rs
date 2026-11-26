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

use ygg_core::value::Value;
use ygg_sandbox::modload::{LoadError, Loaded, ModBundle};
use ygg_sandbox::modules::ModuleSet;

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

// ───────────────────────── 装载与观测面 ─────────────────────────

/// 装载入库 fixture 并返回内核（失败文案带上下文，比裸 unwrap 可读）。
pub fn fixture_loaded() -> Loaded {
    Loaded::from_bundle(&fixture_bundle()).expect("fixture 内核构造")
}

/// 内联 Mod 包：入口源码 + 模块表（manifest 取最小合法清单，资源面留空）。
pub fn bundle_of(mod_source: impl Into<String>, set: ModuleSet) -> ModBundle {
    let mut bundle = ModBundle::in_memory(mod_source);
    bundle.modules = set;
    bundle
}

/// [`bundle_of`] → 装载。失败返回 [`LoadError`]，负例据此断言装载期拒绝。
pub fn loaded_of(mod_source: impl Into<String>, set: ModuleSet) -> Result<Loaded, LoadError> {
    Loaded::from_bundle(&bundle_of(mod_source, set))
}

/// 装载入库 fixture 的 [`ModBundle`]（读档 / 热载用例要反复重建驱动）。
pub fn fixture_bundle() -> ModBundle {
    ModBundle::load(&fixture_dir()).expect("fixture 必须可装载")
}

/// 读一个 `probe.<topic>` 观测值（KV 里的字符串），按行拆开。
///
/// 观测面为什么是 KV：模块顶层受 MOD_EFFECT 约束、不得写状态，入口豁免；所以
/// 「装配 + 留痕」只能发生在入口，而入口写进 KV 是测试唯一不需要事件层就能读到
/// 的通道。返回值按 `\n` 拆行，保持「一行一条断言」的比对口径。
#[track_caller]
pub fn narrate(loaded: &Loaded, topic: &str) -> Vec<String> {
    let store = loaded.kv();
    let store = store.borrow();
    let Some(Value::Str(text)) = store.get(topic) else {
        panic!("观测值缺失或不是字符串：{topic}");
    };
    text.split('\n').map(str::to_owned).collect()
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

// ───────────────────────── packages 面（r3） ─────────────────────────

/// 入库夹具包目录（`<repo>/packages/libs/fixture`）。
pub fn fixture_package_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/libs/fixture")
}

/// 入库 packages 夹具 Mod（`tests/fixtures/packages-mod`）。
pub fn packages_mod_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/packages-mod")
}

/// 由包闭包现算 manifest 的 `packages` 段（`(id, path)` 序，与打包工具同法）。
///
/// 与 [`reseal`] 分开：包根是**外部**输入（一个 Mod 可以消费任意包根），
/// 而 `reseal` 只重算 Mod 自己拥有的字节。
pub fn manifest_packages_for(
    pkg_root: &Path,
    spec: &ygg_sandbox::packages::PackagesSpec,
) -> Vec<Json> {
    let resolved =
        ygg_sandbox::packages::resolve(pkg_root, spec).expect("包闭包必须可解析（测试脚手架）");
    resolved
        .packages()
        .iter()
        .map(|package| {
            let files: Vec<Json> = package
                .files()
                .iter()
                .map(|file| {
                    let bytes = fs::read(pkg_root.join(file.rel()))
                        .unwrap_or_else(|err| panic!("read {}: {err}", file.rel()));
                    json!({
                        "path": file.rel(),
                        "sha256": sha256_hex(&bytes),
                        "byteLength": bytes.len(),
                    })
                })
                .collect();
            json!({ "id": package.id(), "files": files })
        })
        .collect()
}

/// [`reseal`] + 把包面条目写进同一个 manifest（改了包文件就必须重算）。
pub fn reseal_with_packages(dir: &Path, pkg_root: &Path) {
    reseal(dir);
    let pack = read_json(&dir.join("pack.json"));
    let spec = ygg_sandbox::packages::PackagesSpec::parse(&pack)
        .expect("pack.json 的 packages 段必须合法")
        .expect("pack.json 必须声明 packages 段");
    let mut manifest = read_json(&dir.join("manifest.json"));
    manifest["packages"] = json!(manifest_packages_for(pkg_root, &spec));
    write_json(&dir.join("manifest.json"), &manifest);
}
