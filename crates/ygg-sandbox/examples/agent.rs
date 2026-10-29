//! Agent harness CLI（M-Agent 泳道 B）：把内核当 **Agent 的执行骨架**用。
//!
//! Agent 主导驱动：Agent 调本 CLI 派发动作，CLI 回执「处境叙事 + 可选动作」，
//! 每步自动存档，mod 改动热载，局末产出结算报告。全部策略在宿主侧，内核
//! 零改动（`KernelFacade` 的 `dispatch` 信封即唯一接驳面——与
//! `adventure.rs` / `playtest.rs` 同构）。
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
//! ```
//!
//! run 目录产物（`<runs-root>/<run-id>/`，默认 `./runs/<run-id>/`）：
//!
//! | 文件 | 内容 |
//! | --- | --- |
//! | `autosave.json` | 最新存档（= `save` 信封内嵌 save 对象的原样字节，可直接作 `load` payload） |
//! | `events.jsonl` | 每派发一行：`{seq, event, payload, ok, narrative_count, timestamp, ending, ending_line?}` |
//! | `run.json` | 宿主侧运行元信息：`{run_id, mod, mod_signature, started_at, updated_at}`（结算取 `mod`；`mod_signature` 供跨进程热载比对） |
//! | `settlement.json` | 局末结算报告（仅 `--settle` 写出） |
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
use std::process::exit;
use std::time::{SystemTime, UNIX_EPOCH};

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

const AUTOSAVE_FILE: &str = "autosave.json";
const EVENTS_FILE: &str = "events.jsonl";
const META_FILE: &str = "run.json";
const SETTLEMENT_FILE: &str = "settlement.json";

const MOD_SOURCE_FILE: &str = "mod.lua";
const MANIFEST_FILE: &str = "manifest.json";
const PACK_FILE: &str = "pack.json";

