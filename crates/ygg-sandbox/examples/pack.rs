//! mod-pack 打包工具（迭代 C1）：manifest 的 sha256 / byteLength 自动生成。
//!
//! std 示例（serde_json / sha2 均为 ygg-sandbox 既有依赖，零新增依赖面）：
//! 读取 `<mod目录>/pack.json` 声明的条目元数据（id / kind / schema / path），
//! 对每个声明的数据文件**从磁盘实际字节**现算 SHA-256（小写 hex）与字节数，
//! 生成 / 覆写 `<mod目录>/manifest.json`（schemaVersion 1 线格式，与内核
//! `ygg_schema::manifest::parse_manifest` 兼容）。
//!
//! 运行：
//!
//! ```bash
//! cargo run -p ygg-sandbox --example pack -- <mod目录>
//! ```
//!
//! pack.json 形态（计划原定 pack.toml，因 `toml` crate 非 workspace 依赖、
//! 零新增依赖纪律改用 JSON——serde_json 已就位，见迭代报告偏差记录）：
//!
//! ```json
//! {
//!   "schemaVersion": 1,
//!   "entries": [
//!     { "id": "ygg:scene/texts", "kind": "scene-texts",
//!       "schema": "https://yggdrasill.dev/schemas/v1/scene-texts.json",
//!       "path": "data/scenes.json" }
//!   ],
//!   "schemaFiles": ["schemas/scene-texts.json"]
//! }
//! ```
//!
//! - `entries[].id / kind / schema / path`：条目元数据（工具只补哈希与长度，
//!   不改写声明面）；
//! - `schemaFiles`：随包分发的 schema 文件清单（工具校验存在性，不进
//!   manifest——schema 文本由装载方按 id 注册，不经 manifest 寻址）。
//!
//! **host 契约的可选前置校验**：`pack.json` 若声明了 `agent` / `inboxRoutes`
//! 段，打包期即按运行期同一套规则解析（`ygg_sandbox::host_contract`：字段必填、
//! 类型 / 形状校验，错误文案与运行时一致）。段缺失**不算错**——本工具服务所有
//! Mod（含不吃 host 契约的冒险类 Mod），但**写坏了**必须在此拦下，别等装载期。

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::exit;

use serde_json::{json, Value as Json};
use sha2::{Digest, Sha256};
use ygg_sandbox::host_contract::{AgentContract, InboxRoutes};

/// pack.json 的条目声明面（工具消费的最小集）。
struct EntryDecl {
    id: String,
    kind: String,
    schema: String,
    path: String,
}

fn main() {
    // ① 参数：<mod目录> 必填。
    let dir = match std::env::args().nth(1) {
        Some(dir) => PathBuf::from(dir),
        None => {
            eprintln!("用法: cargo run -p ygg-sandbox --example pack -- <mod目录>");
            eprintln!("读取 <mod目录>/pack.json，生成 / 覆写 <mod目录>/manifest.json");
            exit(1);
        }
    };
    if !dir.is_dir() {
        fail(format!("目录不存在或不可读: {}", dir.display()));
    }

    // ② pack.json 读取与解析（缺失 / 非法 JSON / 字段缺失均给可读文案）。
    let pack_path = dir.join("pack.json");
    let pack_text = match fs::read_to_string(&pack_path) {
        Ok(text) => text,
        Err(err) => fail(format!(
            "无法读取 {}: {err}（打包工具以 pack.json 为声明入口，\
             请在该 Mod 目录下先创建 pack.json）",
            pack_path.display()
        )),
    };
    let pack: Json = match serde_json::from_str(&pack_text) {
        Ok(value) => value,
        Err(err) => fail(format!("{} 不是合法 JSON: {err}", pack_path.display())),
    };

    // ③ 条目声明解析（逐字段可读校验；id / path 重复拒绝）。
    let entries = parse_entries(&pack, &pack_path);
    if entries.is_empty() {
        fail(format!(
            "{} 的 entries 为空：资源包至少需要一个数据条目",
            pack_path.display()
        ));
    }

    // ④ schemaFiles 存在性校验（不进 manifest；漏带 schema 文件是
    //    装载方 UnknownSchema 的常见根因，打包期提前拦截）。
    if let Some(files) = pack.get("schemaFiles") {
        let files = match files.as_array() {
            Some(files) => files,
            None => fail("pack.json 的 schemaFiles 必须是字符串数组".to_owned()),
        };
        for file in files {
            let file = match file.as_str() {
                Some(s) => s,
                None => fail("pack.json 的 schemaFiles 必须是字符串数组".to_owned()),
            };
            let path = dir.join(file);
            if !path.is_file() {
                fail(format!(
                    "schema 文件缺失: {}（pack.json schemaFiles 声明）",
                    path.display()
                ));
            }
            println!("  schema   {} ✓", file);
        }
    }

    // ④' host 契约前置校验（声明了才校验；写坏了这里就拦下，不等装载期）。
    if pack.get("agent").is_some() {
        match AgentContract::from_pack(&pack) {
            Ok(contract) => println!("  agent    → {}", contract.entry_event),
            Err(message) => fail(message),
        }
    }
    if pack.get("inboxRoutes").is_some() {
        match InboxRoutes::from_pack(&pack) {
            Ok(routes) => println!("  inbox    {} 条路由 ✓", routes.len()),
            Err(message) => fail(message),
        }
    }

    // ⑤ 逐条目：读实际字节 → sha256 小写 hex + byteLength。
    let mut manifest_entries = Vec::with_capacity(entries.len());
    for entry in &entries {
        let bytes = read_entry_bytes(&dir, entry);
        let manifest_entry = json!({
            "id": entry.id,
            "kind": entry.kind,
            "schema": entry.schema,
            "path": entry.path,
            "sha256": hex_sha256(&bytes),
            "byteLength": bytes.len(),
        });
        println!(
            "  entry    {} ← {} ({} bytes)",
            entry.id,
            entry.path,
            bytes.len()
        );
        manifest_entries.push(manifest_entry);
    }

    // ⑥ 生成 manifest.json（schemaVersion 1 线格式；pretty 2 空格缩进）。
    let manifest = json!({
        "schemaVersion": 1,
        "entries": manifest_entries,
    });
    let mut out = serde_json::to_string_pretty(&manifest).expect("manifest serialize");
    out.push('\n');
    let manifest_path = dir.join("manifest.json");
    if let Err(err) = fs::write(&manifest_path, out) {
        fail(format!("写入 {} 失败: {err}", manifest_path.display()));
    }
    println!(
        "manifest 已生成 → {} ({} entries)",
        manifest_path.display(),
        entries.len()
    );
}

