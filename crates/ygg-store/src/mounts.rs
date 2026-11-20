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

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
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

// ============================================================================
// §挂载点落地（三态幂等）与路径狱
// ============================================================================

/// 一个已冻结的挂载根（attach 时 `canonicalize` 一次，之后不再解析）。
#[derive(Clone, Debug, PartialEq, Eq)]
struct MountRoot {
    root: PathBuf,
    mode: MountMode,
}

/// 落地的待执行动作（先算全部现状再写，保证「不一致 → 零写入」/ 两条声明
/// 里后一条冲突时前一条也不会被顺手建出来）。
enum Landing<'a> {
    /// 无需动盘（已存在且与声明一致）。
    Keep,
    /// 建真目录（私有挂载）。
    MakeDir,
    /// 建指向声明目录的符号链接（bind 语义，不拷贝内容）。
    MakeLink(&'a Path),
}

/// 狱的「要什么」：文件面要求 `<rel>` 非空；目录面接受挂载根本身。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Need {
    File,
    Dir,
}

/// 一次成功的狱裁决。
struct Located {
    mount: String,
    target: PathBuf,
}

/// 文件挂载桥的宿主侧句柄（`pack.json` `mounts` 段的运行时形）。
pub struct MountStore {
    run_dir: PathBuf,
    mounts_root: PathBuf,
    audit: PathBuf,
    roots: BTreeMap<String, MountRoot>,
    /// 剩余写预算（字节；**进程级、不落盘**）。
    budget_left: u64,
}

