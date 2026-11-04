//! 跨后端一致性：同一个场景跑两个后端，断言**语义**等价。
//!
//! 这里刻意不做字节比较——sqlite 不产 `autosave.json` / `events.jsonl`，字节面
//! 只对 `JsonFileStore` 有定义（见 `golden_bytes.rs`）。本文件断言的是：
//! 同样的提交序列在两边得到同样的 `StepRecord`、同样的 seq 规则、同样的
//! 「失败步不落存档」、同样的「reset 归档但不删证据」。
//!
//! 崩溃承诺**不**写成一条两边都成立的弱断言：JSON 是两步写（允许「事件有、
//! 存档旧」），sqlite 是单事务（全或无）。两条分列：
//! - JSON 侧：`json_partial_commit_leaves_event_without_save`；
//! - sqlite 侧：`src/sqlite.rs` 的 crate 内单测（需要直接持有连接注入故障）。

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use serde_json::json;
use ygg_store::{open, Backend, JsonFileStore, RunStore, StepDraft, StepRecord, Tick, EVENTS_FILE};

const TICK: Tick = Tick {
    secs: 1_700_000_000,
    nanos: 1,
};

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ygg-store-consistency-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("建临时 run 目录");
    dir
}

fn draft<'a>(event: &'a str, payload: &'a str, ok: bool, save: Option<&'a [u8]>) -> StepDraft<'a> {
    StepDraft {
        event,
        payload,
        ok,
        narrative_count: usize::from(ok),
        timestamp: TICK.secs,
        ending_line: None,
        save,
    }
}

/// 共用场景：3 个成功步 + 1 个失败步，返回该代全部记录。
///
/// 覆盖：seq 由后端分配且连续、失败步**有行无档**、`next_seq` 与非空行数一致、
/// 存档永远取「最后一个成功步」。
fn happy_and_failed_steps(store: &mut dyn RunStore) -> Vec<StepRecord> {
    for index in 0..3u64 {
        let save = format!("{{\"revision\":{index}}}");
        let committed = store
            .commit_step(draft(
                "step.ok",
                &index.to_string(),
                true,
                Some(save.as_bytes()),
            ))
            .expect("成功步必须提交成功");
        assert_eq!(committed.seq, index, "seq 必须连续分配");
        assert!(
            committed.warnings.is_empty(),
            "正常行不该有告警：{:?}",
            committed.warnings
        );
        assert_eq!(
            store.latest_save().expect("读存档"),
            Some(save.into_bytes()),
            "成功步必须落存档"
        );
    }

    let before = store.latest_save().expect("读存档");
    let failed = store
        .commit_step(draft("step.failed", "x", false, None))
        .expect("失败步的事件行仍要写");
    assert_eq!(failed.seq, 3, "失败步同样占 seq");
    assert_eq!(
        store.latest_save().expect("读存档"),
        before,
        "失败步不得改存档（有行无档）"
    );
    assert_eq!(store.next_seq().expect("seq"), 4);

    let log = store.steps().expect("读记录");
    assert_eq!(log.skipped, 0);
    assert_eq!(log.records.len(), 4);
    assert_eq!(
        log.records.iter().map(|r| r.seq).collect::<Vec<_>>(),
        vec![0, 1, 2, 3],
        "seq 必须无洞"
    );
    assert!(!log.records[3].ok, "第 4 条必须是失败步");
    log.records
}

#[test]
fn backends_agree_on_step_records() {
    let json_dir = temp_dir("agree-json");
    let mut json_store = JsonFileStore::open(&json_dir).expect("打开 JSON 后端");
    let from_json = happy_and_failed_steps(&mut json_store);

    #[cfg(feature = "sqlite")]
    {
        let sqlite_dir = temp_dir("agree-sqlite");
        let mut sqlite_store =
            open(Backend::Sqlite, &sqlite_dir, TICK).expect("打开 sqlite 后端（feature 已开）");
        let from_sqlite = happy_and_failed_steps(sqlite_store.as_mut());
        assert_eq!(
            from_json, from_sqlite,
            "两后端的 StepRecord 必须逐字段相等（语义等价）"
        );
    }
    #[cfg(not(feature = "sqlite"))]
    {
        // 未开 feature 时至少把 JSON 侧的形状钉住（sqlite 侧由 CI 的 sqlite job 覆盖）。
        assert_eq!(from_json.len(), 4);
    }
}

