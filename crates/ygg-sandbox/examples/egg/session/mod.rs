use super::*;

// ============================================================================
// §摘要 Mod 回执标准字段（宿主唯一的语义判定面）
//
// 反耦合的落点：宿主不再认识 `ending` / `fsm.stage` / `verify.passed` / `prop.*` /
// `req.*` 这些 **Mod 私有 KV 键**，只读 Mod **自己声明**的一行摘要（前缀 +
// 单行 JSON）。换 Mod（或 Mod 改内部键名）不需要改宿主。
//
// 缺摘要（旧 Mod / 摘要行坏 JSON）→ [`Summary::unknown`]：各判定返回安全值
// （0 / false / 空），宿主**不回退到读 KV**，驱动侧据此安全停等。
// ============================================================================

/// 一条未解决冲突（Mod 自报：需求 id / 对侧 / 说明）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ConflictNote {
    pub(crate) id: String,
    pub(crate) peer: String,
    pub(crate) note: String,
}

impl ConflictNote {
    /// 人读形态（`r2 ⇄ r3：说明`）。
    pub(crate) fn label(&self) -> String {
        format!("{} ⇄ {}：{}", self.id, self.peer, self.note)
    }
}

/// 一个在跑但还没有提案的作业（Mod 自报：需求 id / 修订号 / 作业阶段）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AwaitingJob {
    pub(crate) req: String,
    pub(crate) rev: u64,
    pub(crate) stage: String,
}

/// Mod 回执标准字段快照。
///
/// `available = false` = Mod 没给摘要（旧 Mod / 坏 JSON），此时**所有**判定
/// 返回保守值；驱动侧必须停等而不是猜。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Summary {
    pub(crate) available: bool,
    /// 当前阶段（仅展示用——宿主不按它分支）。
    pub(crate) stage: String,
    /// 本局已到结局（Mod 侧收口标记）。
    pub(crate) ending: bool,
    /// review 阶段的门禁自检已过。
    pub(crate) verify_passed: bool,
    /// **下一步就是收口动作**（Mod 用「菜单首项是否 review.note」判定）：
    /// 宿主据此在派发前先进 closing，把最后一次 drain 排在收口之前。
    /// r9 常驻语义：只有 run.retire 武装后的终局变体才为 true；未武装的批收口恒 false。
    pub(crate) next_ends_run: bool,
    /// r9 常驻生命周期：mod 自报模式（常量 `"resident"`）。
    /// 缺省空串 = 旧 mod：宿主所有常驻特化行为以它为闸（缺失即行为不变）。
    pub(crate) lifecycle: String,
    /// r9 常驻生命周期：`run.batch` 镜像（批轮转可观测计数；缺省 1）。
    pub(crate) batch: usize,
    /// r9 常驻生命周期：批排空暂停信号 =
    /// `coordinate ∧ gate_problem()==nil ∧ run.retire 未武装`（缺省 false = 旧 mod 不触发）。
    pub(crate) batch_drained: bool,
    /// 待验收提案数。
    pub(crate) pending_proposals: usize,
    /// 已入队需求总数（步数预算按它放宽）。
    pub(crate) requirement_count: usize,
    /// 未解决冲突（非空 = 用户门禁，自动推进必须停下）。
    pub(crate) conflicts: Vec<ConflictNote>,
    /// 在跑但无提案的作业（非空 = 等外部执行方，宿主不造内容）。
    pub(crate) awaiting: Option<AwaitingJob>,
}

impl Summary {
    /// 未知摘要：安全值（`available=false`）。
    fn unknown() -> Self {
        Self::default()
    }

    /// 本局是否已收口：`None` = Mod 未提供摘要（不得猜）。
    pub(crate) fn ending(&self) -> Option<bool> {
        if self.available {
            Some(self.ending)
        } else {
            None
        }
    }

    /// 从回执信封取结构化摘要：读 `meta.<agent.metaKey>`（键名由 Mod 声明）。
    ///
    /// 缺失（旧 Mod / 该动作没调摘要写入接口）或形状不是对象 → [`Summary::unknown`]
    /// （安全值，调用方停等而不是猜）。
    pub(crate) fn from_receipt(receipt: &Json, meta_key: &str) -> Self {
        match &receipt["meta"][meta_key] {
            value if value.is_object() => Self::from_json(value),
            _ => Self::unknown(),
        }
    }

