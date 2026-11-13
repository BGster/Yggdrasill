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
    ArtifactMeta, ArtifactRecord, Backend, Committed, ResetInfo, RunStore, StepDraft, StepLog,
    StepRecord, StoreError, Tick,
};

/// 存档文件名（既有产物名，被外部工具与回执引用）。
pub const AUTOSAVE_FILE: &str = "autosave.json";
/// 事件日志文件名。
pub const EVENTS_FILE: &str = "events.jsonl";
/// inbox 队列文件名。
pub const INBOX_FILE: &str = "inbox.jsonl";
/// inbox 消费位移文件名。
pub const INBOX_CURSOR_FILE: &str = "inbox.cursor";
/// 制品目录（run 内相对路径的落点根；`artifacts/x.md` → `<run_dir>/artifacts/x.md`）。
pub const ARTIFACTS_DIR: &str = "artifacts";

/// 归档前缀（`events.jsonl.reset.<纳秒>`）。
const ARCHIVE_PREFIX: &str = "events.jsonl.reset.";
/// 制品目录的归档前缀（`artifacts.reset.<纳秒>`）——与 events 同一姿态：
/// **归档不删**（旧代制品是证据，永远不会被自动回收）。
const ARTIFACTS_ARCHIVE_PREFIX: &str = "artifacts.reset.";
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
    /// 制品落点根（`<run_dir>/artifacts`；**只记路径**，不建目录）。
    artifacts: PathBuf,
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
            artifacts: run_dir.join(ARTIFACTS_DIR),
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

    fn savepoints(&self) -> Result<Vec<crate::SavepointInfo>, StoreError> {
        Err(StoreError::Misconfigured {
            detail: "JSON 后端无代（generation）概念：reset 归档旧 autosave 后不可恢复，\
                     存档点回退（--at）仅 sqlite 局可用"
                .to_owned(),
        })
    }

    fn save_at(&self, _generation: i64) -> Result<Option<Vec<u8>>, StoreError> {
        Err(StoreError::Misconfigured {
            detail: "JSON 后端不支持按代取档（--at 仅 sqlite 局可用）".to_owned(),
        })
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
        // 制品与事件同代：rename 归档（**不删**）。失败 = fail-closed
        //（旧代证据必须留得住，与 events 归档同一纪律）。
        if self.artifacts.exists() {
            let archived = self
                .run_dir
                .join(format!("{ARTIFACTS_ARCHIVE_PREFIX}{}", tick.nanos));
            fs::rename(&self.artifacts, &archived)
                .map_err(|err| StoreError::io("归档制品目录失败", err))?;
            let archived_count = fs::read_dir(&self.run_dir).map_or(0, |entries| {
                entries
                    .flatten()
                    .filter(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with(ARTIFACTS_ARCHIVE_PREFIX)
                    })
                    .count()
            });
            if archived_count > ARCHIVE_HINT {
                warnings.push(format!(
                    "制品归档已有 {archived_count} 份，建议人工清理（宿主不自动删证据）"
                ));
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
        // 制品面同纪律：只下潜 `artifacts/`（不碰 run 根的其他树），只清
        // `*.tmp.*`（原子写的残留）。
        sweep_artifact_tmp(&self.artifacts, stale_ms, &mut warnings);
        warnings
    }

    fn artifact_put(
        &mut self,
        path: &str,
        content: &[u8],
        tick: Tick,
    ) -> Result<ArtifactMeta, StoreError> {
        crate::validate_artifact_path(path)?;
        crate::validate_artifact_content(content)?;
        // `path` 已由门保证以 `artifacts/` 起头且无 `..` → join 不会逃出 run 目录。
        let target = self.run_dir.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| StoreError::io(format!("创建 {} 失败", parent.display()), err))?;
        }
        // 复用存档面的原子写（tmp + rename）：半写的制品等同于不存在，但
        // 半个文件会让 artifact_list 报出错误的 bytes。
        self.write_compact(&target, content)?;
        Ok(ArtifactMeta {
            path: path.to_owned(),
            bytes: content.len(),
            sha256: crate::sha256_hex(content),
            updated_at: tick.secs,
        })
    }

    fn artifact_get(&self, path: &str) -> Result<Option<ArtifactRecord>, StoreError> {
        crate::validate_artifact_path(path)?;
        let target = self.run_dir.join(path);
        match fs::read(&target) {
            Ok(content) => Ok(Some(ArtifactRecord {
                meta: ArtifactMeta {
                    path: path.to_owned(),
                    bytes: content.len(),
                    // 无 sidecar：摘要**现算**（JSON 后端的代价：读一次算一次）。
                    sha256: crate::sha256_hex(&content),
                    updated_at: mtime_secs(&target),
                },
                content,
            })),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(StoreError::io(
                format!("读取 {} 失败", target.display()),
                err,
            )),
        }
    }

    fn artifact_list(&self) -> Result<Vec<ArtifactMeta>, StoreError> {
        let mut found = Vec::new();
        collect_artifacts(&self.artifacts, ARTIFACTS_DIR, &mut found);
        // 确定性序（与 sqlite 的 `ORDER BY path` 对齐）：不靠文件系统遍历序。
        found.sort_by(|a, b| a.0.cmp(&b.0));
        let mut out = Vec::with_capacity(found.len());
        for (path, target) in found {
            let content = fs::read(&target)
                .map_err(|err| StoreError::io(format!("读取 {} 失败", target.display()), err))?;
            out.push(ArtifactMeta {
                path,
                bytes: content.len(),
                sha256: crate::sha256_hex(&content),
                updated_at: mtime_secs(&target),
            });
        }
        Ok(out)
    }
}

