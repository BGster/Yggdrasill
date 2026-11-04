//! 持久化端口：run 目录里**状态事实**的读写面（`RunStore` trait + 两个后端）。
//!
//! # 为什么有这个 crate
//!
//! 宿主 CLI（`crates/ygg-sandbox/examples/agent.rs`）原来把「存档 / 事件日志 /
//! inbox / 位移 / 重置」全部写死在文件操作里。要让第二个后端（sqlite）可选接入，
//! 必须先把这个面**收敛成一个端口**：既有的 JSON 文件行为逐字节不变，新的后端
//! 只实现同一个 trait。本 crate 因此只承接「状态事实」，**不承接**任何宿主协议
//! （`run.json` / `settlement.json` / `coordination.*` / `decide-input.json` 都留
//! 在宿主：它们是跨进程共识与报告面，两后端必须完全一致，搬进来只会把互斥一起
//! 搬走）。
//!
//! # 三条硬约束（读代码前先记住）
//!
//! 1. **trait 里没有锁，也没有「取锁」的参数**。所有方法都约定「调用方已持有
//!    run 锁」（宿主的 `coordination.lock`）。锁同时守着 `coordination.json` 与
//!    `decide-input.json`，属跨切面；端口只承接「锁内被调用」这一前提。
//! 2. **端口不取时钟**。`timestamp` / `Tick` 一律由宿主传入——否则黄金样本测试
//!    永远对不上（字节面必须可重放）。
//! 3. **JSON 后端的字节就是兼容面**：紧凑 JSON、键序字典序、`autosave.json`
//!    无尾随换行、`events.jsonl` 每行以 `\n` 结尾。sqlite 后端只承诺**语义**
//!    等价（`steps()` 反序列化后逐字段相等），不产这些文件。
//!
//! # 两个后端对崩溃的承诺不同（测试必须分列断言）
//!
//! - JSON：先追加事件行、再原子替换存档（两步）——可以只完成前一步，于是
//!   「事件有第 N 行、存档还是第 N-1 版」是**允许**的（弱承诺）；
//! - sqlite：单事务写 `step` + `snapshot`（必全或无），不存在半提交（强承诺）。
//!
//! 混用两个后端**一律 fail-closed**（见 [`resolve`]）：同目录同时有 JSON 产物与
//! sqlite 库时无法判定谁是权威，宿主不会替你删证据。
//!
//! # 用法
//!
//! ```no_run
//! use std::path::Path;
//! use ygg_store::{open, Backend, RunStore, StepDraft, Tick};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let run_dir = Path::new("/tmp/runs/demo");
//! let tick = Tick::at(1_700_000_000, 0);
//! let mut store = open(Backend::Json, run_dir, tick)?;
//! let committed = store.commit_step(StepDraft {
//!     event: "event.start",
//!     payload: "0",
//!     ok: true,
//!     narrative_count: 1,
//!     timestamp: tick.secs,
//!     ending_line: None,
//!     save: Some(b"{}"),
//! })?;
//! assert_eq!(committed.seq, store.next_seq()? - 1);
//! # Ok(())
//! # }
//! ```

use std::fmt;
use std::path::Path;

pub mod json;
#[cfg(feature = "sqlite")]
pub mod sqlite;

pub use json::{JsonFileStore, AUTOSAVE_FILE, EVENTS_FILE, INBOX_CURSOR_FILE, INBOX_FILE};
#[cfg(feature = "sqlite")]
pub use sqlite::{SqliteStore, SQLITE_FILE};

/// sqlite 后端固定文件名（后端探测用；与 [`SQLITE_FILE`] 同源）。
pub const SQLITE_FILE_NAME: &str = "run.sqlite";

// ============================================================================
// §后端标识
// ============================================================================

/// 存储后端。`run.json` 的 `store` 字段是**权威**，`--store` 旗标只是开局断言。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// 既有默认后端：`autosave.json` + `events.jsonl` + `inbox.jsonl`。
    Json,
    /// 可选后端：`<run-dir>/run.sqlite`（WAL）。
    Sqlite,
}

impl Backend {
    /// 全部取值（用法提示 / 旗标解析共用一份，避免各处硬编码）。
    pub const NAMES: [&'static str; 2] = ["json", "sqlite"];

    /// `run.json` 的 `store` 字段值 / `--store` 取值。
    pub fn as_str(self) -> &'static str {
        match self {
            Backend::Json => "json",
            Backend::Sqlite => "sqlite",
        }
    }

