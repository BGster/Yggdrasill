//! `--pack <mod目录>` 面：多文件 Mod 的整包检查（r3）。
//!
//! 单文件模式检查一份源码；多文件 Mod 的检查面多出三层，本模块负责它们：
//!
//! 1. **声明**：`pack.json` 的 `code` 段（`entry` + `modules[]`）形状与内容
//!    ——逐条报，不在第一处错误就停（作者一轮就能看完所有问题）。
//! 2. **每个模块独立 parse**：**不拼接**。Lua 的 `local` 本就不跨 chunk，
//!    装载器也是一个模块一个 `lua.load(...).call()`，所以「逐模块独立编译」
//!    就是真实边界；拼接反而会制造一个装载期不存在的共享作用域，把
//!    「模块里能用上一模块的 local」这种假阳性/假阴性带进检查结论。
//! 3. **依赖图**：按 AST 扫 `require("…")` 字面量（不执行、不做常量折叠），
//!    校验目标已登记，并用**装载器同一份** [`ygg_modules::LoadStack`] 判循环
//!    与深度——链条文本、深度上限都来自同一实现，报告与装载期报同一句话。
//!
//! # 判定口径复用，不重写
//!
//! 名字/路径/上限全部来自 [`ygg_modules`]（`ygg-sandbox` 的解析面就是它）：
//! `name_of_path` 判「这个路径能不能当模块」、`normalize` 判「这个 require
//! 名是不是规范名」、`MAX_MODULES` / `MAX_CODE_BYTES` / `MAX_DEPTH` 判上限。
//! 本模块**不新增任何判定规则**，只负责把它们应用到磁盘上的实际文件。
//!
//! # 零 IO
//!
//! 读文件由调用方注入（`read: impl FnMut(&str) -> Option<String>`）——与
//! `ygg_sandbox::pack::collect_bytes` 同一套注入纪律：lib 面纯函数，测试可以
//! 用内存表喂料，不碰文件系统。

use std::collections::{BTreeMap, BTreeSet};

use full_moon::ast::{Ast, FunctionCall};
use full_moon::node::Node;
use full_moon::visitors::Visitor;
use serde_json::Value as Json;

use ygg_modules::{
    name_of_path, normalize, path_of, LoadStack, ModuleError, MAX_CODE_BYTES, MAX_DEPTH,
    MAX_MODULES,
};

use crate::diagnostic::{code, Diagnostic, Severity};
use crate::locate::line_col;
use crate::rules::{call_path, effects_for_ast, parse_gate, string_argument, string_literal_value};

/// 整包检查结果：要么这本来就是个单文件 Mod，要么有一份报告。
#[derive(Debug)]
pub enum PackOutcome {
    /// `pack.json` 没有 `code` 段 = 单文件 Mod（**不是错误**：现有三个 Mod
    /// 都是这个形态，`--pack` 对它只该给一句提示）。
    SingleFile,
    /// 多文件 Mod 的检查报告（`code` 段存在；诊断可能为空）。
    Modules(PackReport),
}

/// 多文件 Mod 的检查报告。
#[derive(Debug, Default)]
pub struct PackReport {
    /// 诊断（file = `pack.json` 显示名或模块的包内相对路径）。
    pub diagnostics: Vec<Diagnostic>,
    /// 已登记的模块数（不含入口）。
    pub modules: usize,
    /// 从入口出发的最大 require 链长度（> 0 说明真的拆了依赖）。
    pub max_depth: usize,
    /// 模块源码字节总数。
    pub total_bytes: usize,
}

/// 整包检查入口：`pack_json` 是 `pack.json` 的文本，`read` 按包内相对路径取源码。
///
/// `pack_label` 是 `pack.json` 在诊断里的显示名（CLI 给绝对/相对路径，
/// 测试给任意标签）——声明类诊断都落在它上面，否则作者不知道该改哪个文件。
pub fn lint_pack(
    pack_label: &str,
    pack_json: &str,
    read: impl FnMut(&str) -> Option<String>,
) -> PackOutcome {
    lint_pack_with_packages(pack_label, pack_json, &BTreeMap::new(), read)
}

