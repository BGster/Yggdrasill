use super::*;

// ============================================================================
// §参数 命令行解析（std-only 手写；用法错误 = 退出码 2）
// ============================================================================

/// 一次 inbox 写命令（宿主 CLI 侧）。
///
/// 只带「哪个旗标 + 原始载荷」：**记录形态与 kind 都由 Mod 的
/// `pack.json` → `inboxRoutes.<kind>.write` 声明**（`flag` + `shape`），宿主不枚举
/// kind、也不猜记录形态。本 Mod 没声明匹配的写命令 → 明确拒绝（不静默丢行）。
#[derive(Clone)]
pub(crate) struct InboxWrite {
    /// CLI 旗标名（不带 `--`）：路由表的查找键。
    pub(crate) flag: &'static str,
    /// 命令行给的原始载荷（形态由声明决定怎么落到记录里）。
    pub(crate) value: String,
}

/// 开局方式（r1 语义二分）：新局 = 指定 mod 与存档位置（自动生成 run-id）；
/// 读档 = 只指定存档位置（run-root 即身份），自动取最新存档点续跑。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OpenStyle {
    /// `egg new --mod <目录> [--in <runs 父目录>] [--store <b>]`：自动生成 run-id，
    /// 建 `<in>/<id>/` 并开局（`--force` 允许对已有目录重置）。
    New,
    /// `egg open <run-root> [--at g<代>]`：附着具体局目录；mod 从 run.json 现读。
    Open,
}

#[derive(Clone)]
pub(crate) struct Args {
    pub(crate) mod_dir: Option<PathBuf>,
    /// `--packages <目录>`：能力包根显式覆盖。语义零新增：优先级沿用
    /// `resolve_root` 既有 explicit > 环境变量 `$YGG_PACKAGES_ROOT` > 布局推导。
    /// 仅 new / open 装载路径消费；其余模式接受但忽略。
    pub(crate) packages: Option<PathBuf>,
    /// 开局方式（new/open；serve 与 --builtin-tool 模式为 None）。
    pub(crate) open_style: Option<OpenStyle>,
    /// 局目录（存档目录即身份）。new = `<in>/<自动 id>`（开局前为空，由
    /// `prepare_run_root` 填充）；open = 位置参数原样。
    pub(crate) run_root: PathBuf,
    /// new 的 runs 父目录（`--in`；缺省 `./runs`）。
    pub(crate) runs_parent: PathBuf,
    /// new 的显式重置许可（目录已有旧产物时）；open 语义天然不重置。
    pub(crate) force: bool,
    /// open 的回退存档点（`--at g<代>`；仅 sqlite 局可用）。
    pub(crate) at: Option<i64>,
    pub(crate) settle: bool,
    pub(crate) watch: bool,
    pub(crate) event: Option<String>,
    /// `--payload` 的值；`None` = 用 Mod 契约的 `agent.defaultPayload`（宿主不猜）。
    pub(crate) payload: Option<String>,
    /// 单发回执输出形态：缺省 = 人读文本；`--raw` = 单行 JSON 机器契约（与历史缺省
    /// 逐字节一致）。解析链 = `--raw` > 环境变量 `YGG_FORMAT`（text|json）> 缺省 text
    /// （解析见 [`resolve_output_format`]）。**仅 `--event` 单发路径生效**；其余模式
    /// 接受但忽略，daemon 协议面不受影响。
    pub(crate) output: OutputFormat,
    /// 交互模式推方式（`--drive`；缺省 stdin）。
    pub(crate) drive: Drive,
    /// 异步 inbox 操作（只往本局后端的 inbox 追加一条后退出，不读写 autosave）。
    pub(crate) inbox: Option<InboxWrite>,
    /// `--store`：**开局断言**（`run.json` 的 `store` 字段才是权威事实）。
    /// `None` = 未指定（旧局按产物推断，新局落 `ygg_store::DEFAULT_BACKEND` = sqlite）。
    pub(crate) store: Option<Backend>,
    /// `--wait-tools`：等本局在飞工具全部落地后 drain 一次 inbox 并打印回执。
    pub(crate) wait_tools: bool,
    /// `--tool-timeout <ms>`：`--wait-tools` 的总等待上限（缺省 120_000）。
    pub(crate) tool_timeout_ms: Option<u64>,
    /// 内部模式：工具 worker（`--run-tool-worker <call_id>`）。不对外宣传、不在 usage 里列出。
    pub(crate) run_tool_worker: Option<String>,
    /// `--artifact-put <artifacts/名>=<源文件>`：读源文件写进本局后端制品面。
    pub(crate) artifact_put: Option<String>,
    /// `--artifact-get <artifacts/名>`：把制品原文直出 stdout（缺失 = 退出 1）。
    pub(crate) artifact_get: Option<String>,
    /// `--builtin-tool <op>`：内置工具子命令（见 [`BuiltinTool`]）。
    /// 它的其它旗标（`--lane` / `--path` / …）由 `run_builtin_tool` 二次解析。
    pub(crate) builtin_tool: Option<BuiltinTool>,
    /// `--daemon`：常驻守护模式（stdin 逐行读 JSON 请求，stdout 逐行回 JSON）。
    /// 与所有「一次性动作」旗标互斥：它的整个生命周期都活在同一个 `Session` 上。
    pub(crate) daemon: bool,
    /// `--harness <auto|pi|claude-code|codex|generic>`：harness 适配器（**可选**）。
    /// `None` = 未指定 → `YGG_HARNESS` > `data/config.json` 的 `harness.adapter` > auto
    /// （解析见 [`resolve_harness_adapter`]）。它只影响「默认归档哪个会话库」，
    /// 不改变本局的任何状态机判定。
    pub(crate) harness: Option<String>,
    /// `--actor <id>`：提交者身份（Agent 工具句柄命名空间，与 `exec=` 同一空间）。
    /// 解析链：旗标 > 环境变量 `YGG_ACTOR`（daemon 模式再兜底请求字段，见
    /// [`daemon_dispatch`]）。`None` = 未携带身份（宿主不注入不认领 opener）。
    pub(crate) actor: Option<String>,
    /// `--actor-session <id>`：提交者的原生会话 id（不透明审计字段，永不参与判定）。
    /// 解析链与 `--actor` 完全对称：旗标 > `YGG_ACTOR_SESSION` > daemon 请求字段。
    pub(crate) actor_session: Option<String>,
    /// `serve`（子命令形态）：常驻只读观察台。观察单位 = runs-root（不绑定单局），
    /// GET-only 零写入；与其它全部模式互斥（[`parse_args`] 的互斥矩阵）。
    pub(crate) serve: bool,
    /// `--port <1..65535>`：serve 监听端口（缺省 [`serve::DEFAULT_PORT`]；
    /// 占用即失败退出 1，不自动递增——端口漂移会造成「两份不同事实」错觉）。
    pub(crate) port: Option<u16>,
    /// `--panel-dir <目录>`：内置两级列表页的盘上覆盖（开发期改 HTML 刷新即生效）。
    pub(crate) panel_dir: Option<PathBuf>,
    /// `--poll-ms <ms>`：watcher 变更检测周期（缺省 700，钟制 [200, 5000]）。
    pub(crate) poll_ms: Option<u64>,
}

