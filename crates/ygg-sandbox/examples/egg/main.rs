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

mod args;
mod artifact;
mod drive;
mod env_table;
mod hotreload;
mod identity;
mod mount_bridge;
mod paths;
mod pending_store;
mod session;
mod tool_protocol;
mod usage;

pub(crate) use args::*;
pub(crate) use artifact::*;
pub(crate) use drive::*;
pub(crate) use env_table::*;
pub(crate) use hotreload::*;
pub(crate) use identity::*;
pub(crate) use mount_bridge::*;
pub(crate) use paths::*;
pub(crate) use pending_store::*;
pub(crate) use session::*;
pub(crate) use tool_protocol::*;
pub(crate) use usage::*;

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

/// 进程内同步工具执行器（挂载到 `KernelFacade`，实现 `ygg.tool.call` 同步桥）。
///
/// 强制校验两层白名单：Layer 1（`pack.json` 的 `tools`）与 Layer 2（`data/config.json` 的 `allowedTools`）。
/// 六个内置工具直接在当前进程内调用 `*_fields_with`，不派生子进程；外部工具走 `execute_tool`。
struct InProcessToolRunner {
    tools: ToolRegistry,
    tool_access: ToolAccess,
    mod_dir: PathBuf,
    run_dir: PathBuf,
    runs_root: PathBuf,
    run_id: String,
    harness: Option<String>,
}

impl InProcessToolRunner {
    fn parse_kv_output(raw: &str) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return map;
        }
        let mut any_pair = false;
        for segment in trimmed.split(';') {
            if let Some((k, v)) = segment.split_once('=') {
                let key = k.trim();
                if !key.is_empty() {
                    map.insert(key.to_owned(), v.trim().to_owned());
                    any_pair = true;
                }
            }
        }
        if !any_pair {
            map.insert("output".to_owned(), trimmed.to_owned());
        }
        map
    }

    fn fields_to_map(fields: Vec<(&'static str, String)>) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        for (k, v) in fields {
            map.insert(k.to_owned(), wire_sanitize(&v));
        }
        map
    }

    fn op_failure_to_err(op: &str, fail: OpFailure) -> String {
        match fail {
            OpFailure::Line(line) => line,
            OpFailure::Detail { slug, detail } => {
                format!(
                    "op={};error={};detail={}",
                    wire_sanitize(op),
                    wire_sanitize(slug),
                    wire_sanitize(&detail)
                )
            }
            OpFailure::Reported => format!("op={};error=reported", wire_sanitize(op)),
        }
    }

    fn try_run_builtin(
        &self,
        expanded_args: &[String],
    ) -> Option<Result<BTreeMap<String, String>, String>> {
        if expanded_args.len() < 2 || expanded_args[0] != "--builtin-tool" {
            return None;
        }
        let tool = BuiltinTool::parse(&expanded_args[1])?;
        let mut values: Vec<(String, String)> = Vec::new();
        let mut idx = 2;
        while idx < expanded_args.len() {
            let key = &expanded_args[idx];
            if key.starts_with("--") && idx + 1 < expanded_args.len() {
                values.push((key.clone(), expanded_args[idx + 1].clone()));
                idx += 2;
            } else {
                idx += 1;
            }
        }
        if tool == BuiltinTool::ArtifactProbe && !values.iter().any(|(k, _)| k == "--run-dir") {
            values.push(("--run-dir".to_owned(), self.run_dir.display().to_string()));
        }
        let flags = OpFlags { values };
        let op_name = tool.as_str();
        let outcome = match tool {
            BuiltinTool::WorktreeClaim => {
                if flags.get("--lane").is_none()
                    || flags.get("--req").is_none()
                    || flags.get("--branch").is_none()
                {
                    return Some(Err("missing required worktree-claim flags".to_owned()));
                }
                claim_fields_with(&flags)
            }
            BuiltinTool::WorktreeRelease => {
                if flags.get("--lane").is_none() {
                    return Some(Err("missing required worktree-release --lane".to_owned()));
                }
                release_fields_with(&flags)
            }
            BuiltinTool::LaneProbe => {
                if flags.get("--lane").is_none()
                    || flags.get("--base").is_none()
                    || flags.get("--branch").is_none()
                {
                    return Some(Err("missing required lane-probe flags".to_owned()));
                }
                lane_probe_fields_with(&flags)
            }
            BuiltinTool::ArtifactProbe => {
                if flags.get("--path").is_none() {
                    return Some(Err("missing required artifact-probe --path".to_owned()));
                }
                artifact_probe_fields_with(&flags)
            }
            BuiltinTool::VerifySuite => verify_suite_fields_with(&flags),
            BuiltinTool::ArchiveSessions => {
                if flags.get("--parent").is_none() || flags.get("--batch").is_none() {
                    return Some(Err("missing required archive-sessions flags".to_owned()));
                }
                let inputs = ArchiveInputs {
                    harness: self.harness.clone(),
                    mod_dir: Some(self.mod_dir.clone()),
                    sessions_dir: std::env::var("YGG_SESSIONS_DIR")
                        .ok()
                        .filter(|v| !v.trim().is_empty())
                        .map(PathBuf::from),
                    home: home_dir(),
                    cwd: std::env::current_dir().ok(),
                    env_harness: std::env::var("YGG_HARNESS")
                        .ok()
                        .filter(|v| !v.trim().is_empty()),
                };
                archive_sessions_fields_with(&flags, &inputs)
            }
        };
        Some(
            outcome
                .map(Self::fields_to_map)
                .map_err(|fail| Self::op_failure_to_err(op_name, fail)),
        )
    }
}

impl HostToolRunner for InProcessToolRunner {
    fn call(
        &mut self,
        tool_id: &str,
        args: &BTreeMap<String, String>,
    ) -> Result<BTreeMap<String, String>, String> {
        let Some(spec) = self.tools.get(tool_id).cloned() else {
            return Err(format!("工具 `{tool_id}` 未在 pack.json 的 `tools` 段声明"));
        };
        if !self.tool_access.allows(&spec.id) {
            return Err(format!("工具 `{}` 未被本局 `allowedTools` 放行", spec.id));
        }
        let mut env = load_env_table(Some(&self.mod_dir));
        env.process
            .push(("YGG_RUN_DIR".to_owned(), self.run_dir.display().to_string()));
        env.process.push((
            "YGG_RUNS_ROOT".to_owned(),
            self.runs_root.display().to_string(),
        ));
        env.process
            .push(("YGG_RUN_ID".to_owned(), self.run_id.clone()));
        env.process
            .push(("YGG_MOD_DIR".to_owned(), self.mod_dir.display().to_string()));
        let arg_pairs: Vec<(String, String)> =
            args.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        validate_expansion(&spec, &arg_pairs, &env)?;

        if spec.kind == ToolKind::Cli {
            let is_self = spec.program.as_deref() == Some(ygg_sandbox::host_contract::SELF_PROGRAM)
                || spec.program.as_deref() == Some(SELF_ALIAS);
            if is_self {
                let mut expanded = Vec::with_capacity(spec.args.len());
                for tpl in &spec.args {
                    expanded.push(substitute(tpl, &arg_pairs, &env)?);
                }
                if let Some(res) = self.try_run_builtin(&expanded) {
                    return res;
                }
            }
        }

        let request = ToolRequest {
            id: format!("sync-{}", now_ms()),
            tool: spec.id.clone(),
            args: arg_pairs,
        };
        let call =
            PendingCall::from_spec(&spec, &request, now_ms() + u128::from(spec.timeout_ms), 1);
        let scratch = tool_results_dir(&self.run_dir);
        let _ = fs::create_dir_all(&scratch);
        let outcome = execute_tool(&call, &env, &scratch);
        if outcome.status == "ok" {
            Ok(Self::parse_kv_output(&outcome.output))
        } else {
            Err(outcome.output)
        }
    }
}

/// 子进程结局（`TimedOut` = 已被 kill 并回收）。
enum ChildOutcome {
    Exited(std::process::ExitStatus),
    TimedOut,
}

/// spawn 一个子进程并轮询到退出或超时（stdout / stderr 落临时文件，避开管道死锁）。
fn run_child(
    mut command: Command,
    label: &str,
    timeout_ms: u64,
    scratch: &Path,
    tag: &str,
) -> Result<ChildOutcome, String> {
    let stdout_path = scratch.join(format!("{tag}.stdout"));
    let stderr_path = scratch.join(format!("{tag}.stderr"));
    let stdout = File::create(&stdout_path)
        .map_err(|err| format!("创建 {} 失败：{err}", stdout_path.display()))?;
    let stderr = File::create(&stderr_path)
        .map_err(|err| format!("创建 {} 失败：{err}", stderr_path.display()))?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    let mut child = command
        .spawn()
        .map_err(|err| format!("无法启动 `{label}`：{err}"))?;
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(ChildOutcome::Exited(status)),
            Ok(None) => {}
            Err(err) => return Err(format!("等待 `{label}` 失败：{err}")),
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(ChildOutcome::TimedOut);
        }
        std::thread::sleep(TOOL_POLL_INTERVAL);
    }
}

/// 读一个临时产物文件（最多 `limit` 字节，非法 UTF-8 用替换符兵底）。
fn read_limited(path: &Path, limit: usize) -> String {
    let Ok(file) = File::open(path) else {
        return String::new();
    };
    let mut buffer = Vec::new();
    let _ = file.take(limit as u64).read_to_end(&mut buffer);
    String::from_utf8_lossy(&buffer).into_owned()
}

/// 执行一次工具（worker 侧；每轮只调一次）。
fn execute_tool(call: &PendingCall, env: &EnvTable, scratch: &Path) -> ToolOutcome {
    let outcome = match call.kind {
        ToolKind::Cli => execute_cli(call, env, scratch),
        ToolKind::Http => execute_http(call, env, scratch),
    };
    // 暂存文件只服务本次执行：结局已经进内存 / 结果文件，删掉（不把工具输出
    // 以原形留在 run 目录里等下一次 `git add`）。
    for suffix in ["stdout", "stderr"] {
        let _ = fs::remove_file(scratch.join(format!("{}.{suffix}", call.call_id)));
    }
    let _ = fs::remove_file(scratch.join(format!("{}-body.out", call.call_id)));
    outcome
}

/// `cli`：直接 spawn（**无 shell**），`status.success()` 即 `ok`。
fn execute_cli(call: &PendingCall, env: &EnvTable, scratch: &Path) -> ToolOutcome {
    let Some(program) = call.program.clone() else {
        return ToolOutcome::error(
            "000",
            format!("工具 `{}` 的声明缺少 program", call.tool),
            call.max_output_bytes,
        );
    };
    // `@self` / `egg` → 本可执行文件。两个名字都解析到 `current_exe()`：
    // 内置子命令不能依赖 `$PATH`（部署方可能根本没把 `egg` 装进 PATH，
    // 而 `pack.json` 是静态文件、不能内插绝对路径）。
    let executable = if program == ygg_sandbox::host_contract::SELF_PROGRAM || program == SELF_ALIAS
    {
        match std::env::current_exe() {
            Ok(path) => path.display().to_string(),
            Err(err) => {
                return ToolOutcome::error(
                    "127",
                    format!("工具 `{}` 的 @self 解析失败：{err}", call.tool),
                    call.max_output_bytes,
                )
            }
        }
    } else {
        program.clone()
    };
    let mut command = Command::new(&executable);
    for template in &call.args {
        match substitute(template, &call.placeholders, env) {
            Ok(value) => {
                command.arg(value);
            }
            Err(message) => {
                return ToolOutcome::error(
                    "126",
                    format!("工具 `{}` 的{message}", call.tool),
                    call.max_output_bytes,
                )
            }
        }
    }
    match run_child(command, &program, call.timeout_ms, scratch, &call.call_id) {
        Ok(ChildOutcome::Exited(status)) => {
            let limit = call.max_output_bytes.saturating_add(64);
            let stdout = read_limited(&scratch.join(format!("{}.stdout", call.call_id)), limit);
            let stderr = read_limited(&scratch.join(format!("{}.stderr", call.call_id)), limit);
            let output = if stdout.trim().is_empty() {
                stderr
            } else {
                stdout
            };
            let code = match status.code() {
                Some(code) => code.to_string(),
                None => "signal".to_owned(),
            };
            ToolOutcome {
                status: if status.success() { "ok" } else { "err" },
                code,
                output: sanitize_tool_output(&output, call.max_output_bytes),
            }
        }
        Ok(ChildOutcome::TimedOut) => ToolOutcome::error(
            "timeout",
            format!(
                "工具 `{}` 执行超时（{}ms），已终止",
                call.tool, call.timeout_ms
            ),
            call.max_output_bytes,
        ),
        Err(message) => ToolOutcome::error("127", message, call.max_output_bytes),
    }
}

/// `http`：经 `curl` 发一次请求（`code` = `%{http_code}`；2xx 即 `ok`）。
fn execute_http(call: &PendingCall, env: &EnvTable, scratch: &Path) -> ToolOutcome {
    let (Some(method), Some(url_template)) = (call.method.clone(), call.url.clone()) else {
        return ToolOutcome::error(
            "000",
            format!("工具 `{}` 的声明缺少 method / url", call.tool),
            call.max_output_bytes,
        );
    };
    let url = match substitute(&url_template, &call.placeholders, env) {
        Ok(value) => value,
        Err(message) => {
            return ToolOutcome::error(
                "126",
                format!("工具 `{}` 的{message}", call.tool),
                call.max_output_bytes,
            )
        }
    };
    let body_path = scratch.join(format!("{}-body.out", call.call_id));
    let mut command = Command::new("curl");
    command
        .arg("-sS")
        .arg("-X")
        .arg(&method)
        .arg("--max-time")
        .arg(call.timeout_ms.div_ceil(1000).max(1).to_string())
        .arg("-o")
        .arg(&body_path)
        .arg("-w")
        .arg("%{http_code}");
    for (key, template) in &call.headers {
        match substitute(template, &call.placeholders, env) {
            Ok(value) => {
                command.arg("-H").arg(format!("{key}: {value}"));
            }
            Err(message) => {
                return ToolOutcome::error(
                    "126",
                    format!("工具 `{}` 的{message}", call.tool),
                    call.max_output_bytes,
                )
            }
        }
    }
    if let Some(template) = &call.body_template {
        match substitute(template, &call.placeholders, env) {
            Ok(body) => {
                command.arg("--data-binary").arg(body);
            }
            Err(message) => {
                return ToolOutcome::error(
                    "126",
                    format!("工具 `{}` 的{message}", call.tool),
                    call.max_output_bytes,
                )
            }
        }
    }
    command.arg(&url);
    match run_child(command, "curl", call.timeout_ms, scratch, &call.call_id) {
        Ok(ChildOutcome::Exited(status)) => {
            let limit = call.max_output_bytes.saturating_add(64);
            let code = {
                let raw = read_limited(&scratch.join(format!("{}.stdout", call.call_id)), 16);
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    "000".to_owned()
                } else {
                    sanitize_tool_output(trimmed, 16)
                }
            };
            let body = read_limited(&body_path, limit);
            let stderr = read_limited(&scratch.join(format!("{}.stderr", call.call_id)), limit);
            let output = if body.trim().is_empty() { stderr } else { body };
            let http_ok = code
                .parse::<u16>()
                .is_ok_and(|code| (200..=299).contains(&code));
            let _ = status; // curl 的退出码不进 wire：`code` 是 HTTP 状态码
            ToolOutcome {
                status: if http_ok { "ok" } else { "err" },
                code,
                output: sanitize_tool_output(&output, call.max_output_bytes),
            }
        }
        Ok(ChildOutcome::TimedOut) => ToolOutcome::error(
            "timeout",
            format!(
                "工具 `{}` 执行超时（{}ms），已终止",
                call.tool, call.timeout_ms
            ),
            call.max_output_bytes,
        ),
        Err(message) => ToolOutcome::error("127", message, call.max_output_bytes),
    }
}

/// 写一条工具结果 inbox 记录（worker 侧；与 `--tool-result` 同一条通道）。
///
/// 记录字段名 / 写入形态全部来自 Mod 的 `inboxRoutes` 声明；幂等键取 wire 载荷
/// 的末段（同一值，Mod 侧去重与审计看同一处）。
fn append_tool_result_record(
    run_dir: &Path,
    store: &mut dyn RunStore,
    routes: &InboxRoutes,
    kind: &str,
    payload: &str,
    run_id: &str,
    identity: &ActorIdentity,
) -> Result<Json, String> {
    let route = routes
        .get(kind)
        .ok_or_else(|| format!("inboxRoutes 未声明 kind `{kind}`"))?;
    let Some(shape) = route.write.as_ref().map(|binding| binding.shape) else {
        return Err(format!(
            "inboxRoutes 的 `{kind}` 没有 write 绑定（worker 无处写结果）"
        ));
    };
    let external_field = routes.external_id_field();
    let payload_field = routes.payload_field();
    let mut record = json!({"kind": kind, "created_at": now_secs()});
    let external_id = payload.rsplit('|').next().unwrap_or("").trim();
    record[external_field] = json!(if external_id.is_empty() {
        new_external_id(run_id, store.inbox_len().unwrap_or(0))
    } else {
        external_id.to_owned()
    });
    match shape {
        WriteShape::Payload => record[payload_field] = json!(payload.trim()),
        WriteShape::Named => {
            let (first, rest) = split_requirement(payload);
            let mut named = route.named_fields(external_field);
            if let Some(field) = named.next() {
                record[field.as_str()] = json!(first);
            }
            if let Some(field) = named.next() {
                record[field.as_str()] = json!(rest);
            }
        }
    }
    // 提交者章（worker 自身环境的身份解析产物；None 不写键）：与 `--requirement`
    // 写命令同一套保留键，drain 消费时按记录取回注入声明路由。
    if let Some(actor) = &identity.actor {
        record["actor"] = json!(actor);
    }
    if let Some(session) = &identity.session {
        record["actor_session"] = json!(session);
    }
    // 写前登记（与 `--requirement` 同一准入门禁）：closing / finalized 一律拒绝。
    let token = begin_write(run_dir).map_err(|gate| match gate {
        WriteGate::Closing => {
            "本局正在收口（closing）——工具结果未写入，稍后重试或开新局".to_owned()
        }
        WriteGate::Finalized => "本局已 finalized——工具结果未写入，需开新局".to_owned(),
        WriteGate::Internal(message) => message,
    })?;
    let pushed = store.inbox_push(&record, clock());
    if pushed.is_ok() {
        // opener 认领（成功落行 + 携带身份；仍在 token 临界区内，与写命令同位）。
        if let Some(actor) = &identity.actor {
            if let Err(message) = claim_opener(run_dir, actor, identity.session.as_deref()) {
                eprintln!("[警告] opener 认领失败（不影响本条写入）：{message}");
            }
        }
    }
    end_write(run_dir, &token);
    pushed.map_err(|err| err.to_string())?;
    Ok(record)
}

/// 工具回投深度的归零守卫（panic 展开也归零）。
struct ToolDepthGuard(Rc<Cell<usize>>);

impl Drop for ToolDepthGuard {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(1));
    }
}

// ============================================================================
// §内置工具（`--builtin-tool <op>`）
//
// 六个子命令的**共同合约**（改动前先读）：
//
// 1. **stdout = 恰好一行**，且只在成功时输出。格式 `k=v;k=v;…`：`;` 分隔字段，
//    `,` 分隔列表项。**绝不**出现 `|`（那是外层 `tool.result` 段的分隔符，
//    `host_contract.rs` 的 wire 契约）、不出现换行、不出现 NUL。
// 2. **失败 ⇒ stdout 全空 + stderr 一行人读错误 + 退出 1**。这条是承重的：
//    `execute_cli` 只在 stdout 为空时回落读 stderr，所以「stdout 空」才是失败
//    在 `tool.result` 里可见的前提。
// 3. 每个落进输出的值都过 [`wire_sanitize`]（git 路径**可以**带逗号，所以
//    清洗是必需的，不是美化）。列表先排序再 join：同输入同字节。
// 4. 用法错误（未知旗标 / 缺必需旗标 / 坏数字）→ 退出 **2** 并列出该 op 的旗标。
// 5. 打印的路径一律先 `fs::canonicalize`：`path=<wt>` 是绝对且可复现的。
//
// 这些子命令**不读 Mod、不推进状态**：它们只操作 worktree / 制品面 / 会话目录。
// 在 `pack.json` 里写 `"program": "@self"` 即可调用（`@self` = 本可执行文件）。
// ============================================================================

/// op 级旗标集合（`--builtin-tool` 之后那批）。
///
/// `parse_args` 只负责**不拒绍**它们；真正的白名单在这里——只有这一处知道
/// 「这个 op 该有哪些旗标」，报错才能点名 op。
struct OpFlags {
    values: Vec<(String, String)>,
}

/// 全局旗标里「带值」的那批（重扫 argv 时跳过它们的值）。
const GLOBAL_VALUE_FLAGS: &[&str] = &[
    "--mod",
    "--run-id",
    "--runs-root",
    "--store",
    "--event",
    "--payload",
    "--drive",
    "--harness",
    "--tool-timeout",
    "--run-tool-worker",
    "--artifact-put",
    "--artifact-get",
];

/// 全局旗标里「裸旗标」的那批。
const GLOBAL_BARE_FLAGS: &[&str] = &[
    "--builtin-tool",
    "--resume",
    "--settle",
    "--watch",
    "--wait-tools",
];

impl OpFlags {
    /// 重扫 `argv`，只收 `allowed` 里的旗标；其余必须是已知全局旗标，否则
    /// 退出 2（点名 op 与合法旗标表）。
    fn parse(op: &str, allowed: &[&str]) -> Self {
        let mut values: Vec<(String, String)> = Vec::new();
        let mut iter = std::env::args().skip(1);
        while let Some(argument) = iter.next() {
            let name = argument.as_str();
            if allowed.contains(&name) {
                match iter.next() {
                    Some(value) => values.push((name.to_owned(), value)),
                    None => builtin_usage_error(op, &format!("`{name}` 缺少取值"), allowed),
                }
            } else if GLOBAL_VALUE_FLAGS.contains(&name) {
                // 全局旗标的值不属本 op，丢掉。
                let _ = iter.next();
            } else if GLOBAL_BARE_FLAGS.contains(&name) {
                // 裸旗标无值。
            } else if name.starts_with('-') {
                builtin_usage_error(op, &format!("未知旗标 `{name}`"), allowed);
            }
            // 其余位置参数（非旗标）一律忽略：它们只能来自全局位置面。
        }
        Self { values }
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .rev()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    fn require(&self, op: &str, name: &str, allowed: &[&str]) -> String {
        match self.get(name) {
            Some(value) if !value.trim().is_empty() => value.to_owned(),
            _ => builtin_usage_error(op, &format!("缺少必填旗标 `{name}`"), allowed),
        }
    }
}

/// 用法错误：退出 2 并列出该 op 的合法旗标表（**不**打印全局 usage：那会把
/// 一行定位信息埋在十几行通用帮助里）。
fn builtin_usage_error(op: &str, message: &str, allowed: &[&str]) -> ! {
    eprintln!("error: --builtin-tool {op}: {message}");
    eprintln!("合法旗标：{}", allowed.join(" "));
    exit(2)
}

/// wire 安全的字段值：`;` / `,` / `|` / 换行 / NUL 一律换成 `_`。
///
/// git 路径**可以**合法地包含逗号与空格（以及任意非 NUL 字节），所以不清洗
/// 就会把外层 `tool.result` 的字段边界撕碎。
fn wire_sanitize(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            ';' | ',' | '|' | '\n' | '\r' | '\0' => '_',
            other => other,
        })
        .collect()
}

