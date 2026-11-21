use super::{acquire_run_lock, atomic_write, Cell};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// 用例专属临时目录（互不共享，锁文件互不干扰）。
fn case_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ygg-p0-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// U1：重入不二次 acquire —— 外层取锁后内层再取必须立即返回、不加 depth、
/// 不触碰锁文件；只有真实持有者的 Drop 才归零 depth 并删锁文件。
#[test]
fn u1_reentrant_lock_does_not_reacquire() {
    let dir = case_dir("u1");
    let lock_path = dir.join(super::COORDINATION_LOCK);

    let outer = acquire_run_lock(&dir).expect("首次 acquire 应成功");
    assert!(lock_path.exists(), "真实 acquire 必须留下锁文件");
    assert_eq!(crate::RUN_LOCK_DEPTH.with(Cell::get), 1);
    std::fs::write(&lock_path, b"holder-marker").expect("写锁文件探针");

    let started = Instant::now();
    let inner = acquire_run_lock(&dir).expect("重入必须直接成功，不得阻塞或报超时");
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "重入必须立即返回（真实的第二次 acquire 会阻塞到陈旧抢占/超时）"
    );
    assert_eq!(crate::RUN_LOCK_DEPTH.with(Cell::get), 1, "重入不得加 depth");
    assert_eq!(
        std::fs::read(&lock_path).expect("锁文件应还在"),
        b"holder-marker",
        "重入不得触碰锁文件（= 没有第二次 acquire）"
    );

    drop(inner);
    assert_eq!(
        crate::RUN_LOCK_DEPTH.with(Cell::get),
        1,
        "重入占位的 Drop 不得归零 depth"
    );
    assert!(lock_path.exists(), "重入占位的 Drop 不得删锁文件");
    drop(outer);
    assert_eq!(
        crate::RUN_LOCK_DEPTH.with(Cell::get),
        0,
        "真实持有者的 Drop 必须归零 depth"
    );
    assert!(!lock_path.exists(), "真实持有者的 Drop 必须删锁文件");

    // 归零之后必须能立刻重新取锁（释放是真实的，不是记账残留）。
    let again = acquire_run_lock(&dir).expect("释放后必须能重新取锁");
    assert_eq!(crate::RUN_LOCK_DEPTH.with(Cell::get), 1);
    drop(again);

    let _ = std::fs::remove_dir_all(&dir);
}

/// U3：atomic_write —— 写后原文件内容完整；tmp 已不存在；目标目录不存在
/// 时返回 Err（不 panic）。
#[test]
fn u3_atomic_write_semantics() {
    let dir = case_dir("u3");

    // ① 正常写：内容完整 + 无 tmp 残留。
    let target = dir.join("autosave.json");
    atomic_write(&target, br#"{"kv":{"revision":7}}"#).expect("首次写应成功");
    assert_eq!(
        std::fs::read_to_string(&target).expect("读回"),
        r#"{"kv":{"revision":7}}"#
    );
    let leftovers: Vec<String> = std::fs::read_dir(&dir)
        .expect("读目录")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp."))
        .collect();
    assert!(leftovers.is_empty(), "写后不得残留 tmp：{leftovers:?}");

    // ② 覆盖写：旧全文被新全文原子替换。
    atomic_write(&target, b"second").expect("覆盖写应成功");
    assert_eq!(std::fs::read_to_string(&target).expect("读回"), "second");

    // ③ 目标目录不存在：返回 Err，不 panic，也不落任何文件。
    let missing = dir.join("no-such-dir").join("autosave.json");
    let outcome = atomic_write(&missing, b"x");
    assert!(outcome.is_err(), "目标目录不存在必须报错");
    assert!(!dir.join("no-such-dir").exists(), "不得凭空建目录");

    let _ = std::fs::remove_dir_all(&dir);
}

/// U4：seq 分配口径的三种边界 —— 末行无换行 / 空文件 / 只有空行。
///
/// 函数本体已下沉到 `ygg-store`（`JsonFileStore::next_seq`），断言不变：
/// seq 的唯一事实源是**非空行数**，末行没有换行也要计入。
#[test]
fn u4_event_count_edge_cases() {
    use ygg_store::{JsonFileStore, RunStore};

    let dir = case_dir("u4");
    let path = dir.join(ygg_store::EVENTS_FILE);
    let store = JsonFileStore::open(&dir).expect("绑到用例目录");

    // 空文件。
    std::fs::write(&path, "").expect("写空文件");
    assert_eq!(store.next_seq().expect("seq"), 0, "空文件 = 0 条");

    // 只有空行 / 空白行。
    std::fs::write(&path, "\n\n   \n\t\n").expect("写空白行");
    assert_eq!(store.next_seq().expect("seq"), 0, "空行与空白行不计入");

    // 末行无换行：仍按非空行计数。
    std::fs::write(&path, "{\"seq\":0}\n{\"seq\":1}").expect("写无尾换行");
    assert_eq!(store.next_seq().expect("seq"), 2, "末行无换行也要计入");

    // 正常形态（对照）。
    std::fs::write(&path, "{\"seq\":0}\n{\"seq\":1}\n").expect("写正常形态");
    assert_eq!(store.next_seq().expect("seq"), 2);

    // 文件不存在（开局前）。
    std::fs::remove_file(&path).expect("删事件日志");
    assert_eq!(store.next_seq().expect("seq"), 0, "读不到文件 = 0 条");

    let _ = std::fs::remove_dir_all(&dir);
}

/// U2：临界区内 panic 展开后锁已释放 —— guard 的 Drop 在 unwind 路径上
/// 归零 depth 并删锁文件，随后可以正常重新取锁。
#[test]
fn u2_panic_unwind_releases_the_lock() {
    let dir = case_dir("u2");
    let lock_path = dir.join(super::COORDINATION_LOCK);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = acquire_run_lock(&dir).expect("acquire 应成功");
        assert_eq!(crate::RUN_LOCK_DEPTH.with(Cell::get), 1);
        panic!("临界区内爆炸");
    }));
    assert!(result.is_err(), "panic 必须向上传播（此处只验锁的释放）");
    assert_eq!(
        crate::RUN_LOCK_DEPTH.with(Cell::get),
        0,
        "unwind 后 Drop 必须已归零 depth"
    );
    assert!(!lock_path.exists(), "unwind 后 Drop 必须已删锁文件");

    // panic 之后必须能正常重新取锁（无死锁、无记账残留）。
    let again = acquire_run_lock(&dir).expect("panic 后重新取锁必须成功");
    assert_eq!(crate::RUN_LOCK_DEPTH.with(Cell::get), 1);
    drop(again);
    assert_eq!(crate::RUN_LOCK_DEPTH.with(Cell::get), 0);

    let _ = std::fs::remove_dir_all(&dir);
}
