use super::*;

impl Session {
    /// 新局重置（P0-5 / W4）：归档事件日志（**不截断**——`events.jsonl.reset.<纳秒>`）
    /// + 移除旧 autosave + 显式建空文件 + 初始存档同临界区。
    ///
    /// **fail-closed（plan-r5 §2.5 伪代码的 `?` 语义）**：取锁 / 归档 rename /
    /// 新建空档 / 初始存档任一步失败 → `Err` 上抛，由 `Session::open` 转为开局
    /// 失败（非零退出）。绝不返回「看似成功」的重置：盘上还是旧代产物时继续
    /// 新局 = 全新内核混进旧代（混代比丢开局更坏）。归档已发生的部分留下
    /// 即证据，但不算成功。`cleanup_stale_tmp` 那类可选清理仍只告警；
    /// `remove_file(autosave)` 也只告警——真正的一致性屏障是 `save_now` 的
    /// 原子替换（remove 失败而 save 成功 = 存档仍是新代，状态自洽）。
    ///
    /// `recheck`：进锁后二次判定「本局是否已被另一进程救活」（resume 失败路径传
    /// `true`；显式新局路径传 `false`，用户明确要求重置不跳过）。
    pub(crate) fn reset_artifacts(&mut self, recheck: bool) -> Result<ResetOutcome, String> {
        self.with_run_lock(|session| -> Result<ResetOutcome, String> {
            // 二次判定（锁内）：不猜「证据」，直接重试读档。本进程首次读档失败后、
            // 拿到锁之前，另一进程可能已把本局救活（归档重置后起步 / 修复文件）——
            // 此刻盘上状态**可装载**就续它：状态与内存同源，无混代；且不归档
            // 别人的行。证据启发式（events 非空 + autosave 可解析）不可靠：
            // ① 可解析 ≠ 可装载（版本不符会误判「已恢复」→ 以全新状态混入旧代
            // 事件日志，实测 RED）；② 读档恢复可能只进内存，不留盘面证据（漏判）。
            // 重试读档是唯一忠实判据（终态：不可装载的局不存在活着的使用者）。
            if recheck && session.load_autosave().is_ok() {
                return Ok(ResetOutcome::Recovered);
            }
            // 重置整个动作下沉到端口：归档 → 建新代 → 落初始存档。
            // 初始存档必须**先算出来**（内核 `save` 信封是纯计算，不碰盘）：算不出来
            // 就不该动盘；算出来之后的任何写失败都是 fail-closed。
            //
            // 后端的归档载体不同（JSON = `events.jsonl.reset.<纳秒>` 文件；
            // sqlite = 新 generation），但都只归档不删证据。
            let initial_save = session.current_save()?;
            let info = session
                .store
                .reset(&initial_save, reset_reason(recheck), clock())
                .map_err(|err| err.to_string())?;
            if let Some(archive) = &info.archive {
                // 归档载体的措辞按后端分：JSON 是文件路径（历史文案不变），
                // sqlite 是「第几代」（报路径会让用户去找一个不存在的文件）。
                let notice = match session.store.backend() {
                    Backend::Json => format!("events 已归档至 {archive}"),
                    Backend::Sqlite => format!("旧代已归档为 {archive}"),
                };
                session.notices.push(notice);
            }
            session.notices.extend(info.warnings);
            // 工具事实面属于本代：重置同时清空在飞调用与结果暂存，否则新局会看到
            // 上一代的 pending 条目（父进程早死 → `--wait-tools` 永久 PENDING）。
            // 只告警不阻断（与 `cleanup_stale_tmp` 同纪律：真正的屏障是原子替换）。
            let _ = fs::remove_file(session.run_dir.join(PENDING_TOOLS_FILE));
            let _ = fs::remove_dir_all(session.run_dir.join(TOOL_RESULTS_DIR));
            Ok(ResetOutcome::Reset {
                had_old: info.had_old,
            })
        })
        .map_err(|message| format!("重置产物失败：{message}"))
    }

