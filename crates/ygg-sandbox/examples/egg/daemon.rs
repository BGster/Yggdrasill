use super::*;

// ============================================================================
// §守护模式（`--daemon`）—— 常驻进程 + 逐行 JSON-RPC over stdio
//
// 为什么要有它：调度端每一步都要「派发 → 读摘要 → 看门禁」三次往返。单发模式
// 每次都重新装载 Mod + 读档，局一大就慢到不可用。守护模式把 `Session` 开一次，
// 之后每条请求只在内存里跑（热载 / inbox 消费仍按原语义每次派发前现查）。
//
// 协议（一行一个 JSON 对象）：
//   请求  {"id": <任意>, "op": "<动作>", ...}
//   应答  {"ok": true|false, "op": "<动作>", "id": <原样回显>, ...}
// `id` 可选；出现即原样回显（stdout 是单通道，调用方凭它配对）。
//
// **stdout 只有协议面**：任何 handler 里都不得 `println!`，人读文本一律 stderr。
//
// 动作表：
//   ping      探活（runId + 存储后端）
//   state     诊断信封（`state` 事件的只读回执，不推进状态）
//   summary   只读摘要（现派发 `summaryEvent`，**不**落 RunStore 步）
//   dispatch  派发一个事件（先热载 + 消费 inbox），回执附带最新 `summary`
//   vfs_put   写制品（端口与内核 `ygg.fs` 同时可见）
//   vfs_get   读制品原文     vfs_list  列制品
//   settle    结算（与独立 `--settle` 同一实现，会终结本局）
//   shutdown  退出守护进程
// ============================================================================

/// 写一行应答并 flush。
///
/// flush 是**承重**的：调用方在另一个进程里阻塞等这一行，攒缓冲就是互相死锁。
pub(crate) fn write_reply(out: &mut impl Write, reply: &Json) -> io::Result<()> {
    writeln!(out, "{reply}")?;
    out.flush()
}

/// 守护模式主循环：开局一次，之后逐行读请求、逐行回应答。
///
/// `--mod` 只为**新局**必需：已有本局时按 `run.json` 记录的 mod 路径解析（与
/// `--settle` / `--wait-tools` 同一条 `resolve_mod_dir`）。
pub(crate) fn run_daemon(args: &Args) -> i32 {
    let run_dir = args.run_root.clone();
    let mod_dir = match resolve_mod_dir(args, &run_dir) {
        Ok(dir) => dir,
        Err(message) => {
            eprintln!("error: {message}");
            return 1;
        }
    };
    // daemon 附着本局（r1：不再有「目录无产物自起新局」路径——新局必须先 `egg new`）。
    // open_style 原样传递：open → Attach（读档续跑）；daemon 挂在 new 下 = Fresh。
    let open_args = Args {
        mod_dir: Some(mod_dir),
        ..args.clone()
    };
    let mut session = match Session::open(&open_args) {
        Ok(session) => session,
        Err(message) => {
            eprintln!("error: {message}");
            return 1;
        }
    };

    let stdin = io::stdin();
    let mut out = io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(err) => {
                eprintln!("error: 读 stdin 失败：{err}");
                return 1;
            }
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // 坏 JSON 不退出：回一条错误应答让调用方自己决定（一行坏请求不该打死整局）。
        let request: Json = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(err) => {
                let reply = json!({
                    "ok": false,
                    "error": {
                        "code": "BAD_REQUEST",
                        "message": format!("请求行不是合法 JSON：{err}"),
                    },
                });
                if write_reply(&mut out, &reply).is_err() {
                    return 1;
                }
                continue;
            }
        };
        let (reply, quit) = handle_daemon_request(&mut session, &open_args, &request);
        if write_reply(&mut out, &reply).is_err() {
            return 1;
        }
        if quit {
            break;
        }
    }
    0
}

/// 错误应答（`op` 原样回填，便于调用方在单通道上配对）。
pub(crate) fn daemon_error(op: &str, message: &str) -> Json {
    json!({
        "ok": false,
        "op": op,
        "error": { "code": "DAEMON", "message": message },
    })
}

