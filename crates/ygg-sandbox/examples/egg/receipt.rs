use super::*;

/// 编号选择行的唯一出处（交互 [`print_choices`] 与单发文本渲染共享，防漂移）。
///
/// `pendingChoice` 是对象 → `? <prompt>` + `  <i>. <option>`（1-based，与交互
/// 同一编号函数）；`null` / 缺失 → 空 vec（调用方决定占位文案）。
pub(crate) fn format_choice_lines(receipt: &Json) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(choice) = receipt["pendingChoice"].as_object() {
        if let Some(prompt) = choice["prompt"].as_str() {
            lines.push(format!("? {prompt}"));
        }
        for (index, option) in choice["options"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            let text = option.as_str().unwrap_or("");
            lines.push(format!("  {}. {text}", index + 1));
        }
    }
    lines
}

/// 单发回执的人读文本渲染（缺省形态；**版式不承诺稳定，脚本解析必须 `--raw`**）。
///
/// 行序（r5-plan §2.2，对齐交互 `print_receipt` → `print_choices` 的既有顺序）：
/// ①头行 ②narrative 两格缩进 ③失败 `✗` 行（仅 ok=false 且有 error）
/// ④`[状态]` 行（仅摘要可用；`meta_key` 传 "" 跳过）⑤`[HARNESS ACTION]`
/// ⑥`⚠` 警告 ⑦`⟳` 热载 ⑧`[inbox]` ⑨`[工具]` ⑩结局分隔线
/// ⑪pendingChoice 编号块（null → 占位，不复用交互命令菜单）。
/// 只读已知键、对未知加性键宽容（不渲染、不 panic）。
pub(crate) fn render_receipt_text(receipt: &Json, meta_key: &str) -> Vec<String> {
    let mut lines = Vec::new();
    // ① 头行：`── <event> · <ok|失败> · seq <n> · revision <n> · 存档 <locator> ──`
    //   字段缺失优雅省略（失败无 revision；装载失败回执无 seq/revision；event 空则省事件段）。
    let mut header = String::from("── ");
    let event = receipt["event"].as_str().unwrap_or("");
    if !event.is_empty() {
        header.push_str(event);
        header.push_str(" · ");
    }
    header.push_str(if receipt["ok"] == Json::Bool(true) {
        "ok"
    } else {
        "失败"
    });
    if let Some(seq) = receipt["seq"].as_u64() {
        header.push_str(&format!(" · seq {seq}"));
    }
    if let Some(revision) = receipt["revision"].as_u64() {
        header.push_str(&format!(" · revision {revision}"));
    }
    if let Some(autosave) = receipt["autosave"].as_str() {
        if !autosave.is_empty() {
            header.push_str(&format!(" · 存档 {autosave}"));
        }
    }
    header.push_str(" ──");
    lines.push(header);
    // ② narrative 逐行原样，两格缩进（拒绝/重放前缀原样呈现，宿主不解释）。
    for line in narrative_lines(receipt) {
        lines.push(format!("  {line}"));
    }
    // ③ 失败醒目行：插在 narrative 之后、状态行之前。
    if receipt["ok"] != Json::Bool(true) && receipt["error"].as_object().is_some() {
        lines.push(format!("✗ {}", error_text(receipt)));
    }
    // ④ 状态行：仅 Mod 摘要可用时；装载失败路径 meta_key 传 "" 直接跳过。
    if !meta_key.is_empty() {
        let summary = Summary::from_receipt(receipt, meta_key);
        if summary.available {
            let mut status = format!(
                "[状态] 阶段={} · 需求={} · 待验收提案={} · 冲突={}",
                summary.stage,
                summary.requirement_count,
                summary.pending_proposals,
                summary.conflicts.len()
            );
            if let Some(awaiting) = &summary.awaiting {
                status.push_str(&format!(
                    " · 等待 {}@{}",
                    awaiting.req,
                    if awaiting.stage.is_empty() {
                        "?"
                    } else {
                        &awaiting.stage
                    }
                ));
            }
            if summary.verify_passed {
                status.push_str(" · 门禁自检=过");
            }
            lines.push(status);
        }
    }
    // ⑤-⑨ 附属行：presence 驱动（不经 generic 适配器门——那是 stderr 路由决策）。
    for line in format_harness_action_lines(receipt) {
        lines.push(line);
    }
    for warning in receipt["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
    {
        lines.push(format!("⚠ {warning}"));
    }
    if receipt["hotReload"]["reloaded"] == Json::Bool(true) {
        lines.push(format!(
            "⟳ mod 已热载（{}）：状态无损续局",
            receipt["hotReload"]["source"].as_str().unwrap_or("?")
        ));
    }
    if let Some(inbox) = receipt["inbox"].as_object() {
        lines.push(format!(
            "[inbox] 已消费 {} 条（重放 {} 条；剩 {} 行）",
            inbox["applied"].as_u64().unwrap_or(0),
            inbox["replayed"].as_u64().unwrap_or(0),
            inbox["remaining"].as_u64().unwrap_or(0)
        ));
    }
    for tool in receipt["tools"].as_array().into_iter().flatten() {
        lines.push(format!(
            "[工具] {} {} · {} · 结果经 {}",
            tool["tool"].as_str().unwrap_or("?"),
            tool["callId"].as_str().unwrap_or("?"),
            tool["status"].as_str().unwrap_or("?"),
            tool["resultEvent"].as_str().unwrap_or("?")
        ));
    }
    // ⑩ 结局分隔线（与交互同符）。
    if receipt["ending"] == Json::Bool(true) {
        lines.push(String::new());
        lines.push("———————————".to_owned());
    }
    // ⑪ pendingChoice 编号块：null → 占位（单发没有 s/l/i/config/q 交互命令）。
    let choice_lines = format_choice_lines(receipt);
    if choice_lines.is_empty() {
        lines.push("（无待决选择）".to_owned());
    } else {
        lines.extend(choice_lines);
    }
    lines
}

/// 单发打印点的唯一出口：[`OutputFormat::Raw`] = 单行 JSON 信封（**机器契约**，
/// 与历史缺省逐字节一致，含尾换行）；[`OutputFormat::Text`] = [`render_receipt_text`]
/// 的人读版式。打印点统一 `print!`（换行已含在返回值里）。
pub(crate) fn render_receipt_output(
    receipt: &Json,
    format: OutputFormat,
    meta_key: &str,
) -> String {
    match format {
        OutputFormat::Raw => format!("{receipt}\n"),
        OutputFormat::Text => {
            let mut out = String::new();
            for line in render_receipt_text(receipt, meta_key) {
                out.push_str(&line);
                out.push('\n');
            }
            out
        }
    }
}

/// 打印一步回执（`fresh=false` 时只打印错误——存读档等不推进剧情）。
/// 无模式分支：结局行本身由 Mod 的 narrative 提供，宿主只加一条通用分隔。
pub(crate) fn print_receipt(receipt: &Json, fresh: bool) {
    if receipt["ok"] != Json::Bool(true) {
        println!("[错误 {}] {}", error_code(receipt), error_message(receipt));
        return;
    }
    if fresh {
        for line in receipt["narrative"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|value| value.as_str())
        {
            println!("  {line}");
        }
        for line in format_harness_action_lines(receipt) {
            println!("  {line}");
        }
    }
    for warning in receipt["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
    {
        println!("⚠ {warning}");
    }
    if receipt["hotReload"]["reloaded"] == Json::Bool(true) {
        println!(
            "⟳ mod 已热载（{}）：状态无损续局",
            receipt["hotReload"]["source"].as_str().unwrap_or("?")
        );
    }
    if receipt["ending"] == Json::Bool(true) {
        println!();
        println!("———————————");
    }
}

/// 处境面：待决选择（编号选项）或无选择时的可用命令
/// （`--drive none` 只回显这一屏，stdin 驱动则在后面接提示符）。
pub(crate) fn print_choices(receipt: &Json) {
    println!();
    match receipt["pendingChoice"].as_object() {
        // 编号唯一出处 = format_choice_lines（与单发文本渲染共享，防漂移）。
        Some(_) => {
            for line in format_choice_lines(receipt) {
                println!("{line}");
            }
        }
        None => {
            println!("（无待决选择。数字=选择 / event <名称> [JSON] / s / l / i / config / q）")
        }
    }
}

/// 处境提示面 + 输入提示符（提示符不携带推进方式语义）。
pub(crate) fn print_prompt(receipt: &Json) {
    print_choices(receipt);
    print!("[ygg] > ");
    let _ = io::stdout().flush();
}

/// 不推进状态的回执（续局 / 读档 / none 驱动共用）：`pendingChoice` 现取 `state` 信封。
pub(crate) fn state_receipt(session: &mut Session, event: &str, narrative: Vec<String>) -> Json {
    let state = session.state_envelope();
    json!({
        "ok": true,
        "event": event,
        "narrative": narrative,
        "pendingChoice": state["pendingChoice"].clone(),
        "autosave": session.store.locator(),
    })
}

/// `event <名称> [JSON]` 解析（JSON 可含空格——只切首段）。
///
/// 省略载荷时用 Mod 契约声明的入口载荷（`default_payload`），宿主不持有字面量。
pub(crate) fn split_event_command(input: &str, default_payload: &str) -> Option<(String, String)> {
    let (head, tail) = match input.split_once(char::is_whitespace) {
        Some((head, tail)) => (head, tail),
        None => (input, ""),
    };
    if head != "event" {
        return None;
    }
    let tail = tail.trim();
    if tail.is_empty() {
        return None;
    }
    let (name, payload) = match tail.split_once(char::is_whitespace) {
        Some((name, payload)) => (name, payload.trim()),
        None => (tail, ""),
    };
    let payload = if payload.is_empty() {
        default_payload
    } else {
        payload
    };
    Some((name.to_owned(), payload.to_owned()))
}

/// `i`：诊断面人读输出（内核 state 信封原字段）。
pub(crate) fn print_state(session: &mut Session) {
    let state = session.state_envelope();
    if state["ok"] != Json::Bool(true) {
        println!("[错误 {}] {}", error_code(&state), error_message(&state));
        return;
    }
    let pending = if state["pendingChoice"].is_null() {
        json!(null)
    } else {
        state["pendingChoice"]["prompt"].clone()
    };
    println!(
        "engine={} revision={} kvRevision={} snapshotBytes={} pendingChoice={}",
        state["engineVersion"],
        state["revision"],
        state["kvRevision"],
        state["snapshotBytes"],
        pending,
    );
}

/// `config`：本局宿主侧设置（mod / run 目录 / 热载 / 监视文件 / 序号）。
pub(crate) fn print_config(session: &Session, args: &Args) {
    let watched = session
        .signature
        .iter()
        .map(|stamp| stamp.path.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    println!("mod      : {}", session.mod_label);
    println!("run-id   : {}", session.run_id);
    println!("run 目录 : {}", session.run_dir.display());
    println!(
        "存储后端 : {}（存档定位：{}；run.json 的 \"store\" 是权威）",
        session.store.backend(),
        session.store.locator()
    );
    println!(
        "配置      : {}（play.mode 归 Mod 叙事层；宿主只读其中的 drive 阈值）",
        session.mod_dir.join(&session.config_rel).display()
    );
    let limits = session.limits;
    println!(
        "推进阈值 : 步数硬顶 {} = {} + {}×需求数；无进展判停 {}",
        limits.step_limit(0),
        limits.max_steps,
        limits.steps_per_requirement,
        limits.stall_limit
    );
    let settlement = session.settlement;
    println!(
        "结算口径 : events={} noFailedSteps={} ending={} inboxConsumed={}（config.json 的 settlement 段）",
        settlement.require_events,
        settlement.require_no_failed_steps,
        settlement.require_ending,
        settlement.require_inbox_consumed
    );
    println!(
        "驱动     : {}（推进方式由 --drive 给定；Mod 的 play.mode 仅叙事层自用）",
        args.drive.label()
    );
    println!(
        "事件日志 : {}（已记录 {} 条，下一条 seq={}）",
        session.store.locator(),
        session.count_events(),
        session.count_events()
    );
    println!(
        "需求 inbox: {}（已消费 {} 行 / 共 {} 行；--requirement 只追加）",
        session.inbox_path.display(),
        session.inbox_cursor(),
        session.store.inbox_len().unwrap_or(0)
    );
    println!(
        "热载     : {}",
        if args.watch {
            "开启（每步检查 mtime）"
        } else {
            "关闭（--watch 开启）"
        }
    );
    println!("监视文件 : {watched}");
}

// ============================================================================
// §共用 信封解析 / 错误文案 / 时钟 / 落盘
// ============================================================================

/// 信封解析（内部错误不 exit：agent 长驻，坏信封回可读错误信封续走）。
pub(crate) fn parse_envelope(reply: &str) -> Json {
    serde_json::from_str(reply).unwrap_or_else(|err| {
        json!({
            "ok": false,
            "error": {"code": "INTERNAL", "message": format!("信封不是合法 JSON：{err}")},
        })
    })
}

pub(crate) fn error_code(envelope: &Json) -> &str {
    envelope["error"]["code"].as_str().unwrap_or("UNKNOWN")
}

pub(crate) fn error_message(envelope: &Json) -> &str {
    envelope["error"]["message"].as_str().unwrap_or("")
}

pub(crate) fn error_text(envelope: &Json) -> String {
    format!("[{}] {}", error_code(envelope), error_message(envelope))
}

/// 成功信封的叙事行（失败信封无该键 → 空）。
pub(crate) fn narrative_lines(envelope: &Json) -> Vec<String> {
    envelope["narrative"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect()
}

// ============================================================================
// §r5 单发回执文本渲染（缺省 text / --raw 唯一旗标）
// ============================================================================
#[cfg(test)]
mod tests;