/// 带「包模块声明集」的整包检查（r2）。
///
/// `package_modules` 是 `模块名 → 展示用路径`（如 `pkg.fixture` →
/// `fixture/init.lua`），由调用方从 manifest 的 `packages` 段推导。
///
/// 为什么它只参与 ⑥（名字解析）而**不**参与 ⑧（不可达警告）：包模块是
/// 另一个命名空间，本来就不一定从本 Mod 的入口可达，用本入口去判「死模块」
/// 会给出成片假告警。
///
/// 为什么这里只收一张现成的名字表而不在本 crate 里重新解析包：`ygg-lint` 会被
/// 编进 wasm32，不能依赖 `ygg-sandbox`（那会把 mlua 拉进图）。名字映射的权威
/// 实现是 `ygg_sandbox::packages::module_prefix` + `module_name`；本表是它在
/// 静态面上的**投影**，不是第二套规则。
pub fn lint_pack_with_packages(
    pack_label: &str,
    pack_json: &str,
    package_modules: &BTreeMap<String, String>,
    mut read: impl FnMut(&str) -> Option<String>,
) -> PackOutcome {
    let mut diagnostics = Vec::new();
    let mut report = PackReport::default();

    let pack: Json = match serde_json::from_str(pack_json) {
        Ok(pack) => pack,
        Err(err) => {
            diagnostics.push(decl_error(
                pack_label,
                pack_json,
                0,
                format!("pack.json 不是合法 JSON：{err}"),
                "修好 JSON 语法后重试（`--pack` 以 pack.json 为声明入口）".to_owned(),
            ));
            report.diagnostics = diagnostics;
            return PackOutcome::Modules(report);
        }
    };

    // ① `code` 段缺失 = 单文件 Mod（旧路径零迁移；这里只给模式信号，不算错）。
    let Some(raw_code) = pack.get("code") else {
        return PackOutcome::SingleFile;
    };
    if !raw_code.is_object() {
        diagnostics.push(decl_error(
            pack_label,
            pack_json,
            0,
            "pack.json 的 `code` 段必须是对象（`{\"entry\": \"mod.lua\", \"modules\": [...]}`）"
                .to_owned(),
            "把 code 段写成 {entry, modules} 形态；不要写成字符串或数组".to_owned(),
        ));
        report.diagnostics = diagnostics;
        return PackOutcome::Modules(report);
    }

    // ② 声明形状：entry / modules 逐字段校验（全量报，不在第一处停）。
    let entry = match raw_code.get("entry") {
        Some(Json::String(entry)) => Some(entry.clone()),
        Some(_) => {
            diagnostics.push(decl_error(
                pack_label,
                pack_json,
                0,
                "pack.json 的 `code.entry` 必须是字符串".to_owned(),
                "写入入口文件名（包内相对路径，如 \"mod.lua\"）".to_owned(),
            ));
            None
        }
        None => {
            diagnostics.push(decl_error(
                pack_label,
                pack_json,
                0,
                "pack.json 的 `code` 段缺少 `entry`".to_owned(),
                "补上 `\"entry\": \"mod.lua\"`（入口由宿主直接装载，不登记为模块）".to_owned(),
            ));
            None
        }
    };

    let raw_modules: Vec<Json> = match raw_code.get("modules") {
        Some(Json::Array(items)) => items.clone(),
        Some(_) => {
            diagnostics.push(decl_error(
                pack_label,
                pack_json,
                0,
                "pack.json 的 `code.modules` 必须是字符串数组".to_owned(),
                "逐条列出可 require 的模块路径（没有就写空数组）".to_owned(),
            ));
            Vec::new()
        }
        None => {
            diagnostics.push(decl_error(
                pack_label,
                pack_json,
                0,
                "pack.json 的 `code` 段缺少 `modules`".to_owned(),
                "补上 `\"modules\": []`（显式空数组才表示「只有入口」）".to_owned(),
            ));
            Vec::new()
        }
    };

    // ③ 逐个声明：名字可规范化 → 文件可读 → 单模块与整包字节上限。
    let mut declared: BTreeMap<String, String> = BTreeMap::new(); // 规范名 → 声明路径
    let mut sources: BTreeMap<String, String> = BTreeMap::new(); // 声明路径 → 源码
    let mut seen_paths: BTreeSet<String> = BTreeSet::new();
    if raw_modules.len() > MAX_MODULES {
        diagnostics.push(decl_diagnostic(
            pack_label,
            pack_json,
            0,
            code::MOD_BUDGET,
            Severity::Error,
            format!(
                "`code.modules` 声明了 {} 个模块（上限 {MAX_MODULES}）",
                raw_modules.len()
            ),
            "拆包或合并模块；模块表规模上限是装载期硬约束".to_owned(),
        ));
    }

    for (index, item) in raw_modules.iter().enumerate() {
        let Json::String(path) = item else {
            diagnostics.push(decl_error(
                pack_label,
                pack_json,
                0,
                format!("`code.modules[{index}]` 必须是字符串"),
                "模块路径写成字符串（包内相对路径，`.lua` 结尾）".to_owned(),
            ));
            continue;
        };
        let span = span_of(pack_json, &format!("\"{path}\""));
        if !seen_paths.insert(path.clone()) {
            diagnostics.push(decl_error(
                pack_label,
                pack_json,
                span.0,
                format!("`code.modules` 重复声明 `{path}`"),
                "同一路径只登记一次（重复登记会让装载表的 key 含义不明）".to_owned(),
            ));
            continue;
        }
        if entry.as_deref() == Some(path.as_str()) {
            diagnostics.push(decl_error(
                pack_label,
                pack_json,
                span.0,
                format!("入口 `{path}` 同时被登记为模块"),
                "入口由宿主按 `code.entry` 装载，不要放进 `code.modules`".to_owned(),
            ));
            continue;
        }
        // 名字/路径校验走解析面（装载期判定的唯一实现）。
        let name = match name_of_path(path) {
            Ok(name) => name,
            Err(cause) => {
                diagnostics.push(decl_diagnostic(
                    pack_label,
                    pack_json,
                    span.0,
                    code::MOD_NAME,
                    Severity::Error,
                    format!("`code.modules` 的 `{path}` 不是合法模块路径：{cause}"),
                    "模块路径须为包内相对路径、`.lua` 结尾、每段仅 A-Z a-z 0-9 _（文件名段不得含点）"
                        .to_owned(),
                ));
                continue;
            }
        };
        declared.insert(name, path.clone());

        let Some(source) = read(path) else {
            diagnostics.push(Diagnostic {
                file: path.clone(),
                offset: 0,
                end_offset: 0,
                line: 1,
                column: 1,
                code: code::MOD_MISSING,
                severity: Severity::Error,
                message: format!("`code.modules` 声明了 `{path}`，但包内读不到该文件"),
                fix: "补上模块文件，或从 pack.json 的 code.modules 里删掉这条声明".to_owned(),
            });
            continue;
        };
        if source.len() > MAX_CODE_BYTES {
            diagnostics.push(Diagnostic {
                file: path.clone(),
                offset: 0,
                end_offset: 0,
                line: 1,
                column: 1,
                code: code::MOD_BUDGET,
                severity: Severity::Error,
                message: format!(
                    "模块 `{path}` 源码 {} 字节（上限 {MAX_CODE_BYTES}）",
                    source.len()
                ),
                fix: "拆分模块或裁掉无用代码（装载期按同一上限拒绝）".to_owned(),
            });
            continue;
        }
        report.total_bytes = report.total_bytes.saturating_add(source.len());
        sources.insert(path.clone(), source);
    }

    if report.total_bytes > MAX_CODE_BYTES {
        // 整包上限与单模块上限共用一份 1 MiB 预算（打包期同判，见 pack.rs）。
        diagnostics.push(decl_diagnostic(
            pack_label,
            pack_json,
            0,
            code::MOD_BUDGET,
            Severity::Error,
            format!(
                "模块源码合计 {} 字节（上限 {MAX_CODE_BYTES}）",
                report.total_bytes
            ),
            "减少模块总字节数（打包/装载期按同一预算拒绝）".to_owned(),
        ));
    }

    // ④ 入口源码（图根）：读不到就没法建依赖图，但不影响上面的声明诊断。
    let entry_source = entry.as_deref().and_then(|path| match read(path) {
        Some(source) => Some(source),
        None => {
            diagnostics.push(Diagnostic {
                file: path.to_owned(),
                offset: 0,
                end_offset: 0,
                line: 1,
                column: 1,
                code: code::MOD_MISSING,
                severity: Severity::Error,
                message: format!("`code.entry` 声明了 `{path}`，但包内读不到该文件"),
                fix: "补上入口文件，或修正 pack.json 的 code.entry".to_owned(),
            });
            None
        }
    });
    if let (Some(path), Some(source)) = (entry.as_deref(), entry_source.as_deref()) {
        sources.insert(path.to_owned(), source.to_owned());
    }

    // ⑤ 逐文件：独立 parse（不拼接）→ 解析门 + 副作用面 + 依赖面。
    //
    // L1 主防线（`YGG-MOD-EFFECT`）**只对模块**：模块 chunk 顶层是「装载期就会
    // 执行的面」，而入口本来就是装配清单（注册 / 初始化是它的职责）。哪边是
    // 入口在 ①–④ 已经判完（`code.entry` 与 `code.modules` 不允许重叠），这里只
    // 按路径区分，不看内容猜测。
    let mut units: BTreeMap<String, Unit> = BTreeMap::new();
    for (path, source) in &sources {
        let mut unit = Unit::default();
        match parse_gate(path, source) {
            Ok(ast) => {
                if entry.as_deref() != Some(path.as_str()) {
                    diagnostics.extend(effects_for_ast(path, source, &ast));
                }
                unit.edges = scan_requires(&ast)
                    .into_iter()
                    .map(|hit| Edge {
                        offset: hit.offset,
                        end: hit.end,
                        literal: hit.literal,
                        target: None,
                    })
                    .collect();
            }
            // R3 命中或语法错误：该文件没有可信 AST，依赖面跳过（装载期同样跑不起来）。
            Err(file_diagnostics) => diagnostics.extend(file_diagnostics),
        }
        units.insert(path.clone(), unit);
    }

    // ⑥ 依赖解析：require 的名字必须已登记（装载期只查表，不做路径拼接）。
    let label_by_name: BTreeMap<&str, &str> = declared
        .iter()
        .map(|(name, path)| (name.as_str(), path.as_str()))
        .collect();
    for (path, unit) in units.iter_mut() {
        let source = &sources[path];
        for edge in unit.edges.iter_mut() {
            match normalize(&edge.literal) {
                Ok(name) => match label_by_name
                    .get(name.as_str())
                    .copied()
                    .or_else(|| package_modules.get(name.as_str()).map(String::as_str))
                {
                    Some(target) => edge.target = Some(target.to_owned()),
                    None => diagnostics.push(undeclared_error(path, source, edge, &name)),
                },
                Err(cause) => {
                    diagnostics.push(illegal_name_error(path, source, edge, &cause));
                }
            }
        }
    }

    // ⑦ 依赖图：入口出发的循环与深度（判定结构 = 装载器的 `LoadStack`）。
    let mut visited: BTreeSet<String> = BTreeSet::new();
    let mut stack = LoadStack::new();
    if let Some(entry_path) = entry.as_deref() {
        if let Some(unit) = units.get(entry_path) {
            let root_edges: Vec<&Edge> = unit.edges.iter().collect();
            walk_edges(
                entry_path,
                &root_edges,
                &units,
                &sources,
                &mut stack,
                &mut visited,
                &mut report.max_depth,
                &mut diagnostics,
            );
        }
    }

    // ⑧ 登记了但不可达：**不再报**（M1-E）。`YGG-MOD-UNUSED` 随事件层退役——
    // 死模块只是「多登记了一份字节」，装载期照样登记、照样不装载，没有可观测
    // 差异；在包里逐条提示它，噪音大于收益。可达性仍被 ⑦ 的遍历用到（见
    // `walk_edges` 的 `visited` 备忘），只是不再由它产诊断。

    report.modules = declared.len();
    report.diagnostics = diagnostics;
    PackOutcome::Modules(report)
}

