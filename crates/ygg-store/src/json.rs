//! JSON 文件后端：既有 run 目录产物（`autosave.json` / `events.jsonl` /
//! `inbox.jsonl` / `inbox.cursor`）的**字节级**复刻。
//!
//! 这一层是从宿主原样下沉的，任何「顺手改好一点」都会破坏兼容：
//!
//! | 产物 | 形态 | 禁止 |
//! | --- | --- | --- |
//! | `autosave.json` | 紧凑 JSON、键序字典序、tmp+rename 原子替换、**无尾随换行** | 补 `\n`、pretty 化、开 `preserve_order` |
//! | `events.jsonl` | 每行 `to_string()` + `\n`，单次 `write_all`（不留半行） | 分段写、批量后置 flush |
//! | `inbox.cursor` | pretty JSON + `\n` | 改成紧凑（人读证据面） |
//!
//! 两个写函数刻意分开：`write_compact`（存档，无换行）与 `write_pretty`
//! （cursor，带换行）。**不要合并**。

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::{json, Value as Json};

use crate::{
    Backend, Committed, ResetInfo, RunStore, StepDraft, StepLog, StepRecord, StoreError, Tick,
};

/// 存档文件名（既有产物名，被外部工具与回执引用）。
pub const AUTOSAVE_FILE: &str = "autosave.json";
/// 事件日志文件名。
pub const EVENTS_FILE: &str = "events.jsonl";
/// inbox 队列文件名。
pub const INBOX_FILE: &str = "inbox.jsonl";
/// inbox 消费位移文件名。
pub const INBOX_CURSOR_FILE: &str = "inbox.cursor";

/// 归档前缀（`events.jsonl.reset.<纳秒>`）。
const ARCHIVE_PREFIX: &str = "events.jsonl.reset.";
/// 事件行「超大」告警阈值（异常 payload 的可能信号；只告警不阻断）。
const OVERSIZED_LINE: usize = 16 * 1024;
/// 归档份数提示阈值（超过即提示人工清理；宿主不自动删证据）。
const ARCHIVE_HINT: usize = 5;

/// JSON 文件后端。
pub struct JsonFileStore {
    run_dir: PathBuf,
    autosave: PathBuf,
    events: PathBuf,
    inbox: PathBuf,
    cursor: PathBuf,
}

impl JsonFileStore {
    /// 绑定到一个 run 目录（**只记路径**：建目录是宿主的职责——它可能在裁决前
    /// 就知道不该建）。
    pub fn open(run_dir: &Path) -> Result<Self, StoreError> {
        Ok(Self {
            run_dir: run_dir.to_path_buf(),
            autosave: run_dir.join(AUTOSAVE_FILE),
            events: run_dir.join(EVENTS_FILE),
            inbox: run_dir.join(INBOX_FILE),
            cursor: run_dir.join(INBOX_CURSOR_FILE),
        })
    }

    /// 事件日志路径（回执/诊断面；外部工具按文件名找产物）。
    pub fn events_path(&self) -> &Path {
        &self.events
    }

    /// 存档路径。
    pub fn autosave_path(&self) -> &Path {
        &self.autosave
    }

    /// 紧凑 JSON 落盘（存档面）：tmp + rename 原子替换，**不补换行**。
    ///
    /// tmp 名带 pid 是正确性要求不是风格：固定名会让两个并发写者互踩 tmp 内容，
    /// rename 后拿到「别人的中间态」。
    fn write_compact(&self, path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
        let name = path.file_name().map_or_else(
            || format!(".atomic-{}", std::process::id()),
            |name| format!("{}.tmp.{}", name.to_string_lossy(), std::process::id()),
        );
        let tmp = path.with_file_name(name);
        let mut file = File::create(&tmp)
            .map_err(|err| StoreError::io(format!("写入 {} 失败", tmp.display()), err))?;
        file.write_all(bytes)
            .map_err(|err| StoreError::io(format!("写入 {} 失败", tmp.display()), err))?;
        fs::rename(&tmp, path)
            .map_err(|err| StoreError::io(format!("替换 {} 失败", path.display()), err))
    }

    /// pretty JSON 落盘（人读证据面；末尾补换行便于 diff）。
    fn write_pretty(&self, path: &Path, text: &str) -> Result<(), StoreError> {
        fs::write(path, text)
            .map_err(|err| StoreError::io(format!("写入 {} 失败", path.display()), err))
    }
}

