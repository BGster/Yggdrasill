use super::*;

fn case_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ygg-r1-vfs-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 最小合成 Mod：一个会写 VFS 的 handler + 一个写一半就报错的 handler。
fn write_vfs_mod(dir: &Path) {
    fs::create_dir_all(dir.join("data")).expect("建 data 目录");
    fs::write(
        dir.join("pack.json"),
        r#"{
  "schemaVersion": 1,
  "entries": [],
  "schemaFiles": [],
  "agent": {
    "entryEvent": "event.start",
    "choiceEvent": "ygg.choice_selected",
    "summaryEvent": "agent.summary",
    "endingPrefix": "【结局",
    "metaKey": "summary",
    "rejectionPrefixes": ["【拒绝】"],
    "replayPrefix": "【重放】",
    "defaultPayload": "0"
  },
  "inboxRoutes": {
    "externalIdField": "external_id",
    "payloadField": "payload",
    "extMarker": "@ext=",
    "add": {
      "action": "inbox.add",
      "fields": ["title", "external_id"],
      "write": {"flag": "requirement", "shape": "named"}
    }
  }
}"#,
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
ygg.on("event.write", function()
  local out = ygg.fs.write("artifacts/note.md", "第一版")
  ygg.narrate("w=" .. out.path)
end)
ygg.on("event.boom", function()
  ygg.fs.write("artifacts/dirty.md", "leak")
  error("boom")
end)
ygg.on("agent.summary", function()
  ygg.meta("summary", { open = true })
  ygg.narrate("摘要就绪")
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

fn vfs_args(mod_dir: Option<&Path>, runs_root: &Path, run_id: &str, resume: bool) -> Args {
    Args {
        packages: None,
        mod_dir: mod_dir.map(Path::to_path_buf),
        open_style: Some(if resume {
            crate::OpenStyle::Open
        } else {
            crate::OpenStyle::New
        }),
        run_root: runs_root.join(run_id),
        runs_parent: runs_root.to_path_buf(),
        // 旧「显式新局」= 总是重置 → 新语义映射为 new --force（显式许可）。
        force: !resume,
        at: None,
        settle: false,
        watch: false,
        event: None,
        payload: None,
        output: crate::OutputFormat::Text,
        drive: Drive::None,
        inbox: None,
        store: Some(ygg_store::Backend::Json),
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
        serve: false,
        port: None,
        panel_dir: None,
        poll_ms: None,
    }
}

/// 一个「已经有后端产物」的局目录（制品 CLI 不新造 run，所以必须先有产物）。
fn seeded_run(args: &Args) -> PathBuf {
    let run_dir = args.run_root.clone();
    fs::create_dir_all(&run_dir).expect("建 run 目录");
    fs::write(run_dir.join(AUTOSAVE_FILE), b"{}").expect("写 autosave");
    run_dir
}

// ---- `--artifact-put` / `--artifact-get` 的核心 ----

#[test]
fn artifact_put_then_get_roundtrips_bytes() {
    let dir = case_dir("put-get");
    let args = vfs_args(None, &dir.join("runs"), "pg", false);
    let run_dir = seeded_run(&args);
    let body = "内容没有尾随换行";
    let source = dir.join("source.md");
    fs::write(&source, body).expect("写源文件");

    let receipt = artifact_put_receipt(&args, &format!("artifacts/note.md={}", source.display()))
        .expect("写入必须成功");
    // 回执形状：ok / path / bytes / sha256（**由端口给**，宿主不现算）。
    assert_eq!(receipt["ok"], json!(true));
    assert_eq!(receipt["path"], json!("artifacts/note.md"));
    assert_eq!(receipt["bytes"], json!(body.len()));
    assert_eq!(
        receipt["sha256"].as_str().expect("sha256").len(),
        64,
        "小写十六进制 64 字符"
    );
    // 字节保真：读回来与源文件**逐字节**相等（不补换行）。
    let bytes = artifact_get_bytes(&args, "artifacts/note.md").expect("读回必须成功");
    assert_eq!(bytes, body.as_bytes());
    // 真的落在后端的制品面里（不是只回执成功）。
    assert!(run_dir.join("artifacts/note.md").is_file());
}

#[test]
fn artifact_get_missing_is_err() {
    let dir = case_dir("get-missing");
    let args = vfs_args(None, &dir.join("runs"), "gm", false);
    seeded_run(&args);
    let message = artifact_get_bytes(&args, "artifacts/none.md").expect_err("缺失必须报错");
    assert!(message.contains("不存在"), "{message}");
}

#[test]
fn artifact_put_rejects_bad_path_and_oversize() {
    let dir = case_dir("put-gate");
    let args = vfs_args(None, &dir.join("runs"), "gate", false);
    seeded_run(&args);
    let source = dir.join("s.md");
    fs::write(&source, "x").expect("写源文件");
    let spec = |path: &str| format!("{path}={}", source.display());

    // 路径门在端口内：宿主不重写第二份规则。
    let message =
        artifact_put_receipt(&args, &spec("artifacts/../escape.md")).expect_err("穿越路径必须被拒");
    assert!(message.contains("制品路径"), "{message}");
    let message = artifact_put_receipt(&args, &spec("artifact/x.md")).expect_err("坏前缀必须被拒");
    assert!(message.contains("制品路径"), "{message}");

    // 内容上限：256 KiB + 1 字节。
    let big = dir.join("big.md");
    fs::write(&big, vec![b'a'; 256 * 1024 + 1]).expect("写大文件");
    let message = artifact_put_receipt(&args, &format!("artifacts/big.md={}", big.display()))
        .expect_err("超限必须被拒");
    assert!(message.contains("制品内容"), "{message}");
    // 零写入。
    assert!(!args.runs_parent.join("gate").join("artifacts").exists());
}

#[test]
fn artifact_put_requires_an_existing_run() {
    let dir = case_dir("put-newrun");
    let args = vfs_args(None, &dir.join("runs"), "no-such-run", false);
    let source = dir.join("s.md");
    fs::write(&source, "x").expect("写源文件");
    let message = artifact_put_receipt(&args, &format!("artifacts/a.md={}", source.display()))
        .expect_err("不存在的局必须被拒");
    assert!(message.contains("不存在"), "{message}");
    // **不新造 run**：连目录都不该被建出来。
    assert!(!args.runs_parent.join("no-such-run").exists());
}

// ---- Session 的 VFS 持久化（写 → 落盘 → 续局回灌）----

#[test]
fn session_vfs_survives_resume() {
    let dir = case_dir("resume");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_vfs_mod(&mod_dir);
    let artifact_path = runs_root.join("rv").join("artifacts/note.md");

    {
        let mut session =
            Session::open(&vfs_args(Some(&mod_dir), &runs_root, "rv", false)).expect("新局开局");
        assert_eq!(session.kernel.vfs_len(), 0, "新局从空 VFS 起步");
        let receipt = session.dispatch_event("event.write", "0");
        assert_eq!(receipt["ok"], json!(true), "{receipt}");
        assert_eq!(session.kernel.vfs_len(), 1);
        // 成功步 → 制品**当场**落盘（不是等收口）。
        assert_eq!(
            fs::read(&artifact_path).expect("制品必须在盘上"),
            "第一版".as_bytes()
        );
    }

    // 续局：VFS 不在存档字节里，只能是宿主从制品面回灌。
    let mut resumed =
        Session::open(&vfs_args(Some(&mod_dir), &runs_root, "rv", true)).expect("续局");
    assert_eq!(resumed.kernel.vfs_len(), 1, "续局必须把制品回灌进 VFS");
    assert!(
        resumed.kernel.vfs_take_dirty().is_empty(),
        "预载不是本次写入，不得标脏"
    );
    // 回灌的内容与摘要是盘上那份（不是空壳）。
    let entries = resumed.kernel.vfs_entries();
    assert_eq!(entries[0].0, "artifacts/note.md");
    assert_eq!(entries[0].1, "第一版".as_bytes());
    assert_eq!(entries[0].2, ygg_store::sha256_hex("第一版".as_bytes()));
}

#[test]
fn failed_dispatch_never_reaches_the_artifact_store() {
    let dir = case_dir("rollback");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_vfs_mod(&mod_dir);
    let mut session =
        Session::open(&vfs_args(Some(&mod_dir), &runs_root, "rb", false)).expect("新局开局");
    let receipt = session.dispatch_event("event.boom", "0");
    assert_eq!(receipt["ok"], json!(false), "{receipt}");
    // 内核回滚 + 宿主不 flush ⇒ 盘上不该有失败步写的文件。
    assert_eq!(session.kernel.vfs_len(), 0, "失败步的文件必须回滚");
    assert!(
        !runs_root.join("rb").join("artifacts/dirty.md").exists(),
        "失败步不得落盘"
    );
}

// ---- wire 契约 ----

#[test]
fn wire_line_is_single_line_and_sanitized() {
    let line = wire_line(&[("a", "x;y,z|w\nv\r\0".to_owned()), ("b", String::new())]);
    assert_eq!(line, "a=x_y_z_w_v__;b=");
    for forbidden in ['|', '\n', '\r', '\0'] {
        assert!(
            !line.contains(forbidden),
            "wire 行不得含 {forbidden:?}：{line}"
        );
    }
    // **空值字段仍然在**（调用方按位解析，缺字段与空字段含义不同）。
    assert!(line.ends_with(";b="));
}

// ---- 内置工具（六个 --builtin-tool op）----

fn op_flags(pairs: &[(&str, &str)]) -> OpFlags {
    OpFlags {
        values: pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect(),
    }
}

/// 一个有一次提交的真 git 仓库（worktree 三个 op 的现场）。
fn git_fixture(dir: &Path) -> PathBuf {
    let repo = dir.join("repo");
    fs::create_dir_all(&repo).expect("建仓库");
    let run = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args([
                "-c",
                "user.email=r1@example.com",
                "-c",
                "user.name=r1",
                "-c",
                "init.defaultBranch=main",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .expect("执行 git");
        assert!(
            output.status.success(),
            "git {args:?} 失败：{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    };
    run(&["init", "-q"]);
    fs::write(repo.join("a.txt"), "a").expect("写文件");
    run(&["add", "."]);
    run(&["commit", "-q", "-m", "init"]);
    repo
}

fn lane_wt(root: &Path, repo: &Path, lane: u8) -> PathBuf {
    root.join(format!("{}-lane-{lane}", repo_label(repo)))
}

#[test]
fn worktree_root_for_respects_precedence_and_defaults_to_home_tmp_worktrees() {
    let home = PathBuf::from("/Users/tester");
    assert_eq!(
        worktree_root_for(Some("/custom/wts"), Some("/env/wts"), Some(&home)),
        PathBuf::from("/custom/wts")
    );
    assert_eq!(
        worktree_root_for(None, Some("/env/wts"), Some(&home)),
        PathBuf::from("/env/wts")
    );
    assert_eq!(
        worktree_root_for(None, Some("   "), Some(&home)),
        PathBuf::from("/Users/tester/tmp/worktrees")
    );
    assert_eq!(
        worktree_root_for(None, None, Some(&home)),
        PathBuf::from("/Users/tester/tmp/worktrees")
    );
    assert_eq!(
        worktree_root_for(None, None, None),
        std::env::temp_dir().join("ygg-worktrees")
    );
}

#[test]
fn builtin_worktree_claim_and_release_roundtrip() {
    let dir = case_dir("claim");
    let repo = git_fixture(&dir);
    let root = dir.join("wts");
    let (root_s, repo_s) = (root.display().to_string(), repo.display().to_string());
    let flags = op_flags(&[
        ("--lane", "1"),
        ("--req", "r1"),
        ("--branch", "lane/b1"),
        ("--root", &root_s),
        ("--repo", &repo_s),
    ]);
    let fields = claim_fields_with(&flags).expect("首次认领必须成功");
    assert_eq!(
        fields.iter().map(|(key, _)| *key).collect::<Vec<_>>(),
        vec!["lane", "req", "branch", "base", "clean", "reused", "path"]
    );
    let line = wire_line(&fields);
    assert!(
        line.starts_with("lane=1;req=r1;branch=lane/b1;base="),
        "{line}"
    );
    assert!(line.contains(";clean=1;reused=0;path="), "{line}");

    let wt = lane_wt(&root, &repo, 1);
    assert!(wt.is_dir(), "worktree 必须被建出来");
    assert!(wt.join("a.txt").is_file());

    // 写入 `.git/info/exclude` 忽略 `target/`，并在泳道内放入构建缓存 `target/cache.bin` 与未跟踪脏文件 `wip.txt`。
    let exclude = repo.join(".git/info/exclude");
    fs::write(&exclude, "target/\n").expect("写 exclude");
    fs::create_dir_all(wt.join("target")).expect("建 target");
    fs::write(wt.join("target/cache.bin"), "warm-cache").expect("写构建缓存");
    fs::write(wt.join("wip.txt"), "untracked-scratch").expect("写未跟踪临时文件");

    // 释放（停泊）：目录仍在、HEAD 处于 detached（允许删除原分支 `lane/b1`）、`wip.txt` 被清而 `target/cache.bin` 保留。
    let release = op_flags(&[("--lane", "1"), ("--root", &root_s), ("--repo", &repo_s)]);
    let rel_fields = release_fields_with(&release).expect("释放（停泊）必须成功");
    let rel_line = wire_line(&rel_fields);
    assert!(
        rel_line.starts_with("lane=1;released=1;parked=1;path="),
        "{rel_line}"
    );
    assert!(wt.is_dir(), "停泊后 worktree 目录必须保留");
    assert!(
        !wt.join("wip.txt").exists(),
        "未跟踪非忽略文件必须在停泊时清理"
    );
    assert_eq!(
        fs::read_to_string(wt.join("target/cache.bin")).expect("读构建缓存"),
        "warm-cache",
        "gitignored 构建缓存必须在停泊后保留"
    );
    // 因为 HEAD 已 detach，主检出可以安全删除已合并的分支 `lane/b1`。
    git_capture(&repo, &["branch", "-D", "lane/b1"]).expect("停泊后原分支应当可删");

    // 再次认领同一泳道给新需求 `r2`：`reused=1` 且 `target/cache.bin` 依旧温热。
    let flags_r2 = op_flags(&[
        ("--lane", "1"),
        ("--req", "r2"),
        ("--branch", "lane/b2"),
        ("--root", &root_s),
        ("--repo", &repo_s),
    ]);
    let again = claim_fields_with(&flags_r2).expect("二次认领必须成功");
    let again_line = wire_line(&again);
    assert!(
        again_line.starts_with("lane=1;req=r2;branch=lane/b2;base="),
        "{again_line}"
    );
    assert!(
        again_line.contains(";clean=1;reused=1;path="),
        "{again_line}"
    );
    assert_eq!(
        fs::read_to_string(wt.join("target/cache.bin")).expect("复用后读构建缓存"),
        "warm-cache"
    );
}

#[test]
fn worktree_claim_detaches_branch_if_held_by_another_lane_and_reuses_in_place() {
    let dir = case_dir("claim-reassigned");
    let repo = git_fixture(&dir);
    let root = dir.join("wts");
    let (root_s, repo_s) = (root.display().to_string(), repo.display().to_string());
    let lane1 = op_flags(&[
        ("--lane", "1"),
        ("--req", "r1"),
        ("--branch", "feature/a3f9-map-single-gone"),
        ("--root", &root_s),
        ("--repo", &repo_s),
    ]);
    claim_fields_with(&lane1).expect("车道 1 首次认领");

    // 若 r1 重派到车道 2（同分支 `feature/a3f9-map-single-gone`），车道 1 自动 detach 释放分支锁，车道 2 顺利挂载。
    let lane2 = op_flags(&[
        ("--lane", "2"),
        ("--req", "r1"),
        ("--branch", "feature/a3f9-map-single-gone"),
        ("--root", &root_s),
        ("--repo", &repo_s),
    ]);
    let fields = claim_fields_with(&lane2).expect("跨车道重派认领必须成功");
    let line = wire_line(&fields);
    assert!(
        line.contains("lane=2;req=r1;branch=feature/a3f9-map-single-gone;"),
        "{line}"
    );
    assert!(line.contains(";clean=1;reused=0;path="), "{line}");
}

#[test]
fn worktree_release_is_idempotent_when_lane_dir_is_absent() {
    let dir = case_dir("release");
    let repo = git_fixture(&dir);
    let root = dir.join("wts");
    let (root_s, repo_s) = (root.display().to_string(), repo.display().to_string());
    let release = op_flags(&[("--lane", "3"), ("--root", &root_s), ("--repo", &repo_s)]);

    let fields = release_fields_with(&release).expect("未创建的车道释放也必须成功");
    let line = wire_line(&fields);
    assert!(
        line.starts_with("lane=3;released=1;parked=0;path="),
        "{line}"
    );
}

#[test]
fn lane_probe_reports_distance_and_is_clean_after_claim() {
    let dir = case_dir("lane-probe");
    let repo = git_fixture(&dir);
    let root = dir.join("wts");
    let (root_s, repo_s) = (root.display().to_string(), repo.display().to_string());
    let base = git_one(&repo, &["rev-parse", "HEAD"]).expect("读 HEAD");
    let claim = op_flags(&[
        ("--lane", "4"),
        ("--req", "r4"),
        ("--branch", "lane/b4"),
        ("--root", &root_s),
        ("--repo", &repo_s),
    ]);
    claim_fields_with(&claim).expect("认领");

    let probe = op_flags(&[
        ("--lane", "4"),
        ("--base", &base),
        ("--branch", "lane/b4"),
        ("--root", &root_s),
        ("--repo", &repo_s),
    ]);
    let fields = lane_probe_fields_with(&probe).expect("探测");
    assert_eq!(
        fields.iter().map(|(key, _)| *key).collect::<Vec<_>>(),
        vec!["lane", "branch", "base", "head", "commits", "clean", "files"]
    );
    let line = wire_line(&fields);
    // base 原样回显（短 sha 保持短）；commits=0（刚认领，还没往前走）。
    assert!(
        line.starts_with(&format!("lane=4;branch=lane/b4;base={base};head=")),
        "{line}"
    );
    assert!(line.contains(";commits=0;clean=1;files="), "{line}");
}

#[test]
fn lane_probe_fails_closed_on_an_unknown_branch() {
    let dir = case_dir("lane-probe-bad");
    let repo = git_fixture(&dir);
    let repo_s = repo.display().to_string();
    let base = git_one(&repo, &["rev-parse", "HEAD"]).expect("读 HEAD");
    let probe = op_flags(&[
        ("--lane", "5"),
        ("--base", &base),
        ("--branch", "lane/nope"),
        ("--repo", &repo_s),
    ]);
    match lane_probe_fields_with(&probe) {
        Err(OpFailure::Detail { slug, detail }) => {
            assert_eq!(slug, "rev-list", "错误必须点名哪一步失败：{detail}");
        }
        _ => panic!("未知分支必须失败（不得静默报 commits=0）"),
    }
}

#[test]
fn artifact_probe_answers_exists_zero_instead_of_failing() {
    let dir = case_dir("artifact-probe");
    let args = vfs_args(None, &dir.join("runs"), "ap", false);
    let run_dir = seeded_run(&args);
    let run_dir_s = run_dir.display().to_string();

    // ① 缺失：**正常答案**，不是错误。
    let missing = artifact_probe_fields_with(&op_flags(&[
        ("--path", "artifacts/none.md"),
        ("--run-dir", &run_dir_s),
    ]))
    .expect("缺失必须是正常回执");
    assert_eq!(
        wire_line(&missing),
        "path=artifacts/none.md;exists=0;bytes=0;sha256="
    );

    // ② 存在：bytes / sha256 来自端口（不是宿主自己拼的）。
    fs::create_dir_all(run_dir.join("artifacts")).expect("建制品目录");
    fs::write(run_dir.join("artifacts/none.md"), b"hi").expect("写制品");
    let hit = artifact_probe_fields_with(&op_flags(&[
        ("--path", "artifacts/none.md"),
        ("--run-dir", &run_dir_s),
    ]))
    .expect("存在的制品");
    assert_eq!(
        wire_line(&hit),
        format!(
            "path=artifacts/none.md;exists=1;bytes=2;sha256={}",
            ygg_store::sha256_hex(b"hi")
        )
    );

    // ③ 非法路径 = 调用方 bug，不能当成「不存在」。
    match artifact_probe_fields_with(&op_flags(&[
        ("--path", "artifact/bad.md"),
        ("--run-dir", &run_dir_s),
    ])) {
        Err(OpFailure::Detail { slug, .. }) => assert_eq!(slug, "path"),
        _ => panic!("非法路径必须报错（不得伪装成 exists=0）"),
    }
}

#[test]
fn archive_sessions_moves_only_matching_jsonl() {
    let dir = case_dir("archive");
    let sessions = dir.join("sessions");
    fs::create_dir_all(&sessions).expect("建会话目录");
    fs::write(sessions.join("a.jsonl"), "{\"parent\":\"p1\"}\n").expect("写 a");
    fs::write(sessions.join("b.jsonl"), "{\"x\":1}\n").expect("写 b");
    // parent 自己的转录档：名字里就有 parent → 不搬。
    fs::write(sessions.join("p1.jsonl"), "{\"parent\":\"p1\"}\n").expect("写 p1");
    // 非 `.jsonl`：永不入选（数据红线：本 op 只碰会话转录）。
    fs::write(sessions.join("notes.txt"), "p1").expect("写 txt");

    let trash = dir.join("trash");
    let outcome = sweep_sessions(&sessions, &trash, "p1").expect("清扫");
    assert_eq!(outcome.moved, 1, "只有 a.jsonl 该被搬走");
    assert_eq!(outcome.remaining, 0, "重扫必须为空（清扫完成）");
    assert!(trash.join("a.jsonl").exists());
    assert!(!sessions.join("a.jsonl").exists());
    assert!(sessions.join("b.jsonl").exists(), "不匹配的不得被动");
    assert!(
        sessions.join("p1.jsonl").exists(),
        "parent 自己的转录档不得搬"
    );
    assert!(sessions.join("notes.txt").exists(), "非 .jsonl 永不入选");
}

#[test]
fn verify_gates_match_the_documented_suite() {
    // `fast` = 一个 `cargo check`；`full` = AGENTS.md 的门禁三件套（顺序固定）。
    assert_eq!(verify_gates("fast").len(), 1);
    assert_eq!(verify_gates("fast")[0].1[0], "check");
    let full = verify_gates("full");
    assert_eq!(
        full.iter().map(|(label, _)| *label).collect::<Vec<_>>(),
        vec![
            "cargo test --workspace",
            "cargo test --example egg -p ygg-sandbox",
            "cargo test -p ygg-sandbox --test agent_seed_semantics",
        ]
    );
}

#[test]
fn parse_custom_verify_commands_splits_semicolon_separated_commands() {
    let parsed = parse_custom_verify_commands(" npm test -- --run ; pytest -q ; ");
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].0, "npm test -- --run");
    assert_eq!(parsed[0].1, "npm");
    assert_eq!(parsed[0].2, vec!["test", "--", "--run"]);
    assert_eq!(parsed[1].0, "pytest -q");
    assert_eq!(parsed[1].1, "pytest");
    assert_eq!(parsed[1].2, vec!["-q"]);
}

/// `@self` 别名：`egg` 与 `@self` 都解析到本可执行文件（同一分支）。
#[test]
fn self_alias_matches_the_contract_placeholder() {
    assert_eq!(SELF_ALIAS, "egg");
    assert_eq!(ygg_sandbox::host_contract::SELF_PROGRAM, "@self");
    assert_ne!(SELF_ALIAS, ygg_sandbox::host_contract::SELF_PROGRAM);
}
