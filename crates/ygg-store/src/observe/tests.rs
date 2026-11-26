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

fn json_dir(tag: &str, steps: u64) -> (PathBuf, Box<dyn RunStore>) {
    let dir = case_dir(tag);
    let mut store: Box<dyn RunStore> =
        crate::open(Backend::Json, &dir, TICK).expect("打开 json 局");
    commit_steps(store.as_mut(), steps, b"json").expect("提交步");
    (dir, store)
}

// ---- JSON 存档路径 ----

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
    // records 形状一致；generation/created_at 的来源是 JSON 后端自己的。
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

/// `commit_steps` 的输入泄漏手法（`Box::leak`）只为测试内静态化 payload/save；
/// 这里断言它没有把行为搞错：步数与 seq 对齐。
#[test]
fn commit_helper_counts_match() {
    let (dir, store) = json_dir("helper", 3);
    assert_eq!(store.next_seq().unwrap(), 3);
    assert_eq!(event_page(&dir, Backend::Json, 0, 10).unwrap().latest, 3);
}