    /// 解析 `run.json` 的 `store` 字段或 `--store` 取值；未知串 → `None`。
    pub fn parse(text: &str) -> Option<Self> {
        Self::NAMES
            .iter()
            .copied()
            .find_map(|name| match (name, text) {
                ("json", "json") => Some(Backend::Json),
                ("sqlite", "sqlite") => Some(Backend::Sqlite),
                _ => None,
            })
    }
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ============================================================================
// §数据形状（全部不含锁、不含路径）
// ============================================================================

/// 一步的**可持久化事实**（`events.jsonl` 一行 / sqlite `step` 一行）。
///
/// 字段名与 `events.jsonl` 的键一一对应（字节兼容面）；`ending_line` 仅结局行有值。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepRecord {
    /// 代内 0 基（= 该代已提交步数 - 1）。
    pub seq: u64,
    pub event: String,
    /// 原样字符串（不是解析后的 JSON）：宿主传什么就存什么。
    pub payload: String,
    pub ok: bool,
    pub narrative_count: usize,
    /// unix 秒，**宿主提供**。
    pub timestamp: u64,
    pub ending: bool,
    pub ending_line: Option<String>,
}

/// 一次提交的输入。`seq` 由后端在同一临界区内分配（不是宿主传入）。
pub struct StepDraft<'a> {
    pub event: &'a str,
    pub payload: &'a str,
    pub ok: bool,
    pub narrative_count: usize,
    /// unix 秒，**宿主提供**（后端绝不取时钟）。
    pub timestamp: u64,
    pub ending_line: Option<&'a str>,
    /// `ok=false` 时为 `None`（失败步不落存档）；`Some` 的字节 = 存档原文。
    pub save: Option<&'a [u8]>,
}

/// 一次提交的结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Committed {
    /// 本次提交拿到的 seq（锁内分配）。
    pub seq: u64,
    /// **非致命**告警：事件行超大 / 事件行写入失败但步继续（承接宿主原有的
    /// 「事件写失败只告警、不中断」语义——下沉不得把失败语义悄悄变严）。
    pub warnings: Vec<String>,
}

/// reset 的结果（归档 = 证据，不删）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResetInfo {
    /// 重置前目录里是否已有旧产物（显式新局回执据此提示「已有旧产物」）。
    pub had_old: bool,
    /// 归档载体的人读标识：JSON = 归档文件路径；sqlite = `generation=<n>`。
    pub archive: Option<String>,
    /// 非致命告警（归档堆积提示 / 移除旧存档失败）。
    pub warnings: Vec<String>,
}

/// 事件日志的读取结果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StepLog {
    pub records: Vec<StepRecord>,
    /// 无法解析的行数（JSON 后端才可能非 0；sqlite 恒 0）。
    ///
    /// 注意：这些行**仍然占 seq**（`next_seq()` = 非空行数，不是 `records.len()`），
    /// 这是宿主一以贯之的分配口径，不要在端口里「修正」它。
    pub skipped: usize,
}

/// 宿主时钟（unix 秒 + 纳秒）。端口不取时钟，凡要落时间戳的地方都由宿主给定。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tick {
    pub secs: u64,
    pub nanos: u128,
}

impl Tick {
    pub fn at(secs: u64, nanos: u128) -> Self {
        Self { secs, nanos }
    }
}

// ============================================================================
// §错误
// ============================================================================

/// 端口错误。宿主一律 `map_err(|err| err.to_string())` 接进既有 `String` 错误链；
/// 文案沿用宿主的「{做什么}失败：{原因}」形态，用户可见行为不变。
#[derive(Debug)]
pub enum StoreError {
    /// IO 失败：`what` 是「写入 <path> 失败」这类短语，`source` 是底层错误。
    Io {
        what: String,
        source: std::io::Error,
    },
    /// 内容不合法（坏库 / 版本不符 / 声明冲突）。
    Corrupt { what: String, detail: String },
    /// 库由更新版本写出（schema_version 超出本构建支持）。
    Unsupported { detail: String },
    /// 后端裁决失败（混用 / 旗标与事实不一致）——**零写入**。
    Misconfigured { detail: String },
    /// 本构建未启用该后端（feature 未开启）。
    Unavailable { detail: String },
}