/// 一条守护请求的**完整**处理（纯内存 + 端口）：可单测，永不碰 stdout。
///
/// 返回 `(应答, 是否应退出)`：`shutdown` 是唯一把第二项置 true 的动作。
pub(crate) fn handle_daemon_request(
    session: &mut Session,
    args: &Args,
    req: &Json,
) -> (Json, bool) {
    let op = req["op"].as_str().unwrap_or("");
    let mut quit = false;
    let mut reply = match op {
        "ping" => json!({
            "ok": true,
            "op": "ping",
            "runId": session.run_id.clone(),
            "store": session.store.backend().as_str(),
        }),
        "state" => {
            let mut receipt = state_receipt(session, "state", Vec::new());
            receipt["op"] = json!("state");
            receipt
        }
        "summary" => {
            session.maybe_reload();
            let drain = session.drain_inbox();
            let mut reply = session.query_summary_json();
            reply["op"] = json!("summary");
            if drain.touched() {
                reply["inbox"] = drain.to_json();
            }
            reply
        }
        "dispatch" => daemon_dispatch(session, req),
        "vfs_put" => daemon_vfs_put(session, req),
        "vfs_get" => daemon_vfs_get(session, req),
        "vfs_list" => daemon_vfs_list(session),
        "settle" => daemon_settle(args),
        "shutdown" => {
            quit = true;
            json!({"ok": true, "op": "shutdown"})
        }
        other => daemon_error(
            other,
            &format!(
                "未知的 op `{other}`（只接受 ping / state / summary / dispatch / vfs_put / vfs_get / vfs_list / settle / shutdown）"
            ),
        ),
    };
    // `id` 只在请求给了的时候回填（不给就不凭空造一个，调用方靠 op 也能配对）。
    if let Some(id) = req.get("id") {
        reply["id"] = id.clone();
    }
    (reply, quit)
}

/// `dispatch`：热载 → 消费 inbox → 派发一个事件 → 附上最新 `summary`。
///
/// 一次往返同时给出两类信息（步回执 ⊆ 摘要）：调度端不必再发一条 `summary`，
/// 也就不会看到「派发之后、摘要之前」的中间态。
pub(crate) fn daemon_dispatch(session: &mut Session, req: &Json) -> Json {
    let Some(event) = req["event"].as_str() else {
        return daemon_error("dispatch", "`dispatch` 需要 event 字段（事件名）");
    };
    if event.trim().is_empty() {
        return daemon_error("dispatch", "`dispatch` 的 event 不能为空");
    }
    let payload = match req.get("payload") {
        None | Some(Json::Null) => session.contract.default_payload.clone(),
        // 字符串 = **字符串载荷**（要 JSON 引号包起来才是合法标量）；数字 / 布尔
        // 原样序列化。数组 / 对象不是合法标量（与 `--payload` 同一口径），拒绝。
        Some(Json::String(text)) => serde_json::to_string(text).unwrap_or_default(),
        Some(Json::Number(_)) | Some(Json::Bool(_)) => req["payload"].to_string(),
        Some(other) => {
            return daemon_error(
                "dispatch",
                &format!(
                    "`dispatch` 的 payload 必须是 JSON 标量（当前是 {}）",
                    json_type_name(other)
                ),
            )
        }
    };
    // 提交者身份（解析链与单发同序：旗标 > 环境 > 请求字段；前两层已折进
    // session.identity，请求字段只在前面都缺席时兑底）。请求字段形状非法
    // → 可读错误应答（守护模式没有“用法错误”，拒绝就是应答）。
    let identity = match merge_request_identity(&session.identity, req) {
        Ok(identity) => identity,
        Err(message) => return daemon_error("dispatch", &message),
    };
    // 与单发模式同序：先现查 Mod（热载），再把 inbox 存量排干净，最后派发本次事件。
    session.maybe_reload();
    let drain = session.drain_inbox();
    let mut receipt = session.dispatch_event_as(
        event,
        &payload,
        identity.actor.as_deref(),
        identity.session.as_deref(),
    );
    if drain.touched() {
        receipt["inbox"] = drain.to_json();
    }
    let summary_json = session.query_summary_json();
    if let Some(actions) = summary_json.get("harnessActions") {
        receipt["harnessActions"] = actions.clone();
    }
    receipt["summary"] = summary_json;
    receipt["op"] = json!("dispatch");
    receipt
}

/// daemon `op=dispatch` 请求字段的身份合并：请求里的 `actor` / `actor_session`
/// 只在本进程旗标与环境变量都缺席时兑底（与单发解析链同序）；形状非法 →
/// 可读错误（拒绝就是应答）。
pub(crate) fn merge_request_identity(
    base: &ActorIdentity,
    req: &Json,
) -> Result<ActorIdentity, String> {
    let actor = match base.actor.clone() {
        Some(actor) => Some(actor),
        None => match req["actor"].as_str() {
            Some(raw) => Some(validate_identity_value("actor", raw, 64)?),
            None => None,
        },
    };
    let session = match base.session.clone() {
        Some(session) => Some(session),
        None => match req["actor_session"].as_str() {
            Some(raw) => Some(validate_identity_value("actor_session", raw, 128)?),
            None => None,
        },
    };
    Ok(ActorIdentity { actor, session })
}