/// 失败行：`op=<name>;error=<slug>;detail=<sanitized>`（stdout 保持空）。
fn builtin_fail(op: &str, slug: &str, detail: &str) -> i32 {
    eprintln!(
        "op={};error={};detail={}",
        wire_sanitize(op),
        wire_sanitize(slug),
        wire_sanitize(detail)
    );
    1
}

/// 捕获 `git -C <repo> …` 的标准输出（非零退出 = `Err`，带 git 自己的 stderr）。
fn git_capture(repo: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|err| format!("执行 git {} 失败：{err}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail = stderr.trim();
        return Err(format!(
            "git {} 失败：{}",
            args.join(" "),
            if tail.is_empty() {
                "非零退出"
            } else {
                tail
            }
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// 必须恰好一行（`rev-parse HEAD` 这类）。
fn git_one(repo: &Path, args: &[&str]) -> Result<String, String> {
    let text = git_capture(repo, args)?;
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    match lines.as_slice() {
        [line] => Ok((*line).to_owned()),
        _ => Err(format!(
            "git {} 的输出不是恰好一行（{} 行）",
            args.join(" "),
            lines.len()
        )),
    }
}

/// 逐行 trim（空行丢弃）。
fn git_lines(repo: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    Ok(git_capture(repo, args)?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

/// 目标仓库根（`--repo` 或 CWD 的 toplevel）。
fn repo_root(explicit: Option<&str>) -> Result<PathBuf, String> {
    let start = match explicit {
        Some(dir) => PathBuf::from(dir),
        None => std::env::current_dir().map_err(|err| format!("取 CWD 失败：{err}"))?,
    };
    let text = git_one(&start, &["rev-parse", "--show-toplevel"])?;
    Ok(PathBuf::from(text))
}

/// 仓库名（worktree 目录名的前缀；取不到就用 `repo` 兵底）。
fn repo_label(repo: &Path) -> String {
    repo.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".to_owned())
}

/// worktree 根目录：`--root` > `$YGG_WORKTREE_ROOT` > `$HOME/tmp/worktrees` > `<temp>/ygg-worktrees`。
fn worktree_root(explicit: Option<&str>) -> PathBuf {
    worktree_root_for(
        explicit,
        std::env::var("YGG_WORKTREE_ROOT").ok().as_deref(),
        home_dir().as_deref(),
    )
}

fn worktree_root_for(
    explicit: Option<&str>,
    env_root: Option<&str>,
    home: Option<&Path>,
) -> PathBuf {
    if let Some(dir) = explicit.map(str::trim).filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Some(dir) = env_root.map(str::trim).filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Some(home) = home {
        return home.join("tmp").join("worktrees");
    }
    std::env::temp_dir().join("ygg-worktrees")
}

/// lane 号：`1..=64`（每车道一个 worktree，编号是给人和脚本看的稳定键）。
fn lane_number(op: &str, text: &str, allowed: &[&str]) -> u8 {
    match text.trim().parse::<u8>() {
        Ok(value) if (1..=64).contains(&value) => value,
        _ => builtin_usage_error(
            op,
            &format!("--lane 需要 1..=64 的整数（当前是 `{text}`）"),
            allowed,
        ),
    }
}

/// 分支名：`[A-Za-z0-9._/-]`，无 `..`、不以 `/` 开头。
fn validate_branch(op: &str, branch: &str, allowed: &[&str]) -> String {
    let ok = !branch.is_empty()
        && !branch.starts_with('/')
        && !branch.contains("..")
        && branch
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '/' | '-'));
    if !ok {
        builtin_usage_error(
            op,
            &format!(
                "--branch `{branch}` 不合法（只允许 `[A-Za-z0-9._/-]`，不得含 `..` 或以 `/` 开头）"
            ),
            allowed,
        );
    }
    branch.to_owned()
}

/// lane 的 worktree 路径（claim / release / probe 三处**同源**，否则 release 会
/// 找错目录）。
fn lane_worktree_path(root: &Path, repo: &Path, lane: u8) -> PathBuf {
    root.join(format!("{}-lane-{lane}", repo_label(repo)))
}

/// 把字段拼成**一行** wire 文本（纯函数：格式断言直接测它）。
///
/// 键序 = 传入序（调用方按位解析，别重排）；值逐个过 [`wire_sanitize`]。
fn wire_line(fields: &[(&str, String)]) -> String {
    fields
        .iter()
        .map(|(key, value)| format!("{key}={}", wire_sanitize(value)))
        .collect::<Vec<_>>()
        .join(";")
}

/// 输出一行并返回退出码（成功路径的唯一出口；失败不走这里）。
fn builtin_line(fields: &[(&str, String)]) -> i32 {
    println!("{}", wire_line(fields));
    0
}

/// 失败形状：能成型为 `k=v;k=v` 行的直接给行，其余由 [`run_op`] 拼通用形状。
#[derive(Debug)]
enum OpFailure {
    /// 已经成型的一行（`lane=<k>;error=lane-busy;on=…;head=…` 这类带定位的现场）。
    Line(String),
    /// 通用失败：`op=<name>;error=<slug>;detail=<sanitized>`。
    Detail { slug: &'static str, detail: String },
    /// 已经自己在 stderr 上把话说完了（如 `verify-suite` 的 gate 尾部）：
    /// wrapper 不再补一行，只保持 stdout 空 + 退出 1。
    Reported,
}

impl OpFailure {
    fn detail(slug: &'static str, detail: impl std::fmt::Display) -> Self {
        Self::Detail {
            slug,
            detail: detail.to_string(),
        }
    }
}

/// 成功 → 打一行并退出 0；失败 → **stdout 保持空**、stderr 一行、退出 1。
fn run_op(op: &str, result: Result<Vec<(&'static str, String)>, OpFailure>) -> i32 {
    match result {
        Ok(fields) => builtin_line(&fields),
        Err(OpFailure::Line(line)) => {
            eprintln!("{line}");
            1
        }
        Err(OpFailure::Detail { slug, detail }) => builtin_fail(op, slug, &detail),
        Err(OpFailure::Reported) => 1,
    }
}

/// 内置工具入口（六个 op 的分发；argv 在 [`OpFlags`] 里二次解析）。
fn run_builtin_tool(args: &Args) -> i32 {
    let Some(tool) = args.builtin_tool else {
        // main 只在 `is_some()` 时进来；防御性兜底不走 usage（那是 2 号退出）。
        return builtin_fail("builtin", "no-op", "缺少 --builtin-tool 取值");
    };
    match tool {
        BuiltinTool::WorktreeClaim => run_op(tool.as_str(), claim_fields()),
        BuiltinTool::WorktreeRelease => run_op(tool.as_str(), release_fields()),
        BuiltinTool::LaneProbe => run_op(tool.as_str(), lane_probe_fields()),
        BuiltinTool::ArtifactProbe => run_op(tool.as_str(), artifact_probe_fields()),
        BuiltinTool::VerifySuite => run_op(tool.as_str(), verify_suite_fields()),
        BuiltinTool::ArchiveSessions => run_op(tool.as_str(), archive_sessions_fields(args)),
    }
}

// ---- worktree-claim ----

const CLAIM_FLAGS: &[&str] = &["--lane", "--req", "--branch", "--root", "--repo"];

fn claim_fields() -> Result<Vec<(&'static str, String)>, OpFailure> {
    claim_fields_with(&OpFlags::parse("worktree-claim", CLAIM_FLAGS))
}

/// [`claim_fields`] 的注入面（测试直接给旗标，不读进程 argv）。
fn claim_fields_with(flags: &OpFlags) -> Result<Vec<(&'static str, String)>, OpFailure> {
    const OP: &str = "worktree-claim";
    let lane = lane_number(OP, &flags.require(OP, "--lane", CLAIM_FLAGS), CLAIM_FLAGS);
    let req = flags.require(OP, "--req", CLAIM_FLAGS);
    let branch = validate_branch(OP, &flags.require(OP, "--branch", CLAIM_FLAGS), CLAIM_FLAGS);
    // `req` 形态：`r<数字>`（Issue 里泳道的稳定标识）。
    let req_ok =
        req.len() > 1 && req.starts_with('r') && req[1..].chars().all(|ch| ch.is_ascii_digit());
    if !req_ok {
        builtin_usage_error(
            OP,
            &format!("--req 需要 `r<数字>` 形态（当前是 `{req}`）"),
            CLAIM_FLAGS,
        );
    }

    let repo = repo_root(flags.get("--repo")).map_err(|err| OpFailure::detail("repo", err))?;
    let base =
        git_one(&repo, &["rev-parse", "HEAD"]).map_err(|err| OpFailure::detail("repo", err))?;
    let wt = lane_worktree_path(&worktree_root(flags.get("--root")), &repo, lane);

    // ⓪ 若 `<branch>` 已被其他 worktree 检出（例如 gate reject 后需求重派到另一条泳道），
    // 先将那个 worktree 切为 `--detach HEAD` 释放分支锁。
    let want = fs::canonicalize(&wt).unwrap_or_else(|_| wt.clone());
    let branch_ref = format!("refs/heads/{branch}");
    if let Ok(lines) = git_lines(&repo, &["worktree", "list", "--porcelain"]) {
        let mut current_wt: Option<PathBuf> = None;
        for line in &lines {
            if let Some(path) = line.strip_prefix("worktree ") {
                current_wt = Some(PathBuf::from(path));
            } else if let Some(b) = line.strip_prefix("branch ") {
                if b == branch_ref {
                    if let Some(ref other_wt) = current_wt {
                        let got = fs::canonicalize(other_wt).unwrap_or_else(|_| other_wt.clone());
                        if got != want && other_wt.is_dir() {
                            let _ = git_capture(other_wt, &["checkout", "--detach", "HEAD"]);
                        }
                    }
                }
            }
        }
    }

    // ① 若泳道目录与 `.git` 已存在（持久化泳道池）：原位复用（`reused=1`），
    // `clean -fd` 不带 `-x`，保留 `target/`、`node_modules/` 等 gitignored 构建缓存。
    if wt.is_dir() && wt.join(".git").exists() {
        git_capture(&wt, &["checkout", "--force", "-B", &branch, &base])
            .map_err(|err| OpFailure::detail("worktree-checkout", err))?;
        git_capture(&wt, &["reset", "--hard", &base])
            .map_err(|err| OpFailure::detail("worktree-reset", err))?;
        git_capture(&wt, &["clean", "-fd"])
            .map_err(|err| OpFailure::detail("worktree-clean", err))?;
        return Ok(claim_output(lane, &req, &branch, &base, true, &wt));
    }

    // ② 新建（`reused=0`）：先 prune 清理可能残留的陈旧注册，再用 `-B` 挂载分支。
    if let Some(parent) = wt.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| OpFailure::detail("mkdir", format!("{}：{err}", parent.display())))?;
    }
    let _ = git_capture(&repo, &["worktree", "prune"]);
    let wt_text = wt.display().to_string();
    git_capture(&repo, &["worktree", "add", "-B", &branch, &wt_text, &base])
        .map_err(|err| OpFailure::detail("worktree-add", err))?;

    // ③ 新 worktree 必须干净：脏 = 基线本身脏，**报告，不修**。
    let status = git_lines(&wt, &["status", "--porcelain"])
        .map_err(|err| OpFailure::detail("status", err))?;
    if !status.is_empty() {
        return Err(OpFailure::Line(format!(
            "lane={lane};req={req};branch={branch};clean=0;path={}",
            wire_sanitize(&wt.display().to_string())
        )));
    }
    Ok(claim_output(lane, &req, &branch, &base, false, &wt))
}

/// 成功行（**键序固定**：调用方按位解析，别重排）。
fn claim_output(
    lane: u8,
    req: &str,
    branch: &str,
    base: &str,
    reused: bool,
    wt: &Path,
) -> Vec<(&'static str, String)> {
    let path = fs::canonicalize(wt)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| wt.display().to_string());
    vec![
        ("lane", lane.to_string()),
        ("req", req.to_owned()),
        ("branch", branch.to_owned()),
        ("base", base.to_owned()),
        ("clean", "1".to_owned()),
        ("reused", if reused { "1" } else { "0" }.to_owned()),
        ("path", path),
    ]
}

// ---- worktree-release ----

const RELEASE_FLAGS: &[&str] = &["--lane", "--root", "--repo"];

fn release_fields() -> Result<Vec<(&'static str, String)>, OpFailure> {
    release_fields_with(&OpFlags::parse("worktree-release", RELEASE_FLAGS))
}

/// [`release_fields`] 的注入面（测试直接给旗标，不读进程 argv）。
fn release_fields_with(flags: &OpFlags) -> Result<Vec<(&'static str, String)>, OpFailure> {
    const OP: &str = "worktree-release";
    let lane = lane_number(
        OP,
        &flags.require(OP, "--lane", RELEASE_FLAGS),
        RELEASE_FLAGS,
    );
    let repo = repo_root(flags.get("--repo")).map_err(|err| OpFailure::detail("repo", err))?;
    let wt = lane_worktree_path(&worktree_root(flags.get("--root")), &repo, lane);
    if wt.is_dir() && wt.join(".git").exists() {
        git_capture(&wt, &["checkout", "--force", "--detach", "HEAD"])
            .map_err(|err| OpFailure::detail("worktree-detach", err))?;
        git_capture(&wt, &["reset", "--hard", "HEAD"])
            .map_err(|err| OpFailure::detail("worktree-reset", err))?;
        git_capture(&wt, &["clean", "-fd"])
            .map_err(|err| OpFailure::detail("worktree-clean", err))?;
        let path = fs::canonicalize(&wt)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| wt.display().to_string());
        return Ok(vec![
            ("lane", lane.to_string()),
            ("released", "1".to_owned()),
            ("parked", "1".to_owned()),
            ("path", path),
        ]);
    }
    // 目录不存在：prune 清理可能残留的注册。
    let _ = git_capture(&repo, &["worktree", "prune"]);
    Ok(vec![
        ("lane", lane.to_string()),
        ("released", "1".to_owned()),
        ("parked", "0".to_owned()),
        ("path", wt.display().to_string()),
    ])
}

// ---- lane-probe ----

const PROBE_FLAGS: &[&str] = &["--lane", "--base", "--branch", "--root", "--repo"];

fn lane_probe_fields() -> Result<Vec<(&'static str, String)>, OpFailure> {
    lane_probe_fields_with(&OpFlags::parse("lane-probe", PROBE_FLAGS))
}

/// [`lane_probe_fields`] 的注入面（测试直接给旗标，不读进程 argv）。
fn lane_probe_fields_with(flags: &OpFlags) -> Result<Vec<(&'static str, String)>, OpFailure> {
    const OP: &str = "lane-probe";
    let lane = lane_number(OP, &flags.require(OP, "--lane", PROBE_FLAGS), PROBE_FLAGS);
    // `base` 原样回显（短 sha 保持短）：调用方要能对上自己的台账。
    let base = flags.require(OP, "--base", PROBE_FLAGS);
    let branch = validate_branch(OP, &flags.require(OP, "--branch", PROBE_FLAGS), PROBE_FLAGS);
    let repo = repo_root(flags.get("--repo")).map_err(|err| OpFailure::detail("repo", err))?;
    let spec = format!("{base}..{branch}");
    let commits = git_one(&repo, &["rev-list", "--count", &spec])
        .map_err(|err| OpFailure::detail("rev-list", err))?;
    let head = git_one(&repo, &["rev-parse", &branch])
        .map_err(|err| OpFailure::detail("rev-parse", err))?;
    // 干净度在**泳道 worktree**（若已在）里测：车道自己的未提交改动不在仓库根。
    let lane_wt = lane_worktree_path(&worktree_root(flags.get("--root")), &repo, lane);
    let tree = if lane_wt.is_dir() {
        lane_wt
    } else {
        repo.clone()
    };
    let clean = git_lines(&tree, &["status", "--porcelain"])
        .map(|lines| u8::from(lines.is_empty()))
        .map_err(|err| OpFailure::detail("status", err))?;
    let mut files = git_lines(&repo, &["diff", "--name-only", &spec])
        .map_err(|err| OpFailure::detail("diff", err))?;
    files.sort();
    Ok(vec![
        ("lane", lane.to_string()),
        ("branch", branch),
        ("base", base),
        ("head", head),
        ("commits", commits),
        ("clean", clean.to_string()),
        ("files", files.join(",")),
    ])
}

// ---- artifact-probe ----

const ARTIFACT_PROBE_FLAGS: &[&str] = &["--path", "--run-dir"];

/// `artifact-probe`：本局某件制品在不在、多大、什么摘要。
///
/// **永远退出 0**（只要读取路径本身没崩）：这是个**探针**，`exists=0` 是 Mod
/// 必须能读到的合法答案（它据此分支）。别把它「修」成缺失即退出 1。
fn artifact_probe_fields() -> Result<Vec<(&'static str, String)>, OpFailure> {
    artifact_probe_fields_with(&OpFlags::parse("artifact-probe", ARTIFACT_PROBE_FLAGS))
}

/// [`artifact_probe_fields`] 的注入面（测试直接给旗标，不读进程 argv）。
fn artifact_probe_fields_with(flags: &OpFlags) -> Result<Vec<(&'static str, String)>, OpFailure> {
    const OP: &str = "artifact-probe";
    let path = flags.require(OP, "--path", ARTIFACT_PROBE_FLAGS);
    if let Err(err) = ygg_store::validate_artifact_path(&path) {
        // 调用方 bug（不是 miss）：违法的路径不能当成「不存在」。
        return Err(OpFailure::detail("path", err));
    }
    let run_dir = flags.get("--run-dir").map(PathBuf::from).or_else(|| {
        std::env::var("YGG_RUN_DIR")
            .ok()
            .filter(|dir| !dir.trim().is_empty())
            .map(PathBuf::from)
    });
    let Some(run_dir) = run_dir else {
        builtin_usage_error(
            OP,
            "需要 --run-dir（或环境变量 YGG_RUN_DIR）",
            ARTIFACT_PROBE_FLAGS,
        );
    };
    match probe_artifact(&run_dir, &path).map_err(|err| OpFailure::detail("read", err))? {
        Some(record) => Ok(vec![
            ("path", record.meta.path),
            ("exists", "1".to_owned()),
            ("bytes", record.meta.bytes.to_string()),
            ("sha256", record.meta.sha256),
        ]),
        None => Ok(vec![
            ("path", path),
            ("exists", "0".to_owned()),
            ("bytes", "0".to_owned()),
            ("sha256", String::new()),
        ]),
    }
}

/// 读一件制品（已存在的局走端口；不存在的局走纯文件面——**不建库**）。
fn probe_artifact(run_dir: &Path, path: &str) -> Result<Option<ygg_store::ArtifactRecord>, String> {
    if !run_dir_exists(run_dir) {
        return Ok(read_artifact_file(run_dir, path));
    }
    match open_store_resolved(run_dir, None) {
        Ok(store) => store.artifact_get(path).map_err(|err| err.to_string()),
        // 端口打不开（混代 / 坏库）：回落文件面，而不是把探针变成失败。
        Err(_) => Ok(read_artifact_file(run_dir, path)),
    }
}

/// 纯文件面读制品（JSON 后端布局；不存在 = `None`）。
fn read_artifact_file(run_dir: &Path, path: &str) -> Option<ygg_store::ArtifactRecord> {
    let content = fs::read(run_dir.join(path)).ok()?;
    Some(ygg_store::ArtifactRecord {
        meta: ygg_store::ArtifactMeta {
            path: path.to_owned(),
            bytes: content.len(),
            // 宿主不自己拼一份 SHA-256：用端口暴露的唯一实现（同编码口径）。
            sha256: ygg_store::sha256_hex(&content),
            updated_at: 0,
        },
        content,
    })
}

// ---- verify-suite ----

const VERIFY_FLAGS: &[&str] = &["--mode", "--repo"];

/// 门禁三件套（`AGENTS.md` §一·「门禁三件套」）——顺序即失败顺序。
fn verify_gates(mode: &str) -> Vec<(&'static str, Vec<&'static str>)> {
    if mode == "fast" {
        vec![(
            "cargo check --workspace --all-targets",
            vec!["check", "--workspace", "--all-targets"],
        )]
    } else {
        vec![
            ("cargo test --workspace", vec!["test", "--workspace"]),
            (
                "cargo test --example egg -p ygg-sandbox",
                vec!["test", "--example", "egg", "-p", "ygg-sandbox"],
            ),
            (
                "cargo test -p ygg-sandbox --test agent_seed_semantics",
                vec![
                    "test",
                    "-p",
                    "ygg-sandbox",
                    "--test",
                    "agent_seed_semantics",
                ],
            ),
        ]
    }
}

fn parse_custom_verify_commands(raw: &str) -> Vec<(String, String, Vec<String>)> {
    raw.split(';')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .filter_map(|segment| {
            let mut parts = segment.split_whitespace();
            let program = parts.next()?.to_owned();
            let args: Vec<String> = parts.map(ToOwned::to_owned).collect();
            Some((segment.to_owned(), program, args))
        })
        .collect()
}

fn verify_suite_fields() -> Result<Vec<(&'static str, String)>, OpFailure> {
    verify_suite_fields_with(&OpFlags::parse("verify-suite", VERIFY_FLAGS))
}

/// [`verify_suite_fields`] 的注入面（测试直接给旗标，不读进程 argv）。
fn verify_suite_fields_with(flags: &OpFlags) -> Result<Vec<(&'static str, String)>, OpFailure> {
    const OP: &str = "verify-suite";
    let mode = flags.get("--mode").unwrap_or("full").trim().to_owned();
    if mode != "fast" && mode != "full" {
        builtin_usage_error(
            OP,
            &format!("--mode 只接受 fast / full（当前是 `{mode}`）"),
            VERIFY_FLAGS,
        );
    }
    // 递归守卫：被 `cargo test` 牵着跑时不要再去抢构建锁。
    if std::env::var("YGG_VERIFY_SUITE").as_deref() == Ok("1") && std::env::var("CARGO").is_ok() {
        return Err(OpFailure::detail(
            "recursion",
            "YGG_VERIFY_SUITE=1 且处在 cargo 环境里，拒绝递归验证",
        ));
    }
    let repo = repo_root(flags.get("--repo")).map_err(|err| OpFailure::detail("repo", err))?;
    let custom = std::env::var("YGG_VERIFY_COMMANDS")
        .ok()
        .map(|raw| parse_custom_verify_commands(&raw))
        .unwrap_or_default();
    let commands: Vec<(String, String, Vec<String>)> = if !custom.is_empty() {
        custom
    } else {
        verify_gates(&mode)
            .into_iter()
            .map(|(label, args)| {
                (
                    label.to_owned(),
                    "cargo".to_owned(),
                    args.into_iter().map(ToOwned::to_owned).collect(),
                )
            })
            .collect()
    };
    let total = commands.len();
    for (index, (label, program, cmd_args)) in commands.iter().enumerate() {
        // `output()` 内部并发排空两个管道——不会因为管道塞满而死锁。
        let outcome = Command::new(program)
            .args(cmd_args)
            .current_dir(&repo)
            .stdin(Stdio::null())
            .output();
        match outcome {
            Ok(output) if output.status.success() => {}
            Ok(output) => {
                // 失败：stdout 保持空，stderr = 一行 header + 末 20 行 sanitized。
                eprintln!("gate={};{label}", index + 1);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let lines: Vec<&str> = stderr.lines().collect();
                let start = lines.len().saturating_sub(20);
                for line in &lines[start..] {
                    eprintln!("{}", wire_sanitize(line));
                }
                return Err(OpFailure::Reported);
            }
            Err(err) => return Err(OpFailure::detail("spawn", format!("{label}：{err}"))),
        }
    }
    // summary 数的是**门数**不是用例数（用例数逐工具链变，会让 wire 抖动）。
    Ok(vec![
        ("passed", "1".to_owned()),
        ("summary", format!("{total}/{total}")),
    ])
}

// ---- archive-sessions ----

const ARCHIVE_FLAGS: &[&str] = &["--parent", "--batch", "--sessions-dir", "--trash"];

/// 标识符白名单（`[A-Za-z0-9._-]`，不含 `..`）。
fn archive_ident_ok(text: &str, max: usize) -> bool {
    !text.is_empty()
        && text.len() <= max
        && !text.contains("..")
        && text
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

/// `archive-sessions`：把提到了某个 parent 会话的 `.jsonl` 转录档搬进 trash。
///
/// **数据红线**（项目 `AGENTS.md` §0 硬约束）：本 op 只碰 agent 会话转录，
/// **绝不能**打开 `*.xlsx` / `*.xls` / `*.csv` 之类业务数据。实现上由两件事
/// 兜底：① 只按扩展名 `.jsonl` 入选；② 文件内容**从不回显**（匹配是布尔值）。
fn archive_sessions_fields(args: &Args) -> Result<Vec<(&'static str, String)>, OpFailure> {
    archive_sessions_fields_with(
        &OpFlags::parse("archive-sessions", ARCHIVE_FLAGS),
        &ArchiveInputs::from_process(args),
    )
}

/// harness 适配器的全部取值（`--harness` / `YGG_HARNESS` / `harness.adapter` 同一份）。
const HARNESS_ADAPTERS: &[&str] = &["auto", "pi", "claude-code", "codex", "generic"];

/// `auto` 探测的**输入面**（纯值：单测注入，不读进程环境）。
struct HarnessProbe {
    pi_session_id: bool,
    claude_code: bool,
    codex_home: bool,
}

/// 工作区路径 → 会话库路径段（`/Users/x/Repo` → `Users-x-Repo`）。
///
/// pi 的 `--<slug>--` 与 Claude Code 的 `-<slug>` 用的是**同一个** slug 口径（都是
/// cwd 去掉前导 `/` 后把 `/` 换成 `-`，差别只在两边的包裹字符）。
fn cwd_slug(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .trim_start_matches('/')
        .replace('/', "-")
}

/// `auto` 的现场探测（纯函数：probe / home / cwd 全可注入，不读进程环境）。
///
/// 顺序即优先级：**Pi 的痕迹最强**（会话环境变量 / 工作区的 `.pi` / 该工作区自己的
/// 会话目录），其次 Claude Code，其次 Codex；都没有 = `generic`——
/// `generic` **没有会话库**，归档记 `moved=0;remaining=0`，这不是失败。
fn detect_harness(probe: &HarnessProbe, home: Option<&Path>, cwd: &Path) -> &'static str {
    let pi_sessions = home.map(|home| {
        home.join(".pi")
            .join("agent")
            .join("sessions")
            .join(format!("--{}--", cwd_slug(cwd)))
    });
    if probe.pi_session_id || cwd.join(".pi").exists() || pi_sessions.is_some_and(|d| d.is_dir()) {
        return "pi";
    }
    if probe.claude_code || cwd.join(".claude").exists() {
        return "claude-code";
    }
    if probe.codex_home {
        return "codex";
    }
    "generic"
}

/// 三层解析的**唯一收敛点**：`explicit`（`--harness`）> `env_adapter`
/// （`YGG_HARNESS`，其次 `data/config.json` 的 `harness.adapter`）> 现场探测。
///
/// 返回值恒为 [`HARNESS_ADAPTERS`] 里的一个**非 auto** 值——调用方拿到就能直接查表。
/// 未知取值（`--harness` 已在解析期点名校验，这里只剩程序化调用）→ 当 auto 处理。
fn resolve_harness_adapter(
    explicit: Option<&str>,
    env_adapter: Option<&str>,
    cwd: &Path,
) -> &'static str {
    let requested = [explicit, env_adapter]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|value| !value.is_empty())
        .unwrap_or("auto");
    match requested {
        "pi" => "pi",
        "claude-code" => "claude-code",
        "codex" => "codex",
        "generic" => "generic",
        _ => detect_from_process(cwd),
    }
}

