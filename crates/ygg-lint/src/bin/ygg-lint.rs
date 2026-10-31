//! ygg-lint CLI：两套模式，一套退出码契约。
//!
//! ```text
//! ygg-lint [--json] <.lua 文件|目录>…    单文件模式（r2 行为，逐文件检查）
//! ygg-lint [--json] --pack <mod目录>     多文件 Mod 整包检查（r3：声明 + 逐模块
//!                                        parse + require 依赖图 + 上限）
//! ```
//!
//! **退出码契约（管线 lint 门子进程消费，D-8）**：
//! - 0 = 无 error 级诊断（warning 放行）；
//! - 1 = 存在 error 级诊断（含 YGG-PARSE）；
//! - 2 = 用法 / IO / 内部错误。
//!
//! 输出：默认人读（codespan-reporting 渲染，无色缓冲）；`--json` 输出诊断
//! 数组（serde camelCase，与 lib Diagnostic 一致）——这是管线 lint 门的
//! 唯一机器契约面（D-14）。两种模式下 `--json` 的 stdout 都只有那个数组：
//! 提示与统计一律走 stderr。
//!
//! **`--pack` 遇到没有 `code` 段的 Mod（单文件 Mod）不是错误**：打印一句提示
//! 并退出 0。把「这不是多文件 Mod」判成失败，会让现有三个 Mod 在管线里红掉，
//! 而它们本来就该按单文件模式检查。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::exit;

use codespan_reporting::diagnostic::{Diagnostic as CsDiagnostic, Label};
use codespan_reporting::files::SimpleFiles;
use codespan_reporting::term::{self, termcolor::Buffer, Config};

use ygg_lint::diagnostic::{Diagnostic, Severity};
use ygg_lint::pack::{lint_pack, PackOutcome};
use ygg_lint::rules::lint_source;

fn main() {
    let mut json = false;
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut pack: Option<PathBuf> = None;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--json" => json = true,
            "--pack" => {
                let Some(dir) = arguments.next() else {
                    eprintln!("error: --pack 需要 <mod目录>");
                    exit(2);
                };
                pack = Some(PathBuf::from(dir));
            }
            "--help" | "-h" => {
                print_help();
                exit(0);
            }
            other if other.starts_with("--") => {
                eprintln!("error: 未知参数 {other}");
                exit(2);
            }
            other => paths.push(PathBuf::from(other)),
        }
    }

    if let Some(dir) = pack {
        if !paths.is_empty() {
            eprintln!("error: --pack 是独立模式，不接受额外的文件/目录参数");
            exit(2);
        }
        exit(run_pack(&dir, json));
    }

    if paths.is_empty() {
        print_help();
        exit(2);
    }

    let mut files: Vec<PathBuf> = Vec::new();
    for path in &paths {
        collect_lua_files(path, &mut files);
    }
    if files.is_empty() {
        eprintln!("error: 未找到任何 .lua 文件");
        exit(2);
    }

    let mut sources: BTreeMap<String, String> = BTreeMap::new();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    for file in &files {
        let source = match fs::read_to_string(file) {
            Ok(source) => source,
            Err(err) => {
                eprintln!("error: 无法读取 {}: {err}", file.display());
                exit(2);
            }
        };
        let name = file.display().to_string();
        diagnostics.extend(lint_source(&name, &source));
        sources.insert(name, source);
    }

    if json {
        println!(
            "{}",
            serde_json::to_string(&diagnostics).expect("diagnostics serialize")
        );
    } else {
        render_human(&diagnostics, &sources);
    }

    if diagnostics.iter().any(Diagnostic::is_error) {
        exit(1);
    }
}

