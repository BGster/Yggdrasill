//! serve 模式：常驻只读观察台（r8 · rev2 两级列表 + iframe 托管）。
//!
//! # 定位
//!
//! 继单发 `--event` / 交互 / `--daemon` 之后的第四模式：**加性**宿主执行面。
//! 不改既有单发 CLI 与 daemon 的任何行为；面板默认只读，gate/accept 类人类门禁
//! （`proposal.accept` / `gate.approve` / `coordinate.submit` / `verify.submit` /
//! `review.note`）在 serve 面上**不存在写路径**——路由表只登记 GET，非 GET 一律
//! `405`（物理不可绕过，测试锁 `routes_are_get_only`）。
//!
//! # 三级形态（P11 宿主零语义）
//!
//! ① runs 列表（egg 原生，**多观察根**全量枚举，行带根标识）；② 局内存档列表
//! （egg 原生，`ygg_store::observe` 只读观察面，跨根统一 run-id 寻址，撞名 409）；
//! ③ 存档视图 = **mod 自带 UI**（iframe 托管，
//! egg 不解析不渲染任何 mod 领域语义，只供存档数据 API 裸数据）。
//!
//! # 全局唯一实例（r9）
//!
//! serve 是**按端口的全局唯一实例**（缺省 127.0.0.1:7800）：启动前先做只读探针
//! （手写 `GET /api/runs`，1s 超时，零锁文件——端口即事实）——
//! - 自家实例且请求根 ⊆ 在役根 → 打印面板 URL 幂等复用（退出 0，nohup 脚本安全）；
//! - 自家实例但根不被覆盖 → 退出 1（在役实例不热改根——加控制通道即给只读面开口子）；
//! - 旧代单根实例（响应含 `runsRoot`）→ 退出 1 + 重启指引；
//! - 无自家实例 → 照常 bind；bind 失败（外姓占用）= 原样报错退出 1。
//! 观察根解析链：可重复 `--runs-root` > `~/.config/ygg/serve.json` > `./runs`
//! （拒绝文件系统爬扫——根集只来自显式声明，CONTEXT.md D21）。
//!
//! # 零内核（serve 纯度，比「观察者会话」更强）
//!
//! serve **不装载 ModBundle/kernel/mlua，不构造 `Session`**——上一版 §2.2 的五条
//! 「永不」由进程级结构承接：①不 create_dir_all/建库（观察面先探测后连接）；
//! ②不 write_meta（本模块无此调用面）；③不 mount 工具运行器（无工具面存在）；
//! ④不 drain/dispatch（无 dispatch 路径存在）；⑤只读 `save_get`/`save_list`
//! （经 [`ygg_store::observe`]）。物理锁：`serve_source_has_no_write_path`。
//!
//! # 线程模型（§6.2）
//!
//! 主线程 accept 循环（每连接一线程，并发封顶 64，超出 503）；watcher 线程轮询
//! runs-root 做纯变更检测（无 Session、无内核）；SSE 每连接在自身线程上
//! `wait_timeout` 等待广播。数据面即时读（文件 stat + 轻量 SQL，毫秒级），
//! 无发布快照、无陈旧窗口；唯一共享可变状态 = `version` 广播对。
//!
//! # 安全边界（§9）
//!
//! `127.0.0.1` 绑定常量（无旗标）；GET-only；无凭据面；解析健壮性（32 KiB 头
//! 上限 / 5s 超时 / 64 连接 / 路径白名单 / query 钳制 / run-id 语法门）；无 CORS；
//! `/panel` 双守卫（canonicalize + 前缀，拦 `..` 与 symlink 逃逸）+ nosniff。

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use include_dir::include_dir;
use serde_json::{json, Value as Json};
use ygg_store::observe;

use crate::Args;

// ============================================================================
// §常量（只有**不是契约**的东西；契约面见 docs/DESIGN.md §3.7）
// ============================================================================

/// 绑定地址**常量**（无旗标；单测断言字面量——漂到 0.0.0.0 必须红）。
const BIND_ADDR: &str = "127.0.0.1";
/// 缺省监听端口（占用即失败，不自动递增——端口漂移会造成「两份不同事实」错觉）。
const DEFAULT_PORT: u16 = 7800;
/// watcher 变更检测周期：缺省 700ms，钳制 [200, 5000]。
const DEFAULT_POLL_MS: u64 = 700;
const POLL_MS_MIN: u64 = 200;
const POLL_MS_MAX: u64 = 5_000;
/// SSE 保活注释间隔（`: ping`）。
const SSE_KEEPALIVE: Duration = Duration::from_secs(15);
/// 请求头字节上限（超过即 400；请求体读到即丢弃——GET-only 无 body 语义）。
const MAX_HEADER_BYTES: usize = 32 * 1024;
/// 读写超时（慢客户端/慢读者都会被关连接）。
const IO_TIMEOUT: Duration = Duration::from_secs(5);
/// 并发连接封顶（超出 503 + close）。
const MAX_CONNECTIONS: usize = 64;
/// 单次 `/api/runs` 响应的 run 行数封顶（超出截断 + `truncated:true`）。
const RUNS_CAP: usize = 200;
/// `/api/events` 的 limit 缺省与封顶。
const EVENT_LIMIT_DEFAULT: u32 = 200;
const EVENT_LIMIT_MAX: u32 = 500;
/// 制品原文的读侧封顶（超出 413；写入侧本就有 256 KiB 门，这是纵深防御）。
const ARTIFACT_HTTP_CAP: usize = 1024 * 1024;
/// 单实例探针超时（connect / 读写各 1s；探针是启动前的只读旁路，慢即视为无实例）。
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);
/// serve 根集配置文件（`$HOME` 相对；缺席 = 回退解析链下一层）。
const SERVE_CONFIG_REL: &str = ".config/ygg/serve.json";

/// 内置两级列表页资产（编译期嵌入；`--panel-dir` 可盘上覆盖）。
/// 嵌入对象 = 无构建三件（index.html/app.js/style.css，§8）。
static PANEL_ASSETS: include_dir::Dir = include_dir!("$CARGO_MANIFEST_DIR/assets/panel");

/// `/assets/<name>` 的白名单文件名（`index.html` 只经 `/` 提供）。
const ASSET_WHITELIST: [&str; 2] = ["app.js", "style.css"];

// ============================================================================
// §共享状态（唯一共享可变物：version 广播对）
// ============================================================================

/// watcher → SSE 连接的变更广播对（§6.1：通用变更帧，无数据载荷）。
struct Broadcaster {
    /// `(version, 最近一次变更的 run 名, 该局所属根)`；根级变更 = run 为 `None`、
    /// root 点名变化的根（多根下根级帧也带根；单根时代 root 恒 `Some`）。
    state: Mutex<(u64, Option<String>, Option<String>)>,
    cond: Condvar,
}

impl Broadcaster {
    fn new() -> Self {
        Self {
            state: Mutex::new((0, None, None)),
            cond: Condvar::new(),
        }
    }

    fn version(&self) -> u64 {
        self.state.lock().map_or(0, |state| state.0)
    }

    /// 变更登记：version 自增 + 唤醒全部 SSE 连接。
    fn bump(&self, run: Option<&str>, root: Option<&Path>) {
        if let Ok(mut state) = self.state.lock() {
            state.0 += 1;
            state.1 = run.map(str::to_owned);
            state.2 = root.map(|path| path.display().to_string());
            self.cond.notify_all();
        }
    }

    /// 等待 `seen` 之后的新版本；超时返回 `None`（调用方写 `: ping`）。
    fn wait_since(
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

/// serve 进程的共享只读配置 + 连接计数。
struct ServeShared {
    /// 观察根（**canonicalize 后的绝对形态**：比较、行级 root 标识与探针比对
    /// 的唯一事实形态；旗标/配置里的相对或 `~` 形态不进共享态）。
    roots: Vec<PathBuf>,
    /// 内置页的盘上覆盖（`--panel-dir`；开发期改 HTML 刷新即生效）。
    panel_dir: Option<PathBuf>,
    broadcaster: Broadcaster,
    connections: AtomicUsize,
}

// ============================================================================
// §根集解析与单实例探针（r9；纯函数可测）
// ============================================================================

/// 在役实例判定（[`probe_live_instance`] 的产物；纯数据可测）。
#[derive(Debug)]
enum InstanceVerdict {
    /// 自家实例且请求根 ⊆ 在役根 → 幂等复用（退出 0）。
    Reuse,
    /// 自家实例但请求根不被覆盖 → 拒绝（退出 1 + 处置指引；不热改根）。
    RootsNotCovered {
        served: Vec<PathBuf>,
        requested: Vec<PathBuf>,
    },
    /// 旧代单根实例（`/api/runs` 响应含 `runsRoot`）→ 拒绝（退出 1 + 重启指引）。
    LegacyInstance,
    /// 端口上没有自家实例（连接失败 / 非 200 / 非 egg 形状）→ 照常 bind。
    NoInstance,
}

/// serve 根集配置：`{"roots": ["/abs/…", …]}`。fail-closed：畸形 JSON / 未知键 /
/// 非字符串数组 / 相对路径 → Err（调用方退出 2——「相对路径相对于谁」是无解的
/// 猜测，拒绝优于猜测）。`~` / `~/` 前缀按传入的 home 展开。
fn parse_serve_config(text: &str, home: Option<&Path>) -> Result<Vec<PathBuf>, String> {
    let value: Json = serde_json::from_str(text).map_err(|err| format!("JSON 畸形：{err}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "顶层必须是 JSON 对象".to_owned())?;
    for key in object.keys() {
        if key != "roots" {
            return Err(format!("未知键 `{key}`（只接受 `roots`）"));
        }
    }
    let array = object
        .get("roots")
        .ok_or_else(|| "缺少 `roots` 数组".to_owned())?
        .as_array()
        .ok_or_else(|| "`roots` 必须是字符串数组".to_owned())?;
    if array.is_empty() {
        return Err("`roots` 不能为空数组（要回退 `./runs` 缺省请删除配置文件）".to_owned());
    }
    let mut roots = Vec::with_capacity(array.len());
    for item in array {
        let raw = item
            .as_str()
            .ok_or_else(|| "`roots` 必须是字符串数组".to_owned())?;
        let path = if raw == "~" || raw.starts_with("~/") {
            let home = home.ok_or_else(|| format!("roots 含 `~` 前缀但 $HOME 不可用：`{raw}`"))?;
            match raw.strip_prefix("~/") {
                Some(rest) if !rest.is_empty() => home.join(rest),
                _ => home.to_path_buf(),
            }
        } else {
            PathBuf::from(raw)
        };
        if !path.is_absolute() {
            return Err(format!(
                "roots 含相对路径 `{raw}`（serve 配置只接受绝对路径——相对路径“相对于谁”是无解的猜测，拒绝优于猜测）"
            ));
        }
        roots.push(path);
    }
    Ok(roots)
}

/// 根集解析链：可重复 `--runs-root` 旗标 > serve 配置 > `./runs`（cwd 相对）。
/// 拒绝文件系统爬扫：根集只来自显式声明（D21）。旗标保持逐字原样（既有 cwd
/// 相对语义）；`./runs` 缺省解析为 cwd 绝对形态（全局实例的启动信息可读）。
fn resolve_roots(
    flags: &[PathBuf],
    config_text: Option<&str>,
    home: Option<&Path>,
    cwd: &Path,
) -> Result<Vec<PathBuf>, String> {
    if !flags.is_empty() {
        return Ok(flags.to_vec());
    }
    match config_text {
        Some(text) => parse_serve_config(text, home),
        None => Ok(vec![cwd.join("runs")]),
    }
}

/// canonicalize + 去重（防同一目录双别名产生双行 + watcher 双跳变）；顺序 =
/// 首次出现序。调用方已逐根 is_dir 校验，canonicalize 失败时按 cwd 绝对化兜底。
fn canonicalize_roots(roots: &[PathBuf], cwd: &Path) -> Vec<PathBuf> {
    let mut seen: Vec<PathBuf> = Vec::new();
    for root in roots {
        let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| cwd.join(root));
        if !seen.contains(&canonical) {
            seen.push(canonical);
        }
    }
    seen
}

/// 请求根 ⊆ 在役根（双方均已 canonicalize；精确路径相等）。
fn roots_cover(requested: &[PathBuf], served: &[PathBuf]) -> bool {
    requested.iter().all(|root| served.contains(root))
}

/// 读 serve 配置文件（缺席 = `Ok(None)`；读失败 fail-closed 上抛 → 退出 2）。
fn read_serve_config(home: Option<&Path>) -> Result<Option<String>, String> {
    let Some(home) = home else {
        return Ok(None);
    };
    let path = home.join(SERVE_CONFIG_REL);
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!("读取 {} 失败：{err}", path.display())),
    }
}

/// 单实例探针（**纯读**：手写 `GET /api/runs`，1s 超时，零锁文件——端口即事实）。
/// 判定规则见模块头「全局唯一实例」节；探针↔bind 之间的竞态由 bind 失败路径兜住。
fn probe_live_instance(port: u16, requested: &[PathBuf]) -> InstanceVerdict {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, PROBE_TIMEOUT) else {
        return InstanceVerdict::NoInstance;
    };
    let _ = stream.set_read_timeout(Some(PROBE_TIMEOUT));
    let _ = stream.set_write_timeout(Some(PROBE_TIMEOUT));
    let request = "GET /api/runs HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    if stream.write_all(request.as_bytes()).is_err() {
        return InstanceVerdict::NoInstance;
    }
    let mut raw = Vec::new();
    if stream.read_to_end(&mut raw).is_err() {
        return InstanceVerdict::NoInstance;
    }
    let Some(split) = raw.windows(4).position(|window| window == b"\r\n\r\n") else {
        return InstanceVerdict::NoInstance;
    };
    let head = String::from_utf8_lossy(&raw[..split]);
    let status: u16 = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    if status != 200 {
        return InstanceVerdict::NoInstance;
    }
    let Ok(body) = serde_json::from_slice::<Json>(&raw[split + 4..]) else {
        return InstanceVerdict::NoInstance;
    };
    // egg serve 的正面证据：`runs` 数组（缺它 = 外姓服务，让 bind 失败路径说话）。
    if body.get("runs").and_then(Json::as_array).is_none() {
        return InstanceVerdict::NoInstance;
    }
    // 旧代单根实例：响应携带单数 `runsRoot` → 拒绝 + 重启指引（升级窗口期一次性路径）。
    if body.get("runsRoot").is_some() {
        return InstanceVerdict::LegacyInstance;
    }
    let served: Vec<PathBuf> = body
        .get("roots")
        .and_then(Json::as_array)
        .map(|roots| {
            roots
                .iter()
                .filter_map(|value| value.as_str().map(PathBuf::from))
                .collect()
        })
        .unwrap_or_default();
    if roots_cover(requested, &served) {
        InstanceVerdict::Reuse
    } else {
        InstanceVerdict::RootsNotCovered {
            served,
            requested: requested.to_vec(),
        }
    }
}

