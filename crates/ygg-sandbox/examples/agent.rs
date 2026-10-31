//! Agent harness CLI（M-Agent 泳道 B）：把内核当 **Agent 的执行骨架**用。
//!
//! Agent 主导驱动：Agent 调本 CLI 派发动作，CLI 回执「处境叙事 + 可选动作」，
//! 每步自动存档，mod 改动热载，局末产出结算报告。内核零改动（`KernelFacade`
//! 的 `dispatch` 信封即唯一接驳面——与 `adventure.rs` / `playtest.rs` 同构）。
//!
//! 职责边界：宿主只做**内核物理上做不到的事**（文件 / stdin / 时钟 / 锁 / 跨进程
//! 互斥），**不持有推进策略也不认识 Mod 内部状态**：
//! - 推进策略由 `--drive` 选定（跟人读 stdin、或交给外部决策命令、或什么都不做）；
//! - “何时收口 / 有无冲突 / 有无待验收提案”只问 Mod 自己声明的摘要
//!   （[`SUMMARY_EVENT`] 的 `【摘要】` 行），不读 `ending` / `fsm.stage` /
//!   `prop.*` / `req.*` 等 Mod 私有 KV 键。
//!
//! 三种形态：
//!
//! ```bash
//! # ① 交互模式（人 / 脚本驱动）
//! cargo run -p ygg-sandbox --example agent -- --mod packages/mods/test-adventure --run-id my-task
//!
//! # ② 单发模式（Agent 脚本化调用：动作 → JSON 回执 → 退出）
//! cargo run -p ygg-sandbox --example agent -- --mod <目录> --run-id my-task \
//!   --resume --event ygg.choice_selected --payload '1'
//!
//! # ③ 局末结算（读 autosave + events.jsonl → settlement.json）
//! cargo run -p ygg-sandbox --example agent -- --run-id my-task --settle
//!
//! # ④ 异步补需求（只追 inbox.jsonl 后退出；不碰 autosave，故可在另一进程跑
//! #    本局的同时安全调用）
//! cargo run -p ygg-sandbox --example agent -- --run-id my-task --runs-root .ygg/runs \
//!   --requirement '补一条门禁日志|把 verify 的失败原因写进 events.jsonl'
//! ```
//!
//! ## 异步需求 inbox（④，跨进程队列入口）
//!
//! 协议：`<runs-root>/<run-id>/inbox.jsonl`，每行一个 JSON
//! `{external_id, title, description, created_at}`；`--requirement` 只做
//! `OpenOptions::append`，**不读写 autosave**，所以与运行中的局无锁竞争。
//! 文件保留作审计；消费位移记在 `inbox.cursor`（`{"consumed": N}`，按非空行数）。
//!
//! 消费时机：cmd 自动推进**之前**、stdin 每次提示**之前**、单发每次派发**之前**
//! 都会 drain 新行并 dispatch `requirement.add`（Mod 侧 FIFO 入队）。
//!
//! 崩溃安全：**dispatch 成功且 autosave 落盘后**才推进 cursor。若「存档成功、cursor
//! 未推进」时进程挂掉，重启会重放同一行——`external_id` 是 Mod 侧幂等键，重放返回
//! 已有需求、不会重复入队（回执叙事 `【需求重放】`）。坏 JSON 行跳过并前移 cursor
//! （否则永久阻塞）。局已收口（stage=done）后仍有未消费行 → 非零退出并提示开新局，
//! 不静默丢弃。
//!
//! 回执加性键 `inbox`：`{applied, skipped, remaining}`（本次 drain 的账）。
//!
//! 交互模式的推进方式由 `--drive` 决定（**宿主不解释 Mod 的 `play.mode`**；
//! `config.set play.mode=…` 仍可用，但只影响 Mod 自己的叙事）：
//!
//! | `--drive` | 行为 | 退出 |
//! | --- | --- | --- |
//! | `stdin`（缺省） | 读 stdin：数字选 / `event <名称> [JSON]` / `s` / `l` / `i` / `config` / `q` | `q` / 输入结束 |
//! | `cmd:<路径>` | 不读 stdin：每个待选动作交给决策命令（stdout 单行 `N` 或 `HUMAN[:理由]`）；到结局自动结算 | 结算码；HUMAN / 异常 = 0 |
//! | `none` | 只打印当前回执就退出（不推进、不读 stdin） | 0 |
//!
//! 自动推进的失控防护：步数硬顶 + 每条需求（异步补入的）额外步数 + 连续无进展
//! 判停——阈值读 `<mod>/data/config.json` 的 `drive` 段（[`DriveLimits`]，
//! 缺失 / 非法回落 [`DEFAULT_MAX_STEPS`] / [`DEFAULT_STEPS_PER_REQUIREMENT`] /
//! [`DEFAULT_STALL_LIMIT`]，不因配置问题报错中断局）。合法多需求局不会被误杀，
//! 真循环仍会停；派发失败 / 无 pendingChoice 且未到结局 → 可读错误并非零退出
//! （绝不静默死循环）。
//! 单发模式不受影响：`--event` 一次只派发一个事件，不自动连跑（Agent 的调用语义）。
//!
//! run 目录产物（`<runs-root>/<run-id>/`，默认 `./runs/<run-id>/`）：
//!
//! | 文件 | 内容 |
//! | --- | --- |
//! | `autosave.json` | 最新存档（= `save` 信封内嵌 save 对象的原样字节，可直接作 `load` payload） |
//! | `events.jsonl` | 每派发一行：`{seq, event, payload, ok, narrative_count, timestamp, ending, ending_line?}` |
//! | `run.json` | 宿主侧运行元信息：`{run_id, mod, mod_signature, started_at, updated_at}`（结算取 `mod`；`mod_signature` 供跨进程热载比对） |
//! | `settlement.json` | 局末结算报告（仅 `--settle` 写出） |
//! | `inbox.jsonl` | 异步需求队列（`--requirement` 只追加；运行侧消费） |
//! | `inbox.cursor` | 已消费行数 `{"consumed": N}`（崩溃安全面：dispatch + 存档后才前移） |
//!
//! 单发回执契约（stdout 单行 JSON；Agent 的解析面）：
//!
//! ```json
//! {"ok":true,"event":"event.start","narrative":["…"],"pendingChoice":{"prompt":"…","options":["…"]},
//!  "autosave":"runs/my-task/autosave.json","seq":0,"revision":5}
//! ```
//!
//! - `pendingChoice` 原样透传内核信封（动作白名单的现场呈现；`null` = 无待决
//!   选择）；越权动作的 error 信封原样落 `error`（码 + 文案），并补一次 `state`
//!   回填当前 `pendingChoice`，让 Agent 出错后仍能接着走；
//! - 加性键：`warnings`（内核 D1 警告 + 宿主侧提示）、`hotReload`
//!   （本次派发前的热载结果，形如 `{"reloaded":true,"source":"resume"}`）、
//!   `ending`（本步叙事出现结局行）；
//! - 退出码：0 = ok；1 = 内核错误信封或装载失败；2 = 用法错误。
//!
//! 热载（`--watch` 交互模式 / 单发模式每次派发前）：比对 mod 目录监视文件的
//! mtime + 字节数，变化即「现态存档 → 按新包重装载 → 回灌存档」——状态无损
//! 续局（handler 是代码不入档，重装载即换新逻辑）。重装载失败则**沿用旧 mod
//! 继续**（回执 `hotReload.reloaded=false` + warnings，不中断局）。
//!
//! 单发模式每次都是新进程、开局即按磁盘现状装载，所以「热载」的可观测面在
//! **续局比对**：`run.json` 里记着上一步的 `mod_signature`，`--resume` 时
//! 与现状不同即回执 `hotReload.source = "resume"`——Agent 由此知道自己的 mod
//! 改动已经生效。进程内查盘（`source = "watch"`）主要服务交互模式。
//!
//! 结算口径（`--settle`）：`ok` = 事件日志非空 ∧ 无失败步 ∧ 出现结局叙事行；
//! 退出码与 `ok` 同值（与 `playtest` 约定一致）。`final_state` 从 autosave 的
//! `kv` 面摘出结局 / 评分 / 经验（缺失即 `null`），并附完整 kv 与规模面。

use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{exit, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value as Json};
use ygg_sandbox::driver::KernelFacade;

// ============================================================================
// §常量 文件名 / 契约字面量（与 adventure.rs / playtest.rs / client-web 同源）
// ============================================================================

/// 结局叙事行前缀（出现即本局收口；结算判据，与 playtest 同源同值）。
const ENDING_PREFIX: &str = "【结局";
/// 事件 payload 缺省值（内核口径：事件路径 payload 必填且须为 JSON 标量）。
const DEFAULT_PAYLOAD: &str = "0";
/// 新局自动派发的入口事件名（交互模式；与 test-adventure / seed mod 约定一致）。
const ENTRY_EVENT: &str = "event.start";
/// 选择事件的规范名（与内核保留路由同值）。
const CHOICE_EVENT: &str = "ygg.choice_selected";
/// 自动推进阈值的内置兜底：`<mod>/<data/config.json>` 的 `drive` 段缺失 / 非法时用它。
/// 步数 = 基础硬顶（正常一局 9 步内收口；上限是死循环的硬防御）。
const DEFAULT_MAX_STEPS: usize = 64;
/// 每条需求（异步补入的）额外放宽的自动步数：三个 job + 两个门禁 + 余量。
const DEFAULT_STEPS_PER_REQUIREMENT: usize = 16;
/// 连续「无进展」步数兜底（pendingChoice 指纹不变 = 绕圈；死循环的第二道闸）。
const DEFAULT_STALL_LIMIT: usize = 12;
/// 外部决策命令的最长执行时间；任何异常都 fail closed，绝不回退到固定序号。
const DECIDE_TIMEOUT: Duration = Duration::from_secs(5);
/// 决策命令轮询间隔（std-only：spawn + try_wait + 超时 kill/wait）。
const DECIDE_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Mod 自报摘要的只读动作名（Mod 侧 `ygg.on("agent.summary")`）。
///
/// 宿主判定「何时收口 / 有无冲突 / 有无待验收提案」只认这一条动作的回执，
/// **绝不读 autosave 的 Mod 私有 KV 键**——旧 Mod 没有该处理器时回执无摘要行，
/// 宿主判为「未知」并安全停等，不回退到读 KV。
const SUMMARY_EVENT: &str = "agent.summary";
/// 摘要行前缀（Mod 与宿主之间唯一的字符串契约：前缀 + 单行 JSON）。
const SUMMARY_PREFIX: &str = "【摘要】";

const AUTOSAVE_FILE: &str = "autosave.json";
const EVENTS_FILE: &str = "events.jsonl";
const META_FILE: &str = "run.json";
const SETTLEMENT_FILE: &str = "settlement.json";
const INBOX_FILE: &str = "inbox.jsonl";
const INBOX_CURSOR_FILE: &str = "inbox.cursor";
const DECIDE_INPUT_FILE: &str = "decide-input.json";

/// Mod 侧拒绝的叙事前缀（契约面：`agent-seed` 系 Mod 用叙事而不是错误码表达
/// 「动作被拒 / 载荷被拒 / 需求待排」，内核信封仍是 ok——宿主必须识别，否则会把
/// 被拒的需求误记成已入队）。
const REJECTION_PREFIXES: [&str; 3] = ["【动作被拒】", "【载荷被拒】", "【需求待排】"];
/// Mod 侧幂等重放的叙事前缀（同 external_id 重放：**算成功消费**，只是没新增）。
const REPLAY_PREFIX: &str = "【需求重放】";

const MOD_SOURCE_FILE: &str = "mod.lua";
const MANIFEST_FILE: &str = "manifest.json";
const PACK_FILE: &str = "pack.json";

fn main() {
    let args = parse_args();
    let code = if args.inbox.is_some() {
        run_inbox_append(&args)
    } else if args.settle {
        run_settle(&args, false)
    } else if args.event.is_some() {
        run_single(&args)
    } else {
        run_interactive(&args)
    };
    exit(code);
}

// ============================================================================
// §参数 命令行解析（std-only 手写；用法错误 = 退出码 2）
// ============================================================================

/// 异步 inbox 的操作种类（JSONL `kind` 字段；缺省 = add，兼容旧行）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum InboxKind {
    Add,
    Amend,
    Split,
    Merge,
    Conflict,
    Resolve,
    Proposal,
}

