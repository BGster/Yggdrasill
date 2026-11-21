use super::*;

impl Session {
    /// 回执标准字段：一次只读动作（契约的 `summaryEvent`）取 Mod **自报**的摘要。
    ///
    /// 这里**不读 autosave**：宿主对「结局 / 冲突 / 待验收提案 / 收口前一步」的
    /// 全部认知只来自这一条回执。缺失（旧 Mod）→ [`Summary::unknown`]，调用方
    /// 按安全值处理并停等。结果缓存到本步结束（一次派发只问一次）。
    pub(crate) fn summary(&mut self) -> Summary {
        if let Some(cached) = &self.summary_cache {
            return cached.clone();
        }
        let summary_event = self.contract.summary_event.clone();
        let payload = self.contract.default_payload.clone();
        let meta_key = self.contract.meta_key.clone();
        let envelope = parse_envelope(&self.kernel.dispatch(&summary_event, &payload));
        let summary = if envelope["ok"] == Json::Bool(true) {
            Summary::from_receipt(&envelope, &meta_key)
        } else {
            Summary::unknown()
        };
        self.summary_cache = Some(summary.clone());
        summary
    }

    /// 只读摘要查询（守护模式的 `summary` / `dispatch` 共用）：现派发契约的
    /// `summaryEvent`，把回执的 `meta.<metaKey>` 与 `pendingChoice` 一起交回。
    ///
    /// 关键点：它**不**经 `dispatch_event`——不落 `RunStore` 步、不推进 seq / 存档，
    /// 所以可以在两个请求之间任意次调用而不污染事件日志。
    pub(crate) fn query_summary_json(&mut self) -> Json {
        let summary_event = self.contract.summary_event.clone();
        let payload = self.contract.default_payload.clone();
        let meta_key = self.contract.meta_key.clone();
        let envelope = parse_envelope(&self.kernel.dispatch(&summary_event, &payload));
        let ok = envelope["ok"] == Json::Bool(true);
        // 回执缓存同步刷新：下一次 `summary()`（驱动报）不必再派发一次。
        self.summary_cache = Some(if ok {
            Summary::from_receipt(&envelope, &meta_key)
        } else {
            Summary::unknown()
        });
        let summary_val = envelope["meta"][&meta_key].clone();
        let harness_actions = extract_harness_actions(&summary_val);
        json!({
            "ok": ok,
            "summary": summary_val,
            "pendingChoice": envelope["pendingChoice"].clone(),
            "harnessActions": harness_actions,
        })
    }

    /// 挂载进程内同步工具执行器（两层白名单 + 六个内置工具直调）。
    pub(crate) fn mount_tool_runner(&mut self) {
        let runner = InProcessToolRunner {
            tools: self.tools.clone(),
            tool_access: self.tool_access.clone(),
            mod_dir: self.mod_dir.clone(),
            run_dir: self.run_dir.clone(),
            runs_root: self.runs_root.clone(),
            run_id: self.run_id.clone(),
            harness: self.harness.clone(),
        };
        self.kernel.set_tool_runner(Some(Box::new(runner)));
    }
}

impl Session {
    /// 派发一步：dispatch（锁外）→ 持久化临界区（seq 分配 + 事件行 + 自动存档）→ 回执。
    ///
    /// P0-3（plan-r5 §2.3）：三件事同临界区 —— seq 改为**锁内 count_events 重算**
    /// （唯一事实源，删进程内计数器），事件行单次 write_all，成功步存档同临界区。
    /// 锁内失败的失败面（写 event 成功、写存档失败的偏斜）在同一进程内被消除；
    /// 跨进程的「事件与状态原子同提交」不是 P0 的承诺。
    ///
    /// 旧签名（不带身份）委托 [`Session::dispatch_event_as`]：所有历史调用点
    /// （drain 无记录身份时 / scene-1 工具结果 / playtest 等）语义逐字节不变。
    pub(crate) fn dispatch_event(&mut self, name: &str, payload: &str) -> Json {
        self.dispatch_event_as(name, payload, None, None)
    }

