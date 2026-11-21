use super::*;

impl Session {
    /// inbox 已消费行数（cursor 文件缺失 / 损坏 = 0：重放由 Mod 侧幂等去重兵底）。
    pub(crate) fn inbox_cursor(&self) -> usize {
        self.store.inbox_cursor().unwrap_or(0)
    }

    /// 前移消费位移（仅供 `drain_inbox` 在 dispatch + 存档成功后调）。
    fn write_inbox_cursor(&mut self, consumed: usize) {
        if let Err(message) = self.store.set_inbox_cursor(consumed, clock()) {
            // cursor 写不了不是丢需求：重放靠 external_id 幂等去重，只告警。
            self.notices
                .push(format!("写入 inbox cursor 失败：{message}"));
        }
    }

    /// 消费 inbox 新行：**整段持锁**（P0-6 / W6——两进程同时 drain 不再重复
    /// 消费同一行），逐行 dispatch（重入不二次取锁），成功后前移 cursor。
    ///
    /// 崩溃安全序不变（plan-r5 §2.6「不修 W8」）：dispatch（Mod 侧入队 + 自动
    /// 存档）→ 成功才写 cursor。「存档成功、cursor 未写」的重启会重放同一行，
    /// 靠 external_id 幂等去重；cursor 缺失/被截仍全量重放（W8 归 P1）。Mod 侧
    /// 拒绝（done / 队列满 / 载荷）走叙事面，**不算成功**：不推 cursor，留给
    /// 调用方报错。坏行跳过策略不变（D5 不在 P0）。
    pub(crate) fn drain_inbox(&mut self) -> InboxDrain {
        let started = Instant::now();
        let outcome = self.with_run_lock(|session| Ok(session.drain_inbox_locked()));
        let drain = outcome.unwrap_or_else(|message| {
            // 锁超时：本段未消费任何行（cursor 未动），报错留给调用方非零退出。
            InboxDrain {
                error: Some(message),
                ..InboxDrain::default()
            }
        });
        // 临界区时长守护（P0-11）：>1s 打 drain_slow warning，不阻断（B3）。
        let elapsed_ms = started.elapsed().as_millis();
        if elapsed_ms > DRAIN_SLOW_WARN_MS {
            self.notices.push(format!(
                "drain_slow：inbox 消费耗时 {elapsed_ms}ms（持久化临界区偏长，观测面）"
            ));
        }
        drain
    }

    /// drain 的锁内本体（旧逻辑原样；内部 dispatch_event 重入不加锁）。
    fn drain_inbox_locked(&mut self) -> InboxDrain {
        let mut drain = InboxDrain::default();
        let Ok(lines) = self.store.inbox_lines() else {
            return drain; // 读不到 = 无队列（与「无 inbox 文件」同一语义）
        };
        let mut consumed = self.inbox_cursor();
        if consumed > lines.len() {
            // inbox 被外部替换 / 截短：按从头消费算（幂等去重保证不重复入队）。
            consumed = 0;
        }
        while consumed < lines.len() {
            // 长临界区心跳：逐行续命锁文件 mtime，防等待者按 LOCK_STALE_MS
            // 误判活锁为崩溃残留而抢占（双持锁 = 互斥失效，实测重复 seq）。
            run_lock_heartbeat();
            let line = lines[consumed].as_str();
            let Ok(record) = serde_json::from_str::<Json>(line) else {
                // 坏行永远无法 dispatch：跳过并前移，否则整个队列永久阻塞。
                consumed += 1;
                drain.skipped += 1;
                self.write_inbox_cursor(consumed);
                self.notices
                    .push(format!("inbox 第 {consumed} 行不是合法 JSON，已跳过"));
                continue;
            };
            // kind 缺省 = add（旧行只有 title / description / external_id）。
            let kind = record["kind"].as_str().unwrap_or("add");
            // kind → 动作 + 段序完全来自 Mod 声明（宿主不枚举 kind）。
            let Some(route) = self.inbox_routes.get(kind) else {
                consumed += 1;
                drain.skipped += 1;
                self.write_inbox_cursor(consumed);
                self.notices.push(format!(
                    "inbox 第 {consumed} 行 kind 未在 pack.json 的 inboxRoutes 声明，已跳过"
                ));
                continue;
            };
            let Some(fields) = self.inbox_routes.payload_for(route, &record) else {
                consumed += 1;
                drain.skipped += 1;
                self.write_inbox_cursor(consumed);
                self.notices.push(format!(
                    "inbox 第 {consumed} 行缺字段（external_id / 声明的段全为空），已跳过"
                ));
                continue;
            };
            // payload 必须是 **JSON 字符串**（内核要求 JSON 标量），不是裸 wire format。
            let payload = json!(fields).to_string();
            let action = route.action.clone();
            // 身份随记录走（谁写的行归谁）：写命令/worker 落行时宿主盖的保留键
            // actor/actor_session 在这里取回，经 dispatch_event_as 注入声明路由。
            let actor = record["actor"].as_str().map(str::to_owned);
            let actor_session = record["actor_session"].as_str().map(str::to_owned);
            let receipt = self.dispatch_event_as(
                &action,
                &payload,
                actor.as_deref(),
                actor_session.as_deref(),
            );
            let rejected =
                receipt["ok"] != Json::Bool(true) || dispatch_rejected(&self.contract, &receipt);
            if rejected {
                // 内核错误信封（载荷 / 校验失败）与 Mod 侧拒绝同属「未入队」：
                // cursor 不前移，留给调用方报错与重启重放。
                drain.error = Some(first_rejection(&self.contract, &receipt));
                break;
            }
            consumed += 1;
            drain.applied += 1;
            if narrative_lines(&receipt)
                .iter()
                .any(|line| self.contract.is_replay(line))
            {
                drain.replayed += 1;
            }
            // dispatch_event 成功步已重写 autosave——到这里才允许前移 cursor。
            self.write_inbox_cursor(consumed);
            drain.last = Some(receipt);
        }
        drain.remaining = lines.len().saturating_sub(consumed);
        drain
    }
}
