use super::{
    archive_sessions_fields_with, cwd_slug, detect_harness, harness_session_dirs,
    resolve_harness_adapter, ArchiveInputs, HarnessProbe, OpFlags, Path, PathBuf,
};
use std::fs;

/// 独占临时根（每个用例一个；先清干净，避免上次残留改变结论）。
fn temp_root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ygg-harness-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("建临时根");
    dir
}

/// 探测输入面的简写（三个环境变量各自在不在）。
fn probe(pi: bool, claude: bool, codex: bool) -> HarnessProbe {
    HarnessProbe {
        pi_session_id: pi,
        claude_code: claude,
        codex_home: codex,
    }
}

/// op 级旗标的直连面（`OpFlags::parse` 读进程 argv，单测不能走）。
fn flags(pairs: &[(&str, &str)]) -> OpFlags {
    OpFlags {
        values: pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect(),
    }
}

/// 只有 cwd 不同的归档输入面（会话目录 / HOME / 适配器全部显式注入）。
fn inputs(harness: &str, root: &Path) -> ArchiveInputs {
    ArchiveInputs {
        harness: Some(harness.to_owned()),
        mod_dir: None,
        sessions_dir: None,
        home: Some(root.join("home")),
        cwd: Some(root.join("work")),
        env_harness: None,
    }
}

