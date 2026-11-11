//! 只读观察面：给 `serve` 这类**不持 run 锁的观察者**读取状态事实。
//!
//! # 为什么不进 [`crate::RunStore`]
//!
//! trait 方法带着「调用方已持有 run 锁」的前提，而 [`crate::SqliteStore::open`]
//! 还带 migrate 写面（建表 + `journal_mode=WAL`）——观察者两个都不满足/不需要：
//! 它与宿主进程无关，只看盘上既成事实。存档历史（sqlite `snapshot` 表 vs JSON
//! 单档）是**后端知识**，必须住在这个端口 crate 里，不能泄进宿主。
//!
//! # 零写入纪律（每条都有测试锁，见 `observe_tests`）
//!
//! - **先探测后连接**：库文件缺失 → 空探针 / 空列表 / `None`，**绝不建库**；
//! - sqlite 连接用 `SQLITE_OPEN_NO_CREATE`（防探测-连接之间的 TOCTOU 建库）+
//!   `PRAGMA query_only = 1`（封死写面）+ `busy_timeout(5000)`，**不跑
//!   migrate**（对残缺库会写、对无库目录会建库）；
//! - 为什么以读写方式连接而非 `SQLITE_OPEN_READ_ONLY`：已收口的局 `-wal/-shm`
//!   缺席时只读打开会因缺 shm 失败；`query_only` 已封死写面，连接可能创建的
//!   `-wal/-shm` 属**连接级产物**（宿主 `cleanup_stale` 语义已把该文件类定义为
//!   活文件），不是状态事实；
//! - JSON 路径全部纯读：[`crate::JsonFileStore::open`] 只记路径不落盘，
//!   `steps()` / `next_seq()` / `artifact_*` 均为只读方法。
//!
//! # 并发论证（观察者不持锁为什么安全）
//!
//! sqlite：WAL 并发读者安全，写者冲突由 `busy_timeout` 等待；JSON：tmp+rename
//! 原子替换保证读者只见旧或新整代。此论证与宿主 watcher 读 `latest_save()`
//! 同源——本模块把「观察者读」收口成唯一形态。
//!
//! # 写面归属
//!
//! 写面仍归 [`crate::RunStore`]（锁内）。本模块没有任何写方法，也没有暴露
//! `Connection`——后端知识不外泄。

use std::path::Path;

use crate::{
    ArtifactMeta, ArtifactRecord, Backend, RunStore, StepRecord, StoreError, AUTOSAVE_FILE,
};

#[cfg(feature = "sqlite")]
use crate::SQLITE_FILE_NAME;
#[cfg(feature = "sqlite")]
use rusqlite::OptionalExtension;

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
    /// sqlite：十进制 `snapshot.id`；json：恒 `"latest"`（单档后端没有历史）。
    pub id: String,
    /// sqlite：`snapshot.generation`；json：`None`（无代概念）。
    pub generation: Option<i64>,
    /// 该存档对应的步（sqlite：`snapshot.seq`；json：events 非空行数）。
    pub seq: u64,
    /// unix 秒（sqlite：`snapshot.created_at`；json：autosave mtime，读不到 = 0）。
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
// §入口（按后端分派；`--no-default-features` 下 sqlite 面 fail-closed）
// ============================================================================

/// watcher 变更探针：当前代最新档 + 存档总数。
pub fn save_probe(run_dir: &Path, backend: Backend) -> Result<SaveProbe, StoreError> {
    match backend {
        Backend::Json => json_probe(run_dir),
        #[cfg(feature = "sqlite")]
        Backend::Sqlite => sqlite_probe(run_dir),
        #[cfg(not(feature = "sqlite"))]
        Backend::Sqlite => Err(unavailable_sqlite()),
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
        #[cfg(feature = "sqlite")]
        Backend::Sqlite => sqlite_save_list(run_dir, limit),
        #[cfg(not(feature = "sqlite"))]
        Backend::Sqlite => Err(unavailable_sqlite()),
    }
}

