use super::support::*;
use super::tests_gates::expect_open_failure;
use super::*;
use super::{
    acquire_run_lock, begin_write, enter_closing, finalize_run, leave_closing, run_lock_heartbeat,
    run_single, Args, ClosingEntry, Drive, ResetOutcome, Session, WriteGate, AUTOSAVE_FILE,
    COORDINATION_LOCK, INBOX_FILE,
};
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use ygg_store::{EVENTS_FILE, SQLITE_FILE_NAME};

/// R1：无副作用 resume 不重置——健康局续局零归档、零重置痕迹、resumed=true。
#[test]
fn r1_healthy_resume_never_resets() {
    let dir = case_dir("r1");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_probe_mod(&mod_dir);
    new_game_with_one_step(&mod_dir, &runs_root, "r1");
    let run_dir = runs_root.join("r1");

    let session = Session::open(&probe_args(&mod_dir, &runs_root, "r1", true))
        .expect("健康局 resume 必须成功");
    assert!(session.resumed, "读档成功必须标记 resumed");
    assert!(
        reset_archives(&run_dir).is_empty(),
        "无副作用 resume 不得产生归档"
    );
    assert_eq!(event_line_count(&run_dir), 1, "事件日志不得被动过");
    assert!(
        !session.notices.iter().any(|notice| notice.contains("重置")),
        "不得出现重置提示：{:?}",
        session.notices
    );
    let _ = fs::remove_dir_all(&dir);
}

/// R2：显式新局（非 resume）对已有产物归档**恰好一次**，旧行全在归档里。
#[test]
fn r2_explicit_new_game_archives_exactly_once() {
    let dir = case_dir("r2");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_probe_mod(&mod_dir);
    new_game_with_one_step(&mod_dir, &runs_root, "r2");
    let run_dir = runs_root.join("r2");

    let session =
        Session::open(&probe_args(&mod_dir, &runs_root, "r2", false)).expect("显式新局必须成功");
    let archives = reset_archives(&run_dir);
    assert_eq!(archives.len(), 1, "显式新局必须归档恰好一次");
    assert_eq!(archive_line_count(&archives[0]), 1, "旧代行必须全在归档里");
    assert_eq!(event_line_count(&run_dir), 0, "新一代事件日志从空起步");
    assert!(
        session
            .notices
            .iter()
            .any(|notice| notice.contains("已有旧产物，按新局重置")),
        "必须有旧产物提示：{:?}",
        session.notices
    );
    let _ = fs::remove_dir_all(&dir);
}

/// R3：**可解析但不可装载**的 autosave（版本不符）→ 归档后新局，绝不混代。
/// 这是旧判据（可解析即「已恢复」）的实测 RED 面：修复前会跳过重置并以
/// 全新内核续跑旧代目录——旧 autosave 被无归档覆盖、事件日志混两代。
#[test]
fn r3_parseable_but_unloadable_autosave_archives_not_joins() {
    let dir = case_dir("r3");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_probe_mod(&mod_dir);
    new_game_with_one_step(&mod_dir, &runs_root, "r3");
    let run_dir = runs_root.join("r3");

    // 注入「可解析但不可装载」：JSON 合法、engine_version 不符（内核 load 拒绝）。
    let autosave_path = run_dir.join(AUTOSAVE_FILE);
    let mut save: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&autosave_path).expect("读存档"))
            .expect("存档是 JSON");
    save["engine_version"] = serde_json::json!("0.0.0-not-an-engine");
    fs::write(&autosave_path, save.to_string()).expect("写坏版本号");

    // r1 语义迁移：旧「resume 失败按新局开局」路径已删除（Attach fail-closed）。
    // 等价不变量改经 Fresh（new --force）承接：不读旧档直接重置，归档不混入。
    let mut session = Session::open(&probe_args(&mod_dir, &runs_root, "r3", false))
        .expect("显式新局 open 必须成功");
    assert!(!session.resumed, "新局不得标记 resumed");
    let archives = reset_archives(&run_dir);
    assert_eq!(archives.len(), 1, "不可装载的旧代必须归档（证据不丢）");
    assert_eq!(archive_line_count(&archives[0]), 1, "旧代行全在归档里");
    assert_eq!(event_line_count(&run_dir), 0, "新一代从空起步");

    // 关键反混代断言：新局第一步 seq 必须是 0（修复前会接旧代行号 seq=1）。
    let receipt = session.dispatch_event("event.start", "0");
    assert_eq!(receipt["ok"], serde_json::json!(true));
    assert_eq!(
        receipt["seq"],
        serde_json::json!(0),
        "新局第一步必须从 seq 0 起步（混代即 1）"
    );
    assert_eq!(event_line_count(&run_dir), 1, "events 里只有新代这一行");
    let _ = fs::remove_dir_all(&dir);
}

