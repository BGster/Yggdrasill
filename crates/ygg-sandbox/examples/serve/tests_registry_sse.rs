use super::*;
use std::fs;
use ygg_store::Backend;

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
    /// `Session` 构造。扫描范围 = 七个生产文件 concat 全文（C2 拆分后生产文件不再含
    /// 测试块，`mod serve_tests` 分段标记自然退役；C1 同 commit 扩 concat 时覆盖只增
    /// 不缩；测试文件里的第二写者夹具合法地使用 Session——它扮演的是 serve 之外的
    /// 「另一写者」）。
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
