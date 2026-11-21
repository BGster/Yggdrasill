use super::*;
use std::fs;
use ygg_store::{Backend, RunStore, Tick};

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
