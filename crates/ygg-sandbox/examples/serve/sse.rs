use super::*;

/// SSE 保活注释间隔（`: ping`）。
pub(crate) const SSE_KEEPALIVE: Duration = Duration::from_secs(15);

/// watcher → SSE 连接的变更广播对（§6.1：通用变更帧，无数据载荷）。
pub(crate) struct Broadcaster {
    /// `(version, 最近一次变更的 run 名, 该局所属根)`；根级变更 = run 为 `None`、
    /// root 点名变化的根（多根下根级帧也带根；单根时代 root 恒 `Some`）。
    state: Mutex<(u64, Option<String>, Option<String>)>,
    cond: Condvar,
}

impl Broadcaster {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new((0, None, None)),
            cond: Condvar::new(),
        }
    }

    pub(crate) fn version(&self) -> u64 {
        self.state.lock().map_or(0, |state| state.0)
    }

    /// 变更登记：version 自增 + 唤醒全部 SSE 连接。
    pub(crate) fn bump(&self, run: Option<&str>, root: Option<&Path>) {
        if let Ok(mut state) = self.state.lock() {
            state.0 += 1;
            state.1 = run.map(str::to_owned);
            state.2 = root.map(|path| path.display().to_string());
            self.cond.notify_all();
        }
    }

    /// 等待 `seen` 之后的新版本；超时返回 `None`（调用方写 `: ping`）。
    pub(crate) fn wait_since(
        &self,
        seen: u64,
        timeout: Duration,
    ) -> Option<(u64, Option<String>, Option<String>)> {
        let guard = self.state.lock().ok()?;
        if guard.0 > seen {
            return Some((guard.0, guard.1.clone(), guard.2.clone()));
        }
        let (guard, _timed_out) = self
            .cond
            .wait_timeout_while(guard, timeout, |state| state.0 <= seen)
            .ok()?;
        let (version, run, root) = &*guard;
        (*version > seen).then(|| (*version, run.clone(), root.clone()))
    }
}

// ============================================================================
// §watcher：纯变更检测器（§6.1——无内核无 Session，轮询 → version 自增 → 广播）
// ============================================================================

/// 单局指纹：run.json / coordination.json 的 stat + 存档探针。
#[derive(Default, PartialEq, Eq)]
pub(crate) struct RunFingerprint {
    run_stat: Option<(u128, u64)>,
    coord_stat: Option<(u128, u64)>,
    probe: observe::SaveProbe,
}

pub(crate) fn stat_stamp(path: &Path) -> Option<(u128, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some((modified, meta.len()))
}

pub(crate) fn fingerprint(run_dir: &Path) -> RunFingerprint {
    RunFingerprint {
        run_stat: stat_stamp(&run_dir.join(crate::META_FILE)),
        coord_stat: stat_stamp(&run_dir.join(crate::COORDINATION_FILE)),
        // 后端裁决失败（混代等）→ 空探针：观察面如实显示空，不掩盖也不炸线程。
        probe: resolve_backend(run_dir)
            .ok()
            .and_then(|backend| observe::save_probe(run_dir, backend).ok())
            .unwrap_or_default(),
    }
}

pub(crate) fn watcher_loop(shared: &ServeShared, shutdown: &AtomicBool, poll_ms: u64) {
    // 指纹键 = (根, 局名) 对：跨根同名局各自独立跟踪；单局根恒一名。
    let mut known: BTreeMap<(PathBuf, String), RunFingerprint> = BTreeMap::new();
    while !shutdown.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(poll_ms));
        for root in shared.roots_snapshot() {
            let mut names: Vec<String> = if root.single_run {
                // 单局根：run.json 在 = 局在；被外部删除 = 名单空（行消失）。
                root.path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .filter(|_| root.path.join(crate::META_FILE).is_file())
                    .map(|name| vec![name.to_owned()])
                    .unwrap_or_default()
            } else {
                std::fs::read_dir(&root.path)
                    .map(|entries| {
                        entries
                            .flatten()
                            .filter(|entry| {
                                entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false)
                            })
                            .filter_map(|entry| entry.file_name().into_string().ok())
                            .collect()
                    })
                    .unwrap_or_default()
            };
            names.sort();
            let previous: Vec<String> = known
                .keys()
                .filter(|(known_root, _)| *known_root == root.path)
                .map(|(_, name)| name.clone())
                .collect();
            if names != previous {
                // 根内名单变化（新局出现 / 旧局被删 / 根消失 / 注销）→ 根级变更帧。
                known.retain(|(known_root, name), _| {
                    *known_root != root.path || names.contains(name)
                });
                shared.broadcaster.bump(None, Some(root.path.as_path()));
            }
            for name in &names {
                let key = (root.path.clone(), name.clone());
                let fp = fingerprint(&root.path.join(name));
                if known.get(&key) != Some(&fp) {
                    known.insert(key, fp);
                    shared
                        .broadcaster
                        .bump(Some(name), Some(root.path.as_path()));
                }
            }
        }
    }
}

/// SSE 长驻循环：版本变更 → 一行变更帧；静置超时 → `: ping` 保活。
/// 写失败 = 客户端断开 → 退出（线程随之结束，不泄漏）。
pub(crate) fn sse_loop(stream: &mut TcpStream, broadcaster: &Broadcaster, keepalive: Duration) {
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let mut seen = broadcaster.version();
    loop {
        match broadcaster.wait_since(seen, keepalive) {
            Some((version, run, root)) => {
                seen = version;
                let data =
                    serde_json::to_string(&json!({"version": version, "run": run, "root": root}))
                        .unwrap_or_else(|_| {
                            "{\"version\":0,\"run\":null,\"root\":null}".to_owned()
                        });
                if write_sse(stream, &format!("event: change\ndata: {data}\n\n")).is_err() {
                    return;
                }
            }
            None => {
                if write_sse(stream, ": ping\n\n").is_err() {
                    return;
                }
            }
        }
    }
}

pub(crate) fn write_sse(stream: &mut TcpStream, frame: &str) -> std::io::Result<()> {
    stream.write_all(frame.as_bytes())?;
    stream.flush()
}
