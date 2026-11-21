use super::*;

// ============================================================================
// §交互模式 数字选项 / event / s / l / i / config / q（现场感：每步打印处境）
// 推进方式由 `--drive` 决定（stdin / cmd:<路径> / none）：宿主只提供通道，
// 不持有推进策略——决策由人或外部决策命令给出。
// ============================================================================

pub(crate) fn run_interactive(args: &Args) -> i32 {
    let mut session = match Session::open(args) {
        Ok(session) => session,
        Err(message) => {
            eprintln!("error: {message}");
            return 1;
        }
    };

    let drive = args.drive.clone();
    println!("== Yggdrasill Agent Harness · 交互模式 ==");
    println!("mod      : {}", session.mod_label);
    println!("run 目录 : {}", session.run_dir.display());
    println!("驱动     : {}", drive.label());
    if args.open_style == Some(OpenStyle::New) {
        // new 的稳定输出行（与单发同契约：末行 run-root 供脚本提取）。
        println!("run-root: {}", session.run_dir.display());
    }
    println!("命令：数字=选择 / event <名称> [JSON] / s=存档 / l=读档 / i=状态 / config / q=退出");
    if drive.is_cmd() {
        println!("自动推进 : 由决策命令替选动作；返回 HUMAN / 超时 / 非零退出即安全暂停（exit 0）");
    }
    if args.watch {
        println!("热载     : 开启（每步检查 mod 目录 mtime）");
    }
    for notice in std::mem::take(&mut session.notices) {
        println!("⚠ {notice}");
    }
    println!();

    // 新局自动派发入口事件（与 adventure.rs 同为「开局即给处境」）；
    // 续局 / none 驱动只回显当前处境（pendingChoice 随存档往返），不推进剧情。
    let entry_event = session.contract.entry_event.clone();
    let entry_payload = session.contract.default_payload.clone();
    let mut current = if session.resumed {
        state_receipt(&mut session, "resume", Vec::new())
    } else if matches!(drive, Drive::None) {
        // none = 只读回显：新局也不派发入口事件（零副作用）。
        state_receipt(&mut session, "state", Vec::new())
    } else {
        session.dispatch_event(&entry_event, &entry_payload)
    };
    let mut fresh = true;
    // 自动推进计数（`[drive] 自动推进 N` 与步数预算共用）。
    let mut auto_steps: usize = 0;
    // 连续「无进展」计数（pendingChoice 指纹不变 = 绕圈；防死循环第二道闸）。
    let mut stall_steps: usize = 0;
    let mut last_signature = choice_signature(&current);
    // 协议状态：本进程是否已持有 closing（收口保护）。
    let mut closing_held = false;

    // stdin 只锁一次（与 adventure.rs 同纪律：锁内单循环，无递归重入）。
    // 只有 stdin 驱动才占用 stdin：cmd / none 不读输入。
    let stdin = io::stdin();
    let mut lines = matches!(drive, Drive::Stdin).then(|| stdin.lock().lines());

    loop {
        // 每步开头先消费 inbox：新需求入队会改变 pendingChoice，所以拿到新回执就
        // 直接用它当前态（否则会按**上一拍**的选项推进，正好踩到提前结局防护）。
        let drain = session.drain_inbox();
        if drain.applied > 0 {
            println!(
                "[inbox] 已消费 {} 条异步需求（其中幂等重放 {} 条；剩 {} 行未消费）",
                drain.applied, drain.replayed, drain.remaining
            );
            if let Some(receipt) = drain.last {
                current = receipt;
                fresh = true;
                stall_steps = 0;
            }
        }
        if let Some(message) = &drain.error {
            println!(
                "[警告] inbox 未能全部并入：{message}（剩 {} 行）",
                drain.remaining
            );
        }

        print_receipt(&current, fresh);
        fresh = false;

        // cmd 驱动：不读 stdin；每个待选动作交给外部决策命令，HUMAN/异常即安全暂停。
        if let Drive::Cmd(program) = &drive {
            // ⓪ 回执标准字段（Mod 自报）：没有它 = 旧 Mod / 摘要行损坏，
            //    自动推进无法判定 → 安全停等（绝不回退到读 KV）。
            let summary = session.summary();
            if !summary.available {
                eprintln!(
                    "error: Mod 未提供 summary（动作 `{}` 无摘要行），无法自动判定——\
                     改用 --drive stdin 或升级 Mod；本步未推进",
                    session.contract.summary_event
                );
                return 0;
            }
            // ① 冲突 = **用户门禁**：必须停下打印双方与说明，绝不自动选解决路径。
            if !summary.conflicts.is_empty() {
                println!();
                println!("[drive] 需求冲突——需要你确认解决路径（不会自动选择）：");
                for conflict in &summary.conflicts {
                    println!("  {}", conflict.label());
                }
                println!(
                    "  解决后重跑本命令；或改用 --drive stdin 手动选 pendingChoice 的 1/2/3（keep_both / keep_first / revise）。"
                );
                return 0;
            }
            // ①' r9 常驻批排 安全暂停（判定在 batch_drained_pause，纯函数内嵌测试）：
            //     打印等待文案后 exit 0——哑泵绝不在空批上连点 coordinate.submit 制造
            //     空批结算噪音。状态未推进，投递 --requirement 或武装退役后重跑继续。
            if batch_drained_pause(&summary, unconsumed_inbox_lines(session.store.as_ref())) {
                println!();
                println!(
                    "[drive] 第 {} 批已排空（coordinate，队列清空，未武装退役）——常驻运行安全暂停：",
                    summary.batch
                );
                println!(
                    "        投递新需求（--requirement '标题|描述'）继续下一批；或 config.set run.retire=true 武装退役。"
                );
                println!("        状态未推进，处理后重跑本命令继续。");
                print_receipt(&current, true);
                return 0;
            }
            // 协议：下一步若是收口动作（Mod 自报 nextEndsRun），先进入 closing——
            // 阻止新写入，并在保护下把最后一次 drain 排在收口之前。
            if !closing_held && summary.next_ends_run {
                match enter_closing(&session.run_dir) {
                    Ok(ClosingEntry::AlreadyFinalized) => {
                        eprintln!("error: 本局已 finalized（settlement 已定稿）——需开新局");
                        return 1;
                    }
                    Ok(_) => {}
                    Err(message) => {
                        eprintln!("error: {message}");
                        return 1;
                    }
                }
                let pre = session.drain_inbox();
                if pre.applied > 0 {
                    if let Some(receipt) = pre.last {
                        // drain 到新需求：解除 closing，继续正常 loop（不能结局）。
                        leave_closing(&session.run_dir);
                        current = receipt;
                        fresh = true;
                        continue;
                    }
                }
                // 确认没有新需求 → 在 closing 保护下继续推进收口。
                closing_held = true;
            }
            // ② 有 job 在跑却一份待验收提案都没有 → 宿主**不造内容**（门禁必须由
            //    真实交付满足）。这是「等待外部执行方」的正常停等态：不推进状态、
            //    打印可读原因、退出码 0，收到提案后 --resume 继续。
            if summary.pending_proposals == 0 {
                if let Some(awaiting) = &summary.awaiting {
                    println!();
                    println!(
                        "[drive] {} 的 {} 作业在跑（rev {}），但没有任何待验收提案——",
                        awaiting.req, awaiting.stage, awaiting.rev
                    );
                    println!(
                        "        宿主不生成提案内容，等待外部执行方提交（proposal.submit / --proposal）。"
                    );
                    println!("        安全暂停：状态未推进，收到提案后重跑本命令继续。");
                    print_receipt(&current, true);
                    if closing_held {
                        leave_closing(&session.run_dir);
                    }
                    return 0;
                }
            }
            let limits = session.limits;
            let limit = limits.step_limit(summary.requirement_count);
            match drive_step(&summary, &current, auto_steps, limit) {
                DriveStep::Run => {
                    // 收口前保险：本步可能推进到 review/done（下一拍就结算），
                    // 先把刚到达的行吃掉——否则只能靠结算硬门禁报错，白跑一局。
                    let pre = session.drain_inbox();
                    if pre.applied > 0 {
                        if let Some(receipt) = pre.last {
                            current = receipt;
                            fresh = true;
                            continue;
                        }
                    }
                    // 无进展判据：上一步的 pendingChoice 指纹与本次相同 = 状态没变。
                    let signature = choice_signature(&current);
                    if signature == last_signature {
                        stall_steps += 1;
                    } else {
                        stall_steps = 0;
                        last_signature = signature;
                    }
                    if stall_steps >= limits.stall_limit {
                        eprintln!(
                            "error: [drive] 连续 {} 步 pendingChoice 无变化——自动推进中止（防死循环）",
                            limits.stall_limit
                        );
                        print_receipt(&current, true);
                        return 1;
                    }
                    auto_steps += 1;
                    let choice = match decide_choice(&session, program, &current) {
                        DecideOutcome::Index(index) => index,
                        DecideOutcome::Human(reason) => {
                            eprintln!(
                                "[drive] 需要人工决策：{reason}——安全暂停，处理后重跑本命令继续"
                            );
                            print_receipt(&current, true);
                            println!("当前回执：{current}");
                            if closing_held {
                                leave_closing(&session.run_dir);
                            }
                            return 0;
                        }
                    };
                    println!("[drive] 自动推进 {auto_steps}/{limit}（决策命令返回序号 {choice}）");
                    let choice_event = session.contract.choice_event.clone();
                    current = session.dispatch_event(&choice_event, &choice);
                    if current["ok"] != Json::Bool(true) {
                        eprintln!("error: [drive] 自动推进失败：{}", error_text(&current));
                        print_receipt(&current, true);
                        return 1;
                    }
                    fresh = true;
                    continue;
                }
                DriveStep::Settle => {
                    // 协议：收口必须先把本局从 running 切到 closing，
                    // 之后任何 `--requirement` 都会被命令本身非零拒绝。
                    if !closing_held {
                        match enter_closing(&session.run_dir) {
                            Ok(ClosingEntry::AlreadyFinalized) => {
                                eprintln!(
                                    "error: 本局已 finalized（settlement 已定稿）——不重写报告，需开新局"
                                );
                                return 1;
                            }
                            // 已持 closing：本分支随后直接结算返回，无需再置位。
                            Ok(_) => {}
                            Err(message) => {
                                eprintln!("error: {message}");
                                return 1;
                            }
                        }
                    }
                    // closing 保护下的最后 drain：任何成功写入都 happens-before 这里。
                    let final_drain = session.drain_inbox();
                    if final_drain.applied > 0 {
                        if let Some(receipt) = final_drain.last {
                            leave_closing(&session.run_dir);
                            closing_held = false;
                            current = receipt;
                            fresh = true;
                            continue;
                        }
                    }
                    println!();
                    println!("[drive] 已到结局（自动推进 {auto_steps} 步），开始结算…");
                    let code = run_settle(args, true);
                    // 用**现读**的未消费行数报错（不用 final_drain 的快照：结算本身要花时间）。
                    let leftover = unconsumed_inbox_lines(session.store.as_ref());
                    if leftover > 0 {
                        eprintln!(
                            "error: 结局后仍有 {leftover} 条未消费 inbox 行（{}）——本局已收口，\
                             需开新局（换 --run-id）再并入；settlement 已标 ok=false",
                            session.inbox_path.display()
                        );
                        return if code == 0 { 1 } else { code };
                    }
                    return code;
                }
                DriveStep::Stuck(message) => {
                    eprintln!("error: {message}");
                    return 1;
                }
            }
        }

        // none 驱动：只回显当前回执，不推进、不读 stdin。
        if matches!(drive, Drive::None) {
            print_choices(&current);
            println!();
            println!(
                "（--drive none：只打印当前回执，未推进；要推进请用 --drive stdin 或 --drive cmd:<路径>）"
            );
            return 0;
        }

        print_prompt(&current);

        // 能走到这里的只有 stdin 驱动（cmd 已在上面的分支推进或返回）。
        let Some(lines) = lines.as_mut() else {
            eprintln!("error: 内部错误：非 stdin 驱动却走到输入环节");
            return 1;
        };
        let line = match lines.next() {
            Some(Ok(line)) => line,
            Some(Err(err)) => {
                eprintln!("[错误] 读取输入失败：{err}");
                return 1;
            }
            None => {
                println!();
                println!("（输入结束，退出。）");
                return 0;
            }
        };
        let input = line.trim();
        if input.is_empty() {
            continue;
        }

        match input {
            "q" | "quit" | "exit" => {
                println!("再见。");
                return 0;
            }
            "s" => match session.save_now() {
                Ok(()) => println!("已存档 → {}", session.store.locator()),
                Err(message) => println!("[错误] {message}"),
            },
            "l" => match session.load_autosave() {
                Ok(()) => {
                    current = state_receipt(
                        &mut session,
                        "load",
                        vec!["（读档完成，进度已恢复。）".to_owned()],
                    );
                    fresh = true;
                }
                Err(message) => println!("[错误] {message}"),
            },
            "i" => print_state(&mut session),
            "config" => print_config(&session, args),
            _ => {
                let default_payload = session.contract.default_payload.clone();
                if let Some((name, payload)) = split_event_command(input, &default_payload) {
                    current = dispatch_with_watch(&mut session, args, &name, &payload);
                    fresh = true;
                } else if let Ok(index) = input.parse::<i64>() {
                    if index < 1 {
                        println!("（选项序号从 1 起。）");
                        continue;
                    }
                    let choice_event = session.contract.choice_event.clone();
                    current =
                        dispatch_with_watch(&mut session, args, &choice_event, &index.to_string());
                    fresh = true;
                } else {
                    println!(
                        "（无法识别的输入：{input:?}。\
                         数字=选择 / event <名称> [JSON] / s / l / i / config / q）"
                    );
                }
            }
        }
    }
}

