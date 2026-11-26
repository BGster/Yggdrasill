//! 只读观察面：给宿主这类**不持 run 锁的观察者**读取状态事实。
//!
//! # 为什么不进 [`crate::RunStore`]
//!
//! trait 方法带着「调用方已持有 run 锁」的前提；观察者不满足也不需要——
//! 它与宿主进程无关，只看盘上既成事实。存档历史是**后端知识**，必须住在这个
//! 端口 crate 里，不能泄进宿主。
//!
//! # 零写入纪律（每条都有测试锁，见 `tests`）
//!
//! JSON 路径全部纯读：[`crate::JsonFileStore::open`] 只记路径不落盘，
//! `steps()` / `next_seq()` / `artifact_*` 均为只读方法。
//!
//! # 并发论证（观察者不持锁为什么安全）
//!
//! JSON：tmp+rename 原子替换保证读者只见旧或新整代。
//!
//! # 写面归属
//!
//! 写面仍归 [`crate::RunStore`]（锁内）。本模块没有任何写方法。

use std::path::Path;

use crate::{
    ArtifactMeta, ArtifactRecord, Backend, RunStore, StepRecord, StoreError, AUTOSAVE_FILE,
};

/// 存档列表的缺省条数（`save_list` 的 `limit == 0` 时生效）。
pub const SAVE_LIST_DEFAULT_LIMIT: usize = 50;
/// 存档列表的硬上限（观察面封顶：失控规模如实截断，不内卷内存）。
pub const SAVE_LIST_MAX_LIMIT: usize = 200;
/// 事件分页的硬上限（`event_page` 的 `limit` 封顶）。
pub const EVENT_PAGE_MAX_LIMIT: u32 = 500;

// ============================================================================
// §数据形状
// ============================================================================

/// 存档元信息（第二级列表的一行）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveMeta {
    /// json：恒 `"latest"`（单档后端没有历史）。
    pub id: String,
    /// json：`None`（无代概念）。
    pub generation: Option<i64>,
    /// 该存档对应的步（json：events 非空行数）。
    pub seq: u64,
    /// unix 秒（json：autosave mtime，读不到 = 0）。
    pub created_at: u64,
    pub bytes: usize,
    /// 是否当前代最新档。
    pub latest: bool,
}

/// watcher 变更探针（每 poll 每 run 一次，必须廉价：一条聚合查询）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaveProbe {
    pub generation: Option<i64>,
    pub latest_id: Option<String>,
    pub latest_created_at: u64,
    pub count: u64,
}

/// 事件分页读取结果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventPage {
    /// `seq` 升序、自 `since` 起、至多 `limit` 条（原文透传：`payload` 原样）。
    pub records: Vec<StepRecord>,
    /// 当前代已提交步数（= 下一 seq；与 [`crate::RunStore::next_seq`] 同口径，
    /// 含 JSON 后端不可解析的占号行）。
    pub latest: u64,
}

// ============================================================================
// §入口（按后端分派）
// ============================================================================

/// watcher 变更探针：当前代最新档 + 存档总数。
pub fn save_probe(run_dir: &Path, backend: Backend) -> Result<SaveProbe, StoreError> {
    match backend {
        Backend::Json => json_probe(run_dir),
    }
}

/// 存档列表（新→旧）。`limit == 0` → [`SAVE_LIST_DEFAULT_LIMIT`]；上限
/// [`SAVE_LIST_MAX_LIMIT`]。JSON 局恒单行（`id="latest"`）或空。
pub fn save_list(
    run_dir: &Path,
    backend: Backend,
    limit: usize,
) -> Result<Vec<SaveMeta>, StoreError> {
    let limit = clamp_save_limit(limit);
    match backend {
        Backend::Json => json_save_list(run_dir, limit),
    }
}

/// 取一份存档原文。`"latest"` = 当前代最新档。
/// 其余形状（含 JSON 局的数字 id）→ `Ok(None)`（语法门在调用方，端口不报错）。
pub fn save_get(
    run_dir: &Path,
    backend: Backend,
    id: &str,
) -> Result<Option<(SaveMeta, Vec<u8>)>, StoreError> {
    match backend {
        Backend::Json => json_save_get(run_dir, id),
    }
}

