use super::{resolve_mod_dir, Args, Drive, PathBuf};
use std::fs;

/// 独占临时根（每个用例一个；先清干净，避免上次残留改变 `exists()` 结论）。
fn temp_root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ygg-modalias-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("建临时根");
    dir
}

/// 只在 `mod_dir` 上有差异的宿主参数（本模块不派发，其余字段钉成「续局单发」）。
fn args(mod_dir: Option<PathBuf>) -> Args {
    Args {
        packages: None,
        mod_dir,
        open_style: Some(crate::OpenStyle::Open),
        run_root: PathBuf::from("ygg-runs-should-not-be-touched/r1"),
        runs_parent: PathBuf::from("ygg-runs-should-not-be-touched"),
        force: false,
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

/// ① 历史 `run.json` 记旧名（精确形状）→ 专名拒绝：点名规范路径 + 指引开新 run-id。
#[test]
fn legacy_run_json_gets_the_named_refusal() {
    let root = temp_root("refuse");
    let legacy = root.join("mods/pi-agent-seed");
    assert!(!legacy.exists(), "前提：旧名目录不存在（退役形态）");
    let run_dir = root.join("runs/r1");
    fs::create_dir_all(&run_dir).expect("建 run 目录");
    fs::write(
        run_dir.join("run.json"),
        format!(r#"{{"mod":"{}"}}"#, legacy.display()),
    )
    .expect("写 run.json");
    let err = resolve_mod_dir(&args(None), &run_dir).expect_err("退役旧名必须拒绝");
    assert!(err.contains("pi-agent-seed"), "点名旧名：{err}");
    assert!(err.contains("agent-seed"), "点名同父规范路径：{err}");
    assert!(err.contains("run-id"), "指引开新局：{err}");
    assert!(err.contains("2026-11-12"), "宣布退役日期：{err}");
    let _ = fs::remove_dir_all(&root);
}

/// ② 旧相对形状（`.ygg/mods/pi-agent-seed`）同样专名拒绝（含退役日期与规范名）。
#[test]
fn legacy_relative_shape_gets_the_named_refusal() {
    let root = temp_root("rel");
    let run_dir = root.join("runs/r1");
    fs::create_dir_all(&run_dir).expect("建 run 目录");
    let err = resolve_mod_dir(
        &args(Some(PathBuf::from(".ygg/mods/pi-agent-seed"))),
        &run_dir,
    )
    .expect_err("旧相对形状必须拒绝（不再翻译）");
    assert!(err.contains("2026-11-12"), "宣布退役日期：{err}");
    assert!(err.contains("agent-seed"), "点名规范路径：{err}");
    let _ = fs::remove_dir_all(&root);
}

/// ③ 只认精确形状：前缀撞名与裸值不走专名拒绝，维持通用缺席语义（原样返回）。
#[test]
fn only_the_exact_shape_gets_the_named_refusal() {
    let root = temp_root("shape");
    let run_dir = root.join("runs/r1");
    fs::create_dir_all(&run_dir).expect("建 run 目录");
    for untouched in [
        "pi-agent-seed",              // 无 `mods/` 父段（裸值）
        "/tmp/pi-agent-seed",         // 父段不叫 mods（临时探针目录常态）
        ".ygg/mods/pi-agent-seed-v2", // 前缀撞名
        ".ygg/mods/pi-agent-seeds",
    ] {
        let resolved = resolve_mod_dir(&args(Some(PathBuf::from(untouched))), &run_dir)
            .expect("非精确形状不走专名拒绝（通用错误留在装载期）");
        assert_eq!(
            resolved,
            PathBuf::from(untouched),
            "{untouched} 必须原样返回"
        );
    }
    let _ = fs::remove_dir_all(&root);
}

/// ④ 规范名缺席时即使盘上只剩旧名目录也不收养：原样返回（缺席语义），绝不翻译。
#[test]
fn missing_canonical_never_adopts_the_legacy_dir() {
    let root = temp_root("adopt");
    let legacy = root.join("mods/pi-agent-seed");
    fs::create_dir_all(&legacy).expect("建旧名目录");
    fs::write(legacy.join("pack.json"), "{}").expect("放一个标记文件");
    let requested = root.join("mods/agent-seed");
    assert!(!requested.exists(), "前提：规范名目录不存在");
    let resolved = resolve_mod_dir(&args(Some(requested.clone())), &root.join("runs/r1"))
        .expect("规范名缺席 = 通用缺席语义（原样返回，装载期报错）");
    assert_eq!(resolved, requested, "绝不收养旧名目录");
    let _ = fs::remove_dir_all(&root);
}

/// ⑤ 盘上旧目录不被改写：显式请求**存在的**旧名目录 → 原样使用，不重命名不翻译。
#[test]
fn an_existing_legacy_dir_is_never_rewritten() {
    let root = temp_root("keep-old");
    let legacy = root.join("mods/pi-agent-seed");
    fs::create_dir_all(&legacy).expect("建旧名目录");
    fs::write(legacy.join("pack.json"), "{}").expect("放一个标记文件");
    let resolved = resolve_mod_dir(&args(Some(legacy.clone())), &root.join("runs/r1"))
        .expect("存在的请求路径原样使用（退役禁令只针对缺席翻译）");
    assert_eq!(resolved, legacy);
    assert!(legacy.is_dir(), "旧目录保持原位（不做磁盘改写）");
    let _ = fs::remove_dir_all(&root);
}

/// ⑥ `--mod` 旗标与 run.json 走同一条解析：指向旧名（缺席）时同一份专名拒绝文案。
#[test]
fn the_mod_flag_takes_the_same_refusal() {
    let root = temp_root("flag");
    let run_dir = root.join("runs/r1");
    fs::create_dir_all(&run_dir).expect("建 run 目录");
    let via_flag = resolve_mod_dir(&args(Some(root.join("mods/pi-agent-seed"))), &run_dir)
        .expect_err("旗标旧名必须拒绝");
    let meta_run = root.join("runs/r2");
    fs::create_dir_all(&meta_run).expect("建 run 目录");
    fs::write(
        meta_run.join("run.json"),
        format!(
            r#"{{"mod":"{}"}}"#,
            root.join("mods/pi-agent-seed").display()
        ),
    )
    .expect("写 run.json");
    let via_meta = resolve_mod_dir(&args(None), &meta_run).expect_err("run.json 旧名必须拒绝");
    assert_eq!(via_flag, via_meta, "两条入口同一份拒绝文案");
    let _ = fs::remove_dir_all(&root);
}

/// ⑦ 规范名缺席的通用语义保留：非退役形状原样返回；缺 --mod 且 run.json 无记录
/// 仍是既有可读错误（fail-closed 不变）。
#[test]
fn canonical_missing_keeps_the_generic_error() {
    let root = temp_root("generic");
    let run_dir = root.join("runs/r1");
    fs::create_dir_all(&run_dir).expect("建 run 目录");
    let requested = root.join("mods/agent-seed");
    let resolved = resolve_mod_dir(&args(Some(requested.clone())), &run_dir)
        .expect("规范名缺席 = 通用缺席语义（原样返回）");
    assert_eq!(resolved, requested);
    let empty = root.join("runs/r2");
    fs::create_dir_all(&empty).expect("建空 run 目录");
    let message = resolve_mod_dir(&args(None), &empty).expect_err("缺 Mod 目录必须报错");
    assert!(message.contains("--mod"), "{message}");
    let _ = fs::remove_dir_all(&root);
}
