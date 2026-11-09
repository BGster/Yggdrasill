//! 首批规则（FR-8）：R1 choice 前 emit / R2 local 存实体句柄 / R3 goto 点
//! 形态（预扫包装）+ parse-error 诊断。
//!
//! 判定口径（docs/DESIGN.md §3.1/§3.2 + test-adventure README 清单 1/4/8）：
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
    AnonymousFunction, Ast, Expression, FunctionArgs, FunctionBody, FunctionCall,
    FunctionDeclaration, Index, LocalAssignment, LocalFunction, Prefix, Suffix, Var, VarExpression,
};
use full_moon::node::Node;
use full_moon::tokenizer::{TokenReference, TokenType};
use full_moon::visitors::Visitor;

use ygg_modules::effects;

use crate::diagnostic::{code, Diagnostic, Severity};
use crate::locate::{find_goto_dot_calls, line_col};

/// 单文件 lint 入口（lib 稳定 API；bin 与测试共用，零 IO）。
pub fn lint_source(file: &str, source: &str) -> Vec<Diagnostic> {
    match parse_gate(file, source) {
        Ok(ast) => rules_for_ast(file, source, &ast),
        Err(diagnostics) => diagnostics,
    }
}

/// 解析门：R3 预扫（parser 之外）→ full_moon 解析。
///
/// 两个消费方共用同一道门：单文件 [`lint_source`] 与 `--pack` 的逐模块检查
/// （[`crate::pack`]）。R3 命中即语法错误已成定局（AST 在该处构不出来），此时
/// 只报 R3 不报 PARSE；其余语法错误按 full_moon 的错误区间给真实行列——多文件
/// Mod 的报错因此能指到具体模块的具体行，而不是「某个文件第 1 行」。
pub(crate) fn parse_gate(file: &str, source: &str) -> Result<full_moon::ast::Ast, Vec<Diagnostic>> {
    // ① R3 预扫（parser 之外）：命中即语法错误已成定局，跳过 R1/R2。
    let mut diagnostics = Vec::new();
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
        return Err(diagnostics);
    }

    // ② 解析（失败按错误区间定位）。
    match full_moon::parse(source) {
        Ok(ast) => Ok(ast),
        Err(err) => {
            let messages: Vec<String> = err
                .iter()
                .map(|error| error.error_message().to_string())
                .collect();
            let (start, end) = parse_error_span(source, &err);
            let (line, column) = line_col(source, start);
            Err(vec![Diagnostic {
                file: file.to_owned(),
                offset: start,
                end_offset: end,
                line,
                column,
                code: code::PARSE,
                severity: Severity::Error,
                message: format!("Lua 解析失败：{}", messages.join("；")),
                fix: "修复语法错误后重试（点形态 goto 已由 R3 预扫单列）".to_owned(),
            }])
        }
    }
}

/// 解析错误区间 → 夹进源码范围的 `(start, end)`。
///
/// full_moon 的 `Position::bytes()` 与 AST 节点同口径（`node_span` 用的也是它），
/// 所以这里不另做基准换算；只保证区间落进源码且非退化（空区间无法划线）。
fn parse_error_span(source: &str, errors: &[full_moon::Error]) -> (usize, usize) {
    let limit = source.len().max(1);
    let start = errors
        .iter()
        .map(|error| error.range().0.bytes())
        .min()
        .unwrap_or(0)
        .min(limit - 1);
    let end = errors
        .iter()
        .map(|error| error.range().1.bytes())
        .max()
        .unwrap_or(start + 1)
        .clamp(start + 1, limit);
    (start, end)
}

/// 已解析 AST 上的规则检查（R1/R2）。
///
/// 独立于解析：`--pack` 要在同一份 AST 上再抽一层依赖信息（`require` 字面量），
/// 一次 parse 做两份判定，不重复解析。
pub(crate) fn rules_for_ast(
    file: &str,
    source: &str,
    ast: &full_moon::ast::Ast,
) -> Vec<Diagnostic> {
    let mut visitor = RuleVisitor::new(file, source);
    visitor.visit_ast(ast);
    visitor.into_diagnostics()
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
///
/// `--pack` 的 `require` 扫描共用它（路径判定只有一份：`require` 与
/// `ygg.kv.set` 这类点链调用在 AST 上同形，没必要写第二套遍历）。
pub(crate) fn call_path(call: &FunctionCall) -> Option<String> {
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
                } else {
                    // 动态下标（`ygg[k]`）：静态不可判，路径到此为止。
                    // 继续往下拼会拼出一个**错的**路径（`ygg[k].set` 变成
                    // `ygg.set`），把「不知道」伪装成「知道」——那才是误报的来源。
                    break;
                }
            }
            // 调用边界（Call / MethodCall）：路径收集到此为止。
            _ => break,
        }
    }
}

