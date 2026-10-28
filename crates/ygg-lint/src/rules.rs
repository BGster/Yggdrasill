//! 首批规则（FR-8）：R1 choice 前 emit / R2 local 存实体句柄 / R3 goto 点
//! 形态（预扫包装）+ parse-error 诊断。
//!
//! 判定口径（docs/mod-authoring.md §1/§2 + test-adventure README 清单 1/4/8）：
//! - **R1** handler = 顶层 `ygg.narrative.node("…", function() … end)` 与
//!   `ygg.on("…", function() … end)` 注册的匿名函数体（嵌套闭包计入同一
//!   handler——源序文本近似）。体内 `ygg.emit` 先于首个 `ygg.choice` →
//!   每个 emit 一条 error。`ygg.narrate` 不计（choice 前的叙述直进缓冲
//!   不丢弃，§2 边界①）。已知近似：命名函数引用注册（`ygg.on("x", f)`）
//!   不识别（生成物与 golden 均用匿名函数体；漏报不误报）。
//! - **R2**（D-11 双档）顶层 `local x = ygg.entity.spawn()` 直接捕获 →
//!   error；handler 内同型捕获且本 handler 体内无 `ygg.kv.set(<键>, x)`
//!   落库证据 → warning。kv 化证据 = kv.set 第二实参为同名变量（文本近似）。
//! - **R3** 见 [`crate::locate::find_goto_dot_calls`]；命中即跳过解析。

use full_moon::ast::{
    Expression, FunctionArgs, FunctionBody, FunctionCall, Index, LocalAssignment, Prefix, Suffix,
    Var,
};
use full_moon::node::Node;
use full_moon::tokenizer::{TokenReference, TokenType};
use full_moon::visitors::Visitor;

use crate::diagnostic::{code, Diagnostic, Severity};
use crate::locate::{find_goto_dot_calls, line_col};

/// 单文件 lint 入口（lib 稳定 API；bin 与测试共用，零 IO）。
pub fn lint_source(file: &str, source: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // ① R3 预扫（parser 之外）：命中即语法错误已成定局，跳过 R1/R2。
    for offset in find_goto_dot_calls(source) {
        let end = source[offset..]
            .find('(')
            .map_or(offset + 1, |relative| offset + relative + 1);
        let (line, column) = line_col(source, offset);
        diagnostics.push(Diagnostic {
            file: file.to_owned(),
            offset,
            end_offset: end,
            line,
            column,
            code: code::R3_GOTO_DOT_FORM,
            severity: Severity::Error,
            message: "`ygg.narrative.goto` 点形态在 Lua 5.4 是语法错误（goto 为保留字）".to_owned(),
            fix: "改用括号形态 ygg.narrative[\"goto\"](…)".to_owned(),
        });
    }
    if !diagnostics.is_empty() {
        return diagnostics;
    }

    // ② 解析 + AST 规则（R1/R2）。
    match full_moon::parse(source) {
        Ok(ast) => {
            let mut visitor = RuleVisitor::new(file, source);
            visitor.visit_ast(&ast);
            visitor.into_diagnostics()
        }
        Err(err) => {
            let messages: Vec<String> = err
                .iter()
                .map(|error| error.error_message().to_string())
                .collect();
            vec![Diagnostic {
                file: file.to_owned(),
                offset: 0,
                end_offset: 0,
                line: 1,
                column: 1,
                code: code::PARSE,
                severity: Severity::Error,
                message: format!("Lua 解析失败：{}", messages.join("；")),
                fix: "修复语法错误后重试（点形态 goto 已由 R3 预扫单列）".to_owned(),
            }]
        }
    }
}

/// handler 体内收集帧（调用序 / local spawn / kv 化证据）。
#[derive(Default)]
struct HandlerFrame {
    /// (kind, offset, end_offset)，按源序（含嵌套闭包——文本近似）。
    calls: Vec<(CallKind, usize, usize)>,
    /// 体内 local 捕获的 spawn 变量名与位置（R2 warning 判定用）。
    local_spawns: Vec<(String, usize, usize)>,
    /// 体内 `ygg.kv.set` 第二实参出现的变量名集合（kv 化证据）。
    kv_set_names: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CallKind {
    Emit,
    Choice,
}

enum HandlerState {
    Plain,
    Handler(HandlerFrame),
}

/// R1/R2 访问器：只在三类钩子分类收集，其余交给 Visit impl 自动下降。
/// 注意：full_moon 3.0 的 Visitor 各 hook 默认体为空，子节点遍历在
/// `impl Visit for X`（hook 先行 → 自动下降 → `*_end` 后钩）——覆写
/// 只收集，不手动下发孩子（手动下发会双重访问）。
struct RuleVisitor<'src> {
    source: &'src str,
    file: String,
    diagnostics: Vec<Diagnostic>,
    /// handler 注册调用挂起标记：随后进入的第一个 FunctionBody 即 handler 体。
    pending_handler: bool,
    /// FunctionBody 栈：Handler 帧 / Plain（普通嵌套块）。
    handler_stack: Vec<HandlerState>,
}

