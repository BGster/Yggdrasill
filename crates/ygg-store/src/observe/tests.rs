// ============================================================================
// §crate 内单测（两后端分列；零写入 / 零创建纯度锁）
// ============================================================================

use super::*;
use crate::{RunStore, StepDraft, Tick};
use std::fs;
use std::path::PathBuf;

const TICK: Tick = Tick {
    secs: 1_700_000_000,
    nanos: 11,
};

fn case_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ygg-store-observe-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("建临时 run 目录");
    dir
}

/// 提交 N 个成功步（每步带一份可辨识存档），返回最后一次的结果。
fn commit_steps(store: &mut dyn RunStore, n: u64, tag: &[u8]) -> Result<(), crate::StoreError> {
    for i in 0..n {
        let payload = format!("{{\"i\":{i}}}");
        let save = save_bytes(i, tag);
        store.commit_step(StepDraft {
            event: "step",
            payload: &payload,
            ok: true,
            narrative_count: 1,
            timestamp: TICK.secs + i,
            ending_line: None,
            save: Some(&save),
        })?;
    }
    Ok(())
}

fn save_bytes(i: u64, tag: &[u8]) -> Vec<u8> {
    format!(
        "{{\"step\":{i},\"tag\":\"{}\"}}",
        String::from_utf8_lossy(tag)
    )
    .into_bytes()
}

#[cfg(feature = "sqlite")]
fn sqlite_dir(tag: &str, steps: u64) -> (PathBuf, Box<dyn RunStore>) {
    let dir = case_dir(tag);
    let mut store: Box<dyn RunStore> =
        crate::open(Backend::Sqlite, &dir, TICK).expect("打开 sqlite 局");
    commit_steps(store.as_mut(), steps, b"sqlite").expect("提交步");
    (dir, store)
}

fn json_dir(tag: &str, steps: u64) -> (PathBuf, Box<dyn RunStore>) {
    let dir = case_dir(tag);
    let mut store: Box<dyn RunStore> =
        crate::open(Backend::Json, &dir, TICK).expect("打开 json 局");
    commit_steps(store.as_mut(), steps, b"json").expect("提交步");
    (dir, store)
}

// ---- sqlite 路径 ----

#[cfg(feature = "sqlite")]
#[test]
fn save_list_sqlite_orders_desc() {
    let (dir, _store) = sqlite_dir("desc", 3);
    let list = save_list(&dir, Backend::Sqlite, 0).expect("列表可读");
    assert_eq!(list.len(), 3, "每成功步一行存档");
    // 新→旧：id 降序（3、2、1），seq 逐行对齐，首行 latest。
    let ids: Vec<&str> = list.iter().map(|meta| meta.id.as_str()).collect();
    assert_eq!(ids, vec!["3", "2", "1"]);
    let seqs: Vec<u64> = list.iter().map(|meta| meta.seq).collect();
    assert_eq!(seqs, vec![2, 1, 0]);
    assert!(list[0].latest, "首行是当前代最新档");
    assert!(!list[1].latest && !list[2].latest);
    assert_eq!(list[0].generation, Some(0), "新局从代 0 起步");
    // bytes 与存档本体一致；created_at 来自提交时刻。
    assert_eq!(list[0].bytes, save_bytes(2, b"sqlite").len());
    assert_eq!(list[0].created_at, TICK.secs + 2);
}

#[cfg(feature = "sqlite")]
#[test]
fn save_list_limit_clamped() {
    let (dir, _store) = sqlite_dir("limit", 4);
    // limit=0 → 缺省 50（全量 4 行）。
    assert_eq!(save_list(&dir, Backend::Sqlite, 0).unwrap().len(), 4);
    // limit=2 → 只取最新 2 行，且仍是新→旧。
    let list = save_list(&dir, Backend::Sqlite, 2).unwrap();
    assert_eq!(
        list.iter().map(|meta| meta.id.as_str()).collect::<Vec<_>>(),
        ["4", "3"]
    );
    // 超上限 → 封顶（造 4 行够不着 200，直接断言常量本身被钳制面引用）。
    assert_eq!(clamp_save_limit(usize::MAX), SAVE_LIST_MAX_LIMIT);
    assert_eq!(clamp_save_limit(0), SAVE_LIST_DEFAULT_LIMIT);
}

