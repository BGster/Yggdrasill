//! 可选 sqlite 后端（`--features sqlite`）：同样的状态事实，落 `<run-dir>/run.sqlite`。
//!
//! # 与 JSON 后端的差别（刻意，不是缺陷）
//!
//! | 面 | JSON | sqlite |
//! | --- | --- | --- |
//! | 一次提交 | 事件行 → 存档（两步，可只完成前一步） | 单事务写 `step` + `snapshot`（全或无） |
//! | reset 归档 | `events.jsonl.reset.<纳秒>` 文件 | `generation` 新代 + 旧代仍在库里 |
//! | 残留清理 | 扫 `autosave.json.tmp.*` | **no-op**（`-wal` / `-shm` 是库的活文件） |
//!
//! 耐久度对齐：JSON 的原子写是 `create + write_all + rename`（**无 fsync**），承诺
//! 「进程崩溃安全、掉电不安全」；`WAL + synchronous=NORMAL` 的承诺恰好相同。
//! 因此 NORMAL 是等价取值，**不要**擅自升到 FULL（那是另一个需求）。
//!
//! 并发不变量：宿主锁（`coordination.lock`）→ sqlite 写锁，顺序恒不变。跨进程
//! 写冲突靠 `busy_timeout=5000` 等待，不靠重试循环。

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::Value as Json;

use crate::{
    ArtifactMeta, ArtifactRecord, Backend, Committed, ResetInfo, RunStore, SavepointInfo,
    StepDraft, StepLog, StepRecord, StoreError, Tick,
};

/// 库文件名（建库 / 后端探测与混用检测的识别点）。
///
/// 字面量只在 [`crate::SQLITE_FILE_NAME`]：这里派生而不是重写一份 `"run.sqlite"`，
/// 两处漂移 = `detect()` 探不到自己建的库（静默混代）。
pub const SQLITE_FILE: &str = crate::SQLITE_FILE_NAME;
/// 本构建支持的 schema 版本（写出即钉住；读到大数即 fail-closed）。
pub const SCHEMA_VERSION: u32 = 1;
/// 默认跨进程写等待上限（毫秒）。
pub const DEFAULT_BUSY_TIMEOUT_MS: u64 = 5_000;

const INITIAL_GENERATION_REASON: &str = "initial";