    /// 带提交者身份的派发（宿主通道单点）：声明了身份段的路由由宿主注入
    /// `|actor=<id>` / `|actor_session=<id>`（相对顺序固定 actor 在前），
    /// 并把调用方自带的同名段判为伪造（不派发不落盘）；任意事件回执回显
    /// resolved 身份（加性键，None → null）；携带身份的成功步在同一持久化
    /// 临界区内先到先得认领 opener（不可变）。
    pub(crate) fn dispatch_event_as(
        &mut self,
        name: &str,
        payload: &str,
        actor: Option<&str>,
        actor_session: Option<&str>,
    ) -> Json {
        // 身份段声明来自路由表（声明承重：未声明的动作没有身份段，宿主不碰载荷）。
        let (declares_actor, declares_session) = self.inbox_routes.actor_flags_for(name);
        // ① 保留段伪造拦截：宿主是唯一合法的盖章方，调用方载荷自带 `actor=` /
        //    `actor_session=` 段一律拒绝——不派发、不落盘、不占 seq。
        if let Some(receipt) = self.actor_forged_receipt(
            name,
            payload,
            actor,
            actor_session,
            declares_actor,
            declares_session,
        ) {
            return receipt;
        }
        // ② 声明路由的段注入（exec= 之后、相对顺序固定：actor → actor_session）。
        let payload = &inject_identity_segments(
            payload,
            actor,
            actor_session,
            declares_actor,
            declares_session,
        );
        let envelope = parse_envelope(&self.kernel.dispatch(name, &payload)); // 锁外：Lua 计算
                                                                              // 真实派发可能已经改变语义面（阶段 / 结局 / 提案 / 冲突）→ 摘要缓存失效。
        self.summary_cache = None;
        let ok = envelope["ok"] == Json::Bool(true);
        let narrative = narrative_lines(&envelope);
        let ending = narrative
            .iter()
            .find(|line| line.starts_with(&self.contract.ending_prefix))
            .cloned();
        let ending_line = ending.as_deref();

        // 持久化临界区：seq 分配 + 事件行 + 存档（仅成功步）三件事同一把锁，
        // 全部经端口提交（trait 的前置条件正是「调用方已持锁」）。
        let persisted = self.with_run_lock(|session| -> Result<u64, String> {
            // 存档字节先算（内核纯计算）——算不出来就不该动盘（不写事件行）。
            let save = if ok {
                Some(session.current_save()?)
            } else {
                None
            };
            // 制品落盘（仅成功步）：VFS 的内容寻址让重试幂等，所以放在存档之前。
            // 失败步**不**进这里——内核已把 VFS（含 dirty）回滚到基线，此时写盘
            // 会把被回滚过的文件的旧副本落下来。
            if ok {
                session.flush_artifacts()?;
            }
            let committed = session
                .store
                .commit_step(StepDraft {
                    event: name,
                    payload: &payload,
                    ok,
                    narrative_count: narrative.len(),
                    // 时间戳由宿主给：端口不取时钟（否则黄金样本永不可重放）。
                    timestamp: now_secs(),
                    ending_line,
                    save: save.as_deref(),
                })
                .map_err(|err| err.to_string())?;
            // 非致命告警（事件行超大 / 写入失败但步继续）保持现状语义。
            session.notices.extend(committed.warnings);
            // opener 认领（仅成功步 + 携带身份；锁重入直接成功——同一临界区）。
            // 被拒 / 失败的派发不认领：失败命令不配当开局者。认领失败不阻断
            // 已提交的步（run.json 是元信息面），降级为警告。
            if ok {
                if let Some(id) = actor {
                    if let Err(message) = claim_opener(&session.run_dir, id, actor_session) {
                        session
                            .notices
                            .push(format!("opener 认领失败（不影响本步）：{message}"));
                    }
                }
            }
            Ok(committed.seq)
        });
        let seq = match persisted {
            Ok(seq) => seq,
            Err(message) => {
                // 锁超时 / 存档失败：状态已进内存但未落盘 —— 回执 fail-closed
                //（B2：不声称成功；events.jsonl 未新增行）。单发路径凭 ok=false
                // 非零退出，文案沿用 acquire_dir_lock 的超时报错。
                eprintln!("error: 持久化失败：{message}");
                let failed_envelope = json!({
                    "ok": false,
                    "error": {"code": "PERSIST", "message": message},
                });
                let pending = self.state_envelope()["pendingChoice"].clone();
                let receipt = self.build_receipt(
                    name,
                    self.count_events(),
                    &failed_envelope,
                    &narrative,
                    ending_line,
                    pending,
                );
                self.reload = None;
                return receipt;
            }
        };

        // 待决选择：成功信封自带；失败信封补一次 state（不推进状态），
        // 让回执在「越权动作被拒」之后仍能指引下一步。
        let pending = if ok {
            envelope["pendingChoice"].clone()
        } else {
            self.state_envelope()["pendingChoice"].clone()
        };

        // 工具请求（I1：内核只见瞬态 `meta.tool_calls`；结果以**普通事件**回投）。
        //
        // 必须在 `build_receipt` **之前**跑完：回执会 `mem::take` 掉 notices（R7）。
        // 失败步不跑工具：工具只服务成功派发（半个状态上的副作用最难查）。
        let tool_runs = if ok {
            self.run_tool_requests(&envelope, seq)
        } else {
            Vec::new()
        };

        let mut receipt =
            self.build_receipt(name, seq, &envelope, &narrative, ending_line, pending);
        if !tool_runs.is_empty() {
            receipt["tools"] = json!(tool_runs);
        }
        // 提交者章回显（§3.5 增强，任意事件；加性键，None → null）：
        // config.set 等非声明路由的直派回执也带提交者章，主 Agent 代写与
        // subagent 自主写（如 profile 注册）在回执面可区分。章只进回执，
        // 不进事件行、不进 wire（wire 段注入仅限声明路由）。
        receipt["actor"] = json!(actor);
        receipt["actor_session"] = json!(actor_session);
        self.reload = None;
        receipt
    }