impl InboxKind {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "add" => Some(Self::Add),
            "amend" => Some(Self::Amend),
            "split" => Some(Self::Split),
            "merge" => Some(Self::Merge),
            "conflict" => Some(Self::Conflict),
            "resolve" => Some(Self::Resolve),
            "proposal" => Some(Self::Proposal),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Amend => "amend",
            Self::Split => "split",
            Self::Merge => "merge",
            Self::Conflict => "conflict",
            Self::Resolve => "resolve",
            Self::Proposal => "proposal",
        }
    }

    /// kind → 内核事件名（drain 时 dispatch）。
    fn event(self) -> &'static str {
        match self {
            Self::Add => "requirement.add",
            Self::Amend => "requirement.amend",
            Self::Split => "requirement.split",
            Self::Merge => "requirement.merge",
            Self::Conflict => "requirement.conflict",
            Self::Resolve => "requirement.resolve",
            Self::Proposal => "proposal.submit",
        }
    }
}

struct Args {
    mod_dir: Option<PathBuf>,
    run_id: String,
    runs_root: PathBuf,
    resume: bool,
    settle: bool,
    watch: bool,
    event: Option<String>,
    payload: String,
    /// 交互模式推方式（`--drive`；缺省 stdin）。
    drive: Drive,
    /// 异步 inbox 操作（只追 inbox.jsonl 后退出，不读写 autosave）。
    inbox: Option<(InboxKind, String)>,
}

fn parse_args() -> Args {
    let mut mod_dir: Option<PathBuf> = None;
    let mut run_id: Option<String> = None;
    let mut runs_root = PathBuf::from("runs");
    let mut resume = false;
    let mut settle = false;
    let mut watch = false;
    let mut event: Option<String> = None;
    let mut payload = DEFAULT_PAYLOAD.to_owned();
    let mut drive = Drive::Stdin;
    let mut inbox: Option<(InboxKind, String)> = None;

    // 写 inbox 的六个参数共享同一套「只能给一个」的校验。
    fn set_inbox(inbox: &mut Option<(InboxKind, String)>, kind: InboxKind, value: String) {
        if inbox.is_some() {
            usage_error("一次只能提交一个 inbox 操作");
        }
        *inbox = Some((kind, value));
    }

    let mut iter = std::env::args().skip(1);
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--mod" => mod_dir = Some(PathBuf::from(next_value(&mut iter, "--mod"))),
            "--run-id" => run_id = Some(next_value(&mut iter, "--run-id")),
            "--runs-root" => runs_root = PathBuf::from(next_value(&mut iter, "--runs-root")),
            "--resume" => resume = true,
            "--settle" => settle = true,
            "--watch" => watch = true,
            "--event" => event = Some(next_value(&mut iter, "--event")),
            "--payload" => payload = next_value(&mut iter, "--payload"),
            "--drive" => {
                let text = next_value(&mut iter, "--drive");
                drive = match parse_drive(&text) {
                    Ok(drive) => drive,
                    Err(message) => usage_error(&message),
                };
            }
            "--requirement" => set_inbox(
                &mut inbox,
                InboxKind::Add,
                next_value(&mut iter, "--requirement"),
            ),
            "--amend" => set_inbox(
                &mut inbox,
                InboxKind::Amend,
                next_value(&mut iter, "--amend"),
            ),
            "--split" => set_inbox(
                &mut inbox,
                InboxKind::Split,
                next_value(&mut iter, "--split"),
            ),
            "--merge" => set_inbox(
                &mut inbox,
                InboxKind::Merge,
                next_value(&mut iter, "--merge"),
            ),
            "--conflict" => set_inbox(
                &mut inbox,
                InboxKind::Conflict,
                next_value(&mut iter, "--conflict"),
            ),
            "--resolve" => set_inbox(
                &mut inbox,
                InboxKind::Resolve,
                next_value(&mut iter, "--resolve"),
            ),
            "--proposal" => set_inbox(
                &mut inbox,
                InboxKind::Proposal,
                next_value(&mut iter, "--proposal"),
            ),
            "-h" | "--help" => usage(0),
            other => usage_error(&format!("未知参数 `{other}`")),
        }
    }

    let Some(run_id) = run_id else {
        usage_error("缺少 --run-id <标识>");
    };
    if !is_valid_run_id(&run_id) {
        usage_error("--run-id 只允许字母数字与 `-` `_` `.`（不得含路径分隔符）");
    }
    if settle && event.is_some() {
        usage_error("--settle 与 --event 不能同时使用");
    }
    if let Some((kind, text)) = &inbox {
        if settle || event.is_some() {
            usage_error(
                "inbox 操作（--requirement / --proposal / …）与 --event / --settle 不能同时使用",
            );
        }
        if text.trim().is_empty() {
            usage_error(&format!("--{} 需要非空载荷", kind.label()));
        }
    } else if !settle && mod_dir.is_none() {
        usage_error("缺少 --mod <mod目录>（仅 --settle / inbox 操作可省略）");
    }
    if let Some(name) = &event {
        if name.trim().is_empty() {
            usage_error("--event 需要非空事件名");
        }
    }

    Args {
        mod_dir,
        run_id,
        runs_root,
        resume,
        settle,
        watch,
        event,
        payload,
        drive,
        inbox,
    }
}

/// 取旗标后一位的值（缺失即用法错误）。
fn next_value(iter: &mut impl Iterator<Item = String>, flag: &str) -> String {
    match iter.next() {
        Some(value) => value,
        None => usage_error(&format!("{flag} 缺少取值")),
    }
}

