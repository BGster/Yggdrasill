use super::*;

// ============================================================================
// §常量 只有**不是契约**的东西：run 目录文件名、宿主自身的超时。
//
// 事件名 / 叙事前缀 / 摘要键名 / 入口载荷 / inbox 字段名与幂等标记 / 结算口径 /
// 推进阈值**一律来自 Mod 自己的声明**（`pack.json` 的 `agent` / `inboxRoutes` +
// `config.json` 的 `settlement` / `drive`，解析在 `ygg_sandbox::host_contract`）。
// 本文件**不含任何 Mod 语义字面量**与回退默认值：契约缺失即装载期失败（fail-closed）。
// 归属边界（哪些字面量属于 host）见 `data/playbook/ch4-mod-host.json` 的 `contract.literal-ownership`。
// ============================================================================

/// 外部决策命令的最长执行时间；任何异常都 fail closed，绝不回退到固定序号。
pub(crate) const DECIDE_TIMEOUT: Duration = Duration::from_secs(5);
/// 决策命令轮询间隔（std-only：spawn + try_wait + 超时 kill/wait）。
pub(crate) const DECIDE_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// `cli.program` 的**自指别名**：`egg` 与 [`SELF_PROGRAM`]（`@self`）等价。
///
/// 两个名字都解析到 `current_exe()`（见 `execute_cli`）——内置子命令因此不依赖
/// `$PATH`：`pack.json` 是静态文件，写不了绝对路径；写裸名又不保证能调到。
///
/// 边界：裸名 `egg` 从此被宿主本体占用，外部工具不得取名 `egg`，否则在
/// `pack.json` 里声明同名 `program` 会撞到宿主而不是外部工具。
///
/// [`SELF_PROGRAM`]: ygg_sandbox::host_contract::SELF_PROGRAM
pub(crate) const SELF_ALIAS: &str = "egg";

// 产物文件名里，「存档」与「inbox」两个仍被宿主直接引用（`--settle` 的错误文案、
// 未开局时的回执）；其余文件名（`events.jsonl` / `inbox.cursor` / `run.sqlite`）
// 已完全归后端所有，宿主不再拼字符串——否则改后端时会出现「宿主拼的名字」与
// 「后端写的位置」两张皮。
pub(crate) const META_FILE: &str = "run.json";
pub(crate) const SETTLEMENT_FILE: &str = "settlement.json";
pub(crate) const DECIDE_INPUT_FILE: &str = "decide-input.json";

pub(crate) const MOD_SOURCE_FILE: &str = "mod.lua";
pub(crate) const MANIFEST_FILE: &str = "manifest.json";
pub(crate) const PACK_FILE: &str = "pack.json";

// ---- 工具执行器的宿主侧常量（不是契约：契约在 pack.json / data/config.json） ----

/// 在飞工具的事实面（只落**未解析模板**，绝不落 `$ENV{}` 解析后的值）。
pub(crate) const PENDING_TOOLS_FILE: &str = "pending_tools.json";
/// 工具结果暂存目录（`<run 目录>/tool_results/<call_id>.json`）。
pub(crate) const TOOL_RESULTS_DIR: &str = "tool_results";
/// 一条派发链里的工具回投上限（工具结果事件里再请求工具 = 新的一轮）。
pub(crate) const MAX_TOOL_ROUNDS: usize = 4;
/// 工具结果轮询间隔（与 `DECIDE_POLL_INTERVAL` 同值同源）。
pub(crate) const TOOL_POLL_INTERVAL: Duration = Duration::from_millis(20);
/// 单个参数（含 `$ENV{}` 解析后的值）的字节上限（防凭据被当载荷走私）。
pub(crate) const MAX_TOOL_ARG_BYTES: usize = 8_192;
/// `--wait-tools` 的等待上限缺省（毫秒）。
pub(crate) const WAIT_TOOLS_DEFAULT_MS: u64 = 120_000;
/// 交付认领的宽限（毫秒）：父进程的等待窗口结束后再多给这么多，才让 worker 认领。
///
/// 为什么需要：worker 写完结果文件后会立刻尝试认领。若它比父进程的轮询
/// （`TOOL_POLL_INTERVAL`）先一步，场景 1（同进程就地回投）就永远不会发生
/// ——快工具（<20ms）会全部变成「脱体」。worker 因此等「父进程窗口 + 宽限」
/// 过去再认领：父进程活着就它赢（场景 1），父进程已放弃 / 崩溃才轮到
/// worker（场景 2）。`inlineWaitMs == 0` 时宽限也为 0（父进程根本不轮询）。
pub(crate) const TOOL_CLAIM_SLACK_MS: u64 = 100;
/// `--wait-tools` 的心跳间隔（只在 stderr 上报，不影响状态）。
pub(crate) const WAIT_TOOLS_HEARTBEAT: Duration = Duration::from_secs(1);