pub(crate) fn parse_args() -> Args {
    let mut mod_dir: Option<PathBuf> = None;
    let mut packages: Option<PathBuf> = None;
    let mut positional: Vec<String> = Vec::new();
    let mut open_style: Option<OpenStyle> = None;
    let mut runs_parent: Option<PathBuf> = None;
    let mut force = false;
    let mut at: Option<i64> = None;
    let mut at_seen = false;
    let mut settle = false;
    let mut watch = false;
    let mut event: Option<String> = None;
    let mut payload: Option<String> = None;
    let mut raw = false;
    let mut drive = Drive::Stdin;
    let mut inbox: Option<InboxWrite> = None;
    let mut store: Option<Backend> = None;
    let mut wait_tools = false;
    let mut tool_timeout_ms: Option<u64> = None;
    let mut run_tool_worker: Option<String> = None;
    let mut artifact_put: Option<String> = None;
    let mut artifact_get: Option<String> = None;
    let mut builtin_tool: Option<BuiltinTool> = None;
    let mut daemon = false;
    let mut harness: Option<String> = None;
    let mut actor: Option<String> = None;
    let mut actor_session: Option<String> = None;
    let mut serve = false;
    let mut port: Option<u16> = None;
    let mut panel_dir: Option<PathBuf> = None;
    let mut poll_ms: Option<u64> = None;

    // 写 inbox 的七个参数共享同一套「只能给一个」的校验。
    fn set_inbox(inbox: &mut Option<InboxWrite>, flag: &'static str, value: String) {
        if inbox.is_some() {
            usage_error("一次只能提交一个 inbox 操作");
        }
        *inbox = Some(InboxWrite { flag, value });
    }

    let mut iter = std::env::args().skip(1);
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            // r1 语义二分的两个生命周期子命令（裸词，与 serve 同型）。
            "new" | "open" if positional.is_empty() => {
                if open_style.is_some() {
                    usage_error("new / open / serve 一次只能选一个");
                }
                open_style = Some(if argument == "new" {
                    OpenStyle::New
                } else {
                    OpenStyle::Open
                });
            }
            "--mod" => mod_dir = Some(PathBuf::from(next_value(&mut iter, "--mod"))),
            // ── 退役旗标（r1 fail-closed 可读拒绝；指向新用法，绝不静默映射）──
            "--run-id" => {
                let _ = next_value(&mut iter, "--run-id");
                usage_error(
                    "--run-id 已退役（run-root 即身份）：新局 `egg new --mod <目录> [--in <runs 父目录>]`；\
                     读档 `egg open <run-root>`",
                );
            }
            "--runs-root" => {
                let _ = next_value(&mut iter, "--runs-root");
                usage_error(
                    "--runs-root 已退役：new 用 `--in <runs 父目录>`（缺省 ./runs），open 直接给 run-root；\
                     serve 空根启动后用 `POST /api/roots {\"root\": \"<绝对路径>\"}` 注册观察根",
                );
            }
            "--resume" => usage_error(
                "--resume 已退役：`egg open <run-root>` 自动取最新存档点续跑；\
                 回退到指定存档点用 `egg open <run-root> --at g<代>`",
            ),
            "--in" => runs_parent = Some(PathBuf::from(next_value(&mut iter, "--in"))),
            "--packages" => packages = Some(PathBuf::from(next_value(&mut iter, "--packages"))),
            "--force" => force = true,
            "--at" => {
                at_seen = true;
                let text = next_value(&mut iter, "--at");
                at = Some(parse_at(&text).unwrap_or_else(|m| usage_error(&m)));
            }
            "--settle" => settle = true,
            "--watch" => watch = true,
            "--store" => {
                let text = next_value(&mut iter, "--store");
                store = match Backend::parse(&text) {
                    Some(backend) => Some(backend),
                    None => usage_error(&format!(
                        "未知的 --store 取值 `{text}`（只接受 {}；sqlite 是默认后端，json 是兼容后端）",
                        Backend::NAMES.join(" / ")
                    )),
                };
            }
            "--event" => event = Some(next_value(&mut iter, "--event")),
            "--payload" => payload = Some(next_value(&mut iter, "--payload")),
            // 单发回执输出单行 JSON（机器契约）。环境回退 YGG_FORMAT 在循环后统一解析。
            "--raw" => raw = true,
            "--actor" => {
                let raw = next_value(&mut iter, "--actor");
                actor = Some(
                    validate_identity_value("--actor", &raw, 64)
                        .unwrap_or_else(|m| usage_error(&m)),
                );
            }
            "--actor-session" => {
                let raw = next_value(&mut iter, "--actor-session");
                actor_session = Some(
                    validate_identity_value("--actor-session", &raw, 128)
                        .unwrap_or_else(|m| usage_error(&m)),
                );
            }
            "--drive" => {
                let text = next_value(&mut iter, "--drive");
                drive = match parse_drive(&text) {
                    Ok(drive) => drive,
                    Err(message) => usage_error(&message),
                };
            }
            "--requirement" => set_inbox(
                &mut inbox,
                "requirement",
                next_value(&mut iter, "--requirement"),
            ),
            "--amend" => set_inbox(&mut inbox, "amend", next_value(&mut iter, "--amend")),
            "--split" => set_inbox(&mut inbox, "split", next_value(&mut iter, "--split")),
            "--merge" => set_inbox(&mut inbox, "merge", next_value(&mut iter, "--merge")),
            "--conflict" => set_inbox(&mut inbox, "conflict", next_value(&mut iter, "--conflict")),
            "--resolve" => set_inbox(&mut inbox, "resolve", next_value(&mut iter, "--resolve")),
            "--proposal" => set_inbox(&mut inbox, "proposal", next_value(&mut iter, "--proposal")),
            "--tool-result" => set_inbox(
                &mut inbox,
                "tool-result",
                next_value(&mut iter, "--tool-result"),
            ),
            "--wait-tools" => wait_tools = true,
            "--tool-timeout" => {
                let text = next_value(&mut iter, "--tool-timeout");
                match text.trim().parse::<u64>() {
                    Ok(value) if value > 0 => tool_timeout_ms = Some(value),
                    _ => usage_error(&format!(
                        "--tool-timeout 需要正整数的毫秒数（当前是 `{text}`）"
                    )),
                }
            }
            "--run-tool-worker" => {
                run_tool_worker = Some(next_value(&mut iter, "--run-tool-worker"))
            }
            "--artifact-put" => artifact_put = Some(next_value(&mut iter, "--artifact-put")),
            "--artifact-get" => artifact_get = Some(next_value(&mut iter, "--artifact-get")),
            "--daemon" => daemon = true,
            "--harness" => {
                let text = next_value(&mut iter, "--harness");
                let trimmed = text.trim();
                if !HARNESS_ADAPTERS.contains(&trimmed) {
                    usage_error(&format!(
                        "未知的 --harness 取值 `{text}`（只接受 {}）",
                        HARNESS_ADAPTERS.join(" / ")
                    ));
                }
                harness = Some(trimmed.to_owned());
            }
            "--builtin-tool" => {
                let text = next_value(&mut iter, "--builtin-tool");
                builtin_tool = Some(BuiltinTool::parse(&text).unwrap_or_else(|| {
                    usage_error(&format!(
                        "未知的 --builtin-tool 取值 `{text}`（只接受 {}）",
                        BuiltinTool::NAMES.join(" / ")
                    ))
                }));
                // 该子命令自带的旗标由 `run_builtin_tool` 二次解析（op 级白名单）。
            }
            "serve" if positional.is_empty() => serve = true,
            "--port" => {
                let text = next_value(&mut iter, "--port");
                match text.trim().parse::<u16>() {
                    Ok(value) if value > 0 => port = Some(value),
                    _ => usage_error(&format!("--port 需要 1..=65535 的整数（当前是 `{text}`）")),
                }
            }
            "--panel-dir" => panel_dir = Some(PathBuf::from(next_value(&mut iter, "--panel-dir"))),
            "--poll-ms" => {
                let text = next_value(&mut iter, "--poll-ms");
                match text.trim().parse::<u64>() {
                    Ok(value) => poll_ms = Some(value),
                    _ => usage_error(&format!(
                        "--poll-ms 需要毫秒数（当前是 `{text}`；合法范围 200..=5000，越界钟制）"
                    )),
                }
            }
            "-h" | "--help" => usage(0),
            other => {
                if other.starts_with('-') {
                    // 内置工具模式下，未识别的旗标留给 op 级白名单。
                    if builtin_tool.is_none() {
                        usage_error(&format!("未知参数 `{other}`"));
                    }
                } else {
                    positional.push(other.to_owned());
                }
            }
        }
    }

    // 单发回执输出形态解析链（循环后统一裁决；与 --builtin-tool 冲突检查同层）。
    let output = resolve_output_format(raw, std::env::var("YGG_FORMAT").ok().as_deref())
        .unwrap_or_else(|message| usage_error(&message));

    // 模式互斥：new/open 与 serve 不能同现。
    if serve && open_style.is_some() {
        usage_error("serve 与 new / open 一次只能选一个");
    }

    // `--builtin-tool` 最先裁决：不需要 run-root 也不需要 --mod。
    if builtin_tool.is_some() {
        if settle
            || event.is_some()
            || wait_tools
            || inbox.is_some()
            || run_tool_worker.is_some()
            || artifact_put.is_some()
            || artifact_get.is_some()
            || daemon
            || watch
            || store.is_some()
            || serve
            || open_style.is_some()
            || at_seen
            || force
            || runs_parent.is_some()
        {
            usage_error(
                "--builtin-tool 不能与其它模式（new / open / --event / --settle / --at / --force / --in / --watch / --store / inbox / 制品旗标 / serve）同时使用",
            );
        }
        return Args {
            mod_dir,
            open_style: None,
            run_root: PathBuf::new(),
            runs_parent: PathBuf::from("runs"),
            force,
            at,
            settle,
            watch,
            event,
            payload,
            output,
            drive,
            inbox,
            store,
            wait_tools,
            tool_timeout_ms,
            run_tool_worker,
            artifact_put,
            artifact_get,
            builtin_tool,
            daemon,
            harness,
            actor,
            actor_session,
            serve,
            port,
            panel_dir,
            poll_ms,
            packages,
        };
    }

    // serve：常驻只读观察台，空根启动 + 运行时注册（POST /api/roots）。
    if serve {
        let flags = ServeFlags {
            settle,
            event: event.is_some(),
            wait_tools,
            inbox: inbox.is_some(),
            run_tool_worker: run_tool_worker.is_some(),
            artifact_put: artifact_put.is_some(),
            artifact_get: artifact_get.is_some(),
            daemon,
            watch,
            store: store.is_some(),
            payload: payload.is_some(),
            harness: harness.is_some(),
            tool_timeout: tool_timeout_ms.is_some(),
            drive_is_stdin: drive == Drive::Stdin,
            positional: !positional.is_empty(),
        };
        if let Some(message) = serve_usage_conflict(&flags) {
            usage_error(&message);
        }
        return Args {
            mod_dir: None,
            open_style: None,
            run_root: PathBuf::new(),
            runs_parent: PathBuf::from("runs"),
            force: false,
            at: None,
            settle: false,
            watch: false,
            event: None,
            payload: None,
            drive: Drive::Stdin,
            inbox: None,
            store: None,
            wait_tools: false,
            tool_timeout_ms: None,
            run_tool_worker: None,
            artifact_put: None,
            artifact_get: None,
            builtin_tool: None,
            daemon: false,
            harness: None,
            actor: None,
            actor_session: None,
            output,
            serve: true,
            port,
            panel_dir,
            poll_ms,
            packages,
        };
    }

    // new / open 之外没有开局路径。
    let Some(style) = open_style else {
        usage_error(
            "缺少 new / open / serve 子命令：新局 `egg new --mod <目录> [--in <runs 父目录>]`；\
             读档 `egg open <run-root>`；观察台 `egg serve [--port …]`",
        );
    };

    let run_root = match style {
        OpenStyle::New => {
            if !positional.is_empty() {
                usage_error("new 不接受位置参数（run-id 自动生成；存档位置用 --in 指定父目录）");
            }
            if at_seen {
                usage_error("--at 只能与 open 一起用（new 建的是全新代，没有可回退的存档点）");
            }
            if mod_dir.is_none() {
                usage_error("new 必须指定 --mod <mod目录>（新局要装载 Mod 契约）");
            }
            PathBuf::new() // 由 prepare_run_root 在开局前填充
        }
        OpenStyle::Open => {
            if positional.len() != 1 {
                usage_error("open 需要恰好一个位置参数：<run-root>（存档目录即身份）");
            }
            if force {
                usage_error("--force 只能与 new 一起用（open 永不重置——要重置请 new --force）");
            }
            if runs_parent.is_some() {
                usage_error("--in 只能与 new 一起用（open 的 run-root 已是完整局目录）");
            }
            PathBuf::from(&positional[0])
        }
    };

    // 守护模式是**常驻**模式：单发即退出的旗标都没有意义。
    if daemon
        && (settle
            || event.is_some()
            || wait_tools
            || inbox.is_some()
            || run_tool_worker.is_some()
            || artifact_put.is_some()
            || artifact_get.is_some()
            || watch)
    {
        usage_error(
            "--daemon 不能与 --event / --settle / --wait-tools / --watch / inbox 写命令 / 制品旗标 / --run-tool-worker 同时使用",
        );
    }
    // daemon 只挂 open（attach 语义）；new 的 daemon 入口也合法（先建局再常驻）。
    let artifact_mode = artifact_put.is_some() || artifact_get.is_some();
    if artifact_put.is_some() && artifact_get.is_some() {
        usage_error("--artifact-put 与 --artifact-get 不能同时使用");
    }
    if artifact_mode
        && (settle || event.is_some() || wait_tools || inbox.is_some() || run_tool_worker.is_some())
    {
        usage_error(
            "--artifact-put / --artifact-get 只操作本局制品面，不能与 --event / --settle / --wait-tools / inbox 操作同时使用",
        );
    }
    if settle && event.is_some() {
        usage_error("--settle 与 --event 不能同时使用");
    }
    if wait_tools && (settle || event.is_some()) {
        usage_error("--wait-tools 与 --event / --settle 不能同时使用");
    }
    if tool_timeout_ms.is_some() && !wait_tools {
        usage_error("--tool-timeout 只能与 --wait-tools 一起用");
    }
    if let Some(write) = &inbox {
        if settle || event.is_some() || wait_tools {
            usage_error(
                "inbox 操作（--requirement / --proposal / --tool-result / …）与 --event / --settle / --wait-tools 不能同时使用",
            );
        }
        if write.value.trim().is_empty() {
            usage_error(&format!("--{} 需要非空载荷", write.flag));
        }
    }
    if let Some(name) = &event {
        if name.trim().is_empty() {
            usage_error("--event 需要非空事件名");
        }
    }

    Args {
        mod_dir,
        open_style: Some(style),
        run_root,
        runs_parent: runs_parent.unwrap_or_else(|| PathBuf::from("runs")),
        force,
        at,
        settle,
        watch,
        event,
        payload,
        output,
        drive,
        inbox,
        store,
        wait_tools,
        tool_timeout_ms,
        run_tool_worker,
        artifact_put,
        artifact_get,
        builtin_tool,
        daemon,
        harness,
        actor,
        actor_session,
        serve,
        port,
        panel_dir,
        poll_ms,
        packages,
    }
}
