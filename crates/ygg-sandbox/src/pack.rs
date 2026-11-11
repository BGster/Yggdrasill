//! r2 打包面：`pack.json` 的 `code` 段声明 + `manifest.json` 的 `code` 数组。
//!
//! **为什么需要这一层**：r1 给了装载器（`require` 只查已登记模块表），但
//! 「哪些文件是模块」此前无处声明。缺这一层的后果不是装载失败而是**静默跑
//! 旧代码**：模块文件改了、入口 `mod.lua` 与 `pack.json` 没改 → 宿主的热载
//! 签名（`path + len + mtime`，只覆盖已声明文件）不变 → 热载与续局都读到旧
//! 字节，**没有任何报错**。所以声明面、manifest 内容寻址与宿主监视清单必须
//! 是同一份事实的三个投影：本模块是前两个的单一收口，第三个（`watched_files`）
//! 从同一份 `pack.json` 声明取路径。
//!
//! # 三件东西
//!
//! - [`CodeSpec`]：`pack.json` 的可选 `code` 段——`{entry, modules[]}`。
//!   缺失即旧单文件路径（现有 Mod 零迁移，行为逐字节不变）。
//! - [`entries_for`]：由模块字节现算 [`ManifestCodeEntry`]（sha256 + byteLength）。
//! - [`assemble`]：把「声明 → manifest 登记 → 实际字节」三者对成一张
//!   [`ModuleSet`]。装载方只拿这张表（装载器不对文件系统做任何事）。
//!
//! # 校验口径（装载前失败，全部可读）
//!
//! 路径：包内相对、无通配符、无 `.` / `..` 段、`.lua` 结尾、且能被
//! [`crate::modules::name_of_path`] 规范化（标识符字符集、文件名段禁点——
//! 后者防 `src/a.b.lua` 与 `src/a/b.lua` 撞同一个点分名）。**判定复用 r1 的
//! 解析面**，打包期与装载期不存在第二套规则。
//!
//! 集合：声明与 manifest 必须**双向相等**（声明了没登记 = 忘了跑打包工具；
//! 登记了没声明 = 改了声明没重跑）。部分登记一律拒绝——「半份模块表」会让
//! 装载期报一个看似无关的 `NotFound`。
//!
//! 内容：逐模块 [`ygg_schema::manifest::verify_code_entry`]（byteLength 预检
//! 先于 sha256，与资源包同一套判定）。
//!
//! 上限：模块数 ≤ [`crate::modules::MAX_MODULES`]（128），代码总量 ≤
//! [`MAX_TOTAL_CODE_BYTES`]（复用 r1 的 1 MiB 常量）。**在装载前**判，
//! 不做「先装进来再量体积」。
//!
//! # 签名口径（热载与续局）
//!
//! 宿主的热载签名是 `path + byteLength + mtime`（[`crate::pack`] 不参与、不
//! 改它）：它只回答「变没变」，不回答「是不是同一份内容」——所以 resume
//! 不必为每次续局读全部模块源码重算哈希。**精确性由本模块的 manifest
//! sha256 承担**：一旦签名说「变了」而内容其实没变，代价是一次无意义的
//! 重装载；一旦内容真变了而签名说「没变」才会出事，而这正是把每个模块路径
//! 都纳进宿主监视清单（见 `examples/egg.rs::watched_files`）要解决的那件事。

use std::io;

use serde_json::Value as Json;
use thiserror::Error as ThisError;

use ygg_schema::manifest::{verify_code_entry, ManifestCodeEntry};
use ygg_schema::validate::sha256_hex;

use crate::modules::{self, ModuleError, ModuleSet, MAX_CODE_BYTES, MAX_MODULES};

/// 代码总量上限：整包可 `require` 的模块源码字节总和。
///
/// 复用 r1 的 [`MAX_CODE_BYTES`]（单模块上限）作为整包上限——同一份 1 MiB
/// 预算换个量纲看，不引入第二个魔法数。
pub const MAX_TOTAL_CODE_BYTES: usize = MAX_CODE_BYTES;

/// `pack.json` 的 `code` 段（可选）：入口文件名 + 可 `require` 的模块清单。
///
/// `entry` 不是模块：入口由宿主作为 Mod 源读取（入口顶层即可 `require`），
/// 不登记进 [`ModuleSet`]——同一份源码有两个装载身份只会让缓存与错误定位
/// 产生歧义。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeSpec {
    entry: String,
    modules: Vec<String>,
}

