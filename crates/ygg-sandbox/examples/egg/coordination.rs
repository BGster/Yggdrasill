use super::*;

// ============================================================================
// §协调协议 run 目录内的跨进程「写入 ↔ 收口」互斥（std-only，无新依赖）
//
// 要解决的问题不是「再读一次文件」，而是两个进程对「本局还能不能收增量」的**共识**：
// `--requirement` 是独立进程，本局最后一次 drain 之后、进程退出之前的写入会
// 「成功落盘但永远不被消费」。只靠新增检查点只能降低概率。
//
// 三个文件（全部在 run 目录）：
//   coordination.lock —— 互斥锁（create_new；陈旧按 mtime 抢占，崩溃不永久卡死）
//   coordination.json —— 唯一事实源：{phase, writers, owner_pid, heartbeat_at}
//   inbox.jsonl / inbox.cursor —— 载荷与消费位移（语义不变）
//
// 写命令（--requirement / --proposal / --amend / …）：
//   ① 加锁读 phase：running → 登记 writers[token]=now；
//      closing（心跳未过期）→ **非零拒绝**；finalized → **非零拒绝（开新局）**；
//      closing 但心跳过期 → 判定 owner 已崩，恢复 running 后继续。
//   ② 追加 JSONL 行（**token 仍然持有**，所以收口侧的「等 writers 归零」
//      与「最后 drain」之间不会插进写者）。
//   ③ 加锁摘除自己的 token。
//
// 收口侧（自动推进到结局后的 settle / 独立 --settle）：
//   ① phase=running → closing（记 owner_pid + 心跳）；finalized → 直接拒绝。
//   ② 轮等 writers 归零（在途写者只是一行追加的时长）。
//   ③ **在 closing 保护下**做最后一次 drain：任何成功写入都 happens-before 它。
//      · 有新需求且未 done → phase=running（解除 closing）继续 loop；
//      · 无新需求 → 同一保护下完成 review.note + settle，然后 phase=finalized。
//
// 崩溃恢复：closing 用心跳续命（CLOSING_STALE_MS）；进程崩了以后，后续写命令或
// resume 看到心跳过期的 closing 即判死并恢复 running（已 done 则保留 closing 供结算）。
// 锁也有 mtime 陈旧抢占。两者都不依赖固定 sleep 去「降低概率」。
// ============================================================================
pub(crate) const COORDINATION_FILE: &str = "coordination.json";
pub(crate) const COORDINATION_LOCK: &str = "coordination.lock";
pub(crate) const PHASE_RUNNING: &str = "running";
pub(crate) const PHASE_CLOSING: &str = "closing";
pub(crate) const PHASE_FINALIZED: &str = "finalized";
/// 锁文件 mtime 超过此值即视为崩溃残留（可抢占）。
pub(crate) const LOCK_STALE_MS: u128 = 2_000;
pub(crate) const LOCK_RETRY_MS: u64 = 2;
pub(crate) const LOCK_MAX_TRIES: usize = 1_500;
/// closing 心跳超过此值即视为 owner 已崩溃（可恢复为 running）。
pub(crate) const CLOSING_STALE_MS: u128 = 10_000;
/// 收口侧等待在途写入者的上限（写者只在追加一行期间持有 token）。
pub(crate) const CLOSING_WAIT_MS: u128 = 5_000;

pub(crate) fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis())
}

pub(crate) fn default_coordination() -> Json {
    json!({"phase": PHASE_RUNNING, "writers": {}, "owner_pid": 0, "heartbeat_at": 0})
}

pub(crate) fn read_coordination(run_dir: &Path) -> Json {
    read_json(&run_dir.join(COORDINATION_FILE)).unwrap_or_else(default_coordination)
}

/// 原子写（tmp + rename）：不允许出现半截 coordination.json。
pub(crate) fn write_coordination(run_dir: &Path, value: &Json) -> Result<(), String> {
    let path = run_dir.join(COORDINATION_FILE);
    let tmp = run_dir.join(format!("{COORDINATION_FILE}.tmp-{}", std::process::id()));
    let text =
        serde_json::to_string(value).map_err(|err| format!("序列化 coordination 失败：{err}"))?;
    fs::write(&tmp, text).map_err(|err| format!("写入 {} 失败：{err}", tmp.display()))?;
    fs::rename(&tmp, &path).map_err(|err| format!("替换 {} 失败：{err}", path.display()))
}

pub(crate) struct DirLock {
    pub(crate) path: PathBuf,
    /// true = 同进程重入占位（不持真实锁文件：Drop 不删文件、不重置 depth）。
    pub(crate) reentrant: bool,
}

impl Drop for DirLock {
    fn drop(&mut self) {
        if self.reentrant {
            return;
        }
        RUN_LOCK_DEPTH.with(|depth| depth.set(0));
        RUN_LOCK_FILE.with(|slot| *slot.borrow_mut() = None);
        lock_trace("released");
        let _ = fs::remove_file(&self.path);
    }
}