/// R4：续局失败但盘上状态已可装载（另一进程已救活）→ 锁内重试读档 adopt，
/// 不归档别人的行、不重置。复刻 Session::open 的两段序：首次读档失败（坏档）
/// → 「另一进程」把档修好 → reset_artifacts(true) 必须 Recovered。
#[test]
fn r4_recheck_adopts_recovered_state_without_archiving() {
    let dir = case_dir("r4");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_probe_mod(&mod_dir);
    new_game_with_one_step(&mod_dir, &runs_root, "r4");
    let run_dir = runs_root.join("r4");
    let autosave_path = run_dir.join(AUTOSAVE_FILE);

    let mut session = Session::open(&probe_args(&mod_dir, &runs_root, "r4", true))
        .expect("健康局 resume 必须成功");
    assert!(session.resumed);

    // ① 本进程视角「续局失败」：坏档（复刻 open 的首次读档在锁外失败）。
    let good_save = fs::read_to_string(&autosave_path).expect("读好档");
    fs::write(&autosave_path, "corrupted-bytes").expect("写坏档");
    assert!(
        session.load_autosave().is_err(),
        "坏档读档必须失败（进入二次判定路径的前提）"
    );

    // ② 「另一进程」恢复：把好档写回（重置后起步 / 外部修复，对判据等价）。
    fs::write(&autosave_path, &good_save).expect("另一进程恢复好档");

    // ③ 二次判定：重试读档成功 → Recovered，不归档、不动事件日志。
    let outcome = session.reset_artifacts(true);
    assert!(
        matches!(outcome, Ok(ResetOutcome::Recovered)),
        "盘上状态可装载必须 adopt，不得重置"
    );
    assert!(reset_archives(&run_dir).is_empty(), "不得归档别人的行");
    assert_eq!(event_line_count(&run_dir), 1, "事件日志必须原样保留");
    let _ = fs::remove_dir_all(&dir);
}

// ─────────── 默认后端（r-evo）：新局落 sqlite，旧局**绝不**被变更 ───────────
//
// 这组用例是「默认值」（`ygg_store::DEFAULT_BACKEND`）的唯一判据面：既有用例
// 全部钉住 JSON（见 [`probe_args`]），所以默认值退化不会把它们的红绿弄反。