/// run-id 白名单：字母数字 + `-` `_` `.`（路径穿越与目录分隔符一律拒绝）。
fn is_valid_run_id(run_id: &str) -> bool {
    !run_id.is_empty()
        && run_id != "."
        && run_id != ".."
        && run_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn usage(code: i32) -> ! {
    let text = "\
用法:
  agent --mod <mod目录> --run-id <标识> [--resume] [--watch] [--drive <方式>]
                                                                交互模式
  agent --mod <mod目录> --run-id <标识> --event <名称> [--payload <JSON>] [--resume]
                                                                单发模式（Agent 脚本化）
  agent --run-id <标识> --settle                                局末结算
  agent --run-id <标识> [--runs-root <目录>] --requirement '标题|描述'
                                                                异步补需求（只追 inbox.jsonl）

选项:
  --mod <目录>       Mod 目录（五件套：pack.json / manifest.json / mod.lua /
                     schemas/*.json / data/*.json；--settle / --requirement 可省略）
  --run-id <标识>    本局标识：产物落 <runs-root>/<标识>/（必填）
  --runs-root <目录> runs 根目录（默认 ./runs）
  --resume           从 autosave.json 续局（缺存档则按新局开局）
  --watch            交互模式下每步检查 mod 目录 mtime 变化并热载
  --event <名称>     单发模式：派发一个事件后退出（stdout 单行 JSON 回执）
  --payload <JSON>   事件 payload（默认 \"0\"；必须 JSON 标量）
  --drive <方式>     交互模式推方式（默认 stdin；宿主不替你做决策）：
                       stdin      读 stdin：数字=选择 / event <名称> [JSON] / s / l / i / config / q
                       cmd:<路径> 不读 stdin：每个待选动作交给决策命令（stdout 单行 `N` 或
                                  `HUMAN[:理由]`）；HUMAN / 超时 / 非零退出一律安全暂停并退出 0
                                  （环境变量 YGG_DECIDE_CMD 可临时覆盖路径）
                       none       只打印当前回执就退出（不推进、不读 stdin）
  --requirement <串> 只把 '标题|描述' 追加到 <run 目录>/inbox.jsonl 后退出；
                     不读写 autosave，可在别一进程跑本局时安全并发调用
  --settle           读 autosave + events.jsonl 写 settlement.json
  -h, --help         本帮助

自动推进的失控防护：基础步数 + 每条需求额外步数 + 连续无进展判停；阙值读
  <mod>/data/config.json 的 drive 段（缺失 / 非法 → 代码内兜底值，不中断局）。

异步需求（inbox）：每个自动动作前、stdin 每次提示前、单发每次派发前都会消费
  inbox.jsonl 的新行并 dispatch requirement.add；dispatch 成功且存档后才前移 cursor，
  重放靠 Mod 侧 external_id 幂等去重。局已收口后仍有未消费行 → 非零退出。";
    if code == 0 {
        println!("{text}");
    } else {
        eprintln!("{text}");
    }
    exit(code);
}

fn usage_error(message: &str) -> ! {
    eprintln!("error: {message}");
    eprintln!();
    usage(2)
}

// ============================================================================
// §驱动 交互模式的推进方式（宿主只提供通道，**不替决策**）
//
// 推进策略不在宿主里：宿主要么等 stdin，要么把当前回执交给外部决策命令，
// 要么什么都不做只回显。Mod 的 `play.mode` 仍由 Mod 在叙事层使用（`config.set`
// 动作照旧可用），但 **host 不再读它、也不再解释它**。
// ============================================================================

/// 交互模式的推进方式。
#[derive(Clone, Debug, PartialEq, Eq)]
enum Drive {
    /// 缺省：读 stdin，每一步推进都由人输入（数字 / event / s / l / i / config / q）。
    Stdin,
    /// 不读 stdin：每个待选动作交给外部决策命令（stdout 单行 `N` 或 `HUMAN[:理由]`）。
    Cmd(PathBuf),
    /// 只打印当前回执就退出（不推进、不读 stdin）。
    None,
}

impl Drive {
    /// 人读标签（启动头 / `config` 命令共用）。
    fn label(&self) -> String {
        match self {
            Self::Stdin => "stdin（你逐步输入）".to_owned(),
            Self::Cmd(path) => format!("cmd:{}（外部决策命令自动推进）", path.display()),
            Self::None => "none（只打印当前回执，不推进）".to_owned(),
        }
    }

    fn is_cmd(&self) -> bool {
        matches!(self, Self::Cmd(_))
    }
}

/// `--drive` 取值解析：`stdin` / `cmd:<路径>` / `none`（其余一律用法错误）。
fn parse_drive(text: &str) -> Result<Drive, String> {
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

/// 自动推进阈值（读 `<mod>/<config_rel>` 的 `drive` 段；缺失 / 非法逐字段回落兜底）。
///
/// 阈值是**配置**而不是代码常量，因为「几步算多」取决于 Mod 的流程长度；
/// 但配置是便利面，读不懂的字段一律回落——绝不因配置问题打断一局。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DriveLimits {
    /// 基础步数硬顶。
    max_steps: usize,
    /// 连续无进展判停阈值。
    stall_limit: usize,
    /// 每条需求额外放宽的步数。
    steps_per_requirement: usize,
}

impl Default for DriveLimits {
    fn default() -> Self {
        Self {
            max_steps: DEFAULT_MAX_STEPS,
            stall_limit: DEFAULT_STALL_LIMIT,
            steps_per_requirement: DEFAULT_STEPS_PER_REQUIREMENT,
        }
    }
}

impl DriveLimits {
    /// 从 Mod 静态配置读 `drive` 段（`maxSteps` / `stallLimit` / `stepsPerRequirement`）。
    /// 缺失、非整数、负数、0 一律取该项兜底值；`stepsPerRequirement` 允许 0。
    fn from_config(config: &Json) -> Self {
        let fallback = Self::default();
        let positive = |key: &str, default: usize| {
            let value = config["drive"][key]
                .as_u64()
                .map_or(default, |n| n as usize);
            if value == 0 {
                default
            } else {
                value
            }
        };
        Self {
            max_steps: positive("maxSteps", fallback.max_steps),
            stall_limit: positive("stallLimit", fallback.stall_limit),
            steps_per_requirement: positive("stepsPerRequirement", fallback.steps_per_requirement),
        }
    }

    /// 本局步数预算：基础硬顶 + 每条需求额外步数（饱和运算，不溢出）。
    fn step_limit(&self, requirement_count: usize) -> usize {
        self.max_steps
            .saturating_add(self.steps_per_requirement.saturating_mul(requirement_count))
    }
}

/// 静态配置的包内路径（pack.json 条目 `ygg:agent/config`；缺声明时按约定回落）。
fn agent_config_path(dir: &Path) -> String {
    let fallback = "data/config.json".to_owned();
    let Some(pack) = fs::read_to_string(dir.join(PACK_FILE))
        .ok()
        .and_then(|text| serde_json::from_str::<Json>(&text).ok())
    else {
        return fallback;
    };
    pack["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| entry["id"].as_str() == Some("ygg:agent/config"))
        .and_then(|entry| entry["path"].as_str())
        .map_or(fallback, str::to_owned)
}

// ============================================================================
// §Mod 目录 五件套读盘与装载（运行期读盘——agent 必须支持任意 mod 目录，
//             不能像 adventure.rs 那样 include_str! 编译期固化）
// ============================================================================

struct ModBundle {
    mod_source: String,
    manifest_bytes: Vec<u8>,
    schemas: Vec<(String, String)>,
    resources: Vec<(String, Vec<u8>)>,
}

impl ModBundle {
    /// 读盘装载五件套（pack.json 声明面 → 装载参数；与
    /// `src/playtest.rs::read_mod_bundle` 同形，example 侧独立一份以便热载重读）。
    fn load(dir: &Path) -> Result<Self, String> {
        if !dir.is_dir() {
            return Err(format!("mod 目录不存在或不可读：{}", dir.display()));
        }
        let pack_text = fs::read_to_string(dir.join(PACK_FILE))
            .map_err(|err| format!("读取 {}/{PACK_FILE} 失败：{err}", dir.display()))?;
        let pack: Json = serde_json::from_str(&pack_text)
            .map_err(|err| format!("{}/{PACK_FILE} 不是合法 JSON：{err}", dir.display()))?;
        let entries = pack["entries"]
            .as_array()
            .ok_or_else(|| format!("{}/{PACK_FILE} 缺少 entries 数组", dir.display()))?;
        let schema_files = pack["schemaFiles"]
            .as_array()
            .ok_or_else(|| format!("{}/{PACK_FILE} 缺少 schemaFiles 数组", dir.display()))?;

        let mod_source = fs::read_to_string(dir.join(MOD_SOURCE_FILE))
            .map_err(|err| format!("读取 {}/{MOD_SOURCE_FILE} 失败：{err}", dir.display()))?;
        let manifest_bytes = fs::read(dir.join(MANIFEST_FILE))
            .map_err(|err| format!("读取 {}/{MANIFEST_FILE} 失败：{err}", dir.display()))?;

        // schema 文本按文件内 `$id` 注册（与 pack.rs / playtest 同口径：
        // schema 不进 manifest，装载方按 id 供给）。
        let mut schemas = Vec::new();
        for file in schema_files {
            let rel = file
                .as_str()
                .ok_or_else(|| "schemaFiles 必须是字符串数组".to_owned())?;
            let text = fs::read_to_string(dir.join(rel))
                .map_err(|err| format!("读取 schema 文件 {rel} 失败：{err}"))?;
            let parsed: Json = serde_json::from_str(&text)
                .map_err(|err| format!("schema 文件 {rel} 不是合法 JSON：{err}"))?;
            let id = parsed["$id"]
                .as_str()
                .ok_or_else(|| format!("schema 文件 {rel} 缺少 $id"))?
                .to_owned();
            schemas.push((id, text));
        }

        let mut resources = Vec::new();
        for entry in entries {
            let rel = entry["path"]
                .as_str()
                .ok_or_else(|| "entries[].path 必须是字符串".to_owned())?;
            let bytes =
                fs::read(dir.join(rel)).map_err(|err| format!("读取资源文件 {rel} 失败：{err}"))?;
            resources.push((rel.to_owned(), bytes));
        }

        Ok(Self {
            mod_source,
            manifest_bytes,
            schemas,
            resources,
        })
    }

    /// 按本包构造驱动（资源表 fail-fast 先于任何 Lua 状态）。
    fn kernel(&self) -> Result<KernelFacade, String> {
        KernelFacade::with_resources(
            &self.mod_source,
            &self.manifest_bytes,
            &self.schemas,
            &self.resources,
        )
        .map_err(|err| format!("内核装载失败：{err}"))
    }
}

/// 单文件监视快照（热载变更判据：mtime + 字节数）。
///
/// mtime 是**宿主侧**运行时信息，不涉内核确定性面（内核只消费事件与状态）；
/// 时钟 / 文件系统本就是宿主管辖，与「内核零 IO」纪律不冲突。
#[derive(Clone, PartialEq, Eq)]
struct FileStamp {
    path: String,
    len: u64,
    modified: Option<SystemTime>,
}

/// 监视文件清单：五件套 + pack.json 声明的每个条目 / schema 文件
/// （pack.json 本身损坏时退化为固定三件，仍能检测修复动作）。
fn watched_files(dir: &Path) -> Vec<String> {
    let mut files = vec![
        MOD_SOURCE_FILE.to_owned(),
        MANIFEST_FILE.to_owned(),
        PACK_FILE.to_owned(),
    ];
    if let Ok(text) = fs::read_to_string(dir.join(PACK_FILE)) {
        if let Ok(pack) = serde_json::from_str::<Json>(&text) {
            for entry in pack["entries"].as_array().into_iter().flatten() {
                if let Some(path) = entry["path"].as_str() {
                    files.push(path.to_owned());
                }
            }
            for file in pack["schemaFiles"].as_array().into_iter().flatten() {
                if let Some(path) = file.as_str() {
                    files.push(path.to_owned());
                }
            }
        }
    }
    files.sort();
    files.dedup();
    files
}

/// mod 目录现状快照（与 [`Session::signature`] 逐字段比对判定「变没变」）。
fn mod_signature(dir: &Path) -> Vec<FileStamp> {
    watched_files(dir)
        .into_iter()
        .map(|rel| {
            let meta = fs::metadata(dir.join(&rel)).ok();
            FileStamp {
                path: rel,
                len: meta.as_ref().map_or(0, fs::Metadata::len),
                modified: meta.and_then(|meta| meta.modified().ok()),
            }
        })
        .collect()
}

// ============================================================================
// §摘要 Mod 回执标准字段（宿主唯一的语义判定面）
//
// 反耦合的落点：宿主不再认识 `ending` / `fsm.stage` / `verify.passed` / `prop.*` /
// `req.*` 这些 **Mod 私有 KV 键**，只读 Mod **自己声明**的一行摘要（前缀 +
// 单行 JSON）。换 Mod（或 Mod 改内部键名）不需要改宿主。
//
// 缺摘要（旧 Mod / 摘要行坏 JSON）→ [`Summary::unknown`]：各判定返回安全值
// （0 / false / 空），宿主**不回退到读 KV**，驱动侧据此安全停等。
// ============================================================================

/// 一条未解决冲突（Mod 自报：需求 id / 对侧 / 说明）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ConflictNote {
    id: String,
    peer: String,
    note: String,
}

impl ConflictNote {
    /// 人读形态（`r2 ⇄ r3：说明`）。
    fn label(&self) -> String {
        format!("{} ⇄ {}：{}", self.id, self.peer, self.note)
    }
}

/// 一个在跑但还没有提案的作业（Mod 自报：需求 id / 修订号 / 作业阶段）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct AwaitingJob {
    req: String,
    rev: u64,
    stage: String,
}

/// Mod 回执标准字段快照。
///
/// `available = false` = Mod 没给摘要（旧 Mod / 坏 JSON），此时**所有**判定
/// 返回保守值；驱动侧必须停等而不是猜。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Summary {
    available: bool,
    /// 当前阶段（仅展示用——宿主不按它分支）。
    stage: String,
    /// 本局已到结局（Mod 侧收口标记）。
    ending: bool,
    /// review 阶段的门禁自检已过。
    verify_passed: bool,
    /// **下一步就是收口动作**（Mod 用「菜单首项是否 review.note」判定）：
    /// 宿主据此在派发前先进 closing，把最后一次 drain 排在收口之前。
    next_ends_run: bool,
    /// 待验收提案数。
    pending_proposals: usize,
    /// 已入队需求总数（步数预算按它放宽）。
    requirement_count: usize,
    /// 未解决冲突（非空 = 用户门禁，自动推进必须停下）。
    conflicts: Vec<ConflictNote>,
    /// 在跑但无提案的作业（非空 = 等外部执行方，宿主不造内容）。
    awaiting: Option<AwaitingJob>,
}

impl Summary {
    /// 未知摘要：安全值（`available=false`）。
    fn unknown() -> Self {
        Self::default()
    }

    /// 本局是否已收口：`None` = Mod 未提供摘要（不得猜）。
    fn ending(&self) -> Option<bool> {
        if self.available {
            Some(self.ending)
        } else {
            None
        }
    }

    /// 从回执叙事里取摘要行（首个前缀行的 JSON 对象）。
    fn from_narrative(lines: &[String]) -> Self {
        for line in lines {
            let Some(rest) = line.strip_prefix(SUMMARY_PREFIX) else {
                continue;
            };
            return match serde_json::from_str::<Json>(rest) {
                Ok(value) if value.is_object() => Self::from_json(&value),
                _ => Self::unknown(),
            };
        }
        Self::unknown()
    }

    /// JSON → 摘要（缺字段一律取保守默认值；只有「是对象」才算 available）。
    fn from_json(value: &Json) -> Self {
        let count = |key: &str| value[key].as_u64().map_or(0, |n| n as usize);
        Self {
            available: true,
            stage: value["stage"].as_str().unwrap_or("").to_owned(),
            ending: value["ending"] == Json::Bool(true),
            verify_passed: value["verifyPassed"] == Json::Bool(true),
            next_ends_run: value["nextEndsRun"] == Json::Bool(true),
            pending_proposals: count("pendingProposals"),
            requirement_count: count("requirementCount"),
            conflicts: value["conflicts"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|item| ConflictNote {
                    id: item["id"].as_str().unwrap_or("").to_owned(),
                    peer: item["peer"].as_str().unwrap_or("?").to_owned(),
                    note: item["note"].as_str().unwrap_or("").to_owned(),
                })
                .filter(|item| !item.note.trim().is_empty())
                .collect(),
            awaiting: value["awaiting"].as_object().map(|item| AwaitingJob {
                req: item["req"].as_str().unwrap_or("").to_owned(),
                rev: item["rev"].as_u64().unwrap_or(1),
                stage: item["stage"].as_str().unwrap_or("").to_owned(),
            }),
        }
    }
}

// ============================================================================
// §Session 一局的宿主侧状态：驱动 + run 目录 + 事件序号 + 热载签名
// ============================================================================

/// 热载事件（回执 `hotReload` 键；`reloaded=false` 时旧 mod 仍在跑，局不中断）。
struct Reload {
    /// 是否成功换上新 mod（false = 沿用旧 mod 继续）。
    reloaded: bool,
    /// 触发面：`resume`（开局按新包装载 + 回灌状态）/ `watch`（派发前查盘）。
    source: &'static str,
    /// 失败原因（`reloaded=false` 时非空）。
    message: Option<String>,
}

impl Reload {
    fn to_json(&self) -> Json {
        let mut value = json!({"reloaded": self.reloaded, "source": self.source});
        if let Some(message) = &self.message {
            value["error"] = json!(message);
        }
        value
    }
}

/// 一次 inbox 消费的账（回执 `inbox` 键 + 自动推进 / stdin 提示共用）。
#[derive(Default)]
struct InboxDrain {
    /// 本次成功消费的需求条数（cursor 已前移；含幂等重放）。
    applied: usize,
    /// 其中幂等重放（同 external_id 已存在，Mod 侧去重）的条数。
    replayed: usize,
    /// 坏行跳过条数（cursor 已前移，否则永久阻塞）。
    skipped: usize,
    /// 未消费条数（dispatch 未成功 → cursor 不前移，重启可重放）。
    remaining: usize,
    /// 最近一次成功 dispatch 的回执（自动推进据此刷新 pendingChoice）。
    last: Option<Json>,
    /// 阻塞原因（`remaining > 0` 时非空）。
    error: Option<String>,
}

impl InboxDrain {
    fn touched(&self) -> bool {
        self.applied > 0 || self.skipped > 0 || self.remaining > 0
    }

    fn to_json(&self) -> Json {
        json!({
            "applied": self.applied,
            "replayed": self.replayed,
            "skipped": self.skipped,
            "remaining": self.remaining,
        })
    }
}

struct Session {
    run_id: String,
    mod_dir: PathBuf,
    /// 静态配置资源的包内路径（`play.mode` 的静态面；来自 pack.json 声明）。
    config_rel: String,
    mod_label: String,
    run_dir: PathBuf,
    autosave_path: PathBuf,
    events_path: PathBuf,
    meta_path: PathBuf,
    /// 异步需求队列（`--requirement` 只追加；本进程只消费）。
    inbox_path: PathBuf,
    /// 消费位移（`{consumed: N}`，按非空行数；dispatch + 存档成功后才前移）。
    inbox_cursor_path: PathBuf,
    kernel: KernelFacade,
    /// 下一条事件记录的序号（续局 = 既有行数，保证 seq 单调且可与日志对账）。
    seq: u64,
    /// 装载时的 mod 目录签名（热载比对基线；每次成功/失败尝试后前移）。
    signature: Vec<FileStamp>,
    /// 本次回执待携带的宿主侧提示（热载 / 自动存档 / 重置等）。
    notices: Vec<String>,
    /// 本次派发前的热载结果（一次性；派发收口后清空）。
    reload: Option<Reload>,
    /// 回执标准字段缓存（每次真实派发 / 读档 / 热载后失效）。
    summary_cache: Option<Summary>,
    /// 是否由 `--resume` 成功续局（交互模式据此决定回显而非推进）。
    resumed: bool,
}

impl Session {
    /// 开局：读盘装载 → 建 run 目录 → 续局或重置 → 定 seq → 落元信息。
    fn open(args: &Args) -> Result<Self, String> {
        let Some(mod_dir) = args.mod_dir.clone() else {
            return Err("缺少 --mod <mod目录>".to_owned());
        };
        let run_dir = args.runs_root.join(&args.run_id);

        // 先装载（失败即退出，run 目录不留空壳），再建 run 目录接管产物。
        let bundle = ModBundle::load(&mod_dir)?;
        let kernel = bundle.kernel()?;
        fs::create_dir_all(&run_dir)
            .map_err(|err| format!("创建 run 目录 {} 失败：{err}", run_dir.display()))?;
        // 装载即取签名：开局装载永远读的是磁盘现状，之后的差异才是热载触发。
        let signature = mod_signature(&mod_dir);
        // 上一局落盘的签名（用于报告「续局时 mod 已被改过」——单发模式下
        // 这就是热载的可观测面：新包已装载、状态已回灌）。
        let previous_signature = read_meta_signature(&run_dir.join(META_FILE));

        let mut session = Self {
            run_id: args.run_id.clone(),
            mod_dir: mod_dir.clone(),
            config_rel: agent_config_path(&mod_dir),
            mod_label: mod_dir.display().to_string(),
            autosave_path: run_dir.join(AUTOSAVE_FILE),
            events_path: run_dir.join(EVENTS_FILE),
            meta_path: run_dir.join(META_FILE),
            inbox_path: run_dir.join(INBOX_FILE),
            inbox_cursor_path: run_dir.join(INBOX_CURSOR_FILE),
            run_dir,
            kernel,
            seq: 0,
            signature,
            notices: Vec::new(),
            reload: None,
            summary_cache: None,
            resumed: false,
        };

        if args.resume {
            match session.load_autosave() {
                Ok(()) => session.resumed = true,
                Err(message) => {
                    // 续局失败按新局开局：状态与日志必须同代，混代证据比丢证据更坏。
                    session.notices.push(format!("{message}；按新局开局"));
                    session.reset_artifacts();
                }
            }
        } else if session.reset_artifacts() {
            session
                .notices
                .push(format!("run `{}` 已有旧产物，按新局重置", session.run_id));
        }
        session.seq = session.count_events();
        // 续局成功且 mod 自上次派发起变过：新包已生效（状态无损），回执标记。
        if session.resumed
            && previous_signature
                .as_deref()
                .is_some_and(|previous| previous != session.signature_label())
        {
            session.reload = Some(Reload {
                reloaded: true,
                source: "resume",
                message: None,
            });
        }
        session.write_meta();
        // 崩溃恢复：上次若在 closing 中挂掉（owner 心跳过期）且本局未 done，收回到 running。
        // 只有真的处在 closing 才去问 Mod 的摘要（省一次派发）；摘要缺失（旧 Mod）
        // 时不动协议状态——猜错方向比不动更坏。
        if read_coordination(&session.run_dir)["phase"].as_str() == Some(PHASE_CLOSING) {
            if let Some(ended) = session.summary().ending() {
                if let Some(notice) = recover_stale_closing(&session.run_dir, ended) {
                    session.notices.push(notice);
                }
            }
        }
        Ok(session)
    }

    /// 新局重置：截断事件日志 + 移除 autosave（返回值 = 是否真的清过东西）。
    fn reset_artifacts(&mut self) -> bool {
        let had = self.events_path.exists() || self.autosave_path.exists();
        if let Err(err) = fs::write(&self.events_path, "") {
            self.notices
                .push(format!("重置事件日志失败：{err}（将续写既有日志）"));
        }
        if self.autosave_path.exists() {
            if let Err(err) = fs::remove_file(&self.autosave_path) {
                self.notices.push(format!("移除旧 autosave 失败：{err}"));
            }
        }
        had
    }

    /// 事件日志既有条数（续局 seq 起点）。
    fn count_events(&self) -> u64 {
        match fs::read_to_string(&self.events_path) {
            Ok(text) => text.lines().filter(|line| !line.trim().is_empty()).count() as u64,
            Err(_) => 0,
        }
    }

    /// mod 目录签名的人读串（落 run.json；跨进程比对「mod 变没变」）。
    fn signature_label(&self) -> String {
        self.signature
            .iter()
            .map(|stamp| {
                let modified = stamp
                    .modified
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |elapsed| elapsed.as_nanos());
                format!("{}:{}:{modified}", stamp.path, stamp.len)
            })
            .collect::<Vec<_>>()
            .join("|")
    }

    /// 运行元信息（结算取 `mod` 标签；`started_at` 跨续局保留）。
    fn write_meta(&self) {
        let started_at = read_meta(&self.meta_path)
            .and_then(|meta| meta["started_at"].as_u64())
            .unwrap_or_else(now_secs);
        let meta = json!({
            "run_id": self.run_id,
            "mod": self.mod_label,
            "mod_signature": self.signature_label(),
            "started_at": started_at,
            "updated_at": now_secs(),
        });
        if let Err(err) = write_json_pretty(&self.meta_path, &meta) {
            eprintln!("[警告] {err}");
        }
    }

    /// 现态存档 → `autosave.json`（每成功步自动调用；`s` 命令亦复用）。
    fn save_now(&mut self) -> Result<(), String> {
        let envelope = parse_envelope(&self.kernel.dispatch("save", ""));
        if envelope["ok"] != Json::Bool(true) {
            return Err(format!("自动存档失败：{}", error_text(&envelope)));
        }
        let save = &envelope["save"];
        if save.is_null() {
            return Err("自动存档失败：save 信封缺少 save 字段".to_owned());
        }
        // 原样写 save 对象（紧凑 JSON）——与内核存档字节同形，可直接作 load payload。
        fs::write(&self.autosave_path, save.to_string())
            .map_err(|err| format!("写入 {} 失败：{err}", self.autosave_path.display()))
    }

    /// 磁盘 autosave 回灌进当前驱动（续局 / `l` 命令）。
    fn load_autosave(&mut self) -> Result<(), String> {
        let text = fs::read_to_string(&self.autosave_path)
            .map_err(|err| format!("读取 {} 失败：{err}", self.autosave_path.display()))?;
        let envelope = parse_envelope(&self.kernel.dispatch("load", &text));
        if envelope["ok"] != Json::Bool(true) {
            return Err(format!("读档失败：{}", error_text(&envelope)));
        }
        self.summary_cache = None;
        Ok(())
    }

    /// `state` 诊断信封（不推进状态）。
    fn state_envelope(&mut self) -> Json {
        parse_envelope(&self.kernel.dispatch("state", ""))
    }

    /// 自动推进阈值：读 Mod 静态配置的 `drive` 段（读不到 / 不是 JSON → 全兜底）。
    /// 每步现读（阈值只影响失控防护，不影响状态），改配置后下一步即生效。
    fn drive_limits(&self) -> DriveLimits {
        read_json(&self.mod_dir.join(&self.config_rel))
            .map(|config| DriveLimits::from_config(&config))
            .unwrap_or_default()
    }

    /// 回执标准字段：一次只读动作（[`SUMMARY_EVENT`]）取 Mod **自报**的摘要。
    ///
    /// 这里**不读 autosave**：宿主对「结局 / 冲突 / 待验收提案 / 收口前一步」的
    /// 全部认知只来自这一条回执。缺失（旧 Mod）→ [`Summary::unknown`]，调用方
    /// 按安全值处理并停等。结果缓存到本步结束（一次派发只问一次）。
    fn summary(&mut self) -> Summary {
        if let Some(cached) = &self.summary_cache {
            return cached.clone();
        }
        let envelope = parse_envelope(&self.kernel.dispatch(SUMMARY_EVENT, DEFAULT_PAYLOAD));
        let summary = if envelope["ok"] == Json::Bool(true) {
            Summary::from_narrative(&narrative_lines(&envelope))
        } else {
            Summary::unknown()
        };
        self.summary_cache = Some(summary.clone());
        summary
    }

    /// inbox 已消费行数（cursor 文件缺失 / 损坏 = 0：重放由 Mod 侧幂等去重兵底）。
    fn inbox_cursor(&self) -> usize {
        read_json(&self.inbox_cursor_path)
            .and_then(|cursor| cursor["consumed"].as_u64())
            .map_or(0, |consumed| consumed as usize)
    }

    /// 前移消费位移（仅供 `drain_inbox` 在 dispatch + 存档成功后调）。
    fn write_inbox_cursor(&mut self, consumed: usize) {
        let record = json!({"consumed": consumed, "updated_at": now_secs()});
        if let Err(message) = write_json_pretty(&self.inbox_cursor_path, &record) {
            // cursor 写不了不是丢需求：重放靠 external_id 幂等去重，只告警。
            self.notices
                .push(format!("写入 inbox cursor 失败：{message}"));
        }
    }

    /// 消费 inbox 新行：逐行 dispatch `requirement.add`，成功后前移 cursor。
    ///
    /// 崩溃安全序：dispatch（Mod 侧入队 + 自动存档）→ 成功才写 cursor。因此
    /// 「存档成功、cursor 未写」的重启会重放同一行，靠 external_id 幂等去重。
    /// Mod 侧拒绝（done / 队列满 / 载荷）走叙事面，**不算成功**：不推 cursor，
    /// 留给调用方报错（局末 leftover 必须非零退出，不静默丢弃）。
    fn drain_inbox(&mut self) -> InboxDrain {
        let mut drain = InboxDrain::default();
        let Ok(text) = fs::read_to_string(&self.inbox_path) else {
            return drain; // 无 inbox 文件 = 本局没有异步需求
        };
        let lines: Vec<&str> = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect();
        let mut consumed = self.inbox_cursor();
        if consumed > lines.len() {
            // inbox 被外部替换 / 截短：按从头消费算（幂等去重保证不重复入队）。
            consumed = 0;
        }
        while consumed < lines.len() {
            let line = lines[consumed];
            let Ok(record) = serde_json::from_str::<Json>(line) else {
                // 坏行永远无法 dispatch：跳过并前移，否则整个队列永久阻塞。
                consumed += 1;
                drain.skipped += 1;
                self.write_inbox_cursor(consumed);
                self.notices
                    .push(format!("inbox 第 {consumed} 行不是合法 JSON，已跳过"));
                continue;
            };
            let external = record["external_id"]
                .as_str()
                .unwrap_or("")
                .trim()
                .to_owned();
            // kind 缺省 = add（旧行只有 title / description）。
            let kind = match record["kind"].as_str() {
                None => InboxKind::Add,
                Some(raw) => match InboxKind::parse(raw) {
                    Some(kind) => kind,
                    None => {
                        consumed += 1;
                        drain.skipped += 1;
                        self.write_inbox_cursor(consumed);
                        self.notices
                            .push(format!("inbox 第 {consumed} 行 kind 未知，已跳过"));
                        continue;
                    }
                },
            };
            let Some(fields) = inbox_dispatch_payload(kind, &record, &external) else {
                consumed += 1;
                drain.skipped += 1;
                self.write_inbox_cursor(consumed);
                self.notices.push(format!(
                    "inbox 第 {consumed} 行缺字段（external_id / title / payload），已跳过"
                ));
                continue;
            };
            // payload 必须是 **JSON 字符串**（内核要求 JSON 标量），不是裸 wire format。
            let payload = json!(fields).to_string();
            let receipt = self.dispatch_event(kind.event(), &payload);
            if receipt["ok"] != Json::Bool(true) || dispatch_rejected(&receipt) {
                // 内核错误信封（载荷 / 校验失败）与 Mod 侧拒绝同属「未入队」：
                // cursor 不前移，留给调用方报错与重启重放。
                drain.error = Some(first_rejection(&receipt));
                break;
            }
            consumed += 1;
            drain.applied += 1;
            if narrative_lines(&receipt)
                .iter()
                .any(|line| line.starts_with(REPLAY_PREFIX))
            {
                drain.replayed += 1;
            }
            // dispatch_event 成功步已重写 autosave——到这里才允许前移 cursor。
            self.write_inbox_cursor(consumed);
            drain.last = Some(receipt);
        }
        drain.remaining = lines.len().saturating_sub(consumed);
        drain
    }

    /// 热载检查：mod 目录文件 mtime + 字节数变化 → 状态无损换新 mod。
    ///
    /// 序：现态存档（`save` 信封即基线）→ 按新包 `restore_with_resources`
    /// （重装载 + 回灌单点原子完成）→ 成功即换驱动；失败保留旧驱动继续。
    /// 签名在**尝试后**前移：同一份破损 mod 只告警一次，修好文件 mtime 再变
    /// 即触发下一次尝试。
    fn maybe_reload(&mut self) {
        let signature = mod_signature(&self.mod_dir);
        if signature == self.signature {
            return;
        }
        self.signature = signature;

        // ① 现态存档（热载基线；状态不变，故不写 autosave 文件）。
        let save_envelope = parse_envelope(&self.kernel.dispatch("save", ""));
        let save = &save_envelope["save"];
        if save.is_null() {
            let message = format!("热载中止：现态存档不可用（{}）", error_text(&save_envelope));
            self.notices.push(message.clone());
            self.reload = Some(Reload {
                reloaded: false,
                source: "watch",
                message: Some(message),
            });
            return;
        }
        let save_text = save.to_string();

        // ② 新包重装载 + 回灌（一次原子构造；失败即回滚到旧驱动）。
        let outcome = ModBundle::load(&self.mod_dir).and_then(|bundle| {
            KernelFacade::restore_with_resources(
                save_text.as_bytes(),
                &bundle.mod_source,
                &bundle.manifest_bytes,
                &bundle.schemas,
                &bundle.resources,
            )
            .map_err(|err| format!("内核重装载失败：{err}"))
        });
        match outcome {
            Ok(kernel) => {
                self.kernel = kernel;
                // 换了 Mod 就换了摘要约定 → 丢掉缓存，下一步重新问。
                self.summary_cache = None;
                self.reload = Some(Reload {
                    reloaded: true,
                    source: "watch",
                    message: None,
                });
                // 签名前移后即时落盘：下一进程按新包比对，不重复报「变过」。
                self.write_meta();
            }
            Err(message) => {
                // 旧 mod 继续跑：热载是增强面，不能把在局中的会话打崩。
                self.notices
                    .push(format!("热载失败，沿用旧 mod 继续：{message}"));
                self.reload = Some(Reload {
                    reloaded: false,
                    source: "watch",
                    message: Some(message),
                });
            }
        }
    }

    /// 派发一步：dispatch → 事件日志 → 自动存档（仅成功步）→ 回执。
    fn dispatch_event(&mut self, name: &str, payload: &str) -> Json {
        let envelope = parse_envelope(&self.kernel.dispatch(name, payload));
        // 真实派发可能已经改变语义面（阶段 / 结局 / 提案 / 冲突）→ 摘要缓存失效。
        self.summary_cache = None;
        let ok = envelope["ok"] == Json::Bool(true);
        let narrative = narrative_lines(&envelope);
        let ending = narrative
            .iter()
            .find(|line| line.starts_with(ENDING_PREFIX))
            .cloned();
        let seq = self.seq;
        self.seq += 1;

        // 事件日志：成功与失败都落一行（ok 自证；结算据 ok 计成功步数）。
        self.append_event(seq, name, payload, ok, narrative.len(), ending.as_deref());

        // 自动存档：仅成功步（失败步不改状态，重写同一份 autosave 无意义）。
        if ok {
            if let Err(message) = self.save_now() {
                self.notices.push(message);
            }
        }

        // 待决选择：成功信封自带；失败信封补一次 state（不推进状态），
        // 让回执在「越权动作被拒」之后仍能指引下一步。
        let pending = if ok {
            envelope["pendingChoice"].clone()
        } else {
            self.state_envelope()["pendingChoice"].clone()
        };

        let receipt =
            self.build_receipt(name, seq, &envelope, &narrative, ending.as_deref(), pending);
        self.reload = None;
        receipt
    }

    /// 事件日志追加一行（宿主侧旁路证据；写失败只告警，不中断局）。
    fn append_event(
        &mut self,
        seq: u64,
        name: &str,
        payload: &str,
        ok: bool,
        narrative_count: usize,
        ending: Option<&str>,
    ) {
        let mut record = json!({
            "seq": seq,
            "event": name,
            "payload": payload,
            "ok": ok,
            "narrative_count": narrative_count,
            "timestamp": now_secs(),
            "ending": ending.is_some(),
        });
        if let Some(line) = ending {
            record["ending_line"] = json!(line);
        }
        match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.events_path)
        {
            Ok(mut file) => {
                if let Err(err) = writeln!(file, "{record}") {
                    self.notices
                        .push(format!("写入 {} 失败：{err}", self.events_path.display()));
                }
            }
            Err(err) => self
                .notices
                .push(format!("打开 {} 失败：{err}", self.events_path.display())),
        }
    }

    /// 单发 / 交互统一回执（stdout 契约本体）。
    fn build_receipt(
        &mut self,
        name: &str,
        seq: u64,
        envelope: &Json,
        narrative: &[String],
        ending: Option<&str>,
        pending: Json,
    ) -> Json {
        let ok = envelope["ok"] == Json::Bool(true);
        let mut receipt = json!({
            "ok": ok,
            "event": name,
            "narrative": narrative,
            "pendingChoice": pending,
            "autosave": self.autosave_path.display().to_string(),
            "seq": seq,
        });
        if ok {
            receipt["revision"] = envelope["revision"].clone();
        } else {
            receipt["error"] = envelope["error"].clone();
        }
        if ending.is_some() {
            receipt["ending"] = json!(true);
        }

        // 警告面：内核 D1 warnings + 宿主侧 notices（热载 / 存档 / 重置）。
        let mut warnings: Vec<Json> = envelope["warnings"].as_array().cloned().unwrap_or_default();
        for notice in std::mem::take(&mut self.notices) {
            warnings.push(json!(notice));
        }
        if !warnings.is_empty() {
            receipt["warnings"] = json!(warnings);
        }

        if let Some(reload) = &self.reload {
            receipt["hotReload"] = reload.to_json();
        }
        receipt
    }
}

// ============================================================================
// §inbox 异步需求队列的宿主侧：追加 / 载荷构造 / 拒绝识别 / 步数预算
// ——"新需求随时能进"的真实入口（不依赖 Mod 内部提示词）。
// ============================================================================

/// `--requirement` 的载荷切分：`标题|描述`（只切首段——描述里的竖线保留）。
/// 标题 / 描述里的 `|` 换成全角 `｜`：竖线是 Mod 侧的三段式分隔符（`title|description|external_id`），
/// 混进去会把描述尾部误当幂等键。
fn split_requirement(text: &str) -> (String, String) {
    let (title, description) = match text.split_once('|') {
        Some((head, tail)) => (head.trim(), tail.trim()),
        None => (text.trim(), ""),
    };
    (sanitize_field(title), sanitize_field(description))
}

/// 把分隔符换成全角形态（保留可读性，不丢字）。
fn sanitize_field(text: &str) -> String {
    text.replace('|', "｜")
        .replace(['\n', '\r'], " ")
        .trim()
        .to_owned()
}

/// inbox 记录的 dispatch 载荷组装（不含外层 JSON 引号）。
/// - add：`title|description|external_id`（external_id 是 Mod 侧幂等键）
/// - proposal：`req|rev|stage|summary|artifacts[|changes]` → 补齐第 7 段 external_id
/// - amend / split / merge / conflict / resolve：`<wire>|@ext=<external_id>`
fn inbox_dispatch_payload(kind: InboxKind, record: &Json, external: &str) -> Option<String> {
    if external.is_empty() {
        return None;
    }
    match kind {
        InboxKind::Add => {
            let title = record["title"].as_str()?.trim();
            if title.is_empty() {
                return None;
            }
            let description = record["description"].as_str().unwrap_or("").trim();
            Some(format!(
                "{}|{}|{}",
                sanitize_field(title),
                sanitize_field(description),
                external
            ))
        }
        InboxKind::Proposal => {
            let raw = record["payload"].as_str()?.trim();
            let segments = raw.split('|').count();
            match segments {
                0..=4 => None,
                5 => Some(format!("{raw}||{external}")),
                6 => Some(format!("{raw}|{external}")),
                _ => Some(raw.to_owned()),
            }
        }
        _ => {
            let raw = record["payload"].as_str()?.trim();
            if raw.is_empty() {
                return None;
            }
            Some(format!("{raw}|@ext={external}"))
        }
    }
}

/// Mod 侧拒绝的识别（叙事前缀）：内核信封仍是 ok，宿主必须显式看叙事。
fn dispatch_rejected(receipt: &Json) -> bool {
    narrative_lines(receipt).iter().any(|line| {
        REJECTION_PREFIXES
            .iter()
            .any(|prefix| line.starts_with(prefix))
    })
}

/// 拒绝原因（回执里的第一条拒绝叙事；缺失则给错误码文案）。
fn first_rejection(receipt: &Json) -> String {
    narrative_lines(receipt)
        .into_iter()
        .find(|line| {
            REJECTION_PREFIXES
                .iter()
                .any(|prefix| line.starts_with(prefix))
        })
        .unwrap_or_else(|| error_text(receipt))
}
/// pendingChoice 指纹（自动推进的「无进展」判据：提示词 + 全部选项文本）。
fn choice_signature(receipt: &Json) -> String {
    let Some(choice) = receipt["pendingChoice"].as_object() else {
        return "(none)".to_owned();
    };
    let prompt = choice["prompt"].as_str().unwrap_or("");
    let options = choice["options"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .collect::<Vec<_>>()
        .join("\u{1f}");
    format!("{prompt}\u{1e}{options}")
}

/// 追加一行到 inbox（`OpenOptions::append`；不影响 autosave，故可跨进程并发）。
fn append_inbox_line(path: &Path, record: &Json) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|err| format!("打开 {} 失败：{err}", path.display()))?;
    writeln!(file, "{record}").map_err(|err| format!("写入 {} 失败：{err}", path.display()))
}

/// inbox 现有非空行数（external_id 的序号部分；与消费侧的计数口径一致）。
fn inbox_line_count(path: &Path) -> usize {
    fs::read_to_string(path).map_or(0, |text| {
        text.lines().filter(|line| !line.trim().is_empty()).count()
    })
}

/// run 目录里**尚未消费**的 inbox 行数（现读文件 + 现读 cursor，不信任缓存）。
/// 这是结算硬门禁：inbox 还有行却报 `ok=true` = 静默丢需求。
fn unconsumed_inbox_lines(run_dir: &Path) -> usize {
    let lines = inbox_line_count(&run_dir.join(INBOX_FILE));
    let consumed = read_json(&run_dir.join(INBOX_CURSOR_FILE))
        .and_then(|cursor| cursor["consumed"].as_u64())
        .map_or(0, |value| value as usize);
    lines.saturating_sub(consumed)
}

// ============================================================================
// §协调协议 run 目录内的跨进程「写入 ↔ 收口」互斥（std-only，无新依赖）
//
// 要解决的问题不是「再读一次文件」，而是两个进程对「本局还能不能收增量」的**共识**：
// `--requirement` 是独立进程，本局最后一次 drain 之后、进程退出之前的写入会
// 「成功落盘但永远不被消费」。只靠新增检查点只能降低概率。
//
// 三个文件（全部在 run 目录）：
//   coordination.lock —— 互斥锁（create_new；陈旧按 mtime 抢占，崩溃不永久卡死）
//   coordination.json —— 唯一事实源：{phase, writers, owner_pid, heartbeat_at}
//   inbox.jsonl / inbox.cursor —— 载荷与消费位移（语义不变）
//
// 写命令（--requirement / --proposal / --amend / …）：
//   ① 加锁读 phase：running → 登记 writers[token]=now；
//      closing（心跳未过期）→ **非零拒绝**；finalized → **非零拒绝（开新局）**；
//      closing 但心跳过期 → 判定 owner 已崩，恢复 running 后继续。
//   ② 追加 JSONL 行（**token 仍然持有**，所以收口侧的「等 writers 归零」
//      与「最后 drain」之间不会插进写者）。
//   ③ 加锁摘除自己的 token。
//
// 收口侧（自动推进到结局后的 settle / 独立 --settle）：
//   ① phase=running → closing（记 owner_pid + 心跳）；finalized → 直接拒绝。
//   ② 轮等 writers 归零（在途写者只是一行追加的时长）。
//   ③ **在 closing 保护下**做最后一次 drain：任何成功写入都 happens-before 它。
//      · 有新需求且未 done → phase=running（解除 closing）继续 loop；
//      · 无新需求 → 同一保护下完成 review.note + settle，然后 phase=finalized。
//
// 崩溃恢复：closing 用心跳续命（CLOSING_STALE_MS）；进程崩了以后，后续写命令或
// resume 看到心跳过期的 closing 即判死并恢复 running（已 done 则保留 closing 供结算）。
// 锁也有 mtime 陈旧抢占。两者都不依赖固定 sleep 去「降低概率」。
// ============================================================================
const COORDINATION_FILE: &str = "coordination.json";
const COORDINATION_LOCK: &str = "coordination.lock";
const PHASE_RUNNING: &str = "running";
const PHASE_CLOSING: &str = "closing";
const PHASE_FINALIZED: &str = "finalized";
/// 锁文件 mtime 超过此值即视为崩溃残留（可抢占）。
const LOCK_STALE_MS: u128 = 2_000;
const LOCK_RETRY_MS: u64 = 2;
const LOCK_MAX_TRIES: usize = 1_500;
/// closing 心跳超过此值即视为 owner 已崩溃（可恢复为 running）。
const CLOSING_STALE_MS: u128 = 10_000;
/// 收口侧等待在途写入者的上限（写者只在追加一行期间持有 token）。
const CLOSING_WAIT_MS: u128 = 5_000;

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis())
}

fn default_coordination() -> Json {
    json!({"phase": PHASE_RUNNING, "writers": {}, "owner_pid": 0, "heartbeat_at": 0})
}

fn read_coordination(run_dir: &Path) -> Json {
    read_json(&run_dir.join(COORDINATION_FILE)).unwrap_or_else(default_coordination)
}

/// 原子写（tmp + rename）：不允许出现半截 coordination.json。
fn write_coordination(run_dir: &Path, value: &Json) -> Result<(), String> {
    let path = run_dir.join(COORDINATION_FILE);
    let tmp = run_dir.join(format!("{COORDINATION_FILE}.tmp-{}", std::process::id()));
    let text =
        serde_json::to_string(value).map_err(|err| format!("序列化 coordination 失败：{err}"))?;
    fs::write(&tmp, text).map_err(|err| format!("写入 {} 失败：{err}", tmp.display()))?;
    fs::rename(&tmp, &path).map_err(|err| format!("替换 {} 失败：{err}", path.display()))
}

struct DirLock {
    path: PathBuf,
}

impl Drop for DirLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// 取协调锁（create_new）：不阻塞进程崩溃后永久卡死——陈旧锁按 mtime 抢占。
fn acquire_dir_lock(run_dir: &Path) -> Result<DirLock, String> {
    let path = run_dir.join(COORDINATION_LOCK);
    for _ in 0..LOCK_MAX_TRIES {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                let _ = writeln!(file, "{}", std::process::id());
                return Ok(DirLock { path });
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                if let Ok(meta) = fs::metadata(&path) {
                    let age = meta
                        .modified()
                        .ok()
                        .and_then(|time| SystemTime::now().duration_since(time).ok())
                        .map_or(u128::MAX, |elapsed| elapsed.as_millis());
                    if age > LOCK_STALE_MS {
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                }
                std::thread::sleep(Duration::from_millis(LOCK_RETRY_MS));
            }
            Err(err) => return Err(format!("创建 {} 失败：{err}", path.display())),
        }
    }
    Err(format!("协调锁争用超时（{}）", path.display()))
}

/// 锁内读—改—写 coordination.json（短临界区，不跨文件 IO）。
fn coordination_update<T>(run_dir: &Path, f: impl FnOnce(&mut Json) -> T) -> Result<T, String> {
    let _guard = acquire_dir_lock(run_dir)?;
    let mut record = read_coordination(run_dir);
    let out = f(&mut record);
    record["updated_at"] = json!(now_secs());
    write_coordination(run_dir, &record)?;
    Ok(out)
}

/// 写命令的准入结果。
#[derive(Debug)]
enum WriteGate {
    /// 本局正在收口（owner 心跳未过期）。
    Closing,
    /// 本局已 finalized（settlement 已定稿）。
    Finalized,
    /// 锁 / IO 故障。
    Internal(String),
}

/// 写前登记：成功返回 writer token（落盘完成后必须 `end_write` 摘除）。
fn begin_write(run_dir: &Path) -> Result<String, WriteGate> {
    let token = format!("{}-{}", std::process::id(), now_ms());
    let token_for_record = token.clone();
    let outcome = coordination_update(run_dir, move |record| {
        let phase = record["phase"].as_str().unwrap_or(PHASE_RUNNING).to_owned();
        if phase == PHASE_FINALIZED {
            return Err(WriteGate::Finalized);
        }
        if phase == PHASE_CLOSING {
            let beat = record["heartbeat_at"]
                .as_u64()
                .map_or(0, |value| value as u128);
            if now_ms().saturating_sub(beat) > CLOSING_STALE_MS {
                // 崩溃恢复：owner 已死，收回 closing。
                record["phase"] = json!(PHASE_RUNNING);
                record["owner_pid"] = json!(0);
            } else {
                return Err(WriteGate::Closing);
            }
        }
        let stamp = json!(now_ms() as u64);
        match record["writers"].as_object_mut() {
            Some(map) => {
                map.insert(token_for_record, stamp);
            }
            None => {
                let mut map = serde_json::Map::new();
                map.insert(token_for_record, stamp);
                record["writers"] = Json::Object(map);
            }
        }
        Ok(())
    });
    match outcome {
        Ok(Ok(())) => Ok(token),
        Ok(Err(gate)) => Err(gate),
        Err(message) => Err(WriteGate::Internal(message)),
    }
}

fn end_write(run_dir: &Path, token: &str) {
    let _ = coordination_update(run_dir, |record| {
        if let Some(map) = record["writers"].as_object_mut() {
            map.remove(token);
        }
    });
}

#[derive(Debug, PartialEq, Eq)]
enum ClosingEntry {
    /// 已进入 closing（持锁方负责后续 settle + finalize）。
    Entered,
    /// 本局已 finalized：不再收新写入，也不重写报告。
    AlreadyFinalized,
    /// 已由本进程持有（幂等）。
    Owned,
}

/// 进入 closing 并等所有在途写入者退出（它们的行已落盘 → 后续 drain 必能看到）。
fn enter_closing(run_dir: &Path) -> Result<ClosingEntry, String> {
    let pid = std::process::id();
    let entry = coordination_update(run_dir, |record| {
        let phase = record["phase"].as_str().unwrap_or(PHASE_RUNNING).to_owned();
        if phase == PHASE_FINALIZED {
            return ClosingEntry::AlreadyFinalized;
        }
        if phase == PHASE_CLOSING && record["owner_pid"].as_u64() == Some(pid as u64) {
            record["heartbeat_at"] = json!(now_ms() as u64);
            return ClosingEntry::Owned;
        }
        record["phase"] = json!(PHASE_CLOSING);
        record["owner_pid"] = json!(pid);
        record["heartbeat_at"] = json!(now_ms() as u64);
        ClosingEntry::Entered
    })?;
    if entry == ClosingEntry::AlreadyFinalized {
        return Ok(entry);
    }
    // 等在途写者归零：它们是「已经登记但还没写盘」的 append，必须等它们落盘。
    let deadline = now_ms() + CLOSING_WAIT_MS;
    loop {
        let pending = read_coordination(run_dir)["writers"]
            .as_object()
            .map_or(0, serde_json::Map::len);
        if pending == 0 {
            return Ok(entry);
        }
        if now_ms() > deadline {
            // 崩溃的写者会把 token 留着：清掉心跳过期的，再等最后一下。
            coordination_update(run_dir, |record| {
                let stale_before = now_ms().saturating_sub(CLOSING_STALE_MS) as u64;
                if let Some(map) = record["writers"].as_object_mut() {
                    map.retain(|_, started| started.as_u64().unwrap_or(0) > stale_before);
                }
                record["heartbeat_at"] = json!(now_ms() as u64);
            })?;
            let left = read_coordination(run_dir)["writers"]
                .as_object()
                .map_or(0, serde_json::Map::len);
            if left > 0 {
                return Err(format!("收口等待写入者超时（{left} 个在途）"));
            }
            return Ok(entry);
        }
        std::thread::sleep(Duration::from_millis(LOCK_RETRY_MS));
    }
}

fn closing_heartbeat(run_dir: &Path) {
    let _ = coordination_update(run_dir, |record| {
        record["heartbeat_at"] = json!(now_ms() as u64);
    });
}

/// 解除 closing（回到 running）：drain 到新需求、还没 done 时用。
fn leave_closing(run_dir: &Path) {
    let _ = coordination_update(run_dir, |record| {
        if record["phase"].as_str() != Some(PHASE_FINALIZED) {
            record["phase"] = json!(PHASE_RUNNING);
            record["owner_pid"] = json!(0);
        }
    });
}

/// 收口完成：此后所有写命令一律非零拒绝（需开新局）。
fn finalize_run(run_dir: &Path) {
    let _ = coordination_update(run_dir, |record| {
        record["phase"] = json!(PHASE_FINALIZED);
        record["heartbeat_at"] = json!(now_ms() as u64);
    });
}

/// 崩溃恢复：resume 时若发现「心跳过期的 closing」且本局未 done，恢复 running。
fn recover_stale_closing(run_dir: &Path, ended: bool) -> Option<String> {
    if ended {
        return None;
    }
    let record = read_coordination(run_dir);
    if record["phase"].as_str() != Some(PHASE_CLOSING) {
        return None;
    }
    let beat = record["heartbeat_at"]
        .as_u64()
        .map_or(0, |value| value as u128);
    if now_ms().saturating_sub(beat) <= CLOSING_STALE_MS {
        return None;
    }
    leave_closing(run_dir);
    Some("检测到陈旧的 closing（owner 已退出）——已恢复为 running 继续可写".to_owned())
}

/// 外部幂等 id：`inbox-<行号>-<纳秒>-<pid>`。
/// 行号保证顺序追加唯一，纳秒 + pid 做并发编写的兵底——两者只用于去重，
/// 不参与任何演示 / 状态判定。
fn new_external_id(run_id: &str, line_index: usize) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    format!("inbox-{run_id}-{line_index}-{nanos}-{}", std::process::id())
}

// ============================================================================
// §单发模式 Agent 主用：一次派发 → 单行 JSON 回执 → 退出
// ============================================================================

fn run_single(args: &Args) -> i32 {
    let event = args.event.clone().unwrap_or_else(|| ENTRY_EVENT.to_owned());
    let mut session = match Session::open(args) {
        Ok(session) => session,
        Err(message) => return single_shot_error(args, &event, &message),
    };

    // 单发每次派发前检查热载：Agent 侧改了 mod，下一条动作即生效。
    session.maybe_reload();

    // 派发前先消费 inbox：异步补入的需求不能因为「本步是 Agent 显式动作」而漏掉。
    // 被拒（局末 / 队列满 / 载荷）不阻碍本次派发，但会落 `inbox.remaining` + 警告。
    let drain = session.drain_inbox();

    let mut receipt = session.dispatch_event(&event, &args.payload);
    if drain.touched() {
        receipt["inbox"] = drain.to_json();
    }
    if let Some(message) = &drain.error {
        eprintln!(
            "[警告] inbox 未能全部并入：{message}（剩 {} 行未消费）",
            drain.remaining
        );
    }
    // stdout 只留回执行（Agent 的解析面）；警告另走 stderr 便于人读。
    if let Some(warnings) = receipt["warnings"].as_array() {
        for warning in warnings.iter().filter_map(|value| value.as_str()) {
            eprintln!("[警告] {warning}");
        }
    }
    println!("{receipt}");
    if receipt["ok"] == Json::Bool(true) {
        0
    } else {
        1
    }
}

/// `--requirement`：只往 inbox.jsonl 追加一行，不读不写 autosave 后退出。
///
/// 这是「新需求随时能进」的真实异步入口：本局在另一个进程里跑时，
/// 这里只做 `OpenOptions::append`，不存在与运行中局的状态竞争。
fn run_inbox_append(args: &Args) -> i32 {
    let Some((kind, text)) = args.inbox.clone() else {
        return 2;
    };
    let run_dir = args.runs_root.join(&args.run_id);
    if let Err(err) = fs::create_dir_all(&run_dir) {
        eprintln!("error: 创建 run 目录 {} 失败：{err}", run_dir.display());
        return 2;
    }
    // 协议：写前登记。closing / finalized 一律**命令本身非零拒绝**——
    // 不允许出现「写得很成功但运行侧已无声跨过最后一次 drain」。
    let token = match begin_write(&run_dir) {
        Ok(token) => token,
        Err(WriteGate::Finalized) => {
            eprintln!(
                "error: 本局已 finalized（settlement 已定稿）——请开新局（换 --run-id）再提交；本条未写入"
            );
            return 3;
        }
        Err(WriteGate::Closing) => {
            eprintln!("error: 本局正在收口（closing）——本条未写入，稍后重试；已结束请开新局");
            return 3;
        }
        Err(WriteGate::Internal(message)) => {
            eprintln!("error: {message}");
            return 3;
        }
    };
    let path = run_dir.join(INBOX_FILE);
    let mut record = json!({
        "external_id": new_external_id(&args.run_id, inbox_line_count(&path)),
        "kind": kind.label(),
        "created_at": now_secs(),
    });
    if kind == InboxKind::Add {
        // add 保留 title / description 字段（旧行兼容：旧行只有这两个字段）。
        let (title, description) = split_requirement(&text);
        record["title"] = json!(title);
        record["description"] = json!(description);
    } else {
        record["payload"] = json!(text.trim());
    }
    let receipt = match append_inbox_line(&path, &record) {
        Ok(()) => json!({
            "ok": true,
            "inbox": path.display().to_string(),
            "kind": kind.label(),
            "external_id": record["external_id"].clone(),
            "payload": record.get("payload").cloned().unwrap_or(Json::Null),
            "title": record.get("title").cloned().unwrap_or(Json::Null),
        }),
        Err(message) => {
            eprintln!("error: {message}");
            json!({
                "ok": false,
                "inbox": path.display().to_string(),
                "kind": kind.label(),
                "error": {"code": "INBOX", "message": message},
            })
        }
    };
    println!("{receipt}");
    // 行已落盘 → 摘除 token（closing 侧的「等 writers 归零」以此刻为界）。
    let pending = unconsumed_inbox_lines(&run_dir);
    end_write(&run_dir, &token);
    if pending > 0 && receipt["ok"] == Json::Bool(true) {
        eprintln!("[提示] 还有 {pending} 条未消费 inbox 行（运行侧下一拍会消费）");
    }
    if receipt["ok"] == Json::Bool(true) {
        0
    } else {
        1
    }
}

/// 装载 / 续局阶段的失败也走回执形态（Agent 只需一条解析路径）。
fn single_shot_error(args: &Args, event: &str, message: &str) -> i32 {
    let autosave = args.runs_root.join(&args.run_id).join(AUTOSAVE_FILE);
    let receipt = json!({
        "ok": false,
        "event": event,
        "narrative": [],
        "pendingChoice": Json::Null,
        "autosave": autosave.display().to_string(),
        "error": {"code": "INTERNAL", "message": message},
    });
    eprintln!("error: {message}");
    println!("{receipt}");
    1
}

// ============================================================================
// §drive 外部决策 回调协议 + 5 秒 fail-closed 暂停
// ============================================================================

#[derive(Debug, PartialEq, Eq)]
enum DecideOutcome {
    /// 经 pendingChoice 范围校验后的序号（从 1 开始）。
    Index(String),
    /// 需要人判断，或决策链路的任何一环失败；主状态不得推进。
    Human(String),
}

/// 解析决策命令的唯一 stdout 行：`N` 或 `HUMAN[:理由]`。
/// 非法输出、非数字、0 与越界序号全部按 HUMAN 处理。
fn parse_decision_output(stdout: &str, option_count: usize) -> DecideOutcome {
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
fn decide_choice(session: &Session, program: &Path, current: &Json) -> DecideOutcome {
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
    use super::{
        DriveLimits, DEFAULT_MAX_STEPS, DEFAULT_STALL_LIMIT, DEFAULT_STEPS_PER_REQUIREMENT,
    };
    use serde_json::json;

    #[test]
    fn missing_section_falls_back_to_builtin_defaults() {
        let limits = DriveLimits::from_config(&json!({"play": {"mode": "guide"}}));
        assert_eq!(limits, DriveLimits::default());
        assert_eq!(limits.max_steps, DEFAULT_MAX_STEPS);
        assert_eq!(limits.stall_limit, DEFAULT_STALL_LIMIT);
        assert_eq!(limits.steps_per_requirement, DEFAULT_STEPS_PER_REQUIREMENT);
        // 整份配置不是对象 / 全是 null 也不得 panic
        assert_eq!(
            DriveLimits::from_config(&json!(null)),
            DriveLimits::default()
        );
        assert_eq!(
            DriveLimits::from_config(&json!({"drive": []})),
            DriveLimits::default()
        );
    }

    #[test]
    fn reads_valid_section_and_computes_the_budget() {
        let limits = DriveLimits::from_config(&json!({
            "drive": {"maxSteps": 10, "stallLimit": 3, "stepsPerRequirement": 5}
        }));
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

    #[test]
    fn invalid_fields_fall_back_per_field() {
        // 字符串 / 0 / 负数都不是合法阈值：逐字段回落，不整份作废
        let limits = DriveLimits::from_config(&json!({
            "drive": {"maxSteps": "64", "stallLimit": 0, "stepsPerRequirement": -3}
        }));
        assert_eq!(limits.max_steps, DEFAULT_MAX_STEPS);
        assert_eq!(limits.stall_limit, DEFAULT_STALL_LIMIT);
        assert_eq!(limits.steps_per_requirement, DEFAULT_STEPS_PER_REQUIREMENT);
        // 只给一段也算数：给了的生效，没给的回落
        let partial = DriveLimits::from_config(&json!({"drive": {"maxSteps": 7}}));
        assert_eq!(partial.max_steps, 7);
        assert_eq!(partial.stall_limit, DEFAULT_STALL_LIMIT);
        assert_eq!(partial.steps_per_requirement, DEFAULT_STEPS_PER_REQUIREMENT);
    }
}

#[cfg(test)]
mod summary_tests {
    use super::{Summary, SUMMARY_PREFIX};
    use serde_json::json;

    /// 旧 Mod（无摘要行）→ 安全值、不 panic、不猜（`ending()` 为 None）。
    #[test]
    fn missing_summary_yields_safe_values_without_panicking() {
        let summary = Summary::from_narrative(&["普通叙事".to_owned()]);
        assert!(!summary.available);
        assert_eq!(summary.pending_proposals, 0);
        assert!(summary.conflicts.is_empty());
        assert!(summary.awaiting.is_none());
        assert_eq!(summary.ending(), None);
        assert!(!summary.next_ends_run);
        assert_eq!(summary.requirement_count, 0);
        // 空叙事同样安全
        assert!(!Summary::from_narrative(&[]).available);
        // 坏 JSON / 非对象：判为未知，不 panic 也不半信半疑
        for broken in [
            format!("{SUMMARY_PREFIX}{{不是 JSON"),
            format!("{SUMMARY_PREFIX}[]"),
            format!("{SUMMARY_PREFIX}null"),
            SUMMARY_PREFIX.to_owned(),
        ] {
            assert!(!Summary::from_narrative(&[broken]).available);
        }
    }

    #[test]
    fn parses_the_declared_fields_and_conflicts() {
        let line = format!(
            "{SUMMARY_PREFIX}{}",
            json!({
                "stage": "review",
                "ending": false,
                "verifyPassed": true,
                "nextEndsRun": true,
                "pendingProposals": 2,
                "requirementCount": 3,
                "conflicts": [
                    {"id": "r1", "peer": "r2", "note": "同一文件"},
                    {"id": "r2", "peer": "r1", "note": "  "}
                ],
                "awaiting": {"req": "r3", "rev": 4, "stage": "development"}
            })
        );
        let summary = Summary::from_narrative(&["前一行".to_owned(), line]);
        assert!(summary.available);
        assert_eq!(summary.stage, "review");
        assert_eq!(summary.ending(), Some(false));
        assert_eq!(summary.pending_proposals, 2);
        assert_eq!(summary.requirement_count, 3);
        // 空白说明的冲突不算真冲突（只报未解决的）
        assert_eq!(summary.conflicts.len(), 1);
        assert_eq!(summary.conflicts[0].label(), "r1 ⇄ r2：同一文件");
        let awaiting = summary.awaiting.expect("awaiting 应被解析");
        assert_eq!(awaiting.req, "r3");
        assert_eq!(awaiting.rev, 4);
        assert_eq!(awaiting.stage, "development");
    }

    /// `ending` 未声明 / 缺字段时只能读成 false，绝不能借用其他字段推断。
    #[test]
    fn ending_requires_the_declared_boolean() {
        let declared_true = format!("{SUMMARY_PREFIX}{}", json!({"ending": true}));
        assert_eq!(
            Summary::from_narrative(&[declared_true]).ending(),
            Some(true)
        );
        let missing = format!("{SUMMARY_PREFIX}{}", json!({"stage": "done"}));
        assert_eq!(Summary::from_narrative(&[missing]).ending(), Some(false));
    }
}

// ============================================================================
// §交互模式 数字选项 / event / s / l / i / config / q（现场感：每步打印处境）
// 推进方式由 `--drive` 决定（stdin / cmd:<路径> / none）：宿主只提供通道，
// 不持有推进策略——决策由人或外部决策命令给出。
// ============================================================================

/// 自动推进一步的决策（派发留在主循环里，统一打印与计数）。
enum DriveStep {
    /// 可以再请求外部决策命令推进一步。
    Run,
    /// 本局已收口：调用现有结算逻辑写 settlement.json。
    Settle,
    /// 无路可走 / 超步数：打印可读错误并非零退出（不死循环、不静默成功）。
    Stuck(String),
}

/// 自动推进的一步决策：结局 → 结算；无待选 → 已收口则结算，否则中止；步数硬顶防绕圈。
/// `limit` = 本次局的步数预算（基础硬顶 + 每条需求额外预算，见 [`DriveLimits`]）。
fn drive_step(summary: &Summary, current: &Json, auto_steps: usize, limit: usize) -> DriveStep {
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

fn run_interactive(args: &Args) -> i32 {
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
    let mut current = if session.resumed {
        state_receipt(&mut session, "resume", Vec::new())
    } else if matches!(drive, Drive::None) {
        // none = 只读回显：新局也不派发入口事件（零副作用）。
        state_receipt(&mut session, "state", Vec::new())
    } else {
        session.dispatch_event(ENTRY_EVENT, DEFAULT_PAYLOAD)
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
                    "error: Mod 未提供 summary（{} 无摘要行），无法自动判定——\
                     改用 --drive stdin 或升级 Mod；本步未推进",
                    SUMMARY_EVENT
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
            let limits = session.drive_limits();
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
                    current = session.dispatch_event(CHOICE_EVENT, &choice);
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
                    let leftover = unconsumed_inbox_lines(&session.run_dir);
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
                Ok(()) => println!("已存档 → {}", session.autosave_path.display()),
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
                if let Some((name, payload)) = split_event_command(input) {
                    current = dispatch_with_watch(&mut session, args, &name, &payload);
                    fresh = true;
                } else if let Ok(index) = input.parse::<i64>() {
                    if index < 1 {
                        println!("（选项序号从 1 起。）");
                        continue;
                    }
                    current =
                        dispatch_with_watch(&mut session, args, CHOICE_EVENT, &index.to_string());
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
fn dispatch_with_watch(session: &mut Session, args: &Args, name: &str, payload: &str) -> Json {
    if args.watch {
        session.maybe_reload();
    }
    session.dispatch_event(name, payload)
}

/// 打印一步回执（`fresh=false` 时只打印错误——存读档等不推进剧情）。
/// 无模式分支：结局行本身由 Mod 的 narrative 提供，宿主只加一条通用分隔。
fn print_receipt(receipt: &Json, fresh: bool) {
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
fn print_choices(receipt: &Json) {
    println!();
    match receipt["pendingChoice"].as_object() {
        Some(choice) => {
            if let Some(prompt) = choice["prompt"].as_str() {
                println!("? {prompt}");
            }
            for (index, option) in choice["options"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
            {
                let text = option.as_str().unwrap_or("");
                println!("  {}. {text}", index + 1);
            }
        }
        None => {
            println!("（无待决选择。数字=选择 / event <名称> [JSON] / s / l / i / config / q）")
        }
    }
}

/// 处境提示面 + 输入提示符（提示符不携带推进方式语义）。
fn print_prompt(receipt: &Json) {
    print_choices(receipt);
    print!("[ygg] > ");
    let _ = io::stdout().flush();
}

/// 不推进状态的回执（续局 / 读档 / none 驱动共用）：`pendingChoice` 现取 `state` 信封。
fn state_receipt(session: &mut Session, event: &str, narrative: Vec<String>) -> Json {
    let state = session.state_envelope();
    json!({
        "ok": true,
        "event": event,
        "narrative": narrative,
        "pendingChoice": state["pendingChoice"].clone(),
        "autosave": session.autosave_path.display().to_string(),
    })
}

/// `event <名称> [JSON]` 解析（JSON 可含空格——只切首段）。
fn split_event_command(input: &str) -> Option<(String, String)> {
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
        DEFAULT_PAYLOAD
    } else {
        payload
    };
    Some((name.to_owned(), payload.to_owned()))
}

/// `i`：诊断面人读输出（内核 state 信封原字段）。
fn print_state(session: &mut Session) {
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
fn print_config(session: &Session, args: &Args) {
    let watched = session
        .signature
        .iter()
        .map(|stamp| stamp.path.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    println!("mod      : {}", session.mod_label);
    println!("run-id   : {}", session.run_id);
    println!("run 目录 : {}", session.run_dir.display());
    println!("autosave : {}", session.autosave_path.display());
    println!(
        "配置      : {}（play.mode 归 Mod 叙事层；宿主只读其中的 drive 阈值）",
        session.mod_dir.join(&session.config_rel).display()
    );
    let limits = session.drive_limits();
    println!(
        "推进阈值 : 步数硬顶 {} = {} + {}×需求数；无进展判停 {}",
        limits.step_limit(0),
        limits.max_steps,
        limits.steps_per_requirement,
        limits.stall_limit
    );
    println!(
        "驱动     : {}（推进方式由 --drive 给定；Mod 的 play.mode 仅叙事层自用）",
        args.drive.label()
    );
    println!(
        "事件日志 : {}（已记录 {} 条，下一条 seq={}）",
        session.events_path.display(),
        session.seq,
        session.seq
    );
    println!(
        "需求 inbox: {}（已消费 {} 行 / 共 {} 行；--requirement 只追加）",
        session.inbox_path.display(),
        session.inbox_cursor(),
        inbox_line_count(&session.inbox_path)
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
// §结算 --settle：autosave + events.jsonl → settlement.json + 人读摘要
// ============================================================================

fn run_settle(args: &Args, auto: bool) -> i32 {
    let run_dir = args.runs_root.join(&args.run_id);
    let autosave_path = run_dir.join(AUTOSAVE_FILE);
    let events_path = run_dir.join(EVENTS_FILE);

    // ⓞ 协议：结算必须持有 closing（与 inbox 写入者互斥）。
    //    - finalized：幂等返回已有报告的 ok，不重写；
    //    - 本进程已持 closing（自动推进到结局后的 settle）：直接进；
    //    - 否则现在就进入并等在途写者落盘。
    match enter_closing(&run_dir) {
        Ok(ClosingEntry::AlreadyFinalized) => {
            let existing = run_dir.join(SETTLEMENT_FILE);
            if let Some(report) = read_json(&existing) {
                println!(
                    "本局已 finalized（{}），settlement ok = {}——不重写；需新局请换 --run-id",
                    existing.display(),
                    report["ok"]
                );
                return if report["ok"] == Json::Bool(true) {
                    0
                } else {
                    1
                };
            }
            eprintln!(
                "error: 本局已 finalized 但找不到 settlement.json（{}）",
                existing.display()
            );
            return 1;
        }
        Ok(_) => {}
        Err(message) => {
            eprintln!("error: {message}");
            return 1;
        }
    }
    // 进入 closing 后不再有新写入者；此处只需核对存量未消费行。
    let pending_before = unconsumed_inbox_lines(&run_dir);
    if auto {
        closing_heartbeat(&run_dir);
    }

    // ① 终局存档（唯一状态事实源；缺档即无法结算）。
    let save_text = match fs::read_to_string(&autosave_path) {
        Ok(text) => text,
        Err(err) => {
            eprintln!(
                "error: 读取 {} 失败：{err}（先跑一局再结算）",
                autosave_path.display()
            );
            if !auto {
                leave_closing(&run_dir);
            }
            return 1;
        }
    };
    let save: Json = match serde_json::from_str(&save_text) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("error: {} 不是合法 JSON：{err}", autosave_path.display());
            if !auto {
                leave_closing(&run_dir);
            }
            return 1;
        }
    };

    // ② 事件日志（坏行跳过并记入 errors，不整份报废）。
    let (records, mut errors) = read_event_records(&events_path);

    // ③ 事件面汇总：成功步数 / 失败步 / 结局行。
    let mut steps = 0usize;
    let mut reached_ending = false;
    let mut ending_line: Option<String> = None;
    let mut events = Vec::with_capacity(records.len());
    for record in &records {
        let ok = record["ok"].as_bool().unwrap_or(false);
        if ok {
            steps += 1;
        }
        if record["ending"].as_bool().unwrap_or(false) {
            reached_ending = true;
            if let Some(line) = record["ending_line"].as_str() {
                ending_line = Some(line.to_owned());
            }
        }
        events.push(json!({
            "event": record["event"].as_str().unwrap_or(""),
            "seq": record["seq"].as_u64().unwrap_or(0),
            "ok": ok,
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

    // ⑤ 结论：无失败步 ∧ 有事件 ∧ 达结局 ∧ **无未消费 inbox 行**。
    //    最后一条是硬门禁：inbox 还有行却报 ok=true 等于静默丢需求。
    //    closing 已阻止新写入，故 `pending_before` 就是最终口径；仍现读一次作交叉校验。
    let pending_inbox = unconsumed_inbox_lines(&run_dir).max(pending_before);
    let ok = !events.is_empty() && failed == 0 && reached_ending && pending_inbox == 0;
    if failed > 0 {
        errors.push(format!("{failed} 条事件记录为失败步"));
    }
    if !reached_ending {
        errors.push("未见结局叙事行：本局未收口".to_owned());
    }
    if events.is_empty() {
        errors.push("事件日志为空：本局无任何派发记录".to_owned());
    }
    if pending_inbox > 0 {
        errors.push(format!(
            "结局后仍有 {pending_inbox} 条未消费 inbox 行（{INBOX_FILE}）——本局已收口，需开新局（换 --run-id）"
        ));
    }

    let mut report = json!({
        "run_id": args.run_id,
        "mod": run_mod_label(&run_dir, args),
        "steps": steps,
        "events": events,
        "final_state": final_state,
        "duration_events": records.len(),
        "reached": if reached_ending { "ending" } else { "open" },
        "ok": ok,
        "errors": errors,
        "settled_at": now_secs(),
    });

    // ⑥ 落盘（pretty：人读证据）+ 人读摘要。
    let settlement_path = run_dir.join(SETTLEMENT_FILE);
    if let Err(message) = write_json_pretty(&settlement_path, &report) {
        eprintln!("error: {message}");
        if !auto {
            leave_closing(&run_dir);
        }
        return 1;
    }
    print_settlement(&report, &settlement_path, ending_line.as_deref(), failed);
    if !ok {
        // 未能给绿：已到结局的自动结算就 finalize（后续写入必须开新局）；
        // 独立 --settle 则退回 running，让 resume 先把存量行消费完。
        if auto || pending_inbox > 0 {
            finalize_run(&run_dir);
        } else {
            leave_closing(&run_dir);
        }
        return 1;
    }
    // ⑦ closing 保护下的交叉校验：进入 closing 后不应再出现新行，
    //    但保留一次现读作为兵底检查（万一有绕过协议的写入）。
    let after = unconsumed_inbox_lines(&run_dir);
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
        return 1;
    }
    // 真正的「已定稿」：之后所有写命令一律非零拒绝（需开新局）。
    finalize_run(&run_dir);
    if auto {
        closing_heartbeat(&run_dir);
    }
    0
}

/// 结算报告的 `mod` 标签：run.json 元信息优先（最贴近本局实际装载），
/// 其次 `--mod`，都缺则 `(未知)`。
fn run_mod_label(run_dir: &Path, args: &Args) -> String {
    read_meta(&run_dir.join(META_FILE))
        .and_then(|meta| meta["mod"].as_str().map(str::to_owned))
        .or_else(|| args.mod_dir.as_ref().map(|path| path.display().to_string()))
        .unwrap_or_else(|| "(未知)".to_owned())
}

/// run.json 元信息（缺失 / 非法一律 None：元信息是便利面，不是判据）。
fn read_meta(path: &Path) -> Option<Json> {
    read_json(path)
}

/// 读一个 JSON 文件（缺失 / 非法 → None；宿主侧便利面，不吞错、不死循环）。
fn read_json(path: &Path) -> Option<Json> {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Json>(&text).ok())
}

/// 元信息里的 mod 签名（跨进程热载比对基线）。
fn read_meta_signature(path: &Path) -> Option<String> {
    read_meta(path).and_then(|meta| meta["mod_signature"].as_str().map(str::to_owned))
}

/// 结算摘要（stdout 人读面；JSON 报告是机器面）。
fn print_settlement(report: &Json, path: &Path, ending_line: Option<&str>, failed: usize) {
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

/// 事件日志读盘（逐行 JSON；坏行跳过并记可读错误）。
fn read_event_records(path: &Path) -> (Vec<Json>, Vec<String>) {
    let mut records = Vec::new();
    let mut errors = Vec::new();
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => {
            errors.push(format!("读取 {} 失败：{err}", path.display()));
            return (records, errors);
        }
    };
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<Json>(line) {
            Ok(record) => records.push(record),
            Err(err) => errors.push(format!("{}:{} 记录非法：{err}", path.display(), index + 1)),
        }
    }
    (records, errors)
}

/// kv 摘要人读串（`k=v` 逗号分隔，超限截断并注明余量）。
fn kv_digest(kv: &Json, limit: usize) -> String {
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
fn scalar_label(value: &Json) -> String {
    match value {
        Json::String(text) => text.clone(),
        other => other.to_string(),
    }
}

// ============================================================================
// §共用 信封解析 / 错误文案 / 时钟 / 落盘
// ============================================================================

/// 信封解析（内部错误不 exit：agent 长驻，坏信封回可读错误信封续走）。
fn parse_envelope(reply: &str) -> Json {
    serde_json::from_str(reply).unwrap_or_else(|err| {
        json!({
            "ok": false,
            "error": {"code": "INTERNAL", "message": format!("信封不是合法 JSON：{err}")},
        })
    })
}

fn error_code(envelope: &Json) -> &str {
    envelope["error"]["code"].as_str().unwrap_or("UNKNOWN")
}

fn error_message(envelope: &Json) -> &str {
    envelope["error"]["message"].as_str().unwrap_or("")
}

fn error_text(envelope: &Json) -> String {
    format!("[{}] {}", error_code(envelope), error_message(envelope))
}

/// 成功信封的叙事行（失败信封无该键 → 空）。
fn narrative_lines(envelope: &Json) -> Vec<String> {
    envelope["narrative"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect()
}

/// 宿主侧时钟（unix 秒）：时间戳只进 events.jsonl / 元信息，不触内核状态。
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// pretty JSON 落盘（人读证据面；末尾补换行便于 diff）。
fn write_json_pretty(path: &Path, value: &Json) -> Result<(), String> {
    let mut text = serde_json::to_string_pretty(value)
        .map_err(|err| format!("序列化 {} 失败：{err}", path.display()))?;
    text.push('\n');
    fs::write(path, text).map_err(|err| format!("写入 {} 失败：{err}", path.display()))
}
