use super::*;

// ============================================================================
// §单发模式 Agent 主用：一次派发 → 单行 JSON 回执 → 退出
// ============================================================================

pub(crate) fn run_single(args: &Args) -> i32 {
    let mut session = match Session::open(args) {
        Ok(session) => session,
        Err(message) => {
            let event = args.event.clone().unwrap_or_default();
            return single_shot_error(args, &event, &message);
        }
    };
    // 缺 --event 时用 Mod 声明的入口事件（宿主不持有任何事件名）。
    let event = args
        .event
        .clone()
        .unwrap_or_else(|| session.contract.entry_event.clone());
    // 载荷缺省 = Mod 契约声明（宿主不持有任何 payload 字面量）。
    let payload = args
        .payload
        .clone()
        .unwrap_or_else(|| session.contract.default_payload.clone());

    // 单发每次派发前检查热载：Agent 侧改了 mod，下一条动作即生效。
    session.maybe_reload();

    // 派发前先消费 inbox：异步补入的需求不能因为「本步是 Agent 显式动作」而漏掉。
    // 被拒（局末 / 队列满 / 载荷）不阻碍本次派发，但会落 `inbox.remaining` + 警告。
    let drain = session.drain_inbox();

    // 单发是 Agent 脚本化的主通道：解析链（旗标 > 环境）已在 open 时折进
    // session.identity，这里原样交给 dispatch_event_as（声明路由注入 / 回显 /
    // opener 认领都在那单点）。未携带身份 = 旧语义逐字节。
    let identity = session.identity.clone();
    let mut receipt = session.dispatch_event_as(
        &event,
        &payload,
        identity.actor.as_deref(),
        identity.session.as_deref(),
    );
    if receipt["ok"] == Json::Bool(true) && receipt.get("harnessActions").is_none() {
        let actions = envelope_harness_actions(&receipt, &session.contract.meta_key);
        if !actions.is_empty() {
            receipt["harnessActions"] = json!(actions);
        }
    }
    if drain.touched() {
        receipt["inbox"] = drain.to_json();
    }
    // new 的稳定输出行（r1）：run-root 是后续一切 open / serve 注册的寻址事实，
    // text 回执末行固定 `run-root: <path>`；raw 信封加 `run_root` 字段（加性）。
    if args.open_style == Some(OpenStyle::New) {
        receipt["run_root"] = json!(session.run_dir.display().to_string());
    }
    if let Some(message) = &drain.error {
        eprintln!(
            "[警告] inbox 未能全部并入：{message}（剩 {} 行未消费）",
            drain.remaining
        );
    }
    // stderr 面（r5-plan §2.4 矩阵）：`drain.error` 是宿主运维错误，双模式都保留；
    // 警告与 `[HARNESS ACTION]` 在缺省 text 模式已并入渲染体（跳过 stderr 重复行），
    // 仅 Raw 模式（机器契约完整快照）照旧走 stderr。
    let text_mode = args.output == OutputFormat::Text;
    if !text_mode {
        // Raw：stdout 只留回执行（Agent 的解析面）；警告另走 stderr 便于人读。
        if let Some(warnings) = receipt["warnings"].as_array() {
            for warning in warnings.iter().filter_map(|value| value.as_str()) {
                eprintln!("[警告] {warning}");
            }
        }
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let env_adapter = std::env::var("YGG_HARNESS")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| harness_config_adapter(Some(&session.mod_dir)));
    if !text_mode
        && resolve_harness_adapter(args.harness.as_deref(), env_adapter.as_deref(), &cwd)
            == "generic"
    {
        for line in format_harness_action_lines(&receipt) {
            eprintln!("{line}");
        }
    }
    print!(
        "{}",
        render_receipt_output(&receipt, args.output, &session.contract.meta_key)
    );
    if args.open_style == Some(OpenStyle::New) && text_mode {
        println!("run-root: {}", session.run_dir.display());
    }
    if receipt["ok"] == Json::Bool(true) {
        0
    } else {
        1
    }
}

