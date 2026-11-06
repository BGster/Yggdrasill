//! ygg-modules：Mod 代码模块的**纯解析面**（零依赖——无 Lua、无 IO、无 serde）。
//!
//! 这里是「什么是合法的模块名 / 模块路径 / 模块集合 / 装载栈」的**唯一实现**。
//! 三个消费方共用它，因此不会出现「装载能过但静态检查判非法」这类双份口径：
//!
//! - `ygg_sandbox::modules`：装载器（`require` 闭包 + 缓存 + 副作用影子表）的
//!   Lua 面留在该 crate 的 `binding` 子模块里，判定面全部来自本 crate；
//! - `ygg_sandbox::pack`：打包期把 `pack.json` 的 `code` 声明对成 `ModuleSet`；
//! - `ygg-lint --pack`：静态检查逐模块 parse + 依赖图。
//!
//! 另有一个**共用清单**：[`effects`]——「模块顶层不得触碰的 `ygg` 面」。
//! 内核的影子表与 lint 的 `YGG-MOD-EFFECT` 规则都只读它，不各自维护一份。
//!
//! **为什么要独立成 crate**：最后一条是唯一理由。lint 的耦合纪律是「独立于
//! 管线与内核运行」（`ygg-lint/Cargo.toml` 的依赖白名单），若为了复用这几个
//! 判定函数而依赖 `ygg-sandbox`，lint 会被拖进 mlua（git pin + vendored Lua
//! C 构建）、ygg-schema 与整个 host 面——为 300 行纯函数付一整个内核的构建
//! 代价，也把「lint 可以脱离内核单跑」这条性质弄丢。反过来把判定抄一份进
//! lint 则是这套代码最忌讳的双份口径（装载期说合法、lint 说非法，两边都
//! 「有理由」）。所以判定下沉、两个上层都只调用。
//!
//! `ygg-sandbox` 对解析面做 `pub use` 全量再导出，`ygg_sandbox::modules::*`
//! 路径与语义逐字节不变，既有调用（含测试）零迁移。
//!
//! # 名字与路径的双射
//!
//! 模块名的规范形式是点分标识符串：`^[A-Za-z0-9_]+(\.[A-Za-z0-9_]+)*$`，
//! 每一段 ≤ [`MAX_SEGMENT_LEN`]，整体 ≤ [`MAX_NAME_LEN`]。规范名 `src.util`
//! 映射到相对路径 `src/util.lua`（[`path_of`]），反向由 [`name_of_path`] 给出，
//! 且 **`name_of_path(path_of(n)) == n` 恒成立**（`ygg-sandbox` 侧
//! `tests/modules_unit.rs` 断言）。
//!
//! **文件名段禁止 `.`**（除末尾 `.lua` 扩展名）：否则 `src/a.b.lua` 与
//! `src/a/b.lua` 会落到同一个点分名（`src.a.b`），缓存 key 不再唯一——
//! 同名不同体是「装载序决定谁生效」这类不确定性 bug 的温床。[`name_of_path`]
//! 因此对段内 `.` 直接拒绝（[`ModuleError::AmbiguousSegment`]）；点分名一侧
//! 天然满足该约束（`.` 就是分隔符）。
//!
//! 装载期只查**已登记**的模块表，不做任何路径拼接——名字校验与路径映射是
//! 打包/检查期的事，装载期拿到的已经是字节码表；这既杜绝路径穿越（`..` /
//! 绝对路径根本进不来），也让「装载」保持纯查表（无文件系统、无字符串拼接 ⇒
//! 无平台差异）。

use std::collections::BTreeMap;
use std::fmt;

// ───────────────────────── ① 纯解析面（零 Lua 依赖） ─────────────────────────

/// 模块表规模上限（[`ModuleSet`] 可登记的模块数）。
pub const MAX_MODULES: usize = 128;

/// 装载深度上限（同时存活的嵌套 require 层数；显式判栈，见 [`LoadStack`]）。
pub const MAX_DEPTH: usize = 64;