/// 取一份存档原文。`"latest"` = 当前代最新档；sqlite 另认十进制 `snapshot.id`；
/// 其余形状（含 JSON 局的数字 id）→ `Ok(None)`（语法门在调用方，端口不报错）。
pub fn save_get(
    run_dir: &Path,
    backend: Backend,
    id: &str,
) -> Result<Option<(SaveMeta, Vec<u8>)>, StoreError> {
    match backend {
        Backend::Json => json_save_get(run_dir, id),
        #[cfg(feature = "sqlite")]
        Backend::Sqlite => sqlite_save_get(run_dir, id),
        #[cfg(not(feature = "sqlite"))]
        Backend::Sqlite => Err(unavailable_sqlite()),
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
        #[cfg(feature = "sqlite")]
        Backend::Sqlite => sqlite_event_page(run_dir, since, limit),
        #[cfg(not(feature = "sqlite"))]
        Backend::Sqlite => Err(unavailable_sqlite()),
    }
}

/// 当前代制品元信息（`path` 升序；与 [`crate::RunStore::artifact_list`] 同语义）。
pub fn artifact_list_view(
    run_dir: &Path,
    backend: Backend,
) -> Result<Vec<ArtifactMeta>, StoreError> {
    match backend {
        Backend::Json => crate::json::JsonFileStore::open(run_dir)?.artifact_list(),
        #[cfg(feature = "sqlite")]
        Backend::Sqlite => sqlite_artifact_list(run_dir),
        #[cfg(not(feature = "sqlite"))]
        Backend::Sqlite => Err(unavailable_sqlite()),
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
        #[cfg(feature = "sqlite")]
        Backend::Sqlite => sqlite_artifact_get(run_dir, path),
        #[cfg(not(feature = "sqlite"))]
        Backend::Sqlite => Err(unavailable_sqlite()),
    }
}

