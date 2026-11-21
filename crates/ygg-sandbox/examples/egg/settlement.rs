use super::*;

#[cfg(test)]
mod settlement_policy_tests {
    use super::{Path, SettlementCheck, SettlementPolicy};
    use serde_json::json;

    fn source() -> &'static Path {
        Path::new("data/config.json")
    }

    /// 段缺失 / 四个开关缺任一 / 非布尔 → 硬错误（无「默认开启」）。
    #[test]
    fn missing_section_or_switch_is_a_hard_error() {
        let message = SettlementPolicy::from_config(&json!({"play": {}}), source())
            .expect_err("缺段必须报错");
        assert!(message.contains("`settlement` 段"), "{message}");
        assert!(
            message.contains("\"requireEvents\""),
            "应给出形状示例：{message}"
        );
        assert!(message.contains("README.md"), "{message}");

        for field in [
            "requireEvents",
            "requireNoFailedSteps",
            "requireEnding",
            "requireInboxConsumed",
        ] {
            let mut section = json!({
                "requireEvents": true,
                "requireNoFailedSteps": true,
                "requireEnding": true,
                "requireInboxConsumed": true,
            });
            section.as_object_mut().expect("对象").remove(field);
            let message = SettlementPolicy::from_config(&json!({"settlement": section}), source())
                .expect_err("缺开关必须报错");
            assert!(
                message.contains(&format!("`{field}`")),
                "必须点名缺失字段 `{field}`：{message}"
            );
        }
        // 非布尔也报错（不做「读不懂按 true」）
        for broken in [
            json!({"settlement": {"requireEvents": "true", "requireNoFailedSteps": true, "requireEnding": true, "requireInboxConsumed": true}}),
            json!({"settlement": []}),
        ] {
            assert!(SettlementPolicy::from_config(&broken, source()).is_err());
        }
        // 文件读不到同样是硬错误
        assert!(SettlementPolicy::load(Path::new("/nonexistent/config.json")).is_err());
    }

    /// 四项按声明生效（关 = 不参与 `ok`）。
    #[test]
    fn reads_the_four_required_switches() {
        let policy = SettlementPolicy::from_config(
            &json!({
                "settlement": {
                    "requireEvents": true,
                    "requireNoFailedSteps": false,
                    "requireEnding": false,
                    "requireInboxConsumed": true
                }
            }),
            source(),
        )
        .expect("合法 settlement 段");
        assert!(policy.require_events);
        assert!(!policy.require_no_failed_steps);
        assert!(!policy.require_ending);
        assert!(policy.require_inbox_consumed);
    }

    /// 关掉的项一律算满足（`ok` 由声明口径决定），但事实仍如实记录。
    #[test]
    fn disabled_checks_are_satisfied_but_still_reported() {
        let check = SettlementCheck {
            name: "ending",
            enabled: false,
            passed: false,
            error: "未见结局叙事行：本局未收口".to_owned(),
        };
        assert!(check.satisfied());
        assert_eq!(
            check.to_json(),
            json!({"name": "ending", "enabled": false, "passed": false})
        );
        let enabled = SettlementCheck {
            enabled: true,
            ..check
        };
        assert!(!enabled.satisfied());
    }
}

// ============================================================================
// §结算 --settle：autosave + events.jsonl → settlement.json + 人读摘要
// ============================================================================

/// 一条结算判定项（`settlement.json` 的 `checks` 元素）。
///
/// `enabled` = Mod 在 `config.json` 的 `settlement` 段声明它是否参与 `ok`；
/// `passed` = 该项本身的事实（关掉也照实记录，便于人读诊断）。
pub(crate) struct SettlementCheck {
    pub(crate) name: &'static str,
    pub(crate) enabled: bool,
    pub(crate) passed: bool,
    /// 生效但未通过时的可读错误（进报告 `errors`）。
    pub(crate) error: String,
}

