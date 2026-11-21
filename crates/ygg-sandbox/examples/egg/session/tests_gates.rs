use super::support::*;
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

/// R5：锁心跳——真实持锁才续命（mtime 前移），未持锁绝不触碰别人的锁文件。
#[test]
fn r5_lock_heartbeat_only_while_holding() {
    let dir = case_dir("r5");
    let lock_path = dir.join(COORDINATION_LOCK);

    // ① 未持锁：心跳是空操作，连别人的锁文件都不碰。
    fs::write(&lock_path, b"someone-else").expect("造一把别人的锁");
    let stale = SystemTime::now() - Duration::from_secs(60);
    fs::OpenOptions::new()
        .write(true)
        .open(&lock_path)
        .expect("打开别人的锁")
        .set_modified(stale)
        .expect("回拨 mtime");
    run_lock_heartbeat();
    let untouched = fs::metadata(&lock_path)
        .expect("锁还在")
        .modified()
        .expect("读 mtime");
    assert!(
        untouched <= SystemTime::now() - Duration::from_secs(55),
        "未持锁的心跳绝不能 bump 别人的锁"
    );

    // ② 真实持锁：心跳把 mtime 从「陈旧」续到「新鲜」（陈旧抢占判据不再误伤活锁）。
    let _guard = acquire_run_lock(&dir).expect("acquire 应成功");
    assert_eq!(super::RUN_LOCK_DEPTH.with(Cell::get), 1);
    fs::OpenOptions::new()
        .write(true)
        .open(&lock_path)
        .expect("打开自己的锁")
        .set_modified(stale)
        .expect("回拨 mtime（模拟持锁已超过 LOCK_STALE_MS）");
    run_lock_heartbeat();
    let refreshed = fs::metadata(&lock_path)
        .expect("锁还在")
        .modified()
        .expect("读 mtime");
    assert!(
        refreshed > SystemTime::now() - Duration::from_secs(55),
        "持锁心跳必须把 mtime 续到新鲜（防等待者按陈旧抢占活锁）"
    );

    drop(_guard);
    assert_eq!(
        super::RUN_LOCK_DEPTH.with(Cell::get),
        0,
        "释放后 depth 归零"
    );
    assert!(!lock_path.exists(), "释放后锁文件删除");
    let _ = fs::remove_dir_all(&dir);
}

/// R6：drain 逐行消费一次（cursor 语义），且全程在真实锁内（心跳接线点冒烟）。
#[test]
fn r6_drain_consumes_each_line_once_under_lock() {
    let dir = case_dir("r6");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_probe_mod(&mod_dir);
    new_game_with_one_step(&mod_dir, &runs_root, "r6");
    let run_dir = runs_root.join("r6");

    for index in 0..3 {
        use std::io::Write as _;
        let mut inbox = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(run_dir.join(INBOX_FILE))
            .expect("打开 inbox");
        writeln!(
            inbox,
            "{{\"kind\":\"add\",\"title\":\"t{index}\",\"external_id\":\"ext-{index}\"}}"
        )
        .expect("写 inbox 行");
    }

    let mut session =
        Session::open(&probe_args(&mod_dir, &runs_root, "r6", true)).expect("resume 必须成功");
    let first = session.drain_inbox();
    assert_eq!(first.applied, 3, "三行各消费一次");
    assert_eq!(first.remaining, 0, "消费完不留尾");
    assert_eq!(first.error, None, "三行都必须入队成功");
    // 关键：drain 的 dispatch 走持久化临界区，seq 与开局步连续（无重复/跳号）。
    assert_eq!(event_line_count(&run_dir), 1 + 3, "开局 1 行 + inbox 3 行");

    // 幂等：cursor 已前移，再 drain 不重放（W8 的 cursor 正常面）。
    let second = session.drain_inbox();
    assert_eq!(second.applied, 0, "cursor 已前移不得重放");
    assert_eq!(event_line_count(&run_dir), 4, "事件日志不得新增");
    let _ = fs::remove_dir_all(&dir);
}