impl CodeSpec {
    /// 解析 `pack.json` 的 `code` 段（`None` = 单文件 Mod 的旧路径）。
    ///
    /// 段存在即**全量校验**：不允许「写了个 code 但字段缺一半」这种半声明。
    pub fn parse(pack: &Json) -> Result<Option<Self>, PackError> {
        let Some(raw) = pack.get("code") else {
            return Ok(None);
        };
        if !raw.is_object() {
            return Err(PackError::CodeNotObject);
        }
        let entry = match raw.get("entry") {
            Some(Json::String(entry)) => entry.clone(),
            Some(_) => return Err(PackError::EntryNotString),
            None => return Err(PackError::MissingEntry),
        };
        validate_path(&entry, "entry")?;
        let raw_modules = match raw.get("modules") {
            Some(Json::Array(items)) => items,
            Some(_) => return Err(PackError::ModulesNotArray),
            None => return Err(PackError::MissingModules),
        };
        if raw_modules.len() > MAX_MODULES {
            return Err(PackError::TooManyModules {
                count: raw_modules.len(),
                limit: MAX_MODULES,
            });
        }
        let mut seen: Vec<String> = Vec::with_capacity(raw_modules.len());
        for (index, item) in raw_modules.iter().enumerate() {
            let path = match item {
                Json::String(path) => path,
                _ => return Err(PackError::ModuleNotString { index }),
            };
            validate_path(path, "modules")?;
            if path == &entry {
                return Err(PackError::EntryIsModule { path: path.clone() });
            }
            if seen.contains(path) {
                return Err(PackError::DuplicateModule { path: path.clone() });
            }
            seen.push(path.clone());
        }
        Ok(Some(Self {
            entry,
            modules: seen,
        }))
    }

    /// 入口文件名（包内相对路径，如 `mod.lua`）。
    pub fn entry(&self) -> &str {
        &self.entry
    }

    /// 模块路径清单（声明序；`manifest.json` 的 `code` 数组按路径序写，
    /// 装载查表不依赖顺序）。
    pub fn modules(&self) -> &[String] {
        &self.modules
    }
}

/// 打包期读模块字节（IO 由调用方注入：本 crate 不做文件系统）。
///
/// 返回顺序 = 声明序；缺文件 / 不可读 → [`PackError::ReadModule`]（文案带
/// 路径，打包工具直接可读呈现）。
pub fn collect_bytes<F>(spec: &CodeSpec, mut read: F) -> Result<Vec<(String, Vec<u8>)>, PackError>
where
    F: FnMut(&str) -> io::Result<Vec<u8>>,
{
    let mut collected = Vec::with_capacity(spec.modules.len());
    for path in &spec.modules {
        let bytes = read(path).map_err(|source| PackError::ReadModule {
            path: path.clone(),
            source,
        })?;
        collected.push((path.clone(), bytes));
    }
    Ok(collected)
}

/// 由模块字节现算 manifest `code` 条目（路径序——manifest 确定性，与声明序无关）。
pub fn entries_for(module_bytes: &[(String, Vec<u8>)]) -> Vec<ManifestCodeEntry> {
    let mut entries: Vec<ManifestCodeEntry> = module_bytes
        .iter()
        .map(|(path, bytes)| {
            ManifestCodeEntry::new(path.clone(), sha256_hex(bytes), bytes.len() as u64)
        })
        .collect();
    entries.sort_by(|a, b| a.path().cmp(b.path()));
    entries
}

