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
//! 不进 wasm32 / lint 的依赖图，见 `openwiki/operating/cold-start/topology.md` 的不变量）。
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
//! 规范正文在 `openwiki/contracts/kernel/lua-api.md` §3.1.2 与
//! `openwiki/contracts/mod/mod-package.md` §3.2.2；本文件是实现，不是第二个真源。

use std::fmt;

mod declare;
mod store;

pub use declare::{parse_mounts, MountMode, MountSpec};
pub use store::MountStore;

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

/// 挂载点里一项的类型（= 沙箱面 `kind` 的取值源）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountKind {
    File,
    Dir,
    /// 不存在（只出现在 `stat` 的回执里，`as_str()` 是空串）。
    Missing,
}

impl MountKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MountKind::File => "file",
            MountKind::Dir => "dir",
            MountKind::Missing => "",
        }
    }
}

/// `stat` 的结果（对应沙箱面 `{ exists, bytes, sha256, kind }`）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountStat {
    pub exists: bool,
    pub bytes: usize,
    pub sha256: String,
    pub kind: MountKind,
}

/// `list` 的一行（对应沙箱面 `{ name, path, bytes, sha256 = "", kind }`）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountEntry {
    pub name: String,
    /// 沙箱可见路径（`mounts/<name>[/<rel>]/<entry>`，由**请求路径**拼，不用 canonical 路径）。
    pub path: String,
    pub bytes: usize,
    pub kind: MountKind,
}

/// `write` 的结果（对应沙箱面 `{ path, bytes, sha256 }`）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountWritten {
    pub path: String,
    pub bytes: usize,
    pub sha256: String,
}