impl RunStore for JsonFileStore {
    fn backend(&self) -> Backend {
        Backend::Json
    }

    fn locator(&self) -> String {
        self.autosave.display().to_string()
    }

    fn next_seq(&self) -> Result<u64, StoreError> {
        // seq 的语义是「非空行数」而非「上一行 seq+1」：末行无换行也计入，
        // 不可解析的行同样占一个号（既有口径，别在这里「修正」）。
        Ok(match fs::read_to_string(&self.events) {
            Ok(text) => text.lines().filter(|line| !line.trim().is_empty()).count() as u64,
            Err(_) => 0,
        })
    }

    fn commit_step(&mut self, step: StepDraft<'_>) -> Result<Committed, StoreError> {
        let seq = self.next_seq()?;
        let mut warnings = Vec::new();

        // 序列化先完成：失败不留半行。单次 write_all 消除 Formatter 的分段写；
        // 跨进程原子性由外层锁保证，write_all 只负责同进程内的一次 write(2)。
        let mut record = json!({
            "seq": seq,
            "event": step.event,
            "payload": step.payload,
            "ok": step.ok,
            "narrative_count": step.narrative_count,
            "timestamp": step.timestamp,
            "ending": step.ending_line.is_some(),
        });
        if let Some(line) = step.ending_line {
            record["ending_line"] = json!(line);
        }
        match serde_json::to_string(&record) {
            Ok(mut line) => {
                if line.len() > OVERSIZED_LINE {
                    warnings.push(format!(
                        "事件行超大（{} 字节，seq {seq}）：异常 payload 的可能信号",
                        line.len()
                    ));
                }
                line.push('\n');
                match OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.events)
                {
                    Ok(mut file) => {
                        if let Err(err) = file.write_all(line.as_bytes()) {
                            warnings.push(format!("写入 {} 失败：{err}", self.events.display()));
                        }
                    }
                    Err(err) => {
                        warnings.push(format!("打开 {} 失败：{err}", self.events.display()))
                    }
                }
            }
            Err(err) => warnings.push(format!("序列化事件记录失败：{err}")),
        }