/// 单个模块源码字节上限（1 MiB）。
pub const MAX_CODE_BYTES: usize = 1024 * 1024;

/// 模块名整体长度上限（含点分隔符）。
pub const MAX_NAME_LEN: usize = 256;

/// 模块名单段长度上限。
pub const MAX_SEGMENT_LEN: usize = 64;

/// 模块源码扩展名（[`path_of`] 追加、[`name_of_path`] 剥离）。
pub const MODULE_EXTENSION: &str = ".lua";

/// Mod 模块的**副作用边界**（单一事实源）。
///
/// 新不变量：**模块 chunk 的顶层是纯导出面**——只允许 `local` / 函数定义 /
/// `require` 其它纯模块 / 纯读。任何**注册进内核**（`ygg.on` / `fsm.define` /
/// `narrative.node`）、**写宿主状态**（`kv.set` / `entity.*` 写类 / `fs.write`）或
/// **产生输出**（`narrate` / `emit` / `choice` / `meta` / `narrative.goto`）的
/// 调用都必须发生在函数体内，由入口显式装配调用。
///
/// **为什么**：`ygg.on` 的注册序就是 handler 的派发序。模块顶层能自由注册，
/// 等于让「文件怎么拆」决定「运行时行为」——重构（把函数挪到另一个文件、
/// 调换 require 顺序）就不再安全。模块纯了以后，`require` 才可以在任何时机
/// 调用（包括事件 handler 内部），懒加载与按需装载才成立。
///
/// **入口 chunk 不受这份清单约束**：入口本来就是装配清单，要注册、要初始化。
///
/// 这份清单被两个消费方共用，**不得各写一份**（否则「运行期放过的写法被 lint
/// 判非法」这类双份口径必然出现）：
/// - 内核：`ygg-sandbox::modules` 的副作用影子表——模块 chunk 执行期间把这些
///   面原位替换成拒绝桩，执行完恢复；
/// - 静态检查：`ygg-lint` 的 `YGG-MOD-EFFECT` 规则——模块顶层触及即报。
pub mod effects {
    /// 顶层调用即违规的 `ygg` 直系成员。
    ///
    /// `on` 是注册；`narrate` / `emit` / `choice` 是输出；`meta` 是宿主回执
    /// 通道（宿主据此判定，属输出类）。
    pub const BANNED_CALLS: &[&str] = &["on", "emit", "narrate", "choice", "meta"];

    /// 顶层调用即违规的 `ygg` 子表成员：(子表名, 成员名)。
    ///
    /// `fsm` / `narrative` **不是** Rust 侧 `raw_set` 的成员——它们是 stdlib
    /// 在装载期用 Lua 声明出来的表（`stdlib/init.lua` 建命名空间，`fsm.lua` /
    /// `narrative.lua` 挂成员）。影子表与 lint 一样按「子表 + 成员」处理，
    /// 不把它当成 `ygg` 的直系函数。
    ///
    /// `narrative` 的 `goto` 走 `ygg.narrative["goto"](id)`（`goto` 是 Lua 5.4
    /// 保留字，点号形态是语法错误）。它经 `ygg.emit` 发派生事件，与 `emit`
    /// 同属输出面；单独列出是为了让报错落在**调用点**（否则错误只报
    /// `ygg.emit`、位置落在 stdlib 的 `goto` 体内，作者看不出自己写了什么）。
    pub const BANNED_TABLE_MEMBERS: &[(&str, &str)] = &[
        ("kv", "set"),
        ("entity", "spawn"),
        ("entity", "set_attribute"),
        ("entity", "add_tag"),
        ("entity", "remove_tag"),
        // 写宿主状态（与 `kv.set` 同类：一次调用就产生跨事件的副作用）；
        // `fs.read` / `fs.stat` / `fs.list` 是纯读，不列（见下面 PURE_READ 表）。
        ("fs", "write"),
        ("fsm", "define"),
        ("narrative", "node"),
        ("narrative", "goto"),
    ];