/// run-root 的基名（run-id 的派生形态：目录名即本局标识；run.json / inbox
/// 幂等键 / settlement 报告沿用同一名字空间）。
pub(crate) fn run_basename(run_root: &Path) -> &str {
    run_root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
}

/// `--at` 取值解析：`g<代>`（如 `g0` / `g3`；代 = sqlite generation，存档点边界）。
/// 也接受裸数字（与 `g<N>` 同义）。其余一律用法错误。
pub(crate) fn parse_at(text: &str) -> Result<i64, String> {
    let trimmed = text.trim();
    let digits = trimmed.strip_prefix('g').unwrap_or(trimmed);
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!(
            "--at 需要 `g<代>`（如 g0 / g3；代 = sqlite generation 存档点），当前是 `{text}`"
        ));
    }
    digits
        .parse::<i64>()
        .map_err(|_| format!("--at 的代数超出范围：`{text}`"))
}

/// new 的 run-root 裁决（main 早期、一切模式之前）：自动生成 run-id 并建父目录。
///
/// id 形态 `r-<yyyymmdd>-<hhmmss>-<4hex>`（过 `is_valid_run_id` 白名单；同秒冲突
/// 由随机段吸收，仍撞则换段重试，8 次后报错）。**零创建语义**：这里只定名，
/// 目录由 `Session::open_mode` 的 `create_dir_all` 建立（后端裁决先于任何写入）。
/// open / serve / builtin-tool 原样返回（run-root 已在 parse_args 就位）。
pub(crate) fn prepare_run_root(args: Args) -> Result<Args, String> {
    if args.open_style != Some(OpenStyle::New) {
        return Ok(args);
    }
    let parent = &args.runs_parent;
    if parent.exists() && !parent.is_dir() {
        return Err(format!(
            "--in 指定的 runs 父目录 {} 已存在但不是目录",
            parent.display()
        ));
    }
    fs::create_dir_all(parent)
        .map_err(|err| format!("创建 runs 父目录 {} 失败：{err}", parent.display()))?;
    let stamp = {
        let secs = now_secs();
        // 本地时间戳（人读目录名）：YYYYMMDD-HHMMSS；不引入新依赖，手工换算 UTC。
        let days = secs / 86_400;
        let rem = secs % 86_400;
        let (year, month, day) = civil_from_days(days as i64);
        format!(
            "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
            rem / 3600,
            (rem % 3600) / 60,
            rem % 60
        )
    };
    for attempt in 0..8 {
        let suffix = format!("{:04x}", (now_ms() as u64 ^ (attempt * 0x9e37)) & 0xffff);
        let run_id = format!("r-{stamp}-{suffix}");
        let run_root = parent.join(&run_id);
        if !run_root.exists() {
            return Ok(Args { run_root, ..args });
        }
    }
    Err(format!(
        "自动 run-id 在 {} 下连续 8 次撞名（同秒高并发？）——请重试",
        parent.display()
    ))
}

/// 天数 → 公历（Howard Hinnant 的 civil_from_days 算法；纯整数，无依赖）。
pub(crate) fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// 取旗标后一位的值（缺失即用法错误）。
pub(crate) fn next_value(iter: &mut impl Iterator<Item = String>, flag: &str) -> String {
    match iter.next() {
        Some(value) => value,
        None => usage_error(&format!("{flag} 缺少取值")),
    }
}

/// run-id 白名单：字母数字 + `-` `_` `.`（路径穿越与目录分隔符一律拒绝）。
pub(crate) fn is_valid_run_id(run_id: &str) -> bool {
    !run_id.is_empty()
        && run_id != "."
        && run_id != ".."
        && run_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}