const DDL: &str = "\
CREATE TABLE IF NOT EXISTS meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS generation (
  generation  INTEGER PRIMARY KEY,
  archived_at INTEGER NOT NULL,
  reason      TEXT NOT NULL,
  steps       INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS step (
  generation      INTEGER NOT NULL REFERENCES generation(generation),
  seq             INTEGER NOT NULL,
  event           TEXT    NOT NULL,
  payload         TEXT    NOT NULL,
  ok              INTEGER NOT NULL CHECK (ok IN (0,1)),
  narrative_count INTEGER NOT NULL,
  timestamp       INTEGER NOT NULL,
  ending          INTEGER NOT NULL CHECK (ending IN (0,1)),
  ending_line     TEXT,
  PRIMARY KEY (generation, seq)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS snapshot (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  generation INTEGER NOT NULL,
  seq        INTEGER NOT NULL,
  save       BLOB    NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS snapshot_latest ON snapshot(generation, id);
CREATE TABLE IF NOT EXISTS inbox (
  line_no    INTEGER PRIMARY KEY AUTOINCREMENT,
  kind       TEXT,
  record     TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS inbox_cursor (
  id         INTEGER PRIMARY KEY CHECK (id = 1),
  consumed   INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS artifact (
  generation INTEGER NOT NULL REFERENCES generation(generation),
  path       TEXT    NOT NULL,
  sha256     TEXT    NOT NULL,
  bytes      INTEGER NOT NULL,
  content    BLOB    NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (generation, path)
) WITHOUT ROWID;
";

/// sqlite 后端。
pub struct SqliteStore {
    conn: Connection,
    db_path: PathBuf,
    generation: i64,
}

impl SqliteStore {
    /// 打开 / 初始化 `<run_dir>/run.sqlite`（WAL + NORMAL + busy_timeout 5s）。
    pub fn open(run_dir: &Path, tick: Tick) -> Result<Self, StoreError> {
        Self::open_with_busy_timeout(run_dir, DEFAULT_BUSY_TIMEOUT_MS, tick)
    }

    /// 同 [`SqliteStore::open`]，但可指定写等待上限（注入竞争的测试用）。
    pub fn open_with_busy_timeout(
        run_dir: &Path,
        busy_ms: u64,
        tick: Tick,
    ) -> Result<Self, StoreError> {
        let db_path = run_dir.join(SQLITE_FILE);
        let conn = Connection::open(&db_path).map_err(|err| {
            StoreError::corrupt(format!("打开 {} 失败", db_path.display()), err.to_string())
        })?;
        // journal_mode 会返回一行 → 必须 query_row（execute 会因「有返回结果」报错）。
        let _mode: String = conn
            .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
            .map_err(|err| pragma_error(&db_path, "journal_mode", err))?;
        conn.busy_timeout(Duration::from_millis(busy_ms))
            .map_err(|err| pragma_error(&db_path, "busy_timeout", err))?;
        conn.execute_batch(
            "PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON; PRAGMA wal_autocheckpoint = 1000;",
        )
        .map_err(|err| pragma_error(&db_path, "synchronous/foreign_keys", err))?;
        migrate(&conn, &db_path, tick)?;
        let generation = current_generation(&conn, &db_path)?;
        Ok(Self {
            conn,
            db_path,
            generation,
        })
    }

    /// 库文件路径（诊断面）。
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// 当前代（reset 一次 +1）。
    pub fn generation(&self) -> i64 {
        self.generation
    }

    fn next_seq_in(&self, generation: i64) -> Result<i64, StoreError> {
        self.conn
            .query_row(
                "SELECT COALESCE(MAX(seq) + 1, 0) FROM step WHERE generation = ?1",
                params![generation],
                |row| row.get(0),
            )
            .map_err(|err| db_error(&self.db_path, "读取 seq", err))
    }

    fn insert_save(&self, save: &[u8], seq: i64, created_at: u64) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO snapshot(generation, seq, save, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![self.generation, seq, save, created_at as i64],
            )
            .map_err(|err| db_error(&self.db_path, "写入存档", err))?;
        Ok(())
    }
}

impl RunStore for SqliteStore {
    fn backend(&self) -> Backend {
        Backend::Sqlite
    }

    fn locator(&self) -> String {
        // `<db>#latest`：诚实表达「存档不是文件，是库里的最新快照」。
        format!("{}#latest", self.db_path.display())
    }

    fn next_seq(&self) -> Result<u64, StoreError> {
        Ok(self.next_seq_in(self.generation)?.max(0) as u64)
    }

    fn commit_step(&mut self, step: StepDraft<'_>) -> Result<Committed, StoreError> {
        let generation = self.generation;
        // BEGIN IMMEDIATE：显式取写锁，避免 DEFERRED 升级在并发下退化成死锁。
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|err| db_error(&self.db_path, "开启写事务", err))?;
        let seq: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(seq) + 1, 0) FROM step WHERE generation = ?1",
                params![generation],
                |row| row.get(0),
            )
            .map_err(|err| db_error(&self.db_path, "读取 seq", err))?;
        tx.execute(
            "INSERT INTO step(generation, seq, event, payload, ok, narrative_count, timestamp, \
             ending, ending_line) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                generation,
                seq,
                step.event,
                step.payload,
                i64::from(step.ok),
                step.narrative_count as i64,
                step.timestamp as i64,
                i64::from(step.ending_line.is_some()),
                step.ending_line,
            ],
        )
        .map_err(|err| db_error(&self.db_path, "写入事件记录", err))?;
        // 存档只跟成功步走：与 JSON 后端同一判据（独立表 ⇒ 「有行无档」天然可表达）。
        if step.ok {
            if let Some(save) = step.save {
                tx.execute(
                    "INSERT INTO snapshot(generation, seq, save, created_at) \
                     VALUES (?1, ?2, ?3, ?4)",
                    params![generation, seq, save, step.timestamp as i64],
                )
                .map_err(|err| db_error(&self.db_path, "写入存档", err))?;
            }
        }
        tx.commit()
            .map_err(|err| db_error(&self.db_path, "提交写事务", err))?;
        Ok(Committed {
            seq: seq.max(0) as u64,
            warnings: Vec::new(),
        })
    }

    fn latest_save(&self) -> Result<Option<Vec<u8>>, StoreError> {
        self.conn
            .query_row(
                "SELECT save FROM snapshot WHERE generation = ?1 ORDER BY id DESC LIMIT 1",
                params![self.generation],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()
            .map_err(|err| db_error(&self.db_path, "读取存档", err))
    }

    fn set_save(&mut self, save: &[u8], tick: Tick) -> Result<(), StoreError> {
        // 独立存档（`s` 命令）没有对应事件行：seq 记「当前待提交位」，与后续
        // commit_step 的 seq 相容（updated_at 靠 id 递增定序，不靠 seq）。
        let seq = self.next_seq_in(self.generation)?;
        self.insert_save(save, seq, tick.secs)
    }

    fn savepoints(&self) -> Result<Vec<SavepointInfo>, StoreError> {
        // 以 snapshot 表为事实源：每代至少一份初始存档（reset 事务保证），
        // 无 snapshot 的 generation 行（不应存在）不会凭空出现。
        let mut stmt = self
            .conn
            .prepare(
                "SELECT generation, \
                        (SELECT COUNT(*) FROM step WHERE step.generation = snapshot.generation), \
                        MAX(created_at) \
                 FROM snapshot GROUP BY generation ORDER BY generation",
            )
            .map_err(|err| db_error(&self.db_path, "准备存档点查询", err))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?.max(0) as u64,
                    row.get::<_, i64>(2)?.max(0) as u64,
                ))
            })
            .map_err(|err| db_error(&self.db_path, "读取存档点", err))?;
        let mut out = Vec::new();
        for row in rows {
            let (generation, steps, latest_created_at) =
                row.map_err(|err| db_error(&self.db_path, "读取存档点", err))?;
            out.push(SavepointInfo {
                generation,
                steps,
                latest_created_at,
                current: generation == self.generation,
            });
        }
        Ok(out)
    }

    fn save_at(&self, generation: i64) -> Result<Option<Vec<u8>>, StoreError> {
        self.conn
            .query_row(
                "SELECT save FROM snapshot WHERE generation = ?1 ORDER BY id DESC LIMIT 1",
                params![generation],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()
            .map_err(|err| db_error(&self.db_path, "读取指定代存档", err))
    }

    fn steps(&self) -> Result<StepLog, StoreError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT seq, event, payload, ok, narrative_count, timestamp, ending, ending_line \
                 FROM step WHERE generation = ?1 ORDER BY seq",
            )
            .map_err(|err| db_error(&self.db_path, "准备事件查询", err))?;
        let rows = stmt
            .query_map(params![self.generation], |row| {
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
            .map_err(|err| db_error(&self.db_path, "读取事件记录", err))?;
        let mut log = StepLog::default();
        for record in rows {
            log.records
                .push(record.map_err(|err| db_error(&self.db_path, "读取事件记录", err))?);
        }
        Ok(log)
    }

    fn reset(
        &mut self,
        initial_save: &[u8],
        reason: &str,
        tick: Tick,
    ) -> Result<ResetInfo, StoreError> {
        let generation = self.generation;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|err| db_error(&self.db_path, "开启写事务", err))?;
        let steps: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM step WHERE generation = ?1",
                params![generation],
                |row| row.get(0),
            )
            .map_err(|err| db_error(&self.db_path, "统计旧代步数", err))?;
        // 当前代一步未走过 = 无旧代可归档（新局首开 / 重置后没推进就再重置）。
        // 此时**不**推一代：推了会让审计面凭空多出一代，回执也会报「已归档」
        // ——与 JSON 后端「新目录重置无归档」的可见行为对齐。
        if steps == 0 {
            // seq 用事务内的子查询算（不能调 `next_seq_in`：tx 已可变借 conn）。
            let seq: i64 = tx
                .query_row(
                    "SELECT COALESCE(MAX(seq) + 1, 0) FROM step WHERE generation = ?1",
                    params![generation],
                    |row| row.get(0),
                )
                .map_err(|err| db_error(&self.db_path, "读取 seq", err))?;
            tx.execute(
                "INSERT INTO snapshot(generation, seq, save, created_at) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![generation, seq, initial_save, tick.secs as i64],
            )
            .map_err(|err| db_error(&self.db_path, "写入初始存档", err))?;
            tx.commit()
                .map_err(|err| db_error(&self.db_path, "提交写事务", err))?;
            return Ok(ResetInfo {
                had_old: false,
                archive: None,
                warnings: Vec::new(),
            });
        }
        let next = generation + 1;
        tx.execute(
            "INSERT INTO generation(generation, archived_at, reason, steps) \
             VALUES (?1, ?2, ?3, ?4)",
            params![next, tick.secs as i64, reason, steps],
        )
        .map_err(|err| db_error(&self.db_path, "归档旧代", err))?;
        tx.execute(
            "INSERT INTO snapshot(generation, seq, save, created_at) VALUES (?1, 0, ?2, ?3)",
            params![next, initial_save, tick.secs as i64],
        )
        .map_err(|err| db_error(&self.db_path, "写入初始存档", err))?;
        tx.commit()
            .map_err(|err| db_error(&self.db_path, "提交写事务", err))?;
        self.generation = next;
        Ok(ResetInfo {
            had_old: true,
            // 归档载体是「新代」不是文件：旧代仍在库里可查（证据不丢）。
            archive: Some(format!("generation={next}")),
            warnings: Vec::new(),
        })
    }

    fn inbox_push(&mut self, record: &Json, tick: Tick) -> Result<(), StoreError> {
        // 存原文（紧凑 JSON）而非拆列：坏行也要能存，宿主侧跳过逻辑因此 1:1 复刻。
        self.conn
            .execute(
                "INSERT INTO inbox(kind, record, created_at) VALUES (?1, ?2, ?3)",
                params![
                    record["kind"].as_str(),
                    record.to_string(),
                    tick.secs as i64
                ],
            )
            .map_err(|err| db_error(&self.db_path, "写入 inbox 行", err))?;
        Ok(())
    }

    fn inbox_lines(&self) -> Result<Vec<String>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT record FROM inbox ORDER BY line_no")
            .map_err(|err| db_error(&self.db_path, "准备 inbox 查询", err))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|err| db_error(&self.db_path, "读取 inbox", err))?;
        let mut lines = Vec::new();
        for line in rows {
            lines.push(line.map_err(|err| db_error(&self.db_path, "读取 inbox", err))?);
        }
        Ok(lines)
    }

    fn inbox_len(&self) -> Result<usize, StoreError> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM inbox", [], |row| row.get(0))
            .map_err(|err| db_error(&self.db_path, "统计 inbox", err))?;
        Ok(count.max(0) as usize)
    }

    fn inbox_cursor(&self) -> Result<usize, StoreError> {
        let consumed: Option<i64> = self
            .conn
            .query_row(
                "SELECT consumed FROM inbox_cursor WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| db_error(&self.db_path, "读取 inbox cursor", err))?;
        Ok(consumed.unwrap_or(0).max(0) as usize)
    }

    fn set_inbox_cursor(&mut self, consumed: usize, tick: Tick) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO inbox_cursor(id, consumed, updated_at) VALUES (1, ?1, ?2) \
                 ON CONFLICT(id) DO UPDATE SET consumed = excluded.consumed, \
                 updated_at = excluded.updated_at",
                params![consumed as i64, tick.secs as i64],
            )
            .map_err(|err| db_error(&self.db_path, "写入 inbox cursor", err))?;
        Ok(())
    }

    fn cleanup_stale(&mut self, _stale_ms: u128) -> Vec<String> {
        // 刻意 no-op：`-wal` / `-shm` / `-journal` 是库的活文件，「清理」= 破坏事务。
        Vec::new()
    }

    fn artifact_put(
        &mut self,
        path: &str,
        content: &[u8],
        tick: Tick,
    ) -> Result<ArtifactMeta, StoreError> {
        crate::validate_artifact_path(path)?;
        crate::validate_artifact_content(content)?;
        let sha256 = crate::sha256_hex(content);
        let bytes = content.len();
        self.conn
            .execute(
                "INSERT INTO artifact(generation, path, sha256, bytes, content, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                 ON CONFLICT(generation, path) DO UPDATE SET \
                   sha256 = excluded.sha256, bytes = excluded.bytes, \
                   content = excluded.content, updated_at = excluded.updated_at",
                params![
                    self.generation,
                    path,
                    sha256,
                    bytes as i64,
                    content,
                    tick.secs as i64
                ],
            )
            .map_err(|err| db_error(&self.db_path, "写入制品", err))?;
        Ok(ArtifactMeta {
            path: path.to_owned(),
            bytes,
            sha256,
            updated_at: tick.secs,
        })
    }

    fn artifact_get(&self, path: &str) -> Result<Option<ArtifactRecord>, StoreError> {
        crate::validate_artifact_path(path)?;
        self.conn
            .query_row(
                "SELECT sha256, bytes, content, updated_at FROM artifact \
                 WHERE generation = ?1 AND path = ?2",
                params![self.generation, path],
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
            .map_err(|err| db_error(&self.db_path, "读取制品", err))
    }

    fn artifact_list(&self) -> Result<Vec<ArtifactMeta>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT path, bytes, sha256, updated_at FROM artifact \
                 WHERE generation = ?1 ORDER BY path",
            )
            .map_err(|err| db_error(&self.db_path, "准备制品查询", err))?;
        let rows = stmt
            .query_map(params![self.generation], |row| {
                let path: String = row.get(0)?;
                let bytes: i64 = row.get(1)?;
                Ok(ArtifactMeta {
                    path,
                    bytes: bytes.max(0) as usize,
                    sha256: row.get(2)?,
                    updated_at: row.get::<_, i64>(3)?.max(0) as u64,
                })
            })
            .map_err(|err| db_error(&self.db_path, "读取制品列表", err))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|err| db_error(&self.db_path, "读取制品列表", err))?);
        }
        Ok(out)
    }
}

