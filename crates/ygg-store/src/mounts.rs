//! 文件挂载桥的宿主侧端口（`pack.json` 的 `mounts` 段 → `<run-dir>/mounts/`）。
//!
//! # 为什么落在 `ygg-store`
//!
//! `ygg-sandbox` 的 `driver` / `sandbox` / `modules` 是零 IO 面（`modload.rs` 头注
//! 的纪律），而 run 目录的产物本来就全归本 crate——`artifacts/` 是既有的同类
//! 先例（`json.rs` 的 `ARTIFACTS_DIR`）。`<run-dir>/mounts/` 与
//! `<run-dir>/fs.jsonl` 和 `<run-dir>/artifacts/` 同族，于是落在这里；沙箱侧只
//! 保留一个注入 trait（`ygg_sandbox::host_api::MountFs`，与 `HostToolRunner` 同
//! 型），经 `[dev-dependencies]` 在测试里接到本模块上——依赖方向不变（store 仍
//! 不进 wasm32 / lint 的依赖图，见 `docs/context/cold-start/topology.md` 的不变量）。
//!
//! # 四件事的归属
//!
//! - **声明解析**：`pack.json` 的 `mounts` 段在 attach 期校验，**不**进
//!   `ModBundle::load`——声明坏了不能炸 mod 装载（与 `panel` 段同族）。
//! - **落点与三态幂等**：`<run-dir>/mounts/<name>` 恒存在；已存在且与声明一致
//!   时零写入通过，不一致时 `YGG-FS-MOUNT` 硬拒且零写入。
//! - **路径狱**：沙箱可见的挂载路径只有 `mounts/<name>` 与 `mounts/<name>/<rel>`。
//! - **审计行**：每次写（成功与被拒）追加一行 `<run-dir>/fs.jsonl`。
//!
//! 规范正文在 `docs/design/kernel/lua-api.md` §3.1.2 与
//! `docs/design/mod/mod-package.md` §3.2.2；本文件是实现，不是第二个真源。

use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::Value as Json;

/// 挂载点根目录名（`<run-dir>/mounts`）。
pub const MOUNTS_DIR: &str = "mounts";

/// `ygg.fs.write` 审计行的文件名（`<run-dir>/fs.jsonl`）。
pub const FS_AUDIT_FILE: &str = "fs.jsonl";

/// 沙箱侧挂载面的路径前缀（含结尾 `/`；`mounts/<name>` 单独判等于）。
pub const MOUNT_PATH_PREFIX: &str = "mounts/";

/// 单件 `read` / `write` 的字节上限（4 MiB，读写对称）。
pub const MOUNT_FILE_MAX_BYTES: usize = 4 * 1024 * 1024;

/// 每次 attach 的累计写预算（64 MiB = 16 次满帽写）。
///
/// **进程级、不落盘**：跑飞闸（`for i=1,1e6 do fs.write(..) end`）既不受燃料约束
/// （燃料计指令不计字节）也不受内存帽约束（写完即可丢弃引用），必须有字节预算；
/// 但它不需要跨重启账本（那会把每次写变成读改写账本文件），续局拿到新预算。
pub const MOUNT_RUN_WRITE_BUDGET: u64 = 64 * 1024 * 1024;

/// 沙箱挂载路径的字节上限（含 `mounts/` 前缀）。
pub const MOUNT_PATH_MAX_BYTES: usize = 512;

/// `<rel>` 的段数上限（词法上界，不是安全边界）。
pub const MOUNT_PATH_MAX_SEGMENTS: usize = 24;

/// 单次 `list` 的条目上限（无分页：超大目录不可 list，逐路径 `read` / `stat` 不受影响）。
pub const MOUNT_LIST_MAX_ENTRIES: usize = 16_384;

/// 挂载 `name` 的字节上限（`^[a-z][a-z0-9_-]{0,31}$` 的上界）。
pub const MOUNT_NAME_MAX_BYTES: usize = 32;

/// `dir` 声明的字节上限。
pub const MOUNT_DIR_MAX_BYTES: usize = 4096;

/// 保留名：它是 run 目录里另一个产物目录，同名挂载会让 `mounts/artifacts` 与
/// `artifacts/` 在读者心里混起来（路径面本身不冲突，但错误面会）。
const RESERVED_NAMES: [&str; 1] = ["artifacts"];

