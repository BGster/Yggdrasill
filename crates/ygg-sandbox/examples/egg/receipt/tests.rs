use super::*;

/// `build_receipt` 形状的 ok fixture（新局 requirement.add，r5-plan §2.2 成功样例骨架；r2 rev2 注销 intake 后换存留事件）。
fn ok_receipt() -> Json {
    json!({
        "ok": true,
        "event": "requirement.add",
        "narrative": [
            "[REQ ADDED] r1 \"把宿主 CLI 回执渲染成自然语言\" — 1 requirement(s) in the graph.",
            "second narrative line",
        ],
        "pendingChoice": {
            "prompt": "Next step: accept a proposal, pass a gate, dispatch, or close into final review?",
            "options": [
                "requirement.dispatch — dispatch by readiness order (0 candidates, jobs 0/4, head —)",
                "requirement.add — register another requirement",
                "coordinate.submit — enter final review once every requirement is closed",
            ],
        },
        "autosave": ".ygg/runs/cli-text-probe-01/run.sqlite#latest",
        "seq": 0,
        "revision": 2,
        "meta": {
            "summary": {
                "stage": "coordinate",
                "ending": false,
                "verifyPassed": false,
                "nextEndsRun": false,
                "pendingProposals": 0,
                "requirementCount": 0,
                "conflicts": [],
            }
        },
    })
}

/// 越权拒绝 fixture（r5-plan §2.2 拒绝样例骨架：ok=false、无 revision、
/// pendingChoice 由 state 回填仍存在）。
fn failure_receipt() -> Json {
    json!({
        "ok": false,
        "event": "gate.approve",
        "narrative": ["[REJECTED:ACTION] `gate.approve` is not available in stage coordinate."],
        "pendingChoice": {
            "prompt": "Next step?",
            "options": ["requirement.dispatch — dispatch by readiness order"],
        },
        "autosave": ".ygg/runs/cli-text-probe-01/run.sqlite#latest",
        "seq": 12,
        "error": {
            "code": "REJECTED",
            "message": "This action is only valid in stage `review`.",
        },
        "meta": {
            "summary": {
                "stage": "coordinate",
                "ending": false,
                "verifyPassed": false,
                "nextEndsRun": false,
                "pendingProposals": 0,
                "requirementCount": 0,
                "conflicts": [],
            }
        },
    })
}

fn rendered(receipt: &Json) -> Vec<String> {
    render_receipt_text(receipt, "summary")
}

/// 用例 1：ok 回执渲染头行/narrative/编号块/状态行（含 1-based 编号与行序）。
#[test]
fn text_ok_receipt_renders_header_narrative_choices_status() {
    let lines = rendered(&ok_receipt());
    // 头行：event/seq/revision/存档齐备
    assert_eq!(
            lines[0],
            "── requirement.add · ok · seq 0 · revision 2 · 存档 .ygg/runs/cli-text-probe-01/run.sqlite#latest ──"
        );
    // narrative 原样逐行、两格缩进
    assert_eq!(
        lines[1],
        "  [REQ ADDED] r1 \"把宿主 CLI 回执渲染成自然语言\" — 1 requirement(s) in the graph."
    );
    assert_eq!(lines[2], "  second narrative line");
    // 状态行在 narrative 之后
    let status_at = lines
        .iter()
        .position(|line| line.starts_with("[状态]"))
        .expect("状态行存在");
    assert!(status_at > 2);
    assert_eq!(
        lines[status_at],
        "[状态] 阶段=coordinate · 需求=0 · 待验收提案=0 · 冲突=0"
    );
    // pendingChoice：? prompt + 1-based 编号（与交互共享 format_choice_lines）
    let prompt_at = lines
        .iter()
        .position(|line| line.starts_with("? "))
        .expect("prompt 行存在");
    assert!(status_at < prompt_at, "状态行在编号块之前");
    assert!(lines[prompt_at + 1].starts_with("  1. requirement.dispatch"));
    assert!(lines[prompt_at + 2].starts_with("  2. requirement.add"));
    assert!(lines[prompt_at + 3].starts_with("  3. coordinate.submit"));
    assert_eq!(prompt_at + 4, lines.len(), "编号块之后无多余行");
}