fn outcome(out: &[(&'static str, String)]) -> (String, String, String) {
    (out[0].1.clone(), out[1].1.clone(), out[2].1.clone())
}

/// 显式 `--harness` 压过环境变量；环境变量压过「未表态」。
#[test]
fn explicit_adapter_wins_over_the_env_layer() {
    let cwd = temp_root("explicit");
    for adapter in ["pi", "claude-code", "codex", "generic"] {
        assert_eq!(
            resolve_harness_adapter(Some(adapter), Some("codex"), &cwd),
            adapter,
            "--harness {adapter} 必须压过 YGG_HARNESS"
        );
    }
    // 没有显式值时才轮到第二层（这里用确定值验，auto 分支走 detect_harness 的用例）。
    assert_eq!(
        resolve_harness_adapter(None, Some("claude-code"), &cwd),
        "claude-code"
    );
    let _ = fs::remove_dir_all(&cwd);
}

/// `auto` 的优先级：Pi > Claude Code > Codex > generic；现场痕迹也能叫起适配器。
#[test]
fn auto_detection_prefers_pi_then_claude_then_codex() {
    let root = temp_root("detect");
    let work = root.join("work");
    fs::create_dir_all(&work).expect("建 cwd");
    let home = root.join("home");

    assert_eq!(
        detect_harness(&probe(true, true, true), Some(&home), &work),
        "pi"
    );
    assert_eq!(
        detect_harness(&probe(false, true, true), Some(&home), &work),
        "claude-code"
    );
    assert_eq!(
        detect_harness(&probe(false, false, true), Some(&home), &work),
        "codex"
    );
    assert_eq!(
        detect_harness(&probe(false, false, false), Some(&home), &work),
        "generic"
    );

    // 工作区的 `.pi` / `.claude` 目录同样是证据（不靠环境变量）。
    fs::create_dir_all(work.join(".pi")).expect("建 .pi");
    assert_eq!(
        detect_harness(&probe(false, true, false), Some(&home), &work),
        "pi",
        "`.pi` 现场压过 CLAUDE_CODE"
    );
    let _ = fs::remove_dir_all(work.join(".pi"));
    fs::create_dir_all(work.join(".claude")).expect("建 .claude");
    assert_eq!(
        detect_harness(&probe(false, false, false), Some(&home), &work),
        "claude-code"
    );
    let _ = fs::remove_dir_all(work.join(".claude"));

    // `$HOME/.pi/agent/sessions/--<cwd_slug>--` 存在 → Pi（历史会话就摆在那里）。
    fs::create_dir_all(
        home.join(".pi")
            .join("agent")
            .join("sessions")
            .join(format!("--{}--", cwd_slug(&work))),
    )
    .expect("建 pi 会话库");
    assert_eq!(
        detect_harness(&probe(false, false, false), Some(&home), &work),
        "pi"
    );
    let _ = fs::remove_dir_all(&root);
}

/// 每个适配器的会话库落点（`generic` 没有会话库）。
#[test]
fn session_store_paths_are_per_adapter() {
    let home = Path::new("/home/x");
    assert_eq!(
        harness_session_dirs("pi", home, "a-b"),
        Some((
            PathBuf::from("/home/x/.pi/agent/sessions/--a-b--"),
            PathBuf::from("/home/x/.pi/agent/sessions-trash")
        ))
    );
    assert_eq!(
        harness_session_dirs("claude-code", home, "a-b"),
        Some((
            PathBuf::from("/home/x/.claude/projects/-a-b"),
            PathBuf::from("/home/x/.claude/sessions-trash")
        ))
    );
    assert_eq!(
        harness_session_dirs("codex", home, "a-b"),
        Some((
            PathBuf::from("/home/x/.codex/sessions"),
            PathBuf::from("/home/x/.codex/sessions-trash")
        ))
    );
    assert_eq!(harness_session_dirs("generic", home, "a-b"), None);
    assert_eq!(cwd_slug(Path::new("/Users/x/Repo")), "Users-x-Repo");
}

/// `generic` 没有会话库：记一笔 0/0 台账，**不建任何目录**、不报错。
///
/// 连带钉住一个边界：即使 `$HOME` 也缺席（容器里很常见），`generic` 仍然成功——
/// 它连会话库都不需要，不能让收口卡在一个「推导不出来的路径」上。
#[test]
fn generic_adapter_records_a_zero_ledger() {
    let root = temp_root("generic");
    let flags = flags(&[("--parent", "p1"), ("--batch", "b1")]);
    let out = archive_sessions_fields_with(&flags, &inputs("generic", &root))
        .expect("generic 必须成功（收不出东西不是错误）");
    assert_eq!(
        outcome(&out),
        (
            "0".to_owned(),
            "0".to_owned(),
            "generic:no-session-store:b1".to_owned()
        )
    );
    assert!(
        !root.join("home").exists(),
        "generic 不得顺手建出会话库/trash 目录"
    );
    // `$HOME` 缺席也是同一个答案（generic 不靠任何目录）。
    let mut homeless = inputs("generic", &root);
    homeless.home = None;
    let out = archive_sessions_fields_with(&flags, &homeless).expect("generic 不需要 HOME");
    assert_eq!(
        outcome(&out),
        (
            "0".to_owned(),
            "0".to_owned(),
            "generic:no-session-store:b1".to_owned()
        )
    );
    let _ = fs::remove_dir_all(&root);
}

/// 适配器目录不存在（首次归档）：记 `<适配器>:empty:<batch>`，不失败。
#[test]
fn a_missing_adapter_dir_is_an_empty_ledger() {
    let root = temp_root("empty");
    fs::create_dir_all(root.join("home")).expect("建 HOME");
    for adapter in ["pi", "claude-code", "codex"] {
        let out = archive_sessions_fields_with(
            &flags(&[("--parent", "p1"), ("--batch", "b1")]),
            &inputs(adapter, &root),
        )
        .expect("缺目录必须成功");
        assert_eq!(
            outcome(&out),
            (
                "0".to_owned(),
                "0".to_owned(),
                format!("{adapter}:empty:b1")
            ),
            "{adapter} 的会话库不存在时只能记 0/0"
        );
    }
    let _ = fs::remove_dir_all(&root);
}

/// `pi` 分支真的能搬：命中转录进 `<sessions-trash>/<batch>`，不匹配的原样不动。
#[test]
fn pi_adapter_sweeps_the_matching_transcript() {
    let root = temp_root("pi-sweep");
    let work = root.join("work");
    let home = root.join("home");
    let sessions = home
        .join(".pi")
        .join("agent")
        .join("sessions")
        .join(format!("--{}--", cwd_slug(&work)));
    fs::create_dir_all(&sessions).expect("建会话库");
    fs::write(sessions.join("s1.jsonl"), "head\nparent-777\ntail\n").expect("写命中档");
    fs::write(sessions.join("s2.jsonl"), "nothing here\n").expect("写其它档");

    let out = archive_sessions_fields_with(
        &flags(&[("--parent", "parent-777"), ("--batch", "b9")]),
        &inputs("pi", &root),
    )
    .expect("扫描成功");
    let (moved, remaining, dest) = outcome(&out);
    assert_eq!((moved.as_str(), remaining.as_str()), ("1", "0"));
    let trash = home
        .join(".pi")
        .join("agent")
        .join("sessions-trash")
        .join("b9");
    assert_eq!(
        dest,
        fs::canonicalize(&trash)
            .expect("trash 必已建")
            .display()
            .to_string()
    );
    assert!(trash.join("s1.jsonl").is_file(), "命中的档必须搬进 trash");
    assert!(!sessions.join("s1.jsonl").exists());
    assert!(sessions.join("s2.jsonl").is_file(), "不匹配的档不得被动");
    let _ = fs::remove_dir_all(&root);
}

/// 显式 `--sessions-dir` 永远照旧（即使适配器是 `generic`，也不走台账捷径）。
#[test]
fn an_explicit_sessions_dir_ignores_the_adapter() {
    let root = temp_root("explicit-dir");
    let dir = root.join("sessions");
    fs::create_dir_all(&dir).expect("建显式目录");
    fs::write(dir.join("s1.jsonl"), "parent-777\n").expect("写命中档");
    let mut inputs = inputs("generic", &root);
    inputs.sessions_dir = Some(dir.clone());
    inputs.home = None;
    let out = archive_sessions_fields_with(
        &flags(&[("--parent", "parent-777"), ("--batch", "b1")]),
        &inputs,
    )
    .expect("显式目录不受适配器影响");
    let (moved, remaining, _dest) = outcome(&out);
    assert_eq!((moved.as_str(), remaining.as_str()), ("1", "0"));
    // 显式目录的 trash 根仍按老口径推导：`<dir 的父目录>/trash/<batch>`。
    assert!(root.join("trash").join("b1").join("s1.jsonl").is_file());
    let _ = fs::remove_dir_all(&root);
}
