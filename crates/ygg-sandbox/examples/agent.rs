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
//!   （契约 `summaryEvent` 回执信封的 `meta.summary` 结构化字段），不读
//!   `ending` / `fsm.stage` / `prop.*` / `req.*` 等 Mod 私有 KV 键，
//!   也不从人读叙事里剖数据（旧「`【摘要】` 前缀 + JSON」契约已删除）。
//!
//! 三种形态：
//!
//! ```bash
//! # ① 交互模式（人 / 脚本驱动）
//! cargo run -p ygg-sandbox --example agent -- --mod packages/mods/test-adventure --run-id my-task
//!
//! # ② 单发模式（Agent 脚本化调用：动作 → JSON 回执 → 退出）
//! cargo run -p ygg-sandbox --example agent -- --mod <目录> --run-id my-task \
//!   --resume --event <Mod 的 choiceEvent> --payload '1'
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
//! ## host 契约（**必填、无默认值**；宿主代码不含任何事件名与叙事约定字面量）
//!
//! | 声明位置 | 段 | 内容 |
//! | --- | --- | --- |
//! | `pack.json` | `agent` | `entryEvent` / `choiceEvent` / `summaryEvent` / `endingPrefix` / `metaKey` / `rejectionPrefixes` / `replayPrefix` / `defaultPayload` |
//! | `pack.json` | `inboxRoutes` | 三个保留键 `externalIdField` / `payloadField` / `extMarker` + `kind` → `{action, fields[, write]}` |
//! | `data/config.json` | `settlement` | `requireEvents` / `requireNoFailedSteps` / `requireEnding` / `requireInboxConsumed` |
//! | `data/config.json` | `drive` | `maxSteps` / `stallLimit` / `stepsPerRequirement` |
//!
//! 缺任何一段 / 缺任一字段 / 类型不符 / 非正整数 → **装载期失败**（非零退出，
//! 错误文案点名缺失字段、给出应有形状与文档定位）。这是 fail-closed 的刻意选择：
//! 宿主宁可拒绝启动，也不猜 Mod 的意图（`ygg_sandbox::host_contract` 是唯一解析点）。
//!
//! ## 异步需求 inbox（④，跨进程队列入口）
//!
//! 协议：`<runs-root>/<run-id>/inbox.jsonl`，每行一个 JSON 对象（记录字段名由
//! `inboxRoutes` 的三个保留键声明：幂等键 / 预成形载荷 / 幂等后缀标记）；
//! `--requirement` 只做 `OpenOptions::append`，**不读写 autosave**，所以与运行中
//! 的局无锁竞争。文件保留作审计；消费位移记在 `inbox.cursor`（`{"consumed": N}`，
//! 按非空行数）。
//!
//! 消费时机：cmd 自动推进**之前**、stdin 每次提示**之前**、单发每次派发**之前**
//! 都会 drain 新行，按 `inboxRoutes` 声明的路由（kind → 动作 + 段序）派发。
//! 写命令（`--requirement` / `--proposal` / …）同样只认 Mod 的声明：某条路由带
//! `"write": {"flag": "<旗标名>", "shape": "named|payload"}` 才有对应的 CLI 旗标，
//! 未声明即明确拒绝（退出码 3，不静默丢行）。
//!
//! 崩溃安全：**dispatch 成功且 autosave 落盘后**才推进 cursor。若「存档成功、cursor
//! 未推进」时进程挂掉，重启会重放同一行——幂等键（`inboxRoutes.externalIdField`）
//! 是 Mod 侧去重依据，重放返回已有需求、不会重复入队（回执叙事以契约的
//! `agent.replayPrefix` 开头）。坏 JSON 行跳过并前移 cursor
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
//! 判停——阈值读 `<mod>/data/config.json` 的 `drive` 段（[`DriveLimits`]，**必填且
//! 必须正整数**；缺失 / 非法即装载期失败，见下节）。合法多需求局不会被误杀，
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
//! {"ok":true,"event":"<事件名>","narrative":["…"],"pendingChoice":{"prompt":"…","options":["…"]},
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

use std::cell::{Cell, RefCell};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{exit, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value as Json};
use ygg_sandbox::driver::KernelFacade;
use ygg_sandbox::host_contract::{
    sanitize_field, AgentContract, DriveLimits, InboxRoutes, SettlementPolicy, WriteShape,
    CONTRACT_DOC,
};
use ygg_sandbox::modload::ModBundle;

// ============================================================================
// §常量 只有**不是契约**的东西：run 目录文件名、宿主自身的超时。
//
// 事件名 / 叙事前缀 / 摘要键名 / 入口载荷 / inbox 字段名与幂等标记 / 结算口径 /
// 推进阈值**一律来自 Mod 自己的声明**（`pack.json` 的 `agent` / `inboxRoutes` +
// `config.json` 的 `settlement` / `drive`，解析在 `ygg_sandbox::host_contract`）。
// 本文件**不含任何 Mod 语义字面量**与回退默认值：契约缺失即装载期失败（fail-closed）。
// 归属边界（哪些字面量属于 host）见 `guide.show harness` 的 `literal-ownership`。
// ============================================================================

/// 外部决策命令的最长执行时间；任何异常都 fail closed，绝不回退到固定序号。
const DECIDE_TIMEOUT: Duration = Duration::from_secs(5);
/// 决策命令轮询间隔（std-only：spawn + try_wait + 超时 kill/wait）。
const DECIDE_POLL_INTERVAL: Duration = Duration::from_millis(20);

const AUTOSAVE_FILE: &str = "autosave.json";
const EVENTS_FILE: &str = "events.jsonl";
const META_FILE: &str = "run.json";
const SETTLEMENT_FILE: &str = "settlement.json";
const INBOX_FILE: &str = "inbox.jsonl";
const INBOX_CURSOR_FILE: &str = "inbox.cursor";
const DECIDE_INPUT_FILE: &str = "decide-input.json";

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

/// 一次 inbox 写命令（宿主 CLI 侧）。
///
/// 只带「哪个旗标 + 原始载荷」：**记录形态与 kind 都由 Mod 的
/// `pack.json` → `inboxRoutes.<kind>.write` 声明**（`flag` + `shape`），宿主不枚举
/// kind、也不猜记录形态。本 Mod 没声明匹配的写命令 → 明确拒绝（不静默丢行）。
#[derive(Clone)]
struct InboxWrite {
    /// CLI 旗标名（不带 `--`）：路由表的查找键。
    flag: &'static str,
    /// 命令行给的原始载荷（形态由声明决定怎么落到记录里）。
    value: String,
}

struct Args {
    mod_dir: Option<PathBuf>,
    run_id: String,
    runs_root: PathBuf,
    resume: bool,
    settle: bool,
    watch: bool,
    event: Option<String>,
    /// `--payload` 的值；`None` = 用 Mod 契约的 `agent.defaultPayload`（宿主不猜）。
    payload: Option<String>,
    /// 交互模式推方式（`--drive`；缺省 stdin）。
    drive: Drive,
    /// 异步 inbox 操作（只追 inbox.jsonl 后退出，不读写 autosave）。
    inbox: Option<InboxWrite>,
}