/// 用例 2：失败回执保留 narrative、加 `✗` 行（位于 narrative 之后、状态行之前），
/// 头行无 revision，pendingChoice 仍列出。
#[test]
fn text_failure_receipt_keeps_narrative_adds_error_line() {
    let lines = rendered(&failure_receipt());
    assert_eq!(
        lines[0],
        "── gate.approve · 失败 · seq 12 · 存档 .ygg/runs/cli-text-probe-01/run.sqlite#latest ──",
        "头行无 revision 段"
    );
    assert!(!lines[0].contains("revision"));
    // narrative 保留（拒绝前缀原样）
    assert_eq!(
        lines[1],
        "  [REJECTED:ACTION] `gate.approve` is not available in stage coordinate."
    );
    // ✗ 行存在且位于 narrative 之后、[状态] 之前
    let error_at = lines
        .iter()
        .position(|line| line.starts_with("✗ "))
        .expect("✗ 行存在");
    assert_eq!(
        lines[error_at],
        "✗ [REJECTED] This action is only valid in stage `review`."
    );
    assert!(error_at > 1, "✗ 行在 narrative 之后");
    let status_at = lines.iter().position(|line| line.starts_with("[状态]"));
    assert!(
        status_at.is_some_and(|at| at > error_at),
        "✗ 行在状态行之前"
    );
    // pendingChoice 仍列出（失败后 Agent 仍能接着走）
    assert!(lines.iter().any(|line| line.starts_with("? ")));
    assert!(lines.iter().any(|line| line.starts_with("  1. ")));
}

/// 用例 3：无 meta / 未知加性键（模拟 r3 actor 回显）→ 无状态行、不 panic、渲染完整。
#[test]
fn text_mode_tolerates_missing_summary_and_additive_keys() {
    let mut receipt = ok_receipt();
    receipt.as_object_mut().unwrap().remove("meta");
    receipt["actor"] = json!({"actorId": "agent-1", "actorSession": "s-1"});
    receipt["futureAdditiveKey"] = json!({"whatever": [1, 2, 3]});
    let lines = rendered(&receipt);
    assert!(
        lines.iter().all(|line| !line.starts_with("[状态]")),
        "无摘要时不得渲染状态行"
    );
    assert_eq!(
            lines[0],
            "── requirement.add · ok · seq 0 · revision 2 · 存档 .ygg/runs/cli-text-probe-01/run.sqlite#latest ──"
        );
    assert!(lines
        .iter()
        .any(|line| line.contains("second narrative line")));
    assert!(lines.iter().any(|line| line.starts_with("? ")));
    // 整体输出可用（不 panic 且含尾换行）
    let out = render_receipt_output(&receipt, OutputFormat::Text, "summary");
    assert!(out.ends_with('\n'));
}

/// 用例 4：结局分隔线只在 ending=true 时出现（空行 + 分隔线，紧邻且在编号块之前）。
#[test]
fn ending_separator_only_when_ending_true() {
    let receipt = ok_receipt();
    let lines = rendered(&receipt);
    assert!(!lines.iter().any(|line| line == "———————————"));

    let mut receipt = ok_receipt();
    receipt["ending"] = json!(true);
    let lines = rendered(&receipt);
    let sep_at = lines
        .iter()
        .position(|line| line == "———————————")
        .expect("分隔线存在");
    assert_eq!(lines[sep_at - 1], "", "分隔线前是空行");
    let prompt_at = lines
        .iter()
        .position(|line| line.starts_with("? "))
        .expect("编号块存在");
    assert!(sep_at < prompt_at, "分隔线在编号块之前");
}

