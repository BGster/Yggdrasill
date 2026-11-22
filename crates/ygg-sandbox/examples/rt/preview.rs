//! rt 预览端口（S31 R2）：内嵌 HTTP/SSE 推帧 + 输入回注。
//!
//! **为什么不挂 serve**（D5 裁定）：serve 契约 = 零内核不连运行进程
//! （`docs/design/host/serve.md`）；推帧的数据源在 rt 进程内，挂 serve 须
//! 先造 serve↔rt IPC 桥 = 破坏观察面架构。故预览端口**内嵌本文件**：
//! std TcpListener + HTTP/SSE（仿 serve/http.rs 先例，依赖零新增）。
//!
//! 线程纪律（零 unsafe、零 Rc 过线程）：
//! - accept 线程只收输入字节：POST /api/input 解析后经 mpsc 单向送主循环
//!   （frame 号由主循环落账时定为 `next_n`——与 `--input` 脚本同一条
//!   `InputDevice::at` 入口，录制语义不变，D7）。
//! - 帧快照由**主循环**发布进「最新帧槽」（Mutex<Option<(n, 快照)>>）：
//!   SSE 客户端各自轮询拉取 n 更新的帧——最新胜出语义，慢客户端自然丢帧
//!   （预览是尽力而为面，不背压游戏循环）。
//! - 回放模式同样可出帧（读侧通道，零确定性影响）；输入回注仅录制模式
//!   被主循环消费（回放确定性红线）。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ygg_sandbox::frame::FrameSnapshot;
use ygg_sandbox::rt::InputEvent;

/// 请求头/体读入上限（预览面只过小载荷）。
const REQUEST_CAP: usize = 64 * 1024;
/// 客户端拉取间隔（60fps 预览上限 ≈ 8ms 轮询步）。
const POLL: Duration = Duration::from_millis(8);
/// SSE 保活注释间隔。
const KEEPALIVE: Duration = Duration::from_secs(15);

/// 最新帧槽：`(帧号, 快照)`，主循环写、SSE 客户端读。
struct Latest(Mutex<Option<(u64, FrameSnapshot)>>);

impl Latest {
    fn publish(&self, n: u64, snap: &FrameSnapshot) {
        *self.0.lock().expect("latest lock") = Some((n, snap.clone()));
    }

    fn newer_than(&self, seen: u64) -> Option<(u64, FrameSnapshot)> {
        let slot = self.0.lock().expect("latest lock");
        match &*slot {
            Some((n, snap)) if *n > seen => Some((*n, snap.clone())),
            _ => None,
        }
    }
}

/// 预览服务器句柄（主循环持有）。
pub struct PreviewHandle {
    /// 主循环 ← accept 线程：输入回注（录制模式消费）。
    pub inputs: Receiver<InputEvent>,
    latest: Arc<Latest>,
    pub addr: std::net::SocketAddr,
}

impl PreviewHandle {
    /// 主循环发布一帧（present 过才调；读侧通道，零确定性影响）。
    pub fn publish(&self, n: u64, snap: &FrameSnapshot) {
        self.latest.publish(n, snap);
    }

    /// 主循环排空回注队列（录制模式；每帧循环顶调用）。
    pub fn drain_inputs(&self) -> Vec<InputEvent> {
        let mut out = Vec::new();
        loop {
            match self.inputs.try_recv() {
                Ok(event) => out.push(event),
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
        out
    }
}

/// 绑定 `127.0.0.1:port` 并拉起 accept 线程（失败 = 可读 IO 错误）。
pub fn spawn(port: u16) -> std::io::Result<PreviewHandle> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let addr = listener.local_addr()?;
    let latest = Arc::new(Latest(Mutex::new(None)));
    let (input_tx, input_rx) = std::sync::mpsc::channel::<InputEvent>();
    let worker_latest = Arc::clone(&latest);
    std::thread::Builder::new()
        .name("rt-preview-accept".to_owned())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let latest = Arc::clone(&worker_latest);
                let input_tx = input_tx.clone();
                // 一连接一线程：预览客户端数量级是个位数（人手开页面）。
                std::thread::Builder::new()
                    .name("rt-preview-conn".to_owned())
                    .spawn(move || serve_conn(stream, latest, input_tx))
                    .ok();
            }
        })?;
    Ok(PreviewHandle {
        inputs: input_rx,
        latest,
        addr,
    })
}

fn serve_conn(mut stream: TcpStream, latest: Arc<Latest>, inputs: Sender<InputEvent>) {
    let Some(request) = read_request(&mut stream) else {
        let _ = stream.write_all(
            b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        return;
    };
    let ok = match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/") => respond(stream, "text/html; charset=utf-8", demo_page().as_bytes()),
        ("GET", "/stream") => sse_loop(stream, &latest),
        ("POST", "/api/input") => {
            let outcome = match parse_input(&request.body) {
                Ok(event) => inputs.send(event).is_ok(),
                Err(message) => {
                    let _ = respond(stream, "text/plain; charset=utf-8", message.as_bytes());
                    return;
                }
            };
            let status = if outcome {
                "202 Input scheduled"
            } else {
                "503 not consuming"
            };
            respond_head(&mut stream, status, "text/plain; charset=utf-8", b"")
        }
        _ => respond(stream, "text/plain; charset=utf-8", b"not found\n"),
    };
    if !ok {
        // 客户端断开是常态（页面关闭），静默收线。
    }
}

struct Request {
    method: String,
    path: String,
    body: Vec<u8>,
}

/// 读一个 HTTP 请求（头 + Content-Length 体；超限/断开 = None）。
fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    let head_end = loop {
        if buf.len() > REQUEST_CAP {
            return None;
        }
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..read]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.lines();
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_owned();
    let path = parts.next()?.to_owned();
    let content_length = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .next()
        .unwrap_or(0);
    if content_length > REQUEST_CAP {
        return None;
    }
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < content_length {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(content_length);
    Some(Request { method, path, body })
}