/// `--pack` 模式：读 `<mod目录>/pack.json` → 整包检查 → 渲染 / 退出码。
fn run_pack(dir: &Path, json: bool) -> i32 {
    let pack_path = dir.join("pack.json");
    let pack_text = match fs::read_to_string(&pack_path) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("error: 无法读取 {}: {err}", pack_path.display());
            return 2;
        }
    };
    let pack_label = pack_path.display().to_string();

    // 读盘由本层注入（lib 不碰文件系统）；顺手留住源码供 codespan 渲染。
    let mut sources: BTreeMap<String, String> = BTreeMap::new();
    let outcome = {
        let sources = &mut sources;
        lint_pack(&pack_label, &pack_text, |relative| {
            let text = fs::read_to_string(dir.join(relative)).ok()?;
            sources.insert(relative.to_owned(), text.clone());
            Some(text)
        })
    };

    match outcome {
        // 单文件 Mod：不是错误，是一句提示（现有三个 Mod 都是这个形态）。
        PackOutcome::SingleFile => {
            eprintln!("该 Mod 为单文件，无 code 段：{pack_label}");
            eprintln!("（单文件 Mod 直接检查入口即可：ygg-lint <mod目录>/mod.lua）");
            if json {
                // `--json` 的 stdout 恒为诊断数组：空数组而不是空输出，
                // 否则管线侧的 JSON 解析会先炸在「没有输出」上。
                println!("[]");
            }
            0
        }
        PackOutcome::Modules(report) => {
            sources.insert(pack_label, pack_text);
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&report.diagnostics).expect("diagnostics serialize")
                );
            } else {
                println!(
                    "pack: {} 个模块 / 最大依赖深度 {} / 模块源码 {} 字节",
                    report.modules, report.max_depth, report.total_bytes
                );
                render_human(&report.diagnostics, &sources);
            }
            i32::from(report.diagnostics.iter().any(Diagnostic::is_error))
        }
    }
}

fn print_help() {
    println!("ygg-lint —— Yggdrasill Lua 静态检查（D12 第五 crate）");
    println!();
    println!("用法:");
    println!("  ygg-lint [--json] <.lua 文件|目录>…   单文件模式（目录递归收集 *.lua）");
    println!(
        "  ygg-lint [--json] --pack <mod目录>    多文件 Mod 整包检查（读 pack.json 的 code 段）"
    );
    println!("  --json   机器可读输出（诊断数组；管线 lint 门契约）");
    println!();
    println!("退出码: 0 无 error 级诊断 / 1 存在 error 级诊断 / 2 用法或 IO 错误");
}

/// 文件 / 目录 → *.lua 收集（目录递归，序稳定）。
fn collect_lua_files(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_file() {
        out.push(path.to_path_buf());
        return;
    }
    if path.is_dir() {
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        let mut children: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
        children.sort();
        for child in children {
            collect_lua_files(&child, out);
        }
    }
}

/// 人读渲染：codespan 无色缓冲逐条打印 + 计数尾行。
///
/// 源码由调用方按「诊断里的 file 名」给齐（两种模式的 file 名来源不同：
/// 单文件模式是路径，`--pack` 是包内相对路径与 `pack.json` 的显示名）——
/// 渲染层不再自己读盘，也不会因为读不到就把诊断静默吞掉。
fn render_human(diagnostics: &[Diagnostic], sources: &BTreeMap<String, String>) {
    let mut cs_files = SimpleFiles::new();
    let mut id_by_name: BTreeMap<&str, usize> = BTreeMap::new();
    for (name, source) in sources {
        let id = cs_files.add(name.clone(), source.clone());
        id_by_name.insert(name.as_str(), id);
    }
    for diagnostic in diagnostics {
        let Some(&id) = id_by_name.get(diagnostic.file.as_str()) else {
            // 文件没内容可划线（如「声明了但读不到」）——不能静默吞掉：这类诊断
            // 恰恰是作者最需要看见的。退回一行纯文本，格式与 codespan 的 header 同形。
            let severity = match diagnostic.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            };
            println!(
                "{severity}[{}]: {}:{}:{}: {}（fix: {}）",
                diagnostic.code,
                diagnostic.file,
                diagnostic.line,
                diagnostic.column,
                diagnostic.message,
                diagnostic.fix
            );
            continue;
        };
        let cs = match diagnostic.severity {
            Severity::Error => CsDiagnostic::error(),
            Severity::Warning => CsDiagnostic::warning(),
        }
        .with_code(diagnostic.code)
        .with_message(diagnostic.message.clone())
        .with_labels(vec![Label::primary(
            id,
            diagnostic.offset..diagnostic.end_offset.max(diagnostic.offset + 1),
        )
        .with_message(diagnostic.fix.clone())])
        .with_notes(vec![format!("fix: {}", diagnostic.fix)]);
        let mut buffer = Buffer::no_color();
        let _ = term::emit_to_io_write(&mut buffer, &Config::default(), &cs_files, &cs);
        print!("{}", String::from_utf8_lossy(buffer.as_slice()));
    }
    let errors = diagnostics.iter().filter(|d| d.is_error()).count();
    let warnings = diagnostics.len() - errors;
    println!("{errors} error(s), {warnings} warning(s)");
}
