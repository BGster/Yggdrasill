//! 确定性静态扫描：禁浮点 / 禁时钟 / 禁无序容器 / 依赖白名单（sim-kernel §3.2）。
//!
//! # 为什么先剥注释再扫
//!
//! 与 assetgen 的「注释里也不写」相反：本 crate 的可读性恰恰靠注释解释「为什么禁浮点」
//! （`fixed.rs` / `lib.rs` 的 `//!` 会写 `f64` 字样）。严格版会把解释禁令的文档判成违规，
//! 反而鼓励删注释。剥注释后规则更准：**代码里一个 float token 都不能有，注释里必须解释
//! 为什么没有**。这条差异写进 `lib.rs` 头注（同一分支），下一位执行者不必重推。
//!
//! # 扫描分母必须打印
//!
//! 「命中 0」在扫描了 0 个文件时同样成立。所以每条用例都打印实际扫描的文件数，
//! 并且断言它 ≥ 10（当前 13 个模块，少一个都说明文件枚举坏了）。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `src/**/*.rs` 全部文件（排序后返回，便于稳定输出）。
fn source_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect(&crate_dir().join("src"), &mut files);
    files.sort();
    assert!(files.len() >= 10, "扫描分母异常小：{} 个文件", files.len());
    files
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()));
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// 去掉行注释与块注释（保留字符串字面量的内容与换行）。
///
/// 只处理本 crate 实际使用的写法：`//`、`///`、`//!`、`/* */`；字符串按 `"` 定界并支持
/// `\"` 转义。不做完整的 Rust 词法分析——扫描器要的是「不漏报」，而这个子集覆盖了
/// 本 crate 的全部源码形态（`tests/determinism_scan.rs` 自己不在扫描分母里）。
fn strip_comments(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut index = 0usize;
    let mut in_string = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            out.push(byte as char);
            if byte == b'\\' && index + 1 < bytes.len() {
                out.push(bytes[index + 1] as char);
                index += 2;
                continue;
            }
            if byte == b'"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if byte == b'"' {
            in_string = true;
            out.push('"');
            index += 1;
            continue;
        }
        if byte == b'/' && index + 1 < bytes.len() && bytes[index + 1] == b'/' {
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            continue;
        }
        if byte == b'/' && index + 1 < bytes.len() && bytes[index + 1] == b'*' {
            index += 2;
            while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/') {
                index += 1;
            }
            index = (index + 2).min(bytes.len());
            continue;
        }
        out.push(byte as char);
        index += 1;
    }
    out
}

/// 全词匹配（`contains_word(code, "f64")` 不会命中 `f64x`）。
fn contains_word(code: &str, word: &str) -> bool {
    let bytes = code.as_bytes();
    let needle = word.as_bytes();
    let mut start = 0usize;
    while let Some(found) = code[start..].find(word) {
        let at = start + found;
        let before_ok = at == 0 || !is_word_byte(bytes[at - 1]);
        let after = at + needle.len();
        let after_ok = after >= bytes.len() || !is_word_byte(bytes[after]);
        if before_ok && after_ok {
            return true;
        }
        start = at + needle.len();
    }
    false
}

const fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// 位串搬运面（sim-kernel I2 修宪）的路径后缀：全 crate 唯一的 `f64` 类型词豁免文件。
const BITS_FACE: &str = "src/bits.rs";

fn is_bits_face(path: &Path) -> bool {
    path.ends_with(BITS_FACE)
}

/// 把字符串字面量的内容替换成占位（算术字符检查只看代码，不看错误消息文本）。
/// 输入必须是 `strip_comments` 的产物（转义对 `\"` 已被原样保留，这里跳两字节）。
fn strip_strings(code: &str) -> String {
    let bytes = code.as_bytes();
    let mut out = String::with_capacity(code.len());
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'"' {
            out.push('"');
            index += 1;
            while index < bytes.len() && bytes[index] != b'"' {
                if bytes[index] == b'\\' && index + 1 < bytes.len() {
                    index += 2;
                } else {
                    index += 1;
                }
            }
            if index < bytes.len() {
                out.push('"');
                index += 1;
            }
            continue;
        }
        out.push(byte as char);
        index += 1;
    }
    out
}

