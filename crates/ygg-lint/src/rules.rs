//! 规则面：解析门 + 沙箱白名单面 + 模块 chunk 顶层的副作用面。
//!
//! 三条真规则（`YGG-R1/R2/R3` 随 M1-E 删除事件层一起退役——判据的对象已不
//! 存在，不再有对应代码）：
//!
//! - **解析门**（[`code::PARSE`]）：full_moon 解析失败按错误区间报真实行列；
//! - **白名单面**（[`code::STD`]，单文件与 `--pack` 都跑）：sandbox 装配环境时
//!   摘掉的全局（`os` / `io` / `debug` / `package` …）、`math` 收窄表外的成员与
//!   浮点幂 `^` 一律报 error。M2.5 之前这条防线只存在于一个测试的小词法器里，
//!   测试删掉后就没人在编辑期挡它了——这里是它的正身（见 [`std_for_ast`]）；
//! - **纯导出面**（[`code::MOD_EFFECT`]，只给 `--pack` 的 `code.modules` 跑）：
//!   模块 chunk 顶层触碰有副作用的 host API 即报。
//!
//! 单文件 [`lint_source`] 做解析门 + 白名单面，不做副作用面：入口 chunk 就是
//! 装配清单，顶层写状态是它的职责（`world.spawn` 就写在 `mod.lua` 顶层）。

use full_moon::ast::{
    AnonymousFunction, Ast, BinOp, Expression, FunctionArgs, FunctionCall, FunctionDeclaration,
    Index, LocalFunction, Prefix, Suffix, Var, VarExpression,
};
use full_moon::node::Node;
use full_moon::tokenizer::{TokenReference, TokenType};
use full_moon::visitors::Visitor;

use ygg_modules::effects;

use crate::diagnostic::{code, Diagnostic, Severity};
use crate::locate::line_col;

/// 单文件 lint 入口（lib 稳定 API；bin 与测试共用，零 IO）。
///
/// 做两道面：解析门 + 白名单面（[`std_for_ast`]）。后者是纯单文件判定（不跨
/// 文件、不执行），所以单文件 Mod 与 `--pack` 的每个模块走同一份代码。副作用面
/// 属于整包（`--pack`），见 [`effects_for_ast`]。
pub fn lint_source(file: &str, source: &str) -> Vec<Diagnostic> {
    match parse_gate(file, source) {
        Ok(ast) => std_for_ast(file, source, &ast),
        Err(diagnostics) => diagnostics,
    }
}

