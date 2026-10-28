//! ygg-lint CLI：`ygg-lint [--json] <.lua 文件|目录>…`（目录递归收集 *.lua）。
//!
//! **退出码契约（管线 lint 门子进程消费，D-8）**：
//! - 0 = 无 error 级诊断（warning 放行）；
//! - 1 = 存在 error 级诊断（含 YGG-PARSE）；
//! - 2 = 用法 / IO / 内部错误。
//!
//! 输出：默认人读（codespan-reporting 渲染，无色缓冲）；`--json` 输出诊断
//! 数组（serde camelCase，与 lib Diagnostic 一致）——这是管线 lint 门的
//! 唯一机器契约面（D-14）。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::exit;

use codespan_reporting::diagnostic::{Diagnostic as CsDiagnostic, Label};
use codespan_reporting::files::SimpleFiles;
use codespan_reporting::term::{self, termcolor::Buffer, Config};

use ygg_lint::diagnostic::{Diagnostic, Severity};
use ygg_lint::rules::lint_source;

fn main() {
    let mut json = false;
    let mut paths: Vec<PathBuf> = Vec::new();
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--json" => json = true,
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

    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    for file in &files {
        let source = match fs::read_to_string(file) {
            Ok(source) => source,
            Err(err) => {
                eprintln!("error: 无法读取 {}: {err}", file.display());
                exit(2);
            }
        };
        diagnostics.extend(lint_source(&file.display().to_string(), &source));
    }

    if json {
        println!(
            "{}",
            serde_json::to_string(&diagnostics).expect("diagnostics serialize")
        );
    } else {
        render_human(&diagnostics, &files);
    }

    if diagnostics.iter().any(Diagnostic::is_error) {
        exit(1);
    }
}

fn print_help() {
    println!("ygg-lint —— Yggdrasill Lua 静态检查（D12 第五 crate）");
    println!();
    println!("用法: ygg-lint [--json] <.lua 文件|目录>…");
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
fn render_human(diagnostics: &[Diagnostic], files: &[PathBuf]) {
    let mut cs_files = SimpleFiles::new();
    let mut id_by_name: HashMap<String, usize> = HashMap::new();
    for file in files {
        let name = file.display().to_string();
        let source = fs::read_to_string(file).unwrap_or_default();
        // add 按值接管（Name: Display, Source: AsRef<str>）——传克隆，本地保留原值入索引。
        let id = cs_files.add(name.clone(), source.clone());
        id_by_name.insert(name, id);
    }
    for diagnostic in diagnostics {
        let Some(&id) = id_by_name.get(&diagnostic.file) else {
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