    /// JSON → 摘要（缺字段一律取保守默认值；只有「是对象」才算 available）。
    pub(crate) fn from_json(value: &Json) -> Self {
        let count = |key: &str| value[key].as_u64().map_or(0, |n| n as usize);
        Self {
            available: true,
            stage: value["stage"].as_str().unwrap_or("").to_owned(),
            ending: value["ending"] == Json::Bool(true),
            verify_passed: value["verifyPassed"] == Json::Bool(true),
            next_ends_run: value["nextEndsRun"] == Json::Bool(true),
            lifecycle: value["lifecycle"].as_str().unwrap_or("").to_owned(),
            batch: value["batch"].as_u64().map_or(1, |n| n as usize).max(1),
            batch_drained: value["batchDrained"] == Json::Bool(true),
            pending_proposals: count("pendingProposals"),
            requirement_count: count("requirementCount"),
            conflicts: value["conflicts"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|item| ConflictNote {
                    id: item["id"].as_str().unwrap_or("").to_owned(),
                    peer: item["peer"].as_str().unwrap_or("?").to_owned(),
                    note: item["note"].as_str().unwrap_or("").to_owned(),
                })
                .filter(|item| !item.note.trim().is_empty())
                .collect(),
            awaiting: value["awaiting"].as_object().map(|item| AwaitingJob {
                req: item["req"].as_str().unwrap_or("").to_owned(),
                rev: item["rev"].as_u64().unwrap_or(1),
                stage: item["stage"].as_str().unwrap_or("").to_owned(),
            }),
        }
    }
}

// ============================================================================
// §Session 一局的宿主侧状态：驱动 + run 目录 + 事件序号 + 热载签名
// ============================================================================

/// 热载事件（回执 `hotReload` 键；`reloaded=false` 时旧 mod 仍在跑，局不中断）。
pub(crate) struct Reload {
    /// 是否成功换上新 mod（false = 沿用旧 mod 继续）。
    pub(crate) reloaded: bool,
    /// 触发面：`resume`（开局按新包装载 + 回灌状态）/ `watch`（派发前查盘）。
    pub(crate) source: &'static str,
    /// 失败原因（`reloaded=false` 时非空）。
    pub(crate) message: Option<String>,
}

impl Reload {
    fn to_json(&self) -> Json {
        let mut value = json!({"reloaded": self.reloaded, "source": self.source});
        if let Some(message) = &self.message {
            value["error"] = json!(message);
        }
        value
    }
}

/// 一次 inbox 消费的账（回执 `inbox` 键 + 自动推进 / stdin 提示共用）。
#[derive(Default)]
pub(crate) struct InboxDrain {
    /// 本次成功消费的需求条数（cursor 已前移；含幂等重放）。
    pub(crate) applied: usize,
    /// 其中幂等重放（同 external_id 已存在，Mod 侧去重）的条数。
    pub(crate) replayed: usize,
    /// 坏行跳过条数（cursor 已前移，否则永久阻塞）。
    pub(crate) skipped: usize,
    /// 未消费条数（dispatch 未成功 → cursor 不前移，重启可重放）。
    pub(crate) remaining: usize,
    /// 最近一次成功 dispatch 的回执（自动推进据此刷新 pendingChoice）。
    pub(crate) last: Option<Json>,
    /// 阻塞原因（`remaining > 0` 时非空）。
    pub(crate) error: Option<String>,
}

impl InboxDrain {
    pub(crate) fn touched(&self) -> bool {
        self.applied > 0 || self.skipped > 0 || self.remaining > 0
    }

    pub(crate) fn to_json(&self) -> Json {
        json!({
            "applied": self.applied,
            "replayed": self.replayed,
            "skipped": self.skipped,
            "remaining": self.remaining,
        })
    }
}