impl<'src> RuleVisitor<'src> {
    fn new(file: &str, source: &'src str) -> Self {
        Self {
            source,
            file: file.to_owned(),
            diagnostics: Vec::new(),
            pending_handler: false,
            handler_stack: Vec::new(),
        }
    }

    fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }

    fn push(
        &mut self,
        code_kind: &'static str,
        severity: Severity,
        message: String,
        fix: String,
        span: (usize, usize),
    ) {
        let (line, column) = line_col(self.source, span.0);
        self.diagnostics.push(Diagnostic {
            file: self.file.clone(),
            offset: span.0,
            end_offset: span.1,
            line,
            column,
            code: code_kind,
            severity,
            message,
            fix,
        });
    }

    /// handler 帧收口分析（R1 调用序 + R2 warning 档）。
    fn analyze_handler(&mut self, frame: HandlerFrame) {
        if let Some(choice_index) = frame
            .calls
            .iter()
            .position(|(kind, _, _)| *kind == CallKind::Choice)
        {
            for (kind, offset, end) in &frame.calls[..choice_index] {
                if *kind == CallKind::Emit {
                    self.push(
                        code::R1_EMIT_BEFORE_CHOICE,
                        Severity::Error,
                        "handler 体内 ygg.emit 出现在 ygg.choice 之前——choice 哨兵终结会静默丢弃先入队的派生事件".to_owned(),
                        "把 emit 挪到 ygg.choice_selected 回应 handler（该处无 choice 终结，FIFO 完整排空）；ygg.choice 必须是 handler 最后一个输出调用".to_owned(),
                        (*offset, *end),
                    );
                }
            }
        }
        for (name, offset, end) in &frame.local_spawns {
            if !frame.kv_set_names.iter().any(|kv_name| kv_name == name) {
                self.push(
                    code::R2_LOCAL_SPAWN,
                    Severity::Warning,
                    format!(
                        "handler 内 `local {name} = ygg.entity.spawn()` 未在本 handler 落 ygg.kv——读档后该 local 指向失效 store"
                    ),
                    format!(
                        "spawn 后立即 `ygg.kv.set(\"<键>\", {name})`，使用处从 kv 取回（读档后实体 id 稳定）"
                    ),
                    (*offset, *end),
                );
            }
        }
    }
}

impl<'src> Visitor for RuleVisitor<'src> {
    fn visit_function_call(&mut self, node: &FunctionCall) {
        if let Some(path) = call_path(node) {
            match path.as_str() {
                "ygg.emit" | "ygg.choice" => {
                    let kind = if path == "ygg.emit" {
                        CallKind::Emit
                    } else {
                        CallKind::Choice
                    };
                    if let Some(HandlerState::Handler(frame)) = self.handler_stack.last_mut() {
                        frame
                            .calls
                            .push((kind, node_span(node).0, node_span(node).1));
                    }
                }
                "ygg.kv.set" => {
                    if let Some(HandlerState::Handler(frame)) = self.handler_stack.last_mut() {
                        if let Some(name) = kv_set_second_arg_name(node) {
                            frame.kv_set_names.push(name);
                        }
                    }
                }
                "ygg.on" | "ygg.narrative.node" => {
                    self.pending_handler = true;
                }
                _ => {}
            }
        }
        // full_moon 3.0：下降由 Visit impl 在 hook 后自动进行（hook 默认体为空，
        // 子节点分派在各类型的 Visit impl）——此处只收集，不手动下发。
    }