#[test]
fn seqs_stay_gapless_over_many_steps() {
    let dir = temp_dir("gapless-json");
    let mut store = JsonFileStore::open(&dir).expect("打开 JSON 后端");
    for index in 0..100u64 {
        let save = format!("{{\"n\":{index}}}");
        let committed = store
            .commit_step(draft("step", "0", true, Some(save.as_bytes())))
            .expect("提交");
        assert_eq!(committed.seq, index);
    }
    assert_eq!(store.next_seq().expect("seq"), 100);
    let log = store.steps().expect("读记录");
    assert_eq!(
        log.records.iter().map(|r| r.seq).collect::<Vec<_>>(),
        (0..100).collect::<Vec<_>>()
    );
    assert_eq!(
        store.latest_save().expect("读存档").expect("有存档"),
        br#"{"n":99}"#.to_vec()
    );
}

/// 不可解析的行也要占 seq（既有口径：seq = 非空行数，不是可解析记录数）。
#[test]
fn unparsable_lines_still_consume_seq() {
    let dir = temp_dir("skipped");
    let mut store = JsonFileStore::open(&dir).expect("打开 JSON 后端");
    store
        .commit_step(draft("step", "0", true, Some(b"{\"rev\":0}")))
        .expect("提交");
    // 外部写入一条坏行（模拟历史/人工损坏），它同样占一个空行号。
    let mut text = fs::read_to_string(dir.join(EVENTS_FILE)).expect("读 events");
    text.push_str("not-json\n");
    fs::write(dir.join(EVENTS_FILE), text).expect("写回 events");

    let log = store.steps().expect("读记录");
    assert_eq!(log.records.len(), 1);
    assert_eq!(log.skipped, 1, "坏行计入 skipped");
    assert_eq!(store.next_seq().expect("seq"), 2, "坏行照样占 seq");
}

#[test]
fn reset_archives_but_never_deletes_evidence() {
    let dir = temp_dir("reset-json");
    let mut store = JsonFileStore::open(&dir).expect("打开 JSON 后端");
    store
        .commit_step(draft("step", "0", true, Some(b"{\"rev\":0}")))
        .expect("提交");
    let old_events = fs::read_to_string(dir.join(EVENTS_FILE)).expect("读 events");

    let info = store
        .reset(b"{\"initial\":true}", "explicit-new-game", TICK)
        .expect("重置必须成功");
    assert!(info.had_old, "目录里本来有产物");
    let archive = PathBuf::from(info.archive.expect("JSON 后端必须给归档路径"));
    assert!(
        archive
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("events.jsonl.reset.")),
        "归档文件名必须带纳秒后缀：{}",
        archive.display()
    );
    assert_eq!(
        fs::read_to_string(&archive).expect("读归档"),
        old_events,
        "旧代行必须全部留在归档里"
    );

    // 新代从零起步，存档是初始档。
    assert_eq!(store.next_seq().expect("seq"), 0);
    let log = store.steps().expect("读记录");
    assert!(log.records.is_empty(), "新代事件日志必须为空");
    assert_eq!(
        store.latest_save().expect("读存档").expect("有存档"),
        b"{\"initial\":true}".to_vec()
    );

    // 第二次重置：归档载体再增一份（证据不删，只堆积）。
    // 纳秒后缀由**宿主时钟**给（这里显式给不同值——同值会让 rename 覆写前一份，
    // 那是宿主时钟的职责边界，不是后端的缺陷）。
    store
        .reset(
            b"{\"initial\":false}",
            "resume-failed",
            Tick::at(TICK.secs, 2),
        )
        .expect("第二次重置必须成功");
    let archives = fs::read_dir(&dir)
        .expect("读目录")
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("events.jsonl.reset.")
        })
        .count();
    assert_eq!(archives, 2, "两次重置各归档一份，旧的不得被删");
}

#[test]
fn reset_switches_generation_on_sqlite() {
    #[cfg(feature = "sqlite")]
    {
        let dir = temp_dir("reset-sqlite");
        let mut store = open(Backend::Sqlite, &dir, TICK).expect("打开 sqlite 后端");
        store
            .commit_step(draft("step", "0", true, Some(b"{\"rev\":0}")))
            .expect("提交");

        let info = store
            .reset(b"{\"initial\":true}", "explicit-new-game", TICK)
            .expect("重置必须成功");
        assert!(info.had_old);
        assert_eq!(
            info.archive.as_deref(),
            Some("generation=1"),
            "归档载体是代"
        );
        assert_eq!(store.next_seq().expect("seq"), 0);
        assert!(store.steps().expect("读记录").records.is_empty());
        assert_eq!(
            store.latest_save().expect("读存档").expect("有存档"),
            b"{\"initial\":true}".to_vec()
        );
        // 空 run 目录第一次 reset：had_old 必须是 false（不能凭空报旧产物）。
        let fresh = temp_dir("reset-sqlite-fresh");
        let mut fresh_store = open(Backend::Sqlite, &fresh, TICK).expect("打开 sqlite 后端");
        let info = fresh_store
            .reset(b"{}", "explicit-new-game", TICK)
            .expect("重置必须成功");
        assert!(!info.had_old, "空库不得报旧产物（会误导回执文案）");
        assert!(
            info.archive.is_none(),
            "空库重置不归档（不凭空多出一代，也不会在回执里报「已归档」）"
        );
        assert_eq!(
            fresh_store.next_seq().expect("seq"),
            0,
            "空库重置后仍是 0 步"
        );
    }
    #[cfg(not(feature = "sqlite"))]
    {
        let dir = temp_dir("reset-sqlite-off");
        let mut store = JsonFileStore::open(&dir).expect("打开 JSON 后端");
        store
            .reset(b"{}", "explicit-new-game", TICK)
            .expect("重置必须成功");
    }
}

