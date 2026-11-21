use super::*;

// ============================================================================
// §drive 外部决策 回调协议 + 5 秒 fail-closed 暂停
// ============================================================================

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DecideOutcome {
    /// 经 pendingChoice 范围校验后的序号（从 1 开始）。
    Index(String),
    /// 需要人判断，或决策链路的任何一环失败；主状态不得推进。
    Human(String),
}

/// 解析决策命令的唯一 stdout 行：`N` 或 `HUMAN[:理由]`。
/// 非法输出、非数字、0 与越界序号全部按 HUMAN 处理。
pub(crate) fn parse_decision_output(stdout: &str, option_count: usize) -> DecideOutcome {
    let text = stdout.trim();
    if text.contains(['\n', '\r']) {
        return DecideOutcome::Human(
            "决策命令输出非法：协议只接受单行 N 或 HUMAN[:理由]".to_owned(),
        );
    }
    if text == "HUMAN" {
        return DecideOutcome::Human("决策命令要求人工处理".to_owned());
    }
    if let Some(reason) = text.strip_prefix("HUMAN:") {
        let reason = reason.trim();
        return DecideOutcome::Human(if reason.is_empty() {
            "决策命令要求人工处理".to_owned()
        } else {
            reason.to_owned()
        });
    }
    if !text.is_empty() && text.chars().all(|ch| ch.is_ascii_digit()) {
        if let Ok(index) = text.parse::<usize>() {
            if (1..=option_count).contains(&index) {
                return DecideOutcome::Index(index.to_string());
            }
        }
        return DecideOutcome::Human(format!(
            "决策命令返回越界序号（合法范围 1..={option_count}）"
        ));
    }
    DecideOutcome::Human("决策命令输出非法：仅接受 N 或 HUMAN[:理由]".to_owned())
}

/// 把完整当前回执交给外部决策命令。命令缺失、spawn/读写失败、超时、非零退出、
/// 非法输出与越界序号全部 fail closed；这里不存在「回退选 1」分支。
///
/// `program` 来自 `--drive cmd:<路径>`（`YGG_DECIDE_CMD` 可临时覆盖）。
pub(crate) fn decide_choice(session: &Session, program: &Path, current: &Json) -> DecideOutcome {
    let Some(options) = current["pendingChoice"]["options"].as_array() else {
        return DecideOutcome::Human("当前回执缺少 pendingChoice.options".to_owned());
    };
    if options.is_empty() {
        return DecideOutcome::Human("pendingChoice.options 为空".to_owned());
    }
    let input_path = session.run_dir.join(DECIDE_INPUT_FILE);
    if let Err(message) = write_json_pretty(&input_path, current) {
        return DecideOutcome::Human(format!("写入决策输入失败：{message}"));
    }

    // 显式 env 覆盖优先；即使它指向不存在的命令也不回退到任何默认序号。
    let program = std::env::var_os("YGG_DECIDE_CMD")
        .map(PathBuf::from)
        .unwrap_or_else(|| program.to_path_buf());
    if program.as_os_str().is_empty() {
        return DecideOutcome::Human("决策命令路径为空".to_owned());
    }
    let mut child = match Command::new(&program)
        .arg(&input_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            return DecideOutcome::Human(format!("无法启动决策命令 {}：{err}", program.display()));
        }
    };

    let deadline = Instant::now() + DECIDE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(DECIDE_POLL_INTERVAL);
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return DecideOutcome::Human("决策命令超过 5 秒，已终止".to_owned());
            }
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                return DecideOutcome::Human(format!("等待决策命令失败：{err}"));
            }
        }
    }
    let output = match child.wait_with_output() {
        Ok(output) => output,
        Err(err) => return DecideOutcome::Human(format!("读取决策命令输出失败：{err}")),
    };
    if !output.status.success() {
        return DecideOutcome::Human(format!("决策命令非零退出（{}）", output.status));
    }
    let stdout = match String::from_utf8(output.stdout) {
        Ok(stdout) => stdout,
        Err(_) => return DecideOutcome::Human("决策命令 stdout 不是合法 UTF-8".to_owned()),
    };
    parse_decision_output(&stdout, options.len())
}

#[cfg(test)]
mod decide_tests {
    use super::{parse_decision_output, DecideOutcome};

    #[test]
    fn accepts_only_valid_choice_numbers() {
        assert_eq!(
            parse_decision_output("2\n", 3),
            DecideOutcome::Index("2".to_owned())
        );
        for output in ["0", "4", "-1", "+1", "1 2", ""] {
            assert!(matches!(
                parse_decision_output(output, 3),
                DecideOutcome::Human(_)
            ));
        }
    }

    #[test]
    fn preserves_human_reason_and_rejects_multiline_output() {
        assert_eq!(
            parse_decision_output("HUMAN: 门禁需要核验", 3),
            DecideOutcome::Human("门禁需要核验".to_owned())
        );
        assert!(matches!(
            parse_decision_output("HUMAN: first\nsecond", 3),
            DecideOutcome::Human(reason) if reason.contains("单行")
        ));
    }

    /// 协议的五种返回：合法序号 / HUMAN / 空 / 越界 / 多行——除第一种外一律 Human。
    #[test]
    fn covers_the_five_protocol_returns() {
        // ① 合法序号（含恰好等于选项数的边界）
        assert_eq!(
            parse_decision_output("3", 3),
            DecideOutcome::Index("3".to_owned())
        );
        // ② HUMAN（无理由 / 带理由）
        assert!(matches!(
            parse_decision_output("HUMAN", 1),
            DecideOutcome::Human(reason) if !reason.is_empty()
        ));
        assert_eq!(
            parse_decision_output("HUMAN: 需要人核验", 1),
            DecideOutcome::Human("需要人核验".to_owned())
        );
        // ③ 空输出（纯空白等价空串）
        for output in ["", "   \n"] {
            assert!(matches!(
                parse_decision_output(output, 1),
                DecideOutcome::Human(_)
            ));
        }
        // ④ 越界序号（含 0）
        for output in ["0", "4"] {
            assert!(matches!(
                parse_decision_output(output, 3),
                DecideOutcome::Human(reason) if reason.contains("越界")
            ));
        }
        // ⑤ 多行（只接受单行）
        assert!(matches!(
            parse_decision_output("1\n2", 3),
            DecideOutcome::Human(reason) if reason.contains("单行")
        ));
    }
}