    /// 保留段伪造拦截：声明了身份段的路由上，调用方载荷自带 `actor=` /
    /// `actor_session=` 段一律拒绝（宿主是唯一盖章方；没有宿主章的“身份”
    /// 不该存在）。返回 `Some(回执)` = 已拦截：未派发、未落盘、seq 不前进。
    fn actor_forged_receipt(
        &mut self,
        name: &str,
        payload: &str,
        actor: Option<&str>,
        actor_session: Option<&str>,
        declares_actor: bool,
        declares_session: bool,
    ) -> Option<Json> {
        if !declares_actor && !declares_session {
            return None;
        }
        let offending = payload.split('|').find_map(|segment| {
            let segment = segment.trim();
            // `actor_session=x` 不匹配 `actor=` 前缀（第 6 字节是 `_` 不是 `=`），
            // 两个保留段可独立判定。
            if declares_actor && segment.starts_with("actor=") {
                return Some("actor=");
            }
            if declares_session && segment.starts_with("actor_session=") {
                return Some("actor_session=");
            }
            None
        });
        let segment = offending?;
        let message = format!(
            "载荷自带保留段 `{segment}`…：宿主是 `{name}` 身份段的唯一盖章方（携带身份请用 \
             --actor / YGG_ACTOR / daemon 请求字段），本步未派发未落盘",
        );
        let envelope = json!({
            "ok": false,
            "error": {"code": "ACTOR_FORGED", "message": message},
        });
        let pending = self.state_envelope()["pendingChoice"].clone();
        let narrative = vec![format!("[ACTOR FORGED] {message}")];
        Some(self.build_receipt(
            name,
            self.count_events(),
            &envelope,
            &narrative,
            None,
            pending,
        ))
    }

