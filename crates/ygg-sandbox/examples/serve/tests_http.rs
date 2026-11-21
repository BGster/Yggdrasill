use super::*;
use std::fs;
use ygg_store::Backend;

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