    /// 事件日志既有条数（续局 seq 起点）。
    pub(crate) fn count_events(&self) -> u64 {
        // seq 唯一事实源在端口内：`next_seq` = 非空行数（含不可解析行），
        // 不是「可解析记录数」——两者在坏行场景下会不同，别换用 `steps().len()`。
        self.store.next_seq().unwrap_or(0)
    }

    /// mod 目录签名的人读串（落 run.json；跨进程比对「mod 变没变」）。
    pub(crate) fn signature_label(&self) -> String {
        self.signature
            .iter()
            .map(|stamp| {
                let modified = stamp
                    .modified
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |elapsed| elapsed.as_nanos());
                format!("{}:{}:{modified}", stamp.path, stamp.len)
            })
            .collect::<Vec<_>>()
            .join("|")
    }
}

impl Session {
    /// 现态存档字节（内核 `save` 信封 → 紧凑 JSON）：纯计算，不碰盘。
    ///
    /// 单独抽出来是为了让「存档算不出来」与「存档写不出去」两类失败在调用方
    /// 可分辨：前者不应动盘（reset 前先算），后者是持久化失败。
    pub(crate) fn current_save(&mut self) -> Result<Vec<u8>, String> {
        let envelope = parse_envelope(&self.kernel.dispatch("save", ""));
        if envelope["ok"] != Json::Bool(true) {
            return Err(format!("自动存档失败：{}", error_text(&envelope)));
        }
        let save = &envelope["save"];
        if save.is_null() {
            return Err("自动存档失败：save 信封缺少 save 字段".to_owned());
        }
        // 原样写 save 对象（紧凑 JSON）——与内核存档字节同形，可直接作 load payload。
        Ok(save.to_string().into_bytes())
    }

    /// 从本局后端的制品面回灌 VFS（开局 / 读档后调用）。
    ///
    /// **VFS 不在存档信封里**：`SaveFile` / `ENGINE_VERSION` /
    /// `canonical_state_bytes` 零改动，所以「读档要回到哪一代的制品」不能靠存档
    /// 字节携带——宿主是**唯一**的重新水化者（`cmd_load` 的路径本身不碰 VFS）。
    ///
    /// 回灌**不**置 dirty、**不**动 revision（见 `KernelFacade::vfs_load`）：它不是
    /// 本次会话产生的写入。
    pub(crate) fn preload_artifacts(&mut self) -> Result<(), String> {
        let entries = self.store.artifact_list().map_err(|err| err.to_string())?;
        for meta in entries {
            match self.store.artifact_get(&meta.path) {
                Ok(Some(record)) => {
                    self.kernel
                        .vfs_load(&record.meta.path, &record.content, &record.meta.sha256)
                }
                // 列表与读之间被另一进程归档掉：跳过（下一轮 list 会自愈）。
                Ok(None) => {}
                Err(err) => return Err(format!("预载制品失败：{err}")),
            }
        }
        Ok(())
    }

    /// 把 VFS 的未落盘条目写进本局后端的制品面（每成功步 / `s` 命令）。
    ///
    /// **fail-closed 且幂等**：中途失败时把「失败那条与尚未尝试的条目」一起
    /// 回填成 dirty（不碰 revision），下一轮重试只会重写相同字节——内容寻址让
    /// 重试不可能产生第二件制品，也不会把半个状态留在盘上。
    pub(crate) fn flush_artifacts(&mut self) -> Result<(), String> {
        let pending = self.kernel.vfs_take_dirty();
        for (index, (path, bytes, _sha256)) in pending.iter().enumerate() {
            if let Err(err) = self.store.artifact_put(path, bytes, clock()) {
                let requeue: Vec<String> = pending[index..]
                    .iter()
                    .map(|(path, _, _)| path.clone())
                    .collect();
                self.kernel.vfs_requeue_dirty(&requeue);
                return Err(format!("制品落盘失败：{err}"));
            }
        }
        Ok(())
    }