/// 退役专名拒绝（别名退役，原双向别名机制已删除）：形状=父目录名 `mods` +
/// 末段精确 `pi-agent-seed` 时给出可读理由——点名同父规范路径 `agent-seed`、
/// 宣布退役日期与「历史 run 不迁移、开新 run-id」。只认精确形状：
/// 前缀撞名（`-v2`/`-s`）与无 `mods` 父段的裸值一律不命中（装载期通用错误）。
pub(crate) fn retired_seed_name_reason(requested: &Path) -> Option<String> {
    if requested.file_name()?.to_str()? != "pi-agent-seed" {
        return None;
    }
    let parent = requested.parent()?;
    if parent.file_name()?.to_str()? != "mods" {
        return None;
    }
    Some(format!(
        "Mod 目录名 `pi-agent-seed` 已于 2026-11-12 退役（更名回 `agent-seed`，双向别名机制已删除）\
         ——规范路径：{}（同父目录）；历史 run.json 不迁移：请用新 run-id 开新局",
        parent.join("agent-seed").display()
    ))
}

/// 本局的 Mod 目录：`--mod` 优先，其次 run.json 里记录的本局 mod 路径。
///
/// 两者都没有 → 可读错误。契约是**必填面**：没有 Mod 目录就读不到声明，
/// 宿主不会猜任何默认值（fail-closed）。
/// 旧名 `pi-agent-seed` 已退役（别名退役）：请求路径缺席且命中退役形状时
/// 给专名拒绝（[`retired_seed_name_reason`]）；其余缺席维持既有缺路径错误。
pub(crate) fn resolve_mod_dir(args: &Args, run_dir: &Path) -> Result<PathBuf, String> {
    let requested = args
        .mod_dir
        .clone()
        .or_else(|| {
            read_meta(&run_dir.join(META_FILE))
                .and_then(|meta| meta["mod"].as_str().map(PathBuf::from))
        })
        .ok_or_else(|| {
            format!(
                "本局没有可用的 Mod 目录（缺 --mod，且 {} 里没有记录本局的 mod 路径）——\
                 契约（pack.json / config.json）必须现读，无法继续",
                run_dir.join(META_FILE).display()
            )
        })?;
    if !requested.exists() {
        if let Some(reason) = retired_seed_name_reason(&requested) {
            return Err(reason);
        }
    }
    Ok(requested)
}

/// 宿主 CLI 写命令 → `(路由表, kind)`：完全由 Mod 的 `inboxRoutes` 声明决定。
///
/// 未声明该写命令 → 可读拒绝（调用方报错退出，**不**静默写一行宿主猜的记录）。
/// 同时返回整张表：记录字段名（幂等键 / 预成形载荷）也是契约的一部分。
pub(crate) fn resolve_write_binding(
    args: &Args,
    run_dir: &Path,
    flag: &str,
) -> Result<(InboxRoutes, String), String> {
    let mod_dir = resolve_mod_dir(args, run_dir)?;
    let routes = InboxRoutes::load(&mod_dir)?;
    if routes.is_empty() {
        return Err(format!(
            "本 Mod 的 pack.json `inboxRoutes` 未声明任何 kind（不支持异步 inbox）——`--{flag}` 无处可写"
        ));
    }
    let Some(kind) = routes.write_binding(flag).map(|(kind, _)| kind.to_owned()) else {
        return Err(format!(
            "本 Mod 的 pack.json `inboxRoutes` 未声明 `--{flag}` 的写命令（需要某条路由带 \
             `\"write\": {{\"flag\": \"{flag}\", \"shape\": \"named|payload\"}}`；{CONTRACT_DOC}）"
        ));
    };
    Ok((routes, kind))
}

