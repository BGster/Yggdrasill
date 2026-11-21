use super::{fs, mod_signature, watched_files, Args, Drive, OutputFormat, Path, PathBuf, Session};
use serde_json::Value as Json;

/// 入库 fixture 的目录（`tests/fixtures/multi-script`；与 r4 的三个测试目标同一份）。
fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/multi-script")
}

/// 递归复制（测试要改文件，不能动入库 fixture）。
fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("建目标目录");
    for item in fs::read_dir(from).expect("读源目录") {
        let item = item.expect("目录项");
        let source = item.path();
        let target = to.join(item.file_name());
        if source.is_dir() {
            copy_dir(&source, &target);
        } else {
            fs::copy(&source, &target).expect("复制文件");
        }
    }
}

/// fixture 的临时副本（每个用例独占目录）。
fn fixture_copy(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("egg-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    copy_dir(&fixture_dir(), &dir);
    dir
}

/// 单发模式的宿主参数（`--drive none`：测试自己派发，不推 stdin）。
///
/// `store` 钉住 `Some(Json)`：本模块断言的是内核状态与叙事（与存储无关），
/// 但后端一旦随默认值漂到 sqlite，热载失败会变成「sqlite 在某平台出问题」
/// 的假象。默认后端归 `persistence_reset_semantics_tests` 的 `r10*` 专测。
fn args_for(dir: &Path, run_id: &str) -> Args {
    Args {
        packages: None,
        mod_dir: Some(dir.to_path_buf()),
        open_style: Some(crate::OpenStyle::New),
        run_root: dir.join("runs").join(run_id),
        runs_parent: dir.join("runs"),
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

/// 派发一步并取叙事行（信封必须 ok）。
#[track_caller]
fn narrate(session: &mut Session, event: &str) -> Vec<String> {
    let envelope = super::parse_envelope(&session.kernel.dispatch(event, "0"));
    assert_eq!(
        envelope["ok"],
        Json::Bool(true),
        "{event} 派发失败：{envelope}"
    );
    super::narrative_lines(&envelope)
}

#[test]
fn module_files_are_watched_so_editing_one_flips_the_signature() {
    let dir = fixture_copy("watch-modules");
    let watched = watched_files(&dir);
    // 五件套 + 资源 + schema + **每个模块**（r2：漏一个就静默跑旧代码）
    // + **配置的解析结果**（r4 缺陷：本 fixture 的 pack 未声明
    // `ygg:agent/config`，走回落 `data/config.json`——只收声明路径时，
    // 改回落配置不翻转签名，热载静默不触发）。
    for path in [
        "mod.lua",
        "pack.json",
        "manifest.json",
        "data/config.json",
        "data/scenes.json",
        "schemas/scene-texts.json",
        "src/util.lua",
        "src/handlers.lua",
        "src/shared.lua",
        "src/silent.lua",
    ] {
        assert!(
            watched.iter().any(|entry| entry == path),
            "watched_files 必须纳管 `{path}`：{watched:?}"
        );
    }

    let before = mod_signature(&dir);
    // 只改一个模块文件（长度也变：不依赖 mtime 分辨率）。
    let util = fs::read_to_string(dir.join("src/util.lua")).expect("读模块");
    fs::write(dir.join("src/util.lua"), format!("{util}\n-- 热载探针\n")).expect("写模块");
    let after = mod_signature(&dir);
    assert!(before != after, "改模块文件必须让 mod_signature 变");

    // 变的**只有**被改的那个文件（签名不是"任何风吹草动都变"）。
    let changed: Vec<&str> = before
        .iter()
        .zip(after.iter())
        .filter(|(left, right)| left != right)
        .map(|(stamp, _)| stamp.path.as_str())
        .collect();
    assert_eq!(changed, vec!["src/util.lua"], "签名变化面必须精确");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn hot_reload_swaps_the_code_and_rolls_back_on_failure() {
    let dir = fixture_copy("hot-reload");
    let mut session = Session::open(&args_for(&dir, "hot-reload")).expect("开局");
    assert!(session.reload.is_none(), "开局不该有热载事件");

    // 模块真的装上了：handler 来自 src.util 的 table（`probe.kv` 由入口注册）。
    assert_eq!(narrate(&mut session, "probe.kv"), ["stage=nil"]);

    // ① 合法改动（只动入口：`mod.lua` 不在内容寻址清单里，改它不需要重算
    //    manifest）→ 签名变 → 热载成功 → 新代码立即生效。
    let entry = fs::read_to_string(dir.join("mod.lua")).expect("读入口");
    fs::write(
        dir.join("mod.lua"),
        format!("{entry}\nygg.on(\"probe.kv\", function()\n  ygg.narrate(\"entry-v2\")\nend)\n"),
    )
    .expect("写入口");

    session.maybe_reload();
    let reload = session.reload.take().expect("签名变了必须触发热载");
    assert!(
        reload.reloaded,
        "合法改动必须换上新包：{:?}",
        reload.message
    );
    assert_eq!(reload.source, "watch");
    assert_eq!(
        narrate(&mut session, "probe.kv"),
        ["stage=nil", "entry-v2"],
        "新入口的 handler 必须立即生效"
    );
    // 换装不是"再叠一层"：模块各执行一次、handler 注册序不重复。
    assert_eq!(
        narrate(&mut session, "probe.order"),
        ["util", "shared", "handlers", "entry"],
        "热载后注册序必须与首装一致（不重复注册）"
    );
    assert_eq!(
        narrate(&mut session, "probe.stats"),
        ["util_execs=1;shared_runs=1;silent_runs=1;util_count=0;shared_count=0"],
        "热载重建模块缓存（各执行一次，模块内状态归零）"
    );

    // ② 破损改动（入口语法错）→ 装载失败 → `reloaded=false` + 旧 mod 继续跑。
    let broken = format!("{entry}\nygg.on(\"probe.kv\", function()\n  ygg.narrate(\"entry-v3\")\nend\nlocal oops = = 1\n");
    fs::write(dir.join("mod.lua"), broken).expect("写破损入口");
    session.maybe_reload();
    let reload = session.reload.take().expect("签名变了必须尝试热载");
    assert!(!reload.reloaded, "破损包不得换上");
    let message = reload.message.unwrap_or_default();
    assert!(
        message.contains("mod.lua"),
        "失败原因必须点名入口文件：{message}"
    );

    // 旧驱动仍在跑：上一个成功版本的行为原样保留（不留半更新 —— 既没上新的
    // 半截代码，也没丢掉已经生效的 v2）。
    assert_eq!(
        narrate(&mut session, "probe.kv"),
        ["stage=nil", "entry-v2"],
        "热载失败后旧 mod 必须继续服务"
    );
    assert_eq!(
        narrate(&mut session, "probe.order"),
        ["util", "shared", "handlers", "entry"]
    );

    let _ = fs::remove_dir_all(&dir);
}