    /// 现态存档 → 端口（每成功步自动调用；`s` 命令亦复用）。
    pub(crate) fn save_now(&mut self) -> Result<(), String> {
        // 制品先行（幂等），存档在后：`set_save` 一旦写成功就宣称「这步已落地」，
        // 制品不得落在它之后（否则回执说落地、证据却缺了一件）。
        self.flush_artifacts()?;
        let save = self.current_save()?;
        // 本函数只在持久化临界区内被调用（dispatch_event / `s` 命令）。
        self.store
            .set_save(&save, clock())
            .map_err(|err| err.to_string())
    }

    /// 磁盘存档回灌进当前驱动（续局 / `l` 命令）。
    pub(crate) fn load_autosave(&mut self) -> Result<(), String> {
        let bytes = self
            .store
            .latest_save()
            .map_err(|err| err.to_string())?
            .ok_or_else(|| format!("读取 {} 失败：无存档", self.store.locator()))?;
        let text = String::from_utf8(bytes)
            .map_err(|err| format!("读取 {} 失败：{err}", self.store.locator()))?;
        let envelope = parse_envelope(&self.kernel.dispatch("load", &text));
        if envelope["ok"] != Json::Bool(true) {
            return Err(format!("读档失败：{}", error_text(&envelope)));
        }
        // `l` 命令 / 续局：状态回滚了，制品面也要回到同一代（VFS 不在存档字节里）。
        self.preload_artifacts()?;
        self.summary_cache = None;
        Ok(())
    }

    /// `--at g<代>` 回退（仅 sqlite 局）：取目标代末快照 → 经 reset 建新代续跑。
    ///
    /// 证据语义与显式新局同法：旧代事件与快照全留库（归档 = 新代，不删证据）；
    /// inbox 跨代保留；制品面不回滚（VFS 按新代回灌，与 reset 既有语义一致）。
    /// 目标代 = 当前代 → 无操作提示后走 latest（幂等）。
    pub(crate) fn rollback_to(&mut self, generation: i64) -> Result<(), String> {
        let points = self.store.savepoints().map_err(|err| err.to_string())?;
        let Some(target) = points.iter().find(|point| point.generation == generation) else {
            let available = points
                .iter()
                .map(|point| format!("g{}", point.generation))
                .collect::<Vec<_>>()
                .join(" / ");
            return Err(format!(
                "--at g{generation} 不存在：本局可用存档点 = {available}（代 = sqlite generation）"
            ));
        };
        if target.current {
            self.notices.push(format!(
                "--at g{generation} 即当前代（末快照 = 最新存档点），按最新存档点续跑"
            ));
            return self.load_autosave();
        }
        let bytes = self
            .store
            .save_at(generation)
            .map_err(|err| err.to_string())?
            .ok_or_else(|| format!("--at g{generation}：该代没有快照可取"))?;
        let text = String::from_utf8(bytes)
            .map_err(|err| format!("读取 g{generation} 存档失败：{err}"))?;
        let envelope = parse_envelope(&self.kernel.dispatch("load", &text));
        if envelope["ok"] != Json::Bool(true) {
            return Err(format!(
                "--at g{generation} 读档失败：{}",
                error_text(&envelope)
            ));
        }
        // 回退 = 把目标代末快照作为新代的初始存档：状态机先 load 到目标代，
        // 再 reset 归档当前代、落新代初始存档（与 `reset_artifacts` 同一临界区）。
        let save = self.current_save()?;
        let info = self
            .store
            .reset(&save, &format!("rollback:g{generation}"), clock())
            .map_err(|err| err.to_string())?;
        self.notices.push(format!(
            "已回退到存档点 g{generation}（该代 {} 步）：旧代证据留库（{}），新代从此续跑",
            target.steps,
            info.archive.as_deref().unwrap_or("无旧代")
        ));
        self.notices.extend(info.warnings);
        let _ = fs::remove_file(self.run_dir.join(PENDING_TOOLS_FILE));
        let _ = fs::remove_dir_all(self.run_dir.join(TOOL_RESULTS_DIR));
        self.summary_cache = None;
        Ok(())
    }

    /// `state` 诊断信封（不推进状态）。
    pub(crate) fn state_envelope(&mut self) -> Json {
        parse_envelope(&self.kernel.dispatch("state", ""))
    }
}