    /// 纯读面——显式列出，作为「这些是**允许**的」的依据（lint 与文档用）。
    ///
    /// 未列进禁止面的名字一律放行；这里只钉住「顶层读它不改变任何状态」的
    /// 那几个，免得作者的合法写法被误判。
    ///
    /// 注意 `ygg.stdlib_version` / `ygg.engine_version` 是**值字段**不是函数
    /// （见 `stdlib.rs` / `sandbox.rs`），读字段天然无副作用，不需要列进
    /// 「可调用面」。
    pub const PURE_READ_CALLS: &[&str] = &["data", "ticks"];

    /// 纯读的子表成员（**允许**在顶层调用；miss 返回 `nil` / `false`，不写状态）。
    pub const PURE_READ_TABLE_MEMBERS: &[(&str, &str)] = &[
        ("kv", "get"),
        ("entity", "has_tag"),
        ("entity", "get_attribute"),
        // `ygg.fs` 的读面（这三条不列进 `banned()` 也已是放行态；列在这里是为了
        // 让 `YGG-MOD-EFFECT` 的报错能点名「这些在顶层是合法的」，免得作者把
        // 合法写法当成违规自己改掉）。`fs.write` 不在其中——它列进了禁止面。
        ("fs", "read"),
        ("fs", "stat"),
        ("fs", "list"),
    ];

    /// 禁止面判定（**唯一查询面**，内核影子表与 lint 共用）。
    ///
    /// `path` = `ygg` 之后的成员路径：`["on"]` 命中直系成员，
    /// `["kv", "set"]` 命中子表成员。命中返回规范访问形态（`on` /
    /// `kv.set`，供报错文案直接拼成 `ygg.kv.set`），未命中 = `None`
    /// （含纯读面与一切未列出的名字）。
    pub fn banned(path: &[&str]) -> Option<String> {
        match path {
            [name] => BANNED_CALLS.contains(name).then(|| (*name).to_owned()),
            [table, member] => BANNED_TABLE_MEMBERS
                .contains(&(*table, *member))
                .then(|| format!("{table}.{member}")),
            _ => None,
        }
    }
}

/// 模块名 / 路径 / 装载边的拒绝原因（装载器自判面；模块 chunk 自身的错误
/// 不在此列——那是 Lua 原生错误链，按原文浮出）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleError {
    /// 空名 / 空路径。
    EmptyName,
    /// 名字整体超长。
    NameTooLong { len: usize, limit: usize },
    /// 空段（`a..b` / `.a` / `a.` / `a//b.lua` 等）。
    EmptySegment { input: String },
    /// 单段超长。
    SegmentTooLong {
        segment: String,
        len: usize,
        limit: usize,
    },
    /// 段内出现标识符外字符。
    InvalidChar { segment: String, ch: char },
    /// 含 NUL 字节。
    NulChar,
    /// 含反斜杠（只接受点分 / 正斜杠形态）。
    Backslash { input: String },
    /// 绝对路径（前导 `/` 或盘符前缀）——模块名恒为相对名。
    Absolute { input: String },
    /// 路径缺 `.lua` 扩展名。
    NotLuaFile { path: String },
    /// 路径段内含 `.`（`src/a.b.lua`）——与 `src/a/b.lua` 撞名，破坏双射。
    AmbiguousSegment { path: String, segment: String },
    /// 同一规范名重复登记。
    Duplicate { name: String },
    /// 模块数超过 [`MAX_MODULES`]。
    TooManyModules { count: usize, limit: usize },
    /// 单模块源码超过 [`MAX_CODE_BYTES`]。
    CodeTooLarge {
        name: String,
        len: usize,
        limit: usize,
    },
    /// 规范名未在模块表登记（装载期只查表，不做路径拼接）。
    NotFound { name: String, chain: String },
    /// 循环依赖（链条从首次出现处起算，末端回到起点）。
    Circular { chain: String },
    /// 装载深度超过 [`MAX_DEPTH`]。
    DepthExceeded { chain: String, limit: usize },
    /// 装载器内部面（registry 表访问等）失败——出现即装载器 bug。
    Loader { details: String },
}