/// `--requirement`：只往本局后端的 inbox 追加一条，不读不写 autosave 后退出。
///
/// 这是「新需求随时能进」的真实异步入口：本局在另一个进程里跑时，这里只做一次
/// 单调追加（json = `OpenOptions::append`，sqlite = `INSERT`），不存在与运行
/// 中局的状态竞争。
pub(crate) fn run_inbox_append(args: &Args) -> i32 {
    let Some(write) = args.inbox.clone() else {
        return 2;
    };
    // 提交者身份（旗标 > 环境）：解析失败 = 用法错误（退出 2）；未携带身份
    // 只是不盖章不认领 opener，不改变写命令本身的任何语义（inbox 写路径零变化）。
    let identity = resolve_identity_or_usage(args);
    let run_dir = args.run_root.clone();
    if let Err(err) = fs::create_dir_all(&run_dir) {
        eprintln!("error: 创建 run 目录 {} 失败：{err}", run_dir.display());
        return 2;
    }
    // 写路径同样走端口：sqlite 局没有 inbox.jsonl 可追（追了也永远不被消费）。
    let mut store = match open_store_resolved(&run_dir, args.store) {
        Ok(store) => store,
        Err(message) => {
            eprintln!("error: {message}");
            return 2;
        }
    };
    // 记录形态与 kind 都来自 Mod 声明：契约读不到 / 没声明该写命令 → fail-closed
    // （明确拒绝 + 退出码 3，绝不静默写一行宿主猜出来的记录）。
    let (routes, kind) = match resolve_write_binding(args, &run_dir, write.flag) {
        Ok(binding) => binding,
        Err(message) => {
            eprintln!("error: {message}；本条未写入");
            return 3;
        }
    };
    let Some(route) = routes.get(&kind) else {
        eprintln!("error: 内部错误：写命令绑定的 kind `{kind}` 不在路由表里；本条未写入");
        return 3;
    };
    let Some(shape) = route.write.as_ref().map(|binding| binding.shape) else {
        eprintln!("error: 内部错误：write_binding 只返回带 write 的路由；本条未写入");
        return 3;
    };
    // 协议：写前登记。closing / finalized 一律**命令本身非零拒绝**——
    // 不允许出现「写得很成功但运行侧已无声跨过最后一次 drain」。
    let token = match begin_write(&run_dir) {
        Ok(token) => token,
        Err(WriteGate::Finalized) => {
            eprintln!(
                "error: 本局已 finalized（settlement 已定稿）——请开新局（换 --run-id）再提交；本条未写入"
            );
            return 3;
        }
        Err(WriteGate::Closing) => {
            eprintln!("error: 本局正在收口（closing）——本条未写入，稍后重试；已结束请开新局");
            return 3;
        }
        Err(WriteGate::Internal(message)) => {
            eprintln!("error: {message}");
            return 3;
        }
    };
    let path = run_dir.join(INBOX_FILE);
    // 记录字段名全部来自契约保留键：宿主不知道幂等键 / 载荷字段叫什么。
    let external_field = routes.external_id_field();
    let payload_field = routes.payload_field();
    let mut record = json!({
        "kind": kind,
        "created_at": now_secs(),
    });
    record[external_field] = json!(new_external_id(
        run_basename(&args.run_root),
        store.inbox_len().unwrap_or(0)
    ));
    match shape {
        // 命名段形态：「首段|余下全部」分别落到路由声明的**前两个**非幂等键段
        // （典型就是标题 / 描述；段名由 Mod 声明，宿主不认业务字段名）。
        WriteShape::Named => {
            let (first, rest) = split_requirement(&write.value);
            let mut named = route.named_fields(external_field);
            if let Some(field) = named.next() {
                record[field.as_str()] = json!(first);
            }
            if let Some(field) = named.next() {
                record[field.as_str()] = json!(rest);
            }
        }
        // 预成形 wire 形态：整串载荷进契约声明的载荷字段（宿主不解析、不重排）。
        WriteShape::Payload => record[payload_field] = json!(write.value.trim()),
    }
    // 提交者章（宿主保留字段，消费方 drain 读回后经 dispatch_event_as 注入声明
    // 路由；None 不写键——记录里没有“空身份”这种东西）。payload 形态的载荷
    // 不掺身份：身份只进 record 键，wire 段由派发侧注入。
    if let Some(actor) = &identity.actor {
        record["actor"] = json!(actor);
    }
    if let Some(session) = &identity.session {
        record["actor_session"] = json!(session);
    }
    let receipt = match store.inbox_push(&record, clock()) {
        Ok(()) => {
            // opener 认领（首条带身份的成功写，先到先得）：失败不阻断已落盘的
            // 行（run.json 是元信息面），降级为 stderr 警告。
            let mut claimed_opener = None;
            if let Some(actor) = &identity.actor {
                match claim_opener(&run_dir, actor, identity.session.as_deref()) {
                    Ok(_) => {}
                    Err(message) => {
                        eprintln!("[警告] opener 认领失败（不影响本条写入）：{message}")
                    }
                }
                claimed_opener = read_opener(&run_dir);
            }
            // 回执键 = 记录的契约字段名（幂等键 / 载荷 / 各命名段）：调用方不必猜。
            let mut receipt = json!({
                "ok": true,
                "inbox": path.display().to_string(),
                "kind": kind,
                // 提交者章回显（None → null）；仅携带身份的写命令回显 opener
                //（null 只在理论竞态失败方出现，锁串行下实际不会）。
                "actor": identity.actor,
                "actor_session": identity.session,
            });
            if identity.has_actor() {
                receipt["opener"] = claimed_opener.map(Json::String).unwrap_or(Json::Null);
            }
            receipt[external_field] = record[external_field].clone();
            receipt[payload_field] = record.get(payload_field).cloned().unwrap_or(Json::Null);
            for field in route.named_fields(external_field) {
                receipt[field.as_str()] = record.get(field.as_str()).cloned().unwrap_or(Json::Null);
            }
            receipt
        }
        Err(message) => {
            let text = message.to_string();
            eprintln!("error: {text}");
            json!({
                "ok": false,
                "inbox": path.display().to_string(),
                "kind": kind,
                "error": {"code": "INBOX", "message": text},
            })
        }
    };
    println!("{receipt}");
    // 行已落盘 → 摘除 token（closing 侧的「等 writers 归零」以此刻为界）。
    let pending = unconsumed_inbox_lines(store.as_ref());
    end_write(&run_dir, &token);
    if pending > 0 && receipt["ok"] == Json::Bool(true) {
        eprintln!("[提示] 还有 {pending} 条未消费 inbox 行（运行侧下一拍会消费）");
    }
    if receipt["ok"] == Json::Bool(true) {
        0
    } else {
        1
    }
}