impl SettlementCheck {
    /// 该项是否满足（关掉的项一律算满足——口径由 Mod 声明）。
    fn satisfied(&self) -> bool {
        !self.enabled || self.passed
    }

    fn to_json(&self) -> Json {
        json!({"name": self.name, "enabled": self.enabled, "passed": self.passed})
    }
}

/// 结算口径：从本局 Mod 的 `config.json` 现读 `settlement` 段（必填，无默认值）。
///
/// Mod 目录解析见 [`resolve_mod_dir`]；段缺失 / 缺键 → 可读错误（fail-closed），
/// 由调用方报错退出——**不**用任何「默认开启」兜底。
pub(crate) fn load_settlement_policy(
    run_dir: &Path,
    args: &Args,
) -> Result<SettlementPolicy, String> {
    let mod_dir = resolve_mod_dir(args, run_dir)?;
    SettlementPolicy::load(&mod_dir.join(agent_config_path(&mod_dir)))
}

pub(crate) fn run_settle(args: &Args, auto: bool) -> i32 {
    settle_report(args, auto, true).0
}

/// 结算本体（**唯一**实现：独立 `--settle` 与守护模式的 `settle` 动作共用）。
///
/// 返回 `(退出码, 报告)`：报告为 `None` = 结算在写出报告之前就失败（原因已进
/// stderr）；`Some` = 报告已落 `settlement.json`，直接交给调用方复用。
///
/// `human=false` 时**不**打印人读摘要——守护模式的 stdout 是 JSON-RPC 协议面，
/// 人读文本会撕裂协议流。
pub(crate) fn settle_report(args: &Args, auto: bool, human: bool) -> (i32, Option<Json>) {
    let run_dir = args.run_root.clone();
    // 结算也必须走端口：sqlite 局没有 autosave.json / events.jsonl 可读
    // （照旧读文件 = 把已收口的局报成「无存档」）。
    let store = match open_store_resolved(&run_dir, args.store) {
        Ok(store) => store,
        Err(message) => {
            eprintln!("error: {message}");
            return (1, None);
        }
    };

    // ⓞ 协议：结算必须持有 closing（与 inbox 写入者互斥）。
    //    - finalized：幂等返回已有报告的 ok，不重写；
    //    - 本进程已持 closing（自动推进到结局后的 settle）：直接进；
    //    - 否则现在就进入并等在途写者落盘。
    match enter_closing(&run_dir) {
        Ok(ClosingEntry::AlreadyFinalized) => {
            let existing = run_dir.join(SETTLEMENT_FILE);
            if let Some(report) = read_json(&existing) {
                if human {
                    println!(
                        "本局已 finalized（{}），settlement ok = {}——不重写；需新局请换 --run-id",
                        existing.display(),
                        report["ok"]
                    );
                }
                let code = if report["ok"] == Json::Bool(true) {
                    0
                } else {
                    1
                };
                return (code, Some(report));
            }
            eprintln!(
                "error: 本局已 finalized 但找不到 settlement.json（{}）",
                existing.display()
            );
            return (1, None);
        }
        Ok(_) => {}
        Err(message) => {
            eprintln!("error: {message}");
            return (1, None);
        }
    }
    // 进入 closing 后不再有新写入者；此处只需核对存量未消费行。
    let pending_before = unconsumed_inbox_lines(store.as_ref());
    if auto {
        closing_heartbeat(&run_dir);
    }

    // ① 终局存档（唯一状态事实源；缺档即无法结算）。
    let save_text = match store.latest_save() {
        Ok(Some(bytes)) => match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(err) => {
                eprintln!("error: 存档不是合法 UTF-8（{}）：{err}", store.locator());
                if !auto {
                    leave_closing(&run_dir);
                }
                return (1, None);
            }
        },
        Ok(None) => {
            eprintln!(
                "error: 读取 {} 失败：无存档（先跑一局再结算）",
                store.locator()
            );
            if !auto {
                leave_closing(&run_dir);
            }
            return (1, None);
        }
        Err(err) => {
            eprintln!("error: {err}（先跑一局再结算）");
            if !auto {
                leave_closing(&run_dir);
            }
            return (1, None);
        }
    };
    let save: Json = match serde_json::from_str(&save_text) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("error: {} 不是合法 JSON：{err}", store.locator());
            if !auto {
                leave_closing(&run_dir);
            }
            return (1, None);
        }
    };

    // ② 事件日志（坏行跳过并记入 errors，不整份报废）。
    let log = match store.steps() {
        Ok(log) => log,
        Err(err) => {
            eprintln!("error: {err}");
            if !auto {
                leave_closing(&run_dir);
            }
            return (1, None);
        }
    };
    let records = log.records;
    let mut errors = Vec::new();
    if log.skipped > 0 {
        errors.push(format!(
            "{} 有 {} 行事件记录无法解析（已跳过）",
            store.locator(),
            log.skipped
        ));
    }

    // ③ 事件面汇总：成功步数 / 失败步 / 结局行。
    let mut steps = 0usize;
    let mut reached_ending = false;
    let mut ending_line: Option<String> = None;
    let mut events = Vec::with_capacity(records.len());
    for record in &records {
        if record.ok {
            steps += 1;
        }
        if record.ending {
            reached_ending = true;
            if let Some(line) = &record.ending_line {
                ending_line = Some(line.clone());
            }
        }
        events.push(json!({
            "event": record.event,
            "seq": record.seq,
            "ok": record.ok,
        }));
    }
    let failed = events
        .iter()
        .filter(|e| e["ok"] != Json::Bool(true))
        .count();

    // ④ 终局面：kv 是持久结论面（结局 / 评分 / 经验落在这里），实体表只报规模。
    let kv = save["kv"]["entries"].clone();
    let final_state = json!({
        "ending": kv.get("ending").cloned().unwrap_or(Json::Null),
        "score": kv.get("score").cloned().unwrap_or(Json::Null),
        "exp": kv.get("exp").cloned().unwrap_or(Json::Null),
        "kv": kv,
        "kvRevision": save["kv"]["revision"].clone(),
        "entities": save["entities"]["entities"].as_object().map_or(0, serde_json::Map::len),
    });

    // ⑤ 结论：四个判定项，各自受 Mod 声明的结算口径开关控制（`settlement` 段必填，
    //    无默认值）。`inboxConsumed` 那条是硬门禁：inbox 还有行却报 ok=true 等于
    //    静默丢需求。closing 已阻止新写入，故 `pending_before` 就是最终口径；
    //    仍现读一次作交叉校验。口径读不到 → fail-closed（报告作废并退出非零）。
    let policy = match load_settlement_policy(&run_dir, args) {
        Ok(policy) => policy,
        Err(message) => {
            eprintln!("error: {message}");
            if !auto {
                leave_closing(&run_dir);
            }
            return (1, None);
        }
    };
    let pending_inbox = unconsumed_inbox_lines(store.as_ref()).max(pending_before);
    let checks = vec![
        SettlementCheck {
            name: "events",
            enabled: policy.require_events,
            passed: !events.is_empty(),
            error: "事件日志为空：本局无任何派发记录".to_owned(),
        },
        SettlementCheck {
            name: "noFailedSteps",
            enabled: policy.require_no_failed_steps,
            passed: failed == 0,
            error: format!("{failed} 条事件记录为失败步"),
        },
        SettlementCheck {
            name: "ending",
            enabled: policy.require_ending,
            passed: reached_ending,
            error: "未见结局叙事行：本局未收口".to_owned(),
        },
        SettlementCheck {
            name: "inboxConsumed",
            enabled: policy.require_inbox_consumed,
            passed: pending_inbox == 0,
            error: format!(
                "结局后仍有 {pending_inbox} 条未消费 inbox 行（{INBOX_FILE}）——本局已收口，需开新局（换 --run-id）"
            ),
        },
    ];
    let ok = checks.iter().all(SettlementCheck::satisfied);
    for check in &checks {
        if check.enabled && !check.passed {
            errors.push(check.error.clone());
        }
    }
    let checks_json: Vec<Json> = checks.iter().map(SettlementCheck::to_json).collect();

    let mut report = json!({
        "run_id": run_basename(&args.run_root),
        "mod": run_mod_label(&run_dir, args),
        "steps": steps,
        "events": events,
        "final_state": final_state,
        "duration_events": records.len(),
        "reached": if reached_ending { "ending" } else { "open" },
        "ok": ok,
        "checks": checks_json,
        "errors": errors,
        "settled_at": now_secs(),
    });

    // ⑥ 在飞工具（R6）：**不**做第 5 个 settlement 门禁（`SettlementPolicy` 四键是
    //    Mod 必填面，加键会 break 所有既有 Mod）。只写进报告 + 一行 warning，
    //    `ok` 不变——「在飞工具要不要清零」由 Mod / 运维自己定。
    let tools_pending = pending_calls(&run_dir);
    if !tools_pending.is_empty() {
        report["toolsPending"] = json!(tools_pending
            .iter()
            .map(|call| json!({
                "callId": call.call_id,
                "tool": call.tool,
                "deadlineUnixMs": call.deadline_unix_ms as u64,
            }))
            .collect::<Vec<_>>());
        report["warnings"] = json!([format!(
            "结算时仍有 {} 个在飞工具（{PENDING_TOOLS_FILE}；worker 可能已崩溃）——结果可用 --tool-result 人工补票",
            tools_pending.len()
        )]);
    }

    // ⑥ 落盘（pretty：人读证据）+ 人读摘要。
    let settlement_path = run_dir.join(SETTLEMENT_FILE);
    if let Err(message) = write_json_pretty(&settlement_path, &report) {
        eprintln!("error: {message}");
        if !auto {
            leave_closing(&run_dir);
        }
        return (1, None);
    }
    if human {
        print_settlement(&report, &settlement_path, ending_line.as_deref(), failed);
    }
    if !ok {
        // 未能给绿：已到结局的自动结算就 finalize（后续写入必须开新局）；
        // 独立 --settle 则退回 running，让 resume 先把存量行消费完。
        if auto || pending_inbox > 0 {
            finalize_run(&run_dir);
        } else {
            leave_closing(&run_dir);
        }
        return (1, Some(report));
    }
    // ⑦ closing 保护下的交叉校验：进入 closing 后不应再出现新行，
    //    但保留一次现读作为兵底检查（万一有绕过协议的写入）。
    //    与 ⑤ 的 `inboxConsumed` 同一开关：关掉它 = 明确接受结局后的残留行。
    let after = if policy.require_inbox_consumed {
        unconsumed_inbox_lines(store.as_ref())
    } else {
        0
    };
    if after > 0 {
        eprintln!(
            "error: closing 保护下仍发现 {after} 条未消费 inbox 行（协议外写入）——报告作废（ok=false），需开新局"
        );
        report["ok"] = json!(false);
        if let Some(list) = report["errors"].as_array_mut() {
            list.push(json!(format!(
                "closing 保护下仍发现 {after} 条未消费 inbox 行（需开新局）"
            )));
        }
        if let Err(message) = write_json_pretty(&settlement_path, &report) {
            eprintln!("error: {message}");
        }
        finalize_run(&run_dir);
        return (1, Some(report));
    }
    // 真正的「已定稿」：之后所有写命令一律非零拒绝（需开新局）。
    finalize_run(&run_dir);
    if auto {
        closing_heartbeat(&run_dir);
    }
    (0, Some(report))
}

