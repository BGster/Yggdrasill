//! serve 模式：常驻只读观察台（r1 · rev2 注册表化）。
//!
//! # 定位
//!
//! 继单发 `--event` / 交互 / `--daemon` 之后的第四模式：**加性**宿主执行面。
//! 面板只读，gate/accept 类人类门禁（`proposal.accept` / `gate.approve` /
//! `coordinate.submit` / `verify.submit` / `review.note`）在 serve 面上**不存在
//! 写路径**——数据端点只登记 GET；唯一例外 = `/api/roots` 观察根注册表
//! （POST/DELETE），那是**进程内观察配置**（不写存档、不写局状态、不建目录），
//! 与局状态写（事件/门禁/inbox）严格对象分离（docs/design/host/serve.md §3.7.6）。
//!
//! # 观察根注册表（r1）
//!
//! `egg serve` 空根启动；观察根经运行时 API 注册：`POST /api/roots
//! {"root": "<绝对路径>"}`（runs 父目录或单个 run-root 均可，注册时按 run.json
//! 存在性探测归类）、`DELETE /api/roots?root=<绝对路径>`（注销，幂等）、
//! `GET /api/roots`（自描述）。注册表仅在当前进程内有效（重启为空）；
//! `~/.config/ygg/serve.json` 与 `--runs-root` 旗标已退役（启动给迁移指引）。
//!
//! # 三级形态（P11 宿主零语义）
//!
//! ① runs 列表（egg 原生，注册表全量枚举，行带根标识）；② 局内存档列表
//! （egg 原生，`ygg_store::observe` 只读观察面，跨根统一 run-id 寻址，撞名 409）；
//! ③ 存档视图 = **mod 自带 UI**（iframe 托管，
//! egg 不解析不渲染任何 mod 领域语义，只供存档数据 API 裸数据）。
//!
//! # 全局唯一实例
//!
//! serve 是**按端口的全局唯一实例**（缺省 127.0.0.1:7800）：启动前先做只读探针
//! （手写 `GET /api/roots`，1s 超时，零锁文件——端口即事实）——
//! - 自家实例 → 打印面板 URL 幂等复用（退出 0，nohup 脚本安全；注册表由调用方
//!   自行 POST 扩容，启动面不比较根集）；
//! - 旧代实例（无 `/api/roots` 端点）→ 退出 1 + 重启指引；
//! - 无自家实例 → 照常 bind；bind 失败（外姓占用）= 原样报错退出 1。
//!
//! # 零内核（serve 纯度）
//!
//! serve **不装载 ModBundle/kernel/mlua，不构造 `Session`**：①不 create_dir_all/
//! 建库（观察面先探测后连接；注册面同样零创建——目录不存在即拒绝注册）；
//! ②不 write_meta；③不 mount 工具运行器；④不 drain/dispatch；⑤只读
//! `save_get`/`save_list`（经 [`ygg_store::observe`]）。物理锁：
//! `serve_source_has_no_write_path` + `registry_writes_nothing_on_disk`。
//!
//! # 线程模型
//!
//! 主线程 accept 循环（每连接一线程，并发封顶 64，超出 503）；watcher 线程轮询
//! 注册表做纯变更检测（无 Session、无内核）；SSE 每连接在自身线程上
//! `wait_timeout` 等待广播。数据面即时读（文件 stat + 轻量 SQL，毫秒级）。
//! 共享可变状态 = `version` 广播对 + `roots` 注册表（RwLock，读多写少）。
//!
//! # 安全边界
//!
//! `127.0.0.1` 绑定常量（无旗标）；数据面 GET-only（/api/roots 白名单
//! POST/DELETE）；无凭据面；解析健壮性（32 KiB 头上限 / 5s 超时 / 64 连接 /
//! 路径白名单 / query 钳制 / run-id 语法门）；无 CORS；`/panel` 双守卫
//! （canonicalize + 前缀，拦 `..` 与 symlink 逃逸）+ nosniff。

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use include_dir::include_dir;
use serde_json::{json, Value as Json};
use ygg_store::observe;