// ============================================================================
// §schema 与错误
// ============================================================================

/// 建表 / 版本校验。顺序：先让 `meta` 存在（幂等）→ 读版本 → 判定 → 建全表。
fn migrate(conn: &Connection, db_path: &Path, tick: Tick) -> Result<(), StoreError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
    )
    .map_err(|err| db_error(db_path, "初始化 meta 表", err))?;
    let found: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|err| db_error(db_path, "读取 schema_version", err))?;

    match found {
        // 空库：建全表 + 钉版本（一个事务，半建表不留残骸）。
        None => {
            let tx = conn
                .unchecked_transaction()
                .map_err(|err| db_error(db_path, "开启初始化事务", err))?;
            tx.execute_batch(DDL)
                .map_err(|err| db_error(db_path, "初始化 schema", err))?;
            tx.execute(
                "INSERT INTO generation(generation, archived_at, reason, steps) \
                 VALUES (0, ?1, ?2, 0)",
                params![tick.secs as i64, INITIAL_GENERATION_REASON],
            )
            .map_err(|err| db_error(db_path, "初始化 generation", err))?;
            tx.execute_batch(&format!(
                "INSERT INTO meta(key, value) VALUES ('schema_version', '{SCHEMA_VERSION}');\
                 INSERT INTO meta(key, value) VALUES ('backend', 'sqlite');\
                 INSERT INTO meta(key, value) VALUES ('created_at', '{}');",
                tick.secs
            ))
            .map_err(|err| db_error(db_path, "写入 meta", err))?;
            tx.commit()
                .map_err(|err| db_error(db_path, "提交初始化事务", err))?;
            Ok(())
        }
        Some(text) => {
            let version: u32 = text.parse().map_err(|_| {
                StoreError::corrupt(
                    format!("读取 {} 失败", db_path.display()),
                    format!("meta.schema_version 不是整数：`{text}`"),
                )
            })?;
            if version > SCHEMA_VERSION {
                return Err(StoreError::Unsupported {
                    detail: format!(
                        "{} 由更新版本写出（meta.schema_version={version}），本构建只支持 ≤{SCHEMA_VERSION}；\
                         请升级工具或开新局（换 --run-id）——本局未做任何写入。",
                        db_path.display()
                    ),
                });
            }
            // v1 是首个版本，没有迁移步骤；将来 v1→v2 在此按序追加（每步一事务）。
            conn.execute_batch(DDL)
                .map_err(|err| db_error(db_path, "补齐 schema", err))?;
            Ok(())
        }
    }
}

fn current_generation(conn: &Connection, db_path: &Path) -> Result<i64, StoreError> {
    conn.query_row(
        "SELECT COALESCE(MAX(generation), 0) FROM generation",
        [],
        |row| row.get(0),
    )
    .map_err(|err| db_error(db_path, "读取 generation", err))
}

fn db_error(db_path: &Path, what: &str, err: rusqlite::Error) -> StoreError {
    StoreError::corrupt(
        format!("{what}失败（{}）", db_path.display()),
        err.to_string(),
    )
}

fn pragma_error(db_path: &Path, pragma: &str, err: rusqlite::Error) -> StoreError {
    StoreError::corrupt(
        format!("设置 PRAGMA {pragma} 失败（{}）", db_path.display()),
        err.to_string(),
    )
}

#[cfg(test)]
mod tests;