/// 解析 entries 声明（字段缺失 / 类型不符 / id 或 path 重复 → 可读错误退出）。
fn parse_entries(pack: &Json, pack_path: &Path) -> Vec<EntryDecl> {
    let raw = match pack.get("entries") {
        Some(Json::Array(items)) => items,
        Some(_) => fail(format!("{} 的 entries 必须是数组", pack_path.display())),
        None => fail(format!(
            "{} 缺少 entries 字段（形如 \
             {{\"entries\": [{{\"id\", \"kind\", \"schema\", \"path\"}}]}}）",
            pack_path.display()
        )),
    };
    let mut entries = Vec::with_capacity(raw.len());
    let mut seen_ids: Vec<String> = Vec::new();
    let mut seen_paths: Vec<String> = Vec::new();
    for (index, item) in raw.iter().enumerate() {
        let field = |name: &str| -> String {
            match item.get(name) {
                Some(Json::String(s)) if !s.is_empty() => s.clone(),
                Some(_) => fail(format!("entries[{index}].{name} 必须是非空字符串")),
                None => fail(format!("entries[{index}] 缺少字段 `{name}`")),
            }
        };
        let entry = EntryDecl {
            id: field("id"),
            kind: field("kind"),
            schema: field("schema"),
            path: field("path"),
        };
        if seen_ids.contains(&entry.id) {
            fail(format!("entries[{index}] 的 id `{}` 重复声明", entry.id));
        }
        if seen_paths.contains(&entry.path) {
            fail(format!(
                "entries[{index}] 的 path `{}` 重复声明",
                entry.path
            ));
        }
        seen_ids.push(entry.id.clone());
        seen_paths.push(entry.path.clone());
        entries.push(entry);
    }
    entries
}

/// 读取条目数据文件（路径越界 / 缺失 / 不可读 → 可读错误退出）。
fn read_entry_bytes(dir: &Path, entry: &EntryDecl) -> Vec<u8> {
    let rel = Path::new(&entry.path);
    // 越界防护：相对路径、组件不含 `..` / 根——条目 path 只允许指向包内文件。
    if rel.is_absolute() || rel.components().any(|c| c == Component::ParentDir) {
        fail(format!(
            "entries 的 path 必须是包内相对路径（不含 `..`），got `{}`",
            entry.path
        ));
    }
    let path = dir.join(rel);
    match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) => fail(format!(
            "读取条目文件失败: {} (id `{}`): {err}",
            path.display(),
            entry.id
        )),
    }
}

/// SHA-256 摘要的小写十六进制编码（与 src/hash.rs 同口径；零依赖面：不引入 hex crate）。
fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// 可读错误退出（打包工具的统一失败面；退出码 1）。
fn fail(message: String) -> ! {
    eprintln!("error: {message}");
    exit(1);
}