/// 用例 5：附属行（⚠/⟳/[inbox]/[工具]/[HARNESS ACTION]）逐条渲染且按行序排列。
#[test]
fn auxiliary_lines_warnings_hotreload_inbox_tools_harness() {
    let mut receipt = ok_receipt();
    receipt["warnings"] = json!(["first warning", "second warning"]);
    receipt["hotReload"] = json!({"reloaded": true, "source": "resume"});
    receipt["inbox"] = json!({"applied": 2, "replayed": 1, "skipped": 0, "remaining": 3});
    receipt["tools"] = json!([{
        "callId": "run-0-0",
        "tool": "git.worktree_claim",
        "status": "ok",
        "resultEvent": "tool.result",
    }]);
    receipt["harnessActions"] = json!([{
        "action": "subagent.spawn",
        "reqId": "r1",
        "stage": "development",
        "rev": 1,
        "lane": 2,
        "worktreePath": "/tmp/wt",
        "artifactPath": "artifacts/r1-development.md",
    }]);
    let lines = rendered(&receipt);
    let expected = [
            "[HARNESS ACTION] subagent.spawn r1 (rev 1, stage=development, lane=2, worktree=/tmp/wt, artifact=artifacts/r1-development.md)",
            "⚠ first warning",
            "⚠ second warning",
            "⟳ mod 已热载（resume）：状态无损续局",
            "[inbox] 已消费 2 条（重放 1 条；剩 3 行）",
            "[工具] git.worktree_claim run-0-0 · ok · 结果经 tool.result",
        ];
    for expected_line in expected {
        assert!(
            lines.iter().any(|line| line == expected_line),
            "缺少附属行 `{expected_line}`；实际：{lines:?}"
        );
    }
    // 行序：⑤ HARNESS < ⑥ ⚠ < ⑦ ⟳ < ⑧ [inbox] < ⑨ [工具]
    let position_of = |prefix: &str| {
        lines
            .iter()
            .position(|line| line.starts_with(prefix))
            .unwrap_or_else(|| panic!("行 `{prefix}` 不存在"))
    };
    let order = [
        position_of("[HARNESS ACTION]"),
        position_of("⚠ first"),
        position_of("⟳"),
        position_of("[inbox]"),
        position_of("[工具]"),
    ];
    let mut sorted = order;
    sorted.sort_unstable();
    assert_eq!(order, sorted, "附属行顺序：⑤⑥⑦⑧⑨");
}

/// 用例 6：pendingChoice=null → 占位行，不复用交互模式的命令菜单提示。
#[test]
fn null_choice_renders_placeholder_without_interactive_menu() {
    let mut receipt = ok_receipt();
    receipt["pendingChoice"] = Json::Null;
    let lines = rendered(&receipt);
    assert!(lines.contains(&"（无待决选择）".to_owned()));
    assert!(
        lines
            .iter()
            .all(|line| !line.contains("数字=选择") && !line.contains("s / l / i / config / q")),
        "单发不得出现交互命令菜单提示"
    );
}

/// 用例 7：`--raw` × `YGG_FORMAT` 优先级矩阵（r5-plan §4.1）。
#[test]
fn resolve_output_format_priority_matrix() {
    // --raw 恒压 env（无论 env 为何）
    assert_eq!(
        resolve_output_format(true, Some("json")),
        Ok(OutputFormat::Raw)
    );
    assert_eq!(
        resolve_output_format(true, Some("text")),
        Ok(OutputFormat::Raw)
    );
    assert_eq!(resolve_output_format(true, None), Ok(OutputFormat::Raw));
    // 无旗标：env 决定
    assert_eq!(
        resolve_output_format(false, Some("json")),
        Ok(OutputFormat::Raw)
    );
    assert_eq!(
        resolve_output_format(false, Some("text")),
        Ok(OutputFormat::Text)
    );
    // 全缺 → 缺省 Text
    assert_eq!(resolve_output_format(false, None), Ok(OutputFormat::Text));
    // trim 后空串视同未设
    assert_eq!(
        resolve_output_format(false, Some("  ")),
        Ok(OutputFormat::Text)
    );
    // 非法值 fail-closed：Err 且文案点名 YGG_FORMAT 与合法值
    let error = resolve_output_format(false, Some("bogus")).unwrap_err();
    assert!(
        error.contains("YGG_FORMAT"),
        "文案须点名 YGG_FORMAT：{error}"
    );
    assert!(
        error.contains("text") && error.contains("json"),
        "文案须点名合法值：{error}"
    );
}

/// 用例 8：Raw 分支字节等价钉（单行、含尾换行、非 pretty；机器契约回归钉）。
#[test]
fn raw_output_is_byte_identical_single_line() {
    let receipt = ok_receipt();
    let raw = render_receipt_output(&receipt, OutputFormat::Raw, "summary");
    assert_eq!(
        raw,
        format!("{receipt}\n"),
        "Raw 分支必须恒等 `format!(\"{{receipt}}\\n\")`"
    );
    assert_eq!(raw.matches('\n').count(), 1, "单行（非 pretty）");
    assert!(raw.ends_with('\n'));
    // meta_key 不影响 Raw 分支（机器契约与人读状态面无关）
    assert_eq!(render_receipt_output(&receipt, OutputFormat::Raw, ""), raw);
    // 失败信封同样钉住
    let failure = failure_receipt();
    assert_eq!(
        render_receipt_output(&failure, OutputFormat::Raw, "summary"),
        format!("{failure}\n")
    );
}
