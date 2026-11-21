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

/// 用例专属临时目录（互不共享；先清空再建）。
pub(crate) fn case_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ygg-r5-reset-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 最小合成 Mod（2 步到结局 + 摘要 + inbox 路由）：零业务语义，装载期全部
/// 校验按宿主 fail-closed 规则现过（pack.json agent 段 / config.json drive
/// 与 settlement 段 / manifest）。entries 为空 → manifest 无需内容哈希。
pub(crate) fn write_probe_mod(dir: &Path) {
    fs::create_dir_all(dir.join("data")).expect("建 data 目录");
    fs::write(
        dir.join("pack.json"),
        r#"{
  "schemaVersion": 1,
  "entries": [],
  "schemaFiles": [],
  "agent": {
    "entryEvent": "event.start",
    "choiceEvent": "ygg.choice_selected",
    "summaryEvent": "agent.summary",
    "endingPrefix": "【结局",
    "metaKey": "summary",
    "rejectionPrefixes": ["【拒绝】"],
    "replayPrefix": "【重放】",
    "defaultPayload": "0"
  },
  "inboxRoutes": {
    "externalIdField": "external_id",
    "payloadField": "payload",
    "extMarker": "@ext=",
    "add": {
      "action": "inbox.add",
      "fields": ["title", "external_id"],
      "write": {"flag": "requirement", "shape": "named"}
    }
  }
}"#,
    )
    .expect("写 pack.json");
    fs::write(
        dir.join("manifest.json"),
        r#"{"schemaVersion": 1, "entries": [], "code": []}"#,
    )
    .expect("写 manifest.json");
    fs::write(
        dir.join("mod.lua"),
        r#"ygg.on("event.start", function()
  ygg.kv.set("stage", "started")
  ygg.narrate("开局完成")
  ygg.choice("下一步？", { "推进", "收尾" })
end)
ygg.on("ygg.choice_selected", function(index)
  if index == 2 then
    ygg.narrate("【结局】合成结局")
  else
    ygg.narrate("推进了一步")
    ygg.choice("继续？", { "推进", "收尾" })
  end
end)
ygg.on("agent.summary", function()
  ygg.meta("summary", { open = true })
  ygg.narrate("摘要就绪")
end)
ygg.on("inbox.add", function(payload)
  ygg.narrate("已入队：" .. tostring(payload))
end)
"#,
    )
    .expect("写 mod.lua");
    fs::write(
        dir.join("data/config.json"),
        r#"{
  "drive": {"maxSteps": 50, "stallLimit": 5, "stepsPerRequirement": 20},
  "settlement": {
    "requireEvents": true,
    "requireNoFailedSteps": true,
    "requireEnding": true,
    "requireInboxConsumed": true
  }
}"#,
    )
    .expect("写 config.json");
}

/// `store` 钉住 `Some(Json)`：**本模块的断言全部读 `autosave.json` /
/// `events.jsonl`**（归档台账、事件行数、坏档注入都是 JSON 载体的形状），
/// 后端一旦随默认值翻成 sqlite，这些断言会成批变红或变成空真（空真更危险：
/// 归档列表恒空 = 「恰好归档一次」永远看不出漏归档）。
/// 默认后端（`store: None`）的回归是 `r10*` 系列，二者不合并。
pub(crate) fn probe_args(mod_dir: &Path, runs_root: &Path, run_id: &str, resume: bool) -> Args {
    Args {
        packages: None,
        mod_dir: Some(mod_dir.to_path_buf()),
        open_style: Some(if resume {
            crate::OpenStyle::Open
        } else {
            crate::OpenStyle::New
        }),
        run_root: runs_root.join(run_id),
        runs_parent: runs_root.to_path_buf(),
        // 旧「显式新局」= 总是重置 → 新语义映射为 new --force（显式许可）。
        force: !resume,
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

/// 不传 `--store` 的宿主参数（`store: None`）：**默认后端专测面**。
///
/// 只有 `r10*` 用它——既有用例都钉住 JSON（见 [`probe_args`]），把「后端
/// 是哪个」与「默认值是哪个」拆成两组测试，才能让「默认值改动」与「旧局
/// 兼容」各自有独立的判据。
pub(crate) fn probe_args_default_store(
    mod_dir: &Path,
    runs_root: &Path,
    run_id: &str,
    resume: bool,
) -> Args {
    Args {
        store: None,
        ..probe_args(mod_dir, runs_root, run_id, resume)
    }
}

/// 开新局并推进一步（events=1 行、autosave 可装载的标准起点）。
pub(crate) fn new_game_with_one_step(mod_dir: &Path, runs_root: &Path, run_id: &str) {
    let mut session =
        Session::open(&probe_args(mod_dir, runs_root, run_id, false)).expect("新局开局必须成功");
    let receipt = session.dispatch_event("event.start", "0");
    assert_eq!(receipt["ok"], serde_json::json!(true), "开局步必须成功");
    assert_eq!(receipt["seq"], serde_json::json!(0));
}

/// run 目录里 `events.jsonl.reset.*` 归档份数。
pub(crate) fn reset_archives(run_dir: &Path) -> Vec<PathBuf> {
    let mut archives: Vec<PathBuf> = fs::read_dir(run_dir)
        .expect("读 run 目录")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name().map_or(false, |name| {
                name.to_string_lossy().starts_with("events.jsonl.reset.")
            })
        })
        .collect();
    archives.sort();
    archives
}

pub(crate) fn event_line_count(run_dir: &Path) -> usize {
    fs::read_to_string(run_dir.join(EVENTS_FILE))
        .map(|text| text.lines().filter(|line| !line.trim().is_empty()).count())
        .unwrap_or(0)
}

pub(crate) fn archive_line_count(archive: &Path) -> usize {
    fs::read_to_string(archive)
        .map(|text| text.lines().filter(|line| !line.trim().is_empty()).count())
        .expect("归档必须可读")
}