/// `--no-default-features` 构建（无 sqlite）下，sqlite 观察面 fail-closed。
#[cfg(not(feature = "sqlite"))]
fn unavailable_sqlite() -> StoreError {
    StoreError::Unavailable {
        detail: "本构建未启用 sqlite 后端（sqlite 是编译期面：宿主 `cargo build \
                 --example agent` 默认带上，`--no-default-features` 会关掉它；\
                 ygg-store 自身需 `--features sqlite`）"
            .to_owned(),
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

// ============================================================================
// §sqlite 路径（先探测后连接；NO_CREATE + query_only + busy_timeout；不 migrate）
// ============================================================================

#[cfg(feature = "sqlite")]
fn sqlite_connect(run_dir: &Path) -> Result<Option<rusqlite::Connection>, StoreError> {
    let db_path = run_dir.join(SQLITE_FILE_NAME);
    if !db_path.exists() {
        // 无库 = 无存档事实（空探针 / 空列表 / None），**绝不建库**。
        return Ok(None);
    }
    // 不带 SQLITE_OPEN_CREATE：库文件若在探测后被删（TOCTOU），open 直接失败而
    // 不是静默建空库（SQLite 无独立的 NO_CREATE 旗标——CREATE 是 opt-in）。
    let flags =
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = rusqlite::Connection::open_with_flags(&db_path, flags).map_err(|err| {
        StoreError::corrupt(format!("打开 {} 失败", db_path.display()), err.to_string())
    })?;
    conn.busy_timeout(std::time::Duration::from_millis(
        crate::sqlite::DEFAULT_BUSY_TIMEOUT_MS,
    ))
    .map_err(|err| {
        StoreError::corrupt(
            format!("设置 busy_timeout 失败（{}）", db_path.display()),
            err.to_string(),
        )
    })?;
    // 封死写面：query_only 之后的任何写语句（含 DDL/PRAGMA 写面）都报错。
    // **不跑 migrate**：migrate 会建表 + 写 schema_version（观察者零写入红线）。
    conn.execute_batch("PRAGMA query_only = 1;")
        .map_err(|err| {
            StoreError::corrupt(
                format!("设置 query_only 失败（{}）", db_path.display()),
                err.to_string(),
            )
        })?;
    Ok(Some(conn))
}

/// 当前代（与宿主 `current_generation` 同一判定：`MAX(generation)`，空表 = 0）。
#[cfg(feature = "sqlite")]
fn sqlite_current_generation(
    conn: &rusqlite::Connection,
    db_path: &Path,
) -> Result<i64, StoreError> {
    conn.query_row(
        "SELECT COALESCE(MAX(generation), 0) FROM generation",
        [],
        |row| row.get(0),
    )
    .map_err(|err| {
        StoreError::corrupt(
            format!("读取 generation 失败（{}）", db_path.display()),
            err.to_string(),
        )
    })
}

#[cfg(feature = "sqlite")]
const SAVE_COLUMNS: &str = "id, generation, seq, LENGTH(save), created_at";

#[cfg(feature = "sqlite")]
fn sqlite_row_to_meta(row: &rusqlite::Row<'_>, latest: bool) -> rusqlite::Result<SaveMeta> {
    let id: i64 = row.get(0)?;
    let generation: i64 = row.get(1)?;
    let seq: i64 = row.get(2)?;
    let bytes: i64 = row.get(3)?;
    let created_at: i64 = row.get(4)?;
    Ok(SaveMeta {
        id: id.to_string(),
        generation: Some(generation),
        seq: seq.max(0) as u64,
        created_at: created_at.max(0) as u64,
        bytes: bytes.max(0) as usize,
        latest,
    })
}

#[cfg(feature = "sqlite")]
fn sqlite_probe(run_dir: &Path) -> Result<SaveProbe, StoreError> {
    let db_path = run_dir.join(SQLITE_FILE_NAME);
    let Some(conn) = sqlite_connect(run_dir)? else {
        return Ok(SaveProbe::default());
    };
    let generation = sqlite_current_generation(&conn, &db_path)?;
    let probe = conn
        .query_row(
            "SELECT COUNT(*), COALESCE(MAX(id), 0), \
             COALESCE(MAX(CASE WHEN id = (SELECT MAX(id) FROM snapshot WHERE generation = ?1) \
             THEN created_at END), 0) FROM snapshot WHERE generation = ?1",
            [generation],
            |row| {
                let count: i64 = row.get(0)?;
                let max_id: i64 = row.get(1)?;
                Ok(SaveProbe {
                    generation: Some(generation),
                    latest_id: (max_id > 0).then(|| max_id.to_string()),
                    latest_created_at: row.get::<_, i64>(2)?.max(0) as u64,
                    count: count.max(0) as u64,
                })
            },
        )
        .map_err(|err| {
            StoreError::corrupt(
                format!("读取存档探针失败（{}）", db_path.display()),
                err.to_string(),
            )
        })?;
    Ok(probe)
}

#[cfg(feature = "sqlite")]
fn sqlite_save_list(run_dir: &Path, limit: usize) -> Result<Vec<SaveMeta>, StoreError> {
    let db_path = run_dir.join(SQLITE_FILE_NAME);
    let Some(conn) = sqlite_connect(run_dir)? else {
        return Ok(Vec::new());
    };
    let generation = sqlite_current_generation(&conn, &db_path)?;
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {SAVE_COLUMNS} FROM snapshot WHERE generation = ?1 ORDER BY id DESC LIMIT ?2"
        ))
        .map_err(|err| {
            StoreError::corrupt(
                format!("准备存档列表查询失败（{}）", db_path.display()),
                err.to_string(),
            )
        })?;
    let rows = stmt
        .query_map([generation, limit as i64], |row| {
            // latest 由行序决定：`ORDER BY id DESC` 的第一行即当前代最新档。
            sqlite_row_to_meta(row, false)
        })
        .map_err(|err| {
            StoreError::corrupt(
                format!("读取存档列表失败（{}）", db_path.display()),
                err.to_string(),
            )
        })?;
    let mut out = Vec::new();
    for (index, row) in rows.enumerate() {
        let mut meta = row.map_err(|err| {
            StoreError::corrupt(
                format!("读取存档列表失败（{}）", db_path.display()),
                err.to_string(),
            )
        })?;
        meta.latest = index == 0;
        out.push(meta);
    }
    Ok(out)
}

