use mlua::Error as LuaError;
use thiserror::Error as ThisError;

use ygg_core::save::SaveError;
use ygg_core::snapshot::SnapshotError;

use crate::modules::ModuleError;

/// 信封错误码（稳定字符串契约，A1 线格式；下游按码分支不解析 message）。
pub mod code {
    /// 沙箱拒绝（模块装载拒绝、内存超限等沙箱结构面拒绝）。
    pub const SANDBOX_DENIED: &str = "SANDBOX_DENIED";
    /// 校验拒绝（payload 非标量、事件构造失败、未注册事件、handler 报错、
    /// 存档版本/内容不符）。
    pub const VALIDATION: &str = "VALIDATION";
    /// 资源包边界体积限（S11 激活：`KernelError::ResourceBudget` → 本码）。
    pub const KV_LIMIT: &str = "KV_LIMIT";
    /// 内部错误（重入驱动、不可能路径——出现即驱动 bug）。
    pub const INTERNAL: &str = "INTERNAL";
}

/// 驱动构造错误（dispatch 信封面之外的 Rust 侧错误：构造/restore 即失败）。
#[derive(Debug, ThisError)]
pub enum KernelError {
    #[error(transparent)]
    Sandbox(#[from] LuaError),
    #[error("mod load failed: {0}")]
    ModLoad(String),
    #[error(transparent)]
    Save(#[from] SaveError),
    #[error("engine version mismatch: save written by `{saved}`, current engine `{current}`")]
    EngineMismatch { saved: String, current: String },
    #[error(transparent)]
    Snapshot(#[from] SnapshotError),
    /// 资源包**校验**失败伞形变体（manifest 违规 / payload 违规 / 未知
    /// schema id）；`details` 为首条违规的人读文案（含 file/pointer/code）。
    #[error("resource package rejected: {details}")]
    InvalidManifest { details: String },
    /// 资源内容寻址失败（文件缺失 / byteLength 不符 / sha256 不符）。
    #[error("resource content mismatch: {details}")]
    ContentMismatch { details: String },
    /// 资源预算超限（`KV_LIMIT` 预留码激活点）。
    #[error("resource budget exceeded: {total} bytes requested, limit {limit}")]
    ResourceBudget { total: usize, limit: usize },
}

impl KernelError {
    /// 错误 → A1 稳定码的**单点映射面**（下游按码分支不解析 message；
    /// 全仓任何 KernelError→code 判定必须经本方法，禁止散落 match）：
    ///
    /// | 变体 | 码 | 依据 |
    /// | --- | --- | --- |
    /// | `InvalidManifest` / `ContentMismatch` | `VALIDATION` | 资源包内容不合契约（作者侧可修复） |
    /// | `ResourceBudget` | `KV_LIMIT` | S11 预留码激活（体积限面） |
    /// | `EngineMismatch` | `VALIDATION` | 存档/引擎版本不符（与 `load` 命令同码） |
    /// | `Sandbox` / `ModLoad` / `Save` / `Snapshot` | `INTERNAL` | 结构面/编程面错误（非作者可修复） |
    pub fn code(&self) -> &'static str {
        match self {
            KernelError::InvalidManifest { .. } | KernelError::ContentMismatch { .. } => {
                code::VALIDATION
            }
            KernelError::ResourceBudget { .. } => code::KV_LIMIT,
            KernelError::EngineMismatch { .. } => code::VALIDATION,
            KernelError::Sandbox(_)
            | KernelError::ModLoad(_)
            | KernelError::Save(_)
            | KernelError::Snapshot(_) => code::INTERNAL,
        }
    }
}
/// [`ModuleError`] → [`KernelError`]：模块面拒绝一律走 `ModLoad`（装载期
/// 模块错误与 Mod 源码错误的同一出口，便于上层按码分类）。
impl From<ModuleError> for KernelError {
    fn from(err: ModuleError) -> Self {
        KernelError::ModLoad(err.to_string())
    }
}