/// 结算报告的 `mod` 标签：run.json 元信息优先（最贴近本局实际装载），
/// 其次 `--mod`，都缺则 `(未知)`。
pub(crate) fn run_mod_label(run_dir: &Path, args: &Args) -> String {
    read_meta(&run_dir.join(META_FILE))
        .and_then(|meta| meta["mod"].as_str().map(str::to_owned))
        .or_else(|| args.mod_dir.as_ref().map(|path| path.display().to_string()))
        .unwrap_or_else(|| "(未知)".to_owned())
}

/// run.json 元信息（缺失 / 非法一律 None：元信息是便利面，不是判据）。
pub(crate) fn read_meta(path: &Path) -> Option<Json> {
    read_json(path)
}

/// 读一个 JSON 文件（缺失 / 非法 → None；宿主侧便利面，不吞错、不死循环）。
pub(crate) fn read_json(path: &Path) -> Option<Json> {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Json>(&text).ok())
}

/// 元信息里的 mod 签名（跨进程热载比对基线）。
pub(crate) fn read_meta_signature(path: &Path) -> Option<String> {
    read_meta(path).and_then(|meta| meta["mod_signature"].as_str().map(str::to_owned))
}

/// 结算摘要（stdout 人读面；JSON 报告是机器面）。
pub(crate) fn print_settlement(
    report: &Json,
    path: &Path,
    ending_line: Option<&str>,
    failed: usize,
) {
    let kv = &report["final_state"]["kv"];
    println!(
        "== 局末结算 · {} ==",
        report["run_id"].as_str().unwrap_or("?")
    );
    println!("mod      : {}", report["mod"].as_str().unwrap_or("(未知)"));
    println!(
        "步数     : {} 成功 / {} 条事件记录（{} 失败）",
        report["steps"], report["duration_events"], failed
    );
    match ending_line {
        Some(line) => println!("结局     : {line}"),
        None => println!("结局     : （未达结局，reached={}）", report["reached"]),
    }
    println!(
        "终局面   : ending={} score={} exp={}",
        report["final_state"]["ending"],
        report["final_state"]["score"],
        report["final_state"]["exp"]
    );
    println!("kv 摘要  : {}", kv_digest(kv, 12));
    println!(
        "规模     : 实体 {} 个 / kvRevision {}",
        report["final_state"]["entities"], report["final_state"]["kvRevision"]
    );
    // 判定口径：只有被 Mod 关掉的项才打印（缺省全开 = 不占一行）。
    let waived: Vec<&str> = report["checks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|check| check["enabled"] != Json::Bool(true))
        .filter_map(|check| check["name"].as_str())
        .collect();
    if !waived.is_empty() {
        println!(
            "口径     : {} 项已按 config.settlement 关闭（{}）",
            waived.len(),
            waived.join(" / ")
        );
    }
    if let Some(errors) = report["errors"].as_array() {
        if !errors.is_empty() {
            println!("问题     : {}", errors.len());
            for error in errors.iter().filter_map(|value| value.as_str()) {
                println!("  - {error}");
            }
        }
    }
    println!("ok       : {}", report["ok"]);
    println!("report   : {}", path.display());
}

/// kv 摘要人读串（`k=v` 逗号分隔，超限截断并注明余量）。
pub(crate) fn kv_digest(kv: &Json, limit: usize) -> String {
    let Some(map) = kv.as_object() else {
        return "（无）".to_owned();
    };
    let mut parts = Vec::new();
    for (key, value) in map.iter().take(limit) {
        parts.push(format!("{key}={}", scalar_label(value)));
    }
    if map.len() > limit {
        parts.push(format!("…(+{} 项)", map.len() - limit));
    }
    if parts.is_empty() {
        "（空）".to_owned()
    } else {
        parts.join(", ")
    }
}

/// 标量人读形态（字符串去引号，其余走 JSON 字面量）。
pub(crate) fn scalar_label(value: &Json) -> String {
    match value {
        Json::String(text) => text.clone(),
        other => other.to_string(),
    }
}