impl StoreError {
    pub fn io(what: impl Into<String>, source: std::io::Error) -> Self {
        StoreError::Io {
            what: what.into(),
            source,
        }
    }

    pub fn corrupt(what: impl Into<String>, detail: impl Into<String>) -> Self {
        StoreError::Corrupt {
            what: what.into(),
            detail: detail.into(),
        }
    }

    pub fn misconfigured(detail: impl Into<String>) -> Self {
        StoreError::Misconfigured {
            detail: detail.into(),
        }
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Io { what, source } => write!(f, "{what}：{source}"),
            StoreError::Corrupt { what, detail } => write!(f, "{what}：{detail}"),
            StoreError::Unsupported { detail } => f.write_str(detail),
            StoreError::Misconfigured { detail } => f.write_str(detail),
            StoreError::Unavailable { detail } => f.write_str(detail),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StoreError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

// ============================================================================
// §后端裁决（混用 / 断言不一致 → fail-closed，且零写入）
// ============================================================================

/// run 目录里的后端产物探针。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Products {
    /// 有 `autosave.json` 或 `events.jsonl`（既有 JSON 产物）。
    pub json: bool,
    /// 有 `run.sqlite`。
    pub sqlite: bool,
}

impl Products {
    pub fn any(self) -> bool {
        self.json || self.sqlite
    }
}

/// 探测 run 目录里的后端产物（只读 `Path::exists`，不开文件、不建目录）。
pub fn detect(run_dir: &Path) -> Products {
    Products {
        json: run_dir.join(AUTOSAVE_FILE).exists() || run_dir.join(EVENTS_FILE).exists(),
        sqlite: run_dir.join(SQLITE_FILE_NAME).exists(),
    }
}

/// 后端裁决：`run.json` 的 `store` 事实 > 产物探针 > `--store` 旗标 > 默认 json。
///
/// 严格按「权威性」排：`run.json` 是权威事实，`--store` 只是开局断言。允许的
/// 唯一推断是**旧局**（`run.json` 没有 `store` 键，或连 `run.json` 都没有）——
/// 此时按产物推断，既有的纯 JSON 目录一律落回 `json`（向后兼容）。
pub fn resolve(
    run_dir: &Path,
    declared: Option<Backend>,
    flag: Option<Backend>,
) -> Result<Backend, StoreError> {
    resolve_with(run_dir, declared, flag, detect(run_dir))
}

/// [`resolve`] 的可注入版本（测试用：不落盘也能裁决）。
pub fn resolve_with(
    run_dir: &Path,
    declared: Option<Backend>,
    flag: Option<Backend>,
    products: Products,
) -> Result<Backend, StoreError> {
    let where_ = run_dir.display();
    if products.json && products.sqlite {
        return Err(mixed(where_));
    }
    let probed = if products.sqlite {
        Some(Backend::Sqlite)
    } else if products.json {
        Some(Backend::Json)
    } else {
        None
    };

    if let Some(declared) = declared {
        // 声明了后端，但目录里是另一个后端的产物 → 混代（同一族错误，零写入）。
        if let Some(probed) = probed {
            if probed != declared {
                return Err(declared_conflicts_with_products(where_, declared, probed));
            }
        }
        if let Some(flag) = flag {
            if flag != declared {
                return Err(StoreError::Misconfigured {
                    detail: format!(
                        "本局已声明存储后端 `{declared}`（来自 {where_}/run.json 的 \"store\" 字段），\
                         与 --store {flag} 不一致——run.json 是权威事实，--store 只是开局断言。\
                         要用另一后端请开新局（换 --run-id）；本局未做任何写入。"
                    ),
                });
            }
        }
        return Ok(declared);
    }

    // 无声明：旧局（按产物推断）或新局（按旗标 / 默认 json 定型）。
    if let Some(flag) = flag {
        if let Some(probed) = probed {
            if probed != flag {
                return Err(flag_conflicts_with_products(where_, flag, probed));
            }
        }
        return Ok(flag);
    }
    Ok(probed.unwrap_or(Backend::Json))
}

/// 产物文件名（错误文案里点名，用户才知道该看哪两个文件）。
fn product_files(backend: Backend) -> &'static str {
    match backend {
        Backend::Json => "autosave.json / events.jsonl",
        Backend::Sqlite => "run.sqlite",
    }
}