/// 调用的**单一字符串实参**（`f("x")` 与 `f "x"` 两种形态）；其余形态 → None。
///
/// `require` 只认字面量：拼接出来的名字（`require("src." .. name)`）静态不可判，
/// 由 lint 侧按「非法/不可解析」提示，装载期再按真实字符串判定。
pub(crate) fn string_argument(call: &FunctionCall) -> Option<TokenReference> {
    for suffix in call.suffixes() {
        match suffix {
            Suffix::Call(full_moon::ast::Call::AnonymousCall(args)) => {
                return match args.as_ref() {
                    FunctionArgs::Parentheses { arguments, .. } => {
                        let mut arguments = arguments.iter();
                        match (arguments.next(), arguments.next()) {
                            (Some(Expression::String(token)), None) => Some(token.clone()),
                            _ => None,
                        }
                    }
                    FunctionArgs::String(token) => Some(token.clone()),
                    // 表构造实参（`require{...}`）或未来新增形态：都不是字面量。
                    _ => None,
                };
            }
            // 点链/索引后缀继续找调用位；其他后缀（方法调用等）不是本形态。
            Suffix::Index(_) => continue,
            _ => return None,
        }
    }
    None
}

/// 字符串字面量 → 去引号值（TokenType::StringLiteral 的 literal 已去引号）。
pub(crate) fn string_literal_value(token: &TokenReference) -> String {
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

// ---------------------------------------------------------------------------
// L1 主防线：模块 chunk 顶层不得触碰有副作用的 host API（`YGG-MOD-EFFECT`）
// ---------------------------------------------------------------------------

/// 模块 chunk 顶层（`function_depth == 0`）的禁止面检查（诊断码
/// [`code::MOD_EFFECT`]）。
///
/// **只给 `code.modules` 里的文件跑**：入口是装配清单，注册 / 初始化本来就是
/// 它的职责（[`crate::pack`] 挑文件进来，本函数不猜）。
///
/// 判定与装载期影子表（`ygg-sandbox` 的 `ShadowBanned`）**共用同一份清单**：
/// [`ygg_modules::effects::banned`] 是唯一查询面，这里不维护第二份名单——内核
/// 往禁止面加一个名字，这条规则自动跟着报（tests/pack.rs 有逐条驱动的回归）。
///
/// **取引用也算**：顶层出现 `ygg.on`（哪怕只是 `local f = ygg.on`）就已经违规
/// ——拿它的下一步只可能是注册，而顶层赋值在装载期就执行了；再传出去更是谁在
/// 什么时候注册都说不清。函数体内（`M.install(deps)`）则完全合法。
///
/// **已知局限**（AST 不做数据流跟踪）：别名逃逸（`local y = ygg; y.on(…)`）与
/// 动态下标（`ygg[k].on(…)`，`k` 非字面量）静态不可判；另外「函数体一律放行」
/// 分不出 `pcall(function() ygg.on(…) end)` 这种顶层立刻执行的包装。三种都交给
/// 装载期影子表兜底——那时 `ygg.on` 已经是拒绝桩，别名拿到的也是桩，绕不过去。
pub(crate) fn effects_for_ast(file: &str, source: &str, ast: &Ast) -> Vec<Diagnostic> {
    let mut visitor = EffectVisitor::new(file, source);
    visitor.visit_ast(ast);
    visitor.into_diagnostics()
}

/// 顶层的哪种触碰形态：调用与取引用报同一件事，但「做了什么」不同。
enum TouchForm {
    Call,
    Reference,
}

/// 顶层禁止面访问器：只维护函数深度，深度为 0 时判定调用 / 取引用。
///
/// 只管深度，不管「当前在哪个 chunk」：单文件 Mod 里顶层调 `ygg.on` 是正常
/// 写法（R1/R2 在那儿判 handler 内部），所以本访问器不进 [`lint_source`]。
struct EffectVisitor<'src> {
    source: &'src str,
    file: String,
    diagnostics: Vec<Diagnostic>,
    /// 函数嵌套深度：0 = 模块 chunk 顶层（装载期就会执行的面）。
    ///
    /// 三类函数节点都必须数：`function a.b()` / `local function f()` /
    /// `function() … end` 表达式。漏一类就会把「函数体内合法」误报成违规
    /// （例如 `M.install = function() ygg.on(…) end` 的赋值简写）。
    function_depth: usize,
}

impl<'src> EffectVisitor<'src> {
    fn new(file: &str, source: &'src str) -> Self {
        Self {
            source,
            file: file.to_owned(),
            diagnostics: Vec::new(),
            function_depth: 0,
        }
    }

    fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }

    fn report(&mut self, member: &str, form: TouchForm, span: (usize, usize)) {
        let (line, column) = line_col(self.source, span.0);
        let (message, fix) = effect_wording(member, form);
        self.diagnostics.push(Diagnostic {
            file: self.file.clone(),
            offset: span.0,
            end_offset: span.1.max(span.0 + 1),
            line,
            column,
            code: code::MOD_EFFECT,
            severity: Severity::Error,
            message,
            fix,
        });
    }
}

