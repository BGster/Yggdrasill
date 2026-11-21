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
//! 形态（r1 语义二分：new = 开新局；open = 读档续跑；存档目录即身份）：
//!
//! ```bash
//! # ① 新局（自动 run-id；回执末行 run-root: <path>；--event 可同调用 boot）
//! cargo run -p ygg-sandbox --example egg -- new --mod packages/mods/test-adventure \
//!   --in .ygg/runs --event event.start
//!
//! # ② 单发模式（Agent 脚本化调用：动作 → JSON 回执 → 退出；附着取最新存档点）
//! cargo run -p ygg-sandbox --example egg -- open .ygg/runs/<run-id> \
//!   --event <Mod 的 choiceEvent> --payload '1'
//!
//! # ③ 局末结算（读本局后端的状态事实 → settlement.json；json 局 = autosave + events.jsonl）
//! cargo run -p ygg-sandbox --example egg -- open .ygg/runs/<run-id> --settle
//!
//! # ④ 异步补需求（只追本局后端的 inbox 后退出；不碰 autosave，故可在另一进程跑
//! #    本局的同时安全调用）
//! cargo run -p ygg-sandbox --example egg -- open .ygg/runs/<run-id> \
//!   --requirement '补一条门禁日志|把 verify 的失败原因写进 events.jsonl'
//!
//! # ⑤ 回退到指定存档点（仅 sqlite 局；代 = generation，旧代证据留库）
//! cargo run -p ygg-sandbox --example egg -- open .ygg/runs/<run-id> --at g0 --event map.show
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
//! 单发回执契约（`--raw` 输出 stdout 单行 JSON；机器契约）：
//!
//! 缺省（无旗标）单发 stdout 是**人读文本渲染**（版式不承诺稳定；脚本 / CI 解析
//! stdout 必须加 `--raw`，或环境级 `export YGG_FORMAT=json` 一次性回滚旧行为）；
//! `YGG_FORMAT=text|json` 提供环境级回退，解析链 = `--raw` > env > 缺省 text
//! （非法值 fail-closed 退出 2）。`--settle` / inbox 写命令 / `--artifact-put|get` /
//! `--wait-tools` / `--builtin-tool` / daemon 恒为 JSON，`--raw` 对其无效果。
//! JSON 形态与退出码契约不变：
//!
//! ```json
//! {"ok":true,"event":"<事件名>","narrative":["…"],"pendingChoice":{"prompt":"…","options":["…"]},
//!  "autosave":"runs/my-task/autosave.json","seq":0,"revision":5}
//! ```
//!
//! - `pendingChoice` 原样透传内核信封（动作白名单的现场呈现；`null` = 无待决
//!   选择）；越权动作的 error 信封原样落 `error`（码 + 文案），并补一次 `state`
//!   回填当前 `pendingChoice`，让 Agent 出错后仍能接着走；
//! - 未注册事件名（fail-closed）：本 Mod registry 未注册的名字一律
//!   `ok:false` + `error.code=VALIDATION` + 退出码 1，文案携带已注册全集
//!   （字典序）供一次往返自纠；`ygg.*` 保留事件同样只认 registry（宿主不持
//!   事件名白名单）；
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
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{exit, Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value as Json};
use ygg_sandbox::driver::KernelFacade;
use ygg_sandbox::host_contract::{
    join_segments, require_tool_allowlist, sanitize_field, AgentContract, DriveLimits, InboxRoutes,
    SettlementPolicy, ToolAccess, ToolKind, ToolRegistry, ToolSpec, WriteShape, CONTRACT_DOC,
};
use ygg_sandbox::modload::ModBundle;
use ygg_sandbox::{HostToolRunner, MountEntry, MountFs, MountKind, MountStat, MountWritten};
// 持久化端口（`crates/ygg-store`）：状态事实（存档 / 事件日志 / inbox / 位移 /
// 重置）全部经它读写。**锁、run.json、settlement.json、coordination.* 仍留在
// 宿主**——那是跨进程共识与报告面，两后端必须完全一致。
use ygg_store::{
    Backend, MountStore, RunStore, StepDraft, Tick, AUTOSAVE_FILE, INBOX_FILE, SQLITE_FILE_NAME,
};

mod decide;

/// serve 模式（常驻只读观察台，r8）：HTTP 面板 + 零写入观察面。
/// 路由表只有结构名（runs/saves/events/artifacts/panel），无一个 mod 事件名（P2/P3）。
/// 目录化后 serve/ 仍在 examples/serve/（egg 外唯一子模块），用 #[path] 显式指回。
#[path = "../serve/mod.rs"]
mod serve;

// 各拆分模块的根级再导出：serve 引用的 `crate::Args` / `crate::META_FILE` /
// `super::read_meta` 三类路径在目录化后零改动。
pub(crate) use decide::*;

mod archive;
mod args;
mod artifact;
#[cfg(test)]
mod contract_tests;
mod coordination;
mod daemon;
mod drive;
mod env_table;
mod hotreload;
mod identity;
#[cfg(test)]
mod inbox_routes_tests;
mod interactive;
#[path = "io.rs"]
mod io_util;
mod mount_bridge;
mod ops;
mod paths;
mod pending_store;
mod receipt;
mod session;
mod settlement;
mod single_shot;
mod tool_protocol;
mod tool_runner;
mod tool_worker;
mod usage;
mod verify;

pub(crate) use archive::*;
pub(crate) use args::*;
pub(crate) use artifact::*;
pub(crate) use coordination::*;
pub(crate) use daemon::*;
pub(crate) use decide::*;
pub(crate) use drive::*;
pub(crate) use env_table::*;
pub(crate) use hotreload::*;
pub(crate) use identity::*;
pub(crate) use interactive::*;
pub(crate) use io_util::*;
pub(crate) use mount_bridge::*;
pub(crate) use ops::*;
pub(crate) use paths::*;
pub(crate) use pending_store::*;
pub(crate) use receipt::*;
pub(crate) use session::*;
pub(crate) use settlement::*;
pub(crate) use single_shot::*;
pub(crate) use tool_protocol::*;
pub(crate) use tool_runner::*;
pub(crate) use tool_worker::*;
pub(crate) use usage::*;
pub(crate) use verify::*;

fn main() {
    let args = parse_args();
    let args = match prepare_run_root(args) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("error: {message}");
            exit(1);
        }
    };
    let code = if args.builtin_tool.is_some() {
        // 内置工具模式：不读 Mod、不推进状态、不需要 run 目录（见 `run_builtin_tool`）。
        run_builtin_tool(&args)
    } else if let Some(call_id) = &args.run_tool_worker {
        // 内部模式（不在 usage 里列出）：一次工具调用的执行体。
        run_tool_worker(&args, call_id)
    } else if let Some(spec) = &args.artifact_put {
        run_artifact_put(&args, spec)
    } else if let Some(path) = &args.artifact_get {
        run_artifact_get(&args, path)
    } else if args.wait_tools {
        run_wait_tools(&args)
    } else if args.serve {
        // 常驻只读观察台（r8）：不构造 Session、不装 ModBundle/kernel/mlua，
        // 只经 ygg_store::observe 与文件 stat 做只读访问。
        serve::run_serve(&args)
    } else if args.daemon {
        run_daemon(&args)
    } else if args.inbox.is_some() {
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