// ============================================================================
// §入口与线程编排
// ============================================================================

/// serve 模式主入口（`egg [--runs-root <目录> …] serve …`）。返回进程退出码。
pub(crate) fn run_serve(args: &Args) -> i32 {
    let home = super::home_dir();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    // 根集解析链（旗标 > 配置 > ./runs）。配置面畸形 = 用法错误退出 2（fail-closed）。
    let config_text = match read_serve_config(home.as_deref()) {
        Ok(config_text) => config_text,
        Err(message) => {
            eprintln!("error: serve 配置读取失败：{message}");
            return 2;
        }
    };
    let requested = match resolve_roots(
        &args.serve_runs_roots,
        config_text.as_deref(),
        home.as_deref(),
        &cwd,
    ) {
        Ok(roots) => roots,
        Err(message) => {
            eprintln!("error: serve 根集配置无效：{message}");
            return 2;
        }
    };
    // 逐根必须已存在：观察台**零创建**（不 create_dir_all，也不爬扫文件系统补根）。
    for root in &requested {
        if !root.is_dir() {
            eprintln!(
                "error: runs-root {} 不存在或不是目录（serve 只观察既有 runs-root，不自动创建；\
                 根集解析链：--runs-root 旗标 > ~/.config/ygg/serve.json > ./runs）",
                root.display()
            );
            return 1;
        }
    }
    let roots = canonicalize_roots(&requested, &cwd);
    // 单实例探针（先于 bind；探针↔bind 之间的竞态由 bind 失败路径兜住）。
    let port = args.port.unwrap_or(DEFAULT_PORT);
    match probe_live_instance(port, &roots) {
        InstanceVerdict::Reuse => {
            println!(
                "serve: 已有全局唯一实例在役，复用 http://{BIND_ADDR}:{port}（roots: {}）；\
                 本次调用幂等退出 0",
                join_roots(&roots)
            );
            return 0;
        }
        InstanceVerdict::RootsNotCovered { served, requested } => {
            eprintln!(
                "error: 已有 serve 实例在役 http://{BIND_ADDR}:{port}，但其根不覆盖本次请求（serve 不热改根）：\n  在役 roots: {}\n  请求 roots: {}\n  处置：停掉在役实例后重启（pkill -f 'egg.*serve'），或把缺的根写进 ~/.config/ygg/serve.json",
                join_roots(&served),
                join_roots(&requested)
            );
            return 1;
        }
        InstanceVerdict::LegacyInstance => {
            eprintln!(
                "error: 端口 {port} 上是旧版单根 serve 实例（响应含 runsRoot）。\
                 请停掉旧实例（Ctrl-C / pkill -f 'egg.*serve'）后重启以启用多根观察"
            );
            return 1;
        }
        InstanceVerdict::NoInstance => {}
    }
    let listener = match TcpListener::bind((BIND_ADDR, port)) {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!(
                "error: 绑定 {BIND_ADDR}:{port} 失败：{err}（端口被占用即失败退出，不自动递增）"
            );
            return 1;
        }
    };
    let bound = listener.local_addr().map_or(port, |addr| addr.port());
    let shared = Arc::new(ServeShared {
        roots,
        panel_dir: args.panel_dir.clone(),
        broadcaster: Broadcaster::new(),
        connections: AtomicUsize::new(0),
    });
    eprintln!(
        "serve: 只读观察台已启动 http://{BIND_ADDR}:{bound}（roots: {}；\
         GET-only 零写入；两级列表 + mod 面板 iframe 托管；Ctrl-C 退出）",
        join_roots(&shared.roots)
    );
    let poll_ms = clamp_poll_ms(args.poll_ms);
    let shutdown = Arc::new(AtomicBool::new(false));
    {
        let shared = Arc::clone(&shared);
        let shutdown = Arc::clone(&shutdown);
        std::thread::spawn(move || watcher_loop(&shared, &shutdown, poll_ms));
    }
    serve_accept_loop(listener, &shared, &shutdown, SSE_KEEPALIVE);
    0
}

/// `--poll-ms` 钳制（§2.3：缺省 700，[200, 5000]）。
fn clamp_poll_ms(poll_ms: Option<u64>) -> u64 {
    poll_ms
        .unwrap_or(DEFAULT_POLL_MS)
        .clamp(POLL_MS_MIN, POLL_MS_MAX)
}

/// 人读 roots 列表（启动信息 / 复用文案 / 报错共用）。
fn join_roots(roots: &[PathBuf]) -> String {
    roots
        .iter()
        .map(|root| root.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// 测试/装配共用：绑定一个（测试用 port 0 由 OS 分配）监听器。
#[cfg(test)]
fn bind_serve(
    roots: Vec<PathBuf>,
    panel_dir: Option<PathBuf>,
) -> std::io::Result<(TcpListener, Arc<ServeShared>)> {
    let listener = TcpListener::bind((BIND_ADDR, 0))?;
    let shared = Arc::new(ServeShared {
        roots,
        panel_dir,
        broadcaster: Broadcaster::new(),
        connections: AtomicUsize::new(0),
    });
    Ok((listener, shared))
}

/// accept 循环：每连接一线程，并发封顶 64（超出 503 + close）。
fn serve_accept_loop(
    listener: TcpListener,
    shared: &Arc<ServeShared>,
    shutdown: &AtomicBool,
    keepalive: Duration,
) {
    for stream in listener.incoming() {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }
        let Ok(mut stream) = stream else {
            continue;
        };
        let current = shared.connections.fetch_add(1, Ordering::SeqCst);
        if current >= MAX_CONNECTIONS {
            shared.connections.fetch_sub(1, Ordering::SeqCst);
            let _ = stream.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            continue;
        }
        let shared = Arc::clone(shared);
        std::thread::spawn(move || {
            let _guard = ConnectionGuard(shared.clone());
            let _ = handle_connection(&mut stream, &shared, keepalive);
        });
    }
}

/// 连接计数归还（Drop 保证异常路径也不漏计数）。
struct ConnectionGuard(Arc<ServeShared>);

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.0.connections.fetch_sub(1, Ordering::SeqCst);
    }
}

// ============================================================================
// §watcher：纯变更检测器（§6.1——无内核无 Session，轮询 → version 自增 → 广播）
// ============================================================================

/// 单局指纹：run.json / coordination.json 的 stat + 存档探针。
#[derive(Default, PartialEq, Eq)]
struct RunFingerprint {
    run_stat: Option<(u128, u64)>,
    coord_stat: Option<(u128, u64)>,
    probe: observe::SaveProbe,
}

fn stat_stamp(path: &Path) -> Option<(u128, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some((modified, meta.len()))
}

fn fingerprint(run_dir: &Path) -> RunFingerprint {
    RunFingerprint {
        run_stat: stat_stamp(&run_dir.join(crate::META_FILE)),
        coord_stat: stat_stamp(&run_dir.join(super::COORDINATION_FILE)),
        // 后端裁决失败（混代等）→ 空探针：观察面如实显示空，不掩盖也不炸线程。
        probe: resolve_backend(run_dir)
            .ok()
            .and_then(|backend| observe::save_probe(run_dir, backend).ok())
            .unwrap_or_default(),
    }
}

fn watcher_loop(shared: &ServeShared, shutdown: &AtomicBool, poll_ms: u64) {
    // 指纹键 = (根, 局名) 对：跨根同名局各自独立跟踪（r9 多根化）。
    let mut known: BTreeMap<(PathBuf, String), RunFingerprint> = BTreeMap::new();
    while !shutdown.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(poll_ms));
        for root in &shared.roots {
            let mut names: Vec<String> = std::fs::read_dir(root)
                .map(|entries| {
                    entries
                        .flatten()
                        .filter(|entry| {
                            entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false)
                        })
                        .filter_map(|entry| entry.file_name().into_string().ok())
                        .collect()
                })
                .unwrap_or_default();
            names.sort();
            let previous: Vec<String> = known
                .keys()
                .filter(|(known_root, _)| known_root == root)
                .map(|(_, name)| name.clone())
                .collect();
            if names != previous {
                // 根内名单变化（新局出现 / 旧局被删 / 根消失）→ 根级变更帧（run=null，
                // root 点名变化的根）。消失的根 read_dir 失败 = 空名单，既有容忍不变。
                known.retain(|(known_root, name), _| known_root != root || names.contains(name));
                shared.broadcaster.bump(None, Some(root.as_path()));
            }
            for name in &names {
                let key = (root.clone(), name.clone());
                let fp = fingerprint(&root.join(name));
                if known.get(&key) != Some(&fp) {
                    known.insert(key, fp);
                    shared.broadcaster.bump(Some(name), Some(root.as_path()));
                }
            }
        }
    }
}

// ============================================================================
// §HTTP 子集（GET-only + close 定界；std 手写，零新依赖）
// ============================================================================

struct Request {
    method: String,
    path: String,
    query: String,
}

/// 路由结果：普通响应或 SSE 流（SSE 在连接线程上长驻）。
enum Outcome {
    Http(Response),
    Sse,
}

struct Response {
    status: u16,
    reason: &'static str,
    content_type: &'static str,
    /// 额外响应头（如存档数据 API 的 `X-Ygg-*`）。
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
}

impl Response {
    fn json(status: u16, value: &Json) -> Self {
        Self {
            status,
            reason: reason_phrase(status),
            content_type: "application/json",
            headers: Vec::new(),
            body: serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec()),
        }
    }
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "OK",
    }
}

/// 统一错误形（§3.4）：`{"ok":false,"error":{"code":…,"message":…}}`。
fn error_response(status: u16, code: &str, message: &str) -> Response {
    Response::json(
        status,
        &json!({"ok": false, "error": {"code": code, "message": message}}),
    )
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// 读一个请求（头封顶 32 KiB / 超时 5s）。返回 `None` = 畸形 / 空连接。
fn read_request(stream: &mut TcpStream) -> std::io::Result<Option<Request>> {
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let mut buf: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        if buf.len() > MAX_HEADER_BYTES {
            return Ok(None);
        }
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            return Ok(None); // 客户端直接断开
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let head = String::from_utf8_lossy(&buf);
    Ok(parse_request_from_head(&head))
}

/// 请求行解析（网络读取与测试面共用的同一入口）。
fn parse_request_from_head(head: &str) -> Option<Request> {
    let request_line = head.split("\r\n").next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_owned();
    let target = parts.next()?.to_owned();
    let version = parts.next().unwrap_or("");
    if !version.starts_with("HTTP/") || !target.starts_with('/') {
        return None;
    }
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path.to_owned(), query.to_owned()),
        None => (target, String::new()),
    };
    Some(Request {
        method,
        path,
        query,
    })
}

/// 百分号解码（路径/制品 rel 用；查询数值面不需要）。
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if let Some(hex) = bytes.get(index + 1..index + 3) {
                if let Ok(value) = u8::from_str_radix(&String::from_utf8_lossy(hex), 16) {
                    out.push(value);
                    index += 3;
                    continue;
                }
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query.split('&').find_map(|pair| {
        pair.split_once('=')
            .filter(|(name, _)| *name == key)
            .map(|(_, value)| value)
    })
}

/// 数值查询参数：缺省 / 非法 / 钳制（§9.4 query 数值钳制）。
fn query_usize(query: &str, key: &str, default: usize, max: usize) -> Result<usize, Response> {
    match query_param(query, key) {
        None | Some("") => Ok(default),
        Some(raw) => raw
            .parse::<usize>()
            .map(|value| value.min(max))
            .map_err(|_| {
                error_response(
                    400,
                    "bad_query",
                    &format!("查询参数 {key} 需要非负整数（当前是 `{raw}`）"),
                )
            }),
    }
}

/// 存档 id 语法门：`latest` | 十进制 id（否则 400）。
fn valid_save_id(sid: &str) -> bool {
    sid == "latest" || (!sid.is_empty() && sid.bytes().all(|byte| byte.is_ascii_digit()))
}

fn handle_connection(
    stream: &mut TcpStream,
    shared: &ServeShared,
    keepalive: Duration,
) -> std::io::Result<()> {
    let Some(request) = read_request(stream)? else {
        let _ = stream.write_all(
            b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        return Ok(());
    };
    match route(shared, &request.method, &request.path, &request.query) {
        Outcome::Sse => {
            let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n";
            stream.write_all(head.as_bytes())?;
            stream.flush()?;
            sse_loop(stream, &shared.broadcaster, keepalive);
            Ok(())
        }
        Outcome::Http(response) => write_response(stream, response),
    }
}

fn write_response(stream: &mut TcpStream, response: Response) -> std::io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n",
        response.status, response.reason, response.content_type, response.body.len()
    );
    for (name, value) in &response.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(&response.body)?;
    stream.flush()
}