/// 单个源文件的依赖面（只有出边；入边由解析期反查）。
#[derive(Debug, Default)]
struct Unit {
    edges: Vec<Edge>,
}

/// 一条 `require` 出边（`target` 为解析后的声明路径，未解析 = None）。
#[derive(Debug)]
struct Edge {
    offset: usize,
    end: usize,
    literal: String,
    target: Option<String>,
}

/// 从入口出发深度优先：循环/深度用装载器的 `LoadStack`，`visited` 只防重复展开。
///
/// 已知近似：`visited` 命中的节点不再展开子树，因此「经由已访问节点绕回来的
/// 更深路径」可能被少算（深度只可能少报、不会多报）；循环判定不受影响——
/// 它比对的是当前存活帧（真实路径），不是 `visited`。
#[allow(clippy::too_many_arguments)]
fn walk_edges(
    from_path: &str,
    edges: &[&Edge],
    units: &BTreeMap<String, Unit>,
    sources: &BTreeMap<String, String>,
    stack: &mut LoadStack,
    visited: &mut BTreeSet<String>,
    max_depth: &mut usize,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let source = sources.get(from_path).map(String::as_str).unwrap_or("");
    for edge in edges {
        let Some(target_path) = &edge.target else {
            continue;
        };
        let Some(unit) = units.get(target_path) else {
            continue;
        };
        let Ok(name) = name_of_path(target_path) else {
            continue; // 声明期已报过「非法模块路径」。
        };
        if let Err(err) = stack.push(&name) {
            let (code_str, severity) = match &err {
                ModuleError::Circular { .. } => (code::MOD_CYCLE, Severity::Error),
                _ => (code::MOD_BUDGET, Severity::Error),
            };
            diagnostics.push(span_diagnostic(
                from_path,
                source,
                (edge.offset, edge.end),
                code_str,
                severity,
                format!("require(\"{}\") 会在这里失败：{err}", edge.literal),
                cycle_fix(&err),
            ));
            continue;
        }
        *max_depth = (*max_depth).max(stack.depth());
        let child_edges: Vec<&Edge> = unit.edges.iter().collect();
        if visited.insert(name.clone()) {
            walk_edges(
                target_path,
                &child_edges,
                units,
                sources,
                stack,
                visited,
                max_depth,
                diagnostics,
            );
        }
        stack.pop();
    }
}