/// `vfs_put`：写制品到本局后端，并**同时**灌进内核的 `ygg.fs`。
///
/// 只落盘不灌 VFS 会让同一局里的 `fs.stat` 报「不存在」——单发模式下这个窗口只
/// 有几毫秒（下次开局即一致），守护进程会把它拉到整个局的长度。
pub(crate) fn daemon_vfs_put(session: &mut Session, req: &Json) -> Json {
    let Some(path) = req["path"].as_str() else {
        return daemon_error("vfs_put", "`vfs_put` 需要 path 字段（artifacts/<名>）");
    };
    let Some(content) = req["content"].as_str() else {
        return daemon_error("vfs_put", "`vfs_put` 需要 content 字段（UTF-8 文本）");
    };
    // 两道门都在端口内（宿主不重写一份）：路径形状 + 内容大小。
    if let Err(err) = ygg_store::validate_artifact_path(path) {
        return daemon_error("vfs_put", &err.to_string());
    }
    if let Err(err) = ygg_store::validate_artifact_content(content.as_bytes()) {
        return daemon_error("vfs_put", &err.to_string());
    }
    let bytes = content.as_bytes();
    let meta = session.with_run_lock(|session| {
        session
            .store
            .artifact_put(path, bytes, clock())
            .map_err(|err| err.to_string())
    });
    let meta = match meta {
        Ok(meta) => meta,
        Err(message) => return daemon_error("vfs_put", &message),
    };
    session.kernel.vfs_load(path, bytes, &meta.sha256);
    json!({
        "ok": true,
        "op": "vfs_put",
        "path": meta.path,
        "bytes": meta.bytes,
        "sha256": meta.sha256,
    })
}

/// `vfs_get`：制品原文（缺失 = 带 message 的失败应答，不是 panic）。
pub(crate) fn daemon_vfs_get(session: &mut Session, req: &Json) -> Json {
    let Some(path) = req["path"].as_str() else {
        return daemon_error("vfs_get", "`vfs_get` 需要 path 字段（artifacts/<名>）");
    };
    if let Err(err) = ygg_store::validate_artifact_path(path) {
        return daemon_error("vfs_get", &err.to_string());
    }
    match session.store.artifact_get(path) {
        Ok(Some(record)) => json!({
            "ok": true,
            "op": "vfs_get",
            "path": record.meta.path,
            "bytes": record.meta.bytes,
            "sha256": record.meta.sha256,
            "content": String::from_utf8_lossy(&record.content).into_owned(),
        }),
        Ok(None) => daemon_error(
            "vfs_get",
            &format!(
                "制品 `{path}` 不存在（本局 {}，后端 {}）",
                session.run_dir.display(),
                session.store.backend()
            ),
        ),
        Err(err) => daemon_error("vfs_get", &format!("读制品失败：{err}")),
    }
}

/// `vfs_list`：本局制品清单（升序；与 `ygg.fs.list` 同一份端口事实）。
pub(crate) fn daemon_vfs_list(session: &mut Session) -> Json {
    match session.store.artifact_list() {
        Ok(items) => json!({
            "ok": true,
            "op": "vfs_list",
            "artifacts": items
                .iter()
                .map(|meta| json!({
                    "path": meta.path,
                    "bytes": meta.bytes,
                    "sha256": meta.sha256,
                }))
                .collect::<Vec<_>>(),
        }),
        Err(err) => daemon_error("vfs_list", &format!("列制品失败：{err}")),
    }
}

/// `settle`：**唯一实现**是 `settle_report`（独立 `--settle` 走同一条路）。
///
/// `human=false` 是必须的：结算的人读摘要是 `println!`，在守护模式里会撕裂
/// stdout 的协议面。
pub(crate) fn daemon_settle(args: &Args) -> Json {
    let (code, report) = settle_report(args, false, false);
    match report {
        Some(report) => json!({
            "ok": report["ok"] == Json::Bool(true),
            "op": "settle",
            "exit": code,
            "settlement": report,
        }),
        None => daemon_error(
            "settle",
            "结算在写出报告之前失败（本局不可结算；原因见 stderr）",
        ),
    }
}

// ============================================================================
// §r1 单测：制品 CLI（--artifact-put / --artifact-get）+ VFS 持久化 + 内置工具
//
// 断言口径：**能直接比字节的地方就比字节**。内置工具的输出契约是「一行
// `k=v;k=v`」，所以这里断言 [`wire_line`] 的结果（它就是 `println!` 的那一行），
// 而不是去抓 stdout——抓 stdout 要么引入依赖，要么测一个自己搭的假管道。
// ============================================================================
#[cfg(test)]
mod tests_vfs;

// ============================================================================
// §r2 单测：守护模式（`--daemon`）的 JSON-RPC 面
//
// 断言口径：**只测纯内存的 `handle_daemon_request`**。它的契约就是「不碰
// stdout」（stdout 是守护进程的协议面），stdin 循环与 flush 由 `run_daemon`
// 的十几行承担：去抓管道只能得到「自己搭的假管道」的结论，且与 `--daemon`
// 的进程生命周期绑死。
//
// `summary` 里的 `jobs` / `gates` 是本模块**自造**的载荷：宿主只负责原样透传
// （形状由 Mod 声明），所以这里断言的是通道，不是 Mod 的业务语义——后者在
// `agent_seed_semantics` 的 `summary_exposes_structured_jobs_and_gates` 里。
// ============================================================================
#[cfg(test)]
mod tests_rpc;