/// 解析门：full_moon 解析，失败按错误区间给真实行列。
///
/// 两个消费方共用同一道门：单文件 [`lint_source`] 与 `--pack` 的逐模块检查
/// （[`crate::pack`]）。失败时明细指到具体行，而不是「某个文件第 1 行」——
/// 多文件 Mod 的报错因此可用。
pub(crate) fn parse_gate(file: &str, source: &str) -> Result<Ast, Vec<Diagnostic>> {
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
                fix: "修复语法错误后重试".to_owned(),
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

// ---------------------------------------------------------------------------
// L1 主防线：模块 chunk 顶层不得触碰有副作用的 host API（`YGG-MOD-EFFECT`）
// ---------------------------------------------------------------------------

/// 模块 chunk 顶层（`function_depth == 0`）的禁止面检查（诊断码
/// [`code::MOD_EFFECT`]）。
///
/// **只给 `code.modules` 里的文件跑**：入口是装配清单，初始化本来就是它的
/// 职责（[`crate::pack`] 挑文件进来，本函数不猜）。
///
/// 判定与装载期影子表（`ygg-sandbox` 的 `ShadowBanned`）**共用同一份清单**：
/// [`ygg_modules::effects::banned`] 是唯一查询面，这里不维护第二份名单——内核
/// 往禁止面加一个名字，这条规则自动跟着报（tests/pack.rs 有逐条驱动的回归）。
///
/// **取引用也算**：顶层出现 `ygg.kv.set`（哪怕只是 `local f = ygg.kv.set`）就
/// 已经违规——拿它的下一步只可能是写状态，而顶层赋值在装载期就执行了；再传
/// 出去更是谁在什么时候写都说不清。函数体内（`M.install(deps)`）则完全合法。
///
/// **已知局限**（AST 不做数据流跟踪）：别名逃逸（`local y = ygg.kv; y.set(…)`）
/// 与动态下标（`ygg[k].set(…)`，`k` 非字面量）静态不可判；另外「函数体一律
/// 放行」分不出 `pcall(function() ygg.kv.set(…) end)` 这种顶层立刻执行的包装。
/// 三种都交给装载期影子表兜底——那时 `ygg.kv.set` 已经是拒绝桩，别名拿到的
/// 也是桩，绕不过去。
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
/// 只管深度，不管「当前在哪个 chunk」：入口 chunk 的顶层写状态是正常写法
/// （`mod.lua` 就是装配清单），所以本访问器不进 [`lint_source`]。
struct EffectVisitor<'src> {
    source: &'src str,
    file: String,
    diagnostics: Vec<Diagnostic>,
    /// 函数嵌套深度：0 = 模块 chunk 顶层（装载期就会执行的面）。
    ///
    /// 三类函数节点都必须数：`function a.b()` / `local function f()` /
    /// `function() … end` 表达式。漏一类就会把「函数体内合法」误报成违规
    /// （例如 `M.install = function() ygg.kv.set(…) end` 的赋值简写）。
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
        // 取引用（`local f = ygg.kv.set` / `return ygg.kv.set` / `M.set = ygg.kv.set`）。
        // 正常调用形态不会走到这里：`ygg.kv.set(…)` 的 `ygg.kv.set` 挂在
        // `FunctionCall` 的 prefix+suffixes 上，不是独立的 VarExpression——所以
        // 调用与取引用各报一次，同一处不会叠两条。
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

/// 路径 → 禁止面成员（`ygg.kv.set` → `Some("kv.set")`）。
///
/// 只认 `ygg` 根：`M.set = ygg.kv.set` 左边的 `M.set` 不能被当成 host API。
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
/// [`effects::PURE_READ_TABLE_MEMBERS`]）：作者要知道「只有副作用面需要搬
/// 走」，而这份名字表不能在这里再抄一遍。
fn effect_wording(member: &str, form: TouchForm) -> (String, String) {
    const WHY: &str = "模块 chunk 在装载期执行，顶层写下的状态既不进世界帧也不进存档，\
                       还会随 require 顺序变化——拆文件或调换 require 顺序就会改装载期状态，\
                       重构不再安全";
    let message = match form {
        TouchForm::Call => format!(
            "模块顶层调用了有副作用的 host API `ygg.{member}`：模块 chunk 在装载期执行，\
             写状态会随 require 一起发生。{WHY}"
        ),
        TouchForm::Reference => format!(
            "模块顶层取用了 `ygg.{member}` 的引用：拿它只可能是为了写状态，\
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

// ---------------------------------------------------------------------------
// YGG-STD：沙箱白名单面（全局库 / `math` 收窄表 / 浮点幂）
// ---------------------------------------------------------------------------

/// 沙箱里**根本不存在**的全局库与自由函数（与 `ygg-sandbox` 的
/// `whitelisted_environment_exposes_only_the_allow_list` 同一张面）：装载期
/// 构造环境时逐个摘掉，运行期取到的是 nil——所以 `os.remove()` 在 pack 里不是
/// 「危险」而是**走不到**，静态先挡是为了不让它跑到运行期才炸。
///
/// 字典序。`require` 不在表里：它是受控的模块装载入口，只在
/// `require("src.util")` 这样的字面量调用位合法（见 [`StdFinding::RequireAlias`]）。
const BANNED_GLOBALS: &[&str] = &[
    "collectgarbage",
    "coroutine",
    "debug",
    "dofile",
    "io",
    "load",
    "loadfile",
    "loadstring",
    "os",
    "package",
];

/// `math` 收窄表里被结构性删除的成员（权威清单 = `ygg-sandbox` 的
/// `math_table_is_exact_op_whitelist`）：超越函数与 `sqrt` 走 libm，native 与
/// wasm 的结果位不一致，定量只许定点 API。
const REMOVED_MATH_MEMBERS: &[&str] = &[
    "acos",
    "asin",
    "atan",
    "cos",
    "deg",
    "exp",
    "log",
    "log10",
    "pow",
    "rad",
    "randomseed",
    "sin",
    "sqrt",
    "tan",
];

/// 命中面：决定文案与定位点，不影响是否报。
enum StdFinding {
    /// 白名单外的全局（`os` / `io` / `debug` …）。
    Global(&'static str),
    /// `math` 收窄表外的成员（`math.sin` → `sin`）。
    MathMember(&'static str),
    /// `require` 被当值取用（`local r = require`）：别名绕开模块白名单。
    RequireAlias,
    /// `require` 的实参不是字符串字面量：名字静态不可判。
    RequireComputed,
    /// 浮点幂 `^`（恒返 f64，底层是 libm pow）。
    Power,
}

/// 沙箱白名单面的静态检查（诊断码 [`code::STD`]）。
///
/// **判据在 AST 上，不在文本上**：注释（`--` / `--[[ ]]`）与长短字符串都是
/// 独立 token，天然不参与判定；`table.io` 的 `io` 是点索引的名字 token，不是
/// `Prefix::Name`，所以也不会误报——这三类恰好是逐行正则 / 手写词法器最容易
/// 出错的地方（M2.5 删掉的旧测试词法器就是逐行扫的）。
///
/// **已知局限**（AST 不做数据流跟踪）：`local m = math; m.sin(x)` 这类别名
/// 逃逸静态不可判，`math[name]` 动态下标同理；别名拿到的 `os` / `io` 在沙箱
/// 里本来就是 nil，装载期白名单仍是最后一道。
pub(crate) fn std_for_ast(file: &str, source: &str, ast: &Ast) -> Vec<Diagnostic> {
    let mut visitor = StdVisitor::new(file, source);
    visitor.visit_ast(ast);
    visitor.into_diagnostics()
}

struct StdVisitor<'src> {
    source: &'src str,
    file: String,
    diagnostics: Vec<Diagnostic>,
}

impl<'src> StdVisitor<'src> {
    fn new(file: &str, source: &'src str) -> Self {
        Self {
            source,
            file: file.to_owned(),
            diagnostics: Vec::new(),
        }
    }

    fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }

    fn report(&mut self, finding: StdFinding, span: (usize, usize)) {
        let (line, column) = line_col(self.source, span.0);
        let (message, fix) = std_wording(&finding);
        self.diagnostics.push(Diagnostic {
            file: self.file.clone(),
            offset: span.0,
            end_offset: span.1.max(span.0 + 1),
            line,
            column,
            code: code::STD,
            severity: Severity::Error,
            message,
            fix,
        });
    }

    /// 点路径 → 违规面；定位点优先用根名字 token（`os` / `math`）或点成员
    /// token（`sin`），都取不到（括号包裹等）就退回整个节点的区间。
    fn check_path(
        &mut self,
        path: &str,
        root: Option<&TokenReference>,
        member: Option<&TokenReference>,
        fallback: (usize, usize),
    ) {
        let Some(finding) = std_finding(path) else {
            return;
        };
        let span = match &finding {
            StdFinding::Global(_) => root.map_or(fallback, node_span),
            StdFinding::MathMember(_) => member.or(root).map_or(fallback, node_span),
            _ => fallback,
        };
        self.report(finding, span);
    }
}

impl Visitor for StdVisitor<'_> {
    fn visit_function_call(&mut self, node: &FunctionCall) {
        let Some(path) = call_path(node) else {
            return;
        };
        // `require` 是受控装载入口：字面量实参的调用位合法（模块名形态由
        // YGG-MOD-NAME 判，这里不重复报同一个问题）；其余形态都绕开白名单。
        if root_name(&path) == "require" {
            if string_argument(node).is_none() {
                self.report(StdFinding::RequireComputed, node_span(node));
            }
            return;
        }
        self.check_path(
            &path,
            prefix_name_token(node.prefix()),
            first_dot_member(node.suffixes()),
            node_span(node),
        );
    }

    fn visit_var_expression(&mut self, node: &VarExpression) {
        let Some(path) = var_expression_path(node) else {
            return;
        };
        // 取引用（`local f = os.remove` / `local r = require`）：与调用走同一
        // 张名单，但 `require` 在这里没有「调用位」可言——拿到手里就是绕过。
        if root_name(&path) == "require" {
            let span = prefix_name_token(node.prefix()).map_or_else(|| node_span(node), node_span);
            self.report(StdFinding::RequireAlias, span);
            return;
        }
        self.check_path(
            &path,
            prefix_name_token(node.prefix()),
            first_dot_member(node.suffixes()),
            node_span(node),
        );
    }

    fn visit_var(&mut self, node: &Var) {
        // 裸名字（`local x = os` / `{ os }` / `t[os]`）：没有后缀可走
        // `check_path`，但间接持有整张全局表同样能绕过后面的成员检查。
        let Var::Name(token) = node else {
            return;
        };
        let name = token.token().to_string();
        if name == "require" {
            self.report(StdFinding::RequireAlias, node_span(token));
            return;
        }
        self.check_path(&name, Some(token), None, node_span(token));
    }

    fn visit_expression(&mut self, node: &Expression) {
        // `^` 是唯一一条语法层的判定：`math.pow` 删了不代表路就断了。
        if let Expression::BinaryOperator {
            binop: BinOp::Caret(token),
            ..
        } = node
        {
            self.report(StdFinding::Power, node_span(token));
        }
    }
}

/// 点路径 → 违规面（不含 `require`：它多一个「字面量调用位合法」态，由调用点判）。
fn std_finding(path: &str) -> Option<StdFinding> {
    let mut segments = path.split('.');
    let root = segments.next()?;
    if let Some(name) = BANNED_GLOBALS.iter().copied().find(|name| *name == root) {
        return Some(StdFinding::Global(name));
    }
    if root == "math" {
        let member = segments.next()?;
        if let Some(member) = REMOVED_MATH_MEMBERS
            .iter()
            .copied()
            .find(|name| *name == member)
        {
            return Some(StdFinding::MathMember(member));
        }
    }
    None
}

/// 点路径的根段（`math.sin` → `math`）。
fn root_name(path: &str) -> &str {
    path.split('.').next().unwrap_or("")
}

/// `Prefix` → 根名字 token（`foo.bar.baz` → `foo`；括号包裹的继续下钻）。
fn prefix_name_token(prefix: &Prefix) -> Option<&TokenReference> {
    match prefix {
        Prefix::Name(token) => Some(token),
        Prefix::Expression(expression) => match expression.as_ref() {
            Expression::Var(Var::Expression(var_expression)) => {
                prefix_name_token(var_expression.prefix())
            }
            _ => None,
        },
        _ => None,
    }
}

/// suffixes 里第一个点索引的名字 token（`math.sin` → `sin`）；调用 / 括号边界
/// → None（那之后的成员属于调用结果，不是全局表）。
fn first_dot_member<'a>(suffixes: impl Iterator<Item = &'a Suffix>) -> Option<&'a TokenReference> {
    suffixes.into_iter().find_map(|suffix| match suffix {
        Suffix::Index(Index::Dot { name, .. }) => Some(name),
        _ => None,
    })
}

/// 命中面 →（message：做了什么 + 为什么；fix：该怎么做）。
fn std_wording(finding: &StdFinding) -> (String, String) {
    match finding {
        StdFinding::Global(name) => (
            format!(
                "引用了沙箱不暴露的全局 `{name}`：装载期环境按白名单构造，\
                 白名单外的库在这里是 nil——静态不挡，运行期才会在调用点炸"
            ),
            "删掉这处引用：纯计算用 `string` / `table` / `utf8` / `math` 收窄表的成员，\
             世界读写走 `ygg` / `world`；拆模块用 `require(\"<模块名>\")` 字面量调用位"
                .to_owned(),
        ),
        StdFinding::MathMember(member) => (
            format!(
                "使用了 `math` 收窄表外的 `math.{member}`：超越函数与开方走 libm，\
                 native 与 wasm 的结果位不一致，定量只许定点 API"
            ),
            "改用 `math` 收窄表的成员（abs / ceil / floor / fmod / modf / tointeger / ult / \
             min / max / random / type）或定点 API（如 `world.isqrt`）"
                .to_owned(),
        ),
        StdFinding::RequireAlias => (
            "把 `require` 当值取用：它只在 `require(\"<模块名>\")` 这样的字面量调用位\
             受控装载，取到别名就绕开了模块白名单（装载器只认这个入口）"
                .to_owned(),
            "改成 `require(\"src.util\")` 这样的字面量调用位写法".to_owned(),
        ),
        StdFinding::RequireComputed => (
            "`require` 的实参不是字符串字面量：名字静态不可判，装载期无法按模块表\
             校验，拼接出来的名字会绕过白名单"
                .to_owned(),
            "把模块名写成字符串字面量（如 `require(\"src.util\")`）".to_owned(),
        ),
        StdFinding::Power => (
            "使用了浮点幂运算符 `^`：它恒返回 f64 且底层是 libm pow，\
             native 与 wasm 的结果位不一致——整数 / 定点运算里没有它的位置"
                .to_owned(),
            "改写成整数连乘，或用定点开方 API（如 `world.isqrt`）".to_owned(),
        ),
    }
}