/// `auto` 的**进程探测**（环境变量 + `$HOME`；纯函数 [`detect_harness`] 的取值层）。
fn detect_from_process(cwd: &Path) -> &'static str {
    let probe = HarnessProbe {
        pi_session_id: std::env::var_os("PI_SESSION_ID").is_some(),
        claude_code: std::env::var_os("CLAUDE_CODE").is_some(),
        codex_home: std::env::var_os("CODEX_HOME").is_some(),
    };
    let home = home_dir();
    detect_harness(&probe, home.as_deref(), cwd)
}

/// `$HOME`（空 / 非 UTF-8 一律视为缺席：会话库是**推导面**，不是契约面）。
fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .filter(|home| !home.trim().is_empty())
        .map(PathBuf::from)
}

/// 每个适配器自己的会话库落点 `(sessions 目录, trash 根)`。
///
/// `generic`（以及任何未知适配器）→ `None`：它没有会话库，归档只能记 0/0 台账。
fn harness_session_dirs(adapter: &str, home: &Path, slug: &str) -> Option<(PathBuf, PathBuf)> {
    match adapter {
        "pi" => Some((
            home.join(".pi")
                .join("agent")
                .join("sessions")
                .join(format!("--{slug}--")),
            home.join(".pi").join("agent").join("sessions-trash"),
        )),
        "claude-code" => Some((
            home.join(".claude")
                .join("projects")
                .join(format!("-{slug}")),
            home.join(".claude").join("sessions-trash"),
        )),
        "codex" => Some((
            home.join(".codex").join("sessions"),
            home.join(".codex").join("sessions-trash"),
        )),
        _ => None,
    }
}

/// `data/config.json` 的 `harness.adapter`（路径由 `pack.json` 声明；缺段 / 非串 / 空 →
/// `None`）。这一层只是**静态默认**：本 Mod 的 `agent-config.json` 已把取值限定在
/// [`HARNESS_ADAPTERS`]，所以这里不再重复校验枚举。
fn harness_config_adapter(mod_dir: Option<&Path>) -> Option<String> {
    let dir = mod_dir?;
    let config = read_json(&dir.join(agent_config_path(dir)))?;
    let adapter = config["harness"]["adapter"].as_str()?.trim();
    (!adapter.is_empty()).then(|| adapter.to_owned())
}

/// `archive-sessions` 的**输入面**：进程环境与旗标的唯一注入点（单测不碰 env）。
///
/// Mod dir 决定「`harness.adapter` 这一层能否读到」：`--mod` 优先，其次宿主给 worker
/// 注入的 `$ENV{YGG_MOD_DIR}`（工具桥的常规路径）；两者都缺 = 该层不表态。
struct ArchiveInputs {
    harness: Option<String>,
    mod_dir: Option<PathBuf>,
    sessions_dir: Option<PathBuf>,
    home: Option<PathBuf>,
    cwd: Option<PathBuf>,
    env_harness: Option<String>,
}

impl ArchiveInputs {
    fn from_process(args: &Args) -> Self {
        let env = |key: &str| {
            std::env::var(key)
                .ok()
                .filter(|value| !value.trim().is_empty())
        };
        Self {
            harness: args.harness.clone(),
            mod_dir: args
                .mod_dir
                .clone()
                .or_else(|| env("YGG_MOD_DIR").map(PathBuf::from)),
            sessions_dir: env("YGG_SESSIONS_DIR").map(PathBuf::from),
            home: env("HOME").map(PathBuf::from),
            cwd: std::env::current_dir().ok(),
            env_harness: env("YGG_HARNESS"),
        }
    }

    /// 本局生效的适配器：`--harness` > `YGG_HARNESS` > `harness.adapter` > auto。
    fn adapter(&self) -> &'static str {
        let config = harness_config_adapter(self.mod_dir.as_deref());
        let declared = self
            .env_harness
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .or(config.as_deref());
        let cwd = self.cwd.clone().unwrap_or_else(|| PathBuf::from("."));
        resolve_harness_adapter(self.harness.as_deref(), declared, &cwd)
    }
}

/// 成功回执的三段（`moved=..;remaining=..;dest=..`；顺序稳定便于比对）。
fn archive_outcome(moved: usize, remaining: usize, dest: String) -> Vec<(&'static str, String)> {
    vec![
        ("moved", moved.to_string()),
        ("remaining", remaining.to_string()),
        ("dest", dest),
    ]
}

/// [`archive_sessions_fields`] 的注入面（测试直接给旗标 + 输入面，不读进程 argv / env）。
fn archive_sessions_fields_with(
    flags: &OpFlags,
    inputs: &ArchiveInputs,
) -> Result<Vec<(&'static str, String)>, OpFailure> {
    const OP: &str = "archive-sessions";
    let parent = flags.require(OP, "--parent", ARCHIVE_FLAGS);
    let batch = flags.require(OP, "--batch", ARCHIVE_FLAGS);
    if !archive_ident_ok(&parent, 128) {
        builtin_usage_error(
            OP,
            "--parent 只允许 `[A-Za-z0-9._-]`（1..=128 字节，不含 `..`）",
            ARCHIVE_FLAGS,
        );
    }
    if !archive_ident_ok(&batch, 64) {
        builtin_usage_error(
            OP,
            "--batch 只允许 `[A-Za-z0-9._-]`（1..=64 字节，不含 `..`）",
            ARCHIVE_FLAGS,
        );
    }
    // 优先级：显式 `--sessions-dir` > `YGG_SESSIONS_DIR` > **适配器推导的会话库落点**。
    // 前两层是调用方点名的目录，口径固定（缺目录 = 扫 0 件，不报错、不改写）；
    // 第三层按 harness 适配器推导：目录不存在 = 记 `<适配器>:empty:<batch>`，
    // `generic` 根本没有会话库 = 记 `generic:no-session-store:<batch>`——两者都是
    // **合法答案**（换 harness 本来就收不出东西），不能让收口卡在归档上。
    let explicit_dir = flags
        .get("--sessions-dir")
        .map(PathBuf::from)
        .or_else(|| inputs.sessions_dir.clone());
    let (sessions_dir, default_trash_root) = match explicit_dir {
        Some(dir) => {
            let trash_root = dir.parent().unwrap_or_else(|| Path::new(".")).join("trash");
            (dir, trash_root)
        }
        None => {
            let adapter = inputs.adapter();
            let cwd = inputs.cwd.clone().unwrap_or_else(|| PathBuf::from("."));
            let dirs = inputs
                .home
                .as_deref()
                .and_then(|home| harness_session_dirs(adapter, home, &cwd_slug(&cwd)));
            let Some((dir, trash_root)) = dirs else {
                // 适配器没有会话库（generic）= 合法答案；有库但推不出来（缺 HOME）= 用法错误。
                if adapter == "generic" {
                    return Ok(archive_outcome(
                        0,
                        0,
                        format!("{adapter}:no-session-store:{batch}"),
                    ));
                }
                builtin_usage_error(
                    OP,
                    "需要 --sessions-dir（或环境变量 YGG_SESSIONS_DIR / HOME）",
                    ARCHIVE_FLAGS,
                );
            };
            if !dir.is_dir() {
                return Ok(archive_outcome(0, 0, format!("{adapter}:empty:{batch}")));
            }
            (dir, trash_root)
        }
    };
    let trash = flags
        .get("--trash")
        .map(PathBuf::from)
        .unwrap_or_else(|| default_trash_root.join(&batch));
    let outcome = sweep_sessions(&sessions_dir, &trash, &parent)
        .map_err(|err| OpFailure::detail("sweep", err))?;
    let dest = fs::canonicalize(&trash)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| trash.display().to_string());
    if outcome.remaining > 0 {
        for failure in &outcome.failures {
            eprintln!("archive={}", wire_sanitize(failure));
        }
        // 清扫不全：行已打（调用方看得见进度），但退出码是失败。
        return Err(OpFailure::Detail {
            slug: "incomplete",
            detail: format!(
                "moved={};remaining={};dest={}",
                outcome.moved,
                outcome.remaining,
                wire_sanitize(&dest)
            ),
        });
    }
    Ok(archive_outcome(outcome.moved, outcome.remaining, dest))
}

/// 一次清扫的结果（`remaining > 0` = 清扫不全）。
struct SweepOutcome {
    moved: usize,
    remaining: usize,
    failures: Vec<String>,
}

/// 逐件扫 `sessions_dir` 下提到了 `parent` 的 `.jsonl`，rename 进 `trash`，
/// 然后重扫数剩下的。**匹配是布尔值：文件内容永不出现在输出里。**
fn sweep_sessions(sessions_dir: &Path, trash: &Path, parent: &str) -> Result<SweepOutcome, String> {
    fs::create_dir_all(trash).map_err(|err| format!("创建 {} 失败：{err}", trash.display()))?;
    let matches = |path: &Path, name: &str| -> bool {
        if !name.ends_with(".jsonl") || name.contains(parent) {
            return false;
        }
        fs::read_to_string(path)
            .map(|contents| contents.contains(parent))
            .unwrap_or(false)
    };

    let mut moved = 0usize;
    let mut failures: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(sessions_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !matches(&entry.path(), &name) {
                continue;
            }
            match fs::rename(entry.path(), trash.join(&name)) {
                Ok(()) => moved += 1,
                Err(err) => failures.push(format!("{name}：{err}")),
            }
        }
    }
    let mut remaining = 0usize;
    if let Ok(entries) = fs::read_dir(sessions_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if matches(&entry.path(), &name) {
                remaining += 1;
            }
        }
    }
    Ok(SweepOutcome {
        moved,
        remaining,
        failures,
    })
}

/// 重置原因（写进后端的归档台账；只用于审计，不参与判定）。
fn reset_reason(recheck: bool) -> &'static str {
    if recheck {
        "resume-failed"
    } else {
        "explicit-new-game"
    }
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
    static RUN_LOCK_DEPTH: Cell<u32> = const { Cell::new(0) };
    /// 真实持锁期间的锁文件路径（心跳续命用）；重入占位与未持锁时为空。
    /// Drop（真实持有者）归零 depth 时同步清空，心跳不得越权触碰别人的锁。
    static RUN_LOCK_FILE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
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

/// opener 冷启动认领（§r3：本局首条**携带身份**的成功写先到先得、不可变）。
///
/// `run.json` 增可选字段 `opener_actor` = `{actor, session?, at}`：读档发现已
/// 存在 → `false`（连 `at` 都不刷新——开局时刻是事实，同 id 重复 claim 也是
/// no-op）；缺失 → tmp+rename 原子写入并返回 `true`。锁可重入：inbox 路径已
/// 持 token、dispatch 路径在持久化临界区内，同进程重入直接成功，跨进程由
/// `coordination.lock` 串行——「不同 actor 竞争首个 claim」严格先到先得，
/// 后来者不报错不拒绝（多 subagent 各带身份写命令是常态）。
///
/// 无身份（actor 空）的调用方**根本不进**本函数：失败探针 / 被拒派发不认领，
/// 开局者只可能是一条真实发生且带身份的成功写。
fn claim_opener(run_dir: &Path, actor: &str, session: Option<&str>) -> Result<bool, String> {
    let _guard = acquire_run_lock(run_dir)?;
    let meta_path = run_dir.join(META_FILE);
    let mut meta = read_meta(&meta_path).unwrap_or_else(|| json!({}));
    if meta["opener_actor"]["actor"].as_str().is_some() {
        return Ok(false); // 先到先得、不可变：已有开局者，包括同 id 重复 claim
    }
    let mut opener = json!({ "actor": actor, "at": now_secs() });
    if let Some(session_id) = session {
        opener["session"] = json!(session_id);
    }
    meta["opener_actor"] = opener;
    let mut text = serde_json::to_string_pretty(&meta)
        .map_err(|err| format!("序列化 {} 失败：{err}", meta_path.display()))?;
    text.push('\n');
    atomic_write(&meta_path, text.as_bytes())
        .map_err(|err| format!("写 opener_actor 失败：{err}"))?;
    Ok(true)
}

/// 读本局已认领的 opener（run.json 是元信息面：读不到 = 尚无开局者）。
fn read_opener(run_dir: &Path) -> Option<String> {
    read_meta(&run_dir.join(META_FILE))?["opener_actor"]["actor"]
        .as_str()
        .map(str::to_owned)
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
    let prefix: String = run_id.chars().take(24).collect();
    format!("inbox-{prefix}-{line_index}-{nanos}-{}", std::process::id())
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

    // 单发是 Agent 脚本化的主通道：解析链（旗标 > 环境）已在 open 时折进
    // session.identity，这里原样交给 dispatch_event_as（声明路由注入 / 回显 /
    // opener 认领都在那单点）。未携带身份 = 旧语义逐字节。
    let identity = session.identity.clone();
    let mut receipt = session.dispatch_event_as(
        &event,
        &payload,
        identity.actor.as_deref(),
        identity.session.as_deref(),
    );
    if receipt["ok"] == Json::Bool(true) && receipt.get("harnessActions").is_none() {
        let actions = envelope_harness_actions(&receipt, &session.contract.meta_key);
        if !actions.is_empty() {
            receipt["harnessActions"] = json!(actions);
        }
    }
    if drain.touched() {
        receipt["inbox"] = drain.to_json();
    }
    // new 的稳定输出行（r1）：run-root 是后续一切 open / serve 注册的寻址事实，
    // text 回执末行固定 `run-root: <path>`；raw 信封加 `run_root` 字段（加性）。
    if args.open_style == Some(OpenStyle::New) {
        receipt["run_root"] = json!(session.run_dir.display().to_string());
    }
    if let Some(message) = &drain.error {
        eprintln!(
            "[警告] inbox 未能全部并入：{message}（剩 {} 行未消费）",
            drain.remaining
        );
    }
    // stderr 面（r5-plan §2.4 矩阵）：`drain.error` 是宿主运维错误，双模式都保留；
    // 警告与 `[HARNESS ACTION]` 在缺省 text 模式已并入渲染体（跳过 stderr 重复行），
    // 仅 Raw 模式（机器契约完整快照）照旧走 stderr。
    let text_mode = args.output == OutputFormat::Text;
    if !text_mode {
        // Raw：stdout 只留回执行（Agent 的解析面）；警告另走 stderr 便于人读。
        if let Some(warnings) = receipt["warnings"].as_array() {
            for warning in warnings.iter().filter_map(|value| value.as_str()) {
                eprintln!("[警告] {warning}");
            }
        }
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let env_adapter = std::env::var("YGG_HARNESS")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| harness_config_adapter(Some(&session.mod_dir)));
    if !text_mode
        && resolve_harness_adapter(args.harness.as_deref(), env_adapter.as_deref(), &cwd)
            == "generic"
    {
        for line in format_harness_action_lines(&receipt) {
            eprintln!("{line}");
        }
    }
    print!(
        "{}",
        render_receipt_output(&receipt, args.output, &session.contract.meta_key)
    );
    if args.open_style == Some(OpenStyle::New) && text_mode {
        println!("run-root: {}", session.run_dir.display());
    }
    if receipt["ok"] == Json::Bool(true) {
        0
    } else {
        1
    }
}

/// 退役专名拒绝（别名退役，原双向别名机制已删除）：形状=父目录名 `mods` +
/// 末段精确 `pi-agent-seed` 时给出可读理由——点名同父规范路径 `agent-seed`、
/// 宣布退役日期与「历史 run 不迁移、开新 run-id」。只认精确形状：
/// 前缀撞名（`-v2`/`-s`）与无 `mods` 父段的裸值一律不命中（装载期通用错误）。
fn retired_seed_name_reason(requested: &Path) -> Option<String> {
    if requested.file_name()?.to_str()? != "pi-agent-seed" {
        return None;
    }
    let parent = requested.parent()?;
    if parent.file_name()?.to_str()? != "mods" {
        return None;
    }
    Some(format!(
        "Mod 目录名 `pi-agent-seed` 已于 2026-11-12 退役（更名回 `agent-seed`，双向别名机制已删除）\
         ——规范路径：{}（同父目录）；历史 run.json 不迁移：请用新 run-id 开新局",
        parent.join("agent-seed").display()
    ))
}

/// 本局的 Mod 目录：`--mod` 优先，其次 run.json 里记录的本局 mod 路径。
///
/// 两者都没有 → 可读错误。契约是**必填面**：没有 Mod 目录就读不到声明，
/// 宿主不会猜任何默认值（fail-closed）。
/// 旧名 `pi-agent-seed` 已退役（别名退役）：请求路径缺席且命中退役形状时
/// 给专名拒绝（[`retired_seed_name_reason`]）；其余缺席维持既有缺路径错误。
fn resolve_mod_dir(args: &Args, run_dir: &Path) -> Result<PathBuf, String> {
    let requested = args
        .mod_dir
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
        })?;
    if !requested.exists() {
        if let Some(reason) = retired_seed_name_reason(&requested) {
            return Err(reason);
        }
    }
    Ok(requested)
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

