//! `--pack` 整包检查回归（r3）：声明形状、模块路径/名字、依赖图（缺失 / 未登记 /
//! 非法名 / 循环 / 不可达）、上限。
//!
//! 读盘由测试注入（内存表），因此这里不碰文件系统——与 `ygg-lint` 的 lib 纪律
//! 一致（零 IO），也让每个反例只差几行源码。

use ygg_lint::diagnostic::{code, Severity};
use ygg_lint::pack::{lint_pack, PackOutcome, PackReport};
use ygg_lint::Diagnostic;

/// 内存文件表 → 检查结果（`pack.json` 单独给：它是声明入口，不是模块）。
fn check(pack: &str, files: &[(&str, &str)]) -> PackReport {
    let files: Vec<(String, String)> = files
        .iter()
        .map(|(path, source)| ((*path).to_owned(), (*source).to_owned()))
        .collect();
    check_owned(pack, files)
}

/// 同上，但文件表是拥有所有权的（长链条反例要现拼源码）。
fn check_owned(pack: &str, files: Vec<(String, String)>) -> PackReport {
    match lint_pack("pack.json", pack, |relative| {
        files
            .iter()
            .find(|(path, _)| path == relative)
            .map(|(_, source)| source.clone())
    }) {
        PackOutcome::Modules(report) => report,
        PackOutcome::SingleFile => panic!("expected a code section"),
    }
}

fn codes(diagnostics: &[Diagnostic]) -> Vec<&'static str> {
    diagnostics.iter().map(|d| d.code).collect()
}

fn only<'a>(report: &'a PackReport, wanted: &str) -> &'a Diagnostic {
    let found: Vec<&Diagnostic> = report
        .diagnostics
        .iter()
        .filter(|d| d.code == wanted)
        .collect();
    assert_eq!(
        found.len(),
        1,
        "expected exactly one {wanted}, got {:?}",
        report.diagnostics
    );
    found[0]
}