/// `require` 未登记：装载期会以 `not available` 拒绝，这里提前报。
fn undeclared_error(path: &str, source: &str, edge: &Edge, name: &str) -> Diagnostic {
    // 包模块（`pkg.*`）的修法完全不同：它们**不进** `code.modules`，而是由
    // `pack.json` 的 `packages` 段声明整个包（manifest 再逐文件入册）。把
    // code.modules 的提示照搬到这里，会教作者去做一件装载期根本不接受的事。
    if let Some(rest) = name.strip_prefix("pkg.") {
        let id = rest.split('.').next().unwrap_or(rest);
        return span_diagnostic(
            path,
            source,
            (edge.offset, edge.end),
            code::MOD_UNDECLARED,
            Severity::Error,
            format!(
                "require(\"{}\") 的目标包模块 `{name}` 未声明\
                 （pack.json 的 `packages` 段里没有对应的包，装载期会以可读拒绝失败）",
                edge.literal
            ),
            format!(
                "把 \"{id}\" 加进 pack.json 的 `packages` 段（形如 {{\"id\": \"{id}\", \
                 \"version\": \"0.1.0\"}}），再运行打包工具重建 manifest"
            ),
        );
    }
    let fix = path_of(name).map_or_else(
        |_| "在 pack.json 的 code.modules 里登记该模块".to_owned(),
        |declared_path| {
            format!("在 pack.json 的 code.modules 里加上 \"{declared_path}\" 并运行打包工具")
        },
    );
    span_diagnostic(
        path,
        source,
        (edge.offset, edge.end),
        code::MOD_UNDECLARED,
        Severity::Error,
        format!(
            "require(\"{}\") 的目标模块 `{name}` 未在 pack.json 的 code.modules 登记\
             （装载期只装已登记模块，会以可读拒绝失败）",
            edge.literal
        ),
        fix,
    )
}

