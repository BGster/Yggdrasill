use std::path::{Path, PathBuf};

use serde_json::Value as Json;

use super::{MountError, MOUNT_DIR_MAX_BYTES, MOUNT_NAME_MAX_BYTES};

/// 保留名：它是 run 目录里另一个产物目录，同名挂载会让 `mounts/artifacts` 与
/// `artifacts/` 在读者心里混起来（路径面本身不冲突，但错误面会）。
const RESERVED_NAMES: [&str; 1] = ["artifacts"];

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

/// 解析并校验 `pack.json` 的 `mounts` 段（`openwiki/contracts/mod/mod-package.md` §3.2.2）。
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
    if let Some(ch) = chars
        .find(|ch| !(ch.is_ascii_lowercase() || ch.is_ascii_digit() || *ch == '_' || *ch == '-'))
    {
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

// ============================================================================
// §挂载点落地（三态幂等）与路径狱
// ============================================================================

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

// ============================================================================
// §crate 内单测：声明面逐条钉规则（`openwiki/contracts/mod/mod-package.md` §3.2.2）
// ============================================================================