/// `code` 段字面量（`entry` + 模块清单）。
fn pack_json(entry: &str, modules: &[&str]) -> String {
    let modules = modules
        .iter()
        .map(|path| format!("\"{path}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{{\"code\": {{\"entry\": \"{entry}\", \"modules\": [{modules}]}}}}")
}

#[test]
fn mod_without_code_section_is_single_file_not_an_error() {
    let pack = r#"{"schemaVersion": 1, "entries": [], "schemaFiles": []}"#;
    match lint_pack("pack.json", pack, |_| None) {
        PackOutcome::SingleFile => {}
        PackOutcome::Modules(report) => panic!("a missing code section is not a check: {report:?}"),
    }
}

#[test]
fn clean_multi_file_mod_has_no_diagnostics() {
    let report = check(
        &pack_json("mod.lua", &["src/util.lua"]),
        &[
            (
                "mod.lua",
                "local util = require(\"src.util\")\nreturn util\n",
            ),
            ("src/util.lua", "return { text = \"x\" }\n"),
        ],
    );
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert_eq!(report.modules, 1);
    assert_eq!(report.max_depth, 1);
    assert!(report.total_bytes > 0);
}

#[test]
fn declared_module_without_a_file_is_reported() {
    let report = check(
        &pack_json("mod.lua", &["src/gone.lua"]),
        &[("mod.lua", "return require(\"src.gone\")\n")],
    );
    let diagnostic = only(&report, code::MOD_MISSING);
    assert_eq!(diagnostic.file, "src/gone.lua");
    assert_eq!(diagnostic.severity, Severity::Error);
    // 同一份声明不再叠一条「不可达」（同一问题的回声）。
    assert_eq!(codes(&report.diagnostics), vec![code::MOD_MISSING]);
}

#[test]
fn illegal_module_path_in_the_declaration_is_reported() {
    // 文件名段含点 → 与 src/a/b.lua 撞同一个点分名（解析面拒绝）。
    let report = check(
        &pack_json("mod.lua", &["src/a.b.lua"]),
        &[("mod.lua", "return nil\n"), ("src/a.b.lua", "return {}\n")],
    );
    let diagnostic = only(&report, code::MOD_NAME);
    assert_eq!(diagnostic.file, "pack.json");
    assert!(
        diagnostic.message.contains("src/a.b.lua"),
        "{}",
        diagnostic.message
    );
}

#[test]
fn illegal_require_name_points_at_the_call_site() {
    // 路径形态的 require：装载期会被名字校验拒绝，lint 给正确写法。
    let report = check(
        &pack_json("mod.lua", &["src/util.lua"]),
        &[
            ("mod.lua", "local u = require(\"src/util\")\nreturn u\n"),
            ("src/util.lua", "return {}\n"),
        ],
    );
    let diagnostic = only(&report, code::MOD_NAME);
    assert_eq!(diagnostic.file, "mod.lua");
    assert_eq!(diagnostic.line, 1);
    assert!(
        diagnostic.fix.contains("src.util"),
        "fix must suggest the dotted form: {}",
        diagnostic.fix
    );
}

#[test]
fn undeclared_require_target_is_reported_with_line() {
    let report = check(
        &pack_json("mod.lua", &["src/util.lua"]),
        &[
            (
                "mod.lua",
                "local u = require(\"src.util\")\n\nlocal n = require(\"src.nowhere\")\nreturn u\n",
            ),
            ("src/util.lua", "return {}\n"),
        ],
    );
    let diagnostic = only(&report, code::MOD_UNDECLARED);
    assert_eq!(diagnostic.file, "mod.lua");
    assert_eq!(diagnostic.line, 3);
    assert!(
        diagnostic.fix.contains("src/nowhere.lua"),
        "fix must name the declaration to add: {}",
        diagnostic.fix
    );
}

#[test]
fn cycle_chain_matches_the_loader_wording() {
    let report = check(
        &pack_json("mod.lua", &["src/a.lua", "src/b.lua"]),
        &[
            ("mod.lua", "return require(\"src.a\")\n"),
            ("src/a.lua", "return require(\"src.b\")\n"),
            ("src/b.lua", "return require(\"src.a\")\n"),
        ],
    );
    let diagnostic = only(&report, code::MOD_CYCLE);
    assert!(
        diagnostic
            .message
            .contains("circular require: src.a -> src.b -> src.a"),
        "chain text must come from LoadStack: {}",
        diagnostic.message
    );
    // 定位在闭环的那条边上（b 的 require a），不是入口。
    assert_eq!(diagnostic.file, "src/b.lua");
    assert_eq!(diagnostic.line, 1);
}

#[test]
fn unreachable_module_is_a_warning() {
    let report = check(
        &pack_json("mod.lua", &["src/util.lua", "src/unused.lua"]),
        &[
            ("mod.lua", "return require(\"src.util\")\n"),
            ("src/util.lua", "return {}\n"),
            ("src/unused.lua", "return {}\n"),
        ],
    );
    let diagnostic = only(&report, code::MOD_UNUSED);
    assert_eq!(diagnostic.severity, Severity::Warning);
    assert_eq!(diagnostic.file, "pack.json");
    assert!(diagnostic.message.contains("src/unused.lua"));
}

#[test]
fn duplicate_declaration_and_entry_overlap_are_rejected() {
    let duplicate = check(
        &pack_json("mod.lua", &["src/a.lua", "src/a.lua"]),
        &[("mod.lua", "return nil\n"), ("src/a.lua", "return {}\n")],
    );
    assert_eq!(only(&duplicate, code::PACK_DECL).severity, Severity::Error);

    let overlap = check(
        &pack_json("mod.lua", &["mod.lua"]),
        &[("mod.lua", "return nil\n")],
    );
    assert!(
        only(&overlap, code::PACK_DECL).message.contains("mod.lua"),
        "{:?}",
        overlap.diagnostics
    );
}

#[test]
fn missing_entry_and_missing_modules_field_are_reported() {
    let missing_entry = check(r#"{"code": {"modules": []}}"#, &[]);
    assert!(only(&missing_entry, code::PACK_DECL)
        .message
        .contains("entry"));

    let missing_modules = check(r#"{"code": {"entry": "mod.lua"}}"#, &[("mod.lua", "")]);
    assert!(only(&missing_modules, code::PACK_DECL)
        .message
        .contains("modules"));
}

#[test]
fn too_many_modules_is_a_limit_error() {
    let paths: Vec<String> = (0..=ygg_modules::MAX_MODULES)
        .map(|index| format!("src/m{index}.lua"))
        .collect();
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    let files: Vec<(&str, &str)> = std::iter::once(("mod.lua", "return nil\n"))
        .chain(paths.iter().map(|path| (path.as_str(), "return {}\n")))
        .collect();
    let report = check(&pack_json("mod.lua", &refs), &files);
    let limit = only(&report, code::MOD_LIMIT);
    assert!(
        limit
            .message
            .contains(&ygg_modules::MAX_MODULES.to_string()),
        "{}",
        limit.message
    );
}

#[test]
fn syntax_error_in_a_module_carries_its_path_and_line() {
    let report = check(
        &pack_json("mod.lua", &["src/broken.lua"]),
        &[
            ("mod.lua", "return require(\"src.broken\")\n"),
            ("src/broken.lua", "local x = 1\nlocal y = \nreturn x\n"),
        ],
    );
    let diagnostic = only(&report, code::PARSE);
    assert_eq!(diagnostic.file, "src/broken.lua");
    assert_eq!(diagnostic.severity, Severity::Error);
    assert!(diagnostic.line >= 2, "line must point into the module");
}

#[test]
fn modules_are_parsed_independently_so_locals_do_not_leak() {
    // a.lua 的顶层 `local secret` 在 b.lua 里不可见（Lua 的 chunk 边界）。
    // 若 lint 把模块拼接成一份源码，"secret" 就会在 b.lua 里可解析——
    // 独立 parse 保证静态结论与装载期一致。
    let report = check(
        &pack_json("mod.lua", &["src/a.lua", "src/b.lua"]),
        &[
            (
                "mod.lua",
                "local a = require(\"src.a\")\nlocal b = require(\"src.b\")\nreturn a, b\n",
            ),
            ("src/a.lua", "local secret = 1\nreturn secret\n"),
            ("src/b.lua", "return secret\n"),
        ],
    );
    assert!(
        report.diagnostics.is_empty(),
        "独立 parse 下 b.lua 引用未定义全局是合法 Lua：{:?}",
        report.diagnostics
    );
}

#[test]
fn chain_deeper_than_the_load_limit_is_reported() {
    // 装载器按 MAX_DEPTH 显式判栈（不靠栈溢出兜底）：超过就拒。
    let depth = ygg_modules::MAX_DEPTH + 1;
    let name = |index: usize| format!("src.m{index}");
    let path = |index: usize| format!("src/m{index}.lua");
    let requires_next = |index: usize| {
        if index + 1 < depth {
            format!("return require(\"{}\")\n", name(index + 1))
        } else {
            "return {}\n".to_owned()
        }
    };

    let paths: Vec<String> = (0..depth).map(path).collect();
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    let mut files: Vec<(String, String)> = vec![(
        "mod.lua".to_owned(),
        format!("return require(\"{}\")\n", name(0)),
    )];
    for index in 0..depth {
        files.push((path(index), requires_next(index)));
    }

    let report = check_owned(&pack_json("mod.lua", &refs), files);
    let diagnostic = only(&report, code::MOD_LIMIT);
    assert!(
        diagnostic.message.contains("require depth exceeded"),
        "depth rejection must come from the loader's wording: {}",
        diagnostic.message
    );
    assert_eq!(diagnostic.line, 1);
}