impl MountStore {
    /// 把 `pack.json` 的 `mounts` 段落到 `<run-dir>/mounts/`，并把每个挂载根
    /// canonicalize 一次后冻结。
    ///
    /// 三态幂等（重载 / 续局走同一个入口）：
    /// - 不存在 → 创建（私有 = 真目录；`{name,dir}` = 指向 `canonicalize(dir)` 的软链）；
    /// - 已存在且与声明一致（私有 = 真目录 / 目录挂载 = 软链指向同一 canonical 目标）
    ///   → 静默通过，**零写入**；
    /// - 已存在但不一致 → `YGG-FS-MOUNT` 硬拒且**零写入**（先全部算现状、再落地）。
    ///
    /// 挂载点内的条目仍是懒创建（父目录不存在即拒，见 §狱的规则 7）。
    pub fn attach(run_dir: &Path, pack: &Json, mod_dir: &Path) -> Result<Self, MountError> {
        let specs = parse_mounts(pack, mod_dir)?; // 声明坏了：零写入
        let mounts_root = run_dir.join(MOUNTS_DIR);

        // ① 全量算现状（这一阶段不碰盘任何写）。
        let mut plan: Vec<(&MountSpec, Landing<'_>)> = Vec::with_capacity(specs.len());
        for spec in &specs {
            let slot = mounts_root.join(&spec.name);
            let landing = match fs::symlink_metadata(&slot) {
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => match &spec.dir {
                    None => Landing::MakeDir,
                    Some(dir) => Landing::MakeLink(dir),
                },
                Err(err) => {
                    return Err(MountError::mount(format!(
                        "挂载点 `{}` 不可探测（{err}）",
                        slot.display()
                    )))
                }
                Ok(meta) => {
                    if meta.file_type().is_symlink() {
                        match &spec.dir {
                            None => {
                                return Err(MountError::mount(format!(
                                    "挂载点 `{}` 已存在为软链，但声明是私有挂载（不一致）",
                                    slot.display()
                                )))
                            }
                            Some(dir) => {
                                let actual = fs::read_link(&slot)
                                    .map_err(|err| {
                                        MountError::mount(format!(
                                            "挂载点 `{}` 的链接目标不可读（{err}）",
                                            slot.display()
                                        ))
                                    })?
                                    .canonicalize()
                                    .map_err(|err| {
                                        MountError::mount(format!(
                                            "挂载点 `{}` 的链接目标不可 canonicalize（{err}）",
                                            slot.display()
                                        ))
                                    })?;
                                if actual != *dir {
                                    return Err(MountError::mount(format!(
                                        "挂载点 `{}` 指向 `{}`，与声明的 `{}` 不一致",
                                        slot.display(),
                                        actual.display(),
                                        dir.display()
                                    )));
                                }
                                Landing::Keep
                            }
                        }
                    } else if meta.is_dir() && spec.dir.is_none() {
                        Landing::Keep
                    } else {
                        return Err(MountError::mount(format!(
                            "挂载点 `{}` 已存在，但与声明不一致（{}）",
                            slot.display(),
                            if meta.is_dir() { "声明了 dir」却已存在为真目录" } else { "已存在且不是目录" }
                        )));
                    }
                }
            };
            plan.push((spec, landing));
        }

        // ② 落地（到这里所有声明都已验证过，不会半途才发现冲突）。
        fs::create_dir_all(&mounts_root).map_err(|err| {
            MountError::mount(format!("创建 {} 失败（{err}）", mounts_root.display()))
        })?;
        for (spec, landing) in &plan {
            let slot = mounts_root.join(&spec.name);
            match landing {
                Landing::Keep => {}
                Landing::MakeDir => fs::create_dir_all(&slot).map_err(|err| {
                    MountError::mount(format!("创建 {} 失败（{err}）", slot.display()))
                })?,
                Landing::MakeLink(dir) => link_dir(dir, &slot)?,
            }
        }

        // ③ 冻结每个挂载根（一次 canonicalize；之后的路径裁决只用这个值）。
        let mut roots = BTreeMap::new();
        for spec in &specs {
            let slot = mounts_root.join(&spec.name);
            let root = slot.canonicalize().map_err(|err| {
                MountError::mount(format!("挂载点 {} 不可 canonicalize（{err}）", slot.display()))
            })?;
            roots.insert(
                spec.name.clone(),
                MountRoot {
                    root,
                    mode: spec.mode,
                },
            );
        }

        Ok(Self {
            run_dir: run_dir.to_path_buf(),
            mounts_root,
            audit: run_dir.join(FS_AUDIT_FILE),
            roots,
            budget_left: MOUNT_RUN_WRITE_BUDGET,
        })
    }

    /// run 目录（调用方拼产物路径用）。
    pub fn run_dir(&self) -> &Path {
        &self.run_dir
    }

    /// 挂载点根（`<run-dir>/mounts`；运维观察面）。
    pub fn mounts_root(&self) -> &Path {
        &self.mounts_root
    }

    /// 审计行文件（`<run-dir>/fs.jsonl`）。
    pub fn audit_path(&self) -> &Path {
        &self.audit
    }

    /// 声明的挂载点（名字 → （冻结根，模式）；声明序无关，`BTreeMap` 序即名字序）。
    pub fn declared(&self) -> impl Iterator<Item = (&str, &Path, MountMode)> {
        self.roots
            .iter()
            .map(|(name, entry)| (name.as_str(), entry.root.as_path(), entry.mode))
    }

    /// 剩余写预算（字节）。
    pub fn budget_left(&self) -> u64 {
        self.budget_left
    }

    /// 路径狱（**唯一实现**）：`mounts/<name>` 与 `mounts/<name>/<rel>` 之外一律拒。
    ///
    /// 拒绝条件（顺序即报错顺序，逐条对应 `docs/design/kernel/lua-api.md` §3.1.2）：
    /// ① 不以 `mounts/` 起头；② `<name>` 未声明；③ 任一段为空 / `.` / `..` / 含
    /// NUL 或 ASCII 控制字符；④ 路径 > 512 字节或 `<rel>` 段数 > 24；⑤ canonicalize
    /// 后必须按**路径分量**落在冻结根内；⑥ 末段不得是软链；⑦ 父目录必须存在。
    /// `<rel>` 为空（`mounts/<name>`）只被 `Need::Dir` 接受。
    fn locate(&self, path: &str, need: Need) -> Result<Located, MountError> {
        if path.is_empty() {
            return Err(MountError::jail("路径不得为空"));
        }
        if path.len() > MOUNT_PATH_MAX_BYTES {
            return Err(MountError::jail(format!(
                "路径超过 {MOUNT_PATH_MAX_BYTES} 字节（当前 {} 字节）",
                path.len()
            )));
        }
        let Some(rest) = path.strip_prefix(MOUNT_PATH_PREFIX) else {
            return Err(MountError::jail(format!(
                "挂载路径必须以 `{MOUNT_PATH_PREFIX}` 起头（当前是 `{path}`）"
            )));
        };
        let mut segments = rest.split('/');
        let name = segments.next().unwrap_or_default();
        let Some(entry) = self.roots.get(name) else {
            return Err(MountError::jail(format!(
                "挂载点 `{name}` 未在 pack.json 的 mounts 段声明"
            )));
        };
        let rel: Vec<&str> = segments.collect();
        for segment in &rel {
            if segment.is_empty() {
                return Err(MountError::jail(format!(
                    "路径不得含空段（`//` 或结尾 `/`；当前是 `{path}`）"
                )));
            }
            if *segment == "." || *segment == ".." {
                return Err(MountError::jail(format!(
                    "路径段不得是 `.` 或 `..`（当前段 `{segment}`）"
                )));
            }
            if let Some(ch) = segment
                .chars()
                .find(|ch| *ch == '\u{0}' || (*ch as u32) < 0x20 || *ch == '\u{7f}')
            {
                return Err(MountError::jail(format!(
                    "路径段含 NUL 或 ASCII 控制字符（U+{:04X}）",
                    ch as u32
                )));
            }
        }
        if rel.len() > MOUNT_PATH_MAX_SEGMENTS {
            return Err(MountError::jail(format!(
                "路径段数超过 {MOUNT_PATH_MAX_SEGMENTS}（当前 {} 段）",
                rel.len()
            )));
        }
        if rel.is_empty() {
            if need == Need::File {
                return Err(MountError::jail(format!(
                    "`{path}` 是挂载根本身（目录），不能当文件路径用"
                )));
            }
            return Ok(Located {
                mount: name.to_owned(),
                target: entry.root.clone(),
            });
        }

        // 父目录：canonicalize 一次（同时杀掉 `..` 与中间段软链的两条歧义），
        // 再按路径分量确认它仍在冻结根内。
        let (last, parents) = rel.split_last().expect("rel 非空");
        let mut parent = entry.root.clone();
        for segment in parents {
            parent.push(segment);
        }
        let canonical_parent = parent.canonicalize().map_err(|err| {
            MountError::jail(format!("父目录 {} 不存在或不可达（{err}）", parent.display()))
        })?;
        let meta = fs::metadata(&canonical_parent).map_err(|err| {
            MountError::jail(format!(
                "父路径 {} 不可读（{err}）",
                canonical_parent.display()
            ))
        })?;
        if !meta.is_dir() {
            return Err(MountError::jail(format!(
                "父路径 {} 不是目录",
                canonical_parent.display()
            )));
        }
        if !canonical_parent.starts_with(&entry.root) {
            return Err(MountError::jail(format!(
                "路径越界：`{}` 不在挂载根 `{}` 内",
                canonical_parent.display(),
                entry.root.display()
            )));
        }
        let target = canonical_parent.join(last);
        // 末段永不跟随：已存在的软链一律拒（读穿与写穿两条逃逸同时关闭）。
        if let Ok(existing) = fs::symlink_metadata(&target) {
            if existing.file_type().is_symlink() {
                return Err(MountError::jail(format!(
                    "路径末段不得是符号链接（`{}`）",
                    target.display()
                )));
            }
        }
        Ok(Located {
            mount: name.to_owned(),
            target,
        })
    }
}

/// 建「目录挂载」的符号链接（bind 语义；不拷贝内容）。
#[cfg(unix)]
fn link_dir(dir: &Path, slot: &Path) -> Result<(), MountError> {
    std::os::unix::fs::symlink(dir, slot).map_err(|err| {
        MountError::mount(format!(
            "建挂载软链 {} -> {} 失败（{err}）",
            slot.display(),
            dir.display()
        ))
    })
}

/// 本平台没有符号链接 ⇒ 目录挂载不可用（诚实地拒，而不是默默拷贝一份内容）。
#[cfg(not(unix))]
fn link_dir(dir: &Path, slot: &Path) -> Result<(), MountError> {
    let _ = (dir, slot);
    Err(MountError::mount(
        "本平台不支持符号链接，目录挂载（{name, dir}）不可用；请改用私有挂载",
    ))
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

// ============================================================================
// §crate 内单测：声明面逐条钉规则（`docs/design/mod/mod-package.md` §3.2.2）
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    /// 本用例独占的临时目录（`std::env::temp_dir()` 合成数据，不碰任何业务目录）。
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ygg-mounts-unit-{tag}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    /// 只有 `mounts` 段的 pack（声明面的最小输入）。
    fn pack(mounts: Json) -> Json {
        json!({ "schemaVersion": 1, "mounts": mounts })
    }

    /// 解析失败时取码 + 原因（码必须在前，调用方按首段分派）。
    fn refused(pack: &Json, mod_dir: &Path) -> (String, String) {
        let err = parse_mounts(pack, mod_dir).expect_err("必须拒绍");
        assert_eq!(err.code(), codes::MOUNT, "声明面的拒绍一律 YGG-FS-MOUNT");
        let text = err.to_string();
        let prefix = format!("{}: ", codes::MOUNT);
        assert!(text.starts_with(&prefix), "码必须在前：{text}");
        (err.code().to_owned(), text[prefix.len()..].to_owned())
    }

    #[test]
    fn absent_or_empty_mounts_means_zero_mounts() {
        let dir = temp_dir("absent");
        // 段缺失：旧 mod 100% 兼容。
        let empty = parse_mounts(&json!({ "schemaVersion": 1 }), &dir).expect("段缺失合法");
        assert!(empty.is_empty());
        // 段为空数组：同一语义（显式声明「零挂载」）。
        let none = parse_mounts(&pack(json!([])), &dir).expect("空数组合法");
        assert!(none.is_empty());
    }

    #[test]
    fn non_array_segment_is_refused() {
        let dir = temp_dir("nonarray");
        for bad in [json!(null), json!("x"), json!(1), json!({ "name": "w" })] {
            let (_, detail) = refused(&pack(bad.clone()), &dir);
            assert!(detail.contains("必须是数组"), "{bad}: {detail}");
        }
    }

    #[test]
    fn non_object_element_is_refused() {
        let dir = temp_dir("nonobject");
        let (_, detail) = refused(&pack(json!(["work"])), &dir);
        assert!(detail.contains("mounts[0] 必须是对象"), "{detail}");
    }

    #[test]
    fn name_vectors() {
        let dir = temp_dir("names");
        for good in ["a", "work", "out-2", "a_b", "z9", "abcdefghijklmnopqrstuvwxyz012345"] {
            let specs = parse_mounts(&pack(json!([{ "name": good }])), &dir)
                .unwrap_or_else(|err| panic!("`{good}` 必须合法：{err}"));
            assert_eq!(specs[0].name, good);
        }
        for (bad, hint) in [
            ("", "不得为空"),
            ("Work", "首字符必须是 ASCII 小写字母"),
            ("9x", "首字符必须是 ASCII 小写字母"),
            ("a b", "只允许小写字母"),
            ("a/b", "只允许小写字母"),
            ("_x", "首字符必须是 ASCII 小写字母"),
            ("abcdefghijklmnopqrstuvwxyz0123456", "超过 32 字节"),
        ] {
            let (_, detail) = refused(&pack(json!([{ "name": bad }])), &dir);
            assert!(detail.contains(hint), "`{bad}` 的拒绍理由：{detail}");
        }
        // 缺 name（非字符串也算）：必填字段缺失。
        let (_, detail) = refused(&pack(json!([{ "dir": "/tmp" }])), &dir);
        assert!(detail.contains("缺省必填字段 name"), "{detail}");
    }

    #[test]
    fn duplicate_and_reserved_names_are_refused() {
        let dir = temp_dir("dup");
        let (_, detail) = refused(
            &pack(json!([{ "name": "work" }, { "name": "work" }])),
            &dir,
        );
        assert!(detail.contains("重复"), "{detail}");
        let (_, detail) = refused(&pack(json!([{ "name": "artifacts" }])), &dir);
        assert!(detail.contains("保留名"), "{detail}");
    }

    #[test]
    fn dir_must_be_an_existing_directory() {
        let dir = temp_dir("dir");
        // 不存在。
        let (_, detail) = refused(
            &pack(json!([{ "name": "w", "dir": "missing" }])),
            &dir,
        );
        assert!(detail.contains("无法 canonicalize"), "{detail}");
        // 存在但是文件。
        let file = dir.join("a.txt");
        fs::write(&file, b"x").expect("write file");
        let (_, detail) = refused(
            &pack(json!([{ "name": "w", "dir": file.display().to_string() }])),
            &dir,
        );
        assert!(detail.contains("不是目录"), "{detail}");
    }

    #[test]
    fn dir_resolves_relative_to_the_mod_dir() {
        let mod_dir = temp_dir("reldir");
        let host = mod_dir.join("biz");
        fs::create_dir_all(&host).expect("create host dir");
        let specs = parse_mounts(&pack(json!([{ "name": "w", "dir": "biz" }])), &mod_dir)
            .expect("相对路径合法");
        assert_eq!(
            specs[0].dir.as_deref(),
            Some(host.canonicalize().expect("canonical").as_path()),
            "相对路径对 <mod dir> 解析并 canonicalize"
        );
        // 绝对路径原样（仍然 canonicalize：符号链接 / `..` 都被折叠）。
        let specs = parse_mounts(
            &pack(json!([{ "name": "w", "dir": host.display().to_string() }])),
            &mod_dir,
        )
        .expect("绝对路径合法");
        assert_eq!(
            specs[0].dir.as_deref(),
            Some(host.canonicalize().expect("canonical").as_path())
        );
    }

    #[test]
    fn dir_shape_limits_are_enforced() {
        let dir = temp_dir("dirlimits");
        let (_, detail) = refused(&pack(json!([{ "name": "w", "dir": "" }])), &dir);
        assert!(detail.contains("不得为空串"), "{detail}");
        let long = "x".repeat(MOUNT_DIR_MAX_BYTES + 1);
        let (_, detail) = refused(&pack(json!([{ "name": "w", "dir": long }])), &dir);
        assert!(detail.contains("超过 4096 字节"), "{detail}");
        let (_, detail) = refused(&pack(json!([{ "name": "w", "dir": "a\u{0}b" }])), &dir);
        assert!(detail.contains("不得含 NUL"), "{detail}");
        // 非字符串的 dir：类型不对也是声明错误。
        let (_, detail) = refused(&pack(json!([{ "name": "w", "dir": 3 }])), &dir);
        assert!(detail.contains("dir 必须是字符串"), "{detail}");
    }

    #[test]
    fn mode_defaults_to_ro_and_rejects_unknown_values() {
        let dir = temp_dir("mode");
        let specs = parse_mounts(&pack(json!([{ "name": "w" }])), &dir).expect("缺省合法");
        assert_eq!(specs[0].mode, MountMode::Ro, "缺省是 ro（唯一减压阀）");
        let specs = parse_mounts(&pack(json!([{ "name": "w", "mode": "rw" }])), &dir)
            .expect("rw 合法");
        assert_eq!(specs[0].mode, MountMode::Rw);
        let (_, detail) = refused(&pack(json!([{ "name": "w", "mode": "readonly" }])), &dir);
        assert!(detail.contains("只能是 `ro` / `rw`"), "{detail}");
        let (_, detail) = refused(&pack(json!([{ "name": "w", "mode": 1 }])), &dir);
        assert!(detail.contains("mode 必须是字符串"), "{detail}");
    }

    #[test]
    fn unknown_keys_are_ignored() {
        // 键集宽松：旧宿主忽略未知顶层段，新宿主同样忽略未知元素键。
        let dir = temp_dir("unknown");
        let specs = parse_mounts(
            &json!({
                "schemaVersion": 1,
                "mounts": [{ "name": "w", "futureField": { "nested": true } }],
                "panel": { "futureKey": 1 },
            }),
            &dir,
        )
        .expect("未知键忽略");
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].dir, None, "无 dir = 私有挂载");
    }

    /// attach 的两个入参：一个空 run 目录 + 一个空 mod 目录。
    fn attach_env(tag: &str) -> (PathBuf, PathBuf) {
        let root = temp_dir(tag);
        let run = root.join("run");
        fs::create_dir_all(&run).expect("create run dir");
        let mod_dir = root.join("mod");
        fs::create_dir_all(&mod_dir).expect("create mod dir");
        (run, mod_dir)
    }

    /// `MountStore::attach` 的测试便捷面（pack 只含 `mounts` 段）。
    fn attach(run: &Path, mod_dir: &Path, mounts: Json) -> Result<MountStore, MountError> {
        MountStore::attach(run, &pack(mounts), mod_dir)
    }

    #[test]
    fn jail_rejects_everything_outside_the_two_roots() {
        let (run, mod_dir) = attach_env("jail-basic");
        let store = attach(&run, &mod_dir, json!([{ "name": "w" }])).expect("attach");
        for bad in [
            "artifacts/x.md",
            "mounts",
            "mounts/",
            "mounts/other/x",
            "mounts/w/",
            "mounts/w//x",
            "mounts/w/./x",
            "mounts/w/../x",
            "mounts/w/a\u{0}b",
            "/mounts/w/x",
        ] {
            match store.locate(bad, Need::File) {
                Ok(_) => panic!("`{bad}` 必须拒"),
                Err(err) => assert_eq!(err.code(), codes::JAIL, "`{bad}` → {err}"),
            }
        }
        // File 面拒挂载根本身；Dir 面接受它。
        assert!(store.locate("mounts/w", Need::File).is_err());
        let root = store.locate("mounts/w", Need::Dir).expect("挂载根是目录");
        assert_eq!(root.mount, "w");
        assert_eq!(
            root.target,
            run.join(MOUNTS_DIR)
                .join("w")
                .canonicalize()
                .expect("canonical")
        );
        // 父目录不存在 → 拒（r1 不做挂载点内的 mkdir）。
        assert!(store.locate("mounts/w/nope/x.txt", Need::File).is_err());
        // 超出段数 / 字节上限 → 拒。
        let deep = format!("mounts/w/{}", vec!["d"; MOUNT_PATH_MAX_SEGMENTS + 1].join("/"));
        assert!(store.locate(&deep, Need::File).is_err());
        let long = format!("mounts/w/{}", "x".repeat(MOUNT_PATH_MAX_BYTES));
        assert!(store.locate(&long, Need::File).is_err());
    }

    #[test]
    fn error_display_puts_the_code_first() {
        // 码在前是调用方契约（Lua 侧与 CLI 回执按首段分派）。
        assert_eq!(
            MountError::jail("x").to_string(),
            "YGG-FS-JAIL: x".to_owned()
        );
        assert_eq!(MountError::cap("x").code(), codes::CAP);
        assert_eq!(MountError::mount("x").code(), codes::MOUNT);
        assert_eq!(MountError::audit("x").code(), codes::AUDIT);
    }
}

