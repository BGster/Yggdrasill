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

// ============================================================================
// §测试锁（§11.1–§11.3：路由与解析 / 两级列表与面板托管 / SSE 与端到端）
// ============================================================================

#[cfg(test)]
mod tests_common;
#[cfg(test)]
mod tests_http;
#[cfg(test)]
mod tests_data;
#[cfg(test)]
mod tests_registry_sse;

#[cfg(test)]
pub(crate) use tests_common::*;