/// SSE 长驻循环：版本变更 → 一行变更帧；静置超时 → `: ping` 保活。
/// 写失败 = 客户端断开 → 退出（线程随之结束，不泄漏）。
fn sse_loop(stream: &mut TcpStream, broadcaster: &Broadcaster, keepalive: Duration) {
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

fn write_sse(stream: &mut TcpStream, frame: &str) -> std::io::Result<()> {
    stream.write_all(frame.as_bytes())?;
    stream.flush()
}

// ============================================================================
// §路由（表里只有**结构名**，无一个 mod 事件名——P2/P3 的落点）
// ============================================================================

fn route(shared: &ServeShared, method: &str, raw_path: &str, query: &str) -> Outcome {
    // GET-only 是**物理**边界：非 GET 在路由表之前就被拒（写端点根本不存在）。
    if method != "GET" {
        return Outcome::Http(error_response(
            405,
            "method_not_allowed",
            "只读观察台只支持 GET（写路径只经 CLI/daemon；gate/accept 类人类门禁不在线上）",
        ));
    }
    let path = percent_decode(raw_path);
    let segments: Vec<&str> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let outcome = match segments.as_slice() {
        [] => builtin_asset(shared, "index.html"),
        ["assets", name] => builtin_asset(shared, name),
        ["api", "runs"] => handle_runs(shared),
        ["api", "stream"] => return Outcome::Sse,
        ["api", "runs", run_id, "saves"] => handle_saves(shared, run_id, query),
        ["api", "runs", run_id, "saves", sid] => handle_save_blob(shared, run_id, sid),
        ["api", "runs", run_id, "events"] => handle_events(shared, run_id, query),
        ["api", "runs", run_id, "artifacts"] => handle_artifacts(shared, run_id),
        ["api", "runs", run_id, "artifacts", rest @ ..] => {
            handle_artifact_content(shared, run_id, &rest.join("/"))
        }
        ["panel", run_id] => serve_mod_panel(shared, run_id, ""),
        ["panel", run_id, rest @ ..] => serve_mod_panel(shared, run_id, &rest.join("/")),
        _ => error_response(404, "not_found", "未知路径"),
    };
    Outcome::Http(outcome)
}

/// run-id 路径段语法门（拒 `/`、`\`、`..` 等；在触盘之前）。
fn check_run_id(run_id: &str) -> Result<(), Response> {
    if super::is_valid_run_id(run_id) {
        Ok(())
    } else {
        Err(error_response(
            400,
            "bad_run_id",
            &format!("run-id `{run_id}` 含非法字符（只允许字母数字与 `-` `_` `.`）"),
        ))
    }
}

/// 跨根定位一局：0 命中 → NotFound；1 命中 → Found；>1 命中 → Ambiguous（409，
/// 响亮上报，不静默任选一份——serve 不代调用方选择事实）。
enum RunLocation {
    NotFound,
    Found(PathBuf),
    Ambiguous(Vec<PathBuf>),
}

/// run 目录是否已是「局」（run.json 存在）；半壳目录不是局。
fn is_run_dir(root: &Path, run_id: &str) -> Option<PathBuf> {
    let run_dir = root.join(run_id);
    run_dir.join(crate::META_FILE).is_file().then_some(run_dir)
}

/// 统一跨根定位（数据端点与面板托管共用同一条寻址事实）。
fn locate_run(shared: &ServeShared, run_id: &str) -> RunLocation {
    let mut hits: Vec<PathBuf> = shared
        .roots
        .iter()
        .filter_map(|root| is_run_dir(root, run_id))
        .collect();
    match hits.len() {
        0 => RunLocation::NotFound,
        1 => RunLocation::Found(hits.remove(0)),
        _ => RunLocation::Ambiguous(hits),
    }
}

/// 定位并把三态折叠成「目录或错误响应」（404 / 409；409 的 error 对象内附 roots）。
/// `not_found_code` 由路由面决定：数据端点 = `run_not_found`（既有契约），
/// /panel = `panel_unavailable`（既有契约）。
fn locate_run_dir_code(
    shared: &ServeShared,
    run_id: &str,
    not_found_code: &'static str,
) -> Result<PathBuf, Response> {
    match locate_run(shared, run_id) {
        RunLocation::Found(run_dir) => Ok(run_dir),
        RunLocation::NotFound => Err(error_response(
            404,
            not_found_code,
            &format!("局 `{run_id}` 不存在（run.json 缺失）"),
        )),
        RunLocation::Ambiguous(roots) => Err(Response::json(
            409,
            &json!({
                "ok": false,
                "error": {
                    "code": "run_id_ambiguous",
                    "message": format!(
                        "局 `{run_id}` 在多个观察根下同名（serve 不代选，请合并或改名其中一份）"
                    ),
                    "roots": roots
                        .iter()
                        .map(|root| json!(root.display().to_string()))
                        .collect::<Vec<_>>(),
                }
            }),
        )),
    }
}

fn locate_run_dir(shared: &ServeShared, run_id: &str) -> Result<PathBuf, Response> {
    locate_run_dir_code(shared, run_id, "run_not_found")
}

/// 后端裁决（只读）：run.json 的 `store` 事实 > 产物探针；失败 = 混代等，如实上抛。
fn resolve_backend(run_dir: &Path) -> Result<ygg_store::Backend, String> {
    let declared = super::read_meta(&run_dir.join(crate::META_FILE))
        .and_then(|meta| meta["store"].as_str().map(str::to_owned))
        .and_then(|text| ygg_store::Backend::parse(&text));
    ygg_store::resolve(run_dir, declared, None).map_err(|err| err.to_string())
}

// ============================================================================
// §数据端点（第一级 / 第二级 / 存档数据 API）
// ============================================================================

/// `GET /api/runs`——第一级：runs 枚举（§3.2；r9 起多根扫描，行带根标识）。
fn handle_runs(shared: &ServeShared) -> Response {
    // 外层循环根、内层循环局；个别根不可读 → errors 记录、其余根照常出行
    // （观察面「局部真」优于「全黑」，整体仍 200）。
    let mut rows: Vec<(u64, Json)> = Vec::new();
    let mut errors: Vec<Json> = Vec::new();
    for root in &shared.roots {
        let entries = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(err) => {
                errors.push(json!({
                    "root": root.display().to_string(),
                    "message": format!("读取失败：{err}"),
                }));
                continue;
            }
        };
        for entry in entries.flatten() {
            if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                continue;
            }
            let run_dir = entry.path();
            let meta_path = run_dir.join(crate::META_FILE);
            // run.json 缺失 → 跳过（中止创建的半壳目录不是局）。
            if !meta_path.is_file() {
                continue;
            }
            let run_id = entry.file_name().to_string_lossy().into_owned();
            let (meta, meta_error) = match std::fs::read_to_string(&meta_path)
                .map_err(|err| err.to_string())
                .and_then(|text| serde_json::from_str::<Json>(&text).map_err(|err| err.to_string()))
            {
                Ok(meta) => (meta, None),
                Err(err) => (Json::Null, Some(err)),
            };
            let updated_at = meta["updated_at"].as_u64().unwrap_or(0);
            let mut row = json!({
                "runId": run_id,
                "root": root.display().to_string(),
                "meta": meta,
                "coord": super::read_json(&run_dir.join(super::COORDINATION_FILE)),
            });
            if let Some(meta_error) = meta_error {
                row["metaError"] = json!(meta_error);
            }
            match resolve_backend(&run_dir) {
                Ok(backend) => {
                    row["backend"] = json!(backend.as_str());
                    row["probe"] = observe::save_probe(&run_dir, backend)
                        .map(|probe| {
                            json!({
                                "generation": probe.generation,
                                "latestId": probe.latest_id,
                                "latestCreatedAt": probe.latest_created_at,
                                "count": probe.count,
                            })
                        })
                        .unwrap_or_else(|err| json!({"count": 0, "probeError": err.to_string()}));
                }
                Err(err) => {
                    row["backendError"] = json!(err);
                    row["probe"] = json!({"generation": null, "latestId": null, "latestCreatedAt": 0, "count": 0});
                }
            }
            row["panel"] = panel_json(&run_id, run_mod_dir(&meta, Some(root.as_path())).as_deref());
            rows.push((updated_at, row));
        }
    }
    // updated_at 降序（活跃局自然在前）；meta 解析失败的行（键 0）沉底。
    rows.sort_by_key(|row| std::cmp::Reverse(row.0));
    let truncated = rows.len() > RUNS_CAP;
    rows.truncate(RUNS_CAP);
    let mut body = json!({
        "version": shared.broadcaster.version(),
        "generatedAt": now_secs(),
        "roots": shared
            .roots
            .iter()
            .map(|root| json!(root.display().to_string()))
            .collect::<Vec<_>>(),
        "runs": rows.into_iter().map(|(_, row)| row).collect::<Vec<_>>(),
    });
    if truncated {
        body["truncated"] = json!(true);
    }
    if !errors.is_empty() {
        body["errors"] = json!(errors);
    }
    Response::json(200, &body)
}