/// 装载 / 续局阶段的失败也走回执形态（Agent 只需一条解析路径）。
pub(crate) fn single_shot_error(args: &Args, event: &str, message: &str) -> i32 {
    // 回执的 `autosave` 是「存档定位」而不是「一定是 JSON 文件」：本局已声明
    // 后端（`run.json` 的 `store`）为 sqlite 时必须回 sqlite 的定位串，否则
    // 工具会顺着这个字段去找一个根本不存在的 `autosave.json`。裁决顺序与
    // `ygg_store::resolve` 一致：`run.json` 权威 > `--store` 断言 > 产物探针 >
    // 默认后端。**产物探针不能省**：旧 JSON 局在这里没有 `store` 事实，跳过
    // 探针就会把它的错误回执指向一个不存在的 `run.sqlite`（默认后端已是 sqlite）。
    let run_dir = args.run_root.clone();
    let backend = read_meta(&run_dir.join(META_FILE))
        .and_then(|meta| meta["store"].as_str().map(str::to_owned))
        .and_then(|text| Backend::parse(&text))
        .or(args.store)
        .or_else(|| {
            let products = ygg_store::detect(&run_dir);
            if products.sqlite {
                Some(Backend::Sqlite)
            } else if products.json {
                Some(Backend::Json)
            } else {
                None
            }
        })
        .unwrap_or(ygg_store::DEFAULT_BACKEND);
    let autosave = match backend {
        Backend::Json => run_dir.join(AUTOSAVE_FILE).display().to_string(),
        Backend::Sqlite => format!("{}#latest", run_dir.join(SQLITE_FILE_NAME).display()),
    };
    let receipt = json!({
        "ok": false,
        "event": event,
        "narrative": [],
        "pendingChoice": Json::Null,
        "autosave": autosave,
        "error": {"code": "INTERNAL", "message": message},
    });
    if args.output == OutputFormat::Raw {
        eprintln!("error: {message}");
    }
    // 装载失败无 session：meta_key 传 ""，状态行自然缺席（r5-plan §2.3 打印点 B）。
    print!("{}", render_receipt_output(&receipt, args.output, ""));
    1
}

/// 退役专名拒绝（别名退役）的判定面：旧名 `pi-agent-seed` 不再翻译——精确形状缺席时
/// `resolve_mod_dir` 给点名规范路径的可读拒绝；其余缺席维持通用缺路径错误。
///
/// 为何要有这个模块：更名 agent-seed 的双向别名已退役（2026-11-12），历史 run.json 里的
/// `"mod": ".ygg/mods/pi-agent-seed"` 不迁移——宿主拒绝并指引开新 run-id，磁盘上的
/// `run.json` 一个字节都不动。本模块不装内核、不派发、不读真 Mod：临时目录里建空目录当 Mod。
#[cfg(test)]
mod tests;