pub(crate) struct Session {
    pub(crate) run_id: String,
    pub(crate) mod_dir: PathBuf,
    /// 静态配置资源的包内路径（`play.mode` 的静态面；来自 pack.json 声明）。
    pub(crate) config_rel: String,
    /// 事件名契约（`pack.json` 的 `agent` 段；热载时随之刷新）。
    pub(crate) contract: AgentContract,
    /// inbox 路由表（`pack.json` 的 `inboxRoutes` 段；热载时随之刷新）。
    pub(crate) inbox_routes: InboxRoutes,
    /// 自动推进阈值（`config.json` 的 `drive` 段；装载期校验，热载时随之刷新）。
    pub(crate) limits: DriveLimits,
    /// 结算口径（`config.json` 的 `settlement` 段；装载期校验 = 缺段即拒绝启动，
    /// 不把「口径缺失」推迟到结算那一刻才暴露）。
    pub(crate) settlement: SettlementPolicy,
    /// 工具声明（`pack.json` 的 `tools` 段；装载期校验，热载时随之刷新）。
    /// 空 = 本 Mod 不请求外部工具（既有 Mod 零改动照常工作）。
    pub(crate) tools: ToolRegistry,
    /// 本局放行清单（`config.json` 的 `allowedTools`；空 = 不允许任何工具）。
    pub(crate) tool_access: ToolAccess,
    /// 当前派发链的工具回投深度（`1..=MAX_TOOL_ROUNDS`）。用 `Rc<Cell<_>>` 是为了
    /// 让 `Drop` guard 能归零（函数体用 `&mut self`，无法用借用式 guard）。
    pub(crate) tool_depth: Rc<Cell<usize>>,
    pub(crate) mod_label: String,
    pub(crate) run_dir: PathBuf,
    /// runs 根目录（脱体 worker 需要重建 `--runs-root`，从 run_dir 反推不可靠）。
    pub(crate) runs_root: PathBuf,
    /// 持久化端口（JSON 文件 / sqlite 库）。**状态事实的唯一入口**：存档、
    /// 事件日志、inbox、位移、重置都经它；锁与 run.json / settlement.json /
    /// coordination.* 仍在本文件（端口内不得出现任何锁原语）。
    pub(crate) store: Box<dyn RunStore>,
    pub(crate) meta_path: PathBuf,
    /// 异步需求队列的**文件路径**（仅用于人读输出；真正的读写经 `store`）。
    pub(crate) inbox_path: PathBuf,
    pub(crate) kernel: KernelFacade,
    /// 装载时的 mod 目录签名（热载比对基线；每次成功/失败尝试后前移）。
    pub(crate) signature: Vec<FileStamp>,
    /// 本次回执待携带的宿主侧提示（热载 / 自动存档 / 重置等）。
    pub(crate) notices: Vec<String>,
    /// 本次派发前的热载结果（一次性；派发收口后清空）。
    pub(crate) reload: Option<Reload>,
    /// 回执标准字段缓存（每次真实派发 / 读档 / 热载后失效）。
    pub(crate) summary_cache: Option<Summary>,
    /// 是否由 `--resume` 成功续局（交互模式据此决定回显而非推进）。
    pub(crate) resumed: bool,
    /// `--harness` 适配器覆盖（供进程内内置工具 `archive-sessions` 读取）。
    pub(crate) harness: Option<String>,
    /// `--packages` 显式包根（开局与热载传同一值；`None` = 沿用
    /// `resolve_root` 既有优先级）。
    pub(crate) packages_root: Option<PathBuf>,
    /// 本进程的提交者身份（旗标 > 环境变量，见 [`resolve_identity`]）。
    /// daemon 模式下请求字段可在其上按链补充；drain 派发用**记录自带**身份，
    /// 不用本字段（谁写的行归谁）。
    pub(crate) identity: ActorIdentity,
}

/// 单段 drain 的「过慢」阈值（plan-r5 §2.6 临界区时长守护：观测不阻断）。
pub(crate) const DRAIN_SLOW_WARN_MS: u128 = 1_000;

/// 开局模式（r1 归并）：`--wait-tools` 只观察，**不得**重置别人的局。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenMode {
    /// `egg new`：按新局重置（旧产物归档留证据）。
    Fresh,
    /// `egg open`：附着读档（最新存档点或 `--at` 指定代）；读不到就报错，
    /// 绝不重置（零写入）——`--wait-tools` 与本模式同一路径。
    Attach,
}

pub(crate) enum ResetOutcome {
    /// 已执行重置（含目录本就干净的初次建代）；`had_old` = 目录里本来有旧产物
    /// （显式新局回执用它提示「已有旧产物，按新局重置」）。
    Reset { had_old: bool },
    /// 锁内重试读档成功：本局已被另一进程救活，状态已回灌——不重置、不按新局
    /// 开局，续当前盘上状态（resume 失败路径专用；显式新局不二次判定，不会返回它）。
    Recovered,
}