/// 内部模式：一次工具调用的执行体（不在 usage 里列出）。
///
/// 不打开 state 写面、不装内核、不派发——只做「读模板 → spawn → 等 → 写结果 →
/// 试认领 →（认领成功时）写 inbox 一条 -> 摘 pending」。这样即使父进程早已
/// 退出（场景 2），结果也能落地（比「父进程自己等」更强的崩溃语义）。
fn run_tool_worker(args: &Args, call_id: &str) -> i32 {
    if !is_valid_call_id(call_id) {
        eprintln!("error: 非法 call_id `{call_id}`（只允许 1..=64 字节 [A-Za-z0-9._-]）");
        return 2;
    }
    let run_dir = args.run_root.clone();
    let Some(call) = read_pending_call(&run_dir, call_id) else {
        // 父进程已收口 / 本局已重置：无活可干（不是错误）。
        return 0;
    };
    let env = load_env_table(args.mod_dir.as_deref());
    let scratch = tool_results_dir(&run_dir);
    if let Err(message) = fs::create_dir_all(&scratch) {
        eprintln!("error: 创建 {} 失败：{message}", scratch.display());
        return 1;
    }
    let outcome = execute_tool(&call, &env, &scratch);
    let external_id = new_external_id(run_basename(&args.run_root), 0);
    let result = ToolResult::from_outcome(&call, &outcome, &external_id);
    if let Err(message) = write_tool_result(&run_dir, &result) {
        eprintln!("error: {message}");
        return 1;
    }
    // 交付责任要等父进程的等待窗口过去（否则快工具永远抢不到场景 1）：
    // 结果文件已经写好，父进程在 `inlineWaitMs` 内看到就会就地认领并派发。
    let grace_until = if call.inline_wait_ms == 0 {
        0
    } else {
        call.started_at_unix_ms + u128::from(call.inline_wait_ms) + u128::from(TOOL_CLAIM_SLACK_MS)
    };
    let now = now_ms();
    if grace_until > now {
        std::thread::sleep(Duration::from_millis((grace_until - now) as u64));
    }
    // 父进程已先认领 → 派发责任在父进程，本进程只留结果文件，静默退出。
    if !claim_tool_result(&run_dir, &call.call_id) {
        return 0;
    }
    // 认领成功 = 交付责任在本进程：写 inbox 一条，成功后摘除 pending 条目。
    let delivery = (|| -> Result<(), String> {
        let mod_dir = resolve_mod_dir(args, &run_dir)?;
        let routes = InboxRoutes::load(&mod_dir)?;
        let kind = routes
            .write_binding("tool-result")
            .map(|(kind, _)| kind.to_owned())
            .ok_or_else(|| {
                format!(
                    "本 Mod 的 pack.json `inboxRoutes` 未声明 `--tool-result` 的写命令；\
                     结果留在 {} 与 {}（可人工 --tool-result 补票；{CONTRACT_DOC}）",
                    PENDING_TOOLS_FILE,
                    tool_result_path(&run_dir, &call.call_id).display()
                )
            })?;
        let mut store = open_store_resolved(&run_dir, args.store)?;
        // worker 自身环境的身份（spawn 侧注入 YGG_ACTOR / YGG_ACTOR_SESSION；
        // 未注入 = 不盖章不认领，交付行为不变）。
        let identity = resolve_identity(args)?;
        append_tool_result_record(
            &run_dir,
            store.as_mut(),
            &routes,
            &kind,
            &result.payload,
            run_basename(&args.run_root),
            &identity,
        )
        .map(|_| ())
    })();
    match delivery {
        Ok(()) => {
            let _ = pending_tools_remove(&run_dir, call_id);
            0
        }
        Err(message) => {
            // 交付失败就**保留** pending：`--wait-tools` 会报 TOOLS_PENDING，
            // 并给出人工补票入口（自动重跑会打破单次执行语义，不做）。
            eprintln!("error: {message}");
            1
        }
    }
}

/// `--wait-tools`：等本局在飞工具全部落地（结果由 worker 写 inbox）后消费一次
/// inbox 并打印回执。
///
/// **只观察**：走 `OpenMode::Attach`——读档失败就失败，绝不重置别人的局。
fn run_wait_tools(args: &Args) -> i32 {
    let run_dir = args.run_root.clone();
    let timeout = args.tool_timeout_ms.unwrap_or(WAIT_TOOLS_DEFAULT_MS);
    let deadline = Instant::now() + Duration::from_millis(timeout);
    let mut last_beat = Instant::now();
    loop {
        let pending = pending_calls(&run_dir);
        if pending.is_empty() {
            break;
        }
        if Instant::now() >= deadline {
            let list: Vec<Json> = pending
                .iter()
                .map(|call| {
                    json!({
                        "callId": call.call_id,
                        "tool": call.tool,
                        "deadlineUnixMs": call.deadline_unix_ms as u64,
                    })
                })
                .collect();
            let message = format!(
                "等待在飞工具超时（{timeout}ms）：{} 个调用仍未落地。worker 崩溃留下的条目\
                 不会自动重跑（重跑会破坏单次执行语义），可用 --tool-result 人工补票",
                pending.len()
            );
            eprintln!("error: {message}");
            println!(
                "{}",
                json!({"ok": false, "error": {"code": "TOOLS_PENDING", "message": message}, "pending": list})
            );
            return 1;
        }
        if last_beat.elapsed() >= WAIT_TOOLS_HEARTBEAT {
            let list = pending
                .iter()
                .map(|call| format!("{}({})", call.call_id, call.tool))
                .collect::<Vec<_>>()
                .join(" ");
            eprintln!("[wait-tools] 仍在飞：{list}");
            last_beat = Instant::now();
        }
        std::thread::sleep(TOOL_POLL_INTERVAL);
    }
    // 在飞清零 → 消费 inbox（结果行已落盘，drain 会按 Mod 的 tool_result 路由派发）
    let mod_dir = match resolve_mod_dir(args, &run_dir) {
        Ok(dir) => dir,
        Err(message) => return wait_tools_failure(&message),
    };
    let open_args = Args {
        mod_dir: Some(mod_dir),
        ..args.clone()
    };
    let mut session = match Session::open_mode(&open_args, OpenMode::Attach) {
        Ok(session) => session,
        Err(message) => return wait_tools_failure(&message),
    };
    let drain = session.drain_inbox();
    let receipt = match &drain.last {
        Some(receipt) => receipt.clone(),
        None => state_receipt(&mut session, "", Vec::new()),
    };
    // stdout 只留回执行（Agent 的解析面）；警告另走 stderr 便于人读。
    if let Some(warnings) = receipt["warnings"].as_array() {
        for warning in warnings.iter().filter_map(Json::as_str) {
            eprintln!("[警告] {warning}");
        }
    }
    println!("{receipt}");
    match &drain.error {
        Some(message) => {
            eprintln!(
                "error: inbox 未能全部并入：{message}（剩 {} 行未消费）",
                drain.remaining
            );
            1
        }
        None => 0,
    }
}

/// `--wait-tools` 的失败回执（与单发模式同一条解析路径）。
fn wait_tools_failure(message: &str) -> i32 {
    eprintln!("error: {message}");
    println!(
        "{}",
        json!({"ok": false, "error": {"code": "INTERNAL", "message": message}})
    );
    1
}

// ============================================================================
// §守护模式（`--daemon`）—— 常驻进程 + 逐行 JSON-RPC over stdio
//
// 为什么要有它：调度端每一步都要「派发 → 读摘要 → 看门禁」三次往返。单发模式
// 每次都重新装载 Mod + 读档，局一大就慢到不可用。守护模式把 `Session` 开一次，
// 之后每条请求只在内存里跑（热载 / inbox 消费仍按原语义每次派发前现查）。
//
// 协议（一行一个 JSON 对象）：
//   请求  {"id": <任意>, "op": "<动作>", ...}
//   应答  {"ok": true|false, "op": "<动作>", "id": <原样回显>, ...}
// `id` 可选；出现即原样回显（stdout 是单通道，调用方凭它配对）。
//
// **stdout 只有协议面**：任何 handler 里都不得 `println!`，人读文本一律 stderr。
//
// 动作表：
//   ping      探活（runId + 存储后端）
//   state     诊断信封（`state` 事件的只读回执，不推进状态）
//   summary   只读摘要（现派发 `summaryEvent`，**不**落 RunStore 步）
//   dispatch  派发一个事件（先热载 + 消费 inbox），回执附带最新 `summary`
//   vfs_put   写制品（端口与内核 `ygg.fs` 同时可见）
//   vfs_get   读制品原文     vfs_list  列制品
//   settle    结算（与独立 `--settle` 同一实现，会终结本局）
//   shutdown  退出守护进程
// ============================================================================

/// 写一行应答并 flush。
///
/// flush 是**承重**的：调用方在另一个进程里阻塞等这一行，攒缓冲就是互相死锁。
fn write_reply(out: &mut impl Write, reply: &Json) -> io::Result<()> {
    writeln!(out, "{reply}")?;
    out.flush()
}

/// 守护模式主循环：开局一次，之后逐行读请求、逐行回应答。
///
/// `--mod` 只为**新局**必需：已有本局时按 `run.json` 记录的 mod 路径解析（与
/// `--settle` / `--wait-tools` 同一条 `resolve_mod_dir`）。
fn run_daemon(args: &Args) -> i32 {
    let run_dir = args.run_root.clone();
    let mod_dir = match resolve_mod_dir(args, &run_dir) {
        Ok(dir) => dir,
        Err(message) => {
            eprintln!("error: {message}");
            return 1;
        }
    };
    // daemon 附着本局（r1：不再有「目录无产物自起新局」路径——新局必须先 `egg new`）。
    // open_style 原样传递：open → Attach（读档续跑）；daemon 挂在 new 下 = Fresh。
    let open_args = Args {
        mod_dir: Some(mod_dir),
        ..args.clone()
    };
    let mut session = match Session::open(&open_args) {
        Ok(session) => session,
        Err(message) => {
            eprintln!("error: {message}");
            return 1;
        }
    };

    let stdin = io::stdin();
    let mut out = io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(err) => {
                eprintln!("error: 读 stdin 失败：{err}");
                return 1;
            }
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // 坏 JSON 不退出：回一条错误应答让调用方自己决定（一行坏请求不该打死整局）。
        let request: Json = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(err) => {
                let reply = json!({
                    "ok": false,
                    "error": {
                        "code": "BAD_REQUEST",
                        "message": format!("请求行不是合法 JSON：{err}"),
                    },
                });
                if write_reply(&mut out, &reply).is_err() {
                    return 1;
                }
                continue;
            }
        };
        let (reply, quit) = handle_daemon_request(&mut session, &open_args, &request);
        if write_reply(&mut out, &reply).is_err() {
            return 1;
        }
        if quit {
            break;
        }
    }
    0
}

/// 错误应答（`op` 原样回填，便于调用方在单通道上配对）。
fn daemon_error(op: &str, message: &str) -> Json {
    json!({
        "ok": false,
        "op": op,
        "error": { "code": "DAEMON", "message": message },
    })
}

/// 一条守护请求的**完整**处理（纯内存 + 端口）：可单测，永不碰 stdout。
///
/// 返回 `(应答, 是否应退出)`：`shutdown` 是唯一把第二项置 true 的动作。
fn handle_daemon_request(session: &mut Session, args: &Args, req: &Json) -> (Json, bool) {
    let op = req["op"].as_str().unwrap_or("");
    let mut quit = false;
    let mut reply = match op {
        "ping" => json!({
            "ok": true,
            "op": "ping",
            "runId": session.run_id.clone(),
            "store": session.store.backend().as_str(),
        }),
        "state" => {
            let mut receipt = state_receipt(session, "state", Vec::new());
            receipt["op"] = json!("state");
            receipt
        }
        "summary" => {
            session.maybe_reload();
            let drain = session.drain_inbox();
            let mut reply = session.query_summary_json();
            reply["op"] = json!("summary");
            if drain.touched() {
                reply["inbox"] = drain.to_json();
            }
            reply
        }
        "dispatch" => daemon_dispatch(session, req),
        "vfs_put" => daemon_vfs_put(session, req),
        "vfs_get" => daemon_vfs_get(session, req),
        "vfs_list" => daemon_vfs_list(session),
        "settle" => daemon_settle(args),
        "shutdown" => {
            quit = true;
            json!({"ok": true, "op": "shutdown"})
        }
        other => daemon_error(
            other,
            &format!(
                "未知的 op `{other}`（只接受 ping / state / summary / dispatch / vfs_put / vfs_get / vfs_list / settle / shutdown）"
            ),
        ),
    };
    // `id` 只在请求给了的时候回填（不给就不凭空造一个，调用方靠 op 也能配对）。
    if let Some(id) = req.get("id") {
        reply["id"] = id.clone();
    }
    (reply, quit)
}

/// `dispatch`：热载 → 消费 inbox → 派发一个事件 → 附上最新 `summary`。
///
/// 一次往返同时给出两类信息（步回执 ⊆ 摘要）：调度端不必再发一条 `summary`，
/// 也就不会看到「派发之后、摘要之前」的中间态。
fn daemon_dispatch(session: &mut Session, req: &Json) -> Json {
    let Some(event) = req["event"].as_str() else {
        return daemon_error("dispatch", "`dispatch` 需要 event 字段（事件名）");
    };
    if event.trim().is_empty() {
        return daemon_error("dispatch", "`dispatch` 的 event 不能为空");
    }
    let payload = match req.get("payload") {
        None | Some(Json::Null) => session.contract.default_payload.clone(),
        // 字符串 = **字符串载荷**（要 JSON 引号包起来才是合法标量）；数字 / 布尔
        // 原样序列化。数组 / 对象不是合法标量（与 `--payload` 同一口径），拒绝。
        Some(Json::String(text)) => serde_json::to_string(text).unwrap_or_default(),
        Some(Json::Number(_)) | Some(Json::Bool(_)) => req["payload"].to_string(),
        Some(other) => {
            return daemon_error(
                "dispatch",
                &format!(
                    "`dispatch` 的 payload 必须是 JSON 标量（当前是 {}）",
                    json_type_name(other)
                ),
            )
        }
    };
    // 提交者身份（解析链与单发同序：旗标 > 环境 > 请求字段；前两层已折进
    // session.identity，请求字段只在前面都缺席时兑底）。请求字段形状非法
    // → 可读错误应答（守护模式没有“用法错误”，拒绝就是应答）。
    let identity = match merge_request_identity(&session.identity, req) {
        Ok(identity) => identity,
        Err(message) => return daemon_error("dispatch", &message),
    };
    // 与单发模式同序：先现查 Mod（热载），再把 inbox 存量排干净，最后派发本次事件。
    session.maybe_reload();
    let drain = session.drain_inbox();
    let mut receipt = session.dispatch_event_as(
        event,
        &payload,
        identity.actor.as_deref(),
        identity.session.as_deref(),
    );
    if drain.touched() {
        receipt["inbox"] = drain.to_json();
    }
    let summary_json = session.query_summary_json();
    if let Some(actions) = summary_json.get("harnessActions") {
        receipt["harnessActions"] = actions.clone();
    }
    receipt["summary"] = summary_json;
    receipt["op"] = json!("dispatch");
    receipt
}

/// daemon `op=dispatch` 请求字段的身份合并：请求里的 `actor` / `actor_session`
/// 只在本进程旗标与环境变量都缺席时兑底（与单发解析链同序）；形状非法 →
/// 可读错误（拒绝就是应答）。
fn merge_request_identity(base: &ActorIdentity, req: &Json) -> Result<ActorIdentity, String> {
    let actor = match base.actor.clone() {
        Some(actor) => Some(actor),
        None => match req["actor"].as_str() {
            Some(raw) => Some(validate_identity_value("actor", raw, 64)?),
            None => None,
        },
    };
    let session = match base.session.clone() {
        Some(session) => Some(session),
        None => match req["actor_session"].as_str() {
            Some(raw) => Some(validate_identity_value("actor_session", raw, 128)?),
            None => None,
        },
    };
    Ok(ActorIdentity { actor, session })
}

/// `vfs_put`：写制品到本局后端，并**同时**灌进内核的 `ygg.fs`。
///
/// 只落盘不灌 VFS 会让同一局里的 `fs.stat` 报「不存在」——单发模式下这个窗口只
/// 有几毫秒（下次开局即一致），守护进程会把它拉到整个局的长度。
fn daemon_vfs_put(session: &mut Session, req: &Json) -> Json {
    let Some(path) = req["path"].as_str() else {
        return daemon_error("vfs_put", "`vfs_put` 需要 path 字段（artifacts/<名>）");
    };
    let Some(content) = req["content"].as_str() else {
        return daemon_error("vfs_put", "`vfs_put` 需要 content 字段（UTF-8 文本）");
    };
    // 两道门都在端口内（宿主不重写一份）：路径形状 + 内容大小。
    if let Err(err) = ygg_store::validate_artifact_path(path) {
        return daemon_error("vfs_put", &err.to_string());
    }
    if let Err(err) = ygg_store::validate_artifact_content(content.as_bytes()) {
        return daemon_error("vfs_put", &err.to_string());
    }
    let bytes = content.as_bytes();
    let meta = session.with_run_lock(|session| {
        session
            .store
            .artifact_put(path, bytes, clock())
            .map_err(|err| err.to_string())
    });
    let meta = match meta {
        Ok(meta) => meta,
        Err(message) => return daemon_error("vfs_put", &message),
    };
    session.kernel.vfs_load(path, bytes, &meta.sha256);
    json!({
        "ok": true,
        "op": "vfs_put",
        "path": meta.path,
        "bytes": meta.bytes,
        "sha256": meta.sha256,
    })
}

/// `vfs_get`：制品原文（缺失 = 带 message 的失败应答，不是 panic）。
fn daemon_vfs_get(session: &mut Session, req: &Json) -> Json {
    let Some(path) = req["path"].as_str() else {
        return daemon_error("vfs_get", "`vfs_get` 需要 path 字段（artifacts/<名>）");
    };
    if let Err(err) = ygg_store::validate_artifact_path(path) {
        return daemon_error("vfs_get", &err.to_string());
    }
    match session.store.artifact_get(path) {
        Ok(Some(record)) => json!({
            "ok": true,
            "op": "vfs_get",
            "path": record.meta.path,
            "bytes": record.meta.bytes,
            "sha256": record.meta.sha256,
            "content": String::from_utf8_lossy(&record.content).into_owned(),
        }),
        Ok(None) => daemon_error(
            "vfs_get",
            &format!(
                "制品 `{path}` 不存在（本局 {}，后端 {}）",
                session.run_dir.display(),
                session.store.backend()
            ),
        ),
        Err(err) => daemon_error("vfs_get", &format!("读制品失败：{err}")),
    }
}

/// `vfs_list`：本局制品清单（升序；与 `ygg.fs.list` 同一份端口事实）。
fn daemon_vfs_list(session: &mut Session) -> Json {
    match session.store.artifact_list() {
        Ok(items) => json!({
            "ok": true,
            "op": "vfs_list",
            "artifacts": items
                .iter()
                .map(|meta| json!({
                    "path": meta.path,
                    "bytes": meta.bytes,
                    "sha256": meta.sha256,
                }))
                .collect::<Vec<_>>(),
        }),
        Err(err) => daemon_error("vfs_list", &format!("列制品失败：{err}")),
    }
}

/// `settle`：**唯一实现**是 `settle_report`（独立 `--settle` 走同一条路）。
///
/// `human=false` 是必须的：结算的人读摘要是 `println!`，在守护模式里会撕裂
/// stdout 的协议面。
fn daemon_settle(args: &Args) -> Json {
    let (code, report) = settle_report(args, false, false);
    match report {
        Some(report) => json!({
            "ok": report["ok"] == Json::Bool(true),
            "op": "settle",
            "exit": code,
            "settlement": report,
        }),
        None => daemon_error(
            "settle",
            "结算在写出报告之前失败（本局不可结算；原因见 stderr）",
        ),
    }
}