/// `require` 的名字不合规范（路径形态 / 非法字符 / 超长 …）。
fn illegal_name_error(path: &str, source: &str, edge: &Edge, cause: &ModuleError) -> Diagnostic {
    // 作者很可能写成路径形态（require("src/util")）——给正确写法，别只说「非法」。
    let fix = match name_of_path(&edge.literal) {
        Ok(name) => format!("模块名是点分形态：写成 require(\"{name}\")"),
        Err(_) => "模块名须为点分标识符（如 src.util，对应文件 src/util.lua）".to_owned(),
    };
    span_diagnostic(
        path,
        source,
        (edge.offset, edge.end),
        code::MOD_NAME,
        Severity::Error,
        format!("require(\"{}\") 不是规范模块名：{cause}", edge.literal),
        fix,
    )
}

/// 循环/深度失败的修复建议（两类都是「改依赖形状」，不是改名字）。
fn cycle_fix(err: &ModuleError) -> String {
    match err {
        ModuleError::Circular { .. } => {
            "把共享部分下沉到第三个模块（双方都 require 它），装载器不做部分初始化".to_owned()
        }
        _ => format!("减少嵌套层级（装载深度上限 {MAX_DEPTH}）"),
    }
}

/// `require("…")` 字面量收集器（AST 面：注释与字符串里的 require 不会被误认）。
#[derive(Default)]
struct RequireScanner {
    hits: Vec<RequireHit>,
}