/// inbox 面：行数 / 位移 / 重置后证据保留，且位移不会凭空越过已有行数。
#[test]
fn inbox_cursor_tracks_lines_and_survives_reset() {
    let dir = temp_dir("inbox-json");
    let mut store = JsonFileStore::open(&dir).expect("打开 JSON 后端");
    assert_eq!(store.inbox_len().expect("行数"), 0);
    assert_eq!(store.inbox_cursor().expect("位移"), 0);

    for index in 0..3 {
        store
            .inbox_push(
                &json!({"kind": "add", "title": format!("t{index}"), "external_id": format!("ext-{index}")}),
                TICK,
            )
            .expect("追加 inbox 行");
    }
    assert_eq!(store.inbox_len().expect("行数"), 3);
    let lines = store.inbox_lines().expect("读行");
    assert_eq!(lines.len(), 3);
    assert!(
        lines[0].contains("\"external_id\":\"ext-0\""),
        "行原文必须逐字节保真（紧凑 JSON）: {}",
        lines[0]
    );

    store.set_inbox_cursor(1, TICK).expect("前移位移");
    assert_eq!(store.inbox_cursor().expect("位移"), 1);
    assert!(store.inbox_cursor().expect("位移") <= store.inbox_len().expect("行数"));

    // 提一步不为失败步：位移不因提交而变（推进由宿主在 dispatch 成功后显式做）。
    store
        .commit_step(draft("step", "0", true, Some(b"{\"rev\":0}")))
        .expect("提交");
    assert_eq!(
        store.inbox_cursor().expect("位移"),
        1,
        "提交不得自行前移位移"
    );

    // 重置不清 inbox（证据与位移都留着）。
    store.reset(b"{}", "explicit-new-game", TICK).expect("重置");
    assert_eq!(store.inbox_len().expect("行数"), 3, "重置不得删 inbox 证据");
    assert_eq!(store.inbox_cursor().expect("位移"), 1);

    #[cfg(feature = "sqlite")]
    {
        let sqlite_dir = temp_dir("inbox-sqlite");
        let mut sqlite_store = open(Backend::Sqlite, &sqlite_dir, TICK).expect("打开 sqlite 后端");
        for index in 0..3 {
            sqlite_store
                .inbox_push(
                    &json!({"kind": "add", "external_id": format!("ext-{index}")}),
                    TICK,
                )
                .expect("追加 inbox 行");
        }
        assert_eq!(sqlite_store.inbox_len().expect("行数"), 3);
        assert_eq!(sqlite_store.inbox_cursor().expect("位移"), 0);
        sqlite_store.set_inbox_cursor(2, TICK).expect("前移位移");
        assert_eq!(sqlite_store.inbox_cursor().expect("位移"), 2);
        assert_eq!(
            sqlite_store.inbox_lines().expect("读行").len(),
            3,
            "位移前移不得删行"
        );
    }
}