/// `--requirement`：只往本局后端的 inbox 追加一条，不读不写 autosave 后退出。
///
/// 这是「新需求随时能进」的真实异步入口：本局在另一个进程里跑时，这里只做一次
/// 单调追加（json = `OpenOptions::append`，sqlite = `INSERT`），不存在与运行
/// 中局的状态竞争。
fn run_inbox_append(args: &Args) -> i32 {
    let Some(write) = args.inbox.clone() else {
        return 2;
    };
    // 提交者身份（旗标 > 环境）：解析失败 = 用法错误（退出 2）；未携带身份
    // 只是不盖章不认领 opener，不改变写命令本身的任何语义（inbox 写路径零变化）。
    let identity = resolve_identity_or_usage(args);
    let run_dir = args.run_root.clone();
    if let Err(err) = fs::create_dir_all(&run_dir) {
        eprintln!("error: 创建 run 目录 {} 失败：{err}", run_dir.display());
        return 2;
    }
    // 写路径同样走端口：sqlite 局没有 inbox.jsonl 可追（追了也永远不被消费）。
    let mut store = match open_store_resolved(&run_dir, args.store) {
        Ok(store) => store,
        Err(message) => {
            eprintln!("error: {message}");
            return 2;
        }
    };
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
    record[external_field] = json!(new_external_id(
        run_basename(&args.run_root),
        store.inbox_len().unwrap_or(0)
    ));
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
    // 提交者章（宿主保留字段，消费方 drain 读回后经 dispatch_event_as 注入声明
    // 路由；None 不写键——记录里没有“空身份”这种东西）。payload 形态的载荷
    // 不掺身份：身份只进 record 键，wire 段由派发侧注入。
    if let Some(actor) = &identity.actor {
        record["actor"] = json!(actor);
    }
    if let Some(session) = &identity.session {
        record["actor_session"] = json!(session);
    }
    let receipt = match store.inbox_push(&record, clock()) {
        Ok(()) => {
            // opener 认领（首条带身份的成功写，先到先得）：失败不阻断已落盘的
            // 行（run.json 是元信息面），降级为 stderr 警告。
            let mut claimed_opener = None;
            if let Some(actor) = &identity.actor {
                match claim_opener(&run_dir, actor, identity.session.as_deref()) {
                    Ok(_) => {}
                    Err(message) => {
                        eprintln!("[警告] opener 认领失败（不影响本条写入）：{message}")
                    }
                }
                claimed_opener = read_opener(&run_dir);
            }
            // 回执键 = 记录的契约字段名（幂等键 / 载荷 / 各命名段）：调用方不必猜。
            let mut receipt = json!({
                "ok": true,
                "inbox": path.display().to_string(),
                "kind": kind,
                // 提交者章回显（None → null）；仅携带身份的写命令回显 opener
                //（null 只在理论竞态失败方出现，锁串行下实际不会）。
                "actor": identity.actor,
                "actor_session": identity.session,
            });
            if identity.has_actor() {
                receipt["opener"] = claimed_opener.map(Json::String).unwrap_or(Json::Null);
            }
            receipt[external_field] = record[external_field].clone();
            receipt[payload_field] = record.get(payload_field).cloned().unwrap_or(Json::Null);
            for field in route.named_fields(external_field) {
                receipt[field.as_str()] = record.get(field.as_str()).cloned().unwrap_or(Json::Null);
            }
            receipt
        }
        Err(message) => {
            let text = message.to_string();
            eprintln!("error: {text}");
            json!({
                "ok": false,
                "inbox": path.display().to_string(),
                "kind": kind,
                "error": {"code": "INBOX", "message": text},
            })
        }
    };
    println!("{receipt}");
    // 行已落盘 → 摘除 token（closing 侧的「等 writers 归零」以此刻为界）。
    let pending = unconsumed_inbox_lines(store.as_ref());
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
    // 回执的 `autosave` 是「存档定位」而不是「一定是 JSON 文件」：本局已声明
    // 后端（`run.json` 的 `store`）为 sqlite 时必须回 sqlite 的定位串，否则
    // 工具会顺着这个字段去找一个根本不存在的 `autosave.json`。裁决顺序与
    // `ygg_store::resolve` 一致：`run.json` 权威 > `--store` 断言 > 产物探针 >
    // 默认后端。**产物探针不能省**：旧 JSON 局在这里没有 `store` 事实，跳过
    // 探针就会把它的错误回执指向一个不存在的 `run.sqlite`（默认后端已是 sqlite）。
    let run_dir = args.run_root.clone();
    let backend = read_meta(&run_dir.join(META_FILE))
        .and_then(|meta| meta["store"].as_str().map(str::to_owned))
        .and_then(|text| Backend::parse(&text))
        .or(args.store)
        .or_else(|| {
            let products = ygg_store::detect(&run_dir);
            if products.sqlite {
                Some(Backend::Sqlite)
            } else if products.json {
                Some(Backend::Json)
            } else {
                None
            }
        })
        .unwrap_or(ygg_store::DEFAULT_BACKEND);
    let autosave = match backend {
        Backend::Json => run_dir.join(AUTOSAVE_FILE).display().to_string(),
        Backend::Sqlite => format!("{}#latest", run_dir.join(SQLITE_FILE_NAME).display()),
    };
    let receipt = json!({
        "ok": false,
        "event": event,
        "narrative": [],
        "pendingChoice": Json::Null,
        "autosave": autosave,
        "error": {"code": "INTERNAL", "message": message},
    });
    if args.output == OutputFormat::Raw {
        eprintln!("error: {message}");
    }
    // 装载失败无 session：meta_key 传 ""，状态行自然缺席（r5-plan §2.3 打印点 B）。
    print!("{}", render_receipt_output(&receipt, args.output, ""));
    1
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
/// harness 适配器（`--harness` / `YGG_HARNESS` / `harness.adapter` / auto）的判定面。
///
/// 为何要有这个模块：会话库的默认落点从「只认 Pi」变成「按适配器推导」后，三个非 Pi
/// 分支（claude-code / codex / generic）都没有运行期证据——`generic` 尤其**不能失败**
/// （它根本没有会话库，收不出东西不是错误）。这里用**纯函数 + 注入的输入面**钉住每个
/// 分支：不读进程环境、不碰真会话目录，也只建临时目录。
mod harness_adapter_tests {
    use super::{
        archive_sessions_fields_with, cwd_slug, detect_harness, harness_session_dirs,
        resolve_harness_adapter, ArchiveInputs, HarnessProbe, OpFlags, Path, PathBuf,
    };
    use std::fs;

    /// 独占临时根（每个用例一个；先清干净，避免上次残留改变结论）。
    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ygg-harness-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("建临时根");
        dir
    }

    /// 探测输入面的简写（三个环境变量各自在不在）。
    fn probe(pi: bool, claude: bool, codex: bool) -> HarnessProbe {
        HarnessProbe {
            pi_session_id: pi,
            claude_code: claude,
            codex_home: codex,
        }
    }

    /// op 级旗标的直连面（`OpFlags::parse` 读进程 argv，单测不能走）。
    fn flags(pairs: &[(&str, &str)]) -> OpFlags {
        OpFlags {
            values: pairs
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        }
    }

    /// 只有 cwd 不同的归档输入面（会话目录 / HOME / 适配器全部显式注入）。
    fn inputs(harness: &str, root: &Path) -> ArchiveInputs {
        ArchiveInputs {
            harness: Some(harness.to_owned()),
            mod_dir: None,
            sessions_dir: None,
            home: Some(root.join("home")),
            cwd: Some(root.join("work")),
            env_harness: None,
        }
    }

    fn outcome(out: &[(&'static str, String)]) -> (String, String, String) {
        (out[0].1.clone(), out[1].1.clone(), out[2].1.clone())
    }

    /// 显式 `--harness` 压过环境变量；环境变量压过「未表态」。
    #[test]
    fn explicit_adapter_wins_over_the_env_layer() {
        let cwd = temp_root("explicit");
        for adapter in ["pi", "claude-code", "codex", "generic"] {
            assert_eq!(
                resolve_harness_adapter(Some(adapter), Some("codex"), &cwd),
                adapter,
                "--harness {adapter} 必须压过 YGG_HARNESS"
            );
        }
        // 没有显式值时才轮到第二层（这里用确定值验，auto 分支走 detect_harness 的用例）。
        assert_eq!(
            resolve_harness_adapter(None, Some("claude-code"), &cwd),
            "claude-code"
        );
        let _ = fs::remove_dir_all(&cwd);
    }

    /// `auto` 的优先级：Pi > Claude Code > Codex > generic；现场痕迹也能叫起适配器。
    #[test]
    fn auto_detection_prefers_pi_then_claude_then_codex() {
        let root = temp_root("detect");
        let work = root.join("work");
        fs::create_dir_all(&work).expect("建 cwd");
        let home = root.join("home");

        assert_eq!(
            detect_harness(&probe(true, true, true), Some(&home), &work),
            "pi"
        );
        assert_eq!(
            detect_harness(&probe(false, true, true), Some(&home), &work),
            "claude-code"
        );
        assert_eq!(
            detect_harness(&probe(false, false, true), Some(&home), &work),
            "codex"
        );
        assert_eq!(
            detect_harness(&probe(false, false, false), Some(&home), &work),
            "generic"
        );

        // 工作区的 `.pi` / `.claude` 目录同样是证据（不靠环境变量）。
        fs::create_dir_all(work.join(".pi")).expect("建 .pi");
        assert_eq!(
            detect_harness(&probe(false, true, false), Some(&home), &work),
            "pi",
            "`.pi` 现场压过 CLAUDE_CODE"
        );
        let _ = fs::remove_dir_all(work.join(".pi"));
        fs::create_dir_all(work.join(".claude")).expect("建 .claude");
        assert_eq!(
            detect_harness(&probe(false, false, false), Some(&home), &work),
            "claude-code"
        );
        let _ = fs::remove_dir_all(work.join(".claude"));

        // `$HOME/.pi/agent/sessions/--<cwd_slug>--` 存在 → Pi（历史会话就摆在那里）。
        fs::create_dir_all(
            home.join(".pi")
                .join("agent")
                .join("sessions")
                .join(format!("--{}--", cwd_slug(&work))),
        )
        .expect("建 pi 会话库");
        assert_eq!(
            detect_harness(&probe(false, false, false), Some(&home), &work),
            "pi"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// 每个适配器的会话库落点（`generic` 没有会话库）。
    #[test]
    fn session_store_paths_are_per_adapter() {
        let home = Path::new("/home/x");
        assert_eq!(
            harness_session_dirs("pi", home, "a-b"),
            Some((
                PathBuf::from("/home/x/.pi/agent/sessions/--a-b--"),
                PathBuf::from("/home/x/.pi/agent/sessions-trash")
            ))
        );
        assert_eq!(
            harness_session_dirs("claude-code", home, "a-b"),
            Some((
                PathBuf::from("/home/x/.claude/projects/-a-b"),
                PathBuf::from("/home/x/.claude/sessions-trash")
            ))
        );
        assert_eq!(
            harness_session_dirs("codex", home, "a-b"),
            Some((
                PathBuf::from("/home/x/.codex/sessions"),
                PathBuf::from("/home/x/.codex/sessions-trash")
            ))
        );
        assert_eq!(harness_session_dirs("generic", home, "a-b"), None);
        assert_eq!(cwd_slug(Path::new("/Users/x/Repo")), "Users-x-Repo");
    }

    /// `generic` 没有会话库：记一笔 0/0 台账，**不建任何目录**、不报错。
    ///
    /// 连带钉住一个边界：即使 `$HOME` 也缺席（容器里很常见），`generic` 仍然成功——
    /// 它连会话库都不需要，不能让收口卡在一个「推导不出来的路径」上。
    #[test]
    fn generic_adapter_records_a_zero_ledger() {
        let root = temp_root("generic");
        let flags = flags(&[("--parent", "p1"), ("--batch", "b1")]);
        let out = archive_sessions_fields_with(&flags, &inputs("generic", &root))
            .expect("generic 必须成功（收不出东西不是错误）");
        assert_eq!(
            outcome(&out),
            (
                "0".to_owned(),
                "0".to_owned(),
                "generic:no-session-store:b1".to_owned()
            )
        );
        assert!(
            !root.join("home").exists(),
            "generic 不得顺手建出会话库/trash 目录"
        );
        // `$HOME` 缺席也是同一个答案（generic 不靠任何目录）。
        let mut homeless = inputs("generic", &root);
        homeless.home = None;
        let out = archive_sessions_fields_with(&flags, &homeless).expect("generic 不需要 HOME");
        assert_eq!(
            outcome(&out),
            (
                "0".to_owned(),
                "0".to_owned(),
                "generic:no-session-store:b1".to_owned()
            )
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// 适配器目录不存在（首次归档）：记 `<适配器>:empty:<batch>`，不失败。
    #[test]
    fn a_missing_adapter_dir_is_an_empty_ledger() {
        let root = temp_root("empty");
        fs::create_dir_all(root.join("home")).expect("建 HOME");
        for adapter in ["pi", "claude-code", "codex"] {
            let out = archive_sessions_fields_with(
                &flags(&[("--parent", "p1"), ("--batch", "b1")]),
                &inputs(adapter, &root),
            )
            .expect("缺目录必须成功");
            assert_eq!(
                outcome(&out),
                (
                    "0".to_owned(),
                    "0".to_owned(),
                    format!("{adapter}:empty:b1")
                ),
                "{adapter} 的会话库不存在时只能记 0/0"
            );
        }
        let _ = fs::remove_dir_all(&root);
    }

    /// `pi` 分支真的能搬：命中转录进 `<sessions-trash>/<batch>`，不匹配的原样不动。
    #[test]
    fn pi_adapter_sweeps_the_matching_transcript() {
        let root = temp_root("pi-sweep");
        let work = root.join("work");
        let home = root.join("home");
        let sessions = home
            .join(".pi")
            .join("agent")
            .join("sessions")
            .join(format!("--{}--", cwd_slug(&work)));
        fs::create_dir_all(&sessions).expect("建会话库");
        fs::write(sessions.join("s1.jsonl"), "head\nparent-777\ntail\n").expect("写命中档");
        fs::write(sessions.join("s2.jsonl"), "nothing here\n").expect("写其它档");

        let out = archive_sessions_fields_with(
            &flags(&[("--parent", "parent-777"), ("--batch", "b9")]),
            &inputs("pi", &root),
        )
        .expect("扫描成功");
        let (moved, remaining, dest) = outcome(&out);
        assert_eq!((moved.as_str(), remaining.as_str()), ("1", "0"));
        let trash = home
            .join(".pi")
            .join("agent")
            .join("sessions-trash")
            .join("b9");
        assert_eq!(
            dest,
            fs::canonicalize(&trash)
                .expect("trash 必已建")
                .display()
                .to_string()
        );
        assert!(trash.join("s1.jsonl").is_file(), "命中的档必须搬进 trash");
        assert!(!sessions.join("s1.jsonl").exists());
        assert!(sessions.join("s2.jsonl").is_file(), "不匹配的档不得被动");
        let _ = fs::remove_dir_all(&root);
    }

    /// 显式 `--sessions-dir` 永远照旧（即使适配器是 `generic`，也不走台账捷径）。
    #[test]
    fn an_explicit_sessions_dir_ignores_the_adapter() {
        let root = temp_root("explicit-dir");
        let dir = root.join("sessions");
        fs::create_dir_all(&dir).expect("建显式目录");
        fs::write(dir.join("s1.jsonl"), "parent-777\n").expect("写命中档");
        let mut inputs = inputs("generic", &root);
        inputs.sessions_dir = Some(dir.clone());
        inputs.home = None;
        let out = archive_sessions_fields_with(
            &flags(&[("--parent", "parent-777"), ("--batch", "b1")]),
            &inputs,
        )
        .expect("显式目录不受适配器影响");
        let (moved, remaining, _dest) = outcome(&out);
        assert_eq!((moved.as_str(), remaining.as_str()), ("1", "0"));
        // 显式目录的 trash 根仍按老口径推导：`<dir 的父目录>/trash/<batch>`。
        assert!(root.join("trash").join("b1").join("s1.jsonl").is_file());
        let _ = fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
/// 退役专名拒绝（别名退役）的判定面：旧名 `pi-agent-seed` 不再翻译——精确形状缺席时
/// `resolve_mod_dir` 给点名规范路径的可读拒绝；其余缺席维持通用缺路径错误。
///
/// 为何要有这个模块：更名 agent-seed 的双向别名已退役（2026-11-12），历史 run.json 里的
/// `"mod": ".ygg/mods/pi-agent-seed"` 不迁移——宿主拒绝并指引开新 run-id，磁盘上的
/// `run.json` 一个字节都不动。本模块不装内核、不派发、不读真 Mod：临时目录里建空目录当 Mod。
mod legacy_seed_refusal_tests {
    use super::{resolve_mod_dir, Args, Drive, PathBuf};
    use std::fs;

    /// 独占临时根（每个用例一个；先清干净，避免上次残留改变 `exists()` 结论）。
    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ygg-modalias-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("建临时根");
        dir
    }

    /// 只在 `mod_dir` 上有差异的宿主参数（本模块不派发，其余字段钉成「续局单发」）。
    fn args(mod_dir: Option<PathBuf>) -> Args {
        Args {
            packages: None,
            mod_dir,
            open_style: Some(crate::OpenStyle::Open),
            run_root: PathBuf::from("ygg-runs-should-not-be-touched/r1"),
            runs_parent: PathBuf::from("ygg-runs-should-not-be-touched"),
            force: false,
            at: None,
            settle: false,
            watch: false,
            event: None,
            payload: None,
            output: crate::OutputFormat::Text,
            drive: Drive::None,
            inbox: None,
            store: Some(ygg_store::Backend::Json),
            wait_tools: false,
            tool_timeout_ms: None,
            run_tool_worker: None,
            artifact_put: None,
            artifact_get: None,
            builtin_tool: None,
            daemon: false,
            harness: None,
            actor: None,
            actor_session: None,
            serve: false,
            port: None,
            panel_dir: None,
            poll_ms: None,
        }
    }

    /// ① 历史 `run.json` 记旧名（精确形状）→ 专名拒绝：点名规范路径 + 指引开新 run-id。
    #[test]
    fn legacy_run_json_gets_the_named_refusal() {
        let root = temp_root("refuse");
        let legacy = root.join("mods/pi-agent-seed");
        assert!(!legacy.exists(), "前提：旧名目录不存在（退役形态）");
        let run_dir = root.join("runs/r1");
        fs::create_dir_all(&run_dir).expect("建 run 目录");
        fs::write(
            run_dir.join("run.json"),
            format!(r#"{{"mod":"{}"}}"#, legacy.display()),
        )
        .expect("写 run.json");
        let err = resolve_mod_dir(&args(None), &run_dir).expect_err("退役旧名必须拒绝");
        assert!(err.contains("pi-agent-seed"), "点名旧名：{err}");
        assert!(err.contains("agent-seed"), "点名同父规范路径：{err}");
        assert!(err.contains("run-id"), "指引开新局：{err}");
        assert!(err.contains("2026-11-12"), "宣布退役日期：{err}");
        let _ = fs::remove_dir_all(&root);
    }

    /// ② 旧相对形状（`.ygg/mods/pi-agent-seed`）同样专名拒绝（含退役日期与规范名）。
    #[test]
    fn legacy_relative_shape_gets_the_named_refusal() {
        let root = temp_root("rel");
        let run_dir = root.join("runs/r1");
        fs::create_dir_all(&run_dir).expect("建 run 目录");
        let err = resolve_mod_dir(
            &args(Some(PathBuf::from(".ygg/mods/pi-agent-seed"))),
            &run_dir,
        )
        .expect_err("旧相对形状必须拒绝（不再翻译）");
        assert!(err.contains("2026-11-12"), "宣布退役日期：{err}");
        assert!(err.contains("agent-seed"), "点名规范路径：{err}");
        let _ = fs::remove_dir_all(&root);
    }

    /// ③ 只认精确形状：前缀撞名与裸值不走专名拒绝，维持通用缺席语义（原样返回）。
    #[test]
    fn only_the_exact_shape_gets_the_named_refusal() {
        let root = temp_root("shape");
        let run_dir = root.join("runs/r1");
        fs::create_dir_all(&run_dir).expect("建 run 目录");
        for untouched in [
            "pi-agent-seed",              // 无 `mods/` 父段（裸值）
            "/tmp/pi-agent-seed",         // 父段不叫 mods（临时探针目录常态）
            ".ygg/mods/pi-agent-seed-v2", // 前缀撞名
            ".ygg/mods/pi-agent-seeds",
        ] {
            let resolved = resolve_mod_dir(&args(Some(PathBuf::from(untouched))), &run_dir)
                .expect("非精确形状不走专名拒绝（通用错误留在装载期）");
            assert_eq!(
                resolved,
                PathBuf::from(untouched),
                "{untouched} 必须原样返回"
            );
        }
        let _ = fs::remove_dir_all(&root);
    }

    /// ④ 规范名缺席时即使盘上只剩旧名目录也不收养：原样返回（缺席语义），绝不翻译。
    #[test]
    fn missing_canonical_never_adopts_the_legacy_dir() {
        let root = temp_root("adopt");
        let legacy = root.join("mods/pi-agent-seed");
        fs::create_dir_all(&legacy).expect("建旧名目录");
        fs::write(legacy.join("pack.json"), "{}").expect("放一个标记文件");
        let requested = root.join("mods/agent-seed");
        assert!(!requested.exists(), "前提：规范名目录不存在");
        let resolved = resolve_mod_dir(&args(Some(requested.clone())), &root.join("runs/r1"))
            .expect("规范名缺席 = 通用缺席语义（原样返回，装载期报错）");
        assert_eq!(resolved, requested, "绝不收养旧名目录");
        let _ = fs::remove_dir_all(&root);
    }

    /// ⑤ 盘上旧目录不被改写：显式请求**存在的**旧名目录 → 原样使用，不重命名不翻译。
    #[test]
    fn an_existing_legacy_dir_is_never_rewritten() {
        let root = temp_root("keep-old");
        let legacy = root.join("mods/pi-agent-seed");
        fs::create_dir_all(&legacy).expect("建旧名目录");
        fs::write(legacy.join("pack.json"), "{}").expect("放一个标记文件");
        let resolved = resolve_mod_dir(&args(Some(legacy.clone())), &root.join("runs/r1"))
            .expect("存在的请求路径原样使用（退役禁令只针对缺席翻译）");
        assert_eq!(resolved, legacy);
        assert!(legacy.is_dir(), "旧目录保持原位（不做磁盘改写）");
        let _ = fs::remove_dir_all(&root);
    }

    /// ⑥ `--mod` 旗标与 run.json 走同一条解析：指向旧名（缺席）时同一份专名拒绝文案。
    #[test]
    fn the_mod_flag_takes_the_same_refusal() {
        let root = temp_root("flag");
        let run_dir = root.join("runs/r1");
        fs::create_dir_all(&run_dir).expect("建 run 目录");
        let via_flag = resolve_mod_dir(&args(Some(root.join("mods/pi-agent-seed"))), &run_dir)
            .expect_err("旗标旧名必须拒绝");
        let meta_run = root.join("runs/r2");
        fs::create_dir_all(&meta_run).expect("建 run 目录");
        fs::write(
            meta_run.join("run.json"),
            format!(
                r#"{{"mod":"{}"}}"#,
                root.join("mods/pi-agent-seed").display()
            ),
        )
        .expect("写 run.json");
        let via_meta = resolve_mod_dir(&args(None), &meta_run).expect_err("run.json 旧名必须拒绝");
        assert_eq!(via_flag, via_meta, "两条入口同一份拒绝文案");
        let _ = fs::remove_dir_all(&root);
    }

    /// ⑦ 规范名缺席的通用语义保留：非退役形状原样返回；缺 --mod 且 run.json 无记录
    /// 仍是既有可读错误（fail-closed 不变）。
    #[test]
    fn canonical_missing_keeps_the_generic_error() {
        let root = temp_root("generic");
        let run_dir = root.join("runs/r1");
        fs::create_dir_all(&run_dir).expect("建 run 目录");
        let requested = root.join("mods/agent-seed");
        let resolved = resolve_mod_dir(&args(Some(requested.clone())), &run_dir)
            .expect("规范名缺席 = 通用缺席语义（原样返回）");
        assert_eq!(resolved, requested);
        let empty = root.join("runs/r2");
        fs::create_dir_all(&empty).expect("建空 run 目录");
        let message = resolve_mod_dir(&args(None), &empty).expect_err("缺 Mod 目录必须报错");
        assert!(message.contains("--mod"), "{message}");
        let _ = fs::remove_dir_all(&root);
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
fn dispatch_with_watch(session: &mut Session, args: &Args, name: &str, payload: &str) -> Json {
    if args.watch {
        session.maybe_reload();
    }
    session.dispatch_event(name, payload)
}

/// 编号选择行的唯一出处（交互 [`print_choices`] 与单发文本渲染共享，防漂移）。
///
/// `pendingChoice` 是对象 → `? <prompt>` + `  <i>. <option>`（1-based，与交互
/// 同一编号函数）；`null` / 缺失 → 空 vec（调用方决定占位文案）。
fn format_choice_lines(receipt: &Json) -> Vec<String> {
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
fn render_receipt_text(receipt: &Json, meta_key: &str) -> Vec<String> {
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
fn render_receipt_output(receipt: &Json, format: OutputFormat, meta_key: &str) -> String {
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
fn print_choices(receipt: &Json) {
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
        "autosave": session.store.locator(),
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
    settle_report(args, auto, true).0
}

/// 结算本体（**唯一**实现：独立 `--settle` 与守护模式的 `settle` 动作共用）。
///
/// 返回 `(退出码, 报告)`：报告为 `None` = 结算在写出报告之前就失败（原因已进
/// stderr）；`Some` = 报告已落 `settlement.json`，直接交给调用方复用。
///
/// `human=false` 时**不**打印人读摘要——守护模式的 stdout 是 JSON-RPC 协议面，
/// 人读文本会撕裂协议流。
fn settle_report(args: &Args, auto: bool, human: bool) -> (i32, Option<Json>) {
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

// ============================================================================
// §P0 持久化原子性单测（plan-r5 §4.1；U1–U2 随 S1，U3 随 S2，U4 随 S3）
// ============================================================================
#[cfg(test)]
mod persistence_atomicity_tests {
    use super::{acquire_run_lock, atomic_write, Cell};
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

    /// U4：seq 分配口径的三种边界 —— 末行无换行 / 空文件 / 只有空行。
    ///
    /// 函数本体已下沉到 `ygg-store`（`JsonFileStore::next_seq`），断言不变：
    /// seq 的唯一事实源是**非空行数**，末行没有换行也要计入。
    #[test]
    fn u4_event_count_edge_cases() {
        use ygg_store::{JsonFileStore, RunStore};

        let dir = case_dir("u4");
        let path = dir.join(ygg_store::EVENTS_FILE);
        let store = JsonFileStore::open(&dir).expect("绑到用例目录");

        // 空文件。
        std::fs::write(&path, "").expect("写空文件");
        assert_eq!(store.next_seq().expect("seq"), 0, "空文件 = 0 条");

        // 只有空行 / 空白行。
        std::fs::write(&path, "\n\n   \n\t\n").expect("写空白行");
        assert_eq!(store.next_seq().expect("seq"), 0, "空行与空白行不计入");

        // 末行无换行：仍按非空行计数。
        std::fs::write(&path, "{\"seq\":0}\n{\"seq\":1}").expect("写无尾换行");
        assert_eq!(store.next_seq().expect("seq"), 2, "末行无换行也要计入");

        // 正常形态（对照）。
        std::fs::write(&path, "{\"seq\":0}\n{\"seq\":1}\n").expect("写正常形态");
        assert_eq!(store.next_seq().expect("seq"), 2);

        // 文件不存在（开局前）。
        std::fs::remove_file(&path).expect("删事件日志");
        assert_eq!(store.next_seq().expect("seq"), 0, "读不到文件 = 0 条");

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
// §工具执行器（r1）：契约装载 / 场景 1 就地回投 / 场景 2 脱体 worker + inbox
//
// 全部用例都用**合成 Mod**（本节自己的 `write_tool_mod`）：真实 `.ygg/mods/**`
// 的 `tools` / `allowedTools` 声明由后续迭代落地，本节的断言不依赖它。
// ============================================================================
#[cfg(test)]
mod tool_runner_tests {
    use super::*;

    /// 用例专属临时目录（互不共享；先清空再建）。
    fn case_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ygg-tools-r1-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    /// 最小合成 Mod：两条只读 cli 工具 + `tool_result` inbox 路由 + 三个触发事件。
    ///
    /// `probe.echo` 走场景 1（`inlineWaitMs > 0`），`probe.detach` 恒走场景 2
    /// （`inlineWaitMs = 0`，参数里的 `$ENV{YGG_TOOL_TEST}` 用于验 I4）。
    fn write_tool_mod(dir: &Path, allowed: &[&str]) {
        fs::create_dir_all(dir.join("data")).expect("建 data 目录");
        let pack = json!({
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
                "tool_result": {
                    "action": "tool.result",
                    "fields": ["call_id", "tool_id", "status", "code", "output", "external_id"],
                    "write": {"flag": "tool-result", "shape": "payload"}
                }
            },
            "tools": {
                "probe.echo": {
                    "kind": "cli",
                    "program": "printf",
                    "args": ["%s", "{{text}}"],
                    "timeoutMs": 5_000u64,
                    "inlineWaitMs": 2_000u64,
                    "maxOutputBytes": 1_024u64,
                    "readOnly": true,
                    "resultEvent": "tool.result"
                },
                "probe.detach": {
                    "kind": "cli",
                    "program": "printf",
                    "args": ["%s", "$ENV{YGG_TOOL_TEST}"],
                    "timeoutMs": 5_000u64,
                    "inlineWaitMs": 0u64,
                    "maxOutputBytes": 1_024u64,
                    "readOnly": true,
                    "resultEvent": "tool.result"
                }
            }
        });
        write_json_pretty(&dir.join(PACK_FILE), &pack).expect("写 pack.json");
        fs::write(
            dir.join(MANIFEST_FILE),
            r#"{"schemaVersion": 1, "entries": [], "code": []}"#,
        )
        .expect("写 manifest.json");
        fs::write(
            dir.join("mod.lua"),
            r#"ygg.on("event.plain", function()
  ygg.narrate("普通一步")
end)
ygg.on("event.start", function()
  ygg.meta("tool_calls", { { id = "c1", tool = "probe.echo", args = { text = "hello" } } })
  ygg.narrate("已请求工具")
end)
ygg.on("event.detach", function()
  ygg.meta("tool_calls", { { id = "d1", tool = "probe.detach", args = {} } })
  ygg.narrate("已请求脱体工具")
end)
ygg.on("event.unknown", function()
  ygg.meta("tool_calls", { { id = "u1", tool = "probe.missing", args = {} } })
  ygg.narrate("已请求未声明工具")
end)
ygg.on("tool.result", function(payload)
  ygg.kv.set("last_tool_payload", tostring(payload))
  ygg.narrate("工具结果：" .. tostring(payload))
end)
ygg.on("agent.summary", function()
  ygg.meta("summary", { open = true })
  ygg.narrate("摘要就绪")
end)
"#,
        )
        .expect("写 mod.lua");
        let config = json!({
            "drive": {"maxSteps": 50, "stallLimit": 5, "stepsPerRequirement": 20},
            "settlement": {
                "requireEvents": true,
                "requireNoFailedSteps": true,
                "requireEnding": true,
                "requireInboxConsumed": true
            },
            "allowedTools": allowed,
        });
        write_json_pretty(&dir.join("data/config.json"), &config).expect("写 config.json");
    }

    /// 单发 / 只观察共用的宿主参数（`store` 钉住 JSON：本节断言都在文件面）。
    fn tool_args(mod_dir: &Path, runs_root: &Path, run_id: &str) -> Args {
        Args {
            packages: None,
            mod_dir: Some(mod_dir.to_path_buf()),
            open_style: Some(crate::OpenStyle::New),
            run_root: runs_root.join(run_id),
            runs_parent: runs_root.to_path_buf(),
            force: false,
            at: None,
            settle: false,
            watch: false,
            event: None,
            payload: None,
            output: crate::OutputFormat::Text,
            drive: Drive::None,
            inbox: None,
            store: Some(ygg_store::Backend::Json),
            wait_tools: false,
            tool_timeout_ms: None,
            run_tool_worker: None,
            artifact_put: None,
            artifact_get: None,
            builtin_tool: None,
            daemon: false,
            harness: None,
            actor: None,
            actor_session: None,
            serve: false,
            port: None,
            panel_dir: None,
            poll_ms: None,
        }
    }

    /// 一步派发 + 回执（断言用）。
    #[track_caller]
    fn dispatch(session: &mut Session, event: &str) -> Json {
        let receipt = session.dispatch_event(event, "0");
        assert_eq!(
            receipt["ok"],
            Json::Bool(true),
            "{event} 必须成功：{receipt}"
        );
        receipt
    }

    /// 取一条声明 + 一条请求 → 造出待登记的调用（不 spawn，纯内存）。
    fn pending_call(
        session: &Session,
        tool: &str,
        call_id: &str,
        args: &[(&str, &str)],
    ) -> (ToolSpec, PendingCall) {
        let spec = session.tools.get(tool).cloned().expect("声明必须存在");
        let request = ToolRequest {
            id: call_id.to_owned(),
            tool: tool.to_owned(),
            args: args
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        };
        let call = PendingCall::from_spec(&spec, &request, now_ms() + 5_000, 1);
        (spec, call)
    }

    // ---- 解析面（§6.1） ----

    #[test]
    fn parses_meta_tool_calls() {
        let meta = json!({"tool_calls": [{"tool": "t", "args": {"a": "1"}}]});
        let (requests, notices) = parse_tool_calls(&meta, "run-0");
        assert!(notices.is_empty(), "{notices:?}");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].tool, "t");
        assert_eq!(requests[0].args, vec![("a".to_owned(), "1".to_owned())]);
        // 缺 id → 缺省 `<prefix>-<index>`
        assert_eq!(requests[0].id, "run-0-0");
        // 无该键 / 非数组 / 空数组 → 零请求
        assert!(parse_tool_calls(&json!({}), "run-0").0.is_empty());
        assert!(parse_tool_calls(&json!({"tool_calls": []}), "run-0")
            .0
            .is_empty());
        let (requests, notices) = parse_tool_calls(&json!({"tool_calls": {}}), "run-0");
        assert!(requests.is_empty());
        assert_eq!(notices.len(), 1);
    }

    #[test]
    fn empty_args_array_is_empty_map() {
        // Lua 空表 `{}` 会序列化成 `[]`（host_api 已文档化的怪癖）：等价于无参数。
        let meta = json!({"tool_calls": [{"id": "c1", "tool": "probe.echo", "args": []}]});
        let (requests, notices) = parse_tool_calls(&meta, "run-0");
        assert!(notices.is_empty(), "{notices:?}");
        assert_eq!(requests.len(), 1);
        assert!(requests[0].args.is_empty());
        assert_eq!(requests[0].id, "c1");
    }

    #[test]
    fn rejects_args_with_pipe() {
        let meta = json!({"tool_calls": [{"tool": "t", "args": {"a": "x|y"}}]});
        let (requests, notices) = parse_tool_calls(&meta, "run-0");
        assert!(requests.is_empty(), "含 `|` 的条目必须被跳过");
        assert_eq!(notices.len(), 1);
        assert!(notices[0].contains('|'), "{notices:?}");
        // 嵌套值 / 非对象 args 也各自只丢该条
        for bad in [
            json!({"tool": "t", "args": {"a": ["x"]}}),
            json!({"tool": "t", "args": 1}),
        ] {
            let (requests, notices) = parse_tool_calls(&json!({"tool_calls": [bad]}), "run-0");
            assert!(requests.is_empty());
            assert_eq!(notices.len(), 1);
        }
        // 缺 tool 的条目同样只丢自己
        let (requests, notices) = parse_tool_calls(&json!({"tool_calls": [{"args": {}}]}), "run-0");
        assert!(requests.is_empty());
        assert_eq!(notices.len(), 1);
        // id 非法 = 拒绝（不静默换成 fallback，因为它同时是结果文件名）
        let (requests, notices) = parse_tool_calls(
            &json!({"tool_calls": [{"id": "../x", "tool": "t"}]}),
            "run-0",
        );
        assert!(requests.is_empty());
        assert_eq!(notices.len(), 1);
    }

    // ---- 替换面（§6.2 / §6.3） ----

    #[test]
    fn substitutes_placeholders() {
        let args = vec![("repo".to_owned(), ".".to_owned())];
        let env = EnvTable {
            process: Vec::new(),
            env_local: vec![("TOKEN".to_owned(), "secret".to_owned())],
            env: Vec::new(),
        };
        assert_eq!(
            substitute("{{repo}}/src", &args, &env).expect("替换成功"),
            "./src"
        );
        assert_eq!(
            substitute("Bearer $ENV{TOKEN}", &args, &env).expect("替换成功"),
            "Bearer secret"
        );
        assert_eq!(
            substitute("{{ repo }}", &args, &env).expect("键两侧空白容忍"),
            "."
        );
        assert_eq!(
            substitute("no-templates", &args, &env).expect("原样"),
            "no-templates"
        );
    }

    #[test]
    fn missing_placeholder_is_per_tool_error() {
        let args: Vec<(String, String)> = Vec::new();
        let env = EnvTable {
            process: Vec::new(),
            env_local: Vec::new(),
            env: Vec::new(),
        };
        let error = substitute("--repo={{repo}}", &args, &env).expect_err("缺参数必须报错");
        assert!(error.contains("repo"), "{error}");
        assert!(error.contains("未提供"), "{error}");
    }

    #[test]
    fn env_resolution_precedence() {
        let dir = case_dir("env");
        fs::write(dir.join(".env"), "YGG_TOOL_ONLY=y\nPATH=from-env\n").expect("写 .env");
        fs::write(dir.join(".env.local"), "YGG_TOOL_ONLY=x\nPATH=from-local\n")
            .expect("写 .env.local");
        let env = load_env_table(Some(&dir));
        // 进程环境 > .env.local > .env
        assert_eq!(
            env.lookup("PATH"),
            std::env::var("PATH").ok().as_deref(),
            "进程环境必须压过两个文件"
        );
        assert_eq!(
            env.lookup("YGG_TOOL_ONLY"),
            Some("x"),
            ".env.local 压过 .env"
        );
        // 不存在的目录 = 空表（不报错）
        let empty = load_env_table(Some(&dir.join("missing")));
        assert_eq!(empty.lookup("YGG_TOOL_ONLY"), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_missing_reports_key_only() {
        let env = EnvTable {
            process: Vec::new(),
            env_local: Vec::new(),
            env: Vec::new(),
        };
        let error = resolve_env("YGG_TOOL_NOPE", &env).expect_err("未设置必须报错");
        assert!(error.contains("YGG_TOOL_NOPE"), "{error}");
        assert!(
            !error.contains('='),
            "错误文案不得出现值形态（只有变量名）：{error}"
        );
    }

    #[test]
    fn parse_env_file_lexes_the_usual_forms() {
        let table = parse_env_file(
            "# comment\nexport A=1\nB='two'\nC=\"three\"\nD=\nnot a pair\n=bad\nE=unquoted value\n",
        );
        assert_eq!(
            table,
            vec![
                ("A".to_owned(), "1".to_owned()),
                ("B".to_owned(), "two".to_owned()),
                ("C".to_owned(), "three".to_owned()),
                ("D".to_owned(), String::new()),
                ("E".to_owned(), "unquoted value".to_owned()),
            ]
        );
    }

    // ---- 结果面（§7.1） ----

    #[test]
    fn sanitize_tool_output_wire_safe() {
        let cleaned = sanitize_tool_output("a|b\nc\0d\r\ne", 1_024);
        assert!(!cleaned.contains('|'), "{cleaned}");
        assert!(!cleaned.contains('\n'), "{cleaned}");
        assert!(!cleaned.contains('\r'), "{cleaned}");
        assert!(!cleaned.contains('\0'), "{cleaned}");
        assert_eq!(cleaned.split('|').count(), 1, "输出不得引入段分隔符");
        assert!(cleaned.contains('｜'), "分隔符换成全角形态（保留可读性）");
    }

    #[test]
    fn truncates_on_char_boundary() {
        // 「中文」每个字 3 字节：把上限切在第 4 字节（字中间）→ 必须回退到边界。
        let text = "中文中文";
        let (truncated, hit) = truncate_bytes(text, 4);
        assert!(hit);
        assert_eq!(truncated, "中", "必须回退到字符边界");
        assert!(!truncated.contains('\u{FFFD}'));
        let cleaned = sanitize_tool_output(&"中".repeat(10), 8);
        assert!(!cleaned.contains('\u{FFFD}'), "{cleaned}");
        assert!(cleaned.ends_with("…[truncated]"), "{cleaned}");
        // 不超限时原样返回（不加标记）
        assert_eq!(sanitize_tool_output("short", 1_024), "short");
    }

    #[test]
    fn result_event_payload_is_json_string() {
        // 场景 1 的派发载荷必须与 `drain_inbox_locked` 同形：JSON **字符串**。
        let payload = json!("c1|probe.echo|ok|0|hello|inbox-1").to_string();
        let parsed: Json = serde_json::from_str(&payload).expect("载荷必须能解析");
        assert!(matches!(parsed, Json::String(_)), "{payload}");
    }

    #[test]
    fn wire_payload_has_six_segments() {
        let dir = case_dir("wire");
        let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
        write_tool_mod(&mod_dir, &["probe.echo", "probe.detach"]);
        let session = Session::open(&tool_args(&mod_dir, &runs_root, "wire")).expect("开局");
        let (_, call) = pending_call(&session, "probe.echo", "c1", &[("text", "hello")]);
        let outcome = ToolOutcome {
            status: "ok",
            code: "0".to_owned(),
            output: "hello".to_owned(),
        };
        let payload = wire_payload(&call, &outcome, "inbox-1");
        let segments: Vec<&str> = payload.split('|').collect();
        assert_eq!(segments.len(), 6, "{payload}");
        assert_eq!(segments[0], "c1");
        assert_eq!(segments[1], "probe.echo");
        assert_eq!(segments[2], "ok");
        assert_eq!(segments[3], "0");
        assert_eq!(segments[4], "hello");
        assert_eq!(segments[5], "inbox-1");
        let _ = fs::remove_dir_all(&dir);
    }

    // ---- 在飞事实面（§4） ----

    #[test]
    fn pending_snapshot_never_stores_env_values() {
        let dir = case_dir("pending-redaction");
        let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
        write_tool_mod(&mod_dir, &["probe.echo", "probe.detach"]);
        fs::write(mod_dir.join(".env.local"), "YGG_TOOL_TEST=tok-123\n").expect("写 .env.local");
        let session = Session::open(&tool_args(&mod_dir, &runs_root, "redaction")).expect("开局");
        let (_, call) = pending_call(&session, "probe.detach", "d1", &[]);
        pending_tools_insert(&session.run_dir, &call).expect("登记");
        let snapshot =
            fs::read_to_string(pending_tools_path(&session.run_dir)).expect("读 pending");
        assert!(
            snapshot.contains("$ENV{YGG_TOOL_TEST}"),
            "模板必须原样保留：{snapshot}"
        );
        assert!(
            !snapshot.contains("tok-123"),
            "I4：pending 快照绝不得出现解析后的值：{snapshot}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn pending_tools_update_is_atomic() {
        let dir = case_dir("atomic");
        let run_dir = dir.join("run");
        fs::create_dir_all(&run_dir).expect("建 run 目录");
        std::thread::scope(|scope| {
            for worker in 0..2 {
                let run_dir = run_dir.clone();
                scope.spawn(move || {
                    for index in 0..20 {
                        pending_tools_update(&run_dir, |record| {
                            if let Some(calls) = record["calls"].as_object_mut() {
                                calls.insert(
                                    format!("c{worker}-{index}"),
                                    json!({"tool": "probe.echo"}),
                                );
                            }
                        })
                        .expect("并发登记必须成功");
                    }
                });
            }
        });
        assert_eq!(pending_calls(&run_dir).len(), 40, "并发写不得丢更新");
        // 摘除是幂等的
        pending_tools_remove(&run_dir, "c0-0").expect("摘除");
        pending_tools_remove(&run_dir, "c0-0").expect("重复摘除不报错");
        assert_eq!(pending_calls(&run_dir).len(), 39);
        let _ = fs::remove_dir_all(&dir);
    }

    // ---- 场景 1：同进程就地回投 ----

    #[test]
    fn scenario1_delivers_in_process_and_clears_pending() {
        let dir = case_dir("scenario1");
        let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
        write_tool_mod(&mod_dir, &["probe.echo", "probe.detach"]);
        let mut session = Session::open(&tool_args(&mod_dir, &runs_root, "s1")).expect("开局");
        let (spec, call) = pending_call(&session, "probe.echo", "c1", &[("text", "hello")]);
        pending_tools_insert(&session.run_dir, &call).expect("登记");
        // 模拟 worker 已把结果写好（不 spawn：本用例只验父进程的收敛面）。
        write_tool_result(
            &session.run_dir,
            &ToolResult {
                call_id: "c1".to_owned(),
                tool: "probe.echo".to_owned(),
                status: "ok".to_owned(),
                code: "0".to_owned(),
                output: "hello".to_owned(),
                payload: "c1|probe.echo|ok|0|hello|inbox-test".to_owned(),
                external_id: "inbox-test".to_owned(),
            },
        )
        .expect("写结果");
        let run = session
            .settle_tool_call(&spec, &call)
            .expect("场景 1 必须就地回投");
        assert_eq!(run["status"], "ok");
        assert_eq!(run["tool"], "probe.echo");
        assert_eq!(run["ok"], Json::Bool(true));
        assert!(
            run["seq"].as_u64().is_some(),
            "回投事件必须有自己的 seq：{run}"
        );
        // pending 已摘除、结果文件已消费
        assert!(pending_calls(&session.run_dir).is_empty());
        assert!(!tool_result_path(&session.run_dir, "c1").exists());
        // 事件日志里出现 tool.result 一步（结果以普通事件回投）
        let events =
            fs::read_to_string(session.run_dir.join(ygg_store::EVENTS_FILE)).expect("读事件日志");
        assert!(events.contains("\"tool.result\""), "{events}");
        // 认领文件仍在：它同时是「已交付」标记（防止第二个进程重复派发）
        assert!(tool_claim_path(&session.run_dir, "c1").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scenario1_zero_inline_wait_never_polls() {
        let dir = case_dir("scenario1-zero");
        let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
        write_tool_mod(&mod_dir, &["probe.echo", "probe.detach"]);
        let mut session = Session::open(&tool_args(&mod_dir, &runs_root, "s1z")).expect("开局");
        let (spec, call) = pending_call(&session, "probe.detach", "d1", &[]);
        pending_tools_insert(&session.run_dir, &call).expect("登记");
        // 结果文件就在那里，但 inlineWaitMs = 0 → 父进程不轮询、不认领（场景 2）
        write_tool_result(
            &session.run_dir,
            &ToolResult {
                call_id: "d1".to_owned(),
                tool: "probe.detach".to_owned(),
                status: "ok".to_owned(),
                code: "0".to_owned(),
                output: "detached".to_owned(),
                payload: "d1|probe.detach|ok|0|detached|inbox-d".to_owned(),
                external_id: "inbox-d".to_owned(),
            },
        )
        .expect("写结果");
        assert!(session.settle_tool_call(&spec, &call).is_none());
        assert!(
            !tool_claim_path(&session.run_dir, "d1").exists(),
            "父进程不得认领"
        );
        assert_eq!(pending_calls(&session.run_dir).len(), 1, "条目留给 worker");
        let _ = fs::remove_dir_all(&dir);
    }

    // ---- 场景 2：脱体 worker + inbox 回写 + `--wait-tools` ----

    #[test]
    fn scenario2_worker_delivers_to_inbox_and_wait_tools_drains() {
        let dir = case_dir("scenario2");
        let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
        write_tool_mod(&mod_dir, &["probe.echo", "probe.detach"]);
        fs::write(mod_dir.join(".env.local"), "YGG_TOOL_TEST=tok-123\n").expect("写 .env.local");
        let args = tool_args(&mod_dir, &runs_root, "s2");
        let mut session = Session::open(&args).expect("开局");
        dispatch(&mut session, "event.plain");
        let (_, call) = pending_call(&session, "probe.detach", "d1", &[]);
        pending_tools_insert(&session.run_dir, &call).expect("登记");

        // worker 执行体（真子进程：printf，只读无害）
        assert_eq!(run_tool_worker(&args, "d1"), 0, "worker 必须成功交付");
        assert!(
            pending_calls(&session.run_dir).is_empty(),
            "交付成功后摘除 pending"
        );
        assert!(tool_claim_path(&session.run_dir, "d1").exists());

        // --wait-tools：消费 inbox（结果行走 Mod 的 tool_result 路由）
        let wait_args = Args {
            wait_tools: true,
            ..args.clone()
        };
        assert_eq!(run_wait_tools(&wait_args), 0);
        let events =
            fs::read_to_string(session.run_dir.join(ygg_store::EVENTS_FILE)).expect("读事件日志");
        assert!(events.contains("\"tool.result\""), "{events}");
        assert!(
            events.contains("tok-123"),
            "工具输出必须进回投载荷：{events}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn wait_tools_never_resets_the_run() {
        let dir = case_dir("wait-tools-no-reset");
        let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
        write_tool_mod(&mod_dir, &["probe.echo", "probe.detach"]);
        let args = tool_args(&mod_dir, &runs_root, "wnr");
        let mut session = Session::open(&args).expect("开局");
        dispatch(&mut session, "event.plain");
        dispatch(&mut session, "event.plain");
        let run_dir = session.run_dir.clone();
        let events_before =
            fs::read_to_string(run_dir.join(ygg_store::EVENTS_FILE)).expect("读事件日志");
        let autosave_before = fs::read(run_dir.join(AUTOSAVE_FILE)).expect("读存档");
        let wait_args = Args {
            wait_tools: true,
            tool_timeout_ms: Some(500),
            ..args.clone()
        };
        assert_eq!(run_wait_tools(&wait_args), 0);
        let events_after =
            fs::read_to_string(run_dir.join(ygg_store::EVENTS_FILE)).expect("读事件日志");
        let autosave_after = fs::read(run_dir.join(AUTOSAVE_FILE)).expect("读存档");
        assert_eq!(
            events_before, events_after,
            "--wait-tools 不得新增/重置事件行"
        );
        assert_eq!(autosave_before, autosave_after, "--wait-tools 不得重置存档");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn wait_tools_reports_pending_after_timeout() {
        let dir = case_dir("wait-tools-pending");
        let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
        write_tool_mod(&mod_dir, &["probe.echo", "probe.detach"]);
        let mut session = Session::open(&tool_args(&mod_dir, &runs_root, "wtp")).expect("开局");
        dispatch(&mut session, "event.plain");
        let (_, call) = pending_call(&session, "probe.detach", "stuck", &[]);
        pending_tools_insert(&session.run_dir, &call).expect("登记");
        let wait_args = Args {
            wait_tools: true,
            tool_timeout_ms: Some(150),
            ..tool_args(&mod_dir, &runs_root, "wtp")
        };
        assert_eq!(run_wait_tools(&wait_args), 1, "残留 pending 必须非零退出");
        assert_eq!(
            pending_calls(&session.run_dir).len(),
            1,
            "残留条目不被自动回收"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    // ---- 纵深防御 / 生命周期 ----

    #[test]
    fn unknown_tool_is_skipped_not_fatal() {
        let dir = case_dir("unknown-tool");
        let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
        write_tool_mod(&mod_dir, &["probe.echo", "probe.detach"]);
        let mut session = Session::open(&tool_args(&mod_dir, &runs_root, "unk")).expect("开局");
        let receipt = dispatch(&mut session, "event.unknown");
        assert!(
            receipt.get("tools").is_none(),
            "未声明工具不得产生 runs：{receipt}"
        );
        let warnings = receipt["warnings"].as_array().cloned().unwrap_or_default();
        let text = warnings
            .iter()
            .filter_map(Json::as_str)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("未在 pack.json"), "{text}");
        assert!(pending_calls(&session.run_dir).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn max_tool_rounds_is_bounded() {
        let dir = case_dir("rounds");
        let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
        write_tool_mod(&mod_dir, &["probe.echo", "probe.detach"]);
        let mut session = Session::open(&tool_args(&mod_dir, &runs_root, "rounds")).expect("开局");
        // 已达上限：再来的请求直接忽略（不递归、不 spawn）
        session.tool_depth.set(MAX_TOOL_ROUNDS);
        let receipt = dispatch(&mut session, "event.start");
        assert!(
            receipt.get("tools").is_none(),
            "超限后不得再跑工具：{receipt}"
        );
        let warnings = receipt["warnings"].as_array().cloned().unwrap_or_default();
        let text = warnings
            .iter()
            .filter_map(Json::as_str)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("轮上限"), "{text}");
        assert!(pending_calls(&session.run_dir).is_empty());
        // 归零守卫：未超限的一步把深度还回去（空数组 = 不 spawn，测试不依赖子进程）
        session.tool_depth.set(MAX_TOOL_ROUNDS - 1);
        assert!(session
            .run_tool_requests(&json!({"meta": {"tool_calls": []}}), 0)
            .is_empty());
        assert_eq!(
            session.tool_depth.get(),
            MAX_TOOL_ROUNDS - 1,
            "深度必须归零（Drop guard）"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reset_clears_pending_tools() {
        let dir = case_dir("reset");
        let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
        write_tool_mod(&mod_dir, &["probe.echo", "probe.detach"]);
        let mut session = Session::open(&tool_args(&mod_dir, &runs_root, "reset")).expect("开局");
        let (_, call) = pending_call(&session, "probe.detach", "d1", &[]);
        pending_tools_insert(&session.run_dir, &call).expect("登记");
        write_tool_result(
            &session.run_dir,
            &ToolResult {
                call_id: "d1".to_owned(),
                tool: "probe.detach".to_owned(),
                status: "ok".to_owned(),
                code: "0".to_owned(),
                output: String::new(),
                payload: "d1|probe.detach|ok|0||inbox-d".to_owned(),
                external_id: "inbox-d".to_owned(),
            },
        )
        .expect("写结果");
        session.reset_artifacts(false).expect("重置必须成功");
        assert!(
            !pending_tools_path(&session.run_dir).exists(),
            "pending 必须随本代清空"
        );
        assert!(
            !tool_results_dir(&session.run_dir).exists(),
            "结果暂存必须随本代清空"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn allowlist_gate_is_enforced_at_load_time() {
        let dir = case_dir("allowlist");
        let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
        // 只放行一条：另一条声明必须让开局 fail-closed（I6）。
        write_tool_mod(&mod_dir, &["probe.echo"]);
        let error = match Session::open(&tool_args(&mod_dir, &runs_root, "gate")) {
            Ok(_) => panic!("未放行的声明必须拒绝启动"),
            Err(message) => message,
        };
        assert!(error.contains("probe.detach"), "{error}");
        assert!(error.contains("allowedTools"), "{error}");
        // 补上放行清单即可开局
        write_tool_mod(&mod_dir, &["probe.echo", "probe.detach"]);
        assert!(Session::open(&tool_args(&mod_dir, &runs_root, "gate")).is_ok());
        let _ = fs::remove_dir_all(&dir);
    }
}

// ============================================================================
// §r1 单测：制品 CLI（--artifact-put / --artifact-get）+ VFS 持久化 + 内置工具
//
// 断言口径：**能直接比字节的地方就比字节**。内置工具的输出契约是「一行
// `k=v;k=v`」，所以这里断言 [`wire_line`] 的结果（它就是 `println!` 的那一行），
// 而不是去抓 stdout——抓 stdout 要么引入依赖，要么测一个自己搭的假管道。
// ============================================================================

#[cfg(test)]
mod vfs_host_tests {
    use super::*;

    fn case_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ygg-r1-vfs-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    /// 最小合成 Mod：一个会写 VFS 的 handler + 一个写一半就报错的 handler。
    fn write_vfs_mod(dir: &Path) {
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
  ygg.narrate("开局")
  ygg.choice("下一步？", { "推进" })
end)
ygg.on("event.write", function()
  local out = ygg.fs.write("artifacts/note.md", "第一版")
  ygg.narrate("w=" .. out.path)
end)
ygg.on("event.boom", function()
  ygg.fs.write("artifacts/dirty.md", "leak")
  error("boom")
end)
ygg.on("agent.summary", function()
  ygg.meta("summary", { open = true })
  ygg.narrate("摘要就绪")
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

    fn vfs_args(mod_dir: Option<&Path>, runs_root: &Path, run_id: &str, resume: bool) -> Args {
        Args {
            packages: None,
            mod_dir: mod_dir.map(Path::to_path_buf),
            open_style: Some(if resume {
                crate::OpenStyle::Open
            } else {
                crate::OpenStyle::New
            }),
            run_root: runs_root.join(run_id),
            runs_parent: runs_root.to_path_buf(),
            // 旧「显式新局」= 总是重置 → 新语义映射为 new --force（显式许可）。
            force: !resume,
            at: None,
            settle: false,
            watch: false,
            event: None,
            payload: None,
            output: crate::OutputFormat::Text,
            drive: Drive::None,
            inbox: None,
            store: Some(ygg_store::Backend::Json),
            wait_tools: false,
            tool_timeout_ms: None,
            run_tool_worker: None,
            artifact_put: None,
            artifact_get: None,
            builtin_tool: None,
            daemon: false,
            harness: None,
            actor: None,
            actor_session: None,
            serve: false,
            port: None,
            panel_dir: None,
            poll_ms: None,
        }
    }

    /// 一个「已经有后端产物」的局目录（制品 CLI 不新造 run，所以必须先有产物）。
    fn seeded_run(args: &Args) -> PathBuf {
        let run_dir = args.run_root.clone();
        fs::create_dir_all(&run_dir).expect("建 run 目录");
        fs::write(run_dir.join(AUTOSAVE_FILE), b"{}").expect("写 autosave");
        run_dir
    }

    // ---- `--artifact-put` / `--artifact-get` 的核心 ----

    #[test]
    fn artifact_put_then_get_roundtrips_bytes() {
        let dir = case_dir("put-get");
        let args = vfs_args(None, &dir.join("runs"), "pg", false);
        let run_dir = seeded_run(&args);
        let body = "内容没有尾随换行";
        let source = dir.join("source.md");
        fs::write(&source, body).expect("写源文件");

        let receipt =
            artifact_put_receipt(&args, &format!("artifacts/note.md={}", source.display()))
                .expect("写入必须成功");
        // 回执形状：ok / path / bytes / sha256（**由端口给**，宿主不现算）。
        assert_eq!(receipt["ok"], json!(true));
        assert_eq!(receipt["path"], json!("artifacts/note.md"));
        assert_eq!(receipt["bytes"], json!(body.len()));
        assert_eq!(
            receipt["sha256"].as_str().expect("sha256").len(),
            64,
            "小写十六进制 64 字符"
        );
        // 字节保真：读回来与源文件**逐字节**相等（不补换行）。
        let bytes = artifact_get_bytes(&args, "artifacts/note.md").expect("读回必须成功");
        assert_eq!(bytes, body.as_bytes());
        // 真的落在后端的制品面里（不是只回执成功）。
        assert!(run_dir.join("artifacts/note.md").is_file());
    }

    #[test]
    fn artifact_get_missing_is_err() {
        let dir = case_dir("get-missing");
        let args = vfs_args(None, &dir.join("runs"), "gm", false);
        seeded_run(&args);
        let message = artifact_get_bytes(&args, "artifacts/none.md").expect_err("缺失必须报错");
        assert!(message.contains("不存在"), "{message}");
    }

    #[test]
    fn artifact_put_rejects_bad_path_and_oversize() {
        let dir = case_dir("put-gate");
        let args = vfs_args(None, &dir.join("runs"), "gate", false);
        seeded_run(&args);
        let source = dir.join("s.md");
        fs::write(&source, "x").expect("写源文件");
        let spec = |path: &str| format!("{path}={}", source.display());

        // 路径门在端口内：宿主不重写第二份规则。
        let message = artifact_put_receipt(&args, &spec("artifacts/../escape.md"))
            .expect_err("穿越路径必须被拒");
        assert!(message.contains("制品路径"), "{message}");
        let message =
            artifact_put_receipt(&args, &spec("artifact/x.md")).expect_err("坏前缀必须被拒");
        assert!(message.contains("制品路径"), "{message}");

        // 内容上限：256 KiB + 1 字节。
        let big = dir.join("big.md");
        fs::write(&big, vec![b'a'; 256 * 1024 + 1]).expect("写大文件");
        let message = artifact_put_receipt(&args, &format!("artifacts/big.md={}", big.display()))
            .expect_err("超限必须被拒");
        assert!(message.contains("制品内容"), "{message}");
        // 零写入。
        assert!(!args.runs_parent.join("gate").join("artifacts").exists());
    }

    #[test]
    fn artifact_put_requires_an_existing_run() {
        let dir = case_dir("put-newrun");
        let args = vfs_args(None, &dir.join("runs"), "no-such-run", false);
        let source = dir.join("s.md");
        fs::write(&source, "x").expect("写源文件");
        let message = artifact_put_receipt(&args, &format!("artifacts/a.md={}", source.display()))
            .expect_err("不存在的局必须被拒");
        assert!(message.contains("不存在"), "{message}");
        // **不新造 run**：连目录都不该被建出来。
        assert!(!args.runs_parent.join("no-such-run").exists());
    }

    // ---- Session 的 VFS 持久化（写 → 落盘 → 续局回灌）----

    #[test]
    fn session_vfs_survives_resume() {
        let dir = case_dir("resume");
        let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
        write_vfs_mod(&mod_dir);
        let artifact_path = runs_root.join("rv").join("artifacts/note.md");

        {
            let mut session = Session::open(&vfs_args(Some(&mod_dir), &runs_root, "rv", false))
                .expect("新局开局");
            assert_eq!(session.kernel.vfs_len(), 0, "新局从空 VFS 起步");
            let receipt = session.dispatch_event("event.write", "0");
            assert_eq!(receipt["ok"], json!(true), "{receipt}");
            assert_eq!(session.kernel.vfs_len(), 1);
            // 成功步 → 制品**当场**落盘（不是等收口）。
            assert_eq!(
                fs::read(&artifact_path).expect("制品必须在盘上"),
                "第一版".as_bytes()
            );
        }

        // 续局：VFS 不在存档字节里，只能是宿主从制品面回灌。
        let mut resumed =
            Session::open(&vfs_args(Some(&mod_dir), &runs_root, "rv", true)).expect("续局");
        assert_eq!(resumed.kernel.vfs_len(), 1, "续局必须把制品回灌进 VFS");
        assert!(
            resumed.kernel.vfs_take_dirty().is_empty(),
            "预载不是本次写入，不得标脏"
        );
        // 回灌的内容与摘要是盘上那份（不是空壳）。
        let entries = resumed.kernel.vfs_entries();
        assert_eq!(entries[0].0, "artifacts/note.md");
        assert_eq!(entries[0].1, "第一版".as_bytes());
        assert_eq!(entries[0].2, ygg_store::sha256_hex("第一版".as_bytes()));
    }

    #[test]
    fn failed_dispatch_never_reaches_the_artifact_store() {
        let dir = case_dir("rollback");
        let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
        write_vfs_mod(&mod_dir);
        let mut session =
            Session::open(&vfs_args(Some(&mod_dir), &runs_root, "rb", false)).expect("新局开局");
        let receipt = session.dispatch_event("event.boom", "0");
        assert_eq!(receipt["ok"], json!(false), "{receipt}");
        // 内核回滚 + 宿主不 flush ⇒ 盘上不该有失败步写的文件。
        assert_eq!(session.kernel.vfs_len(), 0, "失败步的文件必须回滚");
        assert!(
            !runs_root.join("rb").join("artifacts/dirty.md").exists(),
            "失败步不得落盘"
        );
    }

    // ---- wire 契约 ----

    #[test]
    fn wire_line_is_single_line_and_sanitized() {
        let line = wire_line(&[("a", "x;y,z|w\nv\r\0".to_owned()), ("b", String::new())]);
        assert_eq!(line, "a=x_y_z_w_v__;b=");
        for forbidden in ['|', '\n', '\r', '\0'] {
            assert!(
                !line.contains(forbidden),
                "wire 行不得含 {forbidden:?}：{line}"
            );
        }
        // **空值字段仍然在**（调用方按位解析，缺字段与空字段含义不同）。
        assert!(line.ends_with(";b="));
    }

    // ---- 内置工具（六个 --builtin-tool op）----

    fn op_flags(pairs: &[(&str, &str)]) -> OpFlags {
        OpFlags {
            values: pairs
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        }
    }

    /// 一个有一次提交的真 git 仓库（worktree 三个 op 的现场）。
    fn git_fixture(dir: &Path) -> PathBuf {
        let repo = dir.join("repo");
        fs::create_dir_all(&repo).expect("建仓库");
        let run = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args([
                    "-c",
                    "user.email=r1@example.com",
                    "-c",
                    "user.name=r1",
                    "-c",
                    "init.defaultBranch=main",
                    "-c",
                    "commit.gpgsign=false",
                ])
                .args(args)
                .output()
                .expect("执行 git");
            assert!(
                output.status.success(),
                "git {args:?} 失败：{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        };
        run(&["init", "-q"]);
        fs::write(repo.join("a.txt"), "a").expect("写文件");
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "init"]);
        repo
    }

    fn lane_wt(root: &Path, repo: &Path, lane: u8) -> PathBuf {
        root.join(format!("{}-lane-{lane}", repo_label(repo)))
    }

    #[test]
    fn worktree_root_for_respects_precedence_and_defaults_to_home_tmp_worktrees() {
        let home = PathBuf::from("/Users/tester");
        assert_eq!(
            worktree_root_for(Some("/custom/wts"), Some("/env/wts"), Some(&home)),
            PathBuf::from("/custom/wts")
        );
        assert_eq!(
            worktree_root_for(None, Some("/env/wts"), Some(&home)),
            PathBuf::from("/env/wts")
        );
        assert_eq!(
            worktree_root_for(None, Some("   "), Some(&home)),
            PathBuf::from("/Users/tester/tmp/worktrees")
        );
        assert_eq!(
            worktree_root_for(None, None, Some(&home)),
            PathBuf::from("/Users/tester/tmp/worktrees")
        );
        assert_eq!(
            worktree_root_for(None, None, None),
            std::env::temp_dir().join("ygg-worktrees")
        );
    }

    #[test]
    fn builtin_worktree_claim_and_release_roundtrip() {
        let dir = case_dir("claim");
        let repo = git_fixture(&dir);
        let root = dir.join("wts");
        let (root_s, repo_s) = (root.display().to_string(), repo.display().to_string());
        let flags = op_flags(&[
            ("--lane", "1"),
            ("--req", "r1"),
            ("--branch", "lane/b1"),
            ("--root", &root_s),
            ("--repo", &repo_s),
        ]);
        let fields = claim_fields_with(&flags).expect("首次认领必须成功");
        assert_eq!(
            fields.iter().map(|(key, _)| *key).collect::<Vec<_>>(),
            vec!["lane", "req", "branch", "base", "clean", "reused", "path"]
        );
        let line = wire_line(&fields);
        assert!(
            line.starts_with("lane=1;req=r1;branch=lane/b1;base="),
            "{line}"
        );
        assert!(line.contains(";clean=1;reused=0;path="), "{line}");

        let wt = lane_wt(&root, &repo, 1);
        assert!(wt.is_dir(), "worktree 必须被建出来");
        assert!(wt.join("a.txt").is_file());

        // 写入 `.git/info/exclude` 忽略 `target/`，并在泳道内放入构建缓存 `target/cache.bin` 与未跟踪脏文件 `wip.txt`。
        let exclude = repo.join(".git/info/exclude");
        fs::write(&exclude, "target/\n").expect("写 exclude");
        fs::create_dir_all(wt.join("target")).expect("建 target");
        fs::write(wt.join("target/cache.bin"), "warm-cache").expect("写构建缓存");
        fs::write(wt.join("wip.txt"), "untracked-scratch").expect("写未跟踪临时文件");

        // 释放（停泊）：目录仍在、HEAD 处于 detached（允许删除原分支 `lane/b1`）、`wip.txt` 被清而 `target/cache.bin` 保留。
        let release = op_flags(&[("--lane", "1"), ("--root", &root_s), ("--repo", &repo_s)]);
        let rel_fields = release_fields_with(&release).expect("释放（停泊）必须成功");
        let rel_line = wire_line(&rel_fields);
        assert!(
            rel_line.starts_with("lane=1;released=1;parked=1;path="),
            "{rel_line}"
        );
        assert!(wt.is_dir(), "停泊后 worktree 目录必须保留");
        assert!(
            !wt.join("wip.txt").exists(),
            "未跟踪非忽略文件必须在停泊时清理"
        );
        assert_eq!(
            fs::read_to_string(wt.join("target/cache.bin")).expect("读构建缓存"),
            "warm-cache",
            "gitignored 构建缓存必须在停泊后保留"
        );
        // 因为 HEAD 已 detach，主检出可以安全删除已合并的分支 `lane/b1`。
        git_capture(&repo, &["branch", "-D", "lane/b1"]).expect("停泊后原分支应当可删");

        // 再次认领同一泳道给新需求 `r2`：`reused=1` 且 `target/cache.bin` 依旧温热。
        let flags_r2 = op_flags(&[
            ("--lane", "1"),
            ("--req", "r2"),
            ("--branch", "lane/b2"),
            ("--root", &root_s),
            ("--repo", &repo_s),
        ]);
        let again = claim_fields_with(&flags_r2).expect("二次认领必须成功");
        let again_line = wire_line(&again);
        assert!(
            again_line.starts_with("lane=1;req=r2;branch=lane/b2;base="),
            "{again_line}"
        );
        assert!(
            again_line.contains(";clean=1;reused=1;path="),
            "{again_line}"
        );
        assert_eq!(
            fs::read_to_string(wt.join("target/cache.bin")).expect("复用后读构建缓存"),
            "warm-cache"
        );
    }

    #[test]
    fn worktree_claim_detaches_branch_if_held_by_another_lane_and_reuses_in_place() {
        let dir = case_dir("claim-reassigned");
        let repo = git_fixture(&dir);
        let root = dir.join("wts");
        let (root_s, repo_s) = (root.display().to_string(), repo.display().to_string());
        let lane1 = op_flags(&[
            ("--lane", "1"),
            ("--req", "r1"),
            ("--branch", "feature/a3f9-map-single-gone"),
            ("--root", &root_s),
            ("--repo", &repo_s),
        ]);
        claim_fields_with(&lane1).expect("车道 1 首次认领");

        // 若 r1 重派到车道 2（同分支 `feature/a3f9-map-single-gone`），车道 1 自动 detach 释放分支锁，车道 2 顺利挂载。
        let lane2 = op_flags(&[
            ("--lane", "2"),
            ("--req", "r1"),
            ("--branch", "feature/a3f9-map-single-gone"),
            ("--root", &root_s),
            ("--repo", &repo_s),
        ]);
        let fields = claim_fields_with(&lane2).expect("跨车道重派认领必须成功");
        let line = wire_line(&fields);
        assert!(
            line.contains("lane=2;req=r1;branch=feature/a3f9-map-single-gone;"),
            "{line}"
        );
        assert!(line.contains(";clean=1;reused=0;path="), "{line}");
    }

    #[test]
    fn worktree_release_is_idempotent_when_lane_dir_is_absent() {
        let dir = case_dir("release");
        let repo = git_fixture(&dir);
        let root = dir.join("wts");
        let (root_s, repo_s) = (root.display().to_string(), repo.display().to_string());
        let release = op_flags(&[("--lane", "3"), ("--root", &root_s), ("--repo", &repo_s)]);

        let fields = release_fields_with(&release).expect("未创建的车道释放也必须成功");
        let line = wire_line(&fields);
        assert!(
            line.starts_with("lane=3;released=1;parked=0;path="),
            "{line}"
        );
    }

    #[test]
    fn lane_probe_reports_distance_and_is_clean_after_claim() {
        let dir = case_dir("lane-probe");
        let repo = git_fixture(&dir);
        let root = dir.join("wts");
        let (root_s, repo_s) = (root.display().to_string(), repo.display().to_string());
        let base = git_one(&repo, &["rev-parse", "HEAD"]).expect("读 HEAD");
        let claim = op_flags(&[
            ("--lane", "4"),
            ("--req", "r4"),
            ("--branch", "lane/b4"),
            ("--root", &root_s),
            ("--repo", &repo_s),
        ]);
        claim_fields_with(&claim).expect("认领");

        let probe = op_flags(&[
            ("--lane", "4"),
            ("--base", &base),
            ("--branch", "lane/b4"),
            ("--root", &root_s),
            ("--repo", &repo_s),
        ]);
        let fields = lane_probe_fields_with(&probe).expect("探测");
        assert_eq!(
            fields.iter().map(|(key, _)| *key).collect::<Vec<_>>(),
            vec!["lane", "branch", "base", "head", "commits", "clean", "files"]
        );
        let line = wire_line(&fields);
        // base 原样回显（短 sha 保持短）；commits=0（刚认领，还没往前走）。
        assert!(
            line.starts_with(&format!("lane=4;branch=lane/b4;base={base};head=")),
            "{line}"
        );
        assert!(line.contains(";commits=0;clean=1;files="), "{line}");
    }

    #[test]
    fn lane_probe_fails_closed_on_an_unknown_branch() {
        let dir = case_dir("lane-probe-bad");
        let repo = git_fixture(&dir);
        let repo_s = repo.display().to_string();
        let base = git_one(&repo, &["rev-parse", "HEAD"]).expect("读 HEAD");
        let probe = op_flags(&[
            ("--lane", "5"),
            ("--base", &base),
            ("--branch", "lane/nope"),
            ("--repo", &repo_s),
        ]);
        match lane_probe_fields_with(&probe) {
            Err(OpFailure::Detail { slug, detail }) => {
                assert_eq!(slug, "rev-list", "错误必须点名哪一步失败：{detail}");
            }
            _ => panic!("未知分支必须失败（不得静默报 commits=0）"),
        }
    }

    #[test]
    fn artifact_probe_answers_exists_zero_instead_of_failing() {
        let dir = case_dir("artifact-probe");
        let args = vfs_args(None, &dir.join("runs"), "ap", false);
        let run_dir = seeded_run(&args);
        let run_dir_s = run_dir.display().to_string();

        // ① 缺失：**正常答案**，不是错误。
        let missing = artifact_probe_fields_with(&op_flags(&[
            ("--path", "artifacts/none.md"),
            ("--run-dir", &run_dir_s),
        ]))
        .expect("缺失必须是正常回执");
        assert_eq!(
            wire_line(&missing),
            "path=artifacts/none.md;exists=0;bytes=0;sha256="
        );

        // ② 存在：bytes / sha256 来自端口（不是宿主自己拼的）。
        fs::create_dir_all(run_dir.join("artifacts")).expect("建制品目录");
        fs::write(run_dir.join("artifacts/none.md"), b"hi").expect("写制品");
        let hit = artifact_probe_fields_with(&op_flags(&[
            ("--path", "artifacts/none.md"),
            ("--run-dir", &run_dir_s),
        ]))
        .expect("存在的制品");
        assert_eq!(
            wire_line(&hit),
            format!(
                "path=artifacts/none.md;exists=1;bytes=2;sha256={}",
                ygg_store::sha256_hex(b"hi")
            )
        );

        // ③ 非法路径 = 调用方 bug，不能当成「不存在」。
        match artifact_probe_fields_with(&op_flags(&[
            ("--path", "artifact/bad.md"),
            ("--run-dir", &run_dir_s),
        ])) {
            Err(OpFailure::Detail { slug, .. }) => assert_eq!(slug, "path"),
            _ => panic!("非法路径必须报错（不得伪装成 exists=0）"),
        }
    }

    #[test]
    fn archive_sessions_moves_only_matching_jsonl() {
        let dir = case_dir("archive");
        let sessions = dir.join("sessions");
        fs::create_dir_all(&sessions).expect("建会话目录");
        fs::write(sessions.join("a.jsonl"), "{\"parent\":\"p1\"}\n").expect("写 a");
        fs::write(sessions.join("b.jsonl"), "{\"x\":1}\n").expect("写 b");
        // parent 自己的转录档：名字里就有 parent → 不搬。
        fs::write(sessions.join("p1.jsonl"), "{\"parent\":\"p1\"}\n").expect("写 p1");
        // 非 `.jsonl`：永不入选（数据红线：本 op 只碰会话转录）。
        fs::write(sessions.join("notes.txt"), "p1").expect("写 txt");

        let trash = dir.join("trash");
        let outcome = sweep_sessions(&sessions, &trash, "p1").expect("清扫");
        assert_eq!(outcome.moved, 1, "只有 a.jsonl 该被搬走");
        assert_eq!(outcome.remaining, 0, "重扫必须为空（清扫完成）");
        assert!(trash.join("a.jsonl").exists());
        assert!(!sessions.join("a.jsonl").exists());
        assert!(sessions.join("b.jsonl").exists(), "不匹配的不得被动");
        assert!(
            sessions.join("p1.jsonl").exists(),
            "parent 自己的转录档不得搬"
        );
        assert!(sessions.join("notes.txt").exists(), "非 .jsonl 永不入选");
    }

    #[test]
    fn verify_gates_match_the_documented_suite() {
        // `fast` = 一个 `cargo check`；`full` = AGENTS.md 的门禁三件套（顺序固定）。
        assert_eq!(verify_gates("fast").len(), 1);
        assert_eq!(verify_gates("fast")[0].1[0], "check");
        let full = verify_gates("full");
        assert_eq!(
            full.iter().map(|(label, _)| *label).collect::<Vec<_>>(),
            vec![
                "cargo test --workspace",
                "cargo test --example egg -p ygg-sandbox",
                "cargo test -p ygg-sandbox --test agent_seed_semantics",
            ]
        );
    }

    #[test]
    fn parse_custom_verify_commands_splits_semicolon_separated_commands() {
        let parsed = parse_custom_verify_commands(" npm test -- --run ; pytest -q ; ");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0, "npm test -- --run");
        assert_eq!(parsed[0].1, "npm");
        assert_eq!(parsed[0].2, vec!["test", "--", "--run"]);
        assert_eq!(parsed[1].0, "pytest -q");
        assert_eq!(parsed[1].1, "pytest");
        assert_eq!(parsed[1].2, vec!["-q"]);
    }

    /// `@self` 别名：`egg` 与 `@self` 都解析到本可执行文件（同一分支）。
    #[test]
    fn self_alias_matches_the_contract_placeholder() {
        assert_eq!(SELF_ALIAS, "egg");
        assert_eq!(ygg_sandbox::host_contract::SELF_PROGRAM, "@self");
        assert_ne!(SELF_ALIAS, ygg_sandbox::host_contract::SELF_PROGRAM);
    }
}

// ============================================================================
// §r2 单测：守护模式（`--daemon`）的 JSON-RPC 面
//
// 断言口径：**只测纯内存的 `handle_daemon_request`**。它的契约就是「不碰
// stdout」（stdout 是守护进程的协议面），stdin 循环与 flush 由 `run_daemon`
// 的十几行承担：去抓管道只能得到「自己搭的假管道」的结论，且与 `--daemon`
// 的进程生命周期绑死。
//
// `summary` 里的 `jobs` / `gates` 是本模块**自造**的载荷：宿主只负责原样透传
// （形状由 Mod 声明），所以这里断言的是通道，不是 Mod 的业务语义——后者在
// `agent_seed_semantics` 的 `summary_exposes_structured_jobs_and_gates` 里。
// ============================================================================

#[cfg(test)]
mod daemon_rpc_tests {
    use super::*;

    fn case_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ygg-daemon-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    /// 最小合成 Mod：一个累加 kv 的 handler、一个**读** `ygg.fs.stat` 的 handler，
    /// 以及一个把 `jobs` / `gates` 写进 `meta.summary` 的只读摘要动作。
    ///
    /// 沙箱裁掉了 `string` / `table` / `math`：只能用 `#` / `..` / `tostring` / `type`。
    fn write_daemon_mod(dir: &Path) {
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
  ygg.narrate("开局")
end)
ygg.on("event.tick", function(payload)
  ygg.kv.set("ticks", (ygg.kv.get("ticks") or 0) + 1)
  ygg.narrate("tick=" .. tostring(ygg.kv.get("ticks")) .. " payload=" .. tostring(payload))
end)
ygg.on("event.probe_vfs", function()
  local hit = ygg.fs.stat("artifacts/note.md")
  local sha = hit.sha256
  ygg.narrate("vfs=" .. tostring(hit.exists) .. ":" .. tostring(hit.bytes) .. ":" .. tostring(#sha))
end)
ygg.on("agent.summary", function()
  ygg.meta("summary", {
    stage = "coordinate",
    jobs = {
      { id = "r1", rev = 3, stage = "development", lane = 1, branch = "feat-x", base = "abc1234", title = "切片一", description = "描述一" },
    },
    gates = {
      { id = "r2", gate = "plan", rev = 2, title = "切片二" },
    },
  })
  ygg.narrate("摘要就绪")
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

    fn daemon_args(mod_dir: &Path, runs_root: &Path, run_id: &str) -> Args {
        Args {
            packages: None,
            mod_dir: Some(mod_dir.to_path_buf()),
            open_style: Some(crate::OpenStyle::New),
            run_root: runs_root.join(run_id),
            runs_parent: runs_root.to_path_buf(),
            force: false,
            at: None,
            settle: false,
            watch: false,
            event: None,
            payload: None,
            output: crate::OutputFormat::Text,
            drive: Drive::None,
            inbox: None,
            store: Some(ygg_store::Backend::Json),
            wait_tools: false,
            tool_timeout_ms: None,
            run_tool_worker: None,
            artifact_put: None,
            artifact_get: None,
            builtin_tool: None,
            daemon: true,
            harness: None,
            actor: None,
            actor_session: None,
            serve: false,
            port: None,
            panel_dir: None,
            poll_ms: None,
        }
    }

    /// 发一条请求并断言「不退出」（只有 `shutdown` 能把第二项置 true）。
    #[track_caller]
    fn call(session: &mut Session, args: &Args, req: Json) -> Json {
        let (reply, quit) = handle_daemon_request(session, args, &req);
        assert!(!quit, "只有 shutdown 能让守护进程退出：{reply}");
        assert!(reply.is_object(), "应答恒为 JSON 对象：{reply}");
        reply
    }

    fn narrative_text(reply: &Json) -> String {
        reply["narrative"].to_string()
    }

    #[test]
    fn rpc_covers_ping_dispatch_vfs_summary_and_shutdown() {
        let dir = case_dir("rpc");
        let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
        write_daemon_mod(&mod_dir);
        let args = daemon_args(&mod_dir, &runs_root, "d1");
        let mut session = Session::open(&args).expect("守护模式开局");

        // ① ping：runId 与存储后端都取自真实装载面（不是硬编码）。
        let ping = call(&mut session, &args, json!({"op": "ping", "id": 7}));
        assert_eq!(ping["ok"], json!(true), "{ping}");
        assert_eq!(ping["op"], json!("ping"));
        assert_eq!(ping["runId"], json!("d1"));
        assert_eq!(ping["store"], json!("json"));
        assert_eq!(
            ping["id"],
            json!(7),
            "id 必须原样回显（单通道配对）：{ping}"
        );

        // ② dispatch：字符串载荷要**包成 JSON 标量**再下发（裸串不是合法载荷）。
        let first = call(
            &mut session,
            &args,
            json!({"op": "dispatch", "event": "event.tick", "payload": "abc"}),
        );
        assert_eq!(first["ok"], json!(true), "{first}");
        assert_eq!(first["op"], json!("dispatch"));
        assert_eq!(first["event"], json!("event.tick"));
        let lines = narrative_text(&first);
        assert!(lines.contains("tick=1"), "{first}");
        assert!(
            lines.contains("payload=abc"),
            "载荷必须是字符串 abc：{first}"
        );

        // dispatch 的回执自带最新摘要：调度端不必再发一条 `summary`（也就不会
        // 看到「派发之后、摘要之前」的中间态）。
        let summary = &first["summary"];
        assert_eq!(summary["ok"], json!(true), "{first}");
        let jobs = summary["summary"]["jobs"].as_array().expect("summary.jobs");
        assert_eq!(jobs.len(), 1, "{first}");
        assert_eq!(jobs[0]["id"], json!("r1"));
        assert_eq!(jobs[0]["stage"], json!("development"));
        assert_eq!(jobs[0]["lane"], json!(1));
        assert_eq!(jobs[0]["branch"], json!("feat-x"));
        assert_eq!(jobs[0]["base"], json!("abc1234"));
        assert_eq!(jobs[0]["title"], json!("切片一"));
        assert_eq!(jobs[0]["description"], json!("描述一"));
        let gates = summary["summary"]["gates"]
            .as_array()
            .expect("summary.gates");
        assert_eq!(gates.len(), 1, "{first}");
        assert_eq!(gates[0]["gate"], json!("plan"));
        assert_eq!(gates[0]["rev"], json!(2));
        assert_eq!(gates[0]["title"], json!("切片二"));

        // ③ vfs_put：端口与内核 `ygg.fs` 必须**同时**可见。
        let body = "第一版";
        let sha = ygg_store::sha256_hex(body.as_bytes());
        let put = call(
            &mut session,
            &args,
            json!({"op": "vfs_put", "path": "artifacts/note.md", "content": body}),
        );
        assert_eq!(put["ok"], json!(true), "{put}");
        assert_eq!(put["path"], json!("artifacts/note.md"));
        assert_eq!(put["bytes"], json!(body.len()));
        assert_eq!(put["sha256"], json!(sha));
        assert_eq!(
            fs::read(runs_root.join("d1/artifacts/note.md")).expect("制品必须落盘"),
            body.as_bytes(),
            "vfs_put 必须写进本局后端"
        );
        // 同一局内的 `ygg.fs.stat` 立即可见（只落盘不灌 VFS 会让这里报 exists=false）。
        let probe = call(
            &mut session,
            &args,
            json!({"op": "dispatch", "event": "event.probe_vfs"}),
        );
        let probe_lines = narrative_text(&probe);
        assert!(
            probe_lines.contains(&format!("vfs=true:{}:64", body.len())),
            "vfs_put 之后内核必须立刻看得见（exists=true / bytes / 64 位 sha）：{probe}"
        );

        // ④ vfs_get / vfs_list：读面与端口同一份事实。
        let got = call(
            &mut session,
            &args,
            json!({"op": "vfs_get", "path": "artifacts/note.md"}),
        );
        assert_eq!(got["ok"], json!(true), "{got}");
        assert_eq!(got["content"], json!(body));
        assert_eq!(got["sha256"], json!(sha));
        let missing = call(
            &mut session,
            &args,
            json!({"op": "vfs_get", "path": "artifacts/none.md"}),
        );
        assert_eq!(missing["ok"], json!(false), "缺失必须是失败应答：{missing}");
        assert!(
            missing["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .contains("不存在"),
            "{missing}"
        );
        let list = call(&mut session, &args, json!({"op": "vfs_list"}));
        let artifacts = list["artifacts"].as_array().expect("artifacts");
        assert_eq!(artifacts.len(), 1, "{list}");
        assert_eq!(artifacts[0]["path"], json!("artifacts/note.md"));
        assert_eq!(artifacts[0]["sha256"], json!(sha));

        // ⑤ summary 是**只读**面：不落 RunStore 步（否则守护进程每探一次就把日志写胖）。
        let before = session.store.steps().expect("读事件日志").records.len();
        let summary = call(&mut session, &args, json!({"op": "summary", "id": "s1"}));
        let after = session.store.steps().expect("读事件日志").records.len();
        assert_eq!(after, before, "summary 不得往事件日志加步");
        assert_eq!(summary["ok"], json!(true), "{summary}");
        assert_eq!(summary["op"], json!("summary"));
        assert_eq!(summary["id"], json!("s1"));
        assert!(summary["summary"]["jobs"].is_array(), "{summary}");

        // ⑥ shutdown：唯一把 should_quit 置 true 的动作。
        let (bye, quit) = handle_daemon_request(&mut session, &args, &json!({"op": "shutdown"}));
        assert_eq!(bye["ok"], json!(true), "{bye}");
        assert!(quit, "shutdown 必须要求退出循环");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rpc_refuses_unknown_ops_and_non_scalar_payloads() {
        let dir = case_dir("refuse");
        let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
        write_daemon_mod(&mod_dir);
        let args = daemon_args(&mod_dir, &runs_root, "d2");
        let mut session = Session::open(&args).expect("守护模式开局");

        let unknown = call(&mut session, &args, json!({"op": "nope"}));
        assert_eq!(unknown["ok"], json!(false), "{unknown}");
        assert!(
            unknown["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .contains("未知的 op"),
            "{unknown}"
        );

        // 数组 / 对象不是合法载荷（与 `--payload` 同一口径：必须是 JSON 标量）。
        let array = call(
            &mut session,
            &args,
            json!({"op": "dispatch", "event": "event.tick", "payload": [1, 2]}),
        );
        assert_eq!(array["ok"], json!(false), "{array}");
        assert!(
            array["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .contains("JSON 标量"),
            "{array}"
        );

        // 缺 event / 空 event：明确拒绝，而不是拿入口载荷去措。
        let no_event = call(&mut session, &args, json!({"op": "dispatch"}));
        assert_eq!(no_event["ok"], json!(false), "{no_event}");
        let blank = call(
            &mut session,
            &args,
            json!({"op": "dispatch", "event": "  "}),
        );
        assert_eq!(blank["ok"], json!(false), "{blank}");

        // 缺数字载荷 = 用契约的 defaultPayload（这里 "0"），不是错误。
        let defaulted = call(
            &mut session,
            &args,
            json!({"op": "dispatch", "event": "event.tick"}),
        );
        assert_eq!(defaulted["ok"], json!(true), "{defaulted}");
        assert!(
            narrative_text(&defaulted).contains("payload=0"),
            "{defaulted}"
        );

        // 制品路径形状由端口裁决（宿主不重写一份）——穿越必须被拒。
        let escaped = call(
            &mut session,
            &args,
            json!({"op": "vfs_put", "path": "../escape.md", "content": "x"}),
        );
        assert_eq!(escaped["ok"], json!(false), "{escaped}");

        let _ = fs::remove_dir_all(&dir);
    }

    /// T4（r11 fail-closed）：daemon `op=dispatch` 派发未注册事件 = 同语义
    /// 错误应答——`ok:false` + `error.code=VALIDATION`（文案含已注册全集）、
    /// `op:"dispatch"` 与 `summary` 字段仍在（调度端形状不破，一次往返的
    /// 契约不受错误步影响）。
    #[test]
    fn rpc_dispatch_unknown_event_returns_readable_dispatch_receipt() {
        let dir = case_dir("t4");
        let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
        write_daemon_mod(&mod_dir);
        let args = daemon_args(&mod_dir, &runs_root, "t4");
        let mut session = Session::open(&args).expect("守护模式开局");

        let reply = call(
            &mut session,
            &args,
            json!({"op": "dispatch", "event": "event.nope", "id": 9}),
        );
        assert_eq!(reply["ok"], json!(false), "{reply}");
        assert_eq!(reply["op"], json!("dispatch"), "op 原样回填：{reply}");
        assert_eq!(reply["id"], json!(9), "{reply}");
        assert_eq!(reply["error"]["code"], json!("VALIDATION"), "{reply}");
        let message = reply["error"]["message"].as_str().unwrap_or_default();
        assert!(
            message.starts_with("未注册事件 `event.nope`："),
            "文案要点名事件：{reply}"
        );
        assert!(
            message.contains("已注册"),
            "文案要带已注册全集（write_daemon_mod 注册 event.tick / event.probe_vfs）：{reply}"
        );
        assert!(message.contains("event.tick"), "{reply}");
        // 调度端形状不破：summary 字段仍在（一次往返 = 步回执 ⊆ 摘要）。
        assert!(reply.get("summary").is_some(), "{reply}");
        assert_eq!(reply["summary"]["ok"], json!(true), "{reply}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn daemon_resumes_an_existing_run_instead_of_resetting_it() {
        let dir = case_dir("resume");
        let (mod_dir, runs_root) = (dir.join("mod"), dir.join("runs"));
        write_daemon_mod(&mod_dir);

        // 第一段：新局跑一步（ticks=1），然后丢弃进程内状态。
        {
            let args = daemon_args(&mod_dir, &runs_root, "d3");
            let mut session = Session::open(&args).expect("新局开局");
            let step = call(
                &mut session,
                &args,
                json!({"op": "dispatch", "event": "event.tick"}),
            );
            assert!(narrative_text(&step).contains("tick=1"), "{step}");
        }

        // 第二段：重启守护进程。目录里已有后端产物 → 自动续局（不重置），
        // 所以 ticks 从 1 累到 2 而不是从头开始。
        //
        // `resume` 这一行就是 `run_daemon` 的判定式（「已有后端产物即续局」）：
        // 它在 CLI 入口，不在 `handle_daemon_request` 里，所以单测只能复刻它。
        let reopen = Args {
            open_style: Some(if ygg_store::detect(&runs_root.join("d3")).any() {
                crate::OpenStyle::Open
            } else {
                crate::OpenStyle::New
            }),
            ..daemon_args(&mod_dir, &runs_root, "d3")
        };
        let mut resumed = Session::open(&reopen).expect("续局（重启守护进程不得清局）");
        let step = call(
            &mut resumed,
            &reopen,
            json!({"op": "dispatch", "event": "event.tick"}),
        );
        assert!(
            narrative_text(&step).contains("tick=2"),
            "重启必须续局（ticks 从 1 累到 2）：{step}"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}

// ============================================================================
// §r5 单发回执文本渲染（缺省 text / --raw 唯一旗标）
// ============================================================================

#[cfg(test)]
mod receipt_text_tests {
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
}