    /// 单发 / 交互统一回执（stdout 契约本体）。
    ///
    /// **`seq` 语义**：这里是**本步**的 seq。若本步的 `meta.tool_calls` 在
    /// `inlineWaitMs` 内落地（场景 1），结果事件会再提交一步——外层回执的 `seq`
    /// 与回执里 `tools[].seq` 因此可能相差 ≥1（不是「seq 不连续」，而是「同一次
    /// 用户动作产生了多步事件」）。场景 2 的结果 seq 由后续 drain 的派发出现。
    fn build_receipt(
        &mut self,
        name: &str,
        seq: u64,
        envelope: &Json,
        narrative: &[String],
        ending: Option<&str>,
        pending: Json,
    ) -> Json {
        let ok = envelope["ok"] == Json::Bool(true);
        let mut receipt = json!({
            "ok": ok,
            "event": name,
            "narrative": narrative,
            "pendingChoice": pending,
            "autosave": self.store.locator(),
            "seq": seq,
        });
        if ok {
            receipt["revision"] = envelope["revision"].clone();
        } else {
            receipt["error"] = envelope["error"].clone();
        }
        if ending.is_some() {
            receipt["ending"] = json!(true);
        }
        // TD1：结构化 meta 原样透传（宿主只读其中的 `summary`，不解释其他键；
        // 它不进叙事、不进存档，只作为回执的独立字段给调用方用）。
        if envelope.get("meta").is_some() {
            receipt["meta"] = envelope["meta"].clone();
        }
        let harness_actions = envelope_harness_actions(envelope, &self.contract.meta_key);
        if !harness_actions.is_empty() {
            receipt["harnessActions"] = json!(harness_actions);
        }

        // 警告面：内核 D1 warnings + 宿主侧 notices（热载 / 存档 / 重置）。
        let mut warnings: Vec<Json> = envelope["warnings"].as_array().cloned().unwrap_or_default();
        for notice in std::mem::take(&mut self.notices) {
            warnings.push(json!(notice));
        }
        if !warnings.is_empty() {
            receipt["warnings"] = json!(warnings);
        }

        if let Some(reload) = &self.reload {
            receipt["hotReload"] = reload.to_json();
        }
        receipt
    }
}

impl Session {
    /// 一条派发链里的工具回投：解析 → 登记 + spawn 脱体 worker → 按 `inlineWaitMs` 收口。
    ///
    /// 返回回执的 `tools` 数组（空 = 本步没有工具请求，回执不出现该键）。
    /// `seq` 是本步已提交的步号（call_id 缺省值的片段；**不**再读端口，
    /// 因为此处已出持久化临界区——trait 的前置条件是「调用方已持锁」）。
    pub(crate) fn run_tool_requests(&mut self, envelope: &Json, seq: u64) -> Vec<Json> {
        if envelope["meta"].get("tool_calls").is_none() {
            return Vec::new();
        }
        if self.tool_depth.get() >= MAX_TOOL_ROUNDS {
            self.notices.push(format!(
                "工具回投已达 {MAX_TOOL_ROUNDS} 轮上限，本步的 meta.tool_calls 已忽略（防自激循环）"
            ));
            return Vec::new();
        }
        self.tool_depth.set(self.tool_depth.get() + 1);
        let _guard = ToolDepthGuard(self.tool_depth.clone());
        self.run_tool_requests_inner(envelope, seq)
    }

