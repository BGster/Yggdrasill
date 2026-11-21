use super::*;

// ============================================================================
// §驱动 交互模式的推进方式（宿主只提供通道，**不替决策**）
//
// 推进策略不在宿主里：宿主要么等 stdin，要么把当前回执交给外部决策命令，
// 要么什么都不做只回显。Mod 的 `play.mode` 仍由 Mod 在叙事层使用（`config.set`
// 动作照旧可用），但 **host 不再读它、也不再解释它**。
// ============================================================================

/// 交互模式的推进方式。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Drive {
    /// 缺省：读 stdin，每一步推进都由人输入（数字 / event / s / l / i / config / q）。
    Stdin,
    /// 不读 stdin：每个待选动作交给外部决策命令（stdout 单行 `N` 或 `HUMAN[:理由]`）。
    Cmd(PathBuf),
    /// 只打印当前回执就退出（不推进、不读 stdin）。
    None,
}

impl Drive {
    /// 人读标签（启动头 / `config` 命令共用）。
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Stdin => "stdin（你逐步输入）".to_owned(),
            Self::Cmd(path) => format!("cmd:{}（外部决策命令自动推进）", path.display()),
            Self::None => "none（只打印当前回执，不推进）".to_owned(),
        }
    }

    pub(crate) fn is_cmd(&self) -> bool {
        matches!(self, Self::Cmd(_))
    }
}

/// `--drive` 取值解析：`stdin` / `cmd:<路径>` / `none`（其余一律用法错误）。
pub(crate) fn parse_drive(text: &str) -> Result<Drive, String> {
    let text = text.trim();
    match text {
        "stdin" => Ok(Drive::Stdin),
        "none" => Ok(Drive::None),
        _ => match text.strip_prefix("cmd:") {
            Some(path) if !path.trim().is_empty() => Ok(Drive::Cmd(PathBuf::from(path.trim()))),
            Some(_) => Err("--drive cmd: 后面必须给决策命令路径".to_owned()),
            None => Err(format!(
                "未知的 --drive 取值 `{text}`（只接受 stdin / cmd:<路径> / none）"
            )),
        },
    }
}

/// 单发回执的输出形态（`--raw` 旗标 / `YGG_FORMAT` 环境回退；缺省人读文本）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OutputFormat {
    /// 缺省：人读文本（版式不承诺稳定，脚本解析必须 `--raw`）。
    Text,
    /// 机器契约：stdout 单行 JSON 信封（与历史缺省逐字节一致）。
    Raw,
}

/// 单发回执输出形态的解析链 = `--raw` > 环境变量 `YGG_FORMAT`（text|json）> 缺省 [`OutputFormat::Text`]。
///
/// - `raw = true` → [`OutputFormat::Raw`]（旗标恒压 env，无论 env 为何——与
///   `--harness` > `YGG_HARNESS` 同一覆盖惯例）；
/// - env `"text"` → [`OutputFormat::Text`]、`"json"` → [`OutputFormat::Raw`]（小写精确匹配；
///   trim 后空串视同未设）；
/// - 其它值 → `Err`（fail-closed：调用方 `usage_error` 退出 2，**不静默回退**；
///   文案点名 `YGG_FORMAT` 与合法值）；
/// - 全缺 → [`OutputFormat::Text`]（缺省）。
pub(crate) fn resolve_output_format(raw: bool, env: Option<&str>) -> Result<OutputFormat, String> {
    if raw {
        return Ok(OutputFormat::Raw);
    }
    match env.map(str::trim) {
        None | Some("") | Some("text") => Ok(OutputFormat::Text),
        Some("json") => Ok(OutputFormat::Raw),
        Some(other) => Err(format!(
            "未知的 YGG_FORMAT 取值 `{other}`（只接受 text / json；text = 人读文本（缺省），json = 单行 JSON（等价 --raw））"
        )),
    }
}

#[cfg(test)]
mod drive_tests {
    use super::{parse_drive, Drive};
    use std::path::PathBuf;

    #[test]
    fn parses_the_three_states() {
        assert_eq!(parse_drive("stdin"), Ok(Drive::Stdin));
        assert_eq!(parse_drive("none"), Ok(Drive::None));
        assert_eq!(
            parse_drive("cmd:/tmp/decide.py"),
            Ok(Drive::Cmd(PathBuf::from("/tmp/decide.py")))
        );
        // 前后空白容忍（命令行粘贴常态）
        assert_eq!(
            parse_drive(" cmd: /tmp/x "),
            Ok(Drive::Cmd(PathBuf::from("/tmp/x")))
        );
    }

    #[test]
    fn rejects_unknown_and_empty_cmd_path() {
        for bad in ["", "goal", "guide", "STDIN", "cmd:", "cmd:   "] {
            assert!(parse_drive(bad).is_err(), "`{bad}` 应为用法错误");
        }
        // 错误文案要点出合法取值，便于命令行纠错
        let message = parse_drive("goal").unwrap_err();
        assert!(message.contains("stdin"), "{message}");
        assert!(message.contains("none"), "{message}");
    }
}