#[cfg(feature = "sqlite")]
fn sqlite_save_get(run_dir: &Path, id: &str) -> Result<Option<(SaveMeta, Vec<u8>)>, StoreError> {
    let db_path = run_dir.join(SQLITE_FILE_NAME);
    let Some(conn) = sqlite_connect(run_dir)? else {
        return Ok(None);
    };
    let generation = sqlite_current_generation(&conn, &db_path)?;
    // "latest" 别名 = 当前代 MAX(id)；数字 id 精确匹配（限当前代）；
    // 其余形状 → None（语法门在调用方，端口不报错）。
    let numeric = id.parse::<i64>().ok();
    if id != "latest" && numeric.is_none() {
        return Ok(None);
    }
    let sql = if id == "latest" {
        format!(
            "SELECT {SAVE_COLUMNS}, save FROM snapshot WHERE generation = ?1 \
             ORDER BY id DESC LIMIT 1"
        )
    } else {
        format!(
            "SELECT {SAVE_COLUMNS}, save FROM snapshot WHERE generation = ?1 AND id = ?2 LIMIT 1"
        )
    };
    let mut stmt = conn.prepare(&sql).map_err(|err| {
        StoreError::corrupt(
            format!("准备存档读取失败（{}）", db_path.display()),
            err.to_string(),
        )
    })?;
    let query = |row: &rusqlite::Row<'_>| -> rusqlite::Result<(SaveMeta, Vec<u8>)> {
        let mut meta = sqlite_row_to_meta(row, false)?;
        let save: Vec<u8> = row.get(5)?;
        meta.bytes = save.len();
        Ok((meta, save))
    };
    let result = if id == "latest" {
        stmt.query_row([generation], query).optional()
    } else {
        stmt.query_row([generation, numeric.unwrap_or(0)], query)
            .optional()
    };
    let latest_id: Option<i64> = conn
        .query_row(
            "SELECT MAX(id) FROM snapshot WHERE generation = ?1",
            [generation],
            |row| row.get(0),
        )
        .map_err(|err| {
            StoreError::corrupt(
                format!("读取最新档 id 失败（{}）", db_path.display()),
                err.to_string(),
            )
        })?;
    result
        .map_err(|err| {
            StoreError::corrupt(
                format!("读取存档失败（{}）", db_path.display()),
                err.to_string(),
            )
        })
        .map(|found| {
            found.map(|(mut meta, bytes)| {
                meta.latest = latest_id == Some(meta.id.parse::<i64>().unwrap_or(-1));
                (meta, bytes)
            })
        })
}

#[cfg(feature = "sqlite")]
fn sqlite_event_page(run_dir: &Path, since: u64, limit: u32) -> Result<EventPage, StoreError> {
    let db_path = run_dir.join(SQLITE_FILE_NAME);
    let Some(conn) = sqlite_connect(run_dir)? else {
        return Ok(EventPage::default());
    };
    let generation = sqlite_current_generation(&conn, &db_path)?;
    let latest: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(seq) + 1, 0) FROM step WHERE generation = ?1",
            [generation],
            |row| row.get(0),
        )
        .map_err(|err| {
            StoreError::corrupt(
                format!("读取事件步数失败（{}）", db_path.display()),
                err.to_string(),
            )
        })?;
    let mut stmt = conn
        .prepare(
            "SELECT seq, event, payload, ok, narrative_count, timestamp, ending, ending_line \
             FROM step WHERE generation = ?1 AND seq >= ?2 ORDER BY seq LIMIT ?3",
        )
        .map_err(|err| {
            StoreError::corrupt(
                format!("准备事件查询失败（{}）", db_path.display()),
                err.to_string(),
            )
        })?;
    let rows = stmt
        .query_map([generation, since as i64, i64::from(limit)], |row| {
            Ok(StepRecord {
                seq: row.get::<_, i64>(0)?.max(0) as u64,
                event: row.get(1)?,
                payload: row.get(2)?,
                ok: row.get::<_, i64>(3)? != 0,
                narrative_count: row.get::<_, i64>(4)?.max(0) as usize,
                timestamp: row.get::<_, i64>(5)?.max(0) as u64,
                ending: row.get::<_, i64>(6)? != 0,
                ending_line: row.get(7)?,
            })
        })
        .map_err(|err| {
            StoreError::corrupt(
                format!("读取事件记录失败（{}）", db_path.display()),
                err.to_string(),
            )
        })?;
    let mut records = Vec::new();
    for record in rows {
        records.push(record.map_err(|err| {
            StoreError::corrupt(
                format!("读取事件记录失败（{}）", db_path.display()),
                err.to_string(),
            )
        })?);
    }
    Ok(EventPage {
        records,
        latest: latest.max(0) as u64,
    })
}