/// 递归收集 `artifacts/` 下的文件，`path` 用 `/` 分隔（Windows 也必须是 `/`：
/// 这是 wire 与 sqlite 共用的键形态）。目录不存在 → 空（新局没有制品是常态）。
fn collect_artifacts(dir: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => {
                collect_artifacts(&path, &format!("{prefix}/{name}"), out);
            }
            Ok(kind) if kind.is_file() => {
                // 原子写的 tmp 永远不是制品（即使它还未被 rename）。
                if name.contains(".tmp.") {
                    continue;
                }
                out.push((format!("{prefix}/{name}"), path));
            }
            _ => {}
        }
    }
}

/// 递归清 `artifacts/` 下的陈旧 `*.tmp.*`（mtime 读不到就保守跳过）。
fn sweep_artifact_tmp(dir: &Path, stale_ms: u128, warnings: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            sweep_artifact_tmp(&entry.path(), stale_ms, warnings);
            continue;
        }
        if !entry.file_name().to_string_lossy().contains(".tmp.") {
            continue;
        }
        if !is_stale(&entry, stale_ms) {
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
}

/// mtime 超过 `stale_ms` ⇒ 可清理（读不到 mtime = 无法证明陈旧 → `false`）。
fn is_stale(entry: &fs::DirEntry, stale_ms: u128) -> bool {
    entry
        .metadata()
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|time| SystemTime::now().duration_since(time).ok())
        .is_some_and(|elapsed| elapsed.as_millis() > stale_ms)
}

/// 文件 mtime → unix 秒（读不到 → `0`）。
///
/// **不**在这里取当前时间兵底：端口不取时钟（`lib.rs` 硬约束 2）。读不到 mtime
/// 就写 `0`，让调用方看得出「时间未知」而不是看到一个伪造的现时。
fn mtime_secs(path: &Path) -> u64 {
    fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |elapsed| elapsed.as_secs())
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