/// R7：closing 准入门禁——running 可写 → closing 拒写 → 解除恢复 →
/// finalized 拒写。writer token / 收口协议的关键不变量。
#[test]
fn r7_closing_gate_lifecycle() {
    let dir = case_dir("r7");

    // running：可写，登记后摘除。
    let token = match begin_write(&dir) {
        Ok(token) => token,
        Err(gate) => panic!("running 必须可写，实际 {gate:?}"),
    };
    super::end_write(&dir, &token);

    // closing：写命令被拒（Closing 门）。
    assert!(matches!(enter_closing(&dir), Ok(ClosingEntry::Entered)));
    assert!(
        matches!(begin_write(&dir), Err(WriteGate::Closing)),
        "closing（心跳未过期）必须拒写"
    );

    // 解除 closing：恢复可写。
    leave_closing(&dir);
    let token = begin_write(&dir).expect("解除 closing 后必须可写");
    super::end_write(&dir, &token);

    // finalized：永久拒写（需开新局）。
    enter_closing(&dir).expect("进 closing");
    finalize_run(&dir);
    assert!(
        matches!(begin_write(&dir), Err(WriteGate::Finalized)),
        "finalized 后必须拒写"
    );
    let _ = fs::remove_dir_all(&dir);
}

// ───────────────────────── reset fail-closed（plan-r5 §2.5 的 `?` 语义）─────────────────────────
//
// 不变量 R8：reset 的取锁 / 归档 / 建档 / 初始存档任一步失败 → `Session::open`
// 必须失败（非零退出），**绝不**伪装 Reset 成功或带全新内核继续新局——盘上
// 还是旧代产物时继续 = 混代。错误注入只用目录形态的确定性手段（不依赖权限
// 断言、不等 3s 超时：旧 mtime 目录让陈旧抢占路径全速循环到失败）。

/// `Session::open` 的 fail-closed 断言：必须 Err 且错误串包含全部给定片段
///（Session 未实现 Debug，expect_err 不可用——match 直取错误串）。
#[track_caller]
pub(crate) fn expect_open_failure(args: &Args, fragments: &[&str]) -> String {
    match Session::open(args) {
        Ok(_) => panic!("开局必须失败（不得带未重置的旧代继续）"),
        Err(message) => {
            for fragment in fragments {
                assert!(
                    message.contains(fragment),
                    "错误串缺少「{fragment}」：{message}"
                );
            }
            message
        }
    }
}

/// `coordination.lock` 变成 mtime 陈旧的目录 → `create_new` 恒 EEXIST、
/// 陈旧抢占的 `remove_file` 对目录恒败 → `acquire_dir_lock` 快速耗尽重试并
/// 报错（实测 0.04s，无 sleep 路径）。
fn inject_unacquirable_lock(run_dir: &Path) {
    let lock = run_dir.join(COORDINATION_LOCK);
    fs::create_dir_all(&lock).expect("造目录形态的锁");
    let stale = SystemTime::now() - Duration::from_secs(60);
    // 目录只能只读打开（write 打开恒 EISDIR）；set_modified 走 futimens，
    // 属主 + 只读 fd 即生效（macOS/APFS 实测；平台差异归 R8/R9 边界）。
    fs::OpenOptions::new()
        .read(true)
        .open(&lock)
        .expect("打开目录")
        .set_modified(stale)
        .expect("回拨目录 mtime");
}