/// run.json 里的 mod 路径（只影响第三级 mod 面板托管；一二级数据面不依赖它）。
/// 相对路径的两级解析链：① 进程 cwd（既有语义，项目内 serve 行为不变）→
/// ② 该局所属根的父目录 / 祖父目录（规范布局 `<project>/.ygg/runs` 的直接推论：
/// 祖父 = `<project>`，`.ygg/mods/...` 命中；扁平布局 `<project>/runs` 的父目录
/// 同理命中；候选都做存在性检查，错候选无害）→ 皆不存 → 返回原值，交由既有的
/// `PanelDecl::Unavailable` 降级（不炸观察）。
fn run_mod_dir(meta: &Json, run_root: Option<&Path>) -> Option<PathBuf> {
    let declared = PathBuf::from(meta["mod"].as_str()?);
    if declared.is_absolute() || declared.exists() {
        return Some(declared);
    }
    if let Some(root) = run_root {
        for base in [root.parent(), root.parent().and_then(Path::parent)] {
            let Some(base) = base else { continue };
            let candidate = base.join(&declared);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    Some(declared)
}

/// `GET /api/runs/<id>/saves?limit=`——第二级：存档列表（§3.3）。
fn handle_saves(shared: &ServeShared, run_id: &str, query: &str) -> Response {
    if let Err(response) = check_run_id(run_id) {
        return response;
    }
    let run_dir = match locate_run_dir(shared, run_id) {
        Ok(run_dir) => run_dir,
        Err(response) => return response,
    };
    let backend = match resolve_backend(&run_dir) {
        Ok(backend) => backend,
        Err(err) => return error_response(500, "backend_unresolved", &err),
    };
    let limit = match query_usize(query, "limit", 0, observe::SAVE_LIST_MAX_LIMIT) {
        Ok(limit) => limit,
        Err(response) => return response,
    };
    let saves = match observe::save_list(&run_dir, backend, limit) {
        Ok(saves) => saves,
        Err(err) => return error_response(500, "observe_failed", &err.to_string()),
    };
    let probe = observe::save_probe(&run_dir, backend).unwrap_or_default();
    let meta = super::read_meta(&run_dir.join(crate::META_FILE));
    let panel = panel_json(
        run_id,
        meta.as_ref()
            .and_then(|meta| run_mod_dir(meta, run_dir.parent()))
            .as_deref(),
    );
    Response::json(
        200,
        &json!({
            "runId": run_id,
            "backend": backend.as_str(),
            "generation": probe.generation,
            "saves": saves.iter().map(save_row).collect::<Vec<_>>(),
            "panel": panel,
        }),
    )
}

fn save_row(meta: &observe::SaveMeta) -> Json {
    json!({
        "id": meta.id,
        "generation": meta.generation,
        "seq": meta.seq,
        "createdAt": meta.created_at,
        "bytes": meta.bytes,
        "latest": meta.latest,
    })
}

/// `GET /api/runs/<id>/saves/<sid>`——存档数据 API：原始字节 + `X-Ygg-*` 元数据头。
/// body = 存档原字节（**零包装零重序**）。
fn handle_save_blob(shared: &ServeShared, run_id: &str, sid: &str) -> Response {
    if let Err(response) = check_run_id(run_id) {
        return response;
    }
    if !valid_save_id(sid) {
        return error_response(
            400,
            "bad_save_id",
            &format!("存档 id `{sid}` 非法（只接受 `latest` 或十进制 id）"),
        );
    }
    let run_dir = match locate_run_dir(shared, run_id) {
        Ok(run_dir) => run_dir,
        Err(response) => return response,
    };
    let backend = match resolve_backend(&run_dir) {
        Ok(backend) => backend,
        Err(err) => return error_response(500, "backend_unresolved", &err),
    };
    match observe::save_get(&run_dir, backend, sid) {
        Ok(Some((meta, bytes))) => Response {
            status: 200,
            reason: "OK",
            content_type: "application/json",
            headers: vec![
                ("X-Ygg-Run", run_id.to_owned()),
                ("X-Ygg-Save-Id", meta.id),
                (
                    "X-Ygg-Generation",
                    meta.generation
                        .map_or(String::new(), |value| value.to_string()),
                ),
                ("X-Ygg-Seq", meta.seq.to_string()),
                ("X-Ygg-Created-At", meta.created_at.to_string()),
                ("X-Ygg-Bytes", meta.bytes.to_string()),
            ],
            body: bytes,
        },
        Ok(None) => error_response(404, "save_not_found", &format!("存档 `{sid}` 不存在")),
        Err(err) => error_response(500, "observe_failed", &err.to_string()),
    }
}

/// `GET /api/runs/<id>/events?since=&limit=`——事件日志分页（seq 升序；payload 原文）。
fn handle_events(shared: &ServeShared, run_id: &str, query: &str) -> Response {
    if let Err(response) = check_run_id(run_id) {
        return response;
    }
    let run_dir = match locate_run_dir(shared, run_id) {
        Ok(run_dir) => run_dir,
        Err(response) => return response,
    };
    let backend = match resolve_backend(&run_dir) {
        Ok(backend) => backend,
        Err(err) => return error_response(500, "backend_unresolved", &err),
    };
    let since = match query_usize(query, "since", 0, usize::MAX) {
        Ok(since) => since as u64,
        Err(response) => return response,
    };
    let limit = match query_usize(
        query,
        "limit",
        EVENT_LIMIT_DEFAULT as usize,
        EVENT_LIMIT_MAX as usize,
    ) {
        Ok(limit) => limit as u32,
        Err(response) => return response,
    };
    let page = match observe::event_page(&run_dir, backend, since, limit) {
        Ok(page) => page,
        Err(err) => return error_response(500, "observe_failed", &err.to_string()),
    };
    let probe = observe::save_probe(&run_dir, backend).unwrap_or_default();
    let records = page
        .records
        .iter()
        .map(|record| {
            json!({
                "seq": record.seq,
                "event": record.event,
                "payload": record.payload, // 原文透传（actor/actor_session 段随 r3 自动出现）
                "ok": record.ok,
                "timestamp": record.timestamp,
                "ending": record.ending,
            })
        })
        .collect::<Vec<_>>();
    Response::json(
        200,
        &json!({
            "runId": run_id,
            "generation": probe.generation,
            "latest": page.latest,
            "since": since,
            "records": records,
        }),
    )
}

/// `GET /api/runs/<id>/artifacts`——当前代制品元信息（path 升序）。
fn handle_artifacts(shared: &ServeShared, run_id: &str) -> Response {
    if let Err(response) = check_run_id(run_id) {
        return response;
    }
    let run_dir = match locate_run_dir(shared, run_id) {
        Ok(run_dir) => run_dir,
        Err(response) => return response,
    };
    let backend = match resolve_backend(&run_dir) {
        Ok(backend) => backend,
        Err(err) => return error_response(500, "backend_unresolved", &err),
    };
    match observe::artifact_list_view(&run_dir, backend) {
        Ok(artifacts) => Response::json(
            200,
            &json!({
                "artifacts": artifacts.iter().map(|meta| json!({
                    "path": meta.path,
                    "bytes": meta.bytes,
                    "sha256": meta.sha256,
                    "updatedAt": meta.updated_at,
                })).collect::<Vec<_>>(),
            }),
        ),
        Err(err) => error_response(500, "observe_failed", &err.to_string()),
    }
}

/// `GET /api/runs/<id>/artifacts/<path..>`——制品原文（`validate_artifact_path` 门复用；
/// 读侧封顶 1 MiB → 413）。
fn handle_artifact_content(shared: &ServeShared, run_id: &str, artifact_path: &str) -> Response {
    if let Err(response) = check_run_id(run_id) {
        return response;
    }
    if let Err(err) = ygg_store::validate_artifact_path(artifact_path) {
        return error_response(400, "bad_artifact_path", &err.to_string());
    }
    let run_dir = match locate_run_dir(shared, run_id) {
        Ok(run_dir) => run_dir,
        Err(response) => return response,
    };
    let backend = match resolve_backend(&run_dir) {
        Ok(backend) => backend,
        Err(err) => return error_response(500, "backend_unresolved", &err),
    };
    match observe::artifact_get_view(&run_dir, backend, artifact_path) {
        Ok(Some(record)) => {
            if record.content.len() > ARTIFACT_HTTP_CAP {
                return error_response(413, "artifact_too_large", "制品超过读侧封顶 1 MiB");
            }
            Response {
                status: 200,
                reason: "OK",
                content_type: content_type_for(artifact_path),
                headers: vec![
                    ("X-Ygg-Run", run_id.to_owned()),
                    ("X-Ygg-Bytes", record.content.len().to_string()),
                    ("X-Ygg-Sha256", record.meta.sha256),
                ],
                body: record.content,
            }
        }
        Ok(None) => error_response(
            404,
            "artifact_not_found",
            &format!("制品 `{artifact_path}` 不存在"),
        ),
        Err(err) => error_response(500, "observe_failed", &err.to_string()),
    }
}

// ============================================================================
// §mod 面板托管（§4：声明解析 + 双守卫现读）
// ============================================================================

/// pack.json `panel` 段的解析结果（§4.1：声明损坏 ≠ 装载错误——降级不炸）。
enum PanelDecl {
    Available { root: PathBuf, entry_file: String },
    Unavailable { reason: String },
}

/// pack.json 可选顶层 `panel` 段：`{"panel":{"entry":"panel/index.html"}}`。
/// 语法门 = 不以 `/` 开头、不含 `..`、以 `.html` 结尾；entry 文件必须真实存在
/// 且 canonicalize 后仍在面板根内（拦 symlink 逃逸）。
fn panel_decl(mod_dir: &Path) -> PanelDecl {
    // 退役旧名（D-MA19）先给专名降级理由：目录缺席且形状命中时，点名规范路径
    // （观察面不炸，§4.1：声明损坏 ≠ 装载错误）。
    if !mod_dir.exists() {
        if let Some(reason) = super::retired_seed_name_reason(mod_dir) {
            return PanelDecl::Unavailable { reason };
        }
    }
    let pack_path = mod_dir.join(super::PACK_FILE);
    let pack = match std::fs::read_to_string(&pack_path)
        .map_err(|err| err.to_string())
        .and_then(|text| serde_json::from_str::<Json>(&text).map_err(|err| err.to_string()))
    {
        Ok(pack) => pack,
        Err(err) => {
            return PanelDecl::Unavailable {
                reason: format!("pack.json 不可读：{err}"),
            }
        }
    };
    let Some(panel) = pack.get("panel") else {
        return PanelDecl::Unavailable {
            reason: "panel 段未声明".to_owned(),
        };
    };
    let Some(entry) = panel["entry"].as_str() else {
        return PanelDecl::Unavailable {
            reason: "panel.entry 未声明或不是字符串".to_owned(),
        };
    };
    if entry.is_empty() {
        return PanelDecl::Unavailable {
            reason: "panel.entry 为空".to_owned(),
        };
    }
    if entry.starts_with('/') {
        return PanelDecl::Unavailable {
            reason: "panel.entry 不得以 / 开头（模内相对路径）".to_owned(),
        };
    }
    if entry.split('/').any(|segment| segment == "..") {
        return PanelDecl::Unavailable {
            reason: "panel.entry 不得含 ..（目录穿越）".to_owned(),
        };
    }
    if !entry.ends_with(".html") {
        return PanelDecl::Unavailable {
            reason: "panel.entry 必须以 .html 结尾".to_owned(),
        };
    }
    let entry_path = mod_dir.join(entry);
    let Some(root) = entry_path.parent().map(Path::to_path_buf) else {
        return PanelDecl::Unavailable {
            reason: "panel.entry 无父目录".to_owned(),
        };
    };
    let (Ok(canon_root), Ok(canon_entry)) = (
        std::fs::canonicalize(&root),
        std::fs::canonicalize(&entry_path),
    ) else {
        return PanelDecl::Unavailable {
            reason: "entry 文件缺失（或 mod 目录不可达）".to_owned(),
        };
    };
    if !canon_entry.starts_with(&canon_root) {
        return PanelDecl::Unavailable {
            reason: "entry 逃出面板根（符号链接？）".to_owned(),
        };
    }
    let entry_file = entry.rsplit('/').next().unwrap_or(entry).to_owned();
    PanelDecl::Available {
        root: canon_root,
        entry_file,
    }
}

/// 行内 panel 字段（§3.4）：`{available, url?, reason?}`。url 指向 entry 本体
/// （两级页的「在面板打开」直接拼 `?save=<sid>`——query 传参可收藏可刷新）。
fn panel_json(run_id: &str, mod_dir: Option<&Path>) -> Json {
    match mod_dir.map(panel_decl) {
        Some(PanelDecl::Available { entry_file, .. }) => json!({
            "available": true,
            "url": format!("/panel/{run_id}/{entry_file}"),
        }),
        Some(PanelDecl::Unavailable { reason }) => json!({"available": false, "reason": reason}),
        None => json!({"available": false, "reason": "run.json 缺少 mod 字段"}),
    }
}

/// `GET /panel/<run-id>/[rel]`——mod 面板托管（§4.2：运行期现读 + 双守卫）。
fn serve_mod_panel(shared: &ServeShared, run_id: &str, rel: &str) -> Response {
    if let Err(response) = check_run_id(run_id) {
        return response;
    }
    let run_dir = match locate_run_dir_code(shared, run_id, "panel_unavailable") {
        Ok(run_dir) => run_dir,
        Err(response) => return response,
    };
    let meta = super::read_meta(&run_dir.join(crate::META_FILE))
        .ok_or_else(|| "run.json 不可解析".to_owned());
    let mod_dir = meta
        .as_ref()
        .ok()
        .and_then(|meta| run_mod_dir(meta, run_dir.parent()));
    let Some(mod_dir) = mod_dir else {
        return error_response(404, "panel_unavailable", "run.json 缺少可用的 mod 路径");
    };
    match panel_decl(&mod_dir) {
        PanelDecl::Unavailable { reason } => error_response(404, "panel_unavailable", &reason),
        PanelDecl::Available { root, entry_file } => {
            // 目录形式（`/panel/<id>/`）= 服务 entry 本体（mod 内相对链接 `./` 起跳）。
            let rel = if rel.is_empty() {
                entry_file.as_str()
            } else {
                rel
            };
            // rel 语法门（canonicalize 之外的第二道守卫：不触盘先拒）。
            if rel.starts_with('/')
                || rel.contains('\\')
                || rel
                    .split('/')
                    .any(|segment| segment == ".." || segment.is_empty())
            {
                return error_response(404, "panel_path_rejected", "面板资源路径非法");
            }
            let target = root.join(rel);
            // 双守卫之一：canonicalize（解析 symlink 与 `..`）。
            let Ok(canon_target) = std::fs::canonicalize(&target) else {
                return error_response(
                    404,
                    "panel_asset_missing",
                    &format!("面板资源 `{rel}` 不存在"),
                );
            };
            // 双守卫之二：前缀校验（canonicalize 后仍必须在面板根内——拦 symlink 逃逸）。
            if !canon_target.starts_with(&root) {
                return error_response(
                    404,
                    "panel_path_rejected",
                    "面板资源逃出面板根（符号链接？）",
                );
            }
            if !canon_target.is_file() {
                return error_response(
                    404,
                    "panel_asset_missing",
                    &format!("面板资源 `{rel}` 不是文件"),
                );
            }
            match std::fs::read(&canon_target) {
                Ok(bytes) => Response {
                    status: 200,
                    reason: "OK",
                    content_type: content_type_for(rel),
                    headers: Vec::new(),
                    body: bytes,
                },
                Err(err) => error_response(500, "panel_asset_unreadable", &err.to_string()),
            }
        }
    }
}

// ============================================================================
// §内置两级列表页资产（include_dir 嵌入 + --panel-dir 盘上覆盖）
// ============================================================================

/// `GET /` 与 `GET /assets/<name>`：内置页三件（白名单），`--panel-dir` 覆盖生效。
fn builtin_asset(shared: &ServeShared, name: &str) -> Response {
    if name != "index.html" && !ASSET_WHITELIST.contains(&name) {
        return error_response(404, "not_found", "未知资产");
    }
    if let Some(panel_dir) = &shared.panel_dir {
        // 盘上覆盖：canonicalize 前缀守卫；缺失 = 404（fail-closed，开发期立刻发现断链）。
        return match override_file(panel_dir, name) {
            Some(bytes) => asset_response(name, bytes),
            None => error_response(
                404,
                "asset_missing",
                &format!("覆盖目录 {} 中缺少 {name}", panel_dir.display()),
            ),
        };
    }
    match PANEL_ASSETS.get_file(name) {
        Some(file) => asset_response(name, file.contents().to_vec()),
        None => error_response(404, "asset_missing", &format!("嵌入资产缺少 {name}")),
    }
}

fn asset_response(name: &str, bytes: Vec<u8>) -> Response {
    Response {
        status: 200,
        reason: "OK",
        content_type: content_type_for(name),
        headers: Vec::new(),
        body: bytes,
    }
}

/// `--panel-dir` 覆盖读取（文件名白名单已在外层；这里再加 canonicalize 前缀守卫）。
fn override_file(panel_dir: &Path, name: &str) -> Option<Vec<u8>> {
    if name.contains('/') || name.contains('\\') || name.split('/').any(|segment| segment == "..") {
        return None;
    }
    let canon_root = std::fs::canonicalize(panel_dir).ok()?;
    let canon_target = std::fs::canonicalize(canon_root.join(name)).ok()?;
    if !canon_target.starts_with(&canon_root) {
        return None;
    }
    std::fs::read(canon_target).ok()
}

/// Content-Type 小映射（§4.2）；未知扩展 → `application/octet-stream` + nosniff
/// （nosniff 由 write_response 恒定携带）。
fn content_type_for(name: &str) -> &'static str {
    let extension = name.rsplit('.').next().unwrap_or("");
    match extension {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "txt" => "text/plain; charset=utf-8",
        "md" => "text/markdown; charset=utf-8",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

// ============================================================================
// §测试锁（§11.1–§11.3：路由与解析 / 两级列表与面板托管 / SSE 与端到端）
// ============================================================================

#[cfg(test)]
mod serve_tests {
    use super::*;
    use serde_json::Value as Json;
    use std::fs;
    use std::path::PathBuf;
    use ygg_store::{Backend, RunStore, StepDraft, Tick};

    const TICK: Tick = Tick {
        secs: 1_700_000_000,
        nanos: 5,
    };

    fn case_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ygg-serve-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    fn shared_for(runs_root: &Path) -> Arc<ServeShared> {
        shared_for_roots(&[runs_root.to_path_buf()])
    }

    fn shared_for_roots(roots: &[PathBuf]) -> Arc<ServeShared> {
        Arc::new(ServeShared {
            roots: roots.to_vec(),
            panel_dir: None,
            broadcaster: Broadcaster::new(),
            connections: AtomicUsize::new(0),
        })
    }

    /// 在 shared 上打一发路由（不碰网络；POST 也经同一路由入口）。
    fn http(shared: &ServeShared, method: &str, target: &str) -> Response {
        let (path, query) = match target.split_once('?') {
            Some((path, query)) => (path, query),
            None => (target, ""),
        };
        match route(shared, method, path, query) {
            Outcome::Http(response) => response,
            Outcome::Sse => Response {
                status: 200,
                reason: "OK",
                content_type: "text/event-stream",
                headers: Vec::new(),
                body: Vec::new(),
            },
        }
    }

    fn body_json(response: &Response) -> Json {
        serde_json::from_slice(&response.body).expect("响应体是合法 JSON")
    }

    /// 起一个真监听的 serve（测试面：port 0 + 后台 watcher/accept 线程）。
    struct LiveServe {
        port: u16,
        shutdown: Arc<AtomicBool>,
    }

    impl LiveServe {
        fn start(roots: &[PathBuf], poll_ms: u64) -> Self {
            let (listener, shared) = bind_serve(roots.to_vec(), None).expect("绑定测试端口");
            let port = listener.local_addr().expect("端口").port();
            let shutdown = Arc::new(AtomicBool::new(false));
            {
                let shared = Arc::clone(&shared);
                let shutdown = Arc::clone(&shutdown);
                std::thread::spawn(move || watcher_loop(&shared, &shutdown, poll_ms));
            }
            {
                let shared = Arc::clone(&shared);
                let shutdown = Arc::clone(&shutdown);
                std::thread::spawn(move || {
                    // keepalive 注入为短值（保活用例同款手法：缩短常量窗口）。
                    serve_accept_loop(listener, &shared, &shutdown, Duration::from_millis(80))
                });
            }
            Self { port, shutdown }
        }

        /// 真实 TCP GET（close 定界：read_to_end 即完整响应）。
        fn get(&self, target: &str) -> (u16, String, Vec<u8>) {
            let mut stream = TcpStream::connect((BIND_ADDR, self.port)).expect("连接测试 serve");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("读超时");
            write!(stream, "GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").expect("写请求");
            let mut raw = Vec::new();
            stream.read_to_end(&mut raw).expect("读响应");
            parse_http(&raw)
        }

        /// SSE：连上后等 `predicate` 命中的帧（有界窗口；ping 帧被自然跳过）。
        fn sse_frame(&self, predicate: impl Fn(&str, &str) -> bool) -> (String, String) {
            let mut stream = TcpStream::connect((BIND_ADDR, self.port)).expect("连接 SSE");
            stream
                .set_read_timeout(Some(Duration::from_millis(2_500)))
                .expect("读超时");
            write!(
                stream,
                "GET /api/stream HTTP/1.1\r\nHost: localhost\r\n\r\n"
            )
            .expect("写请求");
            let deadline = std::time::Instant::now() + Duration::from_secs(4);
            let mut buf: Vec<u8> = Vec::new();
            let mut chunk = [0u8; 512];
            while std::time::Instant::now() < deadline {
                match stream.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    Err(_) => break,
                }
                let text = String::from_utf8_lossy(&buf);
                for frame in text.split("\n\n") {
                    let mut event = String::new();
                    let mut data = String::new();
                    for line in frame.lines() {
                        if let Some(value) = line.strip_prefix("event: ") {
                            event = value.to_owned();
                        }
                        if let Some(value) = line.strip_prefix("data: ") {
                            data = value.to_owned();
                        }
                    }
                    if !event.is_empty() && predicate(&event, &data) {
                        return (event, data);
                    }
                }
            }
            panic!("SSE 窗口内未等到期望帧：{}", String::from_utf8_lossy(&buf));
        }
    }

    impl Drop for LiveServe {
        fn drop(&mut self) {
            self.shutdown.store(true, Ordering::Relaxed);
        }
    }

    fn parse_http(raw: &[u8]) -> (u16, String, Vec<u8>) {
        let split = raw
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("响应头完整");
        let head = String::from_utf8_lossy(&raw[..split]).to_owned();
        let status: u16 = head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);
        (status, head.to_string(), raw[split + 4..].to_vec())
    }

    /// 最小合成 Mod（可带 panel 声明；与既有测试的手写 pack.json 同款）。
    fn write_mod(dir: &Path, panel_entry: Option<&str>) {
        fs::create_dir_all(dir.join("data")).expect("建 data 目录");
        let panel_segment = match panel_entry {
            Some(entry) => format!(r#","panel": {{"entry": "{entry}"}}"#),
            None => String::new(),
        };
        fs::write(
            dir.join("pack.json"),
            format!(
                r#"{{
  "schemaVersion": 1,
  "entries": [],
  "schemaFiles": [],
  "agent": {{
    "entryEvent": "event.start",
    "choiceEvent": "ygg.choice_selected",
    "summaryEvent": "agent.summary",
    "endingPrefix": "【结局",
    "metaKey": "summary",
    "rejectionPrefixes": ["【拒绝】"],
    "replayPrefix": "【重放】",
    "defaultPayload": "0"
  }},
  "inboxRoutes": {{
    "externalIdField": "external_id",
    "payloadField": "payload",
    "extMarker": "@ext="
  }}{panel_segment}
}}"#
            ),
        )
        .expect("写 pack.json");
        fs::write(
            dir.join("manifest.json"),
            r#"{"schemaVersion": 1, "entries": [], "code": []}"#,
        )
        .expect("写 manifest.json");
        fs::write(
            dir.join("mod.lua"),
            r#"ygg.on("event.start", function()
  ygg.narrate("开局")
  ygg.choice("下一步？", { "推进" })
end)
ygg.on("agent.summary", function()
  ygg.meta("summary", { open = true })
end)
ygg.on("ygg.choice_selected", function()
end)
"#,
        )
        .expect("写 mod.lua");
        fs::write(
            dir.join("data/config.json"),
            r#"{
  "drive": {"maxSteps": 50, "stallLimit": 5, "stepsPerRequirement": 20},
  "settlement": {
    "requireEvents": true,
    "requireNoFailedSteps": true,
    "requireEnding": true,
    "requireInboxConsumed": true
  }
}"#,
        )
        .expect("写 config.json");
    }

    /// 直接经持久化端口造一个 N 步的局 + run.json（serve 数据端点的夹具）。
    fn seed_run(runs_root: &Path, run_id: &str, backend: Backend, steps: usize) {
        let run_dir = runs_root.join(run_id);
        fs::create_dir_all(&run_dir).expect("建 run 目录");
        let mut store: Box<dyn RunStore> =
            ygg_store::open(backend, &run_dir, TICK).expect("打开后端");
        for i in 0..steps {
            let payload = format!("{{\"i\":{i}}}");
            let save = format!("{{\"step\":{i}}}");
            store
                .commit_step(StepDraft {
                    event: "event.start",
                    payload: &payload,
                    ok: true,
                    narrative_count: 1,
                    timestamp: TICK.secs + i as u64,
                    ending_line: None,
                    save: Some(save.as_bytes()),
                })
                .expect("提交步");
        }
        fs::write(
            run_dir.join(crate::META_FILE),
            format!(
                "{{\"run_id\":\"{run_id}\",\"mod\":\"{}\",\"started_at\":{},\"updated_at\":{},\"store\":\"{}\"}}\n",
                runs_root.join("_mod").display(),
                TICK.secs,
                TICK.secs + steps as u64,
                backend.as_str(),
            ),
        )
        .expect("写 run.json");
    }

    fn seed_run_with_mod(
        runs_root: &Path,
        run_id: &str,
        backend: Backend,
        steps: usize,
        mod_dir: &Path,
    ) {
        seed_run(runs_root, run_id, backend, steps);
        let run_json = runs_root.join(run_id).join(crate::META_FILE);
        let mut meta: Json =
            serde_json::from_str(&fs::read_to_string(&run_json).expect("读 run.json"))
                .expect("解析 run.json");
        meta["mod"] = json!(mod_dir.display().to_string());
        fs::write(
            &run_json,
            serde_json::to_string_pretty(&meta).expect("序列化") + "\n",
        )
        .expect("写回 run.json");
    }

    // ---- §11.1 路由与解析 ----

    #[test]
    fn request_line_parsing() {
        // 合法形态（走与网络读取同一段解析逻辑）。
        let request = parse_request_from_head("GET /api/runs?since=2 HTTP/1.1\r\nHost: t\r\n\r\n")
            .expect("合法行");
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/api/runs");
        assert_eq!(request.query, "since=2");
        let request = parse_request_from_head("GET / HTTP/1.1\r\n\r\n").expect("合法行");
        assert_eq!(request.path, "/");
        assert_eq!(request.query, "");
        // 畸形：缺版本 / 非 HTTP 版本 / 目标不以 / 开头 / 空行。
        assert!(parse_request_from_head("GET /api/runs\r\n\r\n").is_none());
        assert!(parse_request_from_head("GET / RTSP/1.0\r\n\r\n").is_none());
        assert!(parse_request_from_head("POST api/runs HTTP/1.1\r\n\r\n").is_none());
        assert!(parse_request_from_head("\r\n").is_none());
    }

    #[test]
    fn routes_are_get_only() {
        let dir = case_dir("get-only");
        let shared = shared_for(&dir);
        // 路由表里根本没有非 GET 端点：任何路径的非 GET 一律 405（物理不可绕过）。
        for target in [
            "/",
            "/api/runs",
            "/api/stream",
            "/panel/x/",
            "/api/runs/x/saves",
        ] {
            let response = http(&shared, "POST", target);
            assert_eq!(response.status, 405, "POST {target} 必须 405");
            assert_eq!(
                body_json(&response)["error"]["code"],
                json!("method_not_allowed")
            );
            let response = http(&shared, "DELETE", target);
            assert_eq!(response.status, 405, "DELETE {target} 必须 405");
        }
        // 未知路径 404。
        assert_eq!(http(&shared, "GET", "/nope").status, 404);
    }

    #[test]
    fn bind_is_localhost_constant() {
        // 绑定地址是**常量**（无旗标）：漂到 0.0.0.0 必须红。
        assert_eq!(BIND_ADDR, "127.0.0.1");
        // 真绑定的监听器也必须落在 loopback 上。
        let dir = case_dir("bind");
        let (listener, _shared) = bind_serve(vec![dir], None).expect("绑定");
        let ip = listener.local_addr().expect("地址").ip().to_string();
        assert!(ip.starts_with("127."), "绑定地址必须是 loopback：{ip}");
    }

    #[test]
    fn run_id_path_grammar() {
        let dir = case_dir("run-grammar");
        let shared = shared_for(&dir);
        // `..%2f` 解码后段是 `..`（段级语法门在触盘之前拒绝）。
        let response = http(&shared, "GET", "/api/runs/..%2f/saves");
        assert_eq!(response.status, 400);
        assert_eq!(body_json(&response)["error"]["code"], json!("bad_run_id"));
        // 反斜杠同样被拒；/panel 段同门。
        assert_eq!(http(&shared, "GET", "/api/runs/a%5Cb/saves").status, 400);
        assert_eq!(http(&shared, "GET", "/panel/../secret").status, 400);
        // 语法合法但局不存在 → 404（runs-root 为空目录）。
        assert_eq!(http(&shared, "GET", "/api/runs/ok-id/saves").status, 404);
    }

    #[test]
    fn save_sid_grammar() {
        let dir = case_dir("sid-grammar");
        let shared = shared_for(&dir);
        // 非法 sid → 400（不触盘）。
        for sid in ["abc", "12x", "-1", "1.5", "%20", "latest%00"] {
            let response = http(&shared, "GET", &format!("/api/runs/x/saves/{sid}"));
            assert_eq!(response.status, 400, "sid `{sid}` 必须 400");
            assert_eq!(body_json(&response)["error"]["code"], json!("bad_save_id"));
        }
        // 合法语法但局不存在 → 404。
        let response = http(&shared, "GET", "/api/runs/no-such/saves/999999");
        assert_eq!(response.status, 404);
        assert_eq!(
            body_json(&response)["error"]["code"],
            json!("run_not_found")
        );
    }

    #[test]
    fn events_since_limit_clamped() {
        // 封顶常量面（§3.4：缺省 200、封顶 500）。
        assert_eq!(EVENT_LIMIT_MAX, 500);
        assert_eq!(EVENT_LIMIT_DEFAULT, 200);
        let dir = case_dir("ev-clamp");
        seed_run(&dir, "ev", Backend::Json, 3);
        let shared = shared_for(&dir);
        let response = http(&shared, "GET", "/api/runs/ev/events?since=1&limit=2");
        assert_eq!(response.status, 200);
        let body = body_json(&response);
        assert_eq!(body["since"], json!(1));
        assert_eq!(body["latest"], json!(3));
        let records = body["records"].as_array().expect("records");
        assert_eq!(records.len(), 2, "limit 截断");
        assert_eq!(records[0]["seq"], json!(1));
        // payload 原文透传（actor 段随 r3 落地后同路径自动出现）。
        assert_eq!(records[0]["payload"], json!("{\"i\":1}"));
        assert_eq!(records[0]["event"], json!("event.start"));
        // 非法数值 → 400；越界数值 → 钳制放行。
        assert_eq!(
            http(&shared, "GET", "/api/runs/ev/events?limit=abc").status,
            400
        );
        assert_eq!(
            http(&shared, "GET", "/api/runs/ev/events?since=-1").status,
            400
        );
        assert_eq!(
            http(&shared, "GET", "/api/runs/ev/events?limit=999999").status,
            200
        );
    }

    #[test]
    fn runs_list_shape() {
        let dir = case_dir("runs-shape");
        let shared = shared_for(&dir);
        // 空 root → {runs:[]}。
        assert_eq!(
            body_json(&http(&shared, "GET", "/api/runs"))["runs"],
            json!([])
        );
        // 无 run.json 的目录跳过（且不补写任何文件）；坏 run.json → metaError。
        fs::create_dir_all(dir.join("half-shell")).expect("建半壳目录");
        fs::create_dir_all(dir.join("broken")).expect("建坏局目录");
        fs::write(dir.join("broken/run.json"), b"{not json").expect("写坏 run.json");
        seed_run(&dir, "good", Backend::Json, 1);
        let body = body_json(&http(&shared, "GET", "/api/runs"));
        let runs = body["runs"].as_array().expect("runs");
        assert_eq!(runs.len(), 2, "半壳目录必须被跳过");
        let broken = runs
            .iter()
            .find(|row| row["runId"] == "broken")
            .expect("坏局在列");
        assert!(
            broken["metaError"].is_string(),
            "坏 run.json 必须带 metaError"
        );
        assert!(broken["meta"].is_null());
        let good = runs
            .iter()
            .find(|row| row["runId"] == "good")
            .expect("好局在列");
        assert_eq!(good["backend"], json!("json"));
        assert!(good["metaError"].is_null());
        assert!(good["meta"]["run_id"] == json!("good"));
        assert_eq!(good["probe"]["count"], json!(1));
        // 半壳目录未被补写任何文件。
        let leftovers: Vec<_> = fs::read_dir(dir.join("half-shell")).unwrap().collect();
        assert!(leftovers.is_empty(), "观察面不得写半壳目录");
    }

    #[test]
    fn runs_list_sorted_and_capped() {
        let dir = case_dir("runs-cap");
        seed_run(&dir, "older", Backend::Json, 1);
        seed_run(&dir, "newer", Backend::Json, 2); // updated_at = TICK+2 > TICK+1
        let shared = shared_for(&dir);
        let body = body_json(&http(&shared, "GET", "/api/runs"));
        let runs = body["runs"].as_array().expect("runs");
        let ids: Vec<&str> = runs
            .iter()
            .map(|row| row["runId"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["newer", "older"], "updated_at 降序（活跃局在前）");
        // 封顶常量与 truncated 标记的面。
        assert_eq!(RUNS_CAP, 200);
        assert!(body["truncated"].is_null(), "未超顶不得有 truncated");
        assert!(body["version"].is_u64(), "响应必须带 version");
        assert!(body["generatedAt"].is_u64(), "响应必须带 generatedAt");
    }

    // ---- §11.2 两级列表与面板托管 ----

    #[cfg(feature = "store-sqlite")]
    #[test]
    fn saves_endpoint_sqlite_and_json() {
        let dir = case_dir("saves-ep");
        seed_run(&dir, "sq", Backend::Sqlite, 3);
        seed_run(&dir, "js", Backend::Json, 3);
        let shared = shared_for(&dir);
        let sqlite = body_json(&http(&shared, "GET", "/api/runs/sq/saves"));
        assert_eq!(sqlite["backend"], json!("sqlite"));
        assert_eq!(sqlite["generation"], json!(0));
        let saves = sqlite["saves"].as_array().expect("saves");
        assert_eq!(saves.len(), 3, "每成功步一行存档");
        assert_eq!(saves[0]["id"], json!("3"), "新→旧");
        assert_eq!(saves[0]["seq"], json!(2));
        assert_eq!(saves[0]["latest"], json!(true));
        assert_eq!(saves[0]["bytes"], json!(b"{\"step\":2}".len()));
        assert_eq!(saves[2]["id"], json!("1"));
        assert_eq!(saves[2]["latest"], json!(false));
        // limit 生效：?limit=2 只取最新两行。
        let body = body_json(&http(&shared, "GET", "/api/runs/sq/saves?limit=2"));
        assert_eq!(body["saves"].as_array().expect("saves").len(), 2);
        // JSON 局恒单行 latest（单档后端的物理事实，不伪造历史）。
        let json_run = body_json(&http(&shared, "GET", "/api/runs/js/saves"));
        let saves = json_run["saves"].as_array().expect("saves");
        assert_eq!(saves.len(), 1);
        assert_eq!(saves[0]["id"], json!("latest"));
        assert_eq!(saves[0]["seq"], json!(3));
        assert!(json_run["generation"].is_null());
    }

    #[cfg(feature = "store-sqlite")]
    #[test]
    fn save_blob_is_verbatim() {
        let dir = case_dir("blob");
        seed_run(&dir, "b", Backend::Sqlite, 3);
        let shared = shared_for(&dir);
        // sid = snapshot id（新→旧 3/2/1，对应 seq 2/1/0）：id 2 的档是 seq 1 的存档。
        let expected_id2 = b"{\"step\":1}".to_vec();
        let response = http(&shared, "GET", "/api/runs/b/saves/2");
        assert_eq!(response.status, 200);
        assert_eq!(response.content_type, "application/json");
        assert_eq!(
            response.body, expected_id2,
            "body = 存档原字节（零包装零重序）"
        );
        let header = |name: &str| {
            response
                .headers
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.clone())
                .unwrap_or_else(|| panic!("缺头 {name}"))
        };
        assert_eq!(header("X-Ygg-Run"), "b");
        assert_eq!(header("X-Ygg-Save-Id"), "2");
        assert_eq!(header("X-Ygg-Generation"), "0");
        assert_eq!(header("X-Ygg-Seq"), "1");
        assert_eq!(header("X-Ygg-Created-At"), (TICK.secs + 1).to_string());
        assert_eq!(header("X-Ygg-Bytes"), expected_id2.len().to_string());
        // `latest` 与最大 id（3，seq 2）字节等值。
        let latest = http(&shared, "GET", "/api/runs/b/saves/latest");
        assert_eq!(latest.status, 200);
        assert_eq!(latest.body, b"{\"step\":2}".to_vec());
        assert_eq!(
            latest.body,
            http(&shared, "GET", "/api/runs/b/saves/3").body
        );
        // 缺档 404。
        assert_eq!(http(&shared, "GET", "/api/runs/b/saves/999999").status, 404);
    }

    #[test]
    fn artifacts_endpoints_roundtrip() {
        let dir = case_dir("artifacts");
        let run_dir = dir.join("a");
        fs::create_dir_all(&run_dir).expect("建 run 目录");
        let mut store: Box<dyn RunStore> =
            ygg_store::open(Backend::Json, &run_dir, TICK).expect("打开后端");
        let meta = store
            .artifact_put("artifacts/note.md", "正文".as_bytes(), TICK)
            .expect("写制品");
        drop(store);
        fs::write(
            run_dir.join(crate::META_FILE),
            format!(
                "{{\"run_id\":\"a\",\"started_at\":{},\"updated_at\":{},\"store\":\"json\"}}\n",
                TICK.secs, TICK.secs
            ),
        )
        .expect("写 run.json");
        let shared = shared_for(&dir);
        let body = body_json(&http(&shared, "GET", "/api/runs/a/artifacts"));
        let artifacts = body["artifacts"].as_array().expect("artifacts");
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0]["path"], json!("artifacts/note.md"));
        assert_eq!(artifacts[0]["sha256"], json!(meta.sha256));
        // 原文直出。
        let response = http(&shared, "GET", "/api/runs/a/artifacts/artifacts/note.md");
        assert_eq!(response.status, 200);
        assert_eq!(response.body, "正文".as_bytes());
        // 非法路径 400 / 缺失 404。
        assert_eq!(
            http(&shared, "GET", "/api/runs/a/artifacts/artifacts/../x").status,
            400
        );
        assert_eq!(
            http(&shared, "GET", "/api/runs/a/artifacts/artifacts/none.md").status,
            404
        );
    }

    #[test]
    fn panel_decl_parsing() {
        let dir = case_dir("panel-decl");
        // 无段 → false + reason。
        let old_mod = dir.join("old-mod");
        write_mod(&old_mod, None);
        let PanelDecl::Unavailable { reason } = panel_decl(&old_mod) else {
            panic!("旧 mod 必须不可用");
        };
        assert!(reason.contains("panel 段未声明"), "{reason}");
        // 语法违规三连：绝对路径 / `..` / 非 .html（各带具体 reason）。
        for (entry, fragment) in [
            ("/abs/index.html", "/ 开头"),
            ("../escape/index.html", ".."),
            ("panel/index.htm", ".html"),
        ] {
            let bad_mod = dir.join(format!("bad-{}", entry.replace('/', "-")));
            write_mod(&bad_mod, Some(entry));
            let PanelDecl::Unavailable { reason } = panel_decl(&bad_mod) else {
                panic!("{entry} 必须不可用");
            };
            assert!(reason.contains(fragment), "`{entry}` → {reason}");
        }
        // 声明合法但文件缺失 → false + reason。
        let missing_mod = dir.join("missing-mod");
        write_mod(&missing_mod, Some("panel/index.html"));
        let PanelDecl::Unavailable { reason } = panel_decl(&missing_mod) else {
            panic!("entry 缺失必须不可用");
        };
        assert!(reason.contains("缺失"), "{reason}");
        // 合法声明 + 文件在 → true（root = entry 父目录的 canonical 形态）。
        let good_mod = dir.join("good-mod");
        write_mod(&good_mod, Some("panel/index.html"));
        fs::create_dir_all(good_mod.join("panel")).expect("建 panel 目录");
        fs::write(good_mod.join("panel/index.html"), b"<p>panel</p>").expect("写 entry");
        match panel_decl(&good_mod) {
            PanelDecl::Available { root, entry_file } => {
                assert_eq!(entry_file, "index.html");
                assert!(
                    root.ends_with("panel"),
                    "root = entry 的父目录：{}",
                    root.display()
                );
            }
            PanelDecl::Unavailable { reason } => panic!("合法声明必须可用：{reason}"),
        }
    }

    #[test]
    fn panel_assets_served_with_guards() {
        let dir = case_dir("panel-assets");
        let runs_root = dir.join("runs");
        let mod_dir = dir.join("mod");
        write_mod(&mod_dir, Some("panel/index.html"));
        fs::create_dir_all(mod_dir.join("panel")).expect("建 panel 目录");
        fs::write(mod_dir.join("panel/index.html"), b"<p>panel</p>").expect("写 entry");
        fs::write(mod_dir.join("panel/app.js"), b"// panel js").expect("写 js");
        // symlink 逃逸：escape.html 指向面板根之外。
        let outside = dir.join("outside.html");
        fs::write(&outside, b"secret").expect("写外部文件");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, mod_dir.join("panel/escape.html"))
            .expect("建 symlink");
        seed_run_with_mod(&runs_root, "p", Backend::Json, 1, &mod_dir);
        let shared = shared_for(&runs_root);
        // 入口：目录形式 = entry 本体。
        let response = http(&shared, "GET", "/panel/p/");
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"<p>panel</p>".to_vec());
        assert_eq!(response.content_type, "text/html; charset=utf-8");
        // 子资产现读（Content-Type 小映射 + nosniff 由 write_response 恒定携带）。
        let response = http(&shared, "GET", "/panel/p/app.js");
        assert_eq!(response.status, 200);
        assert_eq!(response.content_type, "text/javascript; charset=utf-8");
        // 穿越形态 → 404（语法门或 canonicalize 前缀守卫，两道都在）。
        assert_eq!(http(&shared, "GET", "/panel/p/../../x").status, 404);
        assert_eq!(http(&shared, "GET", "/panel/p/%2e%2e%2fx").status, 404);
        // symlink 逃逸 → 404（canonicalize 前缀守卫）。
        #[cfg(unix)]
        assert_eq!(http(&shared, "GET", "/panel/p/escape.html").status, 404);
        // 未声明局访问 /panel → 404 JSON（语法合法但不存在的局）。
        let response = http(&shared, "GET", "/panel/no-such/");
        assert_eq!(response.status, 404);
        assert_eq!(
            body_json(&response)["error"]["code"],
            json!("panel_unavailable")
        );
    }

    #[test]
    fn panel_undeclared_graceful() {
        let dir = case_dir("panel-old");
        let runs_root = dir.join("runs");
        let mod_dir = dir.join("old-mod");
        write_mod(&mod_dir, None); // 旧式 mod：无 panel 段
        seed_run_with_mod(&runs_root, "o", Backend::Json, 1, &mod_dir);
        let shared = shared_for(&runs_root);
        // saves 响应：panel.available=false + reason（两级页据此显示降级文案）。
        let body = body_json(&http(&shared, "GET", "/api/runs/o/saves"));
        assert_eq!(body["panel"]["available"], json!(false));
        assert_eq!(body["panel"]["reason"], json!("panel 段未声明"));
        // /panel/<id>/ → 404 JSON 带 reason。
        let response = http(&shared, "GET", "/panel/o/");
        assert_eq!(response.status, 404);
        let body = body_json(&response);
        assert_eq!(body["error"]["code"], json!("panel_unavailable"));
        assert_eq!(body["error"]["message"], json!("panel 段未声明"));
    }

    #[test]
    fn builtin_panel_assets_served() {
        let dir = case_dir("builtin");
        let shared = shared_for(&dir);
        // `GET /` 200 含挂载点与资产引用。
        let response = http(&shared, "GET", "/");
        assert_eq!(response.status, 200);
        assert_eq!(response.content_type, "text/html; charset=utf-8");
        let html = String::from_utf8_lossy(&response.body);
        assert!(html.contains("id=\"view\""), "挂载点必须在内置页里");
        assert!(html.contains("/assets/app.js"), "内置页必须引用 app.js");
        // 白名单资产。
        let js = http(&shared, "GET", "/assets/app.js");
        assert_eq!(js.status, 200);
        assert_eq!(js.content_type, "text/javascript; charset=utf-8");
        let css = http(&shared, "GET", "/assets/style.css");
        assert_eq!(css.status, 200);
        assert_eq!(css.content_type, "text/css; charset=utf-8");
        // 白名单之外 404。
        assert_eq!(http(&shared, "GET", "/assets/secret.txt").status, 404);
    }

    #[test]
    fn panel_dir_override_applies() {
        let dir = case_dir("override");
        let custom = dir.join("custom");
        fs::create_dir_all(&custom).expect("建覆盖目录");
        fs::write(custom.join("index.html"), b"<p>custom</p>").expect("写覆盖页");
        let shared = Arc::new(ServeShared {
            roots: vec![dir.join("runs")],
            panel_dir: Some(custom),
            broadcaster: Broadcaster::new(),
            connections: AtomicUsize::new(0),
        });
        let response = http(&shared, "GET", "/");
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"<p>custom</p>".to_vec());
        // 覆盖目录缺文件 → 404（fail-closed：开发期立刻发现断链）。
        assert_eq!(http(&shared, "GET", "/assets/app.js").status, 404);
    }

    // ---- §11.3 SSE 与端到端 ----

    /// 第二写者：经 **真实 Session**（宿主同一条 dispatch → 持久化临界区链）落盘。
    /// （测试二进制无法把自身当 CLI 子进程 spawn——`current_exe()` 是 libtest
    /// harness；此处以独立 Session + 真实持久化端口承载「另一写者」的事实，
    /// serve 观察到的变更全部来自盘上文件的真变化。）
    fn dispatch_external(
        mod_dir: &Path,
        runs_root: &Path,
        run_id: &str,
        event: &str,
        payload: &str,
    ) {
        use crate::Drive;
        let args = crate::Args {
            mod_dir: Some(mod_dir.to_path_buf()),
            run_id: run_id.to_owned(),
            runs_root: runs_root.to_path_buf(),
            serve_runs_roots: Vec::new(),
            resume: true,
            settle: false,
            watch: false,
            event: None,
            payload: None,
            drive: Drive::None,
            inbox: None,
            store: Some(Backend::Json),
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
            output: crate::OutputFormat::Text,
            serve: false,
            port: None,
            panel_dir: None,
            poll_ms: None,
        };
        let mut session = crate::Session::open(&args).expect("第二写者开局");
        let receipt = session.dispatch_event(event, payload);
        assert_eq!(
            receipt["ok"],
            Json::Bool(true),
            "{event} 派发失败：{receipt}"
        );
    }

    fn runs_of(serve: &LiveServe) -> Vec<String> {
        let (status, _, body) = serve.get("/api/runs");
        assert_eq!(status, 200);
        serde_json::from_slice::<Json>(&body).expect("JSON")["runs"]
            .as_array()
            .expect("runs")
            .iter()
            .filter_map(|row| row["runId"].as_str().map(str::to_owned))
            .collect()
    }

    fn wait_until_seen(serve: &LiveServe, run_id: &str) {
        let deadline = std::time::Instant::now() + Duration::from_secs(4);
        loop {
            if runs_of(serve).iter().any(|id| id == run_id) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "watcher 窗口内未拾取局 `{run_id}`"
            );
            std::thread::sleep(Duration::from_millis(40));
        }
    }

    #[test]
    fn sse_change_frame_on_external_dispatch() {
        let dir = case_dir("sse-e2e");
        let runs_root = dir.join("runs");
        let mod_dir = dir.join("mod");
        write_mod(&mod_dir, None);
        fs::create_dir_all(&runs_root).expect("建空 runs-root");
        let serve = LiveServe::start(&[runs_root.clone()], 50);
        // 先开局（外部写者），等 watcher 拾取。
        dispatch_external(&mod_dir, &runs_root, "x", "event.start", "0");
        wait_until_seen(&serve, "x");
        // SSE：连上流 → 外部写者再派发一步 → 窗口内收到 change 帧且 version 递增、run 指名。
        let (_, data) =
            serve.sse_frame(|event, data| event == "change" && data.contains("\"run\":\"x\""));
        let frame: Json = serde_json::from_str(&data).expect("帧是 JSON");
        assert!(frame["version"].as_u64().expect("version") >= 1, "{frame}");
        assert_eq!(
            frame["root"],
            json!(runs_root.display().to_string()),
            "帧必须点名变更局所属根：{frame}"
        );
        // 随后数据面出现新档、可取。
        let (status, _, body) = serve.get("/api/runs/x/saves");
        assert_eq!(status, 200);
        let saves = serde_json::from_slice::<Json>(&body).expect("JSON")["saves"]
            .as_array()
            .expect("saves")
            .clone();
        assert!(!saves.is_empty(), "派发后必有存档");
        let sid = saves[0]["id"].as_str().expect("sid").to_owned();
        let (status, _, _) = serve.get(&format!("/api/runs/x/saves/{sid}"));
        assert_eq!(status, 200);
    }

    #[test]
    fn sse_keepalive_and_disconnect() {
        // 缩短保活窗口注入：静置收 `: ping`；客户端断开后循环退出（线程不泄漏）。
        let (server, client) = loopback_pair();
        let broadcaster = Broadcaster::new();
        let handle = std::thread::spawn(move || {
            let mut server = server;
            sse_loop(&mut server, &broadcaster, Duration::from_millis(80));
        });
        let mut client = client;
        client
            .set_read_timeout(Some(Duration::from_millis(1_500)))
            .expect("读超时");
        let mut buf: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 128];
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !buf.windows(2).any(|window| window == b": ") {
            assert!(
                std::time::Instant::now() < deadline,
                "保活窗口内未收到 : ping"
            );
            match client.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(_) => break,
            }
        }
        let text = String::from_utf8_lossy(&buf);
        assert!(text.contains(": ping"), "静置必须收到保活注释：{text}");
        // 客户端断开 → sse_loop 写失败退出（join 成功即线程已结束）。
        drop(client);
        handle.join().expect("sse_loop 线程应随断开退出");
    }

    fn loopback_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind((BIND_ADDR, 0)).expect("绑定回环");
        let addr = listener.local_addr().expect("地址");
        let client = TcpStream::connect(addr).expect("连接回环");
        let (server, _) = listener.accept().expect("接受回环");
        (server, client)
    }

    #[test]
    fn serve_picks_up_new_run_late() {
        // 先起 serve（空 root）→ 后开局 → 一级列表出现该局（「先 serve 后开局」合法性的锁）。
        let dir = case_dir("late");
        let runs_root = dir.join("runs");
        let mod_dir = dir.join("mod");
        write_mod(&mod_dir, None);
        fs::create_dir_all(&runs_root).expect("建空 runs-root");
        let serve = LiveServe::start(&[runs_root.clone()], 50);
        let (status, _, body) = serve.get("/api/runs");
        assert_eq!(status, 200);
        assert_eq!(
            serde_json::from_slice::<Json>(&body).expect("JSON")["runs"],
            json!([]),
            "空 root 起服合法"
        );
        dispatch_external(&mod_dir, &runs_root, "late-run", "event.start", "0");
        wait_until_seen(&serve, "late-run");
    }

    /// 事件日志的**语义面**（去 timestamp 后逐行 JSON 等值）：
    /// 两局独立落盘，墙钟秒可能跨秒，timestamp 不参与纯度比对。
    fn events_semantic(path: &Path) -> Vec<Json> {
        String::from_utf8(fs::read(path).expect("读 events"))
            .expect("utf8")
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let mut value: Json = serde_json::from_str(line).expect("事件行 JSON");
                value.as_object_mut().expect("对象").remove("timestamp");
                value
            })
            .collect()
    }

    #[test]
    fn observe_purity_end_to_end() {
        // serve 起服 + 外部写者推进 + 全端点走过 + SSE 收帧之后：
        // 观察局的逻辑产物与「从未被 serve 观察过」的对照局**语义等值**——serve 读侧零扰动。
        let dir = case_dir("purity");
        let mod_dir = dir.join("mod");
        write_mod(&mod_dir, None);
        let watched_root = dir.join("watched");
        let control_root = dir.join("control");
        fs::create_dir_all(&watched_root).expect("建观察 root");
        fs::create_dir_all(&control_root).expect("建对照 root");
        // 对照局：与观察局**相同的写序列**（两步），serve 从未观察过它。
        dispatch_external(&mod_dir, &control_root, "p", "event.start", "0");
        // 观察局：serve 已在观察，先走一遍全部数据端点 + SSE。
        let serve = LiveServe::start(&[watched_root.clone()], 50);
        dispatch_external(&mod_dir, &watched_root, "p", "event.start", "0");
        wait_until_seen(&serve, "p");
        for target in [
            "/api/runs",
            "/api/runs/p/saves",
            "/api/runs/p/saves/latest",
            "/api/runs/p/events?since=0&limit=10",
            "/api/runs/p/artifacts",
        ] {
            let (status, _, _) = serve.get(target);
            assert_eq!(status, 200, "{target}");
        }
        let (_, data) = serve.sse_frame(|event, _| event == "change");
        assert!(data.contains("\"version\""), "{data}");
        // 第 2 步（SSE 连着）：两局同序各推一步，终态才可比。
        dispatch_external(&mod_dir, &watched_root, "p", "ygg.choice_selected", "1");
        wait_until_seen(&serve, "p");
        dispatch_external(&mod_dir, &control_root, "p", "ygg.choice_selected", "1");
        // 纯度断言：autosave 逐字节等值；events 语义等值（timestamp 排除）；
        // run.json 比 run_id/mod/store 字段面（updated_at 是写者的墙钟）。
        assert_eq!(
            fs::read(watched_root.join("p/autosave.json")).expect("读观察局存档"),
            fs::read(control_root.join("p/autosave.json")).expect("读对照局存档"),
            "autosave 必须逐字节等值（serve 零扰动）"
        );
        assert_eq!(
            events_semantic(&watched_root.join("p/events.jsonl")),
            events_semantic(&control_root.join("p/events.jsonl")),
            "events 必须语义等值（serve 零扰动）"
        );
        let field = |root: &Path, key: &str| -> Json {
            serde_json::from_str::<Json>(
                &fs::read_to_string(root.join("p/run.json")).expect("读 run.json"),
            )
            .expect("解析 run.json")[key]
                .clone()
        };
        for key in ["run_id", "mod", "store"] {
            assert_eq!(
                field(&watched_root, key),
                field(&control_root, key),
                "run.json 的 {key} 面等值"
            );
        }
    }

    // ---- §11.5 多根与单实例（r9）----

    #[test]
    fn parse_serve_config_fail_closed() {
        let home = case_dir("cfg-home");
        // 合法：绝对路径 + `~/` 展开。
        let roots =
            parse_serve_config(r#"{"roots":["/tmp/a", "~/b"]}"#, Some(&home)).expect("合法配置");
        assert_eq!(roots, vec![PathBuf::from("/tmp/a"), home.join("b")]);
        // 未知键 → Err（G5：{"root":[...]} 必须被点名拒绝）。
        let err = parse_serve_config(r#"{"root":["/tmp"]}"#, Some(&home)).unwrap_err();
        assert!(err.contains("未知键") && err.contains("root"), "{err}");
        // 相对路径 → Err（「相对于谁」无解，拒绝优于猜测）。
        let err = parse_serve_config(r#"{"roots":["rel/path"]}"#, Some(&home)).unwrap_err();
        assert!(err.contains("相对路径"), "{err}");
        // 畸形 JSON / 顶层非对象 / roots 非数组 / 非字符串项 / 空数组。
        assert!(parse_serve_config("{not json", Some(&home)).is_err());
        assert!(parse_serve_config("[]", Some(&home)).is_err());
        assert!(parse_serve_config(r#"{"roots":"/tmp"}"#, Some(&home)).is_err());
        assert!(parse_serve_config(r#"{"roots":[1]}"#, Some(&home)).is_err());
        assert!(parse_serve_config(r#"{"roots":[]}"#, Some(&home)).is_err());
        // `~` 但无 home → Err。
        assert!(parse_serve_config(r#"{"roots":["~/b"]}"#, None).is_err());
    }

    #[test]
    fn resolve_roots_chain() {
        let cwd = case_dir("chain-cwd");
        fs::create_dir_all(cwd.join("runs")).expect("建缺省根");
        let home = case_dir("chain-home");
        let config_text = r#"{"roots":["/tmp/chain-cfg"]}"#;
        let flag = cwd.join("flag-root");
        // 旗标 > 配置 > 缺省（./runs 的 cwd 绝对形态）。
        assert_eq!(
            resolve_roots(&[flag.clone()], Some(config_text), Some(&home), &cwd).expect("旗标层"),
            vec![flag]
        );
        assert_eq!(
            resolve_roots(&[], Some(config_text), Some(&home), &cwd).expect("配置层"),
            vec![PathBuf::from("/tmp/chain-cfg")]
        );
        assert_eq!(
            resolve_roots(&[], None, Some(&home), &cwd).expect("缺省层"),
            vec![cwd.join("runs")]
        );
    }

    #[test]
    fn canonicalize_roots_dedupes_aliases() {
        let dir = case_dir("dedupe");
        let real = dir.join("real");
        fs::create_dir_all(&real).expect("建真实根");
        let mut raw = vec![real.clone(), real.clone()];
        #[cfg(unix)]
        {
            let alias = dir.join("alias");
            std::os::unix::fs::symlink(&real, &alias).expect("建别名软链");
            raw = vec![real.clone(), alias];
        }
        let cwd = case_dir("dedupe-cwd");
        let roots = canonicalize_roots(&raw, &cwd);
        assert_eq!(
            roots,
            vec![real.canonicalize().expect("canonicalize")],
            "同一目录的双别名必须合并为一根"
        );
    }

    #[test]
    fn roots_cover_semantics() {
        let a = PathBuf::from("/a");
        let b = PathBuf::from("/b");
        let c = PathBuf::from("/c");
        assert!(roots_cover(&[], &[a.clone()]));
        assert!(roots_cover(&[a.clone()], &[a.clone(), b.clone()]));
        assert!(roots_cover(
            &[a.clone(), b.clone()],
            &[b.clone(), a.clone()]
        ));
        assert!(!roots_cover(
            &[a.clone(), c.clone()],
            &[a.clone(), b.clone()]
        ));
        assert!(!roots_cover(&[c], &[]));
    }

    #[test]
    fn locate_run_and_409() {
        let dir = case_dir("locate");
        let root_a = dir.join("a-runs");
        let root_b = dir.join("b-runs");
        seed_run(&root_a, "solo", Backend::Json, 1);
        seed_run(&root_a, "clash", Backend::Json, 1);
        seed_run(&root_b, "clash", Backend::Json, 1);
        let shared = shared_for_roots(&[root_a, root_b]);
        // 0 命中 → 404（既有文案）；1 命中 → 200；撞名 → 409 + error.roots。
        assert_eq!(http(&shared, "GET", "/api/runs/missing/saves").status, 404);
        assert_eq!(http(&shared, "GET", "/api/runs/solo/saves").status, 200);
        let response = http(&shared, "GET", "/api/runs/clash/saves");
        assert_eq!(response.status, 409);
        let body = body_json(&response);
        assert_eq!(body["error"]["code"], json!("run_id_ambiguous"));
        let roots = body["error"]["roots"].as_array().expect("error.roots");
        assert_eq!(roots.len(), 2, "409 必须列出全部撞名根：{body}");
        // 撞名在 /panel 与存档数据 API 上同门（同一 locate_run 事实）。
        assert_eq!(http(&shared, "GET", "/panel/clash/").status, 409);
        assert_eq!(
            http(&shared, "GET", "/api/runs/clash/saves/latest").status,
            409
        );
    }

    #[test]
    fn runs_api_lists_all_roots_with_row_root() {
        let dir = case_dir("multi-runs");
        let root_a = dir.join("a-runs");
        let root_b = dir.join("b-runs");
        seed_run(&root_a, "in-a", Backend::Json, 1);
        seed_run(&root_b, "in-b", Backend::Json, 2);
        let shared = shared_for_roots(&[root_a.clone(), root_b.clone()]);
        let body = body_json(&http(&shared, "GET", "/api/runs"));
        // roots 数组恰两根（= 共享态的观察根形态）；每行带 root 标识。
        assert_eq!(
            body["roots"],
            json!([root_a.display().to_string(), root_b.display().to_string(),])
        );
        let runs = body["runs"].as_array().expect("runs");
        assert_eq!(runs.len(), 2, "跨根各见其局");
        for row in runs {
            assert!(row["root"].is_string(), "行必须带根标识：{row}");
            assert!(row["runsRoot"].is_null(), "单数 runsRoot 必须退役：{row}");
        }
        assert!(body["runsRoot"].is_null(), "单数 runsRoot 必须退役");
        // 可读根照常出行；坏根（运行中目录被删）→ errors 记录 + 整体仍 200。
        let broken_root = dir.join("gone-runs");
        fs::create_dir_all(&broken_root).expect("建将被删除的根");
        let shared = shared_for_roots(&[root_a, broken_root.clone()]);
        fs::remove_dir_all(&broken_root).expect("删根模拟运行中目录被删");
        let response = http(&shared, "GET", "/api/runs");
        assert_eq!(response.status, 200);
        let body = body_json(&response);
        let errors = body["errors"].as_array().expect("errors");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0]["root"], json!(broken_root.display().to_string()));
        assert!(
            !body["runs"].as_array().expect("runs").is_empty(),
            "好根照常出行（局部真优于全黑）"
        );
    }

    #[test]
    fn run_mod_dir_two_level_chain() {
        let dir = case_dir("mod-chain");
        // 规范布局：<project>/.ygg/runs 的 mod 相对路径在 <project>（祖父目录）命中。
        let project = dir.join("proj");
        let root = project.join(".ygg/runs");
        let mods = project.join(".ygg/mods/m");
        fs::create_dir_all(&root).expect("建根");
        fs::create_dir_all(&mods).expect("建 mod");
        let resolved =
            run_mod_dir(&json!({"mod": ".ygg/mods/m"}), Some(&root)).expect("解析链命中");
        assert_eq!(
            resolved,
            mods,
            "祖父目录（规范布局 <project>）命中：{}",
            resolved.display()
        );
        // 扁平布局：<project>/runs 的相对路径在父目录命中。
        let flat_root = dir.join("flat/runs");
        let flat_mods = dir.join("flat/mods/m");
        fs::create_dir_all(&flat_root).expect("建扁平根");
        fs::create_dir_all(&flat_mods).expect("建扁平 mod");
        let resolved = run_mod_dir(&json!({"mod": "mods/m"}), Some(&flat_root)).expect("扁平命中");
        assert_eq!(resolved, flat_mods);
        // 皆不存 → 原值返回（PanelDecl::Unavailable 降级，不炸观察）。
        let root_no_mod = dir.join("nomod/runs");
        fs::create_dir_all(&root_no_mod).expect("建根");
        let resolved = run_mod_dir(&json!({"mod": ".ygg/mods/nowhere"}), Some(&root_no_mod))
            .expect("原值兑底");
        assert_eq!(resolved, PathBuf::from(".ygg/mods/nowhere"));
        // 旧名已退役（D-MA19）：相对旧名不再翻译——链不命中 → 原值返回；
        // panel 面给专名降级理由，点名规范路径（观察面不炸）。
        let retired_root = dir.join("retired/runs");
        fs::create_dir_all(&retired_root).expect("建退役根");
        let resolved = run_mod_dir(
            &json!({"mod": ".ygg/mods/pi-agent-seed"}),
            Some(&retired_root),
        )
        .expect("原值兑底（不再翻译）");
        assert_eq!(resolved, PathBuf::from(".ygg/mods/pi-agent-seed"));
        let retired_abs = dir.join("mods/pi-agent-seed"); // 绝对旧名同样不命中链
        let resolved = run_mod_dir(&json!({"mod": retired_abs.display().to_string()}), None)
            .expect("绝对旧名原样返回");
        assert_eq!(resolved, retired_abs);
        let PanelDecl::Unavailable { reason } = panel_decl(&retired_abs) else {
            panic!("退役旧名必须不可用");
        };
        assert!(
            reason.contains("agent-seed"),
            "理由必须点名规范路径：{reason}"
        );
        assert!(reason.contains("退役"), "理由必须宣布退役：{reason}");
        // 绝对路径原样；缺 mod 字段 → None。
        assert_eq!(
            run_mod_dir(&json!({"mod": "/abs/mod"}), None).expect("绝对原样"),
            PathBuf::from("/abs/mod")
        );
        assert!(run_mod_dir(&json!({}), Some(&root)).is_none());
    }

    #[test]
    fn probe_verdicts_end_to_end() {
        let dir = case_dir("probe");
        let root = dir.join("runs");
        fs::create_dir_all(&root).expect("建根");
        let serve = LiveServe::start(&[root.clone()], 50);
        // 覆盖 → Reuse；不覆盖 → RootsNotCovered（点名双方 roots）。
        match probe_live_instance(serve.port, &[root.clone()]) {
            InstanceVerdict::Reuse => {}
            other => panic!("覆盖请求必须 Reuse：{other:?}"),
        }
        let outsider = case_dir("probe-outsider");
        match probe_live_instance(serve.port, &[outsider]) {
            InstanceVerdict::RootsNotCovered { served, .. } => {
                assert_eq!(served, vec![root], "在役 roots 必须如实回显");
            }
            other => panic!("不覆盖请求必须 RootsNotCovered：{other:?}"),
        }
        // 端口上无 egg → NoInstance（connect refused）。
        let listener = TcpListener::bind((BIND_ADDR, 0)).expect("占一个空闲端口再放手");
        let port = listener.local_addr().expect("端口").port();
        drop(listener);
        assert!(
            matches!(
                probe_live_instance(port, &[dir.join("any")]),
                InstanceVerdict::NoInstance
            ),
            "空端口必须判无实例"
        );
    }

    #[test]
    fn probe_detects_legacy_instance() {
        // 旧代单根实例：/api/runs 响应含 runsRoot → LegacyInstance（升级窗口一次性路径）。
        let listener = TcpListener::bind((BIND_ADDR, 0)).expect("绑定假旧代实例");
        let port = listener.local_addr().expect("端口").port();
        let handle = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                // 先排空请求再应答（真实 HTTP 服务器行为；带未读数据关连 = RST，
                // 会把探针的 read_to_end 炸成 ECONNRESET 而不是干净 EOF）。
                let mut drain = [0u8; 512];
                let _ = stream.read(&mut drain);
                let body = "{\"runs\":[],\"runsRoot\":\"/old/gen\"}";
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(body.as_bytes());
                let _ = stream.flush();
            }
        });
        match probe_live_instance(port, &[PathBuf::from("/whatever")]) {
            InstanceVerdict::LegacyInstance => {}
            other => panic!("runsRoot 响应必须判旧代实例：{other:?}"),
        }
        handle.join().expect("假服务线程收尾");
    }

    #[test]
    fn watcher_picks_up_second_root() {
        // B 根后开一局 → watcher 必须跨根拾取（外层循环根的内层循环局）。
        let dir = case_dir("watch-2root");
        let root_a = dir.join("a-runs");
        let root_b = dir.join("b-runs");
        let mod_dir = dir.join("mod");
        write_mod(&mod_dir, None);
        fs::create_dir_all(&root_a).expect("建根A");
        fs::create_dir_all(&root_b).expect("建根B");
        let serve = LiveServe::start(&[root_a, root_b.clone()], 50);
        dispatch_external(&mod_dir, &root_b, "late-b", "event.start", "0");
        wait_until_seen(&serve, "late-b");
        let (status, _, body) = serve.get("/api/runs/late-b/saves");
        assert_eq!(status, 200, "第二根的局必须可下钻");
        let body: Json = serde_json::from_slice(&body).expect("JSON");
        assert_eq!(body["runId"], json!("late-b"));
    }

    // ---- 互斥矩阵（serve × 全部模式旗标 → usage_error；§11.4 验收映射）----

    /// 矩阵基准 = solo（除 drive_is_stdin 外全 false：serve 单独就是合法形态）。
    fn solo() -> crate::ServeFlags {
        crate::ServeFlags {
            drive_is_stdin: true,
            ..crate::ServeFlags::default()
        }
    }

    #[test]
    fn serve_usage_mutex_matrix() {
        use crate::{serve_usage_conflict, ServeFlags};
        // serve 单独（观察台本体）→ 无冲突。
        assert!(serve_usage_conflict(&solo()).is_none());
        // serve × 任一模式旗标 → 冲突（每条独立命中，矩阵无漏位）。
        let mode_flags = [
            (
                "--settle",
                ServeFlags {
                    settle: true,
                    ..solo()
                },
            ),
            (
                "--event",
                ServeFlags {
                    event: true,
                    ..solo()
                },
            ),
            (
                "--wait-tools",
                ServeFlags {
                    wait_tools: true,
                    ..solo()
                },
            ),
            (
                "inbox 写命令",
                ServeFlags {
                    inbox: true,
                    ..solo()
                },
            ),
            (
                "--run-tool-worker",
                ServeFlags {
                    run_tool_worker: true,
                    ..solo()
                },
            ),
            (
                "--artifact-put",
                ServeFlags {
                    artifact_put: true,
                    ..solo()
                },
            ),
            (
                "--artifact-get",
                ServeFlags {
                    artifact_get: true,
                    ..solo()
                },
            ),
            (
                "--daemon",
                ServeFlags {
                    daemon: true,
                    ..solo()
                },
            ),
            (
                "--resume",
                ServeFlags {
                    resume: true,
                    ..solo()
                },
            ),
            (
                "--watch",
                ServeFlags {
                    watch: true,
                    ..solo()
                },
            ),
            (
                "--store",
                ServeFlags {
                    store: true,
                    ..solo()
                },
            ),
            (
                "--payload",
                ServeFlags {
                    payload: true,
                    ..solo()
                },
            ),
            (
                "--harness",
                ServeFlags {
                    harness: true,
                    ..solo()
                },
            ),
            (
                "--tool-timeout",
                ServeFlags {
                    tool_timeout: true,
                    ..solo()
                },
            ),
            (
                "--drive",
                ServeFlags {
                    drive_is_stdin: false,
                    ..ServeFlags::default()
                },
            ),
        ];
        for (label, flags) in &mode_flags {
            let conflict =
                serve_usage_conflict(flags).unwrap_or_else(|| panic!("serve × {label} 必须冲突"));
            assert!(
                conflict.contains("不能与任何推进/写面旗标"),
                "{label}: {conflict}"
            );
        }
        // serve × --run-id / --mod → 拒单局绑定（观察单位是 runs-root）。
        for (label, flags) in [
            (
                "--run-id",
                ServeFlags {
                    run_id: true,
                    ..solo()
                },
            ),
            (
                "--mod",
                ServeFlags {
                    mod_dir: true,
                    ..solo()
                },
            ),
        ] {
            let conflict =
                serve_usage_conflict(&flags).unwrap_or_else(|| panic!("serve × {label} 必须冲突"));
            assert!(conflict.contains("--run-id / --mod"), "{label}: {conflict}");
        }
    }

    // ---- 宿主替代不变量的物理锁（§6.2）----

    /// serve 的**生产面**不得出现任何写路径：`ygg_store::open` / `RunStore` 写方法 /
    /// `Session` 构造。扫描范围 = 本文件 `mod serve_tests` 之前的生产面（测试模块里的
    /// 第二写者夹具合法地使用 Session——它扮演的是 serve 之外的「另一写者」）。
    /// 探针串经拼接构造，以免测试源码自匹配。
    #[test]
    fn serve_source_has_no_write_path() {
        const SOURCE: &str = include_str!("mod.rs");
        let production = SOURCE.split("mod serve_tests").next().unwrap_or(SOURCE);
        let forbidden = [
            format!("ygg_store::{}(", "open"),
            format!(".{}(", "commit_step"),
            format!(".{}(", "set_save"),
            format!(".{}(", "reset"),
            format!(".{}_{}(", "inbox", "push"),
            ["Sess", "ion::open("].concat(),
            ["Sess", "ion::open_"].concat(),
        ];
        for needle in &forbidden {
            assert!(
                !production.contains(needle.as_str()),
                "serve 生产面出现写路径 `{needle}`（写面只归 CLI/daemon）"
            );
        }
        // 生产面必须保有零写入的正面证据：query_only 观察面走 observe 端口。
        assert!(
            production.contains("observe::"),
            "serve 必须经 observe 观察面读"
        );
    }
}