/// 声明 + manifest 登记 + 实际字节 → 装载用的 [`ModuleSet`]（唯一收口）。
///
/// 顺序即语义（全部在装载前）：
/// ① 声明规模 → ② 集合双向相等 → ③ 字节集合对账（未声明的一律不读）→
/// ④ 总量预算（只量长度）→ ⑤ 内容寻址（byteLength → sha256）→ ⑥ 登记规范名。
///
/// 返回的 [`ModuleSet`] 是装载器唯一可见的代码面：未登记的名字 `require`
/// 装载前就失败（`ModuleError::NotFound`），不存在路径拼接面。
pub fn assemble(
    spec: &CodeSpec,
    declared: &[ManifestCodeEntry],
    module_bytes: &[(String, Vec<u8>)],
) -> Result<ModuleSet, PackError> {
    // ① 声明规模（打包期与装载期同判：超限的包根本进不到装载）。
    if spec.modules.len() > MAX_MODULES {
        return Err(PackError::TooManyModules {
            count: spec.modules.len(),
            limit: MAX_MODULES,
        });
    }
    // ② 集合双向相等（部分登记拒绝：半份模块表只会换来一个无关的 NotFound）。
    if declared.is_empty() && !spec.modules.is_empty() {
        return Err(PackError::ManifestMissingCode {
            modules: spec.modules.len(),
        });
    }
    let mut registry: Vec<&ManifestCodeEntry> = Vec::with_capacity(declared.len());
    for entry in declared {
        if registry.iter().any(|known| known.path() == entry.path()) {
            return Err(PackError::ManifestDuplicateModule {
                path: entry.path().to_owned(),
            });
        }
        if !spec.modules.iter().any(|path| path == entry.path()) {
            return Err(PackError::ManifestUndeclaredModule {
                path: entry.path().to_owned(),
            });
        }
        registry.push(entry);
    }
    for path in &spec.modules {
        if !registry.iter().any(|entry| entry.path() == path) {
            return Err(PackError::ManifestMissingModule { path: path.clone() });
        }
    }
    // ③ 取字节：只认声明清单（多出来的字节不参与装载——不解静默采信）。
    for (path, _) in module_bytes {
        if !spec
            .modules
            .iter()
            .any(|declared_path| declared_path == path)
        {
            return Err(PackError::UndeclaredBytes { path: path.clone() });
        }
    }
    for path in &spec.modules {
        if !module_bytes.iter().any(|(known, _)| known == path) {
            return Err(PackError::MissingBytes { path: path.clone() });
        }
    }
    // ④ 总量预算：只量字节长度，不先算哈希（最廉价失败优先——错哈希的包也
    //    先报「太大」，不把一次 1 MiB 的摘要计算浪费在注定被拒的包上）。
    let total = spec
        .modules
        .iter()
        .filter_map(|path| {
            module_bytes
                .iter()
                .find(|(known, _)| known == path)
                .map(|(_, bytes)| bytes.len())
        })
        .fold(0usize, usize::saturating_add);
    if total > MAX_TOTAL_CODE_BYTES {
        return Err(PackError::TotalCodeTooLarge {
            total,
            limit: MAX_TOTAL_CODE_BYTES,
        });
    }
    // ⑤⑥ 逐模块：内容寻址（byteLength → sha256）→ 规范名登记。
    let mut set = ModuleSet::new();
    for path in &spec.modules {
        let entry = registry
            .iter()
            .find(|entry| entry.path() == path)
            .ok_or_else(|| PackError::ManifestMissingModule { path: path.clone() })?;
        let Some((_, bytes)) = module_bytes.iter().find(|(known, _)| known == path) else {
            return Err(PackError::MissingBytes { path: path.clone() });
        };
        verify_code_entry(bytes, entry).map_err(|err| PackError::Content {
            path: path.clone(),
            details: err.to_string(),
        })?;
        let name = modules::name_of_path(path).map_err(|cause| PackError::BadPath {
            field: "modules",
            path: path.clone(),
            cause,
        })?;
        set.insert(&name, bytes.clone())?;
    }
    Ok(set)
}

/// 模块路径校验（打包期与装载期同一判定：复用 [`crate::modules::name_of_path`]）。
///
/// 通配符与 `.`/`..` 段显式判在 `name_of_path` 之前——它把这两类归入
/// 「非法字符 / 文件名段含点」，文案对作者不够直白；这里先给专用错误，
/// 其余（绝对路径 / 反斜杠 / 非 `.lua` / 标识符字符集）一律委托解析面，
/// 不另写一套规则。
fn validate_path(path: &str, field: &'static str) -> Result<(), PackError> {
    if path
        .chars()
        .any(|ch| matches!(ch, '*' | '?' | '[' | ']' | '{' | '}'))
    {
        return Err(PackError::Glob {
            field,
            path: path.to_owned(),
        });
    }
    if path
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        return Err(PackError::Traversal {
            field,
            path: path.to_owned(),
        });
    }
    modules::name_of_path(path).map_err(|cause| PackError::BadPath {
        field,
        path: path.to_owned(),
        cause,
    })?;
    Ok(())
}

