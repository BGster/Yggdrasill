use super::*;

// ============================================================================
// §HTTP 子集（GET-only + close 定界；std 手写，零新依赖）
// ============================================================================

pub(crate) struct Request {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) query: String,
    /// 请求体（仅 `/api/roots` 注册面消费；数据面 GET 无 body 语义）。
    /// 上限 8 KiB（注册载荷只有一个路径字段，远超所需即拒绝）。
    body: Vec<u8>,
}

pub(crate) struct Response {
    pub(crate) status: u16,
    pub(crate) reason: &'static str,
    pub(crate) content_type: &'static str,
    /// 额外响应头（如存档数据 API 的 `X-Ygg-*`）。
    pub(crate) headers: Vec<(&'static str, String)>,
    pub(crate) body: Vec<u8>,
}

impl Response {
    pub(crate) fn json(status: u16, value: &Json) -> Self {
        Self {
            status,
            reason: reason_phrase(status),
            content_type: "application/json",
            headers: Vec::new(),
            body: serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec()),
        }
    }
}

pub(crate) fn reason_phrase(status: u16) -> &'static str {
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
pub(crate) fn error_response(status: u16, code: &str, message: &str) -> Response {
    Response::json(
        status,
        &json!({"ok": false, "error": {"code": code, "message": message}}),
    )
}

/// 读一个请求（头封顶 32 KiB / 超时 5s）。返回 `None` = 畸形 / 空连接。
pub(crate) fn read_request(stream: &mut TcpStream) -> std::io::Result<Option<Request>> {
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let mut buf: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    // 头终止符的位置（不是 ends_with：POST 的头与体常同包到达，体在终止符之后，
    // 若等 ends_with 会把同包体字节拖进下一轮空读直至超时——实测注册面 400 空包）。
    let split = loop {
        if buf.len() > MAX_HEADER_BYTES {
            return Ok(None);
        }
        if let Some(split) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
            break split;
        }
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            return Ok(None); // 客户端直接断开
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..split]).into_owned();
    let Some(mut request) = parse_request_from_head(&head) else {
        return Ok(None);
    };
    // 请求体：仅注册面（POST）有语义；按 Content-Length 读，封顶 8 KiB。
    // 无 Content-Length 的 POST = 空体（路由面按「缺 root 字段」拒绝）。
    let content_length = head
        .split("\r\n")
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            (name.eq_ignore_ascii_case("content-length"))
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    if content_length > 8 * 1024 {
        return Ok(None);
    }
    // 同包已到的体字节先用掉，再读剩余部分。
    let mut body = buf[split + 4..].to_vec();
    while body.len() < content_length {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            return Ok(None);
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(content_length);
    request.body = body;
    Ok(Some(request))
}

/// 请求行解析（网络读取与测试面共用的同一入口）。
pub(crate) fn parse_request_from_head(head: &str) -> Option<Request> {
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
        body: Vec::new(),
    })
}

/// 百分号解码（路径/制品 rel 用；查询数值面不需要）。
pub(crate) fn percent_decode(text: &str) -> String {
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

pub(crate) fn query_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query.split('&').find_map(|pair| {
        pair.split_once('=')
            .filter(|(name, _)| *name == key)
            .map(|(_, value)| value)
    })
}

/// 数值查询参数：缺省 / 非法 / 钳制（§9.4 query 数值钳制）。
pub(crate) fn query_usize(query: &str, key: &str, default: usize, max: usize) -> Result<usize, Response> {
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

pub(crate) fn handle_connection(
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
    match route(
        shared,
        &request.method,
        &request.path,
        &request.query,
        &request.body,
    ) {
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

pub(crate) fn write_response(stream: &mut TcpStream, response: Response) -> std::io::Result<()> {
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