fn parse_args() -> Args {
    let mut mod_dir: Option<PathBuf> = None;
    let mut run_id: Option<String> = None;
    let mut runs_root = PathBuf::from("runs");
    let mut resume = false;
    let mut settle = false;
    let mut watch = false;
    let mut event: Option<String> = None;
    let mut payload: Option<String> = None;
    let mut drive = Drive::Stdin;
    let mut inbox: Option<InboxWrite> = None;

    // 写 inbox 的七个参数共享同一套「只能给一个」的校验。
    fn set_inbox(inbox: &mut Option<InboxWrite>, flag: &'static str, value: String) {
        if inbox.is_some() {
            usage_error("一次只能提交一个 inbox 操作");
        }
        *inbox = Some(InboxWrite { flag, value });
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
            "--payload" => payload = Some(next_value(&mut iter, "--payload")),
            "--drive" => {
                let text = next_value(&mut iter, "--drive");
                drive = match parse_drive(&text) {
                    Ok(drive) => drive,
                    Err(message) => usage_error(&message),
                };
            }
            "--requirement" => set_inbox(
                &mut inbox,
                "requirement",
                next_value(&mut iter, "--requirement"),
            ),
            "--amend" => set_inbox(&mut inbox, "amend", next_value(&mut iter, "--amend")),
            "--split" => set_inbox(&mut inbox, "split", next_value(&mut iter, "--split")),
            "--merge" => set_inbox(&mut inbox, "merge", next_value(&mut iter, "--merge")),
            "--conflict" => set_inbox(&mut inbox, "conflict", next_value(&mut iter, "--conflict")),
            "--resolve" => set_inbox(&mut inbox, "resolve", next_value(&mut iter, "--resolve")),
            "--proposal" => set_inbox(&mut inbox, "proposal", next_value(&mut iter, "--proposal")),
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
    if let Some(write) = &inbox {
        if settle || event.is_some() {
            usage_error(
                "inbox 操作（--requirement / --proposal / …）与 --event / --settle 不能同时使用",
            );
        }
        if write.value.trim().is_empty() {
            usage_error(&format!("--{} 需要非空载荷", write.flag));
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
                     schemas/*.json / data/*.json）。host 契约（pack.json 的 agent /
                     inboxRoutes、config.json 的 settlement / drive）**必填**；缺任一
                     即装载期失败。--settle / inbox 写命令可省略（改读 run.json 记录的
                     本局 mod 路径，两者都没有则报错）
  --run-id <标识>    本局标识：产物落 <runs-root>/<标识>/（必填）
  --runs-root <目录> runs 根目录（默认 ./runs）
  --resume           从 autosave.json 续局（缺存档则按新局开局）
  --watch            交互模式下每步检查 mod 目录 mtime 变化并热载
  --event <名称>     单发模式：派发一个事件后退出（stdout 单行 JSON 回执）
  --payload <JSON>   事件 payload（缺省 = Mod 契约的 agent.defaultPayload；必须 JSON 标量）
  --drive <方式>     交互模式推方式（默认 stdin；宿主不替你做决策）：
                       stdin      读 stdin：数字=选择 / event <名称> [JSON] / s / l / i / config / q
                       cmd:<路径> 不读 stdin：每个待选动作交给决策命令（stdout 单行 `N` 或
                                  `HUMAN[:理由]`）；HUMAN / 超时 / 非零退出一律安全暂停并退出 0
                                  （环境变量 YGG_DECIDE_CMD 可临时覆盖路径）
                       none       只打印当前回执就退出（不推进、不读 stdin）
  --requirement <串> 只把 '标题|描述' 追加到 <run 目录>/inbox.jsonl 后退出；
                     不读写 autosave，可在别一进程跑本局时安全并发调用
                     （旗标是否可用由 Mod 的 inboxRoutes.<kind>.write 声明）
  --amend <串>       --split <串>    --merge <串>    --conflict <串>
  --resolve <串>     --proposal <串>（同上：只在 Mod 声明了对应 write 时可用）
  --settle           读 autosave + events.jsonl 写 settlement.json
  -h, --help         本帮助

host 契约（**必填、无默认值**，宿主代码不含任何 Mod 语义字面量）：
  <mod>/pack.json       agent{entryEvent,choiceEvent,summaryEvent,endingPrefix,
                              metaKey,rejectionPrefixes,replayPrefix,defaultPayload}
                        inboxRoutes{externalIdField,payloadField,extMarker,
                                    <kind>:{action,fields[,write]}}
  <mod>/data/config.json settlement{requireEvents,requireNoFailedSteps,
                        requireEnding,requireInboxConsumed}
                        drive{maxSteps,stallLimit,stepsPerRequirement}（正整数）
  缺段 / 缺字段 / 类型不符 / 非正整数 → 装载期失败（点名缺失字段 + 应有形状 +
  文档定位：.ygg/mods/agent-seed/README.md 的「host 契约」章节）。

自动推进的失控防护：基础步数 + 每条需求额外步数 + 连续无进展判停；阈值读
  <mod>/data/config.json 的 drive 段（必填）。

异步需求（inbox）：每个自动动作前、stdin 每次提示前、单发每次派发前都会消费
  inbox.jsonl 的新行，按 inboxRoutes 声明派发；dispatch 成功且存档后才前移 cursor，
  重放靠 Mod 侧幂等键去重（字段名由 inboxRoutes.externalIdField 声明）；
  局已收口后仍有未消费行 → 非零退出。";
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
// §Mod 目录装载（运行期读盘——agent 必须支持任意 mod 目录，不能像
//   adventure.rs 那样 include_str! 编译期固化）
//
//   读盘/声明对账只走 `ygg_sandbox::modload`（与走查 playtest 同一份实现）：
//   本文件多出来的诉求只有两条——① 热载要**反复**从同一份参数重建驱动
//   （签名变了才重读），所以持有 `ModBundle` 而非一次性内核；② 错误文案要带
//   宿主口径的前缀。所以下面只有两层薄包装，不复制任何读盘/对账逻辑。
// ============================================================================

/// [`ModBundle::kernel`] 的宿主错误文案（装载失败 = 启动失败）。
fn kernel_from(bundle: &ModBundle) -> Result<KernelFacade, String> {
    bundle
        .kernel()
        .map_err(|err| format!("内核装载失败：{err}"))
}

/// [`ModBundle::restore`] 的宿主错误文案（热载/续局重建失败 = 回滚到旧驱动）。
fn restore_from(bundle: &ModBundle, save_bytes: &[u8]) -> Result<KernelFacade, String> {
    bundle
        .restore(save_bytes)
        .map_err(|err| format!("内核重装载失败：{err}"))
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

/// 监视文件清单：五件套 + pack.json 声明的每个条目 / schema 文件 / 代码模块
/// （pack.json 本身损坏时退化为固定三件，仍能检测修复动作）。
///
/// **`code` 段必须在这里纳管**（r2）：只监视 `mod.lua` 时，改 `src/*.lua` 而入口
/// 一字未动 → 签名不变 → 热载不触发、续局静默跑旧代码，无任何报错。签名本身
/// 只回答「变没变」（路径 + 字节数 + mtime），内容精度由 manifest 的 sha256
/// 在装载时承担——两步分工是既定口径，不为 resume 重算全量哈希。
fn watched_files(dir: &Path) -> Vec<String> {
    let mut files = vec![
        MOD_SOURCE_FILE.to_owned(),
        MANIFEST_FILE.to_owned(),
        PACK_FILE.to_owned(),
    ];
    // 配置的实际解析结果必须纳管：pack 未声明 `ygg:agent/config` 时
    // [`agent_config_path`] 回落 `data/config.json`，而下面只收**声明过的**
    // 路径——只改回落配置（如 `drive.maxSteps`）会让签名不变，热载直接 return，
    // 新阈值与结算口径永不生效且无告警。收「解析结果」而不是「声明」才能对齐
    // 监视面与实际使用面。
    files.push(agent_config_path(dir));
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
            // r2：代码模块（含入口——声明了 `code.entry` 时入口名不一定叫 mod.lua）。
            if let Some(entry) = pack["code"]["entry"].as_str() {
                files.push(entry.to_owned());
            }
            for module in pack["code"]["modules"].as_array().into_iter().flatten() {
                if let Some(path) = module.as_str() {
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

    /// 从回执信封取结构化摘要：读 `meta.<agent.metaKey>`（键名由 Mod 声明）。
    ///
    /// 缺失（旧 Mod / 该动作没调摘要写入接口）或形状不是对象 → [`Summary::unknown`]
    /// （安全值，调用方停等而不是猜）。
    fn from_receipt(receipt: &Json, meta_key: &str) -> Self {
        match &receipt["meta"][meta_key] {
            value if value.is_object() => Self::from_json(value),
            _ => Self::unknown(),
        }
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
    /// 事件名契约（`pack.json` 的 `agent` 段；热载时随之刷新）。
    contract: AgentContract,
    /// inbox 路由表（`pack.json` 的 `inboxRoutes` 段；热载时随之刷新）。
    inbox_routes: InboxRoutes,
    /// 自动推进阈值（`config.json` 的 `drive` 段；装载期校验，热载时随之刷新）。
    limits: DriveLimits,
    /// 结算口径（`config.json` 的 `settlement` 段；装载期校验 = 缺段即拒绝启动，
    /// 不把「口径缺失」推迟到结算那一刻才暴露）。
    settlement: SettlementPolicy,
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

/// 单段 drain 的「过慢」阈值（plan-r5 §2.6 临界区时长守护：观测不阻断）。
const DRAIN_SLOW_WARN_MS: u128 = 1_000;

/// `reset_artifacts` 的裁决结果（resume 失败路径与显式新局路径共用）。
enum ResetOutcome {
    /// 已执行重置（含目录本就干净的初次建代）；`had_old` = 目录里本来有旧产物
    /// （显式新局回执用它提示「已有旧产物，按新局重置」）。
    Reset { had_old: bool },
    /// 锁内重试读档成功：本局已被另一进程救活，状态已回灌——不重置、不按新局
    /// 开局，续当前盘上状态（resume 失败路径专用；显式新局不二次判定，不会返回它）。
    Recovered,
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
        let kernel = kernel_from(&bundle)?;
        // host 契约是**必填面**：事件名 / inbox 路由 / 结算口径 / 推进阈值全部来自
        // Mod 声明，缺任何一项即装载期失败（宿主不保留任何默认值，fail-closed）。
        let contract = AgentContract::load(&mod_dir)?;
        let inbox_routes = InboxRoutes::load(&mod_dir)?;
        let config_rel = agent_config_path(&mod_dir);
        let limits = DriveLimits::load(&mod_dir.join(&config_rel))?;
        let settlement = SettlementPolicy::load(&mod_dir.join(&config_rel))?;
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
            config_rel,
            contract,
            inbox_routes,
            limits,
            settlement,
            mod_label: mod_dir.display().to_string(),
            autosave_path: run_dir.join(AUTOSAVE_FILE),
            events_path: run_dir.join(EVENTS_FILE),
            meta_path: run_dir.join(META_FILE),
            inbox_path: run_dir.join(INBOX_FILE),
            inbox_cursor_path: run_dir.join(INBOX_CURSOR_FILE),
            run_dir,
            kernel,
            signature,
            notices: Vec::new(),
            reload: None,
            summary_cache: None,
            resumed: false,
        };

        // 残留 tmp 清理（W3 配套）：持锁 + 10s mtime 窗口，不误删活进程的 tmp。
        session.cleanup_stale_tmp();
        if args.resume {
            match session.load_autosave() {
                Ok(()) => session.resumed = true,
                Err(message) => match session.reset_artifacts(true) {
                    // 锁内重试读档成功：另一进程已把本局救活（重置后起步 / 修复
                    // 文件），状态已回灌——续当前盘上状态，**不**按新局开局。
                    // 以全新状态混进别人的局，正是二次判定要防的事。
                    ResetOutcome::Recovered => {
                        session.notices.push(format!(
                            "{message}；重试读档成功（另一进程已恢复本局），续当前盘上状态"
                        ));
                        session.resumed = true;
                    }
                    // 续局失败按新局开局：状态与日志必须同代，混代证据比丢证据更坏。
                    // 旧代事件已归档（含「可解析但不可装载」的档——可解析不等于
                    // 还活着，硬开新局才有同代起点）。
                    ResetOutcome::Reset { .. } => {
                        session.notices.push(format!("{message}；按新局开局"));
                    }
                },
            }
        } else if let ResetOutcome::Reset { had_old: true } = session.reset_artifacts(false) {
            session
                .notices
                .push(format!("run `{}` 已有旧产物，按新局重置", session.run_id));
        }
        // seq 不再有进程内计数器：每步在持久化临界区内 count_events 重算（P0-3 / B1），
        // 缓存即回到「多进程各自基线」的原点。
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

    /// 唯一临界区入口（P0-1）：持久化面的多步临界操作（seq 分配 + 事件 +
    /// 存档 / drain / reset）都经它持锁。f 拿 `&mut Self`，避免「持锁」与
    /// 「改状态」的借用冲突；重入（depth>0）时直接执行不重复取锁；panic
    /// 展开时 guard 的 Drop 归零 depth 并删锁文件（U2 的断言面）。
    fn with_run_lock<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        let _guard = acquire_run_lock(&self.run_dir)?;
        f(self)
    }

    /// 清理残留的 autosave tmp（崩溃 / kill -9 留下）：只删 mtime 早于
    /// CLOSING_STALE_MS 的（10s 窗口与 closing 心跳同源，plan-r5 §2.4）；新鲜
    /// tmp 可能有活进程正在写，绝不误删（R6）。持锁执行；清到即告警（B3）。
    fn cleanup_stale_tmp(&mut self) {
        let prefix = format!("{AUTOSAVE_FILE}.tmp.");
        let outcome = self.with_run_lock(|session| -> Result<(), String> {
            let entries = match fs::read_dir(&session.run_dir) {
                Ok(entries) => entries,
                Err(_) => return Ok(()), // 目录读不到 = 无残留可清
            };
            for entry in entries.flatten() {
                if !entry.file_name().to_string_lossy().starts_with(&prefix) {
                    continue;
                }
                // mtime 读不到 → 无法证明陈旧 → 保守跳过（宁可残留不可误删）。
                let Some(age) = entry
                    .metadata()
                    .ok()
                    .and_then(|meta| meta.modified().ok())
                    .and_then(|time| SystemTime::now().duration_since(time).ok())
                    .map(|elapsed| elapsed.as_millis())
                else {
                    continue;
                };
                if age <= CLOSING_STALE_MS {
                    continue;
                }
                match fs::remove_file(entry.path()) {
                    Ok(()) => session
                        .notices
                        .push(format!("清理残留 tmp：{}", entry.path().display())),
                    Err(err) => session.notices.push(format!(
                        "清理残留 tmp 失败：{}：{err}",
                        entry.path().display()
                    )),
                }
            }
            Ok(())
        });
        if let Err(message) = outcome {
            self.notices.push(message);
        }
    }

    /// 新局重置（P0-5 / W4）：归档事件日志（**不截断**——`events.jsonl.reset.<纳秒>`）
    /// + 移除旧 autosave + 显式建空文件 + 初始存档同临界区。
    ///
    /// `recheck`：进锁后二次判定「本局是否已被另一进程救活」（resume 失败路径传
    /// `true`；显式新局路径传 `false`，用户明确要求重置不跳过）。
    fn reset_artifacts(&mut self, recheck: bool) -> ResetOutcome {
        let outcome = self.with_run_lock(|session| -> Result<ResetOutcome, String> {
            // 二次判定（锁内）：不猜「证据」，直接重试读档。本进程首次读档失败后、
            // 拿到锁之前，另一进程可能已把本局救活（归档重置后起步 / 修复文件）——
            // 此刻盘上状态**可装载**就续它：状态与内存同源，无混代；且不归档
            // 别人的行。证据启发式（events 非空 + autosave 可解析）不可靠：
            // ① 可解析 ≠ 可装载（版本不符会误判「已恢复」→ 以全新状态混入旧代
            // 事件日志，实测 RED）；② 读档恢复可能只进内存，不留盘面证据（漏判）。
            // 重试读档是唯一忠实判据（终态：不可装载的局不存在活着的使用者）。
            if recheck && session.load_autosave().is_ok() {
                return Ok(ResetOutcome::Recovered);
            }
            let had_old = session.events_path.exists() || session.autosave_path.exists();
            if session.events_path.exists() {
                let archived = session
                    .run_dir
                    .join(format!("{EVENTS_FILE}.reset.{}", now_nanos()));
                fs::rename(&session.events_path, &archived)
                    .map_err(|err| format!("归档事件日志失败：{err}"))?;
                session
                    .notices
                    .push(format!("events 已归档至 {}", archived.display()));
                // 归档堆积提示（R7）：只提示不自动清理（自动删证据违反「保留原始
                // 证据」的需求精神）；不改 ok / 退出码。
                let archived_count = fs::read_dir(&session.run_dir).map_or(0, |entries| {
                    entries
                        .flatten()
                        .filter(|entry| {
                            entry
                                .file_name()
                                .to_string_lossy()
                                .starts_with(&format!("{EVENTS_FILE}.reset."))
                        })
                        .count()
                });
                if archived_count > 5 {
                    session.notices.push(format!(
                        "events 归档已有 {archived_count} 份，建议人工清理（宿主不自动删证据）"
                    ));
                }
            }
            if session.autosave_path.exists() {
                if let Err(err) = fs::remove_file(&session.autosave_path) {
                    session.notices.push(format!("移除旧 autosave 失败：{err}"));
                }
            }
            // 显式建空文件：归档后立刻有新一代日志（与旧日志不再混代）。
            if let Err(err) = File::create(&session.events_path) {
                session.notices.push(format!("新建事件日志失败：{err}"));
            }
            // 初始存档同临界区：状态与日志同代。
            if let Err(message) = session.save_now() {
                session.notices.push(message);
            }
            Ok(ResetOutcome::Reset { had_old })
        });
        match outcome {
            Ok(reset) => reset,
            Err(message) => {
                // 锁超时等临界区故障：不重试（避免与另一进程连环互撞），
                // 告警后按「目录里有旧东西」处理，由调用方继续新局流程。
                self.notices.push(format!("重置产物失败：{message}"));
                ResetOutcome::Reset { had_old: true }
            }
        }
    }

    /// 事件日志既有条数（续局 seq 起点）。
    fn count_events(&self) -> u64 {
        event_count(&self.events_path)
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
    ///
    /// W5 半修（plan-r5 §2.7）：run.json 是人读元信息面（两进程并发时值相同或
    /// 极近，丢更新无实质后果），**不加锁**；但改走 tmp+rename 消「半截读」。
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
        let mut text = match serde_json::to_string_pretty(&meta) {
            Ok(text) => text,
            Err(err) => {
                eprintln!("[警告] 序列化 {} 失败：{err}", self.meta_path.display());
                return;
            }
        };
        text.push('\n');
        if let Err(err) = atomic_write(&self.meta_path, text.as_bytes()) {
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
        // tmp+rename 原子替换（W3）：读者要么看到旧全文、要么新全文，绝无半截；
        // 本函数只在持久化临界区内被调用（dispatch_event / reset_artifacts）。
        atomic_write(&self.autosave_path, save.to_string().as_bytes())
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

    /// 回执标准字段：一次只读动作（契约的 `summaryEvent`）取 Mod **自报**的摘要。
    ///
    /// 这里**不读 autosave**：宿主对「结局 / 冲突 / 待验收提案 / 收口前一步」的
    /// 全部认知只来自这一条回执。缺失（旧 Mod）→ [`Summary::unknown`]，调用方
    /// 按安全值处理并停等。结果缓存到本步结束（一次派发只问一次）。
    fn summary(&mut self) -> Summary {
        if let Some(cached) = &self.summary_cache {
            return cached.clone();
        }
        let summary_event = self.contract.summary_event.clone();
        let payload = self.contract.default_payload.clone();
        let meta_key = self.contract.meta_key.clone();
        let envelope = parse_envelope(&self.kernel.dispatch(&summary_event, &payload));
        let summary = if envelope["ok"] == Json::Bool(true) {
            Summary::from_receipt(&envelope, &meta_key)
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

    /// 消费 inbox 新行：**整段持锁**（P0-6 / W6——两进程同时 drain 不再重复
    /// 消费同一行），逐行 dispatch（重入不二次取锁），成功后前移 cursor。
    ///
    /// 崩溃安全序不变（plan-r5 §2.6「不修 W8」）：dispatch（Mod 侧入队 + 自动
    /// 存档）→ 成功才写 cursor。「存档成功、cursor 未写」的重启会重放同一行，
    /// 靠 external_id 幂等去重；cursor 缺失/被截仍全量重放（W8 归 P1）。Mod 侧
    /// 拒绝（done / 队列满 / 载荷）走叙事面，**不算成功**：不推 cursor，留给
    /// 调用方报错。坏行跳过策略不变（D5 不在 P0）。
    fn drain_inbox(&mut self) -> InboxDrain {
        let started = Instant::now();
        let outcome = self.with_run_lock(|session| Ok(session.drain_inbox_locked()));
        let drain = outcome.unwrap_or_else(|message| {
            // 锁超时：本段未消费任何行（cursor 未动），报错留给调用方非零退出。
            let mut drain = InboxDrain::default();
            drain.error = Some(message);
            drain
        });
        // 临界区时长守护（P0-11）：>1s 打 drain_slow warning，不阻断（B3）。
        let elapsed_ms = started.elapsed().as_millis();
        if elapsed_ms > DRAIN_SLOW_WARN_MS {
            self.notices.push(format!(
                "drain_slow：inbox 消费耗时 {elapsed_ms}ms（持久化临界区偏长，观测面）"
            ));
        }
        drain
    }

    /// drain 的锁内本体（旧逻辑原样；内部 dispatch_event 重入不加锁）。
    fn drain_inbox_locked(&mut self) -> InboxDrain {
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
            // 长临界区心跳：逐行续命锁文件 mtime，防等待者按 LOCK_STALE_MS
            // 误判活锁为崩溃残留而抢占（双持锁 = 互斥失效，实测重复 seq）。
            run_lock_heartbeat();
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
            // kind 缺省 = add（旧行只有 title / description / external_id）。
            let kind = record["kind"].as_str().unwrap_or("add");
            // kind → 动作 + 段序完全来自 Mod 声明（宿主不枚举 kind）。
            let Some(route) = self.inbox_routes.get(kind) else {
                consumed += 1;
                drain.skipped += 1;
                self.write_inbox_cursor(consumed);
                self.notices.push(format!(
                    "inbox 第 {consumed} 行 kind 未在 pack.json 的 inboxRoutes 声明，已跳过"
                ));
                continue;
            };
            let Some(fields) = self.inbox_routes.payload_for(route, &record) else {
                consumed += 1;
                drain.skipped += 1;
                self.write_inbox_cursor(consumed);
                self.notices.push(format!(
                    "inbox 第 {consumed} 行缺字段（external_id / 声明的段全为空），已跳过"
                ));
                continue;
            };
            // payload 必须是 **JSON 字符串**（内核要求 JSON 标量），不是裸 wire format。
            let payload = json!(fields).to_string();
            let action = route.action.clone();
            let receipt = self.dispatch_event(&action, &payload);
            let rejected =
                receipt["ok"] != Json::Bool(true) || dispatch_rejected(&self.contract, &receipt);
            if rejected {
                // 内核错误信封（载荷 / 校验失败）与 Mod 侧拒绝同属「未入队」：
                // cursor 不前移，留给调用方报错与重启重放。
                drain.error = Some(first_rejection(&self.contract, &receipt));
                break;
            }
            consumed += 1;
            drain.applied += 1;
            if narrative_lines(&receipt)
                .iter()
                .any(|line| self.contract.is_replay(line))
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
        //    契约（agent / inboxRoutes / drive）也在这一步现读：任一项不合法即整体
        //    判定热载失败——**不**存在「内核换了、契约沿用旧的」半更新态。
        let outcome = ModBundle::load(&self.mod_dir).and_then(|bundle| {
            let contract = AgentContract::load(&self.mod_dir)?;
            let inbox_routes = InboxRoutes::load(&self.mod_dir)?;
            let limits = DriveLimits::load(&self.mod_dir.join(&self.config_rel))?;
            let settlement = SettlementPolicy::load(&self.mod_dir.join(&self.config_rel))?;
            restore_from(&bundle, save_text.as_bytes())
                .map(|kernel| (kernel, contract, inbox_routes, limits, settlement))
        });
        match outcome {
            Ok((kernel, contract, inbox_routes, limits, settlement)) => {
                self.kernel = kernel;
                // 换了 Mod 就换了契约（事件名 / inbox 路由 / 摘要约定 / 推进阈值）。
                self.contract = contract;
                self.inbox_routes = inbox_routes;
                self.limits = limits;
                self.settlement = settlement;
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

    /// 派发一步：dispatch（锁外）→ 持久化临界区（seq 分配 + 事件行 + 自动存档）→ 回执。
    ///
    /// P0-3（plan-r5 §2.3）：三件事同临界区 —— seq 改为**锁内 count_events 重算**
    /// （唯一事实源，删进程内计数器），事件行单次 write_all，成功步存档同临界区。
    /// 锁内失败的失败面（写 event 成功、写存档失败的偏斜）在同一进程内被消除；
    /// 跨进程的「事件与状态原子同提交」不是 P0 的承诺。
    fn dispatch_event(&mut self, name: &str, payload: &str) -> Json {
        let envelope = parse_envelope(&self.kernel.dispatch(name, payload)); // 锁外：Lua 计算
                                                                             // 真实派发可能已经改变语义面（阶段 / 结局 / 提案 / 冲突）→ 摘要缓存失效。
        self.summary_cache = None;
        let ok = envelope["ok"] == Json::Bool(true);
        let narrative = narrative_lines(&envelope);
        let ending = narrative
            .iter()
            .find(|line| line.starts_with(&self.contract.ending_prefix))
            .cloned();
        let ending_line = ending.as_deref();

        // 持久化临界区：seq 分配 + 事件行 + 存档（仅成功步）三件事同一把锁。
        let persisted = self.with_run_lock(|session| {
            let seq = session.count_events(); // 锁内重算：唯一分配点（B1）
            session.append_event(seq, name, payload, ok, narrative.len(), ending_line);
            // 自动存档：仅成功步（失败步不改状态，重写同一份 autosave 无意义）。
            if ok {
                session.save_now()?;
            }
            Ok(seq)
        });
        let seq = match persisted {
            Ok(seq) => seq,
            Err(message) => {
                // 锁超时 / 存档失败：状态已进内存但未落盘 —— 回执 fail-closed
                //（B2：不声称成功；events.jsonl 未新增行）。单发路径凭 ok=false
                // 非零退出，文案沿用 acquire_dir_lock 的超时报错。
                eprintln!("error: 持久化失败：{message}");
                let failed_envelope = json!({
                    "ok": false,
                    "error": {"code": "PERSIST", "message": message},
                });
                let pending = self.state_envelope()["pendingChoice"].clone();
                let receipt = self.build_receipt(
                    name,
                    self.count_events(),
                    &failed_envelope,
                    &narrative,
                    ending_line,
                    pending,
                );
                self.reload = None;
                return receipt;
            }
        };

        // 待决选择：成功信封自带；失败信封补一次 state（不推进状态），
        // 让回执在「越权动作被拒」之后仍能指引下一步。
        let pending = if ok {
            envelope["pendingChoice"].clone()
        } else {
            self.state_envelope()["pendingChoice"].clone()
        };

        let receipt = self.build_receipt(name, seq, &envelope, &narrative, ending_line, pending);
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
                // 序列化先完成：失败不留半行。单次 write_all 消除 writeln! 的
                // Formatter 分段写（W2）；跨进程原子性由外层锁保证，write_all
                // 只负责同进程内的一次 write(2)（行 < 4KiB 时内核一次写入）。
                match serde_json::to_string(&record) {
                    Ok(mut line) => {
                        if line.len() > 16 * 1024 {
                            self.notices.push(format!(
                                "事件行超大（{} 字节，seq {seq}）：异常 payload 的可能信号",
                                line.len()
                            ));
                        }
                        line.push('\n');
                        if let Err(err) = file.write_all(line.as_bytes()) {
                            self.notices
                                .push(format!("写入 {} 失败：{err}", self.events_path.display()));
                        }
                    }
                    Err(err) => self.notices.push(format!("序列化事件记录失败：{err}")),
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
        // TD1：结构化 meta 原样透传（宿主只读其中的 `summary`，不解释其他键；
        // 它不进叙事、不进存档，只作为回执的独立字段给调用方用）。
        if envelope.get("meta").is_some() {
            receipt["meta"] = envelope["meta"].clone();
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
/// 标题 / 描述里的 `|` 换成全角 `｜`：竖线是 Mod 侧的分段分隔符，混进去会把
/// 描述尾部误当幂等键。
fn split_requirement(text: &str) -> (String, String) {
    let (title, description) = match text.split_once('|') {
        Some((head, tail)) => (head.trim(), tail.trim()),
        None => (text.trim(), ""),
    };
    (sanitize_field(title), sanitize_field(description))
}

/// Mod 侧拒绝的识别（叙事前缀）：内核信封仍是 ok，宿主必须显式看叙事。
///
/// 前缀集合由 Mod 契约声明（`agent.rejectionPrefixes`），宿主不认识具体文案。
fn dispatch_rejected(contract: &AgentContract, receipt: &Json) -> bool {
    narrative_lines(receipt)
        .iter()
        .any(|line| contract.is_rejection(line))
}

/// 拒绝原因（回执里的第一条拒绝叙事；缺失则给错误码文案）。
fn first_rejection(contract: &AgentContract, receipt: &Json) -> String {
    narrative_lines(receipt)
        .into_iter()
        .find(|line| contract.is_rejection(line))
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

/// inbox 现有非空行数（宿主的 audit 计数；与消费侧的计数口径一致）。
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
    /// true = 同进程重入占位（不持真实锁文件：Drop 不删文件、不重置 depth）。
    reentrant: bool,
}

impl Drop for DirLock {
    fn drop(&mut self) {
        if self.reentrant {
            return;
        }
        RUN_LOCK_DEPTH.with(|depth| depth.set(0));
        RUN_LOCK_FILE.with(|slot| *slot.borrow_mut() = None);
        lock_trace("released");
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
                return Ok(DirLock {
                    path,
                    reentrant: false,
                });
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

thread_local! {
    /// 进程内锁重入记账（P0-1 / C2）：1 = 本进程已持 `coordination.lock`，重入
    /// 不再 acquire。放在**公共锁入口**而不是 Session 上：coordination_update
    /// 等协调协议路径不经过 Session 也要取同一把锁，C3「单锁单记账」必须覆盖
    /// 全部取锁点。单线程模型（C4：临界区内不得 spawn 线程取锁），thread_local 安全。
    static RUN_LOCK_DEPTH: Cell<u32> = Cell::new(0);
    /// 真实持锁期间的锁文件路径（心跳续命用）；重入占位与未持锁时为空。
    /// Drop（真实持有者）归零 depth 时同步清空，心跳不得越权触碰别人的锁。
    static RUN_LOCK_FILE: RefCell<Option<PathBuf>> = RefCell::new(None);
}

/// 调试追踪（仅 debug 构建；`YGG_LOCK_TRACE=1` 时向 stderr 打
/// `acquired / reentered / released` + depth + pid，供 P0-1 重入取证）。
#[cfg(debug_assertions)]
fn lock_trace(stage: &str) {
    if std::env::var_os("YGG_LOCK_TRACE").is_some_and(|value| value == "1") {
        eprintln!(
            "[lock-trace] {stage} depth={} pid={}",
            RUN_LOCK_DEPTH.with(Cell::get),
            std::process::id()
        );
    }
}
#[cfg(not(debug_assertions))]
fn lock_trace(_stage: &str) {}

/// 重入安全的锁入口（P0-1 持久化面唯一临界区入口）：depth>0 = 本进程已持锁，
/// 直接返回重入占位（不 acquire、不加 depth）；只有真实 acquire 走
/// `acquire_dir_lock`（现有语义不变：create_new + mtime 陈旧抢占）并置 depth=1。
fn acquire_run_lock(run_dir: &Path) -> Result<DirLock, String> {
    if RUN_LOCK_DEPTH.with(Cell::get) > 0 {
        lock_trace("reentered");
        return Ok(DirLock {
            path: run_dir.join(COORDINATION_LOCK),
            reentrant: true,
        });
    }
    let dir_lock = acquire_dir_lock(run_dir)?;
    RUN_LOCK_DEPTH.with(|depth| depth.set(1));
    RUN_LOCK_FILE.with(|slot| *slot.borrow_mut() = Some(dir_lock.path.clone()));
    lock_trace("acquired");
    debug_assert_eq!(RUN_LOCK_DEPTH.with(Cell::get), 1);
    Ok(dir_lock)
}

/// 长临界区心跳：bump 锁文件 mtime，防活进程的合法长临界区被按「陈旧残留」
/// 抢占。陈旧抢占判据是 mtime（`LOCK_STALE_MS`），而 mtime 只在 acquire 时写
/// 一次——临界区一旦超过 2s，等待者就会删锁重建、与持有者双持锁（实测：
/// 4000 行 inbox drain ≈3.1s，等待者在 2.2s 即进锁，单轮并发出现数百重复
/// seq）。drain 等无界循环必须逐行续命；其余临界区（seq+事件+存档 /
/// coordination 读写）亚秒级，不需要心跳。只在本进程真实持锁时生效：
/// 未持锁不动任何文件（绝不 bump 别人的锁），重入时续命的是外层真实锁。
fn run_lock_heartbeat() {
    if RUN_LOCK_DEPTH.with(Cell::get) == 0 {
        return;
    }
    RUN_LOCK_FILE.with(|slot| {
        if let Some(path) = slot.borrow().as_ref() {
            if let Ok(file) = fs::OpenOptions::new().write(true).open(path) {
                let _ = file.set_modified(SystemTime::now());
            }
        }
    });
}

/// 锁内读—改—写 coordination.json（短临界区，不跨文件 IO）。
fn coordination_update<T>(run_dir: &Path, f: impl FnOnce(&mut Json) -> T) -> Result<T, String> {
    let _guard = acquire_run_lock(run_dir)?;
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
    let mut session = match Session::open(args) {
        Ok(session) => session,
        Err(message) => {
            let event = args.event.clone().unwrap_or_default();
            return single_shot_error(args, &event, &message);
        }
    };
    // 缺 --event 时用 Mod 声明的入口事件（宿主不持有任何事件名）。
    let event = args
        .event
        .clone()
        .unwrap_or_else(|| session.contract.entry_event.clone());
    // 载荷缺省 = Mod 契约声明（宿主不持有任何 payload 字面量）。
    let payload = args
        .payload
        .clone()
        .unwrap_or_else(|| session.contract.default_payload.clone());

    // 单发每次派发前检查热载：Agent 侧改了 mod，下一条动作即生效。
    session.maybe_reload();

    // 派发前先消费 inbox：异步补入的需求不能因为「本步是 Agent 显式动作」而漏掉。
    // 被拒（局末 / 队列满 / 载荷）不阻碍本次派发，但会落 `inbox.remaining` + 警告。
    let drain = session.drain_inbox();

    let mut receipt = session.dispatch_event(&event, &payload);
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

/// 本局的 Mod 目录：`--mod` 优先，其次 run.json 里记录的本局 mod 路径。
///
/// 两者都没有 → 可读错误。契约是**必填面**：没有 Mod 目录就读不到声明，
/// 宿主不会猜任何默认值（fail-closed）。
fn resolve_mod_dir(args: &Args, run_dir: &Path) -> Result<PathBuf, String> {
    args.mod_dir
        .clone()
        .or_else(|| {
            read_meta(&run_dir.join(META_FILE))
                .and_then(|meta| meta["mod"].as_str().map(PathBuf::from))
        })
        .ok_or_else(|| {
            format!(
                "本局没有可用的 Mod 目录（缺 --mod，且 {} 里没有记录本局的 mod 路径）——\
                 契约（pack.json / config.json）必须现读，无法继续",
                run_dir.join(META_FILE).display()
            )
        })
}

/// 宿主 CLI 写命令 → `(路由表, kind)`：完全由 Mod 的 `inboxRoutes` 声明决定。
///
/// 未声明该写命令 → 可读拒绝（调用方报错退出，**不**静默写一行宿主猜的记录）。
/// 同时返回整张表：记录字段名（幂等键 / 预成形载荷）也是契约的一部分。
fn resolve_write_binding(
    args: &Args,
    run_dir: &Path,
    flag: &str,
) -> Result<(InboxRoutes, String), String> {
    let mod_dir = resolve_mod_dir(args, run_dir)?;
    let routes = InboxRoutes::load(&mod_dir)?;
    if routes.is_empty() {
        return Err(format!(
            "本 Mod 的 pack.json `inboxRoutes` 未声明任何 kind（不支持异步 inbox）——`--{flag}` 无处可写"
        ));
    }
    let Some(kind) = routes.write_binding(flag).map(|(kind, _)| kind.to_owned()) else {
        return Err(format!(
            "本 Mod 的 pack.json `inboxRoutes` 未声明 `--{flag}` 的写命令（需要某条路由带 \
             `\"write\": {{\"flag\": \"{flag}\", \"shape\": \"named|payload\"}}`；{CONTRACT_DOC}）"
        ));
    };
    Ok((routes, kind))
}

/// `--requirement`：只往 inbox.jsonl 追加一行，不读不写 autosave 后退出。
///
/// 这是「新需求随时能进」的真实异步入口：本局在另一个进程里跑时，
/// 这里只做 `OpenOptions::append`，不存在与运行中局的状态竞争。
fn run_inbox_append(args: &Args) -> i32 {
    let Some(write) = args.inbox.clone() else {
        return 2;
    };
    let run_dir = args.runs_root.join(&args.run_id);
    if let Err(err) = fs::create_dir_all(&run_dir) {
        eprintln!("error: 创建 run 目录 {} 失败：{err}", run_dir.display());
        return 2;
    }
    // 记录形态与 kind 都来自 Mod 声明：契约读不到 / 没声明该写命令 → fail-closed
    // （明确拒绝 + 退出码 3，绝不静默写一行宿主猜出来的记录）。
    let (routes, kind) = match resolve_write_binding(args, &run_dir, write.flag) {
        Ok(binding) => binding,
        Err(message) => {
            eprintln!("error: {message}；本条未写入");
            return 3;
        }
    };
    let Some(route) = routes.get(&kind) else {
        eprintln!("error: 内部错误：写命令绑定的 kind `{kind}` 不在路由表里；本条未写入");
        return 3;
    };
    let Some(shape) = route.write.as_ref().map(|binding| binding.shape) else {
        eprintln!("error: 内部错误：write_binding 只返回带 write 的路由；本条未写入");
        return 3;
    };
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
    // 记录字段名全部来自契约保留键：宿主不知道幂等键 / 载荷字段叫什么。
    let external_field = routes.external_id_field();
    let payload_field = routes.payload_field();
    let mut record = json!({
        "kind": kind,
        "created_at": now_secs(),
    });
    record[external_field] = json!(new_external_id(&args.run_id, inbox_line_count(&path)));
    match shape {
        // 命名段形态：「首段|余下全部」分别落到路由声明的**前两个**非幂等键段
        // （典型就是标题 / 描述；段名由 Mod 声明，宿主不认业务字段名）。
        WriteShape::Named => {
            let (first, rest) = split_requirement(&write.value);
            let mut named = route.named_fields(external_field);
            if let Some(field) = named.next() {
                record[field.as_str()] = json!(first);
            }
            if let Some(field) = named.next() {
                record[field.as_str()] = json!(rest);
            }
        }
        // 预成形 wire 形态：整串载荷进契约声明的载荷字段（宿主不解析、不重排）。
        WriteShape::Payload => record[payload_field] = json!(write.value.trim()),
    }
    let receipt = match append_inbox_line(&path, &record) {
        Ok(()) => {
            // 回执键 = 记录的契约字段名（幂等键 / 载荷 / 各命名段）：调用方不必猜。
            let mut receipt = json!({
                "ok": true,
                "inbox": path.display().to_string(),
                "kind": kind,
            });
            receipt[external_field] = record[external_field].clone();
            receipt[payload_field] = record.get(payload_field).cloned().unwrap_or(Json::Null);
            for field in route.named_fields(external_field) {
                receipt[field.as_str()] = record.get(field.as_str()).cloned().unwrap_or(Json::Null);
            }
            receipt
        }
        Err(message) => {
            eprintln!("error: {message}");
            json!({
                "ok": false,
                "inbox": path.display().to_string(),
                "kind": kind,
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

#[cfg(test)]
mod contract_tests {
    use super::{AgentContract, Path};
    use serde_json::json;

    /// `agent` 段的八个字段名（与本模块上一层的 `AGENT_FIELDS` 同集合；这里显式列
    /// 出来是为了让「逐个缺字段」用例覆盖完整形状）。
    const DECLARED_FIELDS: [&str; 8] = [
        "entryEvent",
        "choiceEvent",
        "summaryEvent",
        "endingPrefix",
        "metaKey",
        "rejectionPrefixes",
        "replayPrefix",
        "defaultPayload",
    ];

    /// 完整合法段（错误用例在此基础上逐字段挖掉）。
    fn full_section() -> serde_json::Value {
        json!({
            "entryEvent": "mod.entry",
            "choiceEvent": "mod.choice",
            "summaryEvent": "mod.summary",
            "endingPrefix": "END:",
            "metaKey": "digest",
            "rejectionPrefixes": ["NOPE:", "BAD:"],
            "replayPrefix": "AGAIN:",
            "defaultPayload": "-"
        })
    }

    /// 段缺失 / 八个字段缺任一 → 硬错误（点名缺失字段 + 形状示例 + 文档定位）。
    #[test]
    fn missing_section_or_field_is_a_hard_error() {
        let message = AgentContract::from_pack(&json!({"entries": []})).expect_err("必须报错");
        assert!(message.contains("`agent` 段"), "{message}");
        assert!(
            message.contains("\"entryEvent\"") && message.contains("\"defaultPayload\""),
            "应给出完整形状示例：{message}"
        );
        assert!(message.contains("README.md"), "应给出文档定位：{message}");
        // 读不到 pack.json 也是硬错误（不兜底）
        assert!(AgentContract::load(Path::new("/nonexistent/ygg-mod")).is_err());

        for field in DECLARED_FIELDS {
            let mut section = full_section();
            section.as_object_mut().expect("对象").remove(field);
            let message =
                AgentContract::from_pack(&json!({"agent": section})).expect_err("缺字段必须报错");
            assert!(
                message.contains(&format!("`{field}`")),
                "必须点名缺失字段 `{field}`：{message}"
            );
            assert!(message.contains("README.md"), "{message}");
        }
        // 空串 / 非字符串 / 段不是对象 → 同样硬错误
        for broken in [
            json!({"agent": []}),
            json!({"agent": "x"}),
            json!({"agent": {"entryEvent": 7, "choiceEvent": "c", "summaryEvent": "s", "endingPrefix": "E", "metaKey": "m", "rejectionPrefixes": ["R"], "replayPrefix": "P", "defaultPayload": "0"}}),
        ] {
            assert!(AgentContract::from_pack(&broken).is_err(), "{broken}");
        }
    }

    /// 每个字段的类型 / 空值约束都单独成立（含拒绝前缀数组的三道检查）。
    #[test]
    fn field_types_and_emptiness_are_enforced() {
        let broken = |field: &str, value: serde_json::Value| {
            let mut section = full_section();
            section[field] = value;
            AgentContract::from_pack(&json!({"agent": section})).expect_err("非法字段必须报错")
        };
        for field in [
            "entryEvent",
            "choiceEvent",
            "summaryEvent",
            "endingPrefix",
            "metaKey",
            "replayPrefix",
        ] {
            let message = broken(field, json!("   "));
            assert!(
                message.contains(&format!("`{field}`")) && message.contains("空串"),
                "空串必须被拦：{message}"
            );
            assert!(broken(field, json!(42)).contains(&format!("`{field}`")));
        }
        // 拒绝前缀：空数组 / 含空串 / 非数组 → 三道都拦
        for value in [json!([]), json!(["OK:", ""]), json!("OK:")] {
            let message = broken("rejectionPrefixes", value);
            assert!(message.contains("`rejectionPrefixes`"), "{message}");
        }
        // 入口载荷允许空串（显式声明「空载荷」是合法意图），但不是字符串即错
        let mut section = full_section();
        section["defaultPayload"] = json!("");
        let contract = AgentContract::from_pack(&json!({"agent": section})).expect("空载荷合法");
        assert_eq!(contract.default_payload, "");
        assert!(broken("defaultPayload", json!(null)).contains("`defaultPayload`"));
    }

    /// 声明了就用声明值（换 Mod 只改 pack.json，宿主不认字面量）。
    #[test]
    fn reads_the_eight_declared_fields() {
        let contract =
            AgentContract::from_pack(&json!({"agent": full_section()})).expect("合法 agent 段");
        assert_eq!(contract.entry_event, "mod.entry");
        assert_eq!(contract.choice_event, "mod.choice");
        assert_eq!(contract.summary_event, "mod.summary");
        assert_eq!(contract.ending_prefix, "END:");
        assert_eq!(contract.meta_key, "digest");
        assert_eq!(contract.rejection_prefixes, ["NOPE:", "BAD:"]);
        assert_eq!(contract.replay_prefix, "AGAIN:");
        assert_eq!(contract.default_payload, "-");
        // 判定面也只认声明值
        assert!(contract.is_rejection("NOPE: 动作越阶"));
        assert!(contract.is_rejection("BAD: 载荷非法"));
        assert!(!contract.is_rejection("动作越阶"));
        assert!(contract.is_replay("AGAIN: 同键重放"));
        assert!(!contract.is_replay("NOPE: 动作越阶"));
    }
}

#[cfg(test)]
mod inbox_routes_tests {
    use super::{InboxRoutes, Path};
    use serde_json::{json, Value as Json};

    /// seed Mod 的契约目录（本仓内的真实契据，测试直接读它——避免在测试里
    /// 复刻一份 kind 表，哪边改了都只有一处）。
    fn seed_mod_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.ygg/mods/agent-seed")
    }

    /// 三个保留键（与 kind 同层）。测试里显式声明：宿主不得内置任何字段名。
    /// 后缀标记故意用与真 Mod 不同的值，以证明它只认声明。
    fn record_keys() -> Json {
        json!({
            "externalIdField": "external_id",
            "payloadField": "payload",
            "extMarker": "~ext~"
        })
    }

    /// 保留键 + 若干 kind → 一段合法声明。
    fn section(kinds: Json) -> Json {
        let mut table = record_keys();
        for (kind, spec) in kinds.as_object().expect("对象") {
            table[kind] = spec.clone();
        }
        json!({"inboxRoutes": table})
    }

    /// 缺段 / 缺保留键 / 单条非法 → 硬错误；只声明保留键（零 kind）才合法。
    #[test]
    fn missing_section_or_reserved_key_is_a_hard_error() {
        let message = InboxRoutes::from_pack(&json!({"entries": []})).expect_err("必须报错");
        assert!(message.contains("`inboxRoutes` 段"), "{message}");
        assert!(message.contains("\"action\""), "应给出形状示例：{message}");
        assert!(message.contains("README.md"), "应给出文档定位：{message}");

        // 空表不再是合法声明：三个保留键必填（零 kind 仍然合法）
        let message =
            InboxRoutes::from_pack(&json!({"inboxRoutes": {}})).expect_err("缺保留键必须报错");
        assert!(message.contains("externalIdField"), "{message}");
        for key in ["externalIdField", "payloadField", "extMarker"] {
            let mut table = record_keys();
            table.as_object_mut().expect("对象").remove(key);
            let message = InboxRoutes::from_pack(&json!({"inboxRoutes": table}))
                .expect_err("缺保留键必须报错");
            assert!(
                message.contains(&format!("`{key}`")),
                "必须点名 `{key}`：{message}"
            );
            assert!(message.contains("README.md"), "{message}");
        }
        // 保留键空串 / 非字符串 → 同样硬错误
        for (key, value) in [
            ("externalIdField", json!("  ")),
            ("payloadField", json!(7)),
            ("extMarker", Json::Null),
        ] {
            let mut table = record_keys();
            table[key] = value;
            assert!(
                InboxRoutes::from_pack(&json!({"inboxRoutes": table})).is_err(),
                "`{key}` 非法值必须报错"
            );
        }

        let empty = InboxRoutes::from_pack(&section(json!({}))).expect("零 kind 合法");
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);
        assert_eq!(empty.external_id_field(), "external_id");
        assert_eq!(empty.payload_field(), "payload");
        assert_eq!(empty.ext_marker(), "~ext~");
        assert!(empty.write_binding("requirement").is_none());

        // 单条非法（保留键齐全）→ 逐项点名
        for (kinds, needle) in [
            (json!({"k": {"fields": ["a"]}}), "`action`"),
            (json!({"k": {"action": "a"}}), "`fields`"),
            (json!({"k": {"action": "a", "fields": []}}), "不得为空数组"),
            (
                json!({"k": {"action": "a", "fields": ["x"], "write": {"shape": "named"}}}),
                "`flag`",
            ),
            (
                json!({"k": {"action": "a", "fields": ["x"], "write": {"flag": "f", "shape": "guess"}}}),
                "`named` 或 `payload`",
            ),
            // 命名段形态必须至少有一个非幂等键段（否则写出来的记录无载荷）
            (
                json!({"k": {"action": "a", "fields": ["external_id"], "write": {"flag": "f", "shape": "named"}}}),
                "没有可写段",
            ),
        ] {
            let message = InboxRoutes::from_pack(&section(kinds)).expect_err("必须报错");
            assert!(message.contains(needle), "文案应点名 `{needle}`：{message}");
        }
        assert!(InboxRoutes::from_pack(&json!({"inboxRoutes": []})).is_err());
    }

    /// 声明表真的驱动派发：kind → action + 段序 + 写命令绑定，宿主零 kind 字面量。
    #[test]
    fn declared_routes_drive_the_dispatch() {
        let routes = InboxRoutes::from_pack(&section(json!({
            "mine": {
                "action": "requirement.add",
                "fields": ["title", "description", "external_id"],
                "write": {"flag": "requirement", "shape": "named"}
            },
            "raw": {
                "action": "proposal.submit",
                "fields": ["req", "rev", "stage", "summary", "artifacts", "external_id"],
                "write": {"flag": "proposal", "shape": "payload"}
            },
            "note": {
                "action": "requirement.conflict",
                "fields": ["reqs", "note"],
                "write": {"flag": "conflict", "shape": "payload"}
            }
        })))
        .expect("合法 inboxRoutes");
        assert_eq!(routes.len(), 3);
        let (kind, route) = routes.write_binding("requirement").expect("声明了写命令");
        assert_eq!(kind, "mine");
        assert_eq!(route.action, "requirement.add");
        assert!(route.declares_external(routes.external_id_field()));
        assert_eq!(
            routes
                .payload_for(
                    route,
                    &json!({"title": "t", "description": "d", "external_id": "e"})
                )
                .expect("命名段形态"),
            "t|d|e"
        );
        // 把幂等键声明成段的路径：按段补齐（不拼后缀）
        let (_, raw) = routes.write_binding("proposal").expect("声明了写命令");
        assert_eq!(
            routes
                .payload_for(
                    &raw,
                    &json!({"payload": "r1|1|analysis", "external_id": "e"})
                )
                .expect("预成形形态"),
            "r1|1|analysis|||e"
        );
        // 没把幂等键声明成段的路径：按契约声明的后缀标记注入
        let (_, note) = routes.write_binding("conflict").expect("声明了写命令");
        assert!(!note.declares_external(routes.external_id_field()));
        assert_eq!(
            routes
                .payload_for(&note, &json!({"payload": "r1,r2|说明", "external_id": "e"}))
                .expect("后缀形态"),
            "r1,r2|说明|~ext~e"
        );
        // 未声明的旗标 → None（调用方明确拒绝，不猜）
        assert!(routes.write_binding("amend").is_none());
        // 未声明的 kind 与保留键本身都不被当成 kind（= 坏行）
        assert!(routes.get("add").is_none());
        assert!(routes.get("externalIdField").is_none());
        assert!(routes.get("extMarker").is_none());
    }

    /// **向后兼容的字节面**：真实 seed Mod 契约对旧形态记录产出的载荷与改造前一致。
    #[test]
    fn seed_contract_reproduces_the_legacy_payloads() {
        let routes = InboxRoutes::load(&seed_mod_dir()).expect("seed Mod 的契约必须合法");
        let marker = routes.ext_marker();
        let payload = |flag: &str, record: Json| {
            let (_, route) = routes.write_binding(flag).expect("声明了写命令");
            routes.payload_for(route, &record)
        };

        // 命名段形态（`--requirement`）：title|description|external_id
        assert_eq!(
            payload(
                "requirement",
                json!({"title": "标题", "description": "描述", "external_id": "inbox-1"})
            ),
            Some("标题|描述|inbox-1".to_owned())
        );
        // 段内 `|` / 换行必须被清洗（否则破坏 wire format）
        assert_eq!(
            payload(
                "requirement",
                json!({"title": "a|b", "description": "c\nd", "external_id": "inbox-2"})
            ),
            Some("a｜b|c d|inbox-2".to_owned())
        );
        // 描述为空 → 中间段留空但位置对齐；幂等键缺失 → 坏行
        assert_eq!(
            payload(
                "requirement",
                json!({"title": "只有标题", "external_id": "inbox-3"})
            ),
            Some("只有标题||inbox-3".to_owned())
        );
        assert_eq!(payload("requirement", json!({"title": "t"})), None);

        // 预成形 wire 形态：proposal 的段数补齐（5 段补空 changes，6 段直接补，7 段原样）
        assert_eq!(
            payload(
                "proposal",
                json!({"payload": "r1|1|analysis|摘要|notes/a.md", "external_id": "inbox-5"})
            ),
            Some("r1|1|analysis|摘要|notes/a.md||inbox-5".to_owned())
        );
        assert_eq!(
            payload(
                "proposal",
                json!({"payload": "r1|1|analysis|摘要|notes/a.md|split=a,b", "external_id": "inbox-6"})
            ),
            Some("r1|1|analysis|摘要|notes/a.md|split=a,b|inbox-6".to_owned())
        );
        assert_eq!(
            payload(
                "proposal",
                json!({"payload": "r1|1|analysis|摘要|notes/a.md|split=a,b|my-ext", "external_id": "inbox-7"})
            ),
            Some("r1|1|analysis|摘要|notes/a.md|split=a,b|my-ext".to_owned())
        );

        // 其余写命令：载荷原样透传 + 契约声明的幂等后缀
        for (flag, raw) in [
            ("amend", "r1|新标题|新描述|依赖"),
            ("split", "r1|子A|子B"),
            ("merge", "r2,r3|合并标题|合并描述"),
            ("conflict", "r1,r2|说明"),
            ("resolve", "r1,r2|keep_both"),
        ] {
            assert_eq!(
                payload(flag, json!({"payload": raw, "external_id": "inbox-8"})),
                Some(format!("{raw}|{marker}inbox-8")),
                "`--{flag}` 的载荷必须保持改造前形态"
            );
        }
        // 空载荷 → 坏行
        assert_eq!(
            payload("split", json!({"payload": "  ", "external_id": "inbox-9"})),
            None
        );
    }
}

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
///
/// 省略载荷时用 Mod 契约声明的入口载荷（`default_payload`），宿主不持有字面量。
fn split_event_command(input: &str, default_payload: &str) -> Option<(String, String)> {
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
        session.events_path.display(),
        session.count_events(),
        session.count_events()
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

/// 一条结算判定项（`settlement.json` 的 `checks` 元素）。
///
/// `enabled` = Mod 在 `config.json` 的 `settlement` 段声明它是否参与 `ok`；
/// `passed` = 该项本身的事实（关掉也照实记录，便于人读诊断）。
struct SettlementCheck {
    name: &'static str,
    enabled: bool,
    passed: bool,
    /// 生效但未通过时的可读错误（进报告 `errors`）。
    error: String,
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
fn load_settlement_policy(run_dir: &Path, args: &Args) -> Result<SettlementPolicy, String> {
    let mod_dir = resolve_mod_dir(args, run_dir)?;
    SettlementPolicy::load(&mod_dir.join(agent_config_path(&mod_dir)))
}

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
            return 1;
        }
    };
    let pending_inbox = unconsumed_inbox_lines(&run_dir).max(pending_before);
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
        "run_id": args.run_id,
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
    //    与 ⑤ 的 `inboxConsumed` 同一开关：关掉它 = 明确接受结局后的残留行。
    let after = if policy.require_inbox_consumed {
        unconsumed_inbox_lines(&run_dir)
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

/// 事件日志既有条数（seq 唯一事实源；锁内调用）。
fn event_count(path: &Path) -> u64 {
    match fs::read_to_string(path) {
        Ok(text) => text.lines().filter(|line| !line.trim().is_empty()).count() as u64,
        Err(_) => 0,
    }
}

/// 宿主侧时钟（unix 纳秒）：只用于归档文件命名（同秒撞名不可能）。
fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos())
}

/// pretty JSON 落盘（人读证据面；末尾补换行便于 diff）。
fn write_json_pretty(path: &Path, value: &Json) -> Result<(), String> {
    let mut text = serde_json::to_string_pretty(value)
        .map_err(|err| format!("序列化 {} 失败：{err}", path.display()))?;
    text.push('\n');
    fs::write(path, text).map_err(|err| format!("写入 {} 失败：{err}", path.display()))
}

/// 原子写（tmp + rename）：读者要么看到旧全文、要么看到新全文，绝无半截（W3）。
/// tmp 名带 pid：固定名会让两个并发写者互踩 tmp 内容，rename 后拿到「别人的
/// 中间态」——pid 后缀是正确性要求，不是风格（plan-r5 §2.4）。
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let name = path.file_name().map_or_else(
        || format!(".atomic-{}", std::process::id()),
        |name| format!("{}.tmp.{}", name.to_string_lossy(), std::process::id()),
    );
    let tmp = path.with_file_name(name);
    let mut file =
        File::create(&tmp).map_err(|err| format!("写入 {} 失败：{err}", tmp.display()))?;
    file.write_all(bytes)
        .map_err(|err| format!("写入 {} 失败：{err}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|err| format!("替换 {} 失败：{err}", path.display()))
}

// ============================================================================
// §多文件 Mod 的热载（r4）：签名纳管模块文件 + 换装原子性
//
// 这两条是 r2 的核心承诺，且都只在宿主侧可观测：
//  ① `watched_files` 必须把 `pack.json` 的 `code.modules` 全部纳管——只监视
//     `mod.lua` 时，改 `src/*.lua` 而入口一字未动 → 签名不变 → 热载不触发、
//     续局静默跑旧代码，**没有任何报错**（这是最难查的一类 bug）；
//  ② `maybe_reload` 必须是"换得成就换、换不成就照旧跑"：成功时 `reloaded=true`
//     且新代码立即生效；失败时 `reloaded=false`、旧驱动继续服务，不留半更新。
// ============================================================================
#[cfg(test)]
mod multiscript_hot_reload_tests {
    use super::{fs, mod_signature, watched_files, Args, Drive, Path, PathBuf, Session};
    use serde_json::Value as Json;

    /// 入库 fixture 的目录（`tests/fixtures/multi-script`；与 r4 的三个测试目标同一份）。
    fn fixture_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/multi-script")
    }

    /// 递归复制（测试要改文件，不能动入库 fixture）。
    fn copy_dir(from: &Path, to: &Path) {
        fs::create_dir_all(to).expect("建目标目录");
        for item in fs::read_dir(from).expect("读源目录") {
            let item = item.expect("目录项");
            let source = item.path();
            let target = to.join(item.file_name());
            if source.is_dir() {
                copy_dir(&source, &target);
            } else {
                fs::copy(&source, &target).expect("复制文件");
            }
        }
    }

    /// fixture 的临时副本（每个用例独占目录）。
    fn fixture_copy(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ygg-agent-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        copy_dir(&fixture_dir(), &dir);
        dir
    }

    /// 单发模式的宿主参数（`--drive none`：测试自己派发，不推 stdin）。
    fn args_for(dir: &Path, run_id: &str) -> Args {
        Args {
            mod_dir: Some(dir.to_path_buf()),
            run_id: run_id.to_owned(),
            runs_root: dir.join("runs"),
            resume: false,
            settle: false,
            watch: false,
            event: None,
            payload: None,
            drive: Drive::None,
            inbox: None,
        }
    }

    /// 派发一步并取叙事行（信封必须 ok）。
    #[track_caller]
    fn narrate(session: &mut Session, event: &str) -> Vec<String> {
        let envelope = super::parse_envelope(&session.kernel.dispatch(event, "0"));
        assert_eq!(
            envelope["ok"],
            Json::Bool(true),
            "{event} 派发失败：{envelope}"
        );
        super::narrative_lines(&envelope)
    }

    #[test]
    fn module_files_are_watched_so_editing_one_flips_the_signature() {
        let dir = fixture_copy("watch-modules");
        let watched = watched_files(&dir);
        // 五件套 + 资源 + schema + **每个模块**（r2：漏一个就静默跑旧代码）
        // + **配置的解析结果**（r4 缺陷：本 fixture 的 pack 未声明
        // `ygg:agent/config`，走回落 `data/config.json`——只收声明路径时，
        // 改回落配置不翻转签名，热载静默不触发）。
        for path in [
            "mod.lua",
            "pack.json",
            "manifest.json",
            "data/config.json",
            "data/scenes.json",
            "schemas/scene-texts.json",
            "src/util.lua",
            "src/handlers.lua",
            "src/shared.lua",
            "src/silent.lua",
        ] {
            assert!(
                watched.iter().any(|entry| entry == path),
                "watched_files 必须纳管 `{path}`：{watched:?}"
            );
        }

        let before = mod_signature(&dir);
        // 只改一个模块文件（长度也变：不依赖 mtime 分辨率）。
        let util = fs::read_to_string(dir.join("src/util.lua")).expect("读模块");
        fs::write(dir.join("src/util.lua"), format!("{util}\n-- 热载探针\n")).expect("写模块");
        let after = mod_signature(&dir);
        assert!(before != after, "改模块文件必须让 mod_signature 变");

        // 变的**只有**被改的那个文件（签名不是"任何风吹草动都变"）。
        let changed: Vec<&str> = before
            .iter()
            .zip(after.iter())
            .filter(|(left, right)| left != right)
            .map(|(stamp, _)| stamp.path.as_str())
            .collect();
        assert_eq!(changed, vec!["src/util.lua"], "签名变化面必须精确");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn hot_reload_swaps_the_code_and_rolls_back_on_failure() {
        let dir = fixture_copy("hot-reload");
        let mut session = Session::open(&args_for(&dir, "hot-reload")).expect("开局");
        assert!(session.reload.is_none(), "开局不该有热载事件");

        // 模块真的装上了：handler 来自 src.util 的 table（`probe.kv` 由入口注册）。
        assert_eq!(narrate(&mut session, "probe.kv"), ["stage=nil"]);

        // ① 合法改动（只动入口：`mod.lua` 不在内容寻址清单里，改它不需要重算
        //    manifest）→ 签名变 → 热载成功 → 新代码立即生效。
        let entry = fs::read_to_string(dir.join("mod.lua")).expect("读入口");
        fs::write(
            dir.join("mod.lua"),
            format!(
                "{entry}\nygg.on(\"probe.kv\", function()\n  ygg.narrate(\"entry-v2\")\nend)\n"
            ),
        )
        .expect("写入口");

        session.maybe_reload();
        let reload = session.reload.take().expect("签名变了必须触发热载");
        assert!(
            reload.reloaded,
            "合法改动必须换上新包：{:?}",
            reload.message
        );
        assert_eq!(reload.source, "watch");
        assert_eq!(
            narrate(&mut session, "probe.kv"),
            ["stage=nil", "entry-v2"],
            "新入口的 handler 必须立即生效"
        );
        // 换装不是"再叠一层"：模块各执行一次、handler 注册序不重复。
        assert_eq!(
            narrate(&mut session, "probe.order"),
            ["util", "shared", "handlers", "entry"],
            "热载后注册序必须与首装一致（不重复注册）"
        );
        assert_eq!(
            narrate(&mut session, "probe.stats"),
            ["util_execs=1;shared_runs=1;silent_runs=1;util_count=0;shared_count=0"],
            "热载重建模块缓存（各执行一次，模块内状态归零）"
        );

        // ② 破损改动（入口语法错）→ 装载失败 → `reloaded=false` + 旧 mod 继续跑。
        let broken = format!("{entry}\nygg.on(\"probe.kv\", function()\n  ygg.narrate(\"entry-v3\")\nend\nlocal oops = = 1\n");
        fs::write(dir.join("mod.lua"), broken).expect("写破损入口");
        session.maybe_reload();
        let reload = session.reload.take().expect("签名变了必须尝试热载");
        assert!(!reload.reloaded, "破损包不得换上");
        let message = reload.message.unwrap_or_default();
        assert!(
            message.contains("mod.lua"),
            "失败原因必须点名入口文件：{message}"
        );

        // 旧驱动仍在跑：上一个成功版本的行为原样保留（不留半更新 —— 既没上新的
        // 半截代码，也没丢掉已经生效的 v2）。
        assert_eq!(
            narrate(&mut session, "probe.kv"),
            ["stage=nil", "entry-v2"],
            "热载失败后旧 mod 必须继续服务"
        );
        assert_eq!(
            narrate(&mut session, "probe.order"),
            ["util", "shared", "handlers", "entry"]
        );

        let _ = fs::remove_dir_all(&dir);
    }
}

// ============================================================================
// §P0 持久化原子性单测（plan-r5 §4.1；U1–U2 随 S1，U3 随 S2，U4 随 S3）
// ============================================================================
#[cfg(test)]
mod persistence_atomicity_tests {
    use super::{acquire_run_lock, atomic_write, event_count, Cell};
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    /// 用例专属临时目录（互不共享，锁文件互不干扰）。
    fn case_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ygg-p0-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    /// U1：重入不二次 acquire —— 外层取锁后内层再取必须立即返回、不加 depth、
    /// 不触碰锁文件；只有真实持有者的 Drop 才归零 depth 并删锁文件。
    #[test]
    fn u1_reentrant_lock_does_not_reacquire() {
        let dir = case_dir("u1");
        let lock_path = dir.join(super::COORDINATION_LOCK);

        let outer = acquire_run_lock(&dir).expect("首次 acquire 应成功");
        assert!(lock_path.exists(), "真实 acquire 必须留下锁文件");
        assert_eq!(super::RUN_LOCK_DEPTH.with(Cell::get), 1);
        std::fs::write(&lock_path, b"holder-marker").expect("写锁文件探针");

        let started = Instant::now();
        let inner = acquire_run_lock(&dir).expect("重入必须直接成功，不得阻塞或报超时");
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "重入必须立即返回（真实的第二次 acquire 会阻塞到陈旧抢占/超时）"
        );
        assert_eq!(super::RUN_LOCK_DEPTH.with(Cell::get), 1, "重入不得加 depth");
        assert_eq!(
            std::fs::read(&lock_path).expect("锁文件应还在"),
            b"holder-marker",
            "重入不得触碰锁文件（= 没有第二次 acquire）"
        );

        drop(inner);
        assert_eq!(
            super::RUN_LOCK_DEPTH.with(Cell::get),
            1,
            "重入占位的 Drop 不得归零 depth"
        );
        assert!(lock_path.exists(), "重入占位的 Drop 不得删锁文件");
        drop(outer);
        assert_eq!(
            super::RUN_LOCK_DEPTH.with(Cell::get),
            0,
            "真实持有者的 Drop 必须归零 depth"
        );
        assert!(!lock_path.exists(), "真实持有者的 Drop 必须删锁文件");

        // 归零之后必须能立刻重新取锁（释放是真实的，不是记账残留）。
        let again = acquire_run_lock(&dir).expect("释放后必须能重新取锁");
        assert_eq!(super::RUN_LOCK_DEPTH.with(Cell::get), 1);
        drop(again);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// U3：atomic_write —— 写后原文件内容完整；tmp 已不存在；目标目录不存在
    /// 时返回 Err（不 panic）。
    #[test]
    fn u3_atomic_write_semantics() {
        let dir = case_dir("u3");

        // ① 正常写：内容完整 + 无 tmp 残留。
        let target = dir.join("autosave.json");
        atomic_write(&target, br#"{"kv":{"revision":7}}"#).expect("首次写应成功");
        assert_eq!(
            std::fs::read_to_string(&target).expect("读回"),
            r#"{"kv":{"revision":7}}"#
        );
        let leftovers: Vec<String> = std::fs::read_dir(&dir)
            .expect("读目录")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty(), "写后不得残留 tmp：{leftovers:?}");

        // ② 覆盖写：旧全文被新全文原子替换。
        atomic_write(&target, b"second").expect("覆盖写应成功");
        assert_eq!(std::fs::read_to_string(&target).expect("读回"), "second");

        // ③ 目标目录不存在：返回 Err，不 panic，也不落任何文件。
        let missing = dir.join("no-such-dir").join("autosave.json");
        let outcome = atomic_write(&missing, b"x");
        assert!(outcome.is_err(), "目标目录不存在必须报错");
        assert!(!dir.join("no-such-dir").exists(), "不得凭空建目录");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// U4：event_count 三种边界 —— 末行无换行 / 空文件 / 只有空行。
    #[test]
    fn u4_event_count_edge_cases() {
        let dir = case_dir("u4");
        let path = dir.join(super::EVENTS_FILE);

        // 空文件。
        std::fs::write(&path, "").expect("写空文件");
        assert_eq!(event_count(&path), 0, "空文件 = 0 条");

        // 只有空行 / 空白行。
        std::fs::write(&path, "\n\n   \n\t\n").expect("写空白行");
        assert_eq!(event_count(&path), 0, "空行与空白行不计入");

        // 末行无换行：仍按非空行计数。
        std::fs::write(&path, "{\"seq\":0}\n{\"seq\":1}").expect("写无尾换行");
        assert_eq!(event_count(&path), 2, "末行无换行也要计入");

        // 正常形态（对照）。
        std::fs::write(&path, "{\"seq\":0}\n{\"seq\":1}\n").expect("写正常形态");
        assert_eq!(event_count(&path), 2);

        // 文件不存在（开局前）。
        let absent = dir.join("absent.jsonl");
        assert_eq!(event_count(&absent), 0, "读不到文件 = 0 条");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// U2：临界区内 panic 展开后锁已释放 —— guard 的 Drop 在 unwind 路径上
    /// 归零 depth 并删锁文件，随后可以正常重新取锁。
    #[test]
    fn u2_panic_unwind_releases_the_lock() {
        let dir = case_dir("u2");
        let lock_path = dir.join(super::COORDINATION_LOCK);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = acquire_run_lock(&dir).expect("acquire 应成功");
            assert_eq!(super::RUN_LOCK_DEPTH.with(Cell::get), 1);
            panic!("临界区内爆炸");
        }));
        assert!(result.is_err(), "panic 必须向上传播（此处只验锁的释放）");
        assert_eq!(
            super::RUN_LOCK_DEPTH.with(Cell::get),
            0,
            "unwind 后 Drop 必须已归零 depth"
        );
        assert!(!lock_path.exists(), "unwind 后 Drop 必须已删锁文件");

        // panic 之后必须能正常重新取锁（无死锁、无记账残留）。
        let again = acquire_run_lock(&dir).expect("panic 后重新取锁必须成功");
        assert_eq!(super::RUN_LOCK_DEPTH.with(Cell::get), 1);
        drop(again);
        assert_eq!(super::RUN_LOCK_DEPTH.with(Cell::get), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

// ============================================================================
// §reset 语义 + 锁心跳 通用回归（game-1102-1/r5 接手复核）
//
// 不变量（全部通用游戏内核语义，零 agent/Mod 场景假设）：
//   R1 无副作用 resume 不重置（读档成功根本不进 reset 路径）；
//   R2 显式新局仍重置，且恰好归档一次（旧行全在归档里，证据不丢）；
//   R3 续局失败且盘上状态**不可装载**（坏档 / 可解析但版本不符）→ 归档后新局，
//      绝不以全新状态混入旧代（不混代、旧 autosave 不被无归档覆盖）；
//   R4 续局失败但盘上状态**已可装载**（另一进程已救活）→ 重试读档 adopt，
//      不归档别人的行；
//   R5 持锁心跳：真实持锁才 bump 锁文件 mtime，未持锁绝不触碰别人的锁；
//   R6 drain 逐行消费一次（cursor 语义）且全程在真实锁内（心跳接线点）；
//   R7 closing 准入门禁：running 可写 → closing 拒写 → 解除恢复 → finalized 拒写。
// ============================================================================

#[cfg(test)]
mod persistence_reset_semantics_tests {
    use super::{
        acquire_run_lock, begin_write, enter_closing, finalize_run, leave_closing,
        run_lock_heartbeat, Args, ClosingEntry, Drive, ResetOutcome, Session, WriteGate,
        AUTOSAVE_FILE, COORDINATION_LOCK, EVENTS_FILE, INBOX_FILE,
    };
    use std::cell::Cell;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    /// 用例专属临时目录（互不共享；先清空再建）。
    fn case_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ygg-r5-reset-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    /// 最小合成 Mod（2 步到结局 + 摘要 + inbox 路由）：零业务语义，装载期全部
    /// 校验按宿主 fail-closed 规则现过（pack.json agent 段 / config.json drive
    /// 与 settlement 段 / manifest）。entries 为空 → manifest 无需内容哈希。
    fn write_probe_mod(dir: &Path) {
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

    fn probe_args(mod_dir: &Path, runs_root: &Path, run_id: &str, resume: bool) -> Args {
        Args {
            mod_dir: Some(mod_dir.to_path_buf()),
            run_id: run_id.to_owned(),
            runs_root: runs_root.to_path_buf(),
            resume,
            settle: false,
            watch: false,
            event: None,
            payload: None,
            drive: Drive::None,
            inbox: None,
        }
    }

    /// 开新局并推进一步（events=1 行、autosave 可装载的标准起点）。
    fn new_game_with_one_step(mod_dir: &Path, runs_root: &Path, run_id: &str) {
        let mut session = Session::open(&probe_args(mod_dir, runs_root, run_id, false))
            .expect("新局开局必须成功");
        let receipt = session.dispatch_event("event.start", "0");
        assert_eq!(receipt["ok"], serde_json::json!(true), "开局步必须成功");
        assert_eq!(receipt["seq"], serde_json::json!(0));
    }

    /// run 目录里 `events.jsonl.reset.*` 归档份数。
    fn reset_archives(run_dir: &Path) -> Vec<PathBuf> {
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

    fn event_line_count(run_dir: &Path) -> usize {
        fs::read_to_string(run_dir.join(EVENTS_FILE))
            .map(|text| text.lines().filter(|line| !line.trim().is_empty()).count())
            .unwrap_or(0)
    }

    fn archive_line_count(archive: &Path) -> usize {
        fs::read_to_string(archive)
            .map(|text| text.lines().filter(|line| !line.trim().is_empty()).count())
            .expect("归档必须可读")
    }

    /// R1：无副作用 resume 不重置——健康局续局零归档、零重置痕迹、resumed=true。
    #[test]
    fn r1_healthy_resume_never_resets() {
        let dir = case_dir("r1");
        let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
        write_probe_mod(&mod_dir);
        new_game_with_one_step(&mod_dir, &runs_root, "r1");
        let run_dir = runs_root.join("r1");

        let session = Session::open(&probe_args(&mod_dir, &runs_root, "r1", true))
            .expect("健康局 resume 必须成功");
        assert!(session.resumed, "读档成功必须标记 resumed");
        assert!(
            reset_archives(&run_dir).is_empty(),
            "无副作用 resume 不得产生归档"
        );
        assert_eq!(event_line_count(&run_dir), 1, "事件日志不得被动过");
        assert!(
            !session.notices.iter().any(|notice| notice.contains("重置")),
            "不得出现重置提示：{:?}",
            session.notices
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// R2：显式新局（非 resume）对已有产物归档**恰好一次**，旧行全在归档里。
    #[test]
    fn r2_explicit_new_game_archives_exactly_once() {
        let dir = case_dir("r2");
        let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
        write_probe_mod(&mod_dir);
        new_game_with_one_step(&mod_dir, &runs_root, "r2");
        let run_dir = runs_root.join("r2");

        let session = Session::open(&probe_args(&mod_dir, &runs_root, "r2", false))
            .expect("显式新局必须成功");
        let archives = reset_archives(&run_dir);
        assert_eq!(archives.len(), 1, "显式新局必须归档恰好一次");
        assert_eq!(archive_line_count(&archives[0]), 1, "旧代行必须全在归档里");
        assert_eq!(event_line_count(&run_dir), 0, "新一代事件日志从空起步");
        assert!(
            session
                .notices
                .iter()
                .any(|notice| notice.contains("已有旧产物，按新局重置")),
            "必须有旧产物提示：{:?}",
            session.notices
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// R3：**可解析但不可装载**的 autosave（版本不符）→ 归档后新局，绝不混代。
    /// 这是旧判据（可解析即「已恢复」）的实测 RED 面：修复前会跳过重置并以
    /// 全新内核续跑旧代目录——旧 autosave 被无归档覆盖、事件日志混两代。
    #[test]
    fn r3_parseable_but_unloadable_autosave_archives_not_joins() {
        let dir = case_dir("r3");
        let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
        write_probe_mod(&mod_dir);
        new_game_with_one_step(&mod_dir, &runs_root, "r3");
        let run_dir = runs_root.join("r3");

        // 注入「可解析但不可装载」：JSON 合法、engine_version 不符（内核 load 拒绝）。
        let autosave_path = run_dir.join(AUTOSAVE_FILE);
        let mut save: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&autosave_path).expect("读存档"))
                .expect("存档是 JSON");
        save["engine_version"] = serde_json::json!("0.0.0-not-an-engine");
        fs::write(&autosave_path, save.to_string()).expect("写坏版本号");

        let mut session = Session::open(&probe_args(&mod_dir, &runs_root, "r3", true))
            .expect("续局失败按新局开局，open 本身必须成功");
        assert!(!session.resumed, "不可装载的局不得标记 resumed");
        assert!(
            session
                .notices
                .iter()
                .any(|notice| notice.contains("按新局开局")),
            "必须有按新局开局提示：{:?}",
            session.notices
        );
        let archives = reset_archives(&run_dir);
        assert_eq!(archives.len(), 1, "不可装载的旧代必须归档（证据不丢）");
        assert_eq!(archive_line_count(&archives[0]), 1, "旧代行全在归档里");
        assert_eq!(event_line_count(&run_dir), 0, "新一代从空起步");

        // 关键反混代断言：新局第一步 seq 必须是 0（修复前会接旧代行号 seq=1）。
        let receipt = session.dispatch_event("event.start", "0");
        assert_eq!(receipt["ok"], serde_json::json!(true));
        assert_eq!(
            receipt["seq"],
            serde_json::json!(0),
            "新局第一步必须从 seq 0 起步（混代即 1）"
        );
        assert_eq!(event_line_count(&run_dir), 1, "events 里只有新代这一行");
        let _ = fs::remove_dir_all(&dir);
    }

    /// R4：续局失败但盘上状态已可装载（另一进程已救活）→ 锁内重试读档 adopt，
    /// 不归档别人的行、不重置。复刻 Session::open 的两段序：首次读档失败（坏档）
    /// → 「另一进程」把档修好 → reset_artifacts(true) 必须 Recovered。
    #[test]
    fn r4_recheck_adopts_recovered_state_without_archiving() {
        let dir = case_dir("r4");
        let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
        write_probe_mod(&mod_dir);
        new_game_with_one_step(&mod_dir, &runs_root, "r4");
        let run_dir = runs_root.join("r4");
        let autosave_path = run_dir.join(AUTOSAVE_FILE);

        let mut session = Session::open(&probe_args(&mod_dir, &runs_root, "r4", true))
            .expect("健康局 resume 必须成功");
        assert!(session.resumed);

        // ① 本进程视角「续局失败」：坏档（复刻 open 的首次读档在锁外失败）。
        let good_save = fs::read_to_string(&autosave_path).expect("读好档");
        fs::write(&autosave_path, "corrupted-bytes").expect("写坏档");
        assert!(
            session.load_autosave().is_err(),
            "坏档读档必须失败（进入二次判定路径的前提）"
        );

        // ② 「另一进程」恢复：把好档写回（重置后起步 / 外部修复，对判据等价）。
        fs::write(&autosave_path, &good_save).expect("另一进程恢复好档");

        // ③ 二次判定：重试读档成功 → Recovered，不归档、不动事件日志。
        let outcome = session.reset_artifacts(true);
        assert!(
            matches!(outcome, ResetOutcome::Recovered),
            "盘上状态可装载必须 adopt，不得重置"
        );
        assert!(reset_archives(&run_dir).is_empty(), "不得归档别人的行");
        assert_eq!(event_line_count(&run_dir), 1, "事件日志必须原样保留");
        let _ = fs::remove_dir_all(&dir);
    }

    /// R5：锁心跳——真实持锁才续命（mtime 前移），未持锁绝不触碰别人的锁文件。
    #[test]
    fn r5_lock_heartbeat_only_while_holding() {
        let dir = case_dir("r5");
        let lock_path = dir.join(COORDINATION_LOCK);

        // ① 未持锁：心跳是空操作，连别人的锁文件都不碰。
        fs::write(&lock_path, b"someone-else").expect("造一把别人的锁");
        let stale = SystemTime::now() - Duration::from_secs(60);
        fs::OpenOptions::new()
            .write(true)
            .open(&lock_path)
            .expect("打开别人的锁")
            .set_modified(stale)
            .expect("回拨 mtime");
        run_lock_heartbeat();
        let untouched = fs::metadata(&lock_path)
            .expect("锁还在")
            .modified()
            .expect("读 mtime");
        assert!(
            untouched <= SystemTime::now() - Duration::from_secs(55),
            "未持锁的心跳绝不能 bump 别人的锁"
        );

        // ② 真实持锁：心跳把 mtime 从「陈旧」续到「新鲜」（陈旧抢占判据不再误伤活锁）。
        let _guard = acquire_run_lock(&dir).expect("acquire 应成功");
        assert_eq!(super::RUN_LOCK_DEPTH.with(Cell::get), 1);
        fs::OpenOptions::new()
            .write(true)
            .open(&lock_path)
            .expect("打开自己的锁")
            .set_modified(stale)
            .expect("回拨 mtime（模拟持锁已超过 LOCK_STALE_MS）");
        run_lock_heartbeat();
        let refreshed = fs::metadata(&lock_path)
            .expect("锁还在")
            .modified()
            .expect("读 mtime");
        assert!(
            refreshed > SystemTime::now() - Duration::from_secs(55),
            "持锁心跳必须把 mtime 续到新鲜（防等待者按陈旧抢占活锁）"
        );

        drop(_guard);
        assert_eq!(
            super::RUN_LOCK_DEPTH.with(Cell::get),
            0,
            "释放后 depth 归零"
        );
        assert!(!lock_path.exists(), "释放后锁文件删除");
        let _ = fs::remove_dir_all(&dir);
    }

    /// R6：drain 逐行消费一次（cursor 语义），且全程在真实锁内（心跳接线点冒烟）。
    #[test]
    fn r6_drain_consumes_each_line_once_under_lock() {
        let dir = case_dir("r6");
        let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
        write_probe_mod(&mod_dir);
        new_game_with_one_step(&mod_dir, &runs_root, "r6");
        let run_dir = runs_root.join("r6");

        for index in 0..3 {
            use std::io::Write as _;
            let mut inbox = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(run_dir.join(INBOX_FILE))
                .expect("打开 inbox");
            writeln!(
                inbox,
                "{{\"kind\":\"add\",\"title\":\"t{index}\",\"external_id\":\"ext-{index}\"}}"
            )
            .expect("写 inbox 行");
        }

        let mut session =
            Session::open(&probe_args(&mod_dir, &runs_root, "r6", true)).expect("resume 必须成功");
        let first = session.drain_inbox();
        assert_eq!(first.applied, 3, "三行各消费一次");
        assert_eq!(first.remaining, 0, "消费完不留尾");
        assert_eq!(first.error, None, "三行都必须入队成功");
        // 关键：drain 的 dispatch 走持久化临界区，seq 与开局步连续（无重复/跳号）。
        assert_eq!(event_line_count(&run_dir), 1 + 3, "开局 1 行 + inbox 3 行");

        // 幂等：cursor 已前移，再 drain 不重放（W8 的 cursor 正常面）。
        let second = session.drain_inbox();
        assert_eq!(second.applied, 0, "cursor 已前移不得重放");
        assert_eq!(event_line_count(&run_dir), 4, "事件日志不得新增");
        let _ = fs::remove_dir_all(&dir);
    }

    /// R7：closing 准入门禁——running 可写 → closing 拒写 → 解除恢复 →
    /// finalized 拒写。writer token / 收口协议的关键不变量。
    #[test]
    fn r7_closing_gate_lifecycle() {
        let dir = case_dir("r7");

        // running：可写，登记后摘除。
        let token = match begin_write(&dir) {
            Ok(token) => token,
            Err(gate) => panic!("running 必须可写，实际 {gate:?}"),
        };
        super::end_write(&dir, &token);

        // closing：写命令被拒（Closing 门）。
        assert!(matches!(enter_closing(&dir), Ok(ClosingEntry::Entered)));
        assert!(
            matches!(begin_write(&dir), Err(WriteGate::Closing)),
            "closing（心跳未过期）必须拒写"
        );

        // 解除 closing：恢复可写。
        leave_closing(&dir);
        let token = begin_write(&dir).expect("解除 closing 后必须可写");
        super::end_write(&dir, &token);

        // finalized：永久拒写（需开新局）。
        enter_closing(&dir).expect("进 closing");
        finalize_run(&dir);
        assert!(
            matches!(begin_write(&dir), Err(WriteGate::Finalized)),
            "finalized 后必须拒写"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