    /// `run_tool_requests` 的本体（深度已记账）。
    fn run_tool_requests_inner(&mut self, envelope: &Json, seq: u64) -> Vec<Json> {
        let prefix = format!("{}-{seq}", self.run_id);
        let (requests, notices) = parse_tool_calls(&envelope["meta"], &prefix);
        self.notices.extend(notices);
        if requests.is_empty() {
            return Vec::new();
        }
        let env = load_env_table(Some(&self.mod_dir));
        let mut runs = Vec::new();
        for request in requests {
            let Some(spec) = self.tools.get(&request.tool).cloned() else {
                self.notices.push(format!(
                    "工具 `{}` 未在 pack.json 的 `tools` 段声明，已跳过",
                    request.tool
                ));
                continue;
            };
            // 纵深防御：装载期已拦过未放行的声明，这里再确认一次（热载 / 手改档的兵底）。
            if !self.tool_access.allows(&spec.id) {
                self.notices.push(format!(
                    "工具 `{}` 未被本局 `allowedTools` 放行，已跳过",
                    spec.id
                ));
                continue;
            }
            if let Err(message) = validate_expansion(&spec, &request.args, &env) {
                self.notices.push(message);
                continue;
            }
            let call = PendingCall::from_spec(
                &spec,
                &request,
                now_ms() + u128::from(spec.timeout_ms),
                self.tool_depth.get(),
            );
            // 锁内登记（模板 + deadline），锁外 spawn + 轮询（I2）。
            // 若该 call_id 已在飞（Mod 的 meta.tool_calls 快照重报了尚未收口的后台项），
            // 直接记为 detached 并跳过二次 spawn，严格守住 I3（单一执行）。
            match pending_tools_insert(&self.run_dir, &call) {
                Ok(true) => {}
                Ok(false) => {
                    runs.push(json!({
                        "callId": call.call_id,
                        "tool": spec.id,
                        "status": "detached",
                        "resultEvent": spec.result_event,
                    }));
                    continue;
                }
                Err(message) => {
                    self.notices.push(format!(
                        "工具 `{}` 未能登记到 {PENDING_TOOLS_FILE}（{message}），已跳过",
                        spec.id
                    ));
                    continue;
                }
            }
            // 实时转换期不派生后台脱体 OS 子进程：inlineWaitMs > 0 时同进程同步执行并就地回投，
            // inlineWaitMs == 0 时仅登记在飞模板（供 `--wait-tools` / `--tool-result` 使用）。
            if spec.inline_wait_ms > 0 {
                let scratch = tool_results_dir(&self.run_dir);
                let _ = fs::create_dir_all(&scratch);
                let outcome = execute_tool(&call, &env, &scratch);
                let external_id = new_external_id(&self.run_id, 0);
                let result = ToolResult::from_outcome(&call, &outcome, &external_id);
                let _ = write_tool_result(&self.run_dir, &result);
                if let Some(run) = self.settle_tool_call(&spec, &call) {
                    runs.push(run);
                    continue;
                }
            }
            match self.settle_tool_call(&spec, &call) {
                Some(run) => runs.push(run),
                None => runs.push(json!({
                    "callId": call.call_id,
                    "tool": spec.id,
                    "status": "detached",
                    "resultEvent": spec.result_event,
                })),
            }
        }
        runs
    }

    /// 场景 1 收敛：轮询结果文件 → **认领** → 摘 pending → 就地 `dispatch_event`。
    ///
    /// 返回 `None` = 交付责任在 worker（场景 2：`inlineWaitMs == 0`、超时未落地、
    /// 或 worker 已先认领）。
    pub(crate) fn settle_tool_call(&mut self, spec: &ToolSpec, call: &PendingCall) -> Option<Json> {
        if spec.inline_wait_ms == 0 {
            return None;
        }
        let deadline = Instant::now() + Duration::from_millis(spec.inline_wait_ms);
        loop {
            if let Some(result) = read_tool_result(&self.run_dir, &call.call_id) {
                if claim_tool_result(&self.run_dir, &call.call_id) {
                    let _ = pending_tools_remove(&self.run_dir, &call.call_id);
                    let _ = fs::remove_file(tool_result_path(&self.run_dir, &call.call_id));
                    // 载荷必须是 **JSON 字符串**（与 `drain_inbox_locked` 同形）。
                    let payload = json!(result.payload.clone()).to_string();
                    let inner = self.dispatch_event(&spec.result_event, &payload);
                    return Some(json!({
                        "callId": result.call_id,
                        "tool": spec.id,
                        "status": result.status,
                        "code": result.code,
                        "seq": inner["seq"].clone(),
                        "ok": inner["ok"].clone(),
                    }));
                }
                // worker 已先认领：回投走 inbox（场景 2），本次不重复派发（I3）。
                return None;
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(TOOL_POLL_INTERVAL);
        }
    }
}