#[cfg(feature = "sqlite")]
#[test]
fn save_get_by_id_and_latest() {
    let (dir, _store) = sqlite_dir("get", 3);
    // "latest" 与最大 id 字节等值。
    let (_, latest_bytes) = save_get(&dir, Backend::Sqlite, "latest")
        .expect("读 latest")
        .expect("latest 存在");
    let (_, by_id) = save_get(&dir, Backend::Sqlite, "3")
        .expect("读 id")
        .expect("id=3 存在");
    assert_eq!(latest_bytes, by_id);
    // 十进制 id 逐字节取回。
    let (meta, bytes) = save_get(&dir, Backend::Sqlite, "1").unwrap().unwrap();
    assert_eq!(bytes, save_bytes(0, b"sqlite"));
    assert_eq!(meta.seq, 0);
    assert!(!meta.latest, "非最新档 latest=false");
    // 缺 id → None；非数字非 latest → None（语法门在调用方）。
    assert!(save_get(&dir, Backend::Sqlite, "999999").unwrap().is_none());
    assert!(save_get(&dir, Backend::Sqlite, "abc").unwrap().is_none());
}

#[cfg(feature = "sqlite")]
#[test]
fn observe_creates_nothing() {
    let dir = case_dir("creates");
    fs::write(dir.join("run.json"), b"{\"run_id\":\"bare\"}").expect("只写 run.json");
    // 全部观察入口打在一个「只有 run.json」的目录上。
    assert_eq!(
        save_probe(&dir, Backend::Sqlite).unwrap(),
        SaveProbe::default()
    );
    assert!(save_list(&dir, Backend::Sqlite, 0).unwrap().is_empty());
    assert!(save_get(&dir, Backend::Sqlite, "latest").unwrap().is_none());
    let page = event_page(&dir, Backend::Sqlite, 0, 10).unwrap();
    assert!(page.records.is_empty() && page.latest == 0);
    assert!(artifact_list_view(&dir, Backend::Sqlite)
        .unwrap()
        .is_empty());
    // 纯度：目录里**没有多出任何文件**（run.sqlite / -wal / -shm 都不许有）。
    let entries: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        entries,
        vec!["run.json"],
        "观察面不得创建任何文件：{entries:?}"
    );
}

#[cfg(feature = "sqlite")]
#[test]
fn observe_writes_nothing() {
    let (dir, _store) = sqlite_dir("writes", 2);
    // 断言范围 = run 目录的**逻辑产物**。-wal / -shm 明确排除：observe 连接
    // 可能创建它们（连接级产物，宿主 cleanup_stale 语义已定义为活文件类），
    // 主库字节在查询下不变（query_only + 只读 SQL）。
    fn digest(dir: &Path) -> Vec<(String, u64)> {
        let mut out: Vec<(String, u64)> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                let len = fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
                (name, len)
            })
            .filter(|(name, _)| !name.ends_with("-wal") && !name.ends_with("-shm"))
            .collect();
        out.sort();
        out
    }
    let before = digest(&dir);
    save_probe(&dir, Backend::Sqlite).unwrap();
    save_list(&dir, Backend::Sqlite, 0).unwrap();
    save_get(&dir, Backend::Sqlite, "latest").unwrap();
    event_page(&dir, Backend::Sqlite, 0, 10).unwrap();
    artifact_list_view(&dir, Backend::Sqlite).unwrap();
    artifact_get_view(&dir, Backend::Sqlite, "artifacts/a.md").unwrap();
    assert_eq!(digest(&dir), before, "观察面不得改动任何既有产物");
}

#[cfg(feature = "sqlite")]
#[test]
fn event_page_since_limit() {
    let (dir, _store) = sqlite_dir("page", 5);
    let page = event_page(&dir, Backend::Sqlite, 2, 2).unwrap();
    assert_eq!(page.latest, 5, "latest = 已提交步数（下一 seq）");
    let seqs: Vec<u64> = page.records.iter().map(|record| record.seq).collect();
    assert_eq!(seqs, vec![2, 3], "自 since 起 seq 升序、limit 截断");
    // payload 原文透传。
    assert_eq!(page.records[0].payload, "{\"i\":2}");
    // limit 封顶常量面。
    assert_eq!(EVENT_PAGE_MAX_LIMIT, 500);
}

#[cfg(feature = "sqlite")]
#[test]
fn artifact_views_roundtrip() {
    let (dir, mut store) = sqlite_dir("artifact", 1);
    let meta = store
        .artifact_put("artifacts/note.md", "正文".as_bytes(), TICK)
        .expect("写制品");
    let view = artifact_list_view(&dir, Backend::Sqlite).unwrap();
    assert_eq!(view, vec![meta.clone()]);
    let record = artifact_get_view(&dir, Backend::Sqlite, "artifacts/note.md")
        .unwrap()
        .expect("制品存在");
    assert_eq!(record.meta, meta);
    assert_eq!(record.content, "正文".as_bytes());
    // 与 RunStore::artifact_list / artifact_get 语义等价。
    assert_eq!(store.artifact_list().unwrap(), view);
    assert_eq!(
        store.artifact_get("artifacts/note.md").unwrap(),
        Some(record)
    );
}