// ============================================================================
// §crate 内单测：JSON 后端的制品面（目录树 / reset 归档 / tmp 清理）
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const TICK: Tick = Tick {
        secs: 1_700_000_000,
        nanos: 42,
    };

    fn case_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ygg-store-json-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("建临时 run 目录");
        dir
    }

    /// JSON 后端无代概念：savepoints / save_at 必须明确拒绝（不是静默空清单）。
    #[test]
    fn savepoints_refused_on_json_backend() {
        let dir = case_dir("savepoints-refused");
        let store = JsonFileStore::open(&dir).expect("打开");
        let err = store.savepoints().expect_err("JSON 不支持存档点清单");
        assert!(err.to_string().contains("sqlite"), "文案须指向 sqlite: {err}");
        let err = store.save_at(1).expect_err("JSON 不支持按代取档");
        assert!(err.to_string().contains("sqlite"), "文案须指向 sqlite: {err}");
    }

    /// put → get → list：`artifacts/` 目录树按 path 升序，字节保真。
    #[test]
    fn artifact_roundtrip_put_get_list() {
        let dir = case_dir("roundtrip");
        let mut store = JsonFileStore::open(&dir).expect("打开");
        for (path, content) in [
            ("artifacts/b.md", b"second".to_vec()),
            ("artifacts/a.md", b"first".to_vec()),
            ("artifacts/notes/z.md", b"\xe4\xb8\xad\xe6\x96\x87".to_vec()),
        ] {
            let meta = store.artifact_put(path, &content, TICK).expect("写入");
            assert_eq!(meta.path, path);
            assert_eq!(meta.bytes, content.len());
            assert_eq!(meta.sha256, crate::sha256_hex(&content));
            assert_eq!(meta.updated_at, TICK.secs);
        }
        // 真的落在 `<run_dir>/artifacts/...`（不是拼在文件名里）。
        assert!(dir.join("artifacts/a.md").is_file());
        assert!(dir.join("artifacts/notes/z.md").is_file());

        let got = store
            .artifact_get("artifacts/a.md")
            .expect("读")
            .expect("存在");
        assert_eq!(got.content, b"first".to_vec());
        assert_eq!(got.meta.sha256, crate::sha256_hex(b"first"));

        let list = store.artifact_list().expect("列表");
        assert_eq!(
            list.iter().map(|m| m.path.as_str()).collect::<Vec<_>>(),
            vec!["artifacts/a.md", "artifacts/b.md", "artifacts/notes/z.md"],
            "列表必须按 path 升序（与 sqlite 的 ORDER BY path 对齐）"
        );
        assert!(store
            .artifact_get("artifacts/missing.md")
            .expect("读缺失")
            .is_none());
    }

    /// 目录不存在时列表为空集（新局没有制品是常态，不是错误）。
    #[test]
    fn artifact_list_of_missing_dir_is_empty() {
        let dir = case_dir("empty");
        let store = JsonFileStore::open(&dir).expect("打开");
        assert!(!dir.join(ARTIFACTS_DIR).exists());
        assert!(store.artifact_list().expect("列表").is_empty());
    }

    /// 覆盖写：内容变 → sha / bytes 变，但目录里仍然只有一件。
    #[test]
    fn artifact_put_overwrites_and_reshapes_sha() {
        let dir = case_dir("overwrite");
        let mut store = JsonFileStore::open(&dir).expect("打开");
        let first = store
            .artifact_put("artifacts/x.md", b"one", TICK)
            .expect("首次");
        let second = store
            .artifact_put("artifacts/x.md", b"two-two", TICK)
            .expect("覆盖");
        assert_ne!(first.sha256, second.sha256);
        assert_eq!(second.bytes, 7);
        let list = store.artifact_list().expect("列表");
        assert_eq!(list.len(), 1);
        assert_eq!(
            store
                .artifact_get("artifacts/x.md")
                .expect("读")
                .expect("存在")
                .content,
            b"two-two".to_vec()
        );
    }

    /// 同内容重写：摘要稳定。
    #[test]
    fn artifact_put_keeps_same_meta_when_content_identical() {
        let dir = case_dir("identical");
        let mut store = JsonFileStore::open(&dir).expect("打开");
        let first = store
            .artifact_put("artifacts/s.md", b"payload", TICK)
            .expect("首次");
        let again = store
            .artifact_put("artifacts/s.md", b"payload", TICK)
            .expect("重写");
        assert_eq!(first.sha256, again.sha256);
        assert_eq!(first.bytes, again.bytes);
    }

    /// reset 归档目录（rename 为 `artifacts.reset.<纳秒>`）而不删：旧代证据仍在盘上。
    #[test]
    fn artifact_reset_archives_previous_generation() {
        let dir = case_dir("reset");
        let mut store = JsonFileStore::open(&dir).expect("打开");
        store
            .artifact_put("artifacts/keep.md", b"evidence", TICK)
            .expect("写入");
        let info = store
            .reset(b"{\"initial\":true}", "explicit-new-game", TICK)
            .expect("重置");
        assert!(
            !info.had_old,
            "新目录还没写过事件行（`archive` 是 events 归档串，与制品归档独立）"
        );

        // 当前代看不到旧件。
        assert!(store
            .artifact_get("artifacts/keep.md")
            .expect("读")
            .is_none());
        assert!(store.artifact_list().expect("列表").is_empty());

        // 旧件在归档目录里可找（归档不删）。
        let archived = dir.join(format!("{ARTIFACTS_ARCHIVE_PREFIX}{}", TICK.nanos));
        assert!(archived.is_dir(), "归档目录必须在：{}", archived.display());
        assert_eq!(
            fs::read(archived.join("keep.md")).expect("读归档件"),
            b"evidence".to_vec()
        );

        // 新代可重新写同一路径（与归档不冲突）。
        store
            .artifact_put("artifacts/keep.md", b"new", TICK)
            .expect("新代写");
        assert_eq!(
            store
                .artifact_get("artifacts/keep.md")
                .expect("读")
                .expect("存在")
                .content,
            b"new".to_vec()
        );
    }

    /// 门在端口内：坏路径 / 超限内容 → `Invalid`，零写入。
    #[test]
    fn artifact_put_rejects_before_writing() {
        let dir = case_dir("gate");
        let mut store = JsonFileStore::open(&dir).expect("打开");
        let err = store
            .artifact_put("artifacts/../escape.md", b"x", TICK)
            .expect_err("穿越路径必须被拒");
        assert!(matches!(err, StoreError::Invalid { .. }), "{err}");
        let over = vec![0u8; crate::ARTIFACT_CONTENT_MAX_BYTES + 1];
        let err = store
            .artifact_put("artifacts/big.md", &over, TICK)
            .expect_err("超限内容必须被拒");
        assert!(matches!(err, StoreError::Invalid { .. }), "{err}");
        assert!(!dir.join(ARTIFACTS_DIR).exists(), "零写入（连目录都不建）");
    }

    /// `cleanup_stale` 下潜到 `artifacts/` 清陈旧 tmp，但**不碰**真制品。
    #[test]
    fn cleanup_stale_sweeps_artifact_tmp() {
        let dir = case_dir("sweep");
        let mut store = JsonFileStore::open(&dir).expect("打开");
        store
            .artifact_put("artifacts/keep.md", b"keep", TICK)
            .expect("写入");
        let residue = dir.join("artifacts/keep.md.tmp.999");
        fs::write(&residue, b"half").expect("造残留");
        // 回拨 mtime 造「崩溃残留」（不 sleep：确定性强、不拖慢门禁）。
        fs::OpenOptions::new()
            .write(true)
            .open(&residue)
            .expect("打开残留")
            .set_modified(SystemTime::now() - std::time::Duration::from_secs(60))
            .expect("回拨 mtime");
        let warnings = store.cleanup_stale(10_000);
        assert!(
            warnings.iter().any(|line| line.contains("keep.md.tmp.999")),
            "必须报出清理了哪个文件：{warnings:?}"
        );
        assert!(!residue.exists(), "残留必须被删");
        assert!(dir.join("artifacts/keep.md").is_file(), "真制品不得被动");
    }
}