/// `run.json` 声明了 `declared`，但目录里是 `probed` 的产物 → 混代。
fn declared_conflicts_with_products(
    where_: impl fmt::Display,
    declared: Backend,
    probed: Backend,
) -> StoreError {
    StoreError::Misconfigured {
        detail: format!(
            "本局已声明存储后端 `{declared}`（来自 {where_}/run.json 的 \"store\" 字段），\
             但目录里只有 `{probed}` 的产物（{}）——混代状态无法判定谁是权威，拒绝继续；\
             请人工确认后修正 run.json 或删除多余的那一份（宿主不自动删证据）；\
             本局未做任何写入。",
            product_files(probed)
        ),
    }
}

/// `--store <flag>` 与目录里已有的产物（`probed`）冲突 → 拒绝（旧局漏写事实的常见情形）。
fn flag_conflicts_with_products(
    where_: impl fmt::Display,
    flag: Backend,
    probed: Backend,
) -> StoreError {
    StoreError::Misconfigured {
        detail: format!(
            "{where_} 里是 `{probed}` 后端的产物（{}），与 --store {flag} 不一致——\
             存储后端由目录产物与 run.json 的 \"store\" 字段决定，--store 只是开局断言。\
             要用另一后端请开新局（换 --run-id）；本局未做任何写入。",
            product_files(probed)
        ),
    }
}

fn mixed(where_: impl fmt::Display) -> StoreError {
    StoreError::Misconfigured {
        detail: format!(
            "{where_} 同时存在 JSON 产物（{AUTOSAVE_FILE} / {EVENTS_FILE}）与 sqlite 库\
             （{SQLITE_FILE_NAME}）——混代状态无法判定谁是权威，拒绝继续；\
             请人工确认后删除多余的那一份（宿主不自动删证据）。"
        ),
    }
}

// ============================================================================
// §端口
// ============================================================================

/// run 目录的持久化端口。
///
/// **前置条件（全部方法）**：调用方已持有 run 锁（宿主的 `coordination.lock`）。
/// 本 trait 不含任何锁原语——锁同时守着 `coordination.json` / `decide-input.json`，
/// 属宿主的跨切面职责；端口只承接「锁内被调用」这一事实。
///
/// 锁序不变量（防死锁）：**恒为 宿主锁 → 后端锁**，绝不反向。
pub trait RunStore {
    /// 本实例的后端标识（`run.json` 的 `store` 字段值由它给出）。
    fn backend(&self) -> Backend;

    /// 定位串（回执 `autosave` 字段）：JSON = 存档文件路径；sqlite = `<db>#latest`。
    fn locator(&self) -> String;

    /// 该代已提交步数 = 下一个 seq（= 非空事件行数，含不可解析行）。
    ///
    /// 与 [`RunStore::commit_step`] **必须分开**：失败路径要报 seq，而提交已失败。
    fn next_seq(&self) -> Result<u64, StoreError>;

