//! ygg-modules：Mod 代码模块的**纯解析面**（零依赖——无 Lua、无 IO、无 serde）。
//!
//! 这里是「什么是合法的模块名 / 模块路径 / 模块集合 / 装载栈」的**唯一实现**。
//! 三个消费方共用它，因此不会出现「装载能过但静态检查判非法」这类双份口径：
//!
//! - `ygg_sandbox::modules`：装载器（`require` 闭包 + 缓存 + `seal`）的 Lua 面
//!   留在该 crate 的 `binding` 子模块里，判定面全部来自本 crate；
//! - `ygg_sandbox::pack`：打包期把 `pack.json` 的 `code` 声明对成 `ModuleSet`；
//! - `ygg-lint --pack`：静态检查逐模块 parse + 依赖图。
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
    /// `seal` 之后的 require（装载期专属面）。
    Sealed { name: String },
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
            ModuleError::Sealed { name } => write!(
                f,
                "sandbox denied: module '{name}' cannot be loaded here (require is a \
                 load-time-only surface: it closes once the mod has finished loading)"
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
