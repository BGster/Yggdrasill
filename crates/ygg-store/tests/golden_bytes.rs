//! 字节黄金样本：`JsonFileStore` 的产物必须与**历史版本宿主**写出的字节完全一致。
//!
//! fixture 来历见 `tests/fixtures/legacy-run/README.md`（合成探针 Mod、两步、非真实局）。
//!
//! 为什么逐字节而不是「JSON 语义相等」：`serde_json::Value` 比较会把**键序**差异
//! 抹平（`Map` 是 BTreeMap，反序列化后两侧都是字典序）——一旦有人打开
//! `serde_json/preserve_order`，语义比较仍然全绿，而历史产物与外部工具的 diff
//! 全红。所以这里断言的是字节。

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value as Json;
use ygg_store::{JsonFileStore, RunStore, StepDraft, AUTOSAVE_FILE, EVENTS_FILE};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/legacy-run")
        .join(name)
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ygg-store-golden-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("建临时 run 目录");
    dir
}

fn fixture_lines() -> Vec<Json> {
    let text = fs::read_to_string(fixture(EVENTS_FILE)).expect("读 fixture events.jsonl");
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Json>(line).expect("fixture 行必须是合法 JSON"))
        .collect()
}

/// 回放 fixture 的两步：产物必须**逐字节**相同（events.jsonl + autosave.json）。
///
/// 中间存档字节无法从 fixture 还原（fixture 只冻结最终存档），故第 0 步塞一份
/// 任意字节——最终存档由第 1 步覆盖，正是要断言的那一份。
#[test]
fn replaying_fixture_reproduces_bytes_exactly() {
    let events_text = fs::read_to_string(fixture(EVENTS_FILE)).expect("读 events");
    let autosave = fs::read(fixture(AUTOSAVE_FILE)).expect("读 autosave");
    let records = fixture_lines();
    assert_eq!(records.len(), 2, "fixture 是两步探针");

    let dir = temp_dir("replay");
    let mut store = JsonFileStore::open(&dir).expect("打开 JSON 后端");
    let last = records.len() - 1;
    for (index, value) in records.iter().enumerate() {
        let save: &[u8] = if index == last {
            &autosave
        } else {
            br#"{"engine_version":"0.1.0","format_version":1}"#
        };
        let committed = store
            .commit_step(StepDraft {
                event: value["event"].as_str().expect("event"),
                payload: value["payload"].as_str().expect("payload"),
                ok: value["ok"].as_bool().expect("ok"),
                narrative_count: value["narrative_count"].as_u64().expect("narrative_count")
                    as usize,
                timestamp: value["timestamp"].as_u64().expect("timestamp"),
                ending_line: value["ending_line"].as_str(),
                save: Some(save),
            })
            .expect("提交必须成功");
        assert_eq!(
            committed.seq,
            value["seq"].as_u64().expect("seq"),
            "seq 必须由后端按非空行数分配"
        );
        assert!(
            committed.warnings.is_empty(),
            "两步都是正常行，不该有告警：{:?}",
            committed.warnings
        );
    }

    assert_eq!(
        fs::read(dir.join(EVENTS_FILE)).expect("读产物 events"),
        events_text.as_bytes(),
        "events.jsonl 必须逐字节一致（键序 / 分隔符 / 尾随 \\n）"
    );
    assert_eq!(
        fs::read(dir.join(AUTOSAVE_FILE)).expect("读产物 autosave"),
        autosave,
        "autosave.json 必须逐字节一致（紧凑 + 无尾随换行）"
    );
}

/// 形状硬约束：`autosave.json` 无尾随换行；`events.jsonl` 每行以 `\n` 结尾。
#[test]
fn trailing_newline_shape_is_frozen() {
    let autosave = fs::read_to_string(fixture(AUTOSAVE_FILE)).expect("读 autosave");
    assert!(
        !autosave.ends_with('\n'),
        "autosave.json 不得有尾随换行（历史形态末字节是 `}}`）"
    );
    let events = fs::read_to_string(fixture(EVENTS_FILE)).expect("读 events");
    assert!(
        events.ends_with('\n'),
        "events.jsonl 每行必须以 \\n 结尾（包括末行）"
    );
    assert_eq!(
        events.matches('\n').count(),
        events.lines().filter(|line| !line.is_empty()).count(),
        "一行一个换行，不得有空行"
    );
}

/// 键序必须是**字典序**：这是 `serde_json` 默认（BTreeMap）的证据。
///
/// 若有人给 `serde_json` 打开 `preserve_order`，这里的断言会红——那正是本测试
/// 存在的理由（值相等但键序变了，是「看起来正常」的破约）。
#[test]
fn json_map_keys_remain_lexicographic() {
    let mixed: Json = serde_json::from_str(r#"{"b":1,"a":2,"C":3,"c":4,"_":5}"#).expect("解析");
    assert_eq!(
        mixed.to_string(),
        r#"{"C":3,"_":5,"a":2,"b":1,"c":4}"#,
        "serde_json 的 Map 必须是 BTreeMap（字典序），不是插入序"
    );
    // fixture 自身也必须已经是字典序（否则上面的回放断言只是「两边都乱」）。
    let autosave: Json = serde_json::from_str(&fs::read_to_string(fixture(AUTOSAVE_FILE)).unwrap())
        .expect("fixture autosave 是 JSON");
    let keys: Vec<&String> = autosave.as_object().expect("对象").keys().collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted, "fixture autosave 的键序必须是字典序");
}

/// 旧局可读：把 fixture 当**既有 run 目录**，端口必须原样读出记录与存档。
#[test]
fn legacy_directory_is_readable() {
    let dir = temp_dir("legacy-read");
    fs::copy(fixture(EVENTS_FILE), dir.join(EVENTS_FILE)).expect("复制 events");
    fs::copy(fixture(AUTOSAVE_FILE), dir.join(AUTOSAVE_FILE)).expect("复制 autosave");

    let store = JsonFileStore::open(&dir).expect("打开旧局目录");
    let log = store.steps().expect("读事件记录");
    assert_eq!(log.skipped, 0, "fixture 没有坏行");
    assert_eq!(log.records.len(), 2);
    assert_eq!(store.next_seq().expect("seq"), 2, "seq = 非空行数");
    assert_eq!(
        store.latest_save().expect("读存档").expect("有存档"),
        fs::read(fixture(AUTOSAVE_FILE)).expect("读 fixture autosave"),
        "存档必须原样返回（不做 JSON 重整）"
    );
}

/// 缺档 / 空目录：`latest_save` 是 `None`（不是错误），`next_seq` 是 0。
#[test]
fn empty_directory_reads_as_empty_store() {
    let dir = temp_dir("empty");
    let store = JsonFileStore::open(&dir).expect("打开空目录");
    assert!(store.latest_save().expect("读存档").is_none());
    assert_eq!(store.next_seq().expect("seq"), 0);
    let log = store.steps().expect("读事件");
    assert!(log.records.is_empty());
    assert_eq!(log.skipped, 0);
}