#[cfg(feature = "sqlite")]
fn sqlite_artifact_list(run_dir: &Path) -> Result<Vec<ArtifactMeta>, StoreError> {
    let db_path = run_dir.join(SQLITE_FILE_NAME);
    let Some(conn) = sqlite_connect(run_dir)? else {
        return Ok(Vec::new());
    };
    let generation = sqlite_current_generation(&conn, &db_path)?;
    let mut stmt = conn
        .prepare(
            "SELECT path, bytes, sha256, updated_at FROM artifact \
             WHERE generation = ?1 ORDER BY path",
        )
        .map_err(|err| {
            StoreError::corrupt(
                format!("准备制品查询失败（{}）", db_path.display()),
                err.to_string(),
            )
        })?;
    let rows = stmt
        .query_map([generation], |row| {
            let path: String = row.get(0)?;
            let bytes: i64 = row.get(1)?;
            Ok(ArtifactMeta {
                path,
                bytes: bytes.max(0) as usize,
                sha256: row.get(2)?,
                updated_at: row.get::<_, i64>(3)?.max(0) as u64,
            })
        })
        .map_err(|err| {
            StoreError::corrupt(
                format!("读取制品列表失败（{}）", db_path.display()),
                err.to_string(),
            )
        })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|err| {
            StoreError::corrupt(
                format!("读取制品列表失败（{}）", db_path.display()),
                err.to_string(),
            )
        })?);
    }
    Ok(out)
}

#[cfg(feature = "sqlite")]
fn sqlite_artifact_get(run_dir: &Path, path: &str) -> Result<Option<ArtifactRecord>, StoreError> {
    crate::validate_artifact_path(path)?;
    let db_path = run_dir.join(SQLITE_FILE_NAME);
    let Some(conn) = sqlite_connect(run_dir)? else {
        return Ok(None);
    };
    let generation = sqlite_current_generation(&conn, &db_path)?;
    conn.query_row(
        "SELECT sha256, bytes, content, updated_at FROM artifact \
         WHERE generation = ?1 AND path = ?2",
        rusqlite::params![generation, path],
        |row| {
            let sha256: String = row.get(0)?;
            let bytes: i64 = row.get(1)?;
            let content: Vec<u8> = row.get(2)?;
            let updated_at: i64 = row.get(3)?;
            Ok(ArtifactRecord {
                meta: ArtifactMeta {
                    path: path.to_owned(),
                    bytes: bytes.max(0) as usize,
                    sha256,
                    updated_at: updated_at.max(0) as u64,
                },
                content,
            })
        },
    )
    .optional()
    .map_err(|err| {
        StoreError::corrupt(
            format!("读取制品失败（{}）", db_path.display()),
            err.to_string(),
        )
    })
}

// ============================================================================
// §crate 内单测（两后端分列；零写入 / 零创建纯度锁）
// ============================================================================

#[cfg(test)]
mod observe_tests {
    use super::*;
    use crate::{RunStore, StepDraft, Tick};
    use std::fs;
    use std::path::PathBuf;

    const TICK: Tick = Tick {
        secs: 1_700_000_000,
        nanos: 11,
    };