impl Session {
    /// 开局：读盘装载 → 建 run 目录 → 续局或重置 → 定 seq → 落元信息。
    pub(crate) fn open(args: &Args) -> Result<Self, String> {
        let mode = match args.open_style {
            Some(OpenStyle::New) => OpenMode::Fresh,
            _ => OpenMode::Attach,
        };
        Self::open_mode(args, mode)
    }

    /// 开局（带模式）：`Attach` = 只观察/附着（open 与 `--wait-tools` 共用）。
    pub(crate) fn open_mode(args: &Args, mode: OpenMode) -> Result<Self, String> {
        let run_dir = args.run_root.clone();
        // open 的 --mod 可省略：run.json 记录的本局 mod 路径是唯一回退事实
        // （resolve_mod_dir = 旗标 > run.json > 可读拒绝）。
        let requested = resolve_mod_dir(args, &run_dir)?;
        let run_id = run_dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| args.run_root.display().to_string());
        let runs_root = run_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        // 别名退役 退役专名拒绝在入口生效（与 resolve_mod_dir 同一份理由源）：旧名不再翻译，
        // 缺席且形状命中即给可读拒绝；其余缺席维持既有装载期通用错误。
        let mod_dir = if requested.exists() {
            requested
        } else {
            match retired_seed_name_reason(&requested) {
                Some(reason) => return Err(reason),
                None => requested,
            }
        };

        // 先装载（失败即退出，run 目录不留空壳），再建 run 目录接管产物。
        let bundle = ModBundle::load_with_packages_root(&mod_dir, args.packages.as_deref())?;
        let mut kernel = kernel_from(&bundle)?;
        // 激活触点（S-1）：开局装配失败 = 装载失败同语义（fail-closed 退出）。
        // attach 只落 <run>/mounts/ 壳（不产 autosave/events/sqlite 产物），
        // 不影响后文的产物探测与 --force 判定。
        activate_host_faces(&mut kernel, &run_dir, &mod_dir)?;
        // host 契约是**必填面**：事件名 / inbox 路由 / 结算口径 / 推进阈值全部来自
        // Mod 声明，缺任何一项即装载期失败（宿主不保留任何默认值，fail-closed）。
        let contract = AgentContract::load(&mod_dir)?;
        let inbox_routes = InboxRoutes::load(&mod_dir)?;
        let config_rel = agent_config_path(&mod_dir);
        let limits = DriveLimits::load(&mod_dir.join(&config_rel))?;
        let settlement = SettlementPolicy::load(&mod_dir.join(&config_rel))?;
        // 两层工具白名单也在装载期现读：Layer 1（`tools`）定形状与能力，
        // Layer 2（`allowedTools`）定本局放行清单；声明的工具未被放行即拒绝启动
        // （I6：不把半声明推迟到执行时）。两层都缺失 = 工具能力不可用（合法）。
        let tools = ToolRegistry::load(&mod_dir)?;
        let tool_access = ToolAccess::load(&mod_dir.join(&config_rel))?;
        require_tool_allowlist(&tools, &tool_access, &mod_dir.join(&config_rel))?;
        // 存储后端裁决（`run.json` 的 `store` 是权威事实，`--store` 只是开局断言）：
        // **必须在任何写入之前**（含 create_dir_all / 建库），不一致就零写入失败。
        let declared = read_meta(&run_dir.join(META_FILE))
            .and_then(|meta| meta["store"].as_str().map(str::to_owned))
            .and_then(|text| Backend::parse(&text));
        let backend =
            ygg_store::resolve(&run_dir, declared, args.store).map_err(|err| err.to_string())?;
        // new 的 --force 判定必须在 open_store 之前快照：sqlite 建库即产生产物，
        // 事后 detect 会把自己刚建的库误判为「旧产物」（R10a 实测 RED）。
        let had_products_before_open = ygg_store::detect(&run_dir).any();
        fs::create_dir_all(&run_dir)
            .map_err(|err| format!("创建 run 目录 {} 失败：{err}", run_dir.display()))?;
        let store = open_store(backend, &run_dir)?;
        // 装载即取签名：开局装载永远读的是磁盘现状，之后的差异才是热载触发。
        let signature = mod_signature(&mod_dir);
        // 上一局落盘的签名（用于报告「续局时 mod 已被改过」——单发模式下
        // 这就是热载的可观测面：新包已装载、状态已回灌）。
        let previous_signature = read_meta_signature(&run_dir.join(META_FILE));