    /// 提交一步：事件行 + （仅成功步的）存档，**同一临界区**。
    ///
    /// JSON 后端弱承诺：事件行写失败只进 `warnings`（不中断）；存档写失败是硬错。
    fn commit_step(&mut self, step: StepDraft<'_>) -> Result<Committed, StoreError>;

    /// 最新存档原文（字节级；无档 → `None`）。
    fn latest_save(&self) -> Result<Option<Vec<u8>>, StoreError>;

    /// 写一份存档（`s` 命令这类「不改状态但要落盘」的场景；不新增事件行）。
    fn set_save(&mut self, save: &[u8], tick: Tick) -> Result<(), StoreError>;

    /// 该代全部事件记录（`skipped` = 不可解析行数）。
    fn steps(&self) -> Result<StepLog, StoreError>;

    /// 新局重置：归档旧代 → 建新代 → 落初始存档。任一步失败即 `Err`（fail-closed）。
    fn reset(
        &mut self,
        initial_save: &[u8],
        reason: &str,
        tick: Tick,
    ) -> Result<ResetInfo, StoreError>;

    /// inbox 追加一行（记录原文，不解析；坏行也要能存）。
    fn inbox_push(&mut self, record: &serde_json::Value, tick: Tick) -> Result<(), StoreError>;

    /// inbox 全部**非空**行原文（按写入序）。解析与路由是宿主语义，端口不碰。
    fn inbox_lines(&self) -> Result<Vec<String>, StoreError>;

    /// inbox 非空行数（审计口径，与 [`RunStore::inbox_lines`] 一致）。
    fn inbox_len(&self) -> Result<usize, StoreError>;

    /// 已消费位移（缺失 / 损坏 = 0：重放靠 Mod 侧幂等去重兵底）。
    fn inbox_cursor(&self) -> Result<usize, StoreError>;

    /// 前移消费位移（仅 dispatch + 存档成功后调）。
    fn set_inbox_cursor(&mut self, consumed: usize, tick: Tick) -> Result<(), StoreError>;

    /// 清理崩溃残留（JSON：陈旧的 `autosave.json.tmp.*`；sqlite：no-op）。
    ///
    /// sqlite 实现**绝不**触碰 `-wal` / `-shm` / `-journal`：那是库的活文件，
    /// 「清理」在语义上等于破坏事务。
    fn cleanup_stale(&mut self, stale_ms: u128) -> Vec<String>;
}

/// 按后端打开端口（宿主唯一入口）。
pub fn open(backend: Backend, run_dir: &Path, tick: Tick) -> Result<Box<dyn RunStore>, StoreError> {
    match backend {
        Backend::Json => Ok(Box::new(JsonFileStore::open(run_dir)?)),
        Backend::Sqlite => open_sqlite(run_dir, tick),
    }
}

#[cfg(feature = "sqlite")]
fn open_sqlite(run_dir: &Path, tick: Tick) -> Result<Box<dyn RunStore>, StoreError> {
    Ok(Box::new(SqliteStore::open(run_dir, tick)?))
}

#[cfg(not(feature = "sqlite"))]
fn open_sqlite(_run_dir: &Path, _tick: Tick) -> Result<Box<dyn RunStore>, StoreError> {
    Err(StoreError::Unavailable {
        detail: "本构建未启用 sqlite 后端（sqlite 是编译期可选项，需要以 \
                 `--features store-sqlite` 构建宿主）"
            .to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dir(label: &str) -> PathBuf {
        Path::new("/tmp").join(format!("ygg-store-resolve-{label}"))
    }

    /// 既有的纯 JSON 目录（无 `store` 字段）必须推断为 json——向后兼容的判据。
    #[test]
    fn legacy_json_products_infer_json() {
        let products = Products {
            json: true,
            sqlite: false,
        };
        let backend = resolve_with(&dir("legacy"), None, None, products).expect("旧局必须可推断");
        assert_eq!(backend, Backend::Json);
        // 旧局 + 显式 --store sqlite：产物证明是 json → 混用族错误（零写入）。
        let err = resolve_with(&dir("legacy"), None, Some(Backend::Sqlite), products).unwrap_err();
        assert!(matches!(err, StoreError::Misconfigured { .. }));
    }

    /// 空目录（新局）：默认 json；给旗标就按旗标定型。
    #[test]
    fn empty_dir_takes_flag_or_default() {
        let products = Products::default();
        assert_eq!(
            resolve_with(&dir("new"), None, None, products).unwrap(),
            Backend::Json
        );
        assert_eq!(
            resolve_with(&dir("new"), None, Some(Backend::Sqlite), products).unwrap(),
            Backend::Sqlite
        );
    }

    /// `run.json` 的 `store` 是权威：与旗标不一致 → 拒绝，且错误串四个要点齐全。
    #[test]
    fn declared_fact_wins_and_flag_mismatch_is_rejected() {
        let products = Products::default();
        let err = resolve_with(
            &dir("declared"),
            Some(Backend::Sqlite),
            Some(Backend::Json),
            products,
        )
        .unwrap_err();
        let text = err.to_string();
        for fragment in ["sqlite", "json", "run.json", "未做任何写入"] {
            assert!(text.contains(fragment), "错误串缺少「{fragment}」：{text}");
        }
        // 一致时放行。
        assert_eq!(
            resolve_with(
                &dir("declared"),
                Some(Backend::Sqlite),
                Some(Backend::Sqlite),
                products
            )
            .unwrap(),
            Backend::Sqlite
        );
    }

    /// 同目录两套产物 → 混用 fail-closed，文案点名两件产物。
    #[test]
    fn mixed_products_are_rejected() {
        let products = Products {
            json: true,
            sqlite: true,
        };
        let text = resolve_with(&dir("mixed"), None, None, products)
            .unwrap_err()
            .to_string();
        for fragment in [AUTOSAVE_FILE, SQLITE_FILE_NAME] {
            assert!(text.contains(fragment), "错误串缺少「{fragment}」：{text}");
        }
    }
}