    fn case_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ygg-store-observe-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("建临时 run 目录");
        dir
    }

    /// 提交 N 个成功步（每步带一份可辨识存档），返回最后一次的结果。
    fn commit_steps(store: &mut dyn RunStore, n: u64, tag: &[u8]) -> Result<(), crate::StoreError> {
        for i in 0..n {
            let payload = format!("{{\"i\":{i}}}");
            let save = save_bytes(i, tag);
            store.commit_step(StepDraft {
                event: "step",
                payload: &payload,
                ok: true,
                narrative_count: 1,
                timestamp: TICK.secs + i,
                ending_line: None,
                save: Some(&save),
            })?;
        }
        Ok(())
    }

    fn save_bytes(i: u64, tag: &[u8]) -> Vec<u8> {
        format!(
            "{{\"step\":{i},\"tag\":\"{}\"}}",
            String::from_utf8_lossy(tag)
        )
        .into_bytes()
    }

    #[cfg(feature = "sqlite")]
    fn sqlite_dir(tag: &str, steps: u64) -> (PathBuf, Box<dyn RunStore>) {
        let dir = case_dir(tag);
        let mut store: Box<dyn RunStore> =
            crate::open(Backend::Sqlite, &dir, TICK).expect("打开 sqlite 局");
        commit_steps(store.as_mut(), steps, b"sqlite").expect("提交步");
        (dir, store)
    }

    fn json_dir(tag: &str, steps: u64) -> (PathBuf, Box<dyn RunStore>) {
        let dir = case_dir(tag);
        let mut store: Box<dyn RunStore> =
            crate::open(Backend::Json, &dir, TICK).expect("打开 json 局");
        commit_steps(store.as_mut(), steps, b"json").expect("提交步");
        (dir, store)
    }

    // ---- sqlite 路径 ----

    #[cfg(feature = "sqlite")]
    #[test]
    fn save_list_sqlite_orders_desc() {
        let (dir, _store) = sqlite_dir("desc", 3);
        let list = save_list(&dir, Backend::Sqlite, 0).expect("列表可读");
        assert_eq!(list.len(), 3, "每成功步一行存档");
        // 新→旧：id 降序（3、2、1），seq 逐行对齐，首行 latest。
        let ids: Vec<&str> = list.iter().map(|meta| meta.id.as_str()).collect();
        assert_eq!(ids, vec!["3", "2", "1"]);
        let seqs: Vec<u64> = list.iter().map(|meta| meta.seq).collect();
        assert_eq!(seqs, vec![2, 1, 0]);
        assert!(list[0].latest, "首行是当前代最新档");
        assert!(!list[1].latest && !list[2].latest);
        assert_eq!(list[0].generation, Some(0), "新局从代 0 起步");
        // bytes 与存档本体一致；created_at 来自提交时刻。
        assert_eq!(list[0].bytes, save_bytes(2, b"sqlite").len());
        assert_eq!(list[0].created_at, TICK.secs + 2);
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn save_list_limit_clamped() {
        let (dir, _store) = sqlite_dir("limit", 4);
        // limit=0 → 缺省 50（全量 4 行）。
        assert_eq!(save_list(&dir, Backend::Sqlite, 0).unwrap().len(), 4);
        // limit=2 → 只取最新 2 行，且仍是新→旧。
        let list = save_list(&dir, Backend::Sqlite, 2).unwrap();
        assert_eq!(
            list.iter().map(|meta| meta.id.as_str()).collect::<Vec<_>>(),
            ["4", "3"]
        );
        // 超上限 → 封顶（造 4 行够不着 200，直接断言常量本身被钳制面引用）。
        assert_eq!(clamp_save_limit(usize::MAX), SAVE_LIST_MAX_LIMIT);
        assert_eq!(clamp_save_limit(0), SAVE_LIST_DEFAULT_LIMIT);
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn save_get_by_id_and_latest() {
        let (dir, _store) = sqlite_dir("get", 3);
        // "latest" 与最大 id 字节等值。
        let (_, latest_bytes) = save_get(&dir, Backend::Sqlite, "latest")
            .expect("读 latest")
            .expect("latest 存在");
        let (_, by_id) = save_get(&dir, Backend::Sqlite, "3")
            .expect("读 id")
            .expect("id=3 存在");
        assert_eq!(latest_bytes, by_id);
        // 十进制 id 逐字节取回。
        let (meta, bytes) = save_get(&dir, Backend::Sqlite, "1").unwrap().unwrap();
        assert_eq!(bytes, save_bytes(0, b"sqlite"));
        assert_eq!(meta.seq, 0);
        assert!(!meta.latest, "非最新档 latest=false");
        // 缺 id → None；非数字非 latest → None（语法门在调用方）。
        assert!(save_get(&dir, Backend::Sqlite, "999999").unwrap().is_none());
        assert!(save_get(&dir, Backend::Sqlite, "abc").unwrap().is_none());
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn observe_creates_nothing() {
        let dir = case_dir("creates");
        fs::write(dir.join("run.json"), b"{\"run_id\":\"bare\"}").expect("只写 run.json");
        // 全部观察入口打在一个「只有 run.json」的目录上。
        assert_eq!(
            save_probe(&dir, Backend::Sqlite).unwrap(),
            SaveProbe::default()
        );
        assert!(save_list(&dir, Backend::Sqlite, 0).unwrap().is_empty());
        assert!(save_get(&dir, Backend::Sqlite, "latest").unwrap().is_none());
        let page = event_page(&dir, Backend::Sqlite, 0, 10).unwrap();
        assert!(page.records.is_empty() && page.latest == 0);
        assert!(artifact_list_view(&dir, Backend::Sqlite)
            .unwrap()
            .is_empty());
        // 纯度：目录里**没有多出任何文件**（run.sqlite / -wal / -shm 都不许有）。
        let entries: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            entries,
            vec!["run.json"],
            "观察面不得创建任何文件：{entries:?}"
        );
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn observe_writes_nothing() {
        let (dir, _store) = sqlite_dir("writes", 2);
        // 断言范围 = run 目录的**逻辑产物**。-wal / -shm 明确排除：observe 连接
        // 可能创建它们（连接级产物，宿主 cleanup_stale 语义已定义为活文件类），
        // 主库字节在查询下不变（query_only + 只读 SQL）。
        fn digest(dir: &Path) -> Vec<(String, u64)> {
            let mut out: Vec<(String, u64)> = fs::read_dir(dir)
                .unwrap()
                .map(|entry| {
                    let path = entry.unwrap().path();
                    let name = path.file_name().unwrap().to_string_lossy().into_owned();
                    let len = fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
                    (name, len)
                })
                .filter(|(name, _)| !name.ends_with("-wal") && !name.ends_with("-shm"))
                .collect();
            out.sort();
            out
        }
        let before = digest(&dir);
        save_probe(&dir, Backend::Sqlite).unwrap();
        save_list(&dir, Backend::Sqlite, 0).unwrap();
        save_get(&dir, Backend::Sqlite, "latest").unwrap();
        event_page(&dir, Backend::Sqlite, 0, 10).unwrap();
        artifact_list_view(&dir, Backend::Sqlite).unwrap();
        artifact_get_view(&dir, Backend::Sqlite, "artifacts/a.md").unwrap();
        assert_eq!(digest(&dir), before, "观察面不得改动任何既有产物");
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn event_page_since_limit() {
        let (dir, _store) = sqlite_dir("page", 5);
        let page = event_page(&dir, Backend::Sqlite, 2, 2).unwrap();
        assert_eq!(page.latest, 5, "latest = 已提交步数（下一 seq）");
        let seqs: Vec<u64> = page.records.iter().map(|record| record.seq).collect();
        assert_eq!(seqs, vec![2, 3], "自 since 起 seq 升序、limit 截断");
        // payload 原文透传。
        assert_eq!(page.records[0].payload, "{\"i\":2}");
        // limit 封顶常量面。
        assert_eq!(EVENT_PAGE_MAX_LIMIT, 500);
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn artifact_views_roundtrip() {
        let (dir, mut store) = sqlite_dir("artifact", 1);
        let meta = store
            .artifact_put("artifacts/note.md", "正文".as_bytes(), TICK)
            .expect("写制品");
        let view = artifact_list_view(&dir, Backend::Sqlite).unwrap();
        assert_eq!(view, vec![meta.clone()]);
        let record = artifact_get_view(&dir, Backend::Sqlite, "artifacts/note.md")
            .unwrap()
            .expect("制品存在");
        assert_eq!(record.meta, meta);
        assert_eq!(record.content, "正文".as_bytes());
        // 与 RunStore::artifact_list / artifact_get 语义等价。
        assert_eq!(store.artifact_list().unwrap(), view);
        assert_eq!(
            store.artifact_get("artifacts/note.md").unwrap(),
            Some(record)
        );
    }

    // ---- JSON 路径 ----

    #[test]
    fn save_list_json_single_row() {
        let (dir, mut store) = json_dir("json-single", 2);
        let list = save_list(&dir, Backend::Json, 0).expect("列表可读");
        assert_eq!(list.len(), 1, "JSON 局恒单行（单档后端的物理事实）");
        assert_eq!(list[0].id, "latest");
        assert_eq!(list[0].generation, None);
        assert_eq!(list[0].seq, 2, "seq = events 非空行数");
        assert!(list[0].latest);
        assert_eq!(
            list[0].bytes,
            fs::metadata(dir.join(AUTOSAVE_FILE)).unwrap().len() as usize
        );
        // reset 后 seq 归零重来：单档口径跟着当前代走。
        store.reset(b"{}", "observe-test", TICK).expect("重置");
        let after = save_list(&dir, Backend::Json, 0).unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].seq, 0, "重置后 events 为空文件 → seq 0");
        // 数字 id 在 JSON 局不存在 → None（不伪造历史）。
        assert!(save_get(&dir, Backend::Json, "1").unwrap().is_none());
        let (_, bytes) = save_get(&dir, Backend::Json, "latest").unwrap().unwrap();
        assert_eq!(bytes, store.latest_save().unwrap().unwrap());
    }

    #[test]
    fn event_page_since_limit_json() {
        let (dir, _store) = json_dir("json-page", 4);
        let page = event_page(&dir, Backend::Json, 1, 2).unwrap();
        assert_eq!(page.latest, 4);
        let seqs: Vec<u64> = page.records.iter().map(|record| record.seq).collect();
        assert_eq!(seqs, vec![1, 2]);
        assert_eq!(page.records[0].payload, "{\"i\":1}");
        // 与 sqlite 分页语义等价（records 形状一致，仅 generation/created_at 来源不同）。
        assert_eq!(page.records[0].event, "step");
    }

    #[test]
    fn json_probe_reports_latest_and_mtime() {
        let (dir, _store) = json_dir("json-probe", 1);
        let probe = save_probe(&dir, Backend::Json).unwrap();
        assert_eq!(probe.count, 1);
        assert_eq!(probe.latest_id.as_deref(), Some("latest"));
        assert_eq!(probe.generation, None);
        assert!(probe.latest_created_at > 0, "mtime 可读");
        // 无 autosave → 空探针。
        let empty = case_dir("json-probe-empty");
        assert_eq!(
            save_probe(&empty, Backend::Json).unwrap(),
            SaveProbe::default()
        );
        assert!(save_list(&empty, Backend::Json, 0).unwrap().is_empty());
    }

    #[test]
    fn artifact_views_roundtrip_json() {
        let (dir, mut store) = json_dir("json-artifact", 1);
        let meta = store
            .artifact_put("artifacts/note.md", "正文".as_bytes(), TICK)
            .expect("写制品");
        // JSON 读侧的 updated_at 回 mtime（无 sidecar，写入 tick 不可复得）——
        // ArtifactMeta::updated_at 的既有契约「跨面不强求相等」，故按字段比对。
        let view = artifact_list_view(&dir, Backend::Json).unwrap();
        assert_eq!(view.len(), 1);
        assert_eq!(&view[0].path, &meta.path);
        assert_eq!(view[0].bytes, meta.bytes);
        assert_eq!(view[0].sha256, meta.sha256);
        assert!(view[0].updated_at > 0, "mtime 可读");
        let record = artifact_get_view(&dir, Backend::Json, "artifacts/note.md")
            .unwrap()
            .unwrap();
        assert_eq!(record.content, "正文".as_bytes());
        assert_eq!(record.meta.path, meta.path);
        assert_eq!(record.meta.sha256, meta.sha256);
    }

    /// `--no-default-features` 构建（无 sqlite）下，观察面必须 fail-closed 且零写入。
    #[cfg(not(feature = "sqlite"))]
    #[test]
    fn sqlite_views_unavailable_without_feature() {
        let dir = case_dir("unavailable");
        let err = save_probe(&dir, Backend::Sqlite).unwrap_err();
        assert!(
            matches!(err, crate::StoreError::Unavailable { .. }),
            "{err}"
        );
    }

    /// `commit_steps` 的输入泄漏手法（`Box::leak`）只为测试内静态化 payload/save；
    /// 这里断言它没有把行为搞错：步数与 seq 对齐。
    #[test]
    fn commit_helper_counts_match() {
        let (dir, store) = json_dir("helper", 3);
        assert_eq!(store.next_seq().unwrap(), 3);
        assert_eq!(event_page(&dir, Backend::Json, 0, 10).unwrap().latest, 3);
    }
}
