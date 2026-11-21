use super::*;
use std::fs;
use ygg_store::{Backend, RunStore, StepDraft, Tick};

    pub(crate) const TICK: Tick = Tick {
        secs: 1_700_000_000,
        nanos: 5,
    };

    pub(crate) fn case_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ygg-serve-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    pub(crate) fn shared_for(runs_root: &Path) -> Arc<ServeShared> {
        shared_for_roots(&[runs_root.to_path_buf()])
    }

    /// 测试装配：PathBuf → RootEntry（按 run.json 存在性归类，与 POST 注册同规则）。
    pub(crate) fn root_entries(roots: &[PathBuf]) -> Vec<RootEntry> {
        roots
            .iter()
            .map(|path| RootEntry {
                single_run: path.join(crate::META_FILE).is_file(),
                path: path.clone(),
            })
            .collect()
    }

    pub(crate) fn shared_for_roots(roots: &[PathBuf]) -> Arc<ServeShared> {
        Arc::new(ServeShared {
            roots: RwLock::new(root_entries(roots)),
            panel_dir: None,
            broadcaster: Broadcaster::new(),
            connections: AtomicUsize::new(0),
        })
    }

    /// 在 shared 上打一发路由（不碰网络；POST 也经同一路由入口）。
    pub(crate) fn http(shared: &ServeShared, method: &str, target: &str) -> Response {
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

    pub(crate) fn body_json(response: &Response) -> Json {
        serde_json::from_slice(&response.body).expect("响应体是合法 JSON")
    }

    /// 起一个真监听的 serve（测试面：port 0 + 后台 watcher/accept 线程）。
    pub(crate) struct LiveServe {
        pub(crate) port: u16,
        shutdown: Arc<AtomicBool>,
    }

    impl LiveServe {
        pub(crate) fn start(roots: &[PathBuf], poll_ms: u64) -> Self {
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
        pub(crate) fn get(&self, target: &str) -> (u16, String, Vec<u8>) {
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
        pub(crate) fn sse_frame(&self, predicate: impl Fn(&str, &str) -> bool) -> (String, String) {
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

    pub(crate) fn parse_http(raw: &[u8]) -> (u16, String, Vec<u8>) {
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
    pub(crate) fn write_mod(dir: &Path, panel_entry: Option<&str>) {
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
    pub(crate) fn seed_run(runs_root: &Path, run_id: &str, backend: Backend, steps: usize) {
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

    pub(crate) fn seed_run_with_mod(
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

    /// 第二写者：经 **真实 Session**（宿主同一条 dispatch → 持久化临界区链）落盘。
    /// （测试二进制无法把自身当 CLI 子进程 spawn——`current_exe()` 是 libtest
    /// harness；此处以独立 Session + 真实持久化端口承载「另一写者」的事实，
    /// serve 观察到的变更全部来自盘上文件的真变化。）
    pub(crate) fn dispatch_external(
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

    pub(crate) fn runs_of(serve: &LiveServe) -> Vec<String> {
        let (status, _, body) = serve.get("/api/runs");
        assert_eq!(status, 200);
        serde_json::from_slice::<Json>(&body).expect("JSON")["runs"]
            .as_array()
            .expect("runs")
            .iter()
            .filter_map(|row| row["runId"].as_str().map(str::to_owned))
            .collect()
    }

    pub(crate) fn wait_until_seen(serve: &LiveServe, run_id: &str) {
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

    pub(crate) fn loopback_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind((BIND_ADDR, 0)).expect("绑定回环");
        let addr = listener.local_addr().expect("地址");
        let client = TcpStream::connect(addr).expect("连接回环");
        let (server, _) = listener.accept().expect("接受回环");
        (server, client)
    }

    /// 事件日志的**语义面**（去 timestamp 后逐行 JSON 等值）：
    /// 两局独立落盘，墙钟秒可能跨秒，timestamp 不参与纯度比对。
    pub(crate) fn events_semantic(path: &Path) -> Vec<Json> {
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

    /// 矩阵基准 = solo（除 drive_is_stdin 外全 false：serve 单独就是合法形态）。
    pub(crate) fn solo() -> crate::ServeFlags {
        crate::ServeFlags {
            drive_is_stdin: true,
            ..crate::ServeFlags::default()
        }
    }