fn respond_head(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) -> bool {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).is_ok() && stream.write_all(body).is_ok()
}

fn respond(mut stream: TcpStream, content_type: &str, body: &[u8]) -> bool {
    respond_head(&mut stream, "200 OK", content_type, body)
}

/// SSE 推帧循环（客户端断开 = write 失败自然收线）。
fn sse_loop(mut stream: TcpStream, latest: &Latest) -> bool {
    if !stream
        .write_all(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n"
                .as_bytes(),
        )
        .is_ok()
    {
        return false;
    }
    let mut seen = 0u64;
    let mut last_sent = Instant::now();
    loop {
        match latest.newer_than(seen) {
            Some((n, snap)) => {
                seen = n;
                let event = format!(
                    "data: {{\"n\":{},\"w\":{},\"h\":{},\"rgba\":\"{}\"}}\n\n",
                    n,
                    snap.w,
                    snap.h,
                    b64(rgba_bytes(&snap).as_slice()),
                );
                if stream.write_all(event.as_bytes()).is_err() {
                    return false;
                }
                let _ = stream.flush();
                last_sent = Instant::now();
            }
            None => {
                if last_sent.elapsed() > KEEPALIVE && stream.write_all(b": ping\n\n").is_err() {
                    return false;
                }
                std::thread::sleep(POLL);
            }
        }
    }
}

/// `POST /api/input` 体解析：`{"down":[..],"up":[..],"mouse":[dx,dy,buttons?]}`，
/// 键码/位移全整数（InputDevice 的校验在主循环落账时执行）。
fn parse_input(body: &[u8]) -> Result<InputEvent, String> {
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|err| format!("不是合法 JSON：{err}\n"))?;
    let codes = |name: &str| -> Result<Vec<i64>, String> {
        match value[name].as_array() {
            None => Ok(Vec::new()),
            Some(list) => list
                .iter()
                .map(|v| v.as_i64().ok_or(format!("{name} 含非整数键码\n")))
                .collect(),
        }
    };
    let (dx, dy, buttons) = match value["mouse"].as_array() {
        None => (0, 0, None),
        Some(list) if list.len() == 2 || list.len() == 3 => (
            list[0].as_i64().ok_or("mouse[0] 非整数\n")?,
            list[1].as_i64().ok_or("mouse[1] 非整数\n")?,
            list.get(2)
                .map(|v| v.as_i64().ok_or("mouse[2] 非整数\n"))
                .transpose()?,
        ),
        Some(_) => return Err("mouse 期望 [dx,dy] 或 [dx,dy,buttons]\n".to_owned()),
    };
    Ok(InputEvent {
        down: codes("down")?,
        up: codes("up")?,
        mouse_dx: dx,
        mouse_dy: dy,
        mouse_buttons: buttons,
    })
}

fn rgba_bytes(snap: &FrameSnapshot) -> Vec<u8> {
    let mut out = Vec::with_capacity(snap.pixels.len() * 4);
    for p in &snap.pixels {
        out.extend_from_slice(&p.to_be_bytes());
    }
    out
}

/// base64（标准字母表 + padding；预览面专用，零新增依赖）。
fn b64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let word = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(word >> 18) as usize & 63] as char);
        out.push(TABLE[(word >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(word >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[word as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// 演示页：canvas putImageData + EventSource + 键盘回注（≥1 用点自证）。
fn demo_page() -> String {
    format!(
        r#"<!doctype html>
<html><head><meta charset="utf-8"><title>ygg.rt.frame preview</title>
<style>
body{{background:#111;color:#ddd;font:14px/1.5 monospace;margin:1em}}
canvas{{image-rendering:pixelated;border:1px solid #444;background:#000}}
</style></head><body>
<p>ygg.rt.frame 预览 — <span id="fps">0</span> fps · 帧 <span id="n">-</span>（键入即回注 /api/input）</p>
<canvas id="c" width="320" height="200"></canvas>
<script>
const cv = document.getElementById('c'), ctx = cv.getContext('2d');
const es = new EventSource('/stream');
let frames = 0, t0 = performance.now();
es.onmessage = (e) => {{
  const f = JSON.parse(e.data);
  const raw = Uint8Array.from(atob(f.rgba), c => c.charCodeAt(0));
  cv.width = f.w; cv.height = f.h;
  ctx.putImageData(new ImageData(new Uint8ClampedArray(raw.buffer), f.w, f.h), 0, 0);
  document.getElementById('n').textContent = f.n;
  frames++;
  if (performance.now() - t0 > 1000) {{
    document.getElementById('fps').textContent = frames;
    frames = 0; t0 = performance.now();
  }}
}};
function keyOf(e) {{
  const map = {{ArrowLeft:2, ArrowRight:3, ArrowUp:4, ArrowDown:5,
               KeyA:6, KeyD:7, KeyW:8, KeyS:9, Space:10}};
  return map[e.code] !== undefined ? map[e.code] : (e.keyCode % 54) + 11;
}}
addEventListener('keydown', (e) => {{
  fetch('/api/input', {{method:'POST', body: JSON.stringify({{down:[keyOf(e)]}})}});
  e.preventDefault();
}});
addEventListener('keyup', (e) => {{
  fetch('/api/input', {{method:'POST', body: JSON.stringify({{up:[keyOf(e)]}})}});
}});
</script></body></html>
"#
    )
}

// 引用占位：POLL/KEEPALIVE 在 sse_loop 内消费（防 dead_code 误报的显式锚）。
const _: () = {
    assert!(POLL.as_millis() > 0 && KEEPALIVE.as_secs() > 0);
};