/// JSON 后端的**弱承诺**：事件行先落、存档后写，两步之间可以被打破。
///
/// 注入手法：把 `autosave.json` 换成目录 → 原子替换的 `rename` 必败。此时事件
/// 行已经在盘上（seq 前移），存档仍是上一版——这正是 JSON 后端允许的状态，
/// 也是 sqlite 后端（单事务）**不允许**的状态（见 `src/sqlite.rs` 的 crate 内单测）。
#[test]
fn json_partial_commit_leaves_event_without_save() {
    let dir = temp_dir("json-partial");
    let mut store = JsonFileStore::open(&dir).expect("打开 JSON 后端");
    store
        .commit_step(draft("step", "0", true, Some(b"{\"revision\":0}")))
        .expect("第一个成功步");
    let good_save = store.latest_save().expect("读存档").expect("有存档");

    // 造故障：存档位置变成目录（rename 目标不可替换）。
    fs::remove_file(dir.join("autosave.json")).expect("移除存档");
    fs::create_dir(dir.join("autosave.json")).expect("造目录形态的存档");

    let err = store
        .commit_step(draft("step", "1", true, Some(b"{\"revision\":1}")))
        .expect_err("存档写不出去必须硬失败");
    assert!(
        err.to_string().contains("替换"),
        "错误串必须可读（原子替换失败）：{err}"
    );

    // 弱承诺：事件行已落（多一行），存档读不出来——但**绝不**出现「存档新于事件」。
    let log = store.steps().expect("读记录");
    assert_eq!(log.records.len(), 2, "事件行先落是现状语义");
    assert!(
        store.latest_save().is_err(),
        "存档位置被占成目录，读存档必须报错（不是静默给旧值）"
    );

    // 恢复现场：把好档写回，可继续推进（seq 接着事件行数走）。
    fs::remove_dir(dir.join("autosave.json")).expect("移除目录");
    store.set_save(&good_save, TICK).expect("写回存档");
    let committed = store
        .commit_step(draft("step", "2", true, Some(b"{\"revision\":2}")))
        .expect("现场恢复后必须能继续提交");
    assert_eq!(committed.seq, 2, "seq 以事件行数为准，不因存档失败回退");
}

/// 两后端都能被 `open()` 统一入口打开，且 `backend()` / `locator()` 如实自报。
#[test]
fn open_reports_its_own_backend() {
    let dir = temp_dir("open-json");
    let store = open(Backend::Json, &dir, TICK).expect("打开 JSON 后端");
    assert_eq!(store.backend(), Backend::Json);
    assert!(
        store.locator().ends_with("autosave.json"),
        "JSON 的定位串是存档路径：{}",
        store.locator()
    );

    #[cfg(feature = "sqlite")]
    {
        let sqlite_dir = temp_dir("open-sqlite");
        let store = open(Backend::Sqlite, &sqlite_dir, TICK).expect("打开 sqlite 后端");
        assert_eq!(store.backend(), Backend::Sqlite);
        assert!(
            store.locator().ends_with("run.sqlite#latest"),
            "sqlite 的定位串是 <db>#latest：{}",
            store.locator()
        );
        assert!(
            sqlite_dir.join("run.sqlite").exists(),
            "sqlite 后端必须在 run 目录里建库"
        );
    }
}

/// `cleanup_stale` 的差别：JSON 清陈旧 tmp，sqlite 恒 no-op（绝不碰 -wal/-shm）。
#[test]
fn cleanup_stale_is_json_only() {
    let dir = temp_dir("cleanup");
    let mut store = JsonFileStore::open(&dir).expect("打开 JSON 后端");
    let stale = dir.join("autosave.json.tmp.999");
    fs::write(&stale, "half").expect("造残留 tmp");
    // 陈旧判定用 mtime 窗口：刚写的文件不该被删（宁可残留不可误删）。
    assert!(
        store.cleanup_stale(10_000).is_empty(),
        "新鲜的 tmp 不得被清理"
    );
    assert!(stale.exists());
    // 回拨 mtime 造「崩溃残留」（不 sleep：确定性强、不拖慢门禁）。
    fs::OpenOptions::new()
        .write(true)
        .open(&stale)
        .expect("打开 tmp")
        .set_modified(SystemTime::now() - Duration::from_secs(60))
        .expect("回拨 mtime");
    let warnings = store.cleanup_stale(10_000);
    assert_eq!(warnings.len(), 1, "陈旧 tmp 必须被清理并告警");
    assert!(warnings[0].contains("清理残留 tmp"));
    assert!(!stale.exists());

    #[cfg(feature = "sqlite")]
    {
        let sqlite_dir = temp_dir("cleanup-sqlite");
        let mut sqlite_store = open(Backend::Sqlite, &sqlite_dir, TICK).expect("打开 sqlite 后端");
        sqlite_store
            .commit_step(draft("step", "0", true, Some(b"{}")))
            .expect("提交");
        assert!(
            sqlite_store.cleanup_stale(0).is_empty(),
            "sqlite 的清理必须是 no-op"
        );
        assert!(
            sqlite_dir.join("run.sqlite").exists(),
            "no-op 之后库文件必须还在（清理不是删库）"
        );
    }
}

/// `run.json` 的 `store` 字段解析（端口只给枚举，不碰 run.json 本身）。
#[test]
fn backend_names_round_trip() {
    for name in Backend::NAMES {
        assert_eq!(Backend::parse(name).map(Backend::as_str), Some(name));
    }
    assert!(Backend::parse("sqlite3").is_none(), "只认精确取值");
    assert!(Backend::parse("JSON").is_none(), "大小写不宽容");
}