/// 事件分页（`seq` 升序，自 `since` 起，至多 `limit` 条，封顶
/// [`EVENT_PAGE_MAX_LIMIT`]）。JSON 后端坏行**仍占 seq**（`latest` 口径不破坏）。
pub fn event_page(
    run_dir: &Path,
    backend: Backend,
    since: u64,
    limit: u32,
) -> Result<EventPage, StoreError> {
    let limit = limit.min(EVENT_PAGE_MAX_LIMIT);
    match backend {
        Backend::Json => json_event_page(run_dir, since, limit),
    }
}

/// 当前代制品元信息（`path` 升序；与 [`crate::RunStore::artifact_list`] 同语义）。
pub fn artifact_list_view(
    run_dir: &Path,
    backend: Backend,
) -> Result<Vec<ArtifactMeta>, StoreError> {
    match backend {
        Backend::Json => crate::json::JsonFileStore::open(run_dir)?.artifact_list(),
    }
}

/// 读一件制品原文（缺失 → `Ok(None)`；与 [`crate::RunStore::artifact_get`] 同语义）。
pub fn artifact_get_view(
    run_dir: &Path,
    backend: Backend,
    path: &str,
) -> Result<Option<ArtifactRecord>, StoreError> {
    match backend {
        Backend::Json => crate::json::JsonFileStore::open(run_dir)?.artifact_get(path),
    }
}

/// 存档列表 limit 的统一钳制（`0` = 缺省；封顶 [`SAVE_LIST_MAX_LIMIT`]）。
fn clamp_save_limit(limit: usize) -> usize {
    if limit == 0 {
        SAVE_LIST_DEFAULT_LIMIT
    } else {
        limit.min(SAVE_LIST_MAX_LIMIT)
    }
}

// ============================================================================
// §JSON 路径（纯读：JsonFileStore 只记路径；mtime 用作 created_at）
// ============================================================================

fn json_probe(run_dir: &Path) -> Result<SaveProbe, StoreError> {
    let autosave = run_dir.join(AUTOSAVE_FILE);
    if !autosave.exists() {
        return Ok(SaveProbe::default());
    }
    let meta = std::fs::metadata(&autosave)
        .map_err(|err| StoreError::io(format!("读取 {} 失败", autosave.display()), err))?;
    Ok(SaveProbe {
        generation: None,
        latest_id: Some("latest".to_owned()),
        latest_created_at: json_mtime_secs(&meta),
        count: 1,
    })
}

fn json_save_list(run_dir: &Path, limit: usize) -> Result<Vec<SaveMeta>, StoreError> {
    let store = crate::json::JsonFileStore::open(run_dir)?;
    let autosave = run_dir.join(AUTOSAVE_FILE);
    if !autosave.exists() {
        return Ok(Vec::new());
    }
    if limit == 0 {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(&autosave)
        .map_err(|err| StoreError::io(format!("读取 {} 失败", autosave.display()), err))?;
    let created_at = std::fs::metadata(&autosave)
        .map(|meta| json_mtime_secs(&meta))
        .unwrap_or(0);
    Ok(vec![SaveMeta {
        id: "latest".to_owned(),
        generation: None,
        seq: store.next_seq()?,
        created_at,
        bytes: bytes.len(),
        latest: true,
    }])
}

fn json_save_get(run_dir: &Path, id: &str) -> Result<Option<(SaveMeta, Vec<u8>)>, StoreError> {
    if id != "latest" {
        // 单档后端没有历史快照可寻：物理事实如实返回「没有」，不伪造。
        return Ok(None);
    }
    let Some(meta) = json_save_list(run_dir, 1)?.into_iter().next() else {
        return Ok(None);
    };
    let bytes = std::fs::read(run_dir.join(AUTOSAVE_FILE))
        .map_err(|err| StoreError::io("读取存档失败".to_owned(), err))?;
    Ok(Some((meta, bytes)))
}

fn json_event_page(run_dir: &Path, since: u64, limit: u32) -> Result<EventPage, StoreError> {
    let store = crate::json::JsonFileStore::open(run_dir)?;
    let log = store.steps()?;
    // `latest` = 非空行数（含坏行占号），与 `next_seq()` 同一口径。
    let latest = log.records.len() as u64 + log.skipped as u64;
    let records = log
        .records
        .into_iter()
        .filter(|record| record.seq >= since)
        .take(limit as usize)
        .collect();
    Ok(EventPage { records, latest })
}

/// autosave mtime → unix 秒（读不到 = 0：mtime 是便利面不是判据）。
fn json_mtime_secs(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[cfg(test)]
mod tests;