struct RequireHit {
    offset: usize,
    end: usize,
    literal: String,
}

impl Visitor for RequireScanner {
    fn visit_function_call(&mut self, node: &FunctionCall) {
        if call_path(node).as_deref() == Some("require") {
            if let Some(token) = string_argument(node) {
                let (offset, end) = node_span(node);
                self.hits.push(RequireHit {
                    offset,
                    end,
                    literal: string_literal_value(&token),
                });
            }
        }
    }
}

/// AST → `require` 字面量表（源序）。
fn scan_requires(ast: &Ast) -> Vec<RequireHit> {
    let mut scanner = RequireScanner::default();
    scanner.visit_ast(ast);
    let mut hits = scanner.hits;
    hits.sort_by_key(|hit| hit.offset);
    hits
}

/// 字面量在 `pack.json` 文本里的位置（声明类诊断的定位；找不到退回 0）。
fn span_of(haystack: &str, needle: &str) -> (usize, usize) {
    match haystack.find(needle) {
        Some(start) => (start, start + needle.len()),
        None => (0, 0),
    }
}

/// 声明类诊断（落 `pack.json` 上；`offset` 处给划线）。
fn decl_error(
    pack_label: &str,
    pack_json: &str,
    offset: usize,
    message: String,
    fix: String,
) -> Diagnostic {
    decl_diagnostic(
        pack_label,
        pack_json,
        offset,
        code::PACK_DECL,
        Severity::Error,
        message,
        fix,
    )
}

/// 声明类诊断（可指定码与严重级：`code` 段的问题不全是 PACK_DECL）。
fn decl_diagnostic(
    pack_label: &str,
    pack_json: &str,
    offset: usize,
    code_str: &'static str,
    severity: Severity,
    message: String,
    fix: String,
) -> Diagnostic {
    span_diagnostic(
        pack_label,
        pack_json,
        (offset, offset + 1),
        code_str,
        severity,
        message,
        fix,
    )
}

/// 通用诊断构造：file/行列都由「文件显示名 + 该文件源码 + 字节区间」现算。
fn span_diagnostic(
    file: &str,
    source: &str,
    span: (usize, usize),
    code_str: &'static str,
    severity: Severity,
    message: String,
    fix: String,
) -> Diagnostic {
    let offset = span.0.min(source.len());
    let (line, column) = line_col(source, offset);
    Diagnostic {
        file: file.to_owned(),
        offset,
        end_offset: span.1.max(offset + 1),
        line,
        column,
        code: code_str,
        severity,
        message,
        fix,
    }
}

/// AST 节点 → (start_bytes, end_bytes)（与 rules.rs 的 `node_span` 同口径）。
fn node_span<N: Node>(node: &N) -> (usize, usize) {
    let start = node.start_position().map_or(0, |position| position.bytes());
    let end = node
        .end_position()
        .map_or(start, |position| position.bytes());
    (start, end)
}