/// R9a：旧局（`run.json` 没有 `store` 字段）→ 按产物推断为 `json`，续局成功且
/// 开局后把事实**补写**进 `run.json`（单源落盘：只见一个键，不做双写）。
///
/// 建局显式用 JSON（[`probe_args`] 已钉住）：本用例的主题是「旧局的**产物**
/// 推断」，不是「默认后端是哪个」——默认值归 `r10*`。
#[test]
fn r9a_legacy_run_without_store_fact_infers_json() {
    let dir = case_dir("r9a");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_probe_mod(&mod_dir);
    new_game_with_one_step(&mod_dir, &runs_root, "r9a");
    let run_dir = runs_root.join("r9a");
    let meta_path = run_dir.join(super::META_FILE);

    // 抹掉 store 事实 → 复刻旧版本 CLI 写的目录（纯 JSON 产物 + 旧 5 键元信息）。
    let mut meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&meta_path).expect("读 run.json"))
            .expect("run.json 是 JSON");
    meta.as_object_mut().expect("对象").remove("store");
    fs::write(&meta_path, meta.to_string()).expect("写回无 store 的 run.json");

    let session = Session::open(&probe_args(&mod_dir, &runs_root, "r9a", true))
        .expect("旧局必须可续（向后兼容）");
    assert!(session.resumed, "旧局续局必须标记 resumed");
    assert_eq!(
        session.store.backend(),
        ygg_store::Backend::Json,
        "无 sqlite 产物 → 推断 json"
    );
    drop(session);
    let meta = super::read_meta(&meta_path).expect("run.json");
    assert_eq!(
        meta["store"].as_str(),
        Some("json"),
        "开局后必须把后端事实补写进 run.json"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// R9b：旧局（无 `store` 字段）+ `--store sqlite` → fail-closed，**零写入**。
/// 产物探针说这是 json 局，旗标不得推翻盘上事实。
///
/// 建局同样显式用 JSON（旧局回归的前提；默认后端归 `r10*`）。
#[test]
fn r9b_flag_conflicting_with_products_fails_closed() {
    let dir = case_dir("r9b");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_probe_mod(&mod_dir);
    new_game_with_one_step(&mod_dir, &runs_root, "r9b");
    let run_dir = runs_root.join("r9b");
    let meta_path = run_dir.join(super::META_FILE);
    let mut meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&meta_path).expect("读 run.json"))
            .expect("run.json 是 JSON");
    meta.as_object_mut().expect("对象").remove("store");
    fs::write(&meta_path, meta.to_string()).expect("写回无 store 的 run.json");

    let autosave_before = fs::read(run_dir.join(AUTOSAVE_FILE)).expect("读旧存档");
    let events_before = fs::read(run_dir.join(EVENTS_FILE)).expect("读旧事件");

    let mut args = probe_args(&mod_dir, &runs_root, "r9b", true);
    args.store = Some(ygg_store::Backend::Sqlite);
    let message = expect_open_failure(
        &args,
        &["json", "sqlite", "autosave.json", "--store", "未做任何写入"],
    );
    assert!(
        message.contains("run.json"),
        "文案必须指出权威事实的位置：{message}"
    );

    // 零写入：旧代产物逐字节不动，且没有开出一个 sqlite 库。
    assert_eq!(
        fs::read(run_dir.join(AUTOSAVE_FILE)).expect("旧存档还在"),
        autosave_before,
        "拒绝开局时存档不得被动"
    );
    assert_eq!(
        fs::read(run_dir.join(EVENTS_FILE)).expect("旧事件还在"),
        events_before,
        "拒绝开局时事件日志不得被动"
    );
    assert!(
        !run_dir.join("run.sqlite").exists(),
        "拒绝开局时不得建库（否则目录就同时有两套产物了）"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// R9c：`run.json` 声明 sqlite + `--store json` → fail-closed（事实优先），
/// 错误串四个要点齐全：事实值 / 来源 / 旗标值 / 谁权威 + 补救路径。
#[test]
fn r9c_declared_fact_beats_the_flag() {
    let dir = case_dir("r9c");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_probe_mod(&mod_dir);
    let run_dir = runs_root.join("r9c");
    fs::create_dir_all(&run_dir).expect("建 run 目录");
    // 只有 run.json（声明的后端是 sqlite），没有任何产物 → 杆杆就是「事实 vs 旗标」。
    fs::write(
        run_dir.join(super::META_FILE),
        r#"{"run_id":"r9c","mod":"probe","store":"sqlite"}"#,
    )
    .expect("写 run.json");

    let mut args = probe_args(&mod_dir, &runs_root, "r9c", false);
    args.store = Some(ygg_store::Backend::Json);
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
        !run_dir.join("autosave.json").exists() && !run_dir.join("run.sqlite").exists(),
        "拒绝开局时不得写出任何后端产物"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// R8a：显式新局 + 锁不可得 → open 失败、旧代产物逐字节不动、无归档。
#[test]
fn r8a_explicit_reset_without_lock_fails_closed() {
    let dir = case_dir("r8a");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_probe_mod(&mod_dir);
    new_game_with_one_step(&mod_dir, &runs_root, "r8a");
    let run_dir = runs_root.join("r8a");
    let autosave_before = fs::read(&run_dir.join(AUTOSAVE_FILE)).expect("读旧存档");
    inject_unacquirable_lock(&run_dir);

    let started = Instant::now();
    expect_open_failure(
        &probe_args(&mod_dir, &runs_root, "r8a", false),
        &["重置产物失败"],
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "陈旧目录锁必须快速失败（真实 3s 超时路径另有人工用例）"
    );

    // 旧代证据逐字节不动：未新增、未归档、未覆盖、未混代。
    assert_eq!(event_line_count(&run_dir), 1, "旧事件不得被动过");
    assert!(reset_archives(&run_dir).is_empty(), "未取得锁就不得归档");
    assert_eq!(
        fs::read(&run_dir.join(AUTOSAVE_FILE)).expect("旧存档还在"),
        autosave_before,
        "旧存档不得被重写"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// R8b（r1 重写）：Attach 附着坏档 → fail-closed 在读档即失败，**不进入重置**
/// （旧「resume 失败 → 重置回退」路径已删除；锁不可得场景下更不得有任何
/// 归档/重写动作——旧事件未新增、无归档、坏档原样留作证据）。
#[test]
fn r8b_resume_failed_reset_without_lock_fails_closed() {
    let dir = case_dir("r8b");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_probe_mod(&mod_dir);
    new_game_with_one_step(&mod_dir, &runs_root, "r8b");
    let run_dir = runs_root.join("r8b");
    fs::write(run_dir.join(AUTOSAVE_FILE), "corrupted").expect("写坏档");
    inject_unacquirable_lock(&run_dir);

    expect_open_failure(
        &probe_args(&mod_dir, &runs_root, "r8b", true),
        &["读档失败", "open 只附着不重置"],
    );
    assert_eq!(event_line_count(&run_dir), 1, "旧事件不得被动过");
    assert!(reset_archives(&run_dir).is_empty(), "未取得锁就不得归档");
    let _ = fs::remove_dir_all(&dir);
}

/// R8c：重置中途的初始存档失败（autosave 位置是目录 → 原子替换 rename 必败）
/// → open 失败；已归档的旧代事件留下即证据，但**不得返回成功**，且新代
/// 绝不启动（无任何事件写入）。修复前此路径只告警并继续：全新内核把
/// seq 0 写进了新建的空 events（实测 RED）。
#[test]
fn r8c_mid_reset_save_failure_fails_closed_and_keeps_evidence() {
    let dir = case_dir("r8c");
    let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
    write_probe_mod(&mod_dir);
    new_game_with_one_step(&mod_dir, &runs_root, "r8c");
    let run_dir = runs_root.join("r8c");
    fs::remove_file(run_dir.join(AUTOSAVE_FILE)).expect("移除存档文件");
    fs::create_dir(run_dir.join(AUTOSAVE_FILE)).expect("造目录形态的 autosave");

    expect_open_failure(
        &probe_args(&mod_dir, &runs_root, "r8c", false),
        &["重置产物失败", "替换"],
    );

    // 归档已发生 → 证据留下；但新代绝不启动：events 只有空文件、零事件。
    let archives = reset_archives(&run_dir);
    assert_eq!(archives.len(), 1, "归档已发生的部分留下即证据");
    assert_eq!(archive_line_count(&archives[0]), 1, "旧代行全在归档里");
    assert_eq!(
        event_line_count(&run_dir),
        0,
        "重置未成功就绝不允许新代事件写入"
    );
    let _ = fs::remove_dir_all(&dir);
}