impl Visitor for EffectVisitor<'_> {
    fn visit_function_declaration(&mut self, _node: &FunctionDeclaration) {
        self.function_depth += 1;
    }

    fn visit_function_declaration_end(&mut self, _node: &FunctionDeclaration) {
        self.function_depth = self.function_depth.saturating_sub(1);
    }

    fn visit_local_function(&mut self, _node: &LocalFunction) {
        self.function_depth += 1;
    }

    fn visit_local_function_end(&mut self, _node: &LocalFunction) {
        self.function_depth = self.function_depth.saturating_sub(1);
    }

    fn visit_anonymous_function(&mut self, _node: &AnonymousFunction) {
        self.function_depth += 1;
    }

    fn visit_anonymous_function_end(&mut self, _node: &AnonymousFunction) {
        self.function_depth = self.function_depth.saturating_sub(1);
    }

    fn visit_function_call(&mut self, node: &FunctionCall) {
        if self.function_depth > 0 {
            return;
        }
        // 点链 / 字符串下标两种形态在 `call_path` 里已经归一
        // （`ygg["kv"]["set"](…)` → `ygg.kv.set`）；动态下标在拼路径处截断，
        // 因此这里拿到的要么是完整可判的字面路径，要么就是短的。
        let Some(path) = call_path(node) else {
            return;
        };
        let Some(member) = banned_member(&path) else {
            return;
        };
        self.report(&member, TouchForm::Call, node_span(node));
    }

    fn visit_var_expression(&mut self, node: &VarExpression) {
        if self.function_depth > 0 {
            return;
        }
        // 取引用（`local f = ygg.on` / `return ygg.kv.set` / `M.on = ygg.on`）。
        // 正常调用形态不会走到这里：`ygg.on(…)` 的 `ygg.on` 挂在 `FunctionCall`
        // 的 prefix+suffixes 上，不是独立的 VarExpression——所以调用与取引用
        // 各报一次，同一处不会叠两条。
        let Some(path) = var_expression_path(node) else {
            return;
        };
        let Some(member) = banned_member(&path) else {
            return;
        };
        self.report(&member, TouchForm::Reference, node_span(node));
    }
}

/// `VarExpression` → 点路径（与 [`call_path`] 同口径：点链与字符串下标都算）。
fn var_expression_path(node: &VarExpression) -> Option<String> {
    let mut parts = Vec::new();
    collect_prefix(node.prefix(), &mut parts);
    collect_suffix_index_names(node.suffixes(), &mut parts);
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("."))
    }
}

/// 路径 → 禁止面成员（`ygg.on` → `Some("on")`；`ygg.kv.set` → `Some("kv.set")`）。
///
/// 只认 `ygg` 根：`M.on = ygg.on` 左边的 `M.on` 不能被当成 host API。
/// 判定本身交给 [`effects::banned`]——那是内核影子表用的同一个查询面。
fn banned_member(path: &str) -> Option<String> {
    let mut segments = path.split('.');
    if segments.next()? != "ygg" {
        return None;
    }
    effects::banned(&segments.collect::<Vec<_>>())
}

/// 禁止面命中的文案：**做了什么 + 为什么**进 `message`，**该怎么做**进 `fix`。
///
/// 顶层仍允许的纯读面从清单现拼（[`effects::PURE_READ_CALLS`] /
/// [`effects::PURE_READ_TABLE_MEMBERS`]）：作者要知道「只有副作用面需要搬走」，
/// 而这份名字表不能在这里再抄一遍。
fn effect_wording(member: &str, form: TouchForm) -> (String, String) {
    const WHY: &str = "注册序就是派发序：模块自己注册，等于让「文件怎么拆」决定「运行时行为」，\
                       拆文件或调整 require 顺序都会改派发序，重构不再安全";
    let message = match form {
        TouchForm::Call => format!(
            "模块顶层调用了有副作用的 host API `ygg.{member}`：模块 chunk 在装载期执行，\
             注册 / 写状态 / 输出会随 require 一起发生。{WHY}"
        ),
        TouchForm::Reference => format!(
            "模块顶层取用了 `ygg.{member}` 的引用：拿它只可能是为了注册 / 写状态 / 输出，\
             而顶层代码在装载期就执行了。{WHY}"
        ),
    };
    let fix = format!(
        "模块顶层只留纯导出（函数定义与常量）：把这段搬进模块函数 \
         `function M.install(deps) … ygg.{member}(…) … end`，由入口（pack.json 的 \
         code.entry）require 后显式调用 `M.install(deps)`——函数体内调用完全合法。\
         顶层仍允许纯读面（{}）",
        pure_read_list()
    );
    (message, fix)
}

/// 顶层仍允许的纯读面清单（`ygg.data` / `ygg.kv.get` …），现拼不写死。
fn pure_read_list() -> String {
    effects::PURE_READ_CALLS
        .iter()
        .map(|name| format!("ygg.{name}"))
        .chain(
            effects::PURE_READ_TABLE_MEMBERS
                .iter()
                .map(|(table, member)| format!("ygg.{table}.{member}")),
        )
        .collect::<Vec<_>>()
        .join(" / ")
}