// ---- JSON 路径 ----

#[test]
fn save_list_json_single_row() {
    let (dir, mut store) = json_dir("json-single", 2);
    let list = save_list(&dir, Backend::Json, 0).expect("列表可读");
    assert_eq!(list.len(), 1, "JSON 局恒单行（单档后端的物理事实）");
    assert_eq!(list[0].id, "latest");
    assert_eq!(list[0].generation, None);
    assert_eq!(list[0].seq, 2, "seq = events 非空行数");
    assert!(list[0].latest);
    assert_eq!(
        list[0].bytes,
        fs::metadata(dir.join(AUTOSAVE_FILE)).unwrap().len() as usize
    );
    // reset 后 seq 归零重来：单档口径跟着当前代走。
    store.reset(b"{}", "observe-test", TICK).expect("重置");
    let after = save_list(&dir, Backend::Json, 0).unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].seq, 0, "重置后 events 为空文件 → seq 0");
    // 数字 id 在 JSON 局不存在 → None（不伪造历史）。
    assert!(save_get(&dir, Backend::Json, "1").unwrap().is_none());
    let (_, bytes) = save_get(&dir, Backend::Json, "latest").unwrap().unwrap();
    assert_eq!(bytes, store.latest_save().unwrap().unwrap());
}

#[test]
fn event_page_since_limit_json() {
    let (dir, _store) = json_dir("json-page", 4);
    let page = event_page(&dir, Backend::Json, 1, 2).unwrap();
    assert_eq!(page.latest, 4);
    let seqs: Vec<u64> = page.records.iter().map(|record| record.seq).collect();
    assert_eq!(seqs, vec![1, 2]);
    assert_eq!(page.records[0].payload, "{\"i\":1}");
    // 与 sqlite 分页语义等价（records 形状一致，仅 generation/created_at 来源不同）。
    assert_eq!(page.records[0].event, "step");
}

#[test]
fn json_probe_reports_latest_and_mtime() {
    let (dir, _store) = json_dir("json-probe", 1);
    let probe = save_probe(&dir, Backend::Json).unwrap();
    assert_eq!(probe.count, 1);
    assert_eq!(probe.latest_id.as_deref(), Some("latest"));
    assert_eq!(probe.generation, None);
    assert!(probe.latest_created_at > 0, "mtime 可读");
    // 无 autosave → 空探针。
    let empty = case_dir("json-probe-empty");
    assert_eq!(
        save_probe(&empty, Backend::Json).unwrap(),
        SaveProbe::default()
    );
    assert!(save_list(&empty, Backend::Json, 0).unwrap().is_empty());
}

#[test]
fn artifact_views_roundtrip_json() {
    let (dir, mut store) = json_dir("json-artifact", 1);
    let meta = store
        .artifact_put("artifacts/note.md", "正文".as_bytes(), TICK)
        .expect("写制品");
    // JSON 读侧的 updated_at 回 mtime（无 sidecar，写入 tick 不可复得）——
    // ArtifactMeta::updated_at 的既有契约「跨面不强求相等」，故按字段比对。
    let view = artifact_list_view(&dir, Backend::Json).unwrap();
    assert_eq!(view.len(), 1);
    assert_eq!(&view[0].path, &meta.path);
    assert_eq!(view[0].bytes, meta.bytes);
    assert_eq!(view[0].sha256, meta.sha256);
    assert!(view[0].updated_at > 0, "mtime 可读");
    let record = artifact_get_view(&dir, Backend::Json, "artifacts/note.md")
        .unwrap()
        .unwrap();
    assert_eq!(record.content, "正文".as_bytes());
    assert_eq!(record.meta.path, meta.path);
    assert_eq!(record.meta.sha256, meta.sha256);
}

/// `--no-default-features` 构建（无 sqlite）下，观察面必须 fail-closed 且零写入。
#[cfg(not(feature = "sqlite"))]
#[test]
fn sqlite_views_unavailable_without_feature() {
    let dir = case_dir("unavailable");
    let err = save_probe(&dir, Backend::Sqlite).unwrap_err();
    assert!(
        matches!(err, crate::StoreError::Unavailable { .. }),
        "{err}"
    );
}

/// `commit_steps` 的输入泄漏手法（`Box::leak`）只为测试内静态化 payload/save；
/// 这里断言它没有把行为搞错：步数与 seq 对齐。
#[test]
fn commit_helper_counts_match() {
    let (dir, store) = json_dir("helper", 3);
    assert_eq!(store.next_seq().unwrap(), 3);
    assert_eq!(event_page(&dir, Backend::Json, 0, 10).unwrap().latest, 3);
}