        let mut session = Self {
            run_id,
            mod_dir: mod_dir.clone(),
            config_rel,
            contract,
            inbox_routes,
            limits,
            settlement,
            tools,
            tool_access,
            tool_depth: Rc::new(Cell::new(0)),
            mod_label: mod_dir.display().to_string(),
            store,
            meta_path: run_dir.join(META_FILE),
            inbox_path: run_dir.join(INBOX_FILE),
            run_dir,
            runs_root,
            kernel,
            signature,
            notices: Vec::new(),
            reload: None,
            summary_cache: None,
            resumed: false,
            harness: args.harness.clone(),
            packages_root: args.packages.clone(),
            identity: resolve_identity_or_usage(args),
        };
        session.mount_tool_runner();

        // 残留 tmp 清理（W3 配套）：持锁 + 10s mtime 窗口，不误删活进程的 tmp。
        // 先把「后端事实」落盘再动状态：`run.json.store` 必须早于第一次写入
        // 存档，否则旧版工具可能先看到一个无后端声明的目录。
        session.write_meta();
        session.cleanup_stale_tmp();
        if mode == OpenMode::Attach {
            // 附着面（open / --wait-tools）：读档失败就是失败，**不**按新局重置
            // ——那会把别人正在跑的局清掉（零写入）。
            if let Some(generation) = args.at {
                session.rollback_to(generation)?;
            } else {
                session.load_autosave().map_err(|message| {
                    format!("{message}；open 只附着不重置（新局请用 `egg new`）")
                })?;
            }
            session.resumed = true;
        } else {
            // new 的 fail-closed：目录已有旧产物且未给 --force → 零写入拒绝
            // （自动 id 撞既有目录几乎不可能，此处守的是「显式重置必须显式许可」）。
            if !args.force && had_products_before_open {
                return Err(format!(
                    "run `{}` 已有旧产物——new 不隐式重置（加 --force 显式许可，\
                     或用 `egg open {}` 附着续跑）",
                    session.run_id,
                    session.run_dir.display()
                ));
            }
            match session.reset_artifacts(false) {
                Ok(ResetOutcome::Reset { had_old: true }) => {
                    session
                        .notices
                        .push(format!("run `{}` 已有旧产物，按新局重置", session.run_id));
                }
                Ok(_) => {}
                Err(reset_error) => return Err(reset_error),
            }
        }
        // 制品面回灌（VFS 侧）：**必须在 reset / 读档之后**——新代从空 VFS 起步，
        // 而「这一代盘上有哪些制品」只有宿主知道（VFS 不在存档信封里，见
        // `preload_artifacts` 的文档）。
        session.preload_artifacts()?;
        // seq 不再有进程内计数器：每步在持久化临界区内 count_events 重算（P0-3 / B1），
        // 缓存即回到「多进程各自基线」的原点。
        // 续局成功且 mod 自上次派发起变过：新包已生效（状态无损），回执标记。
        if session.resumed
            && previous_signature
                .as_deref()
                .is_some_and(|previous| previous != session.signature_label())
        {
            session.reload = Some(Reload {
                reloaded: true,
                source: "resume",
                message: None,
            });
        }
        session.write_meta();
        // 崩溃恢复：上次若在 closing 中挂掉（owner 心跳过期）且本局未 done，收回到 running。
        // 只有真的处在 closing 才去问 Mod 的摘要（省一次派发）；摘要缺失（旧 Mod）
        // 时不动协议状态——猜错方向比不动更坏。
        if read_coordination(&session.run_dir)["phase"].as_str() == Some(PHASE_CLOSING) {
            if let Some(ended) = session.summary().ending() {
                if let Some(notice) = recover_stale_closing(&session.run_dir, ended) {
                    session.notices.push(notice);
                }
            }
        }
        Ok(session)
    }

    /// 唯一临界区入口（P0-1）：持久化面的多步临界操作（seq 分配 + 事件 +
    /// 存档 / drain / reset）都经它持锁。f 拿 `&mut Self`，避免「持锁」与
    /// 「改状态」的借用冲突；重入（depth>0）时直接执行不重复取锁；panic
    /// 展开时 guard 的 Drop 归零 depth 并删锁文件（U2 的断言面）。
    pub(crate) fn with_run_lock<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        let _guard = acquire_run_lock(&self.run_dir)?;
        f(self)
    }

    /// 清理残留的 autosave tmp（崩溃 / kill -9 留下）：只删 mtime 早于
    /// CLOSING_STALE_MS 的（10s 窗口与 closing 心跳同源，plan-r5 §2.4）；新鲜
    /// tmp 可能有活进程正在写，绝不误删（R6）。持锁执行；清到即告警（B3）。
    ///
    /// 具体清什么由后端决定：JSON 清 `autosave.json.tmp.*`；sqlite 是 no-op
    /// （`-wal` / `-shm` 是库的活文件，
    /// 「清理」等同于破坏事务）。
    fn cleanup_stale_tmp(&mut self) {
        let outcome = self.with_run_lock(|session| -> Result<Vec<String>, String> {
            Ok(session.store.cleanup_stale(CLOSING_STALE_MS))
        });
        match outcome {
            Ok(warnings) => self.notices.extend(warnings),
            Err(message) => self.notices.push(message),
        }
    }
}