impl fmt::Display for ModuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModuleError::EmptyName => write!(f, "module name is empty"),
            ModuleError::NameTooLong { len, limit } => {
                write!(f, "module name is too long: {len} bytes (limit {limit})")
            }
            ModuleError::EmptySegment { input } => {
                write!(f, "module name has an empty segment: '{input}'")
            }
            ModuleError::SegmentTooLong {
                segment,
                len,
                limit,
            } => write!(
                f,
                "module name segment '{segment}' is too long: {len} bytes (limit {limit})"
            ),
            ModuleError::InvalidChar { segment, ch } => write!(
                f,
                "module name segment '{segment}' contains a forbidden character {ch:?} \
                 (allowed: A-Z a-z 0-9 _)"
            ),
            ModuleError::NulChar => write!(f, "module name contains a NUL byte"),
            ModuleError::Backslash { input } => write!(
                f,
                "module name must be dotted (e.g. src.util): '{input}' contains a backslash"
            ),
            ModuleError::Absolute { input } => write!(
                f,
                "module name must be relative (no leading '/' or drive prefix): '{input}'"
            ),
            ModuleError::NotLuaFile { path } => write!(
                f,
                "module path must end with '{MODULE_EXTENSION}': '{path}'"
            ),
            ModuleError::AmbiguousSegment { path, segment } => write!(
                f,
                "module path segment '{segment}' contains '.' ({path}): filename segments \
                 must not carry dots, otherwise src/a.b.lua and src/a/b.lua collide on one \
                 dotted module name"
            ),
            ModuleError::Duplicate { name } => {
                write!(f, "module '{name}' is registered more than once")
            }
            ModuleError::TooManyModules { count, limit } => {
                write!(f, "too many modules: {count} (limit {limit})")
            }
            ModuleError::CodeTooLarge { name, len, limit } => write!(
                f,
                "module '{name}' source is too large: {len} bytes (limit {limit})"
            ),
            ModuleError::NotFound { name, chain } => write!(
                f,
                "sandbox denied: module '{name}' is not available (require chain: {chain}; \
                 only the mod's own registered module set is loadable)"
            ),
            ModuleError::Circular { chain } => write!(f, "circular require: {chain}"),
            ModuleError::DepthExceeded { chain, limit } => write!(
                f,
                "require depth exceeded ({limit} nested modules): {chain}"
            ),
            ModuleError::Loader { details } => {
                write!(f, "module loader internal error: {details}")
            }
        }
    }
}

impl std::error::Error for ModuleError {}

/// 校验并返回规范模块名（点分标识符串）。
///
/// 通过即「规范名」：`normalize(normalize(x)) == normalize(x)`，[`path_of`] /
/// [`name_of_path`] 的双射以规范名为域。
pub fn normalize(name: &str) -> Result<String, ModuleError> {
    if name.is_empty() {
        return Err(ModuleError::EmptyName);
    }
    if name.contains('\0') {
        return Err(ModuleError::NulChar);
    }
    if name.contains('\\') {
        return Err(ModuleError::Backslash {
            input: name.to_owned(),
        });
    }
    if is_absolute(name) {
        return Err(ModuleError::Absolute {
            input: name.to_owned(),
        });
    }
    if name.len() > MAX_NAME_LEN {
        return Err(ModuleError::NameTooLong {
            len: name.len(),
            limit: MAX_NAME_LEN,
        });
    }
    for segment in name.split('.') {
        validate_segment(segment, name)?;
    }
    Ok(name.to_owned())
}