/// R10a：空目录 + 不传 `--store` → 新局落默认后端 sqlite，且**不产任何 JSON 文件**。
#[cfg(feature = "store-sqlite")]
#[test]
fn r10a_fresh_run_defaults_to_sqlite() {
    let dir = case_dir("r10a");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_probe_mod(&mod_dir);
    let mut session = Session::open(&probe_args_default_store(
        &mod_dir, &runs_root, "r10a", false,
    ))
    .expect("默认后端开局必须成功");
    assert_eq!(
        session.store.backend(),
        ygg_store::Backend::Sqlite,
        "空目录的新局必须落默认后端 sqlite"
    );
    let receipt = session.dispatch_event("event.start", "0");
    assert_eq!(
        receipt["ok"],
        serde_json::json!(true),
        "开局步必须成功：{receipt}"
    );
    let run_dir = runs_root.join("r10a");
    assert!(
        run_dir.join(SQLITE_FILE_NAME).exists(),
        "默认后端必须真的建出 run.sqlite"
    );
    assert!(
        !run_dir.join(AUTOSAVE_FILE).exists() && !run_dir.join(EVENTS_FILE).exists(),
        "sqlite 局不得写出另一后端的产物"
    );
    drop(session);
    let meta = super::read_meta(&run_dir.join(super::META_FILE)).expect("run.json");
    assert_eq!(
        meta["store"].as_str(),
        Some("sqlite"),
        "开局即把后端事实写进 run.json（权威面）"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// R10b：`--store json` 仍是回退通道——空目录上写 JSON 产物、不建库。
/// r1 `--at` 跨代回退（sqlite）：g0 推进两步 → New+force 重置为 g1（旧代归档）→
/// open --at g0 = 取 g0 末快照建新代 g2 续跑；状态 = g0 末（stage=started），
/// 旧代证据全留库（savepoints 三代可查），新代事件从空起步。
#[test]
fn rollback_to_earlier_generation_restores_save_and_archives() {
    let dir = case_dir("rollback");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_probe_mod(&mod_dir);
    let run_id = "rb1";
    // g0：开局 + event.start（stage=started）。
    let mut session = Session::open(&probe_args_default_store(
        &mod_dir, &runs_root, run_id, false,
    ))
    .expect("g0 开局");
    let receipt = session.dispatch_event("event.start", "0");
    assert_eq!(receipt["ok"], serde_json::json!(true));
    drop(session);
    // g1：显式新局重置（New+force；旧代归档留库）+ 推进一步（有步才归档，
    // 零步代重置不推新代——sqlite reset 既有语义）。
    let mut session = Session::open(&probe_args_default_store(
        &mod_dir, &runs_root, run_id, false,
    ))
    .expect("g1 重置");
    let receipt = session.dispatch_event("event.start", "0");
    assert_eq!(receipt["ok"], serde_json::json!(true));
    let points = session.store.savepoints().expect("存档点清单");
    assert_eq!(points.len(), 2, "重置后应有 g0/g1 两代：{points:?}");
    assert!(points[1].current);
    drop(session);
    // open --at g0：回退到 g0 末快照（stage=started），新代 g2 续跑。
    let args = Args {
        open_style: Some(crate::OpenStyle::Open),
        at: Some(0),
        ..probe_args_default_store(&mod_dir, &runs_root, run_id, true)
    };
    let mut session = Session::open(&args).expect("--at g0 回退必须成功");
    assert!(session.resumed, "回退按附着计");
    assert!(
        session
            .notices
            .iter()
            .any(|notice| notice.contains("已回退到存档点 g0")),
        "必须有回退提示：{:?}",
        session.notices
    );
    let save: serde_json::Value =
        serde_json::from_slice(&session.current_save().expect("现态存档")).expect("存档是 JSON");
    assert_eq!(
        save["kv"]["entries"]["stage"],
        serde_json::json!("started"),
        "状态必须回到 g0 末（stage=started）"
    );
    let points = session.store.savepoints().expect("回退后清单");
    assert_eq!(points.len(), 3, "回退后 g0/g1/g2 三代都在库：{points:?}");
    assert!(points[2].current, "新代是当前代");
    // 越界代：可读错误点名可用存档点。
    let bad = Args {
        at: Some(99),
        ..args
    };
    expect_open_failure(&bad, &["g99", "可用存档点"]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn r10b_json_flag_new_run_writes_json_files() {
    let dir = case_dir("r10b");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_probe_mod(&mod_dir);
    let mut session = Session::open(&probe_args(&mod_dir, &runs_root, "r10b", false))
        .expect("显式 --store json 必须可开新局");
    assert_eq!(
        session.store.backend(),
        ygg_store::Backend::Json,
        "旗标必须能压过默认值（回退通道）"
    );
    let receipt = session.dispatch_event("event.start", "0");
    assert_eq!(
        receipt["ok"],
        serde_json::json!(true),
        "开局步必须成功：{receipt}"
    );
    let run_dir = runs_root.join("r10b");
    assert_eq!(event_line_count(&run_dir), 1, "json 局必须有事件行");
    assert!(
        !run_dir.join(SQLITE_FILE_NAME).exists(),
        "--store json 不得建出 run.sqlite"
    );
    drop(session);
    let meta = super::read_meta(&run_dir.join(super::META_FILE)).expect("run.json");
    assert_eq!(
        meta["store"].as_str(),
        Some("json"),
        "run.json 必须是事实值"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// R10c：旧 JSON 局（`run.json` 无 `store`）+ 不传 `--store` → 仍按产品推断
/// `json`，**绝不**误建 `run.sqlite`（需求 2 的正面判据：改默认值不得改动旧局）。
#[test]
fn r10c_legacy_json_run_never_gets_sqlite_db() {
    let dir = case_dir("r10c");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_probe_mod(&mod_dir);
    new_game_with_one_step(&mod_dir, &runs_root, "r10c");
    let run_dir = runs_root.join("r10c");

    // 复刻旧版本 CLI 写的目录：抹掉 store 事实，只留纯 JSON 产物。
    let meta_path = run_dir.join(super::META_FILE);
    let mut meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&meta_path).expect("读 run.json"))
            .expect("run.json 是 JSON");
    meta.as_object_mut().expect("对象").remove("store");
    fs::write(&meta_path, meta.to_string()).expect("写回无 store 的 run.json");
    let events_before = event_line_count(&run_dir);
    let autosave_before = fs::read(run_dir.join(AUTOSAVE_FILE)).expect("读旧存档");

    let session = Session::open(&probe_args_default_store(
        &mod_dir, &runs_root, "r10c", true,
    ))
    .expect("旧 JSON 局必须仍可续局");
    assert!(session.resumed, "旧局续局必须标记 resumed");
    assert_eq!(
        session.store.backend(),
        ygg_store::Backend::Json,
        "有 JSON 产物 → 必须推断 json（默认值不得压过产品探针）"
    );
    drop(session);

    assert!(
        !run_dir.join(SQLITE_FILE_NAME).exists(),
        "旧 JSON 局绝不得被建出 run.sqlite（混代就不可逆了）"
    );
    assert_eq!(
        event_line_count(&run_dir),
        events_before,
        "续局不得动旧事件"
    );
    assert_eq!(
        fs::read(run_dir.join(AUTOSAVE_FILE)).expect("旧存档还在"),
        autosave_before,
        "续局不得重写旧存档"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// R10d：旧 JSON 局 `--settle`（走 `run_settle` → `open_store_resolved`）仍落
/// json：报告写得出、无 `run.sqlite`。
#[test]
fn r10d_settle_on_legacy_json_run_stays_json() {
    let dir = case_dir("r10d");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_probe_mod(&mod_dir);
    new_game_with_one_step(&mod_dir, &runs_root, "r10d");
    let run_dir = runs_root.join("r10d");
    let meta_path = run_dir.join(super::META_FILE);
    let mut meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&meta_path).expect("读 run.json"))
            .expect("run.json 是 JSON");
    meta.as_object_mut().expect("对象").remove("store");
    fs::write(&meta_path, meta.to_string()).expect("写回无 store 的 run.json");

    let mut args = probe_args_default_store(&mod_dir, &runs_root, "r10d", false);
    args.settle = true;
    // 只有 1 步、未到结局 → 结算不会给绿；本用例判定的是**后端**，不是结算结论。
    let code = super::run_settle(&args, false);
    assert_eq!(code, 1, "只有 1 步（未到结局）→ 结算不得给绿");

    assert!(
        run_dir.join(super::SETTLEMENT_FILE).exists(),
        "结算报告必须写得出（旧局读得到自己的产品）"
    );
    assert!(
        !run_dir.join(SQLITE_FILE_NAME).exists(),
        "旧局 --settle 不得建库"
    );
    let store = super::open_store_resolved(&run_dir, None).expect("旧局仍须以 json 打开");
    assert_eq!(
        store.backend(),
        ygg_store::Backend::Json,
        "--settle 不得把旧局的后端翻成 sqlite"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// R10e：新局（默认 sqlite）落定后，`--store json` 只是开局断言 → fail-closed。
/// 反向断言：拒绝不得删已有库、也不得写出另一后端的产物。
#[cfg(feature = "store-sqlite")]
#[test]
fn r10e_new_run_then_declared_fact_locks_backend() {
    let dir = case_dir("r10e");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_probe_mod(&mod_dir);
    let mut session = Session::open(&probe_args_default_store(
        &mod_dir, &runs_root, "r10e", false,
    ))
    .expect("默认后端开局必须成功");
    let receipt = session.dispatch_event("event.start", "0");
    assert_eq!(
        receipt["ok"],
        serde_json::json!(true),
        "开局步必须成功：{receipt}"
    );
    drop(session);
    let run_dir = runs_root.join("r10e");
    assert!(run_dir.join(SQLITE_FILE_NAME).exists(), "前提：新局已落库");

    let args = probe_args(&mod_dir, &runs_root, "r10e", true); // store: Some(Json)
    let message = expect_open_failure(
        &args,
        &[
            "sqlite",
            "json",
            "run.json",
            "--store",
            "未做任何写入",
            "换 --run-id",
        ],
    );
    assert!(
        message.contains("权威"),
        "必须一句话讲清谁是权威：{message}"
    );
    assert!(
        run_dir.join(SQLITE_FILE_NAME).exists(),
        "拒绝开局不得删已有库（证据不删）"
    );
    assert!(
        !run_dir.join(AUTOSAVE_FILE).exists() && !run_dir.join(EVENTS_FILE).exists(),
        "拒绝开局不得写出另一后端的产物"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// R10f：回退构建（`--no-default-features`）下开新局 → fail-closed 报
/// 「未启用 sqlite」，且目录里不出现任何后端产物（默认后端不可用 **不**
/// 等于静默降级到 json——那样会在用户不知情时换掉崩溃承诺）。
#[cfg(not(feature = "store-sqlite"))]
#[test]
fn r10f_default_sqlite_without_feature_fails_closed() {
    let dir = case_dir("r10f");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_probe_mod(&mod_dir);
    expect_open_failure(
        &probe_args_default_store(&mod_dir, &runs_root, "r10f", false),
        &["未启用 sqlite 后端", "--no-default-features"],
    );
    let run_dir = runs_root.join("r10f");
    for product in [AUTOSAVE_FILE, EVENTS_FILE, SQLITE_FILE_NAME] {
        assert!(
            !run_dir.join(product).exists(),
            "回退构建不得静默落到某一后端（出现了 {product}）"
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

/// T1（r11 fail-closed）：单发 `--event` 未注册事件 = 可读 VALIDATION 拒绝。
/// 三层断言：① `run_single` 返回 1（ok=false 的既有退出码映射）；
/// ② 审计行照写且 ok=false（与 payload 校验拒绝同形）；③ 回执形
/// ok:false + `error.code=VALIDATION` + 文案三要素（事件名 + 计数 +
/// 已注册全集字典序样本）。零产品代码：这是既有失败步链路的自然呈现。
#[test]
fn t1_single_shot_unknown_event_fails_closed_with_readable_validation() {
    let dir = case_dir("t1");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_probe_mod(&mod_dir);

    // ① 单发路径本体：未注册事件 → 非零退出（run_single 的 ok 分支）。
    let args = Args {
        event: Some("event.nope".to_owned()),
        ..probe_args(&mod_dir, &runs_root, "t1", false)
    };
    assert_eq!(run_single(&args), 1, "未知事件必须非零退出");

    // ② 审计行 ok=false 照写（拒绝不落存档，但事件日志必须有行可查）。
    let mut session = Session::open(&probe_args(&mod_dir, &runs_root, "t1", true))
        .expect("失败步之后必须可续局（审计行已落、无存档写入）");
    let records = session.store.steps().expect("读事件日志").records;
    assert_eq!(records.len(), 1, "拒绝也必须落一行审计：{records:?}");
    assert_eq!(records[0].event, "event.nope");
    assert!(!records[0].ok, "审计行必须 ok=false：{records:?}");
    drop(session);

    // ③ 回执形（stdout 单行 JSON 的域内）：经 run_single 同一 dispatch_event
    //    路径断言信封字段。write_probe_mod 注册 4 个事件 + stdlib 内置 1 个。
    let mut session =
        Session::open(&probe_args(&mod_dir, &runs_root, "t1b", false)).expect("新局开局");
    let receipt = session.dispatch_event("event.nope", "0");
    assert_eq!(receipt["ok"], serde_json::json!(false), "{receipt}");
    assert_eq!(
        receipt["error"]["code"],
        serde_json::json!("VALIDATION"),
        "{receipt}"
    );
    assert_eq!(receipt["narrative"], serde_json::json!([]), "{receipt}");
    assert!(receipt.get("autosave").is_some(), "{receipt}");
    let message = receipt["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.starts_with("未注册事件 `event.nope`："),
        "文案要点名事件：{receipt}"
    );
    assert!(
        message.contains("已注册 5 个事件"),
        "文案要带计数：{receipt}"
    );
    for sample in [
        "agent.summary",
        "event.start",
        "inbox.add",
        "ygg.choice_selected",
        "ygg.narrative.goto",
    ] {
        assert!(
            message.contains(sample),
            "文案要含已注册全集（字典序）样本 {sample}：{receipt}"
        );
    }
    let _ = fs::remove_dir_all(&dir);
}