use crate::Args;
// ============================================================================
// §拆分模块（纯搬移目录化：生产面 6 件；子文件 use super::* 取父名空间，
// 根级 pub(crate) glob 再导出保 serve 平铺名空间——批一 egg 目录化先例）
// ============================================================================

mod handlers;
mod http;
mod locate;
mod panels;
mod roots;
mod sse;

pub(crate) use handlers::*;
pub(crate) use http::*;
pub(crate) use locate::*;
pub(crate) use panels::*;
pub(crate) use roots::*;
pub(crate) use sse::*;

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

// ============================================================================
// §共享状态（唯一共享可变物：version 广播对）
// ============================================================================

/// 注册表条目：runs 父目录（枚举其下各局）或单个 run-root（只观察这一局）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RootEntry {
    /// canonicalize 后的绝对形态（比较、行级 root 标识的唯一事实形态）。
    path: PathBuf,
    /// true = 单局根（注册时 `<path>/run.json` 存在）；false = runs 父目录。
    single_run: bool,
}

/// serve 进程的共享配置 + 观察根注册表 + 连接计数。
pub(crate) struct ServeShared {
    /// 观察根注册表（r1：运行时 POST/DELETE /api/roots 维护；进程内易失）。
    /// RwLock：数据面高频读（watcher 轮询 + 每请求扫描），注册/注销低频写。
    roots: RwLock<Vec<RootEntry>>,
    /// 内置页的盘上覆盖（`--panel-dir`；开发期改 HTML 刷新即生效）。
    panel_dir: Option<PathBuf>,
    broadcaster: Broadcaster,
    connections: AtomicUsize,
}

impl ServeShared {
    /// 注册表快照（读侧统一入口；锁不跨 IO——先克隆再扫盘）。
    fn roots_snapshot(&self) -> Vec<RootEntry> {
        self.roots
            .read()
            .map(|roots| roots.clone())
            .unwrap_or_default()
    }
}

// ============================================================================
// §单实例探针（r1：自家实例幂等复用，不再比较根集——根集由运行时注册扩容）
// ============================================================================

/// 在役实例判定（[`probe_live_instance`] 的产物；纯数据可测）。
#[derive(Debug)]
enum InstanceVerdict {
    /// 自家实例（`/api/roots` 应答 egg 形状）→ 幂等复用（退出 0）。
    Reuse,
    /// 旧代实例（无 `/api/roots` 端点）→ 拒绝（退出 1 + 重启指引）。
    LegacyInstance,
    /// 端口上没有自家实例（连接失败 / 非 200 / 非 egg 形状）→ 照常 bind。
    NoInstance,
}

/// 单实例探针（**纯读**：手写 `GET /api/roots`，1s 超时，零锁文件——端口即事实）。
/// 判定规则见模块头「全局唯一实例」节；探针↔bind 之间的竞态由 bind 失败路径兜住。
fn probe_live_instance(port: u16) -> InstanceVerdict {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, PROBE_TIMEOUT) else {
        return InstanceVerdict::NoInstance;
    };
    let _ = stream.set_read_timeout(Some(PROBE_TIMEOUT));
    let _ = stream.set_write_timeout(Some(PROBE_TIMEOUT));
    let request = "GET /api/roots HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
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
    if status == 404 || status == 405 {
        // 旧代实例：路由表里没有 /api/roots（r9 及更早的 GET-only 面）。
        return InstanceVerdict::LegacyInstance;
    }
    if status != 200 {
        return InstanceVerdict::NoInstance;
    }
    let Ok(body) = serde_json::from_slice::<Json>(&raw[split + 4..]) else {
        return InstanceVerdict::NoInstance;
    };
    // egg serve 的正面证据：`roots` 数组（缺它 = 外姓服务，让 bind 失败路径说话）。
    if body.get("roots").and_then(Json::as_array).is_none() {
        return InstanceVerdict::NoInstance;
    }
    InstanceVerdict::Reuse
}