/// 规范名 → 相对路径（`src.util` → `src/util.lua`）。
pub fn path_of(name: &str) -> Result<String, ModuleError> {
    let canonical = normalize(name)?;
    Ok(format!("{}{MODULE_EXTENSION}", canonical.replace('.', "/")))
}

/// 相对路径 → 规范名（`src/util.lua` → `src.util`）；[`path_of`] 的逆。
///
/// 拒绝：绝对路径、反斜杠、NUL、非 `.lua`、空段、段内 `.`（[`ModuleError::
/// AmbiguousSegment`]）、标识符外字符、超长。`..` / `.` 这类路径段因此天然
/// 被段内 `.` 规则挡下——装载侧不存在路径穿越面。
pub fn name_of_path(path: &str) -> Result<String, ModuleError> {
    if path.is_empty() {
        return Err(ModuleError::EmptyName);
    }
    if path.contains('\0') {
        return Err(ModuleError::NulChar);
    }
    if path.contains('\\') {
        return Err(ModuleError::Backslash {
            input: path.to_owned(),
        });
    }
    if is_absolute(path) {
        return Err(ModuleError::Absolute {
            input: path.to_owned(),
        });
    }
    let stem = path
        .strip_suffix(MODULE_EXTENSION)
        .ok_or_else(|| ModuleError::NotLuaFile {
            path: path.to_owned(),
        })?;
    let mut segments = Vec::new();
    for segment in stem.split('/') {
        if segment.is_empty() {
            return Err(ModuleError::EmptySegment {
                input: path.to_owned(),
            });
        }
        if segment.contains('.') {
            return Err(ModuleError::AmbiguousSegment {
                path: path.to_owned(),
                segment: segment.to_owned(),
            });
        }
        validate_segment(segment, path)?;
        segments.push(segment);
    }
    normalize(&segments.join("."))
}