    fn visit_local_assignment(&mut self, node: &LocalAssignment) {
        let names: Vec<String> = node
            .names()
            .iter()
            .map(|token| token.token().to_string())
            .collect();
        for (index, expression) in node.expressions().iter().enumerate() {
            if let Expression::FunctionCall(call) = expression {
                if call_path(call).as_deref() == Some("ygg.entity.spawn") {
                    let name = names.get(index).cloned().unwrap_or_default();
                    let span = node_span(call);
                    match self.handler_stack.last_mut() {
                        None => {
                            self.push(
                                code::R2_LOCAL_SPAWN,
                                Severity::Error,
                                format!(
                                    "实体 id 存入顶层 local `{name}`——读档后 chunk 重装载，local 与 store 脱节且顶层重跑会重复 spawn"
                                ),
                                "id 落 ygg.kv 持久化（spawn 加 kv 门防重复），handler 内从 kv 取回".to_owned(),
                                span,
                            );
                        }
                        Some(HandlerState::Handler(frame)) => {
                            frame.local_spawns.push((name, span.0, span.1));
                        }
                        Some(HandlerState::Plain) => {}
                    }
                }
            }
        }
        // full_moon 3.0：表达式子树由 Visit impl 自动遍历，此处只收集。
    }

    fn visit_function_body(&mut self, _node: &FunctionBody) {
        let state = if std::mem::take(&mut self.pending_handler) {
            HandlerState::Handler(HandlerFrame::default())
        } else {
            HandlerState::Plain
        };
        self.handler_stack.push(state);
        // full_moon 3.0：参数 + 块由 Visit impl 在本 hook 后自动遍历。
    }

    fn visit_function_body_end(&mut self, _node: &FunctionBody) {
        // 收口：handler 帧出栈分析（孩子已由 Visit impl 遍历完毕）。
        if let Some(HandlerState::Handler(frame)) = self.handler_stack.pop() {
            self.analyze_handler(frame);
        }
    }
}

/// Node → (start_bytes, end_bytes)（缺位回退 0——仅驱动 bug 时出现）。
fn node_span<N: Node>(node: &N) -> (usize, usize) {
    let start = node.start_position().map_or(0, |position| position.bytes());
    let end = node
        .end_position()
        .map_or(start, |position| position.bytes());
    (start, end)
}

/// FunctionCall → 点路径（`ygg.narrative["goto"]` 归一为 `ygg.narrative.goto`）；
/// 调用目标为表达式等不可解析形态 → None。对两种解析形态鲁棒：点链可能
/// 挂在 call 的 suffixes 上，也可能整体在其 prefix 的 VarExpression 内。
fn call_path(call: &FunctionCall) -> Option<String> {
    let mut parts = Vec::new();
    collect_prefix(call.prefix(), &mut parts);
    collect_suffix_index_names(call.suffixes(), &mut parts);
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("."))
    }
}

fn collect_prefix(prefix: &Prefix, parts: &mut Vec<String>) {
    match prefix {
        Prefix::Name(token) => parts.push(token.token().to_string()),
        Prefix::Expression(expression) => {
            if let Expression::Var(Var::Expression(var_expression)) = expression.as_ref() {
                collect_prefix(var_expression.prefix(), parts);
                collect_suffix_index_names(var_expression.suffixes(), parts);
            }
        }
        _ => {}
    }
}

fn collect_suffix_index_names<'a>(
    suffixes: impl Iterator<Item = &'a Suffix>,
    parts: &mut Vec<String>,
) {
    for suffix in suffixes {
        match suffix {
            Suffix::Index(Index::Dot { name, .. }) => parts.push(name.token().to_string()),
            Suffix::Index(Index::Brackets { expression, .. }) => {
                if let Expression::String(token) = expression.as_ref() {
                    parts.push(string_literal_value(token));
                }
            }
            // 调用边界（Call / MethodCall）：路径收集到此为止。
            _ => break,
        }
    }
}

/// 字符串字面量 → 去引号值（TokenType::StringLiteral 的 literal 已去引号）。
fn string_literal_value(token: &TokenReference) -> String {
    match token.token_type() {
        TokenType::StringLiteral { literal, .. } => literal.to_string(),
        _ => token.to_string(),
    }
}

/// `ygg.kv.set(<字面键>, <变量>)` 的第二实参变量名（kv 化证据，文本近似）。
fn kv_set_second_arg_name(call: &FunctionCall) -> Option<String> {
    for suffix in call.suffixes() {
        if let Suffix::Call(full_moon::ast::Call::AnonymousCall(args)) = suffix {
            if let FunctionArgs::Parentheses { arguments, .. } = args.as_ref() {
                if let Some(Expression::Var(Var::Name(token))) = arguments.iter().nth(1) {
                    return Some(token.token().to_string());
                }
                return None;
            }
        }
    }
    None
}