/// 打包面拒绝（构造期/打包期；文案自带路径，作者不用猜是哪一条）。
#[derive(Debug, ThisError)]
pub enum PackError {
    /// `code` 段不是对象。
    #[error(
        "pack.json `code` must be an object like \
         {{\"entry\": \"mod.lua\", \"modules\": [\"src/util.lua\"]}}"
    )]
    CodeNotObject,
    /// `code.entry` 缺失。
    #[error(
        "pack.json `code.entry` is required (the entry file name inside the package, \
         e.g. \"mod.lua\")"
    )]
    MissingEntry,
    /// `code.entry` 非字符串。
    #[error("pack.json `code.entry` must be a string")]
    EntryNotString,
    /// `code.modules` 缺失（显式空数组才表示「只有入口」）。
    #[error(
        "pack.json `code.modules` is required when `code` is present (declare every \
         require-able module; use an empty array for none)"
    )]
    MissingModules,
    /// `code.modules` 非数组。
    #[error("pack.json `code.modules` must be an array of strings")]
    ModulesNotArray,
    /// `code.modules[i]` 非字符串。
    #[error("pack.json `code.modules[{index}]` must be a string")]
    ModuleNotString { index: usize },
    /// 路径含通配符（模块必须逐条枚举，不做 glob 展开）。
    #[error(
        "pack.json `code.{field}` path `{path}` must not contain glob wildcards \
         (`*` `?` `[` `]` `{{` `}}`): modules are enumerated one by one"
    )]
    Glob { field: &'static str, path: String },
    /// 路径含 `.` / `..` 段（包内相对路径，无上跳面）。
    #[error(
        "pack.json `code.{field}` path `{path}` must not contain `.` or `..` segments \
         (package-relative paths only)"
    )]
    Traversal { field: &'static str, path: String },
    /// 路径不能规范化为模块名（绝对路径 / 反斜杠 / 非 `.lua` / 非法字符 / 段含点）。
    #[error("pack.json `code.{field}` path `{path}` is not a valid module path: {cause}")]
    BadPath {
        field: &'static str,
        path: String,
        cause: ModuleError,
    },
    /// 同一路径在 `code.modules` 里出现两次。
    #[error("pack.json `code.modules` declares `{path}` more than once")]
    DuplicateModule { path: String },
    /// 入口同时被登记为模块。
    #[error(
        "pack.json `code.entry` (`{path}`) must not also be listed in `code.modules` \
         (the entry is loaded as the mod source, not as a module)"
    )]
    EntryIsModule { path: String },
    /// 声明模块数超上限。
    #[error("pack.json declares {count} code modules (limit {limit})")]
    TooManyModules { count: usize, limit: usize },
    /// 代码总量超上限（装载前即可读失败）。
    #[error("module code totals {total} bytes (limit {limit})")]
    TotalCodeTooLarge { total: usize, limit: usize },
    /// 有声明但 manifest 没有 `code` 数组（忘了跑打包工具）。
    #[error(
        "manifest.json has no `code` array while pack.json declares {modules} module(s): \
         run the pack tool to regenerate the manifest"
    )]
    ManifestMissingCode { modules: usize },
    /// 声明了但 manifest 未登记。
    #[error("module `{path}` is declared in pack.json but not registered in manifest.json")]
    ManifestMissingModule { path: String },
    /// manifest 登记了但声明里没有。
    #[error(
        "module `{path}` is registered in manifest.json but not declared in pack.json \
         (run the pack tool after editing the declaration)"
    )]
    ManifestUndeclaredModule { path: String },
    /// manifest 同一路径登记两次。
    #[error("manifest.json registers module `{path}` more than once")]
    ManifestDuplicateModule { path: String },
    /// 声明了但调用方没给字节。
    #[error("module `{path}` was declared but its bytes were not provided")]
    MissingBytes { path: String },
    /// 给了未声明的字节。
    #[error("module `{path}` bytes were provided but the path is not declared in pack.json")]
    UndeclaredBytes { path: String },
    /// 读盘失败（打包期；IO 由调用方注入）。
    #[error("cannot read module `{path}`: {source}")]
    ReadModule {
        path: String,
        #[source]
        source: io::Error,
    },
    /// 内容寻址失败（byteLength / sha256；文案带路径与期望值）。
    #[error("module `{path}` content rejected: {details}")]
    Content { path: String, details: String },
    /// 模块表登记失败（单模块体积 / 重复 / 数量——兜底面）。
    #[error(transparent)]
    Module(#[from] ModuleError),
}