/// 错误码（`YGG-<SUBSYSTEM>-<COND>`，与 `YGG-MOD-EFFECT` / `YGG-PACK` 同族）。
///
/// 码恒是 message 的**首段**（`Display` 拼 `"{code}: {detail}"`）：信封码仍是
/// `code::VALIDATION`，新增信封码要动映射面与下游按码分支的消费者，收益只是
/// 省一次前缀解析。
pub mod codes {
    /// 路径狱拒绝（首段非两根本 / 段非法 / 越界 / 父缺失 / 末段软链）。
    pub const JAIL: &str = "YGG-FS-JAIL";
    /// 尺寸帽拒绝（单件 / 写预算 / list 条目数）。
    pub const CAP: &str = "YGG-FS-CAP";
    /// 声明非法（attach 期）、挂载点状态冲突、写只读挂载、挂载点不可用（IO 失败）。
    pub const MOUNT: &str = "YGG-FS-MOUNT";
    /// 宿主未挂载文件桥（沙箱侧产生；本模块只导出常量供对齐）。
    pub const UNMOUNTED: &str = "YGG-FS-UNMOUNTED";
    /// 已写未审（写成功但审计行 append 失败）。
    pub const AUDIT: &str = "YGG-FS-AUDIT";
}

/// 挂载面错误：稳定码 + 人读原因。
///
/// `Display` 是 `"{code}: {detail}"`——**码在前**，因为调用方（Lua 侧、CLI 回执）
/// 按首段分派，人读原因在后。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountError {
    code: &'static str,
    detail: String,
}

impl MountError {
    /// 路径狱拒绝。
    pub fn jail(detail: impl Into<String>) -> Self {
        Self {
            code: codes::JAIL,
            detail: detail.into(),
        }
    }

    /// 尺寸帽拒绝。
    pub fn cap(detail: impl Into<String>) -> Self {
        Self {
            code: codes::CAP,
            detail: detail.into(),
        }
    }

    /// 声明非法 / 挂载点冲突 / 只读挂载 / IO 失败。
    pub fn mount(detail: impl Into<String>) -> Self {
        Self {
            code: codes::MOUNT,
            detail: detail.into(),
        }
    }

    /// 已写未审。
    pub fn audit(detail: impl Into<String>) -> Self {
        Self {
            code: codes::AUDIT,
            detail: detail.into(),
        }
    }

    /// 稳定码（下游按码分支；`detail` 只作人读定位）。
    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for MountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for MountError {}

/// 挂载点的读写模式。`ro` 是**缺省**（唯一减压阀：不声明 `mode` 就不会有写）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountMode {
    /// 只读：任何写 → `YGG-FS-MOUNT`。
    Ro,
    /// 读写。
    Rw,
}

impl MountMode {
    /// 声明面 / 回执面的取值。
    pub fn as_str(self) -> &'static str {
        match self {
            MountMode::Ro => "ro",
            MountMode::Rw => "rw",
        }
    }

    /// 解析 `mode` 字段；未知串 → `None`（调用方报 `YGG-FS-MOUNT`）。
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "ro" => Some(MountMode::Ro),
            "rw" => Some(MountMode::Rw),
            _ => None,
        }
    }
}

/// 一条挂载声明（已解析、已 canonicalize 的宿主侧事实）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountSpec {
    /// 沙箱可见的挂载点名（`mounts/<name>`）。
    pub name: String,
    /// 宿主目录的 canonical 路径；`None` = **私有挂载**（内容是本局产物）。
    pub dir: Option<PathBuf>,
    /// 读写模式（缺省 [`MountMode::Ro`]）。
    pub mode: MountMode,
}

impl MountSpec {
    /// 私有挂载（无 `dir`）的构造面。
    pub fn private(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            dir: None,
            mode: MountMode::Ro,
        }
    }
}