/// 取协调锁（create_new）：不阻塞进程崩溃后永久卡死——陈旧锁按 mtime 抢占。
pub(crate) fn acquire_dir_lock(run_dir: &Path) -> Result<DirLock, String> {
    let path = run_dir.join(COORDINATION_LOCK);
    for _ in 0..LOCK_MAX_TRIES {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                let _ = writeln!(file, "{}", std::process::id());
                return Ok(DirLock {
                    path,
                    reentrant: false,
                });
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                if let Ok(meta) = fs::metadata(&path) {
                    let age = meta
                        .modified()
                        .ok()
                        .and_then(|time| SystemTime::now().duration_since(time).ok())
                        .map_or(u128::MAX, |elapsed| elapsed.as_millis());
                    if age > LOCK_STALE_MS {
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                }
                std::thread::sleep(Duration::from_millis(LOCK_RETRY_MS));
            }
            Err(err) => return Err(format!("创建 {} 失败：{err}", path.display())),
        }
    }
    Err(format!("协调锁争用超时（{}）", path.display()))
}

thread_local! {
    /// 进程内锁重入记账（P0-1 / C2）：1 = 本进程已持 `coordination.lock`，重入
    /// 不再 acquire。放在**公共锁入口**而不是 Session 上：coordination_update
    /// 等协调协议路径不经过 Session 也要取同一把锁，C3「单锁单记账」必须覆盖
    /// 全部取锁点。单线程模型（C4：临界区内不得 spawn 线程取锁），thread_local 安全。
    pub(crate) static RUN_LOCK_DEPTH: Cell<u32> = const { Cell::new(0) };
    /// 真实持锁期间的锁文件路径（心跳续命用）；重入占位与未持锁时为空。
    /// Drop（真实持有者）归零 depth 时同步清空，心跳不得越权触碰别人的锁。
    pub(crate) static RUN_LOCK_FILE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// 调试追踪（仅 debug 构建；`YGG_LOCK_TRACE=1` 时向 stderr 打
/// `acquired / reentered / released` + depth + pid，供 P0-1 重入取证）。
#[cfg(debug_assertions)]
pub(crate) fn lock_trace(stage: &str) {
    if std::env::var_os("YGG_LOCK_TRACE").is_some_and(|value| value == "1") {
        eprintln!(
            "[lock-trace] {stage} depth={} pid={}",
            RUN_LOCK_DEPTH.with(Cell::get),
            std::process::id()
        );
    }
}
#[cfg(not(debug_assertions))]
pub(crate) fn lock_trace(_stage: &str) {}

/// 重入安全的锁入口（P0-1 持久化面唯一临界区入口）：depth>0 = 本进程已持锁，
/// 直接返回重入占位（不 acquire、不加 depth）；只有真实 acquire 走
/// `acquire_dir_lock`（现有语义不变：create_new + mtime 陈旧抢占）并置 depth=1。
pub(crate) fn acquire_run_lock(run_dir: &Path) -> Result<DirLock, String> {
    if RUN_LOCK_DEPTH.with(Cell::get) > 0 {
        lock_trace("reentered");
        return Ok(DirLock {
            path: run_dir.join(COORDINATION_LOCK),
            reentrant: true,
        });
    }
    let dir_lock = acquire_dir_lock(run_dir)?;
    RUN_LOCK_DEPTH.with(|depth| depth.set(1));
    RUN_LOCK_FILE.with(|slot| *slot.borrow_mut() = Some(dir_lock.path.clone()));
    lock_trace("acquired");
    debug_assert_eq!(RUN_LOCK_DEPTH.with(Cell::get), 1);
    Ok(dir_lock)
}

/// 长临界区心跳：bump 锁文件 mtime，防活进程的合法长临界区被按「陈旧残留」
/// 抢占。陈旧抢占判据是 mtime（`LOCK_STALE_MS`），而 mtime 只在 acquire 时写
/// 一次——临界区一旦超过 2s，等待者就会删锁重建、与持有者双持锁（实测：
/// 4000 行 inbox drain ≈3.1s，等待者在 2.2s 即进锁，单轮并发出现数百重复
/// seq）。drain 等无界循环必须逐行续命；其余临界区（seq+事件+存档 /
/// coordination 读写）亚秒级，不需要心跳。只在本进程真实持锁时生效：
/// 未持锁不动任何文件（绝不 bump 别人的锁），重入时续命的是外层真实锁。
pub(crate) fn run_lock_heartbeat() {
    if RUN_LOCK_DEPTH.with(Cell::get) == 0 {
        return;
    }
    RUN_LOCK_FILE.with(|slot| {
        if let Some(path) = slot.borrow().as_ref() {
            if let Ok(file) = fs::OpenOptions::new().write(true).open(path) {
                let _ = file.set_modified(SystemTime::now());
            }
        }
    });
}

/// 锁内读—改—写 coordination.json（短临界区，不跨文件 IO）。
pub(crate) fn coordination_update<T>(
    run_dir: &Path,
    f: impl FnOnce(&mut Json) -> T,
) -> Result<T, String> {
    let _guard = acquire_run_lock(run_dir)?;
    let mut record = read_coordination(run_dir);
    let out = f(&mut record);
    record["updated_at"] = json!(now_secs());
    write_coordination(run_dir, &record)?;
    Ok(out)
}

/// opener 冷启动认领（§r3：本局首条**携带身份**的成功写先到先得、不可变）。
///
/// `run.json` 增可选字段 `opener_actor` = `{actor, session?, at}`：读档发现已
/// 存在 → `false`（连 `at` 都不刷新——开局时刻是事实，同 id 重复 claim 也是
/// no-op）；缺失 → tmp+rename 原子写入并返回 `true`。锁可重入：inbox 路径已
/// 持 token、dispatch 路径在持久化临界区内，同进程重入直接成功，跨进程由
/// `coordination.lock` 串行——「不同 actor 竞争首个 claim」严格先到先得，
/// 后来者不报错不拒绝（多 subagent 各带身份写命令是常态）。
///
/// 无身份（actor 空）的调用方**根本不进**本函数：失败探针 / 被拒派发不认领，
/// 开局者只可能是一条真实发生且带身份的成功写。
pub(crate) fn claim_opener(
    run_dir: &Path,
    actor: &str,
    session: Option<&str>,
) -> Result<bool, String> {
    let _guard = acquire_run_lock(run_dir)?;
    let meta_path = run_dir.join(META_FILE);
    let mut meta = read_meta(&meta_path).unwrap_or_else(|| json!({}));
    if meta["opener_actor"]["actor"].as_str().is_some() {
        return Ok(false); // 先到先得、不可变：已有开局者，包括同 id 重复 claim
    }
    let mut opener = json!({ "actor": actor, "at": now_secs() });
    if let Some(session_id) = session {
        opener["session"] = json!(session_id);
    }
    meta["opener_actor"] = opener;
    let mut text = serde_json::to_string_pretty(&meta)
        .map_err(|err| format!("序列化 {} 失败：{err}", meta_path.display()))?;
    text.push('\n');
    atomic_write(&meta_path, text.as_bytes())
        .map_err(|err| format!("写 opener_actor 失败：{err}"))?;
    Ok(true)
}

/// 读本局已认领的 opener（run.json 是元信息面：读不到 = 尚无开局者）。
pub(crate) fn read_opener(run_dir: &Path) -> Option<String> {
    read_meta(&run_dir.join(META_FILE))?["opener_actor"]["actor"]
        .as_str()
        .map(str::to_owned)
}

/// 写命令的准入结果。
#[derive(Debug)]
pub(crate) enum WriteGate {
    /// 本局正在收口（owner 心跳未过期）。
    Closing,
    /// 本局已 finalized（settlement 已定稿）。
    Finalized,
    /// 锁 / IO 故障。
    Internal(String),
}

/// 写前登记：成功返回 writer token（落盘完成后必须 `end_write` 摘除）。
pub(crate) fn begin_write(run_dir: &Path) -> Result<String, WriteGate> {
    let token = format!("{}-{}", std::process::id(), now_ms());
    let token_for_record = token.clone();
    let outcome = coordination_update(run_dir, move |record| {
        let phase = record["phase"].as_str().unwrap_or(PHASE_RUNNING).to_owned();
        if phase == PHASE_FINALIZED {
            return Err(WriteGate::Finalized);
        }
        if phase == PHASE_CLOSING {
            let beat = record["heartbeat_at"]
                .as_u64()
                .map_or(0, |value| value as u128);
            if now_ms().saturating_sub(beat) > CLOSING_STALE_MS {
                // 崩溃恢复：owner 已死，收回 closing。
                record["phase"] = json!(PHASE_RUNNING);
                record["owner_pid"] = json!(0);
            } else {
                return Err(WriteGate::Closing);
            }
        }
        let stamp = json!(now_ms() as u64);
        match record["writers"].as_object_mut() {
            Some(map) => {
                map.insert(token_for_record, stamp);
            }
            None => {
                let mut map = serde_json::Map::new();
                map.insert(token_for_record, stamp);
                record["writers"] = Json::Object(map);
            }
        }
        Ok(())
    });
    match outcome {
        Ok(Ok(())) => Ok(token),
        Ok(Err(gate)) => Err(gate),
        Err(message) => Err(WriteGate::Internal(message)),
    }
}

pub(crate) fn end_write(run_dir: &Path, token: &str) {
    let _ = coordination_update(run_dir, |record| {
        if let Some(map) = record["writers"].as_object_mut() {
            map.remove(token);
        }
    });
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ClosingEntry {
    /// 已进入 closing（持锁方负责后续 settle + finalize）。
    Entered,
    /// 本局已 finalized：不再收新写入，也不重写报告。
    AlreadyFinalized,
    /// 已由本进程持有（幂等）。
    Owned,
}

/// 进入 closing 并等所有在途写入者退出（它们的行已落盘 → 后续 drain 必能看到）。
pub(crate) fn enter_closing(run_dir: &Path) -> Result<ClosingEntry, String> {
    let pid = std::process::id();
    let entry = coordination_update(run_dir, |record| {
        let phase = record["phase"].as_str().unwrap_or(PHASE_RUNNING).to_owned();
        if phase == PHASE_FINALIZED {
            return ClosingEntry::AlreadyFinalized;
        }
        if phase == PHASE_CLOSING && record["owner_pid"].as_u64() == Some(pid as u64) {
            record["heartbeat_at"] = json!(now_ms() as u64);
            return ClosingEntry::Owned;
        }
        record["phase"] = json!(PHASE_CLOSING);
        record["owner_pid"] = json!(pid);
        record["heartbeat_at"] = json!(now_ms() as u64);
        ClosingEntry::Entered
    })?;
    if entry == ClosingEntry::AlreadyFinalized {
        return Ok(entry);
    }
    // 等在途写者归零：它们是「已经登记但还没写盘」的 append，必须等它们落盘。
    let deadline = now_ms() + CLOSING_WAIT_MS;
    loop {
        let pending = read_coordination(run_dir)["writers"]
            .as_object()
            .map_or(0, serde_json::Map::len);
        if pending == 0 {
            return Ok(entry);
        }
        if now_ms() > deadline {
            // 崩溃的写者会把 token 留着：清掉心跳过期的，再等最后一下。
            coordination_update(run_dir, |record| {
                let stale_before = now_ms().saturating_sub(CLOSING_STALE_MS) as u64;
                if let Some(map) = record["writers"].as_object_mut() {
                    map.retain(|_, started| started.as_u64().unwrap_or(0) > stale_before);
                }
                record["heartbeat_at"] = json!(now_ms() as u64);
            })?;
            let left = read_coordination(run_dir)["writers"]
                .as_object()
                .map_or(0, serde_json::Map::len);
            if left > 0 {
                return Err(format!("收口等待写入者超时（{left} 个在途）"));
            }
            return Ok(entry);
        }
        std::thread::sleep(Duration::from_millis(LOCK_RETRY_MS));
    }
}

pub(crate) fn closing_heartbeat(run_dir: &Path) {
    let _ = coordination_update(run_dir, |record| {
        record["heartbeat_at"] = json!(now_ms() as u64);
    });
}

/// 解除 closing（回到 running）：drain 到新需求、还没 done 时用。
pub(crate) fn leave_closing(run_dir: &Path) {
    let _ = coordination_update(run_dir, |record| {
        if record["phase"].as_str() != Some(PHASE_FINALIZED) {
            record["phase"] = json!(PHASE_RUNNING);
            record["owner_pid"] = json!(0);
        }
    });
}

/// 收口完成：此后所有写命令一律非零拒绝（需开新局）。
pub(crate) fn finalize_run(run_dir: &Path) {
    let _ = coordination_update(run_dir, |record| {
        record["phase"] = json!(PHASE_FINALIZED);
        record["heartbeat_at"] = json!(now_ms() as u64);
    });
}

/// 崩溃恢复：resume 时若发现「心跳过期的 closing」且本局未 done，恢复 running。
pub(crate) fn recover_stale_closing(run_dir: &Path, ended: bool) -> Option<String> {
    if ended {
        return None;
    }
    let record = read_coordination(run_dir);
    if record["phase"].as_str() != Some(PHASE_CLOSING) {
        return None;
    }
    let beat = record["heartbeat_at"]
        .as_u64()
        .map_or(0, |value| value as u128);
    if now_ms().saturating_sub(beat) <= CLOSING_STALE_MS {
        return None;
    }
    leave_closing(run_dir);
    Some("检测到陈旧的 closing（owner 已退出）——已恢复为 running 继续可写".to_owned())
}

/// 外部幂等 id：`inbox-<行号>-<纳秒>-<pid>`。
/// 行号保证顺序追加唯一，纳秒 + pid 做并发编写的兵底——两者只用于去重，
/// 不参与任何演示 / 状态判定。
pub(crate) fn new_external_id(run_id: &str, line_index: usize) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let prefix: String = run_id.chars().take(24).collect();
    format!("inbox-{prefix}-{line_index}-{nanos}-{}", std::process::id())
}