fn main() {
    let args = parse_args();
    let code = if args.settle {
        run_settle(&args)
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

struct Args {
    mod_dir: Option<PathBuf>,
    run_id: String,
    runs_root: PathBuf,
    resume: bool,
    settle: bool,
    watch: bool,
    event: Option<String>,
    payload: String,
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
    if !settle && mod_dir.is_none() {
        usage_error("缺少 --mod <mod目录>（仅 --settle 可省略）");
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
  agent --mod <mod目录> --run-id <标识> [--resume] [--watch]    交互模式
  agent --mod <mod目录> --run-id <标识> --event <名称> [--payload <JSON>] [--resume]
                                                                单发模式（Agent 脚本化）
  agent --run-id <标识> --settle                                局末结算

选项:
  --mod <目录>       Mod 目录（五件套：pack.json / manifest.json / mod.lua /
                     schemas/*.json / data/*.json；--settle 可省略）
  --run-id <标识>    本局标识：产物落 <runs-root>/<标识>/（必填）
  --runs-root <目录> runs 根目录（默认 ./runs）
  --resume           从 autosave.json 续局（缺存档则按新局开局）
  --watch            交互模式下每步检查 mod 目录 mtime 变化并热载
  --event <名称>     单发模式：派发一个事件后退出（stdout 单行 JSON 回执）
  --payload <JSON>   事件 payload（默认 \"0\"；必须 JSON 标量）
  --settle           读 autosave + events.jsonl 写 settlement.json
  -h, --help         本帮助";
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

struct Session {
    run_id: String,
    mod_dir: PathBuf,
    mod_label: String,
    run_dir: PathBuf,
    autosave_path: PathBuf,
    events_path: PathBuf,
    meta_path: PathBuf,
    kernel: KernelFacade,
    /// 下一条事件记录的序号（续局 = 既有行数，保证 seq 单调且可与日志对账）。
    seq: u64,
    /// 装载时的 mod 目录签名（热载比对基线；每次成功/失败尝试后前移）。
    signature: Vec<FileStamp>,
    /// 本次回执待携带的宿主侧提示（热载 / 自动存档 / 重置等）。
    notices: Vec<String>,
    /// 本次派发前的热载结果（一次性；派发收口后清空）。
    reload: Option<Reload>,
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
            mod_label: mod_dir.display().to_string(),
            autosave_path: run_dir.join(AUTOSAVE_FILE),
            events_path: run_dir.join(EVENTS_FILE),
            meta_path: run_dir.join(META_FILE),
            run_dir,
            kernel,
            seq: 0,
            signature,
            notices: Vec::new(),
            reload: None,
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
        Ok(())
    }

    /// `state` 诊断信封（不推进状态）。
    fn state_envelope(&mut self) -> Json {
        parse_envelope(&self.kernel.dispatch("state", ""))
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

    let receipt = session.dispatch_event(&event, &args.payload);
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
// §交互模式 数字选项 / event / s / l / i / config / q（现场感：每步打印处境）
// ============================================================================

fn run_interactive(args: &Args) -> i32 {
    let mut session = match Session::open(args) {
        Ok(session) => session,
        Err(message) => {
            eprintln!("error: {message}");
            return 1;
        }
    };

    println!("== Yggdrasill Agent Harness · 交互模式 ==");
    println!("mod      : {}", session.mod_label);
    println!("run 目录 : {}", session.run_dir.display());
    println!("命令：数字=选择 / event <名称> [JSON] / s=存档 / l=读档 / i=状态 / config / q=退出");
    if args.watch {
        println!("热载     : 开启（每步检查 mod 目录 mtime）");
    }
    for notice in std::mem::take(&mut session.notices) {
        println!("⚠ {notice}");
    }
    println!();

    // 新局自动派发入口事件（与 adventure.rs 同为「开局即给处境」）；
    // 续局只回显当前处境（pendingChoice 随存档往返），不推进剧情。
    let mut current = if session.resumed {
        let state = session.state_envelope();
        json!({
            "ok": true,
            "event": "resume",
            "narrative": [],
            "pendingChoice": state["pendingChoice"].clone(),
            "autosave": session.autosave_path.display().to_string(),
        })
    } else {
        session.dispatch_event(ENTRY_EVENT, DEFAULT_PAYLOAD)
    };
    let mut fresh = true;

    // stdin 只锁一次（与 adventure.rs 同纪律：锁内单循环，无递归重入）。
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();

    loop {
        print_receipt(&current, fresh);
        print_prompt(&current);
        fresh = false;

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
                    let state = session.state_envelope();
                    current = json!({
                        "ok": true,
                        "event": "load",
                        "narrative": ["（读档完成，进度已恢复。）"],
                        "pendingChoice": state["pendingChoice"].clone(),
                        "autosave": session.autosave_path.display().to_string(),
                    });
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
        println!("—— 本局已到结局（q 退出后可 --settle 结算）——");
    }
}

/// 处境提示面：待决选择（编号选项）或无选择时的可用命令。
fn print_prompt(receipt: &Json) {
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
    print!("> ");
    let _ = io::stdout().flush();
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
        "事件日志 : {}（已记录 {} 条，下一条 seq={}）",
        session.events_path.display(),
        session.seq,
        session.seq
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

fn run_settle(args: &Args) -> i32 {
    let run_dir = args.runs_root.join(&args.run_id);
    let autosave_path = run_dir.join(AUTOSAVE_FILE);
    let events_path = run_dir.join(EVENTS_FILE);

    // ① 终局存档（唯一状态事实源；缺档即无法结算）。
    let save_text = match fs::read_to_string(&autosave_path) {
        Ok(text) => text,
        Err(err) => {
            eprintln!(
                "error: 读取 {} 失败：{err}（先跑一局再结算）",
                autosave_path.display()
            );
            return 1;
        }
    };
    let save: Json = match serde_json::from_str(&save_text) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("error: {} 不是合法 JSON：{err}", autosave_path.display());
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

    // ⑤ 结论：无失败步 ∧ 有事件 ∧ 达结局（未达结局 = 局未收口，ok=false）。
    let ok = !events.is_empty() && failed == 0 && reached_ending;
    if failed > 0 {
        errors.push(format!("{failed} 条事件记录为失败步"));
    }
    if !reached_ending {
        errors.push("未见结局叙事行：本局未收口".to_owned());
    }
    if events.is_empty() {
        errors.push("事件日志为空：本局无任何派发记录".to_owned());
    }

    let report = json!({
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
        return 1;
    }
    print_settlement(&report, &settlement_path, ending_line.as_deref(), failed);
    if ok {
        0
    } else {
        1
    }
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