        // 存档只跟成功步走（失败步不改状态，重写同一份存档无意义）。
        if step.ok {
            if let Some(save) = step.save {
                self.write_compact(&self.autosave, save)?;
            }
        }
        Ok(Committed { seq, warnings })
    }

    fn latest_save(&self) -> Result<Option<Vec<u8>>, StoreError> {
        match fs::read(&self.autosave) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(StoreError::io(
                format!("读取 {} 失败", self.autosave.display()),
                err,
            )),
        }
    }

    fn set_save(&mut self, save: &[u8], _tick: Tick) -> Result<(), StoreError> {
        self.write_compact(&self.autosave, save)
    }

    fn steps(&self) -> Result<StepLog, StoreError> {
        let mut log = StepLog::default();
        let Ok(text) = fs::read_to_string(&self.events) else {
            return Ok(log); // 无日志 = 0 条（不是错误）
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<Json>(line)
                .ok()
                .as_ref()
                .and_then(record_from)
            {
                Some(record) => log.records.push(record),
                None => log.skipped += 1,
            }
        }
        Ok(log)
    }

    fn reset(
        &mut self,
        initial_save: &[u8],
        _reason: &str,
        tick: Tick,
    ) -> Result<ResetInfo, StoreError> {
        let had_old = self.events.exists() || self.autosave.exists();
        let mut warnings = Vec::new();
        let mut archive = None;

        if self.events.exists() {
            let archived = self.run_dir.join(format!("{ARCHIVE_PREFIX}{}", tick.nanos));
            // fail-closed：归档不了就不重置（旧代证据必须留得住）。
            fs::rename(&self.events, &archived)
                .map_err(|err| StoreError::io("归档事件日志失败", err))?;
            archive = Some(archived.display().to_string());
            // 归档堆积提示：只提示不自动清理（自动删证据违反「保留原始证据」）。
            let archived_count = fs::read_dir(&self.run_dir).map_or(0, |entries| {
                entries
                    .flatten()
                    .filter(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with(ARCHIVE_PREFIX)
                    })
                    .count()
            });
            if archived_count > ARCHIVE_HINT {
                warnings.push(format!(
                    "events 归档已有 {archived_count} 份，建议人工清理（宿主不自动删证据）"
                ));
            }
        }
        if self.autosave.exists() {
            // 移除失败只告警：真正的一致性屏障是后面那次原子替换。
            if let Err(err) = fs::remove_file(&self.autosave) {
                warnings.push(format!("移除旧 autosave 失败：{err}"));
            }
        }
        // 显式建空文件：归档后立刻有新一代日志（与旧日志不再混代）。
        // 建档失败必须 fail-closed：events 缺失/旧态时继续新局 = 混代。
        File::create(&self.events).map_err(|err| StoreError::io("新建事件日志失败", err))?;
        // 初始存档同临界区：写不出来还继续 = 让无档新局推进。
        self.write_compact(&self.autosave, initial_save)?;
        Ok(ResetInfo {
            had_old,
            archive,
            warnings,
        })
    }

    fn inbox_push(&mut self, record: &Json, _tick: Tick) -> Result<(), StoreError> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.inbox)
            .map_err(|err| StoreError::io(format!("打开 {} 失败", self.inbox.display()), err))?;
        // `{record}` = `Value` 的 Display = 紧凑 JSON：与历史行逐字节同形。
        writeln!(file, "{record}")
            .map_err(|err| StoreError::io(format!("写入 {} 失败", self.inbox.display()), err))
    }

    fn inbox_lines(&self) -> Result<Vec<String>, StoreError> {
        Ok(match fs::read_to_string(&self.inbox) {
            Ok(text) => text
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(str::to_owned)
                .collect(),
            Err(_) => Vec::new(), // 无 inbox 文件 = 本局没有异步需求
        })
    }

    fn inbox_len(&self) -> Result<usize, StoreError> {
        Ok(self.inbox_lines()?.len())
    }

    fn inbox_cursor(&self) -> Result<usize, StoreError> {
        Ok(read_json(&self.cursor)
            .and_then(|cursor| cursor["consumed"].as_u64())
            .map_or(0, |consumed| consumed as usize))
    }

    fn set_inbox_cursor(&mut self, consumed: usize, tick: Tick) -> Result<(), StoreError> {
        let record = json!({"consumed": consumed, "updated_at": tick.secs});
        let mut text = serde_json::to_string_pretty(&record).map_err(|err| {
            StoreError::corrupt(format!("序列化 {}", self.cursor.display()), err.to_string())
        })?;
        text.push('\n');
        self.write_pretty(&self.cursor, &text)
    }

    fn cleanup_stale(&mut self, stale_ms: u128) -> Vec<String> {
        let mut warnings = Vec::new();
        let prefix = format!("{AUTOSAVE_FILE}.tmp.");
        let entries = match fs::read_dir(&self.run_dir) {
            Ok(entries) => entries,
            Err(_) => return warnings, // 目录读不到 = 无残留可清
        };
        for entry in entries.flatten() {
            if !entry.file_name().to_string_lossy().starts_with(&prefix) {
                continue;
            }
            // mtime 读不到 → 无法证明陈旧 → 保守跳过（宁可残留不可误删）。
            let Some(age) = entry
                .metadata()
                .ok()
                .and_then(|meta| meta.modified().ok())
                .and_then(|time| SystemTime::now().duration_since(time).ok())
                .map(|elapsed| elapsed.as_millis())
            else {
                continue;
            };
            if age <= stale_ms {
                continue;
            }
            match fs::remove_file(entry.path()) {
                Ok(()) => warnings.push(format!("清理残留 tmp：{}", entry.path().display())),
                Err(err) => warnings.push(format!(
                    "清理残留 tmp 失败：{}：{err}",
                    entry.path().display()
                )),
            }
        }
        warnings
    }
}

/// 一行事件记录 → [`StepRecord`]（字段缺失即视为不可解析行）。
fn record_from(value: &Json) -> Option<StepRecord> {
    Some(StepRecord {
        seq: value["seq"].as_u64()?,
        event: value["event"].as_str()?.to_owned(),
        payload: value["payload"].as_str()?.to_owned(),
        ok: value["ok"].as_bool()?,
        narrative_count: value["narrative_count"].as_u64()? as usize,
        timestamp: value["timestamp"].as_u64()?,
        ending: value["ending"].as_bool()?,
        ending_line: value["ending_line"].as_str().map(str::to_owned),
    })
}

/// 读一个 JSON 文件（缺失 / 非法 → `None`；便利面，不吞错不死循环）。
fn read_json(path: &Path) -> Option<Json> {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Json>(&text).ok())
}