/// `--watch` 下的派发前热载检查（未开启则跳过：交互模式默认不查盘）。
pub(crate) fn dispatch_with_watch(
    session: &mut Session,
    args: &Args,
    name: &str,
    payload: &str,
) -> Json {
    if args.watch {
        session.maybe_reload();
    }
    session.dispatch_event(name, payload)
}

// ============================================================================
// §r9 常驻生命周期（宿主可见性与暂停）：摘要三字段容忍解析 + 批排暂停判定
//
// Summary 的三个新字段（lifecycle / batch / batchDrained）全部**加性**：
// 旧 mod 的 meta.summary 缺这些字段 → 容忍式解析取保守默认（"" / 1 / false），
// 所有常驻特化行为以 lifecycle=="resident" 为闸，旧 mod 行为不变（probe mod
// 无 lifecycle 字段天然走旧路径，既有用例零改动即覆盖该面）。
// ============================================================================
#[cfg(test)]
mod resident_lifecycle_tests {
    use super::{batch_drained_pause, Summary};
    use serde_json::json;

    /// 旧 mod：meta.summary 没有三个新字段 → 保守默认（lifecycle=""，batch=1，
    /// batch_drained=false），且批排暂停绝不触发（lifecycle 闸）。
    #[test]
    fn missing_fields_take_conservative_defaults_and_never_pause() {
        let legacy = Summary::from_json(&json!({
            "stage": "coordinate",
            "ending": false,
            "nextEndsRun": false,
            "pendingProposals": 0,
            "requirementCount": 0,
            "conflicts": [],
            "jobs": [],
            "gates": []
        }));
        assert_eq!(legacy.lifecycle, "");
        assert_eq!(legacy.batch, 1);
        assert!(!legacy.batch_drained);
        assert!(
            !batch_drained_pause(&legacy, 0),
            "旧 mod 摘要绝不触发批排暂停"
        );
    }