/// bits.rs 白名单面的算术禁令：剥注释与字符串、去掉返回箭头 `->` 后，
/// 不得出现 `+ - * / %` 五个运算符字符（位运算与比较不受限——它们不是浮点运算，
/// 见 sim-kernel I2 与 `src/bits.rs` 头注）。
fn arithmetic_findings(code: &str) -> Vec<String> {
    let no_strings = strip_strings(code);
    let no_arrows = no_strings.replace("->", " ");
    let mut findings = Vec::new();
    for (at, ch) in no_arrows.char_indices() {
        if matches!(ch, '+' | '-' | '*' | '/' | '%') {
            findings.push(format!("算术运算符字符 {ch:?} at byte {at}"));
        }
    }
    findings
}

/// 浮点字面量 / 浮点转换的命中列表（空 = 干净）。
fn float_findings(code: &str) -> Vec<String> {
    let mut findings = Vec::new();
    for word in ["f32", "f64"] {
        if contains_word(code, word) {
            findings.push(format!("出现 {word}"));
        }
    }
    for word in ["powf", "sqrt", "hypot", "floorf", "ceilf"] {
        if contains_word(code, word) {
            findings.push(format!("出现浮点函数 {word}"));
        }
    }
    // 小数或指数形态的数值字面量
    for line in code.lines() {
        let bytes = line.as_bytes();
        let mut index = 0usize;
        while index < bytes.len() {
            if !bytes[index].is_ascii_digit() {
                index += 1;
                continue;
            }
            let prev_ok = index == 0 || !is_word_byte(bytes[index - 1]);
            if !prev_ok {
                index += 1;
                continue;
            }
            // 收集整数部分
            let mut cursor = index;
            while cursor < bytes.len() && (bytes[cursor].is_ascii_digit() || bytes[cursor] == b'_')
            {
                cursor += 1;
            }
            // 小数：`.digit`
            if cursor + 1 < bytes.len()
                && bytes[cursor] == b'.'
                && bytes[cursor + 1].is_ascii_digit()
            {
                findings.push(format!(
                    "浮点字面量：{}",
                    &line[index..(cursor + 2).min(line.len())]
                ));
            } else if cursor < bytes.len() && (bytes[cursor] == b'e' || bytes[cursor] == b'E') {
                // 指数：`1e9` / `1E-9`
                let mut probe = cursor + 1;
                if probe < bytes.len() && (bytes[probe] == b'+' || bytes[probe] == b'-') {
                    probe += 1;
                }
                if probe < bytes.len() && bytes[probe].is_ascii_digit() {
                    findings.push(format!(
                        "指数字面量：{}",
                        &line[index..(probe + 1).min(line.len())]
                    ));
                }
            }
            index = cursor.max(index + 1);
        }
    }
    findings
}

/// R1′：模拟态禁浮点（`src/**/*.rs`，剥注释后）。
///
/// 位串搬运面 `src/bits.rs`（sim-kernel I2）是唯一的 `f64`/`f32` 类型词豁免文件：
/// 它只做 pack/unpack/is_canonical/order_key 的位搬运，豁免面内浮点字面量与浮点函数
/// 照旧禁，且另断言算术运算符字符零命中（`arithmetic_findings`）。
#[test]
fn r1_no_float_in_simulation_code() {
    let files = source_files();
    let mut findings = Vec::new();
    let mut bits_faces = 0usize;
    for path in &files {
        let text = std::fs::read_to_string(path).expect("读源码");
        let code = strip_comments(&text);
        let whitelisted = is_bits_face(path);
        if whitelisted {
            bits_faces += 1;
        }
        for finding in float_findings(&code) {
            // 豁免面只放行 f64/f32 类型词本身（pack/unpack 签名与 to_bits/from_bits）；
            // 「浮点字面量」「指数字面量」「浮点函数」三类照旧禁。
            if whitelisted && (finding == "出现 f64" || finding == "出现 f32") {
                continue;
            }
            findings.push(format!("{}: {finding}", path.display()));
        }
        if whitelisted {
            for finding in arithmetic_findings(&code) {
                findings.push(format!("{}: {finding}", path.display()));
            }
        }
    }
    println!(
        "determinism_scan.r1_prime files={} bits_faces={bits_faces} hits={}",
        files.len(),
        findings.len()
    );
    assert_eq!(bits_faces, 1, "f64 豁免面必须恰好一个文件：{BITS_FACE}");
    assert!(findings.is_empty(), "R1′ 命中：{findings:#?}");
}