/// 前导 `/` 或 `C:` 形态一律视为绝对路径（模块名恒为相对名）。
fn is_absolute(input: &str) -> bool {
    if input.starts_with('/') {
        return true;
    }
    let bytes = input.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// 单段校验：非空、≤ [`MAX_SEGMENT_LEN`]、字符集 `[A-Za-z0-9_]`。
fn validate_segment(segment: &str, input: &str) -> Result<(), ModuleError> {
    if segment.is_empty() {
        return Err(ModuleError::EmptySegment {
            input: input.to_owned(),
        });
    }
    if segment.len() > MAX_SEGMENT_LEN {
        return Err(ModuleError::SegmentTooLong {
            segment: segment.to_owned(),
            len: segment.len(),
            limit: MAX_SEGMENT_LEN,
        });
    }
    for ch in segment.chars() {
        if !(ch.is_ascii_alphanumeric() || ch == '_') {
            return Err(ModuleError::InvalidChar {
                segment: segment.to_owned(),
                ch,
            });
        }
    }
    Ok(())
}

/// 装载栈：循环与深度判定的唯一数据结构（纯 Rust，无 Lua）。
///
/// 语义：`push` 先判循环（命中已存活帧即 [`ModuleError::Circular`]，链条从
/// 首次出现处起算），再判深度（`frames.len() >= MAX_DEPTH` 即
/// [`ModuleError::DepthExceeded`]）；两种拒绝都**不改动栈态**。`pop` 与
/// `push` 成对（装载器在 chunk 执行前后各一次，成败都出栈）。
#[derive(Debug, Default, Clone)]
pub struct LoadStack {
    frames: Vec<String>,
}

impl LoadStack {
    /// 空栈。
    pub fn new() -> Self {
        Self::default()
    }

    /// 入栈一层：循环 → 深度 → 入栈（失败不入栈）。
    pub fn push(&mut self, name: &str) -> Result<(), ModuleError> {
        if let Some(start) = self.frames.iter().position(|frame| frame == name) {
            let mut cycle: Vec<&str> = self.frames[start..].iter().map(String::as_str).collect();
            cycle.push(name);
            return Err(ModuleError::Circular {
                chain: cycle.join(" -> "),
            });
        }
        if self.frames.len() >= MAX_DEPTH {
            let mut chain: Vec<&str> = self.frames.iter().map(String::as_str).collect();
            chain.push(name);
            return Err(ModuleError::DepthExceeded {
                chain: elide_chain(&chain),
                limit: MAX_DEPTH,
            });
        }
        self.frames.push(name.to_owned());
        Ok(())
    }

    /// 出栈一层（与 [`LoadStack::push`] 成对；空栈为 no-op）。
    pub fn pop(&mut self) {
        self.frames.pop();
    }

    /// 当前装载深度（存活帧数）。
    pub fn depth(&self) -> usize {
        self.frames.len()
    }

    /// 当前存活帧（自外向内）。
    pub fn frames(&self) -> &[String] {
        &self.frames
    }

    /// 以 `pending` 为末端的 require 链文本（拒绝时报告完整链条）。
    pub fn chain(&self, pending: &str) -> String {
        let mut parts: Vec<&str> = self.frames.iter().map(String::as_str).collect();
        parts.push(pending);
        elide_chain(&parts)
    }
}

/// 超长链条的中段省略（错误文本可读优先：保留首尾各 3 段）。
fn elide_chain(parts: &[&str]) -> String {
    const EDGE: usize = 3;
    if parts.len() <= 2 * EDGE + 1 {
        return parts.join(" -> ");
    }
    let head = parts[..EDGE].join(" -> ");
    let tail = parts[parts.len() - EDGE..].join(" -> ");
    format!("{head} -> … -> {tail}")
}

/// 模块表：规范名 → 源码字节（确定性 `BTreeMap`，遍历序 = 名字序）。
///
/// **只装已登记模块**：装载期不做路径拼接（见模块文档）。登记即校验
/// （[`ModuleSet::insert`]）：名字规范、单模块 ≤ [`MAX_CODE_BYTES`]、
/// 总模块数 ≤ [`MAX_MODULES`]、同名不得重复（重复登记 = 作者侧歧义，
/// 不静默取后者）。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ModuleSet {
    by_name: BTreeMap<String, Vec<u8>>,
}

impl ModuleSet {
    /// 空模块表（单文件 Mod 的基线：require 对任何名字都可读拒绝）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 由 `(名字, 源码)` 序列构造（登记序无关：`BTreeMap` 定序）。
    pub fn from_pairs<I>(pairs: I) -> Result<Self, ModuleError>
    where
        I: IntoIterator<Item = (String, Vec<u8>)>,
    {
        let mut set = Self::new();
        for (name, code) in pairs {
            set.insert(&name, code)?;
        }
        Ok(set)
    }

    /// 登记一个模块（名字规范化为 key；上限与重复在登记期判）。
    pub fn insert(&mut self, name: &str, code: Vec<u8>) -> Result<(), ModuleError> {
        let canonical = normalize(name)?;
        if code.len() > MAX_CODE_BYTES {
            return Err(ModuleError::CodeTooLarge {
                name: canonical,
                len: code.len(),
                limit: MAX_CODE_BYTES,
            });
        }
        if self.by_name.contains_key(&canonical) {
            return Err(ModuleError::Duplicate { name: canonical });
        }
        if self.by_name.len() >= MAX_MODULES {
            return Err(ModuleError::TooManyModules {
                count: self.by_name.len() + 1,
                limit: MAX_MODULES,
            });
        }
        self.by_name.insert(canonical, code);
        Ok(())
    }

    /// 按规范名查源码（装载期唯一查表面；不做名字规范化）。
    pub fn get(&self, canonical_name: &str) -> Option<&[u8]> {
        self.by_name.get(canonical_name).map(Vec::as_slice)
    }

    /// 已登记模块数。
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    /// 是否空表。
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    /// 已登记规范名（名字序；诊断面）。
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.by_name.keys().map(String::as_str)
    }
}