/// 解析并校验 `pack.json` 的 `mounts` 段（`docs/design/mod/mod-package.md` §3.2.2）。
///
/// 规则（顺序即报错顺序；**任一条违规 → `YGG-FS-MOUNT`，零写入**）：
/// ① 段缺失或 `[]` → 零挂载；段存在但不是数组 / 元素不是对象 → 拒；
/// ② `name` 匹配 `^[a-z][a-z0-9_-]{0,31}$`、同一 pack 内唯一、不得为保留名；
/// ③ `dir`（可选）1..=4096 字节、无 NUL，相对路径对 `mod_dir` 解析，
///    `canonicalize` 后必须是**已存在目录**；
/// ④ `mode`（可选）取 `ro` / `rw`，缺省 `ro`；
/// ⑤ 未知键忽略（键集宽松、已知键取值严格）。
pub fn parse_mounts(pack: &Json, mod_dir: &Path) -> Result<Vec<MountSpec>, MountError> {
    let Some(segment) = pack.get("mounts") else {
        return Ok(Vec::new()); // 旧 mod 100% 兼容
    };
    let Some(items) = segment.as_array() else {
        return Err(MountError::mount(format!(
            "pack.json 的 mounts 段必须是数组，当前是 {}",
            type_name(segment)
        )));
    };
    let mut out: Vec<MountSpec> = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let Some(object) = item.as_object() else {
            return Err(MountError::mount(format!(
                "mounts[{index}] 必须是对象，当前是 {}",
                type_name(item)
            )));
        };
        let Some(raw_name) = object.get("name").and_then(Json::as_str) else {
            return Err(MountError::mount(format!(
                "mounts[{index}] 缺省必填字段 name（字符串）"
            )));
        };
        if let Some(reason) = mount_name_violation(raw_name) {
            return Err(MountError::mount(format!(
                "mounts[{index}] 的 name `{raw_name}` 不合法：{reason}"
            )));
        }
        if out.iter().any(|spec| spec.name == raw_name) {
            return Err(MountError::mount(format!(
                "mounts[{index}] 的 name `{raw_name}` 在同一 pack 内重复"
            )));
        }
        let mode = match object.get("mode") {
            None => MountMode::Ro,
            Some(value) => {
                let Some(text) = value.as_str() else {
                    return Err(MountError::mount(format!(
                        "mounts[{index}] 的 mode 必须是字符串，当前是 {}",
                        type_name(value)
                    )));
                };
                MountMode::parse(text).ok_or_else(|| {
                    MountError::mount(format!(
                        "mounts[{index}] 的 mode 只能是 `ro` / `rw`，当前是 `{text}`"
                    ))
                })?
            }
        };
        let dir = match object.get("dir") {
            None => None,
            Some(value) => {
                let Some(text) = value.as_str() else {
                    return Err(MountError::mount(format!(
                        "mounts[{index}] 的 dir 必须是字符串，当前是 {}",
                        type_name(value)
                    )));
                };
                Some(resolve_mount_dir(raw_name, text, mod_dir)?)
            }
        };
        out.push(MountSpec {
            name: raw_name.to_owned(),
            dir,
            mode,
        });
    }
    Ok(out)
}

/// `name` 的合法性（`Ok(())` 之外的每个分支都带人读原因）。
fn mount_name_violation(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        return Some("不得为空");
    }
    if name.len() > MOUNT_NAME_MAX_BYTES {
        return Some("超过 32 字节");
    }
    let mut chars = name.chars();
    let head = chars.next()?;
    if !head.is_ascii_lowercase() {
        return Some("首字符必须是 ASCII 小写字母");
    }
    if let Some(ch) = chars.find(|ch| !(ch.is_ascii_lowercase() || ch.is_ascii_digit() || *ch == '_' || *ch == '-')) {
        return Some(if ch.is_ascii_uppercase() {
            "只允许小写字母 / 数字 / `_` / `-`（不得含大写）"
        } else {
            "只允许小写字母 / 数字 / `_` / `-`"
        });
    }
    if RESERVED_NAMES.contains(&name) {
        return Some("是保留名（`artifacts`）");
    }
    None
}

/// `dir` 的解析与 canonicalize（`mod_dir` 是相对路径的基准）。
fn resolve_mount_dir(name: &str, text: &str, mod_dir: &Path) -> Result<PathBuf, MountError> {
    if text.is_empty() {
        return Err(MountError::mount(format!(
            "mounts `{name}` 的 dir 不得为空串（要私有挂载就省略该字段）"
        )));
    }
    if text.len() > MOUNT_DIR_MAX_BYTES {
        return Err(MountError::mount(format!(
            "mounts `{name}` 的 dir 超过 {MOUNT_DIR_MAX_BYTES} 字节"
        )));
    }
    if text.contains('\0') {
        return Err(MountError::mount(format!(
            "mounts `{name}` 的 dir 不得含 NUL"
        )));
    }
    let declared = Path::new(text);
    let joined = if declared.is_absolute() {
        declared.to_path_buf()
    } else {
        mod_dir.join(declared)
    };
    let canon = joined.canonicalize().map_err(|err| {
        MountError::mount(format!(
            "mounts `{name}` 的 dir `{}` 无法 canonicalize（{err}）",
            joined.display()
        ))
    })?;
    let meta = std::fs::metadata(&canon).map_err(|err| {
        MountError::mount(format!(
            "mounts `{name}` 的 dir `{}` 不可读（{err}）",
            canon.display()
        ))
    })?;
    if !meta.is_dir() {
        return Err(MountError::mount(format!(
            "mounts `{name}` 的 dir `{}` 不是目录",
            canon.display()
        )));
    }
    Ok(canon)
}

/// JSON 值的类型名（错误文案用；`serde_json` 的类型名对作者不够直白）。
fn type_name(value: &Json) -> &'static str {
    match value {
        Json::Null => "null",
        Json::Bool(_) => "boolean",
        Json::Number(_) => "number",
        Json::String(_) => "string",
        Json::Array(_) => "array",
        Json::Object(_) => "object",
    }
}