/// R2：不读时钟 / 不起线程 / 不起进程（wasm 可见性也依赖这一条）。
#[test]
fn r2_no_clock_or_thread_or_process() {
    let files = source_files();
    let banned = [
        "Instant",
        "SystemTime",
        "std::time",
        "chrono",
        "std::thread",
        "std::process",
    ];
    let mut findings = Vec::new();
    for path in &files {
        let text = std::fs::read_to_string(path).expect("读源码");
        let code = strip_comments(&text);
        for needle in banned {
            if code.contains(needle) {
                findings.push(format!("{}: {needle}", path.display()));
            }
        }
    }
    println!(
        "determinism_scan.r2 files={} hits={}",
        files.len(),
        findings.len()
    );
    assert!(findings.is_empty(), "R2 命中：{findings:#?}");
}

/// R3：迭代序不得来自无序容器。
#[test]
fn r3_no_unordered_containers() {
    let files = source_files();
    let banned = ["HashMap", "HashSet", "DefaultHasher", "RandomState"];
    let mut findings = Vec::new();
    for path in &files {
        let text = std::fs::read_to_string(path).expect("读源码");
        let code = strip_comments(&text);
        for needle in banned {
            if code.contains(needle) {
                findings.push(format!("{}: {needle}", path.display()));
            }
        }
    }
    println!(
        "determinism_scan.r3 files={} hits={}",
        files.len(),
        findings.len()
    );
    assert!(findings.is_empty(), "R3 命中：{findings:#?}");
}

/// R4：依赖白名单（`[dependencies]` == {thiserror}；`[dev-dependencies]` ⊆ {serde_json}）。
#[test]
fn r4_dependency_allowlist() {
    let manifest = std::fs::read_to_string(crate_dir().join("Cargo.toml")).expect("读 Cargo.toml");
    let deps = section_keys(&manifest, "[dependencies]");
    let dev_deps = section_keys(&manifest, "[dev-dependencies]");
    let deps: BTreeSet<String> = deps.into_iter().collect();
    let dev_deps: BTreeSet<String> = dev_deps.into_iter().collect();

    let want: BTreeSet<String> = ["thiserror".to_string()].into_iter().collect();
    assert_eq!(deps, want, "运行时依赖必须恰好是 {{thiserror}}：{deps:?}");

    let allowed_dev: BTreeSet<String> = ["serde_json".to_string()].into_iter().collect();
    assert!(
        dev_deps.is_subset(&allowed_dev),
        "dev 依赖越界：{:?}",
        dev_deps.difference(&allowed_dev).collect::<Vec<_>>()
    );

    let forbidden = ["serde", "sha2", "rand", "mlua", "getrandom", "tokio"];
    for name in forbidden {
        assert!(!deps.contains(name), "运行时依赖出现禁用项 {name}");
        assert!(!dev_deps.contains(name), "dev 依赖出现禁用项 {name}");
    }
    println!(
        "determinism_scan.r4 deps={:?} dev={:?} hits=0",
        deps, dev_deps
    );
}

/// 读 `[section]` 之后到下一个 `[` 之间的键名（`name = "..."` / `name = { ... }`）。
fn section_keys(manifest: &str, section: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut in_section = false;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_section = trimmed == section;
            continue;
        }
        if !in_section || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((key, _)) = trimmed.split_once('=') {
            keys.push(key.trim().to_string());
        }
    }
    keys
}

/// 类型词汇单源：`AttrType::label()` 与 `packages/libs/entity/init.lua` 的四类型字面量逐字相同。
///
/// 这条让「同源形状」不再是口头承诺：改一侧即红（assetgen 的 `rng-not-copied` 同范式）。
#[test]
fn type_vocabulary_matches_entity_package() {
    use ygg_sim::component::AttrType;

    let entity_lua = crate_dir()
        .parent()
        .and_then(|crates| crates.parent())
        .map(|root| root.join("packages/libs/entity/init.lua"))
        .expect("仓根可解析");
    let text = std::fs::read_to_string(&entity_lua)
        .unwrap_or_else(|err| panic!("读不到 {}：{err}", entity_lua.display()));
    let mut checked = 0usize;
    for ty in [
        AttrType::Number { frac_bits: 32 },
        AttrType::Str,
        AttrType::Bool,
        AttrType::Ref,
        AttrType::Tag,
    ] {
        let label = ty.label();
        let quoted = format!("\"{label}\"");
        // `tag` 只在 Tier 1 有词汇（Tier 2 侧写作 `tags.<name>`），故只要求四类型对齐。
        if ty == AttrType::Tag {
            continue;
        }
        assert!(
            text.contains(&quoted),
            "entity.md/init.lua 里找不到类型字面量 {quoted}（Tier 1 与 Tier 2 词汇已分叉）"
        );
        checked += 1;
    }
    println!("determinism_scan.vocab checked={checked} tag=Tier1-only hits=0");
    assert_eq!(checked, 4);
}