impl Session {
    /// 运行元信息（结算取 `mod` 标签；`started_at` 跨续局保留）。
    ///
    /// W5 半修（plan-r5 §2.7）：run.json 是人读元信息面（两进程并发时值相同或
    /// 极近，丢更新无实质后果），**不加锁**；但改走 tmp+rename 消「半截读」。
    pub(crate) fn write_meta(&self) {
        let started_at = read_meta(&self.meta_path)
            .and_then(|meta| meta["started_at"].as_u64())
            .unwrap_or_else(now_secs);
        // opener 是既成事实（先到先得、不可变）：重写 meta 时原样带回
        //（与 started_at 的保留逻辑同法）——否则开局 / reset / 热载后的
        // 任意一次 write_meta 都会把认领记录抹掉。
        let opener_actor =
            read_meta(&self.meta_path).and_then(|meta| meta.get("opener_actor").cloned());
        let mut meta = json!({
            "run_id": self.run_id,
            "mod": self.mod_label,
            "mod_signature": self.signature_label(),
            "started_at": started_at,
            "updated_at": now_secs(),
            // 存储后端**权威事实**（单源：只在这里写，与状态写入同批落盘）。
            // `--store` 仅是开局断言；旧局没有这个键 → 按产物推断为 json。
            "store": self.store.backend().as_str(),
        });
        if let Some(opener) = opener_actor {
            meta["opener_actor"] = opener;
        }
        let mut text = match serde_json::to_string_pretty(&meta) {
            Ok(text) => text,
            Err(err) => {
                eprintln!("[警告] 序列化 {} 失败：{err}", self.meta_path.display());
                return;
            }
        };
        text.push('\n');
        if let Err(err) = atomic_write(&self.meta_path, text.as_bytes()) {
            eprintln!("[警告] {err}");
        }
    }
}

// ============================================================================
// §reset 语义 + 锁心跳 通用回归（game-1102-1/r5 接手复核）
//
// 不变量（全部通用游戏内核语义，零 agent/Mod 场景假设）：
//   R1 无副作用 resume 不重置（读档成功根本不进 reset 路径）；
//   R2 显式新局仍重置，且恰好归档一次（旧行全在归档里，证据不丢）；
//   R3 续局失败且盘上状态**不可装载**（坏档 / 可解析但版本不符）→ 归档后新局，
//      绝不以全新状态混入旧代（不混代、旧 autosave 不被无归档覆盖）；
//   R4 续局失败但盘上状态**已可装载**（另一进程已救活）→ 重试读档 adopt，
//      不归档别人的行；
//   R5 持锁心跳：真实持锁才 bump 锁文件 mtime，未持锁绝不触碰别人的锁；
//   R6 drain 逐行消费一次（cursor 语义）且全程在真实锁内（心跳接线点）；
//   R7 closing 准入门禁：running 可写 → closing 拒写 → 解除恢复 → finalized 拒写。
// ============================================================================
mod dispatch;
mod inbox;
mod persist;
#[cfg(test)]
pub(crate) mod support;
#[cfg(test)]
mod tests_gates;
#[cfg(test)]
mod tests_reset;
