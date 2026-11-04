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
    Backend, Committed, ResetInfo, RunStore, StepDraft, StepLog, StepRecord, StoreError, Tick,
};

/// 库文件名（后端探测与混用检测的识别点）。
pub const SQLITE_FILE: &str = "run.sqlite";
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
            had_old: steps > 0,
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

// ============================================================================
// §crate 内单测：需要直接持有连接才能注入的故障（跨进程锁竞争 / 事务中途失败）
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const TICK: Tick = Tick {
        secs: 1_700_000_000,
        nanos: 7,
    };

    fn case_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ygg-store-sqlite-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("建临时 run 目录");
        dir
    }

    fn draft<'a>(save: Option<&'a [u8]>, ok: bool) -> StepDraft<'a> {
        StepDraft {
            event: "step",
            payload: "0",
            ok,
            narrative_count: 1,
            timestamp: TICK.secs,
            ending_line: None,
            save,
        }
    }

    /// 强承诺之一：事务**中途**失败（step 已插入、snapshot 写不进去）→ 整体回滚。
    ///
    /// 注入手法：把 `snapshot` 表删掉。这正好卡在「事件行已插入、存档尚未插入」的
    /// 那一步——JSON 后端在这种位置会留下「有事件无存档」（见 consistency.rs），
    /// sqlite 必须一条都不留。
    #[test]
    fn commit_rolls_back_when_snapshot_write_fails() {
        let dir = case_dir("rollback");
        let mut store = SqliteStore::open(&dir, TICK).expect("打开库");
        store
            .commit_step(draft(Some(b"{\"revision\":0}"), true))
            .expect("第一个成功步");
        store
            .conn
            .execute_batch("DROP TABLE snapshot;")
            .expect("删表注入故障");

        let err = store
            .commit_step(draft(Some(b"{\"revision\":1}"), true))
            .expect_err("存档写不进去必须硬失败");
        assert!(
            err.to_string().contains("写入存档失败"),
            "错误必须点名: {err}"
        );

        // 全或无：事件行没留下（否则下一轮会把 seq 算错）。
        assert_eq!(store.next_seq().expect("seq"), 1, "失败的提交不得占 seq");
        assert_eq!(store.steps().expect("读记录").records.len(), 1);
    }

    /// 强承诺之二：跨进程写竞争 → `busy_timeout` 到点即失败，且库内零变化。
    ///
    /// 锁序不变量：另一连接持 `BEGIN IMMEDIATE`，本连接必须等超时而不是死锁。
    #[test]
    fn commit_fails_closed_when_another_writer_holds_the_lock() {
        let dir = case_dir("busy");
        let mut store = SqliteStore::open_with_busy_timeout(&dir, 50, TICK).expect("打开库");
        store
            .commit_step(draft(Some(b"{\"revision\":0}"), true))
            .expect("第一个成功步");

        let blocker = Connection::open(dir.join(SQLITE_FILE)).expect("第二个连接");
        blocker
            .execute_batch("BEGIN IMMEDIATE; INSERT INTO meta(key, value) VALUES('probe', '1');")
            .expect("另一进程持写锁");

        let err = store
            .commit_step(draft(Some(b"{\"revision\":1}"), true))
            .expect_err("拿不到写锁必须失败（不能静默丢一步）");
        assert!(
            err.to_string().contains("开启写事务失败"),
            "错误必须点名开启事务: {err}"
        );
        assert_eq!(store.next_seq().expect("seq"), 1, "竞争失败不得占 seq");
        assert_eq!(
            store.latest_save().expect("读存档").expect("有存档"),
            b"{\"revision\":0}".to_vec(),
            "存档必须还是上一版"
        );

        blocker.execute_batch("ROLLBACK;").expect("释放写锁");
        drop(blocker);
        store
            .commit_step(draft(Some(b"{\"revision\":1}"), true))
            .expect("锁释放后必须能继续");
        assert_eq!(store.next_seq().expect("seq"), 2);
    }

    /// reset 的归档是「新代」而不是删行：旧代仍可查（证据不丢）。
    #[test]
    fn reset_keeps_old_generation_queryable() {
        let dir = case_dir("generation");
        let mut store = SqliteStore::open(&dir, TICK).expect("打开库");
        store
            .commit_step(draft(Some(b"{\"revision\":0}"), true))
            .expect("提交");
        let info = store
            .reset(b"{\"initial\":true}", "explicit-new-game", TICK)
            .expect("重置");
        assert_eq!(info.archive.as_deref(), Some("generation=1"));
        assert_eq!(store.generation(), 1);

        // 当前代看不到旧行。
        assert!(store.steps().expect("读记录").records.is_empty());
        // 旧代仍在库里，且台账记下了归档时的步数。
        // 台账语义：`generation` 行描述**创建该代的那次归档**（steps = 被归档代的步数），
        // 所以归档 generation 0 的信息落在 generation 1 那行上。
        let (archived_steps, reason): (i64, String) = store
            .conn
            .query_row(
                "SELECT steps, reason FROM generation WHERE generation = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("查归档台账");
        assert_eq!(archived_steps, 1, "归档台账必须记下旧代步数");
        assert_eq!(reason, "explicit-new-game", "归档原因必须落库（审计面）");
        let initial_steps: i64 = store
            .conn
            .query_row(
                "SELECT steps FROM generation WHERE generation = 0",
                [],
                |row| row.get(0),
            )
            .expect("查 0 代台账");
        assert_eq!(initial_steps, 0, "0 代是初建，没有「被归档的旧代」");
        let old_rows: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM step WHERE generation = 0",
                [],
                |row| row.get(0),
            )
            .expect("查旧代事件");
        assert_eq!(old_rows, 1, "旧代事件行必须还在");

        // 重开库也一样（归档不是内存态）。
        let reopened = SqliteStore::open(&dir, TICK).expect("重开库");
        assert_eq!(reopened.generation(), 1, "当前代必须持久化");
    }

    /// 库由更新版本写出 → fail-closed（不猜、不迁移、不降级）。
    #[test]
    fn newer_schema_version_is_rejected() {
        let dir = case_dir("future");
        let db = dir.join(SQLITE_FILE);
        {
            let conn = Connection::open(&db).expect("建库");
            conn.execute_batch(
                "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);\
                 INSERT INTO meta(key, value) VALUES ('schema_version', '999');",
            )
            .expect("写未来版本号");
        }
        let err = match SqliteStore::open(&dir, TICK) {
            Ok(_) => panic!("未来版本必须拒绝（不得降级/迁移）"),
            Err(err) => err,
        };
        let text = err.to_string();
        for fragment in ["schema_version=999", "未做任何写入"] {
            assert!(text.contains(fragment), "错误串缺少「{fragment}」：{text}");
        }
    }

    /// 空库首次打开：建全表 + 钉版本 + 建 0 代（幂等，重开不重复建）。
    #[test]
    fn fresh_open_initialises_once() {
        let dir = case_dir("init");
        let store = SqliteStore::open(&dir, TICK).expect("首次打开");
        assert_eq!(store.generation(), 0);
        let version: String = store
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .expect("读版本");
        assert_eq!(version, SCHEMA_VERSION.to_string());
        drop(store);
        let reopened = SqliteStore::open(&dir, TICK).expect("重开");
        let generations: i64 = reopened
            .conn
            .query_row("SELECT COUNT(*) FROM generation", [], |row| row.get(0))
            .expect("统计代");
        assert_eq!(generations, 1, "重开不得重复建 0 代");
    }
}