// ============================================================================
// §入口与线程编排
// ============================================================================

/// serve 模式主入口（`egg serve [--port …] [--panel-dir …] [--poll-ms …]`）。
/// 空根启动：观察根经运行时 `POST /api/roots` 注册（进程内注册表，重启为空）。
pub(crate) fn run_serve(args: &Args) -> i32 {
    let home = super::home_dir();
    // 退役配置的迁移指引（不自动导入——「面板在观察什么」必须显式注册）。
    if let Some(home) = home.as_deref() {
        let legacy = home.join(SERVE_CONFIG_REL);
        if let Ok(text) = std::fs::read_to_string(&legacy) {
            let old_roots = serde_json::from_str::<Json>(&text)
                .ok()
                .and_then(|value| {
                    value["roots"].as_array().map(|array| {
                        array
                            .iter()
                            .filter_map(|item| item.as_str().map(str::to_owned))
                            .collect::<Vec<_>>()
                    })
                })
                .unwrap_or_default();
            eprintln!(
                "serve: 注意 {} 已退役（不会自动导入）。如需观察旧根，请逐一注册：",
                legacy.display()
            );
            for root in &old_roots {
                eprintln!(
                    "  curl -X POST http://127.0.0.1:{}/api/roots -H 'Content-Type: application/json' -d '{{\"root\": \"{}\"}}'",
                    args.port.unwrap_or(DEFAULT_PORT),
                    root
                );
            }
        }
    }
    // 单实例探针（先于 bind；探针↔bind 之间的竞态由 bind 失败路径兜住）。
    let port = args.port.unwrap_or(DEFAULT_PORT);
    match probe_live_instance(port) {
        InstanceVerdict::Reuse => {
            println!(
                "serve: 已有全局唯一实例在役，复用 http://{BIND_ADDR}:{port}；\
                 本次调用幂等退出 0（观察根用 POST /api/roots 注册扩容）"
            );
            return 0;
        }
        InstanceVerdict::LegacyInstance => {
            eprintln!(
                "error: 端口 {port} 上是旧版 serve 实例（无 /api/roots 端点）。\
                 请停掉旧实例（Ctrl-C / pkill -f 'egg.*serve'）后重启以启用注册表化观察"
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
        roots: RwLock::new(Vec::new()),
        panel_dir: args.panel_dir.clone(),
        broadcaster: Broadcaster::new(),
        connections: AtomicUsize::new(0),
    });
    eprintln!(
        "serve: 只读观察台已启动 http://{BIND_ADDR}:{bound}（空根启动；\
         注册观察根：curl -X POST http://{BIND_ADDR}:{bound}/api/roots \
         -d '{{\"root\": \"<绝对路径>\"}}'；Ctrl-C 退出）"
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

/// 测试/装配共用：绑定一个（测试用 port 0 由 OS 分配）监听器。
#[cfg(test)]
fn bind_serve(
    roots: Vec<RootEntry>,
    panel_dir: Option<PathBuf>,
) -> std::io::Result<(TcpListener, Arc<ServeShared>)> {
    let listener = TcpListener::bind((BIND_ADDR, 0))?;
    let shared = Arc::new(ServeShared {
        roots: RwLock::new(roots),
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

/// 路由结果：普通响应或 SSE 流（SSE 在连接线程上长驻）。
enum Outcome {
    Http(Response),
    Sse,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// 存档 id 语法门：`latest` | 十进制 id（否则 400）。
fn valid_save_id(sid: &str) -> bool {
    sid == "latest" || (!sid.is_empty() && sid.bytes().all(|byte| byte.is_ascii_digit()))
}

// ============================================================================
// §路由（表里只有**结构名**，无一个 mod 事件名——P2/P3 的落点）
// ============================================================================

fn route(shared: &ServeShared, method: &str, raw_path: &str, query: &str, body: &[u8]) -> Outcome {
    // 方法白名单（物理边界）：数据面 GET-only；唯一例外 = /api/roots 注册面
    // （POST 注册 / DELETE 注销——进程内观察配置，不写局状态）。
    let path = percent_decode(raw_path);
    let segments: Vec<&str> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments.as_slice() == ["api", "roots"] {
        let outcome = match method {
            "GET" => handle_roots_get(shared),
            "POST" => handle_roots_post(shared, body),
            "DELETE" => handle_roots_delete(shared, query),
            _ => error_response(
                405,
                "method_not_allowed",
                "/api/roots 只支持 GET（注册表）/ POST（注册）/ DELETE（注销）",
            ),
        };
        return Outcome::Http(outcome);
    }
    if method != "GET" {
        return Outcome::Http(error_response(
            405,
            "method_not_allowed",
            "观察台数据端点只支持 GET（局状态写只经 CLI/daemon；gate/accept 类人类门禁不在线上）",
        ));
    }
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

    /// 测试装配：PathBuf → RootEntry（按 run.json 存在性归类，与 POST 注册同规则）。
    fn root_entries(roots: &[PathBuf]) -> Vec<RootEntry> {
        roots
            .iter()
            .map(|path| RootEntry {
                single_run: path.join(crate::META_FILE).is_file(),
                path: path.clone(),
            })
            .collect()
    }

    fn shared_for_roots(roots: &[PathBuf]) -> Arc<ServeShared> {
        Arc::new(ServeShared {
            roots: RwLock::new(root_entries(roots)),
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
        match route(shared, method, path, query, &[]) {
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
            let (listener, shared) = bind_serve(root_entries(roots), None).expect("绑定测试端口");
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
        let (listener, _shared) = bind_serve(root_entries(&[dir]), None).expect("绑定");
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
            roots: RwLock::new(root_entries(&[dir.join("runs")])),
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
        // r1 语义：本局已有产物 → Attach 续跑；全新目录 → New 开局
        // （旧 resume=true 的「缺档回退重置」路径已随 r1 删除）。
        let run_root = runs_root.join(run_id);
        let fresh = !ygg_store::detect(&run_root).any();
        let args = crate::Args {
            mod_dir: Some(mod_dir.to_path_buf()),
            open_style: Some(if fresh {
                crate::OpenStyle::New
            } else {
                crate::OpenStyle::Open
            }),
            run_root,
            runs_parent: runs_root.to_path_buf(),
            force: false,
            at: None,
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
            packages: None,
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

    // ---- §11.5 注册表与单实例（r1）----

    /// POST /api/roots 形状校验：相对路径 / 不存在目录 / 缺字段 / 非 JSON 全拒。
    #[test]
    fn roots_post_shape_validation() {
        let dir = case_dir("roots-shape");
        let shared = shared_for_roots(&[]);
        let post = |body: &str| {
            let (path, query) = ("/api/roots", "");
            match route(&shared, "POST", path, query, body.as_bytes()) {
                Outcome::Http(response) => response,
                Outcome::Sse => unreachable!("roots 不是 SSE"),
            }
        };
        // 缺 root 字段 / 非 JSON / 空串 → 400。
        assert_eq!(post("{}").status, 400);
        assert_eq!(post("not json").status, 400);
        assert_eq!(post(r#"{"root":""}"#).status, 400);
        // 相对路径 → 400（「相对于谁」无解，拒绝优于猜测）。
        let response = post(r#"{"root":"rel/path"}"#);
        assert_eq!(response.status, 400);
        assert!(String::from_utf8_lossy(&response.body).contains("绝对路径"));
        // 不存在目录 → 404（零创建）。
        let missing = dir.join("no-such-dir");
        let response = post(format!(r#"{{"root":"{}"}}"#, missing.display()).as_str());
        assert_eq!(response.status, 404);
        assert!(!missing.exists(), "注册失败不得创建目录");
        // 注册表仍为空。
        assert_eq!(
            body_json(&http(&shared, "GET", "/api/roots"))["roots"],
            json!([])
        );
    }

    /// POST 注册幂等 + DELETE 注销幂等 + 单局根归类 + 盘上零写。
    #[test]
    fn roots_register_unregister_idempotent_and_zero_disk_write() {
        let dir = case_dir("roots-registry");
        let parent = dir.join("runs");
        seed_run(&parent, "solo", Backend::Json, 1);
        let single = parent.join("solo");
        let shared = shared_for_roots(&[]);
        let post = |target: &Path| {
            let body = format!(r#"{{"root":"{}"}}"#, target.display());
            match route(&shared, "POST", "/api/roots", "", body.as_bytes()) {
                Outcome::Http(response) => response,
                Outcome::Sse => unreachable!(),
            }
        };
        // 注册父目录根：added:true；重复注册 added:false（幂等，无双行）。
        let first = body_json(&post(&parent));
        assert_eq!(first["added"], json!(true));
        assert_eq!(first["singleRun"], json!(false));
        let again = body_json(&post(&parent));
        assert_eq!(again["added"], json!(false));
        assert_eq!(again["roots"].as_array().expect("roots").len(), 1);
        // 注册单个 run-root：归类 singleRun:true。
        let single_response = body_json(&post(&single));
        assert_eq!(single_response["added"], json!(true));
        assert_eq!(single_response["singleRun"], json!(true));
        // 单局根只产该局行；父目录根也产该局（同一局两处可见 = 观察语义）。
        let runs = body_json(&http(&shared, "GET", "/api/runs"));
        assert_eq!(runs["runs"].as_array().expect("runs").len(), 2);
        // 注销：removed:true；再注销 removed:false（幂等）。
        let target = format!("/api/roots?root={}", single.display());
        assert_eq!(
            body_json(&http(&shared, "DELETE", &target))["removed"],
            json!(true)
        );
        assert_eq!(
            body_json(&http(&shared, "DELETE", &target))["removed"],
            json!(false)
        );
        // 注销后单局行消失，父目录行仍在。
        let runs = body_json(&http(&shared, "GET", "/api/runs"));
        assert_eq!(runs["runs"].as_array().expect("runs").len(), 1);
    }

    /// 注册表 cap：32 根上限，第 33 根 409。
    #[test]
    fn roots_registry_cap() {
        let dir = case_dir("roots-cap");
        let shared = shared_for_roots(&[]);
        for index in 0..ROOTS_CAP {
            let root = dir.join(format!("r{index}"));
            fs::create_dir_all(&root).expect("建根");
            let body = format!(r#"{{"root":"{}"}}"#, root.display());
            match route(&shared, "POST", "/api/roots", "", body.as_bytes()) {
                Outcome::Http(response) => assert_eq!(response.status, 200, "第 {index} 根"),
                Outcome::Sse => unreachable!(),
            }
        }
        let extra = dir.join("overflow");
        fs::create_dir_all(&extra).expect("建溢出根");
        let body = format!(r#"{{"root":"{}"}}"#, extra.display());
        match route(&shared, "POST", "/api/roots", "", body.as_bytes()) {
            Outcome::Http(response) => assert_eq!(response.status, 409),
            Outcome::Sse => unreachable!(),
        }
    }

    /// 物理锁：注册/注销路径盘上零写（目录树在操作前后逐字节一致）。
    #[test]
    fn registry_writes_nothing_on_disk() {
        let dir = case_dir("roots-zero-write");
        let parent = dir.join("runs");
        seed_run(&parent, "witness", Backend::Json, 1);
        let snapshot = |root: &Path| -> Vec<(String, u64)> {
            let mut entries: Vec<(String, u64)> = std::fs::read_dir(root)
                .expect("读目录")
                .flatten()
                .map(|entry| {
                    let meta = entry.metadata().expect("stat");
                    (entry.file_name().to_string_lossy().into_owned(), meta.len())
                })
                .collect();
            entries.sort();
            entries
        };
        let before = snapshot(&dir);
        let shared = shared_for_roots(&[]);
        let body = format!(r#"{{"root":"{}"}}"#, parent.display());
        match route(&shared, "POST", "/api/roots", "", body.as_bytes()) {
            Outcome::Http(response) => assert_eq!(response.status, 200),
            Outcome::Sse => unreachable!(),
        }
        let target = format!("/api/roots?root={}", parent.display());
        assert_eq!(http(&shared, "DELETE", &target).status, 200);
        assert_eq!(snapshot(&dir), before, "注册/注销不得改变盘上任何字节");
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
        // 旧名已退役（别名退役）：相对旧名不再翻译——链不命中 → 原值返回；
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
        // 自家实例（/api/roots 应答 egg 形状）→ Reuse（不再比较根集——注册表
        // 由调用方 POST 扩容，启动面无根集可比较）。
        match probe_live_instance(serve.port) {
            InstanceVerdict::Reuse => {}
            other => panic!("自家实例必须 Reuse：{other:?}"),
        }
        // 端口上无 egg → NoInstance（connect refused）。
        let listener = TcpListener::bind((BIND_ADDR, 0)).expect("占一个空闲端口再放手");
        let port = listener.local_addr().expect("端口").port();
        drop(listener);
        assert!(
            matches!(probe_live_instance(port), InstanceVerdict::NoInstance),
            "空端口必须判无实例"
        );
    }

    #[test]
    fn probe_detects_legacy_instance() {
        // 旧代实例：/api/roots 不在路由表（404）→ LegacyInstance（升级窗口一次性路径）。
        let listener = TcpListener::bind((BIND_ADDR, 0)).expect("绑定假旧代实例");
        let port = listener.local_addr().expect("端口").port();
        let handle = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                // 先排空请求再应答（真实 HTTP 服务器行为；带未读数据关连 = RST，
                // 会把探针的 read_to_end 炸成 ECONNRESET 而不是干净 EOF）。
                let mut drain = [0u8; 512];
                let _ = stream.read(&mut drain);
                let body = "{\"ok\":false,\"error\":{\"code\":\"not_found\"}}";
                let head = format!(
                    "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(body.as_bytes());
                let _ = stream.flush();
            }
        });
        match probe_live_instance(port) {
            InstanceVerdict::LegacyInstance => {}
            other => panic!("无 /api/roots 端点必须判旧代实例：{other:?}"),
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
        // serve × 位置参数（run-root / new / open 的残留）→ 拒单局绑定
        // （空根启动，观察根经 POST /api/roots 注册）。
        let flags = ServeFlags {
            positional: true,
            ..solo()
        };
        let conflict = serve_usage_conflict(&flags).expect("serve × 位置参数 必须冲突");
        assert!(conflict.contains("POST /api/roots"), "{conflict}");
    }

    // ---- 宿主替代不变量的物理锁（§6.2）----

    /// serve 的**生产面**不得出现任何写路径：`ygg_store::open` / `RunStore` 写方法 /
    /// `Session` 构造。扫描范围 = 七个生产文件 concat 后 `mod serve_tests` 之前的
    /// 生产面（C1 拆分时守卫同 commit 扩到全部生产文件，覆盖只增不缩；测试模块里的
    /// 第二写者夹具合法地使用 Session——它扮演的是 serve 之外的「另一写者」）。
    /// 探针串经拼接构造，以免测试源码自匹配。
    #[test]
    fn serve_source_has_no_write_path() {
        const SOURCE: &str = concat!(
            include_str!("http.rs"),
            include_str!("sse.rs"),
            include_str!("locate.rs"),
            include_str!("handlers.rs"),
            include_str!("roots.rs"),
            include_str!("panels.rs"),
            include_str!("mod.rs"),
        );
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