    /// resident 摘要：三字段全进；batch 的 0 / 缺失都折成 1（计数下界）。
    #[test]
    fn resident_summary_fields_parse_additively() {
        let resident = Summary::from_json(&json!({
            "stage": "coordinate",
            "ending": false,
            "verifyPassed": true,
            "nextEndsRun": false,
            "lifecycle": "resident",
            "batch": 3,
            "batchDrained": true,
            "pendingProposals": 0,
            "requirementCount": 2,
            "conflicts": [],
            "jobs": [],
            "gates": []
        }));
        assert_eq!(resident.lifecycle, "resident");
        assert_eq!(resident.batch, 3);
        assert!(resident.batch_drained);

        let zero = Summary::from_json(&json!({"lifecycle": "resident", "batch": 0}));
        assert_eq!(zero.batch, 1, "batch 计数下界是 1（非法 0 折成 1）");
        let absent = Summary::from_json(&json!({"lifecycle": "resident"}));
        assert_eq!(absent.batch, 1, "batch 缺省 1");
    }

    /// 批排暂停判定：三条件缺一不可（lifecycle 闸 / 排空信号 / inbox 无未消费行）。
    #[test]
    fn batch_drained_pause_requires_resident_drained_and_empty_inbox() {
        let summary = |lifecycle: &str, drained: bool| Summary {
            available: true,
            lifecycle: lifecycle.to_owned(),
            batch_drained: drained,
            ..Default::default()
        };
        // ① 常驻 + 已排空 + inbox 空 → 暂停。
        assert!(batch_drained_pause(&summary("resident", true), 0));
        // ② lifecycle 缺失（旧 mod）→ 不触发（即使 batchDrained 误报 true）。
        assert!(!batch_drained_pause(&summary("", true), 0));
        // ③ 未排空（含武装退役时刻：mod 侧把 batchDrained 报 false）→ 不触发。
        assert!(!batch_drained_pause(&summary("resident", false), 0));
        // ④ inbox 有未消费行 → 不暂停（先消费再判定，绝不在有进料时空停）。
        assert!(!batch_drained_pause(&summary("resident", true), 2));
    }
}