#[cfg(test)]
mod drive_limits_tests {
    use super::{DriveLimits, Path};
    use serde_json::json;

    fn source() -> &'static Path {
        Path::new("data/config.json")
    }

    /// 段缺失 / 非对象 / 缺字段 / 非正整数一律**硬错误**（无兜底、无默认值）。
    #[test]
    fn missing_or_invalid_thresholds_are_hard_errors() {
        for (config, needle) in [
            (json!({"play": {"mode": "guide"}}), "缺少 `drive` 段"),
            (json!({"drive": []}), "必须是对象"),
            (
                json!({"drive": {"maxSteps": 64, "stallLimit": 12}}),
                "`stepsPerRequirement`",
            ),
            (
                json!({"drive": {"maxSteps": 0, "stallLimit": 12, "stepsPerRequirement": 16}}),
                "`maxSteps`",
            ),
            (
                json!({"drive": {"maxSteps": "64", "stallLimit": 12, "stepsPerRequirement": 16}}),
                "`maxSteps`",
            ),
            (
                json!({"drive": {"maxSteps": 64, "stallLimit": -3, "stepsPerRequirement": 16}}),
                "`stallLimit`",
            ),
        ] {
            let message = DriveLimits::from_config(&config, source()).expect_err("必须报错");
            assert!(message.contains(needle), "文案应点名 `{needle}`：{message}");
            assert!(
                message.contains("README.md"),
                "文案应给出文档定位：{message}"
            );
        }
    }

    /// 三项都声明了就用声明值；步数预算按需求数线性放宽（饱和不溢出）。
    #[test]
    fn reads_the_three_required_thresholds_and_computes_the_budget() {
        let limits = DriveLimits::from_config(
            &json!({"drive": {"maxSteps": 10, "stallLimit": 3, "stepsPerRequirement": 5}}),
            source(),
        )
        .expect("合法 drive 段");
        assert_eq!(limits.max_steps, 10);
        assert_eq!(limits.stall_limit, 3);
        assert_eq!(limits.steps_per_requirement, 5);
        assert_eq!(limits.step_limit(0), 10);
        assert_eq!(limits.step_limit(4), 30);
        // 饱和运算：极大值不溢出
        let huge = DriveLimits {
            max_steps: usize::MAX,
            stall_limit: 1,
            steps_per_requirement: usize::MAX,
        };
        assert_eq!(huge.step_limit(2), usize::MAX);
    }
}

/// 自动推进一步的决策（派发留在主循环里，统一打印与计数）。
pub(crate) enum DriveStep {
    /// 可以再请求外部决策命令推进一步。
    Run,
    /// 本局已收口：调用现有结算逻辑写 settlement.json。
    Settle,
    /// 无路可走 / 超步数：打印可读错误并非零退出（不死循环、不静默成功）。
    Stuck(String),
}

/// 自动推进的一步决策：结局 → 结算；无待选 → 已收口则结算，否则中止；步数硬顶防绕圈。
/// `limit` = 本次局的步数预算（基础硬顶 + 每条需求额外预算，见 [`DriveLimits`]）。
pub(crate) fn drive_step(
    summary: &Summary,
    current: &Json,
    auto_steps: usize,
    limit: usize,
) -> DriveStep {
    // ① 本步叙事出现结局行：本局收口，自动结算。
    if current["ending"] == Json::Bool(true) {
        return DriveStep::Settle;
    }
    // ② 无待决动作：续局落在局末（Mod 自报 ending）也直接结算；否则是死路。
    if current["pendingChoice"].is_null() {
        if summary.ending() == Some(true) {
            return DriveStep::Settle;
        }
        return DriveStep::Stuck(
            "[drive] 当前无 pendingChoice 且未到结局——自动推进中止（Mod 没给出下一步动作）"
                .to_owned(),
        );
    }
    // ③ 防失控：步数预算（基础步数 + 每条异步需求额外步数）。
    if auto_steps >= limit {
        return DriveStep::Stuck(format!(
            "[drive] 已达最大步数 {limit} 仍未到结局——自动推进中止（防死循环）"
        ));
    }
    DriveStep::Run
}

/// r9 常驻批排 暂停判定（纯函数，内嵌测试在 resident_lifecycle_tests）：
/// mod 自报 lifecycle=resident ∧ 本批已排空且未武装退役 ∧ inbox 无未消费行。
/// 三者缺一不可：旧 mod（lifecycle 缺失）不触发；武装退役时不暂停（让泵走
/// 预 closing 协议收尾）；inbox 有未消费行时不暂停（先消费再判定）。
pub(crate) fn batch_drained_pause(summary: &Summary, unconsumed: usize) -> bool {
    summary.lifecycle == "resident" && summary.batch_drained && unconsumed == 0
}
