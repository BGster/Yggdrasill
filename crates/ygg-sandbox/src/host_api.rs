//! S9 host API（D-S9-1..D-S9-4）：`ygg.*` 完整面 + 收集-回放桥 + 装载/运行期分相。
//!
//! - **生命周期与分相**（D-S9-1）：[`install`] 在**装载期**安装完整 `ygg` 面
//!   （`on / emit / narrate / choice / kv.* / entity.*`）；[`HostApi::seal`]
//!   在装载结束后把 `ygg.on` **原位替换**为可读报错桩——运行期注册禁令
//!   （D-B4 的运行时侧权威执行面）。无 mlua Scope、无线程局部存储或静态可变
//!   全局：全部共享状态经 `Rc<RefCell<HostState>>` 单线程持有。
//! - **收集-回放桥**（D-S9-2）：`ygg.emit / narrate / choice` 在 Lua 执行期间
//!   只向 [`HostState::calls`] **按序追加** [`HostCall`]；[`LuaHandler`]
//!   （core `EventHandler` 反转面实现）在每次 handler 调用前清空调用缓冲，
//!   以事件 payload（core `Value` → mlua `Value` marshal）调用装载期存下的
//!   Lua 函数，随后把缓冲**按原序回放**进 sink（emit→`sink.emit`、
//!   narrate→`sink.narrate`、choose→`sink.choose`）。
//!   **等价性论证**：回放序 = 追加序 = Lua 执行序，故 emit / narrate / choose
//!   三类输出的**相对顺序**在桥前后逐项保真——downstream handler 观察到的
//!   FIFO 序、叙述序与 pending_choice 与「直接直通 sink」的实现逐项一致。
//!   KV / 实体操作是**有副作用的即席状态写**，不走回放：经 `Rc<RefCell>`
//!   立即调用 core 把关方法，副作用时点 = Lua 执行时点（其相对输出序在
//!   handler 内部由构造保证；门拒绝如非有限值以可读 mlua 错误浮现）。
//! - **sentinel 机制**（D-S9-3）：`ygg.choice` 记录 `Choose` 后以
//!   install 时从内核种子派生的 **per-kernel sentinel**（[`HostState::sentinel`]，
//!   不可预测 token）错误终止当前 Lua 执行（后续代码不得运行——
//!   choice-as-state：挂起即数据）；桥把首行恰好等于 sentinel 的 mlua 错误识别为
//!   **正常终止**（继续回放缓冲），其余错误包成 core [`HandlerError`]。
//!   Mod 面无法预测该 token（错误伪造只会成为普通错误浮出，不会被哨兵吞掉）。
//! - **choice 前置输出丢弃的运行期警告**（M0 迭代 D1）：`ygg.choice` 被调用时
//!   统计调用缓冲中已有的 `Emit` 条目（这些派生事件将因 `pending_choice`
//!   终结清空 FIFO 而被语义丢弃）；replay 后若计数非零，记一条人读警告到
//!   [`HostState::warning`]（每次 dispatch 至多一条，首个胜出，由驱动经
//!   [`HostApi::take_dispatch_warning`] drain 进信封加性 `warnings` 键）。
//!   只计 `Emit` 不计 `Narrate`：narrate 直进叙述缓冲、不随 FIFO 丢弃——
//!   把它计入警告会与信封 `narrative` 字段自相矛盾。静态 ygg-lint 仍挂账 M1。
//! - **注册序**（D-S9-4）：`ygg.on` 调用序 = `Vec` 追加序；
//!   [`HostApi::register_into`] 在 seal 后按该序逐条
//!   `HandlerRegistry::register`（每条一个 [`LuaHandler`]），注册序即派发序。
//! - **结构化 meta 通道**（TD1）：`ygg.meta(key, value)` 把**结构化数据**写进
//!   [`HostState::meta`]（`BTreeMap<String, MetaValue>`），与叙事面完全分离——
//!   Mod 不必再把人读文本当结构化传输（旧 `【摘要】<JSON>` 前端耦合已删除）。
//!   三条纪律：① 不走 [`HostState::calls`] 缓冲（不回放、不改 core trait），
//!   驱动在转换结束后经 [`HostApi::take_meta`] 取走并拼进信封 `meta` 键；
//!   ② key 必须是长度 1..=`META_KEY_MAX_BYTES` 的字符串（非空、可读报错）；
//!   ③ value 只允许标量 / 稠密序列 / 字符串键表（深度 `META_MAX_DEPTH` 封顶），
//!   非有限值（NaN/±∞）fail-fast 拒绝——与 `ygg.emit` 同纪律。
//!   **meta 是瞬态回执面**：不入 canonical store / 存档 / 状态哈希（驱动侧
//!   每次 dispatch 入口 clear、转换后 take，生命周期不跨事件）。
//!   **值类型取 `serde_json::Value` 而非 core `Value`**：core `Value` 只有四个
//!   标量，装不下回执需要的对象/序列（`meta.summary.conflicts` 就是数组）；
//!   标量 leaf 仍走 [`value_from_lua`]（同一套类型门），容器内层用
//!   `BTreeMap` 序 + serde_json 默认字典序 Map 保证确定性字节。
//! - **payload 双向 marshal**：Integer↔`Int`、Number↔`Float`、String↔`Str`、
//!   Boolean↔`Bool`；Lua→core 遇 NaN/±∞ 由 core 门拒绝（可读 mlua 错误）、
//!   `-0.0` 由 core 规范化；`nil` 仅在 get 类miss 上作为 Lua `nil` 返回；
//!   table / function / 其他类型 → 可读 `unsupported payload type: {lua_type}`。
//!   `ygg.choice` 的 options 必须是字符串的 Lua 序列，否则可读报错。

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use mlua::{Error, Function, Lua, Table, Value as LuaValue};
use serde_json::{Map as JsonMap, Value as MetaValue};
use ygg_core::bus::{EventHandler, HandlerError, HandlerRegistry, NarrationSink, PendingChoice};
use ygg_core::entity::{
    like_shape, normalize_f64, CompareOp, EntityStore, IsTest, Order, Predicate, Selector, Value,
};
use ygg_core::event::Event;
use ygg_core::kv::KvStore;
use ygg_schema::validate::sha256_hex;

use crate::inject::splitmix64;

/// 运行期注册禁令的可读报错文案（seal 后 `ygg.on` 桩与 sealed 双检共用）。
const SEALED_MESSAGE: &str = "ygg.on is sealed: runtime handler registration is forbidden \
 (handlers may only be registered during the load phase)";

/// `ygg.meta` 键长上限（字节）。键是**回执字段名**，不是数据面通道——
/// 上限防「把人读内容塞键名」的滥用回流。
const META_KEY_MAX_BYTES: usize = 64;

/// `ygg.meta` 值嵌套深度上限（含顶层表：1 层）。超限可读报错而非爆栈。
const META_MAX_DEPTH: usize = 8;

/// `ygg.tool.call` 的工具 ID 字节上限。
const TOOL_ID_MAX_BYTES: usize = 64;

/// 宿主侧同步工具执行器注入面（Mod 的 `ygg.tool.call(tool_id, args_table)` 桥接）。
pub trait HostToolRunner {
    fn call(
        &mut self,
        tool_id: &str,
        args: &BTreeMap<String, String>,
    ) -> Result<BTreeMap<String, String>, String>;
}

// ============================================================================
// §文件挂载桥的注入面（`ygg.fs.*` 的 `mounts/` 根）
// ============================================================================

/// 挂载点里一项的类型（= `ygg.fs.stat` / `ygg.fs.list` 回执里的 `kind` 取值源）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountKind {
    File,
    Dir,
    /// 不存在（只出现在 `stat` 的回执里，`as_str()` 是空串）。
    Missing,
}

impl MountKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MountKind::File => "file",
            MountKind::Dir => "dir",
            MountKind::Missing => "",
        }
    }
}

/// 一次挂载写的回执（`ygg.fs.write` 的 `{ path, bytes, sha256 }`）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountWritten {
    pub path: String,
    pub bytes: usize,
    pub sha256: String,
}

/// 一个挂载路径的探测结果（`ygg.fs.stat` 的 `{ exists, bytes, sha256, kind }`）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountStat {
    pub exists: bool,
    pub bytes: usize,
    pub sha256: String,
    pub kind: MountKind,
}

/// `ygg.fs.list` 的一行（`{ name, path, bytes, sha256, kind }`；`sha256` 恒为 `""`）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountEntry {
    pub name: String,
    pub path: String,
    pub bytes: usize,
    pub kind: MountKind,
}

/// 宿主侧文件挂载桥注入面（与 [`HostToolRunner`] 同型：沙箱只转发，零 IO）。
///
/// 实现方是持盘面（宿主 CLI / 测试桥），错误串必须是 `YGG-FS-*` 首段的人读
/// 文案——沙箱原样包成 Lua 运行时错误，不重新拼码。
pub trait MountFs {
    /// 读一个挂载文件（`miss → None`，与 `artifacts/` 面同口径）。
    fn read(&self, path: &str) -> Result<Option<Vec<u8>>, String>;
    /// 写一个挂载文件（写穿磁盘；成功即已落盘）。
    fn write(&mut self, path: &str, content: &[u8]) -> Result<MountWritten, String>;
    /// 探测一个挂载路径。
    fn stat(&self, path: &str) -> Result<MountStat, String>;
    /// 列一个挂载目录（不递归、不算摘要）。
    fn list(&self, path: &str) -> Result<Vec<MountEntry>, String>;
}

/// 沙箱侧挂载路径的前缀（与 `ygg_store::mounts::MOUNT_PATH_PREFIX` 同值；
/// 沙箱是零依赖面，不能引用 store 的常量）。
pub const MOUNT_PREFIX: &str = "mounts/";

/// 是否落在挂载面（`mounts` 本身或 `mounts/...`）。
fn is_mount_path(path: &str) -> bool {
    path == "mounts" || path.starts_with(MOUNT_PREFIX)
}

/// 未挂载桥时的统一表述（码在首段：`nil` 与「文件不存在」不可分辨，静默降级
/// 会把「宿主没挂」读成「文件没了」）。
fn unmounted_message(what: &str, path: &str) -> String {
    format!(
        "{}: 宿主未挂载文件桥，`{what}(\"{path}\")` 不可用（旧面 `artifacts/` 不受影响）",
        ygg_fs_code::UNMOUNTED
    )
}

/// 沙箱侧用到的两个错误码（与宿主侧 `ygg_store::mounts::codes` 同值）。
pub mod ygg_fs_code {
    /// 宿主未挂载文件桥。
    pub const UNMOUNTED: &str = "YGG-FS-UNMOUNTED";
    /// 路径首段既不是 `artifacts/` 也不是 `mounts`。
    pub const JAIL: &str = "YGG-FS-JAIL";
}

/// 宿主相位：`ygg.tool.call` 仅允许在转换期（`Phase::Transition`）调用。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Loading,
    Idle,
    Transition,
}

/// 宿主侧状态句柄（S9 测试 / S10 驱动的注入面）。
pub struct HostHandles {
    pub kv: Rc<RefCell<KvStore>>,
    pub entities: Rc<RefCell<EntityStore>>,
}

impl HostHandles {
    /// 构造句柄（供需要同时挂载或替换工具执行器的宿主使用）。
    pub fn new(kv: Rc<RefCell<KvStore>>, entities: Rc<RefCell<EntityStore>>) -> Self {
        Self { kv, entities }
    }
}

/// 单条待回放的 host 调用（D-S9-2 收集面的载体，按追加序回放）。
pub enum HostCall {
    /// `ygg.emit(name, payload)` → 回放为 `sink.emit(Event::derived(..))`。
    Emit { name: String, payload: Value },
    /// `ygg.narrate(line)` → 回放为 `sink.narrate(line)`。
    Narrate(String),
    /// `ygg.choice(prompt, options)` → 回放为 `sink.choose(..)`（first-wins 在驱动层）。
    Choose(PendingChoice),
}

// ============================================================================
// §VFS（`ygg.fs` 的内存态：装载期写入面与转换期读面共用同一个 Rc）
// ============================================================================

/// VFS 路径前缀（与 `ygg_store::ARTIFACT_PREFIX` 同值：VFS 的键就是制品的
/// run 内相对路径，宿主 flush 时直接当 `RunStore::artifact_put` 的入参）。
pub const VFS_PREFIX: &str = "artifacts/";
/// VFS 路径字节上限（与 `ygg_store::ARTIFACT_PATH_MAX_BYTES` 同值）。
pub const VFS_PATH_MAX_BYTES: usize = 128;
/// VFS 单件内容字节上限（与 `ygg_store::ARTIFACT_CONTENT_MAX_BYTES` 同值）。
pub const VFS_CONTENT_MAX_BYTES: usize = 256 * 1024;

/// 一件 VFS 文件（内容 + 摘要；摘要与 `ygg-store` 的制品面同编码）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VfsFile {
    pub bytes: Vec<u8>,
    pub sha256: String,
}

/// `ygg.fs` 的内存态 VFS。
///
/// 三条确定性纪律（改动前先读）：
/// 1. **`BTreeMap`，不是 `HashMap`**：`ygg.fs.list()` 的遍历序 = 键序 = 字典序，
///    同输入同字节（确定性路径一律有序容器——与 `ygg_core` 的 KV / 实体同纪律）。
/// 2. **`revision` 只增**：每次成功写 +1；宿主据此判断「有没有新东西要 flush」。
/// 3. **`dirty` 是落盘水位**：写完即入集合，宿主 `take_dirty()` 排空并写进
///    `RunStore::artifact_put`。它随 dispatch 级 savepoint 基线一起克隆/还原，
///    所以失败 dispatch 里写的文件**不会**漏到盘上（见 `driver::cmd_event`）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VfsStore {
    files: BTreeMap<String, VfsFile>,
    dirty: BTreeSet<String>,
    revision: u64,
}

impl VfsStore {
    /// 读一件文件（miss = `None`）。
    pub fn get(&self, path: &str) -> Option<&VfsFile> {
        self.files.get(path)
    }

    /// 全量条目（`BTreeMap` 序 = 字典序；`ygg.fs.list` 与宿主 flush 面共用）。
    pub fn entries(&self) -> impl Iterator<Item = (&str, &VfsFile)> + '_ {
        self.files.iter().map(|(path, file)| (path.as_str(), file))
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// 写代数（每次成功 `put` +1；预载与排空 **不**动它）。
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Lua 写面：先过路径与大小门，再算摘要、入表。
    ///
    /// 报错串以 `ygg.fs.write 拒绝：` 开头（可 grep），原因由门给。
    pub fn put(&mut self, path: &str, content: &[u8]) -> Result<VfsFile, String> {
        validate_vfs_path(path).map_err(|reason| format!("ygg.fs.write 拒绝：{reason}"))?;
        if content.len() > VFS_CONTENT_MAX_BYTES {
            return Err(format!(
                "ygg.fs.write 拒绝：`{path}` 的内容 {} 字节超过上限 {VFS_CONTENT_MAX_BYTES} 字节（256 KiB）",
                content.len()
            ));
        }
        let file = VfsFile {
            bytes: content.to_vec(),
            sha256: sha256_hex(content),
        };
        self.files.insert(path.to_owned(), file.clone());
        self.dirty.insert(path.to_owned());
        self.revision += 1;
        Ok(file)
    }

    /// 宿主预载（开局从 `RunStore::artifact_get` 回灌，或 `--artifact-put`）：
    /// **不校验内容、不入 dirty、不动 revision**——它不是本次会话产生的写入。
    ///
    /// 路径仍过门：预载来源（盘上的制品面）与调用方入参都不该有机会把
    /// `../` 之类的键塞进来，而 `ygg.fs.list` 会把它原样回执出去。
    pub fn load(&mut self, path: &str, content: &[u8], sha256: &str) {
        if validate_vfs_path(path).is_err() {
            return; // 静默跳过：预载面的错路径不是致命错误（不建键）
        }
        self.files.insert(
            path.to_owned(),
            VfsFile {
                bytes: content.to_vec(),
                sha256: sha256.to_owned(),
            },
        );
    }

    /// 排空并返回未落盘条目（`BTreeMap` 序）。宿主 flush 成功后调用。
    pub fn take_dirty(&mut self) -> Vec<(String, VfsFile)> {
        let paths: Vec<String> = self.dirty.iter().cloned().collect();
        self.dirty.clear();
        let mut out = Vec::with_capacity(paths.len());
        for path in paths {
            if let Some(file) = self.files.get(&path) {
                out.push((path, file.clone()));
            }
        }
        out
    }

    /// 把一批路径**重新标脏**（落盘中途失败后的回填）。
    ///
    /// 不动 `revision`、不动内容：没有新的写入发生，只是「这几件还没落盘」。
    /// 内容寻址让重试只重写相同字节，不会产生第二件制品。
    pub fn requeue_dirty(&mut self, paths: &[String]) {
        for path in paths {
            if self.files.contains_key(path) {
                self.dirty.insert(path.clone());
            }
        }
    }

    /// `ygg.fs.list()` 的回执形状（路径 / 字节数 / 摘要，`BTreeMap` 序）。
    pub fn snapshot(&self) -> Vec<(String, usize, String)> {
        self.files
            .iter()
            .map(|(path, file)| (path.clone(), file.bytes.len(), file.sha256.clone()))
            .collect()
    }
}

/// VFS 路径门（**本地、私有、fail-fast 面**）。规则与
/// `ygg_store::validate_artifact_path` **逐条相同**，却是另一份实现——这是
/// 有意的双门：`ygg-store` 是本 crate 的 **dev-dependency**，运行时无权调用
///（否则会把 `rusqlite` 拉进 wasm32）；而且两份门的角色不同：
///
/// | 门 | 位置 | 作用 |
/// | --- | --- | --- |
/// | 本函数 | Lua 调用点 | `ygg.fs.write("…/../x")` 当场可读报错，不等落盘 |
/// | `ygg_store::validate_artifact_path` | 端口内 | 权威持久化门（兼盖 `--artifact-put` 这类不经 Lua 的入口） |
///
/// **漂移守卫**：两边的测试向量表逐字相同（本文件的单测 ↔
/// `ygg-store/src/lib.rs` 的同名用例）。改规则必须一次提交改两边。
fn validate_vfs_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.len() > VFS_PATH_MAX_BYTES {
        return Err(format!(
            "路径长度必须在 1..={VFS_PATH_MAX_BYTES} 字节之间（当前 {} 字节：`{path}`）",
            path.len()
        ));
    }
    let Some(rest) = path.strip_prefix(VFS_PREFIX) else {
        return Err(format!("路径必须以 `{VFS_PREFIX}` 起头（当前是 `{path}`）"));
    };
    if rest.is_empty() {
        return Err(format!(
            "`{VFS_PREFIX}` 之后必须还有文件名（当前是 `{path}`）"
        ));
    }
    if rest.ends_with('/') {
        return Err(format!("路径不得以 `/` 结尾（当前是 `{path}`）"));
    }
    if rest.split('/').any(str::is_empty) {
        return Err(format!(
            "路径不得含空路径段（`{VFS_PREFIX}` 之后是 `{rest}`）"
        ));
    }
    if let Some(ch) = path
        .chars()
        .find(|ch| !(ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '/' | '-')))
    {
        return Err(format!("路径含非法字符 `{ch}`（只允许 `[A-Za-z0-9._/-]`）"));
    }
    if path.contains("..") {
        return Err(format!("路径不得含 `..`（目录穿越；当前是 `{path}`）"));
    }
    Ok(())
}

/// 桥共享状态：注册序缓冲 + 调用缓冲 + 宿主句柄 + 相位标志 + per-kernel
/// choice sentinel（D-S9-1 / D-S9-3）。
pub struct HostState {
    registered: Vec<(String, Function)>,
    calls: Vec<HostCall>,
    handles: HostHandles,
    /// `ygg.fs` 的内存态 VFS（与驱动共享同一 `Rc`；savepoint 基线由驱动克隆）。
    ///
    /// **不在 `HostHandles` 里**：句柄是宿主注入面（`crates/ygg-sandbox/tests`
    /// 与 `stdlib` 都构造它），加字段会破外部构造点；VFS 是 `install` 自建的
    /// 内部状态，经 `HostApi::vfs_handle()` 交给驱动。
    vfs: Rc<RefCell<VfsStore>>,
    /// 宿主侧同步工具执行器（未挂载时 `None`，`ygg.tool.call` 返回 `unmounted = true`）。
    tool_runner: Rc<RefCell<Option<Box<dyn HostToolRunner>>>>,
    /// 宿主侧文件挂载桥（未挂载时 `None`，访问 `mounts/…` 抛 `YGG-FS-UNMOUNTED`）。
    ///
    /// 与 `vfs` 的区别是纪律性的：VFS 是沙箱内存态（dispatch 级回滚），
    /// 挂载桥是写穿磁盘的对外面（不回滚、每次写审计）。两者共享同一个 `ygg.fs` 表，
    /// 按路径首段分派。
    mount: Rc<RefCell<Option<Box<dyn MountFs>>>>,
    /// 宿主侧模拟内核实例（Tier 1；未注入时 `None`，`ygg.sim.*` 全部抛 `YGG-SIM-UNMOUNTED`）。
    ///
    /// **不在 `HostHandles` 里**（与 `vfs` 同一条纪律）：句柄是外部构造点，加字段会破
    /// `crates/ygg-sandbox/tests` 与 stdlib 的既有构造；Sim 只是 `install` 自建的空槽。
    sim: Rc<RefCell<Option<Box<ygg_sim::Sim>>>>,
    /// 当前执行相位（装载期 `Loading`，封口后待机 `Idle`，handler 执行期间 `Transition`）。
    phase: Phase,
    sealed: bool,
    sentinel: String,
    /// 本次 handler 调用中，`ygg.choice` 之前已入缓冲的 `Emit` 条目数
    /// （D1：这些条目将被 pending_choice 终结丢弃；每 handler 调用重置）。
    pre_choice_emits: u64,
    /// 本次 dispatch 的人读警告（D1；至多一条，首个胜出，驱动 drain 后清空）。
    warning: Option<String>,
    /// 结构化 meta（TD1）：`ygg.meta(key, value)` 写入面，键序 = `BTreeMap`
    /// 序（天然确定性）。瞬态——驱动每次 dispatch 入口 clear、转换后 take，
    /// 不入 canonical snapshot / autosave / 状态哈希。
    meta: BTreeMap<String, MetaValue>,
}

/// 宿主 API 的 Rust 侧控制柄：seal / 注册进注册表。
pub struct HostApi {
    lua: Lua,
    state: Rc<RefCell<HostState>>,
}

/// 装载期注册的单个 Lua handler 的桥实现（core `EventHandler` 反转面）。
struct LuaHandler {
    lua: Lua,
    function: Function,
    state: Rc<RefCell<HostState>>,
}

impl LuaHandler {
    /// 本 kernel install 的 choice sentinel（audit 代决：不可预测 token，
    /// 防 Mod 以 `\u{1}` 转义伪造首行吞掉自身错误——伪造只会成为普通错误浮出）。
    fn is_choice_sentinel(&self, err: &Error) -> bool {
        is_choice_sentinel(&self.state.borrow().sentinel, err)
    }
}

impl EventHandler for LuaHandler {
    fn handle(&mut self, event: &Event, sink: &mut dyn NarrationSink) -> Result<(), HandlerError> {
        // ① 清空调用缓冲与 choice 前置计数（上一 handler 的余量不跨 handler 泄漏），进入 Transition 相位。
        let prev_phase = {
            let mut state = self.state.borrow_mut();
            state.calls.clear();
            state.pre_choice_emits = 0;
            let prev = state.phase;
            state.phase = Phase::Transition;
            prev
        };
        struct PhaseReset {
            state: Rc<RefCell<HostState>>,
            prev: Phase,
        }
        impl Drop for PhaseReset {
            fn drop(&mut self) {
                self.state.borrow_mut().phase = self.prev;
            }
        }
        let _phase_guard = PhaseReset {
            state: Rc::clone(&self.state),
            prev: prev_phase,
        };
        // ② core Value → mlua Value，装载期存下的函数以 payload 为唯一实参。
        let payload = value_to_lua(&self.lua, event.payload()).map_err(|e| {
            HandlerError(format!(
                "event `{}` payload marshal failed: {e}",
                event.name()
            ))
        })?;
        // ③ 调用：sentinel = choice 正常终结；其余错误包成 HandlerError。
        match self.function.call::<()>(payload) {
            Ok(()) => {}
            Err(err) if self.is_choice_sentinel(&err) => {}
            Err(err) => {
                return Err(HandlerError(format!(
                    "lua handler for event `{}` failed: {err}",
                    event.name()
                )))
            }
        }
        // ③' D1 警告：choice 前置 Emit 已被 pending_choice 终结语义丢弃——
        //    事件名在此可得（choice 回调时只有缓冲可见），每次 dispatch
        //    首条胜出（warning 为 None 才写入）。
        let pre_choice = self.state.borrow().pre_choice_emits;
        if pre_choice > 0 {
            let mut state = self.state.borrow_mut();
            if state.warning.is_none() {
                state.warning = Some(format!(
                    "事件 `{}` 的 handler 在 ygg.choice 之前产生了 {} 条派生事件（ygg.emit），\
                     已按语义丢弃（choice 须为 handler 最后一个输出调用）",
                    event.name(),
                    pre_choice
                ));
            }
        }
        // ④ 按追加序回放进 sink（等价性论证见模块文档）。
        let calls = std::mem::take(&mut self.state.borrow_mut().calls);
        for call in calls {
            match call {
                HostCall::Emit { name, payload } => {
                    let derived =
                        Event::derived(name, payload).map_err(|e| HandlerError(e.to_string()))?;
                    sink.emit(derived);
                }
                HostCall::Narrate(line) => sink.narrate(line),
                HostCall::Choose(choice) => sink.choose(choice),
            }
        }
        Ok(())
    }
}

/// 恰好以 per-kernel sentinel 为首行的错误才是 choice 终结（D-S9-3）。
/// 错误跨 Lua 边界后 mlua 包装为 `CallbackError{ cause }` 链（每层边界一层，
/// 原 RuntimeError 在链尾），故递归解包 cause；消息比对取首行——mlua 在
/// 某些路径会把 stack traceback 并入消息体。sentinel 为 install 时由内核种子
/// 经 splitmix64 派生的不可预测 token（S9 审计代决：静态常量可被 Mod 以
/// `\u{1}` 转义伪造首行而吞掉自身错误；token 化后伪造只会成为普通错误浮出）。
fn is_choice_sentinel(sentinel: &str, err: &Error) -> bool {
    match err {
        Error::RuntimeError(msg) => msg.split('\n').next() == Some(sentinel),
        Error::CallbackError { cause, .. } => is_choice_sentinel(sentinel, cause),
        _ => false,
    }
}

/// mlua `Value` 的 Lua 类型名（可读报错用）。
fn lua_type_name(value: &LuaValue) -> &'static str {
    match value {
        LuaValue::Nil => "nil",
        LuaValue::Boolean(_) => "boolean",
        LuaValue::LightUserData(_) => "light userdata",
        LuaValue::Integer(_) => "integer",
        LuaValue::Number(_) => "number",
        LuaValue::String(_) => "string",
        LuaValue::Table(_) => "table",
        LuaValue::Function(_) => "function",
        LuaValue::Thread(_) => "thread",
        LuaValue::UserData(_) => "userdata",
        _ => "unknown",
    }
}

/// core `Value` → mlua `Value`（core→Lua 方向；只映射四个标量）。
fn value_to_lua(lua: &Lua, value: &Value) -> Result<LuaValue, Error> {
    Ok(match value {
        Value::Int(v) => LuaValue::Integer(*v),
        Value::Float(v) => LuaValue::Number(*v),
        Value::Bool(v) => LuaValue::Boolean(*v),
        Value::Str(v) => LuaValue::String(lua.create_string(v)?),
    })
}

/// mlua `Value` → core `Value`（Lua→core 方向）：四个标量直通；
/// table / function / 其他 → 可读 `unsupported payload type` 拒绝。
/// NaN / ±∞ 在此放行为 `Value::Float`，由调用点的 core 门拒绝（事件构造 /
/// KvStore::set / EntityStore::set_attribute 均带定位）。
fn value_from_lua(value: &LuaValue) -> Result<Value, Error> {
    Ok(match value {
        LuaValue::Integer(v) => Value::Int(*v),
        LuaValue::Number(v) => Value::Float(*v),
        LuaValue::Boolean(v) => Value::Bool(*v),
        LuaValue::String(v) => Value::Str(v.to_str()?.to_owned()),
        other => {
            return Err(Error::runtime(format!(
                "unsupported payload type: {}",
                lua_type_name(other)
            )))
        }
    })
}

// ============================================================================
// `ygg.entity.*` 批式面的形状校验（P-K A1；铁律②「数组进出、单次调用承载整批」）
// ============================================================================

/// 形状不对（非数组 / 元素不是表 / 字段类型错 / 值非标量）。
pub const ENTITY_FRAME: &str = "YGG-ENTITY-FRAME";
/// 值不在值域（非有限浮点）。
pub const ENTITY_VALUE: &str = "YGG-ENTITY-VALUE";
/// 未知实体 id。
pub const ENTITY_ID: &str = "YGG-ENTITY-ID";

/// 取实体句柄（批式面的统一入口；`HostState` 借用只在调用瞬间）。
fn entities_of(state: &Rc<RefCell<HostState>>) -> Rc<RefCell<EntityStore>> {
    Rc::clone(&state.borrow().handles.entities)
}

fn entity_frame(detail: impl std::fmt::Display) -> Error {
    Error::runtime(format!("{ENTITY_FRAME}: {detail}"))
}

fn entity_value(detail: impl std::fmt::Display) -> Error {
    Error::runtime(format!("{ENTITY_VALUE}: {detail}"))
}

fn entity_id_error(id: u64) -> Error {
    Error::runtime(format!("{ENTITY_ID}: 未知实体 id {id}"))
}

/// 一个待造的行：属性袋 + tag 名单（都是可选的）。
#[derive(Clone, Debug, Default)]
pub struct EntityRowSpec {
    pub attrs: Vec<(String, Value)>,
    pub tags: Vec<String>,
}

/// 值域闸门：标量四型直通，非有限浮点拒（`EntityStore::set_attribute` 同样拒，
/// 但这一层先拒 = 整批零副作用）。
fn entity_value_arg(value: &LuaValue, what: &str) -> Result<Value, Error> {
    let value = value_from_lua(value).map_err(|_| {
        entity_value(format!(
            "`{what}` 只接受标量（boolean / number / string），得到 {}",
            lua_type_name(value)
        ))
    })?;
    if let Value::Float(f) = &value {
        if !f.is_finite() {
            return Err(entity_value(format!("`{what}` 不接受 NaN/±∞")));
        }
    }
    Ok(value)
}

/// 稠密数组 → Vec（非表即拒；`raw_len` 为 0 的表按空批处理）。
fn entity_array(table: &Table, what: &str) -> Result<Vec<LuaValue>, Error> {
    let len = table.raw_len();
    let mut out = Vec::with_capacity(len);
    for index in 1..=len {
        out.push(
            table
                .raw_get::<LuaValue>(index)
                .map_err(|_| entity_frame(format!("`{what}` 的第 {index} 项不可读")))?,
        );
    }
    Ok(out)
}

fn entity_field(row: &Table, key: &str) -> Result<Option<Table>, Error> {
    match row
        .raw_get::<LuaValue>(key)
        .map_err(|_| entity_frame("字段不可读"))?
    {
        LuaValue::Nil => Ok(None),
        LuaValue::Table(t) => Ok(Some(t)),
        other => Err(entity_frame(format!(
            "`{key}` 必须是表或 nil，得到 {}",
            lua_type_name(&other)
        ))),
    }
}

/// `spawn_batch(rows)`：每个元素 `{ attrs = {k = v}, tags = {t, …} }`。
pub fn entity_rows(rows: &Table) -> Result<Vec<EntityRowSpec>, Error> {
    let mut out = Vec::new();
    for (index, item) in entity_array(rows, "rows")?.iter().enumerate() {
        let LuaValue::Table(row) = item else {
            return Err(entity_frame(format!(
                "rows[{index}] 必须是表，得到 {}",
                lua_type_name(item)
            )));
        };
        let mut spec = EntityRowSpec::default();
        if let Some(attrs) = entity_field(row, "attrs")? {
            for pair in attrs.clone().pairs::<LuaValue, LuaValue>() {
                let (key, value) = pair.map_err(|_| entity_frame("attrs 的键值对不可读"))?;
                let LuaValue::String(key) = key else {
                    return Err(entity_frame("attrs 的键必须是字符串"));
                };
                let key = key
                    .to_str()
                    .map_err(|_| entity_frame("attrs 的键非 UTF-8"))?;
                spec.attrs
                    .push((key.to_string(), entity_value_arg(&value, key.as_ref())?));
            }
        }
        if let Some(tags) = entity_field(row, "tags")? {
            for item in entity_array(&tags, "tags")? {
                let LuaValue::String(tag) = item else {
                    return Err(entity_frame("tags 的元素必须是字符串"));
                };
                spec.tags.push(
                    tag.to_str()
                        .map_err(|_| entity_frame("tag 非 UTF-8"))?
                        .to_string(),
                );
            }
        }
        out.push(spec);
    }
    Ok(out)
}

/// `attr_apply(ops)`：每个元素 `{ id = n, attrs = {k = v} }`。
pub fn entity_attr_ops(ops: &Table) -> Result<Vec<(u64, Vec<(String, Value)>)>, Error> {
    let mut out = Vec::new();
    for (index, item) in entity_array(ops, "ops")?.iter().enumerate() {
        let LuaValue::Table(op) = item else {
            return Err(entity_frame(format!(
                "ops[{index}] 必须是表，得到 {}",
                lua_type_name(item)
            )));
        };
        let id = match op.raw_get::<LuaValue>("id") {
            Ok(LuaValue::Integer(v)) if v >= 0 => v as u64,
            _ => return Err(entity_frame(format!("ops[{index}].id 必须是非负整数"))),
        };
        let mut kvs = Vec::new();
        if let Some(attrs) = entity_field(op, "attrs")? {
            for pair in attrs.clone().pairs::<LuaValue, LuaValue>() {
                let (key, value) = pair.map_err(|_| entity_frame("attrs 的键值对不可读"))?;
                let LuaValue::String(key) = key else {
                    return Err(entity_frame("attrs 的键必须是字符串"));
                };
                let key = key
                    .to_str()
                    .map_err(|_| entity_frame("attrs 的键非 UTF-8"))?;
                kvs.push((key.to_string(), entity_value_arg(&value, key.as_ref())?));
            }
        }
        out.push((id, kvs));
    }
    Ok(out)
}

/// `attr_fetch(queries)`：每个元素 `{ id = n, keys = {k, …} }`。
pub fn entity_fetch_queries(queries: &Table) -> Result<Vec<(u64, Vec<String>)>, Error> {
    let mut out = Vec::new();
    for (index, item) in entity_array(queries, "queries")?.iter().enumerate() {
        let LuaValue::Table(query) = item else {
            return Err(entity_frame(format!(
                "queries[{index}] 必须是表，得到 {}",
                lua_type_name(item)
            )));
        };
        let id = match query.raw_get::<LuaValue>("id") {
            Ok(LuaValue::Integer(v)) if v >= 0 => v as u64,
            _ => return Err(entity_frame(format!("queries[{index}].id 必须是非负整数"))),
        };
        let mut keys = Vec::new();
        if let Some(list) = entity_field(query, "keys")? {
            for item in entity_array(&list, "keys")? {
                let LuaValue::String(key) = item else {
                    return Err(entity_frame("keys 的元素必须是字符串"));
                };
                keys.push(
                    key.to_str()
                        .map_err(|_| entity_frame("key 非 UTF-8"))?
                        .to_string(),
                );
            }
        }
        out.push((id, keys));
    }
    Ok(out)
}

// ============================================================================
// `ygg.entity.query` 的文法解码（S24 v2 §1；**先校验后扫**）
// ============================================================================
//
// 语义不在这里：选择器 / 比较矩阵 / 索引判定都在 `ygg_core::entity`。本区只把
// Lua 表解码成 [`Selector`]，并把两态书写（字符串态糖 + 结构态逃生口）的**每一种**
// 越界形态拒成 `YGG-ENTITY-FRAME`（值域错仍是 `YGG-ENTITY-VALUE`，沿用 A1 两族）。

/// `query(selector)` 的完整解码；返回即全部校验完毕（扫描前零副作用）。
///
/// 四个顶层键，多一个即 FRAME：`tag` / `where` / `select` / `order`。
pub fn entity_query_selector(selector: &Table) -> Result<Selector, Error> {
    let mut out = Selector::default();
    for pair in selector.clone().pairs::<LuaValue, LuaValue>() {
        let (key, value) = pair.map_err(|_| entity_frame("selector 的键值对不可读"))?;
        let name = match &key {
            LuaValue::String(text) => text
                .to_str()
                .map_err(|_| entity_frame("selector 键非 UTF-8"))?
                .to_string(),
            other => {
                return Err(entity_frame(format!(
                    "selector 的键必须是字符串，得到 {}",
                    lua_type_name(other)
                )))
            }
        };
        match name.as_str() {
            "tag" => match value {
                LuaValue::Nil => {}
                LuaValue::String(tag) => {
                    out.tag = Some(
                        tag.to_str()
                            .map_err(|_| entity_frame("`tag` 非 UTF-8"))?
                            .to_string(),
                    )
                }
                other => {
                    return Err(entity_frame(format!(
                        "`tag` 必须是字符串，得到 {}",
                        lua_type_name(&other)
                    )))
                }
            },
            "where" => match value {
                LuaValue::Nil => {}
                LuaValue::Table(map) => out.predicates = entity_predicate_map(&map)?,
                other => {
                    return Err(entity_frame(format!(
                        "`where` 必须是表，得到 {}",
                        lua_type_name(&other)
                    )))
                }
            },
            "select" => match value {
                LuaValue::Nil => {}
                LuaValue::Table(list) => out.select = entity_projection(&list)?,
                other => {
                    return Err(entity_frame(format!(
                        "`select` 必须是表，得到 {}",
                        lua_type_name(&other)
                    )))
                }
            },
            "order" => match value {
                LuaValue::Nil => {}
                LuaValue::String(text) => {
                    out.order = Some(entity_order(
                        &text
                            .to_str()
                            .map_err(|_| entity_frame("`order` 非 UTF-8"))?,
                    )?)
                }
                other => {
                    return Err(entity_frame(format!(
                        "`order` 必须是字符串，得到 {}",
                        lua_type_name(&other)
                    )))
                }
            },
            other => {
                return Err(entity_frame(format!(
                    "未知 selector 键 `{other}`（只有 tag / where / select / order）"
                )))
            }
        }
    }
    Ok(out)
}

/// `where` = 属性名 → 一个谓词（字符串态 / 结构态）或谓词数组（同键多条 = 隐式 AND）。
fn entity_predicate_map(map: &Table) -> Result<Vec<Predicate>, Error> {
    if map.raw_len() > 0 {
        // v1 的三元组数组形已退役：点名新形，防旧代码静默变成「无过滤」。
        return Err(entity_frame(
            "`where` 是**属性名 → 谓词**的映射（如 `{ hp = \"gte.10\" }`），不是三元组数组",
        ));
    }
    let mut out = Vec::new();
    for pair in map.clone().pairs::<LuaValue, LuaValue>() {
        let (key, value) = pair.map_err(|_| entity_frame("`where` 的键值对不可读"))?;
        let attr = match &key {
            LuaValue::String(text) => text
                .to_str()
                .map_err(|_| entity_frame("`where` 的属性名非 UTF-8"))?
                .to_string(),
            other => {
                return Err(entity_frame(format!(
                    "`where` 的键必须是字符串，得到 {}",
                    lua_type_name(other)
                )))
            }
        };
        match value {
            LuaValue::String(text) => {
                let text = text
                    .to_str()
                    .map_err(|_| entity_frame("谓词文本非 UTF-8"))?;
                out.push(text_predicate(&attr, &text)?);
            }
            LuaValue::Table(spec) => out.extend(entity_predicate_value(&attr, &spec)?),
            other => {
                return Err(entity_frame(format!(
                    "`{attr}` 的谓词必须是字符串或表，得到 {}",
                    lua_type_name(&other)
                )))
            }
        }
    }
    Ok(out)
}

/// 同键值的两态判别：**表里有 `op` 字符串键 ⇒ 结构态；否则视为谓词数组**
/// （数组不可能有 `op` 键 ⇒ 无歧义）。
fn entity_predicate_value(attr: &str, spec: &Table) -> Result<Vec<Predicate>, Error> {
    match spec.raw_get::<LuaValue>("op") {
        Ok(LuaValue::String(_)) => Ok(vec![structured_predicate(attr, spec)?]),
        Ok(LuaValue::Nil) => {
            let mut out = Vec::new();
            for item in entity_array(spec, attr)? {
                let LuaValue::String(text) = item else {
                    return Err(entity_frame(format!(
                        "`{attr}` 的谓词数组元素必须是字符串，得到 {}",
                        lua_type_name(&item)
                    )));
                };
                let text = text
                    .to_str()
                    .map_err(|_| entity_frame("谓词文本非 UTF-8"))?;
                out.push(text_predicate(attr, &text)?);
            }
            Ok(out)
        }
        Ok(other) => Err(entity_frame(format!(
            "`{attr}` 的 `op` 必须是字符串，得到 {}",
            lua_type_name(&other)
        ))),
        Err(_) => Err(entity_frame(format!("`{attr}` 的谓词表不可读"))),
    }
}

/// 字符串态一条谓词：`"op"` / `"op.operand"` / `"not.op.operand"`
/// （`op` = 首个 `.` 之前，其余 = 操作数，空余 = 空串）。
fn text_predicate(attr: &str, text: &str) -> Result<Predicate, Error> {
    let (negated, op_name, operand) = split_op_text(text);
    let operand = operand.unwrap_or("");
    let op = match op_name {
        "eq" => CompareOp::Eq(text_scalar(operand)?),
        "neq" => CompareOp::Neq(text_scalar(operand)?),
        "gt" => CompareOp::Gt(text_scalar(operand)?),
        "gte" => CompareOp::Gte(text_scalar(operand)?),
        "lt" => CompareOp::Lt(text_scalar(operand)?),
        "lte" => CompareOp::Lte(text_scalar(operand)?),
        "in" => text_in(operand)?,
        "like" | "ilike" => text_like(op_name, operand)?,
        "is" => CompareOp::Is(text_is(operand)?),
        other => {
            return Err(entity_frame(format!(
                "未知 op `{other}`（可选 eq/neq/gt/gte/lt/lte/in/like/ilike/is；`not.` 是前缀）"
            )))
        }
    };
    let predicate = Predicate::new(attr, op);
    Ok(if negated {
        predicate.negate()
    } else {
        predicate
    })
}

/// 结构态一条谓词：`{op = "gte", value = 10}`。`op` 可带 **`not.` 前缀**，但**不得**
/// 自带操作数（操作数只从 `value` 取）——两种来源同时存在就是歧义。
fn structured_predicate(attr: &str, spec: &Table) -> Result<Predicate, Error> {
    let mut op_text: Option<String> = None;
    let mut value: Option<LuaValue> = None;
    for pair in spec.clone().pairs::<LuaValue, LuaValue>() {
        let (key, item) = pair.map_err(|_| entity_frame("结构态谓词的键值对不可读"))?;
        let name = match &key {
            LuaValue::String(text) => text
                .to_str()
                .map_err(|_| entity_frame("结构态谓词键非 UTF-8"))?
                .to_string(),
            other => {
                return Err(entity_frame(format!(
                    "`{attr}` 的结构态谓词键必须是字符串，得到 {}",
                    lua_type_name(other)
                )))
            }
        };
        match name.as_str() {
            "op" => match item {
                LuaValue::String(text) => {
                    op_text = Some(
                        text.to_str()
                            .map_err(|_| entity_frame("`op` 非 UTF-8"))?
                            .to_string(),
                    )
                }
                other => {
                    return Err(entity_frame(format!(
                        "`{attr}` 的 `op` 必须是字符串，得到 {}",
                        lua_type_name(&other)
                    )))
                }
            },
            "value" => value = Some(item),
            other => {
                return Err(entity_frame(format!(
                    "结构态谓词只认 `op` / `value`，`{attr}` 多出 `{other}`"
                )))
            }
        }
    }
    let op_text = op_text.ok_or_else(|| entity_frame(format!("`{attr}` 的结构态谓词缺 `op`")))?;
    let (negated, op_name, inline) = split_op_text(&op_text);
    if inline.is_some() {
        return Err(entity_frame(format!(
            "结构态的 `op` 不能带操作数（操作数走 `value`），`{attr}` 得到 `{op_text}`"
        )));
    }
    let value = value.ok_or_else(|| entity_frame(format!("`{attr}` 的结构态谓词缺 `value`")))?;
    let op = match op_name {
        "eq" | "neq" | "gt" | "gte" | "lt" | "lte" => {
            let scalar = entity_value_arg(&value, attr)?;
            match op_name {
                "eq" => CompareOp::Eq(scalar),
                "neq" => CompareOp::Neq(scalar),
                "gt" => CompareOp::Gt(scalar),
                "gte" => CompareOp::Gte(scalar),
                "lt" => CompareOp::Lt(scalar),
                _ => CompareOp::Lte(scalar),
            }
        }
        "in" => {
            let LuaValue::Table(list) = value else {
                return Err(entity_frame(format!(
                    "`{attr}` 的 `in` 操作数必须是数组，得到 {}",
                    lua_type_name(&value)
                )));
            };
            let mut values = Vec::new();
            for item in entity_array(&list, attr)? {
                values.push(entity_value_arg(&item, attr)?);
            }
            CompareOp::In(values)
        }
        "like" | "ilike" => {
            let LuaValue::String(text) = value else {
                return Err(entity_frame(format!(
                    "`{attr}` 的 `{op_name}` 操作数必须是字符串，得到 {}",
                    lua_type_name(&value)
                )));
            };
            let text = text
                .to_str()
                .map_err(|_| entity_frame("`like` 操作数非 UTF-8"))?;
            text_like(op_name, &text)?
        }
        "is" => {
            let LuaValue::String(text) = value else {
                return Err(entity_frame(format!(
                    "`{attr}` 的 `is` 操作数必须是字符串，得到 {}",
                    lua_type_name(&value)
                )));
            };
            CompareOp::Is(text_is(
                &text
                    .to_str()
                    .map_err(|_| entity_frame("`is` 操作数非 UTF-8"))?,
            )?)
        }
        other => {
            return Err(entity_frame(format!(
                "未知 op `{other}`（可选 eq/neq/gt/gte/lt/lte/in/like/ilike/is；`not.` 是前缀）"
            )))
        }
    };
    let predicate = Predicate::new(attr, op);
    Ok(if negated {
        predicate.negate()
    } else {
        predicate
    })
}

/// `not.` 前缀 + `op[.operand]` 的切分（两态共用）。
fn split_op_text(text: &str) -> (bool, &str, Option<&str>) {
    let (negated, rest) = match text.strip_prefix("not.") {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    match rest.split_once('.') {
        Some((op, operand)) => (negated, op, Some(operand)),
        None => (negated, rest, None),
    }
}

/// 字符串态的标量解析（文档化、与 locale 无关的确定性规则）：
/// `-?[0-9]+(\.[0-9]+)?` → 数字；`true`/`false` → 布尔；其余 → 字符串；
/// `null` → **拒**（判空走 `is`）。
fn text_scalar(text: &str) -> Result<Value, Error> {
    if text == "null" {
        return Err(entity_frame("`null` 不是操作数（判空请用 `is.null`）"));
    }
    if text == "true" {
        return Ok(Value::Bool(true));
    }
    if text == "false" {
        return Ok(Value::Bool(false));
    }
    Ok(text_number(text).unwrap_or_else(|| Value::Str(text.to_owned())))
}

/// 纯数字形态 → `Int` / `Float`（带小数点才走浮点，与 Lua 的整浮身份直觉一致）。
fn text_number(text: &str) -> Option<Value> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    let (int_part, frac_part) = match digits.split_once('.') {
        Some((int_part, frac)) => (int_part, Some(frac)),
        None => (digits, None),
    };
    if int_part.is_empty() || !int_part.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if let Some(frac) = frac_part {
        if frac.is_empty() || !frac.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
    }
    match frac_part {
        // 超 i64 的整数字面量降为 Float（与 `Value` 反序列化 visitor 的取舍同源）
        None => Some(
            text.parse::<i64>()
                .map(Value::Int)
                .or_else(|_| text.parse::<f64>().map(Value::Float))
                .ok()
                .filter(|value| match value {
                    Value::Float(number) => number.is_finite(),
                    _ => true,
                })?,
        ),
        Some(_) => Some(Value::Float(
            text.parse::<f64>().ok().filter(|f| f.is_finite())?,
        )),
    }
}

/// 字符串态 `in.(a,b)`（空 `()` = 空数组 = 恒假）。
fn text_in(operand: &str) -> Result<CompareOp, Error> {
    let inner = operand
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
        .ok_or_else(|| {
            entity_frame(format!(
                "`in` 的字符串态必须是 `in.(a,b)`，得到 `{operand}`"
            ))
        })?;
    if inner.is_empty() {
        return Ok(CompareOp::In(Vec::new()));
    }
    let mut values = Vec::new();
    for item in inner.split(',') {
        if item.is_empty() {
            return Err(entity_frame("`in.(…)` 的元素不能为空"));
        }
        values.push(text_scalar(item)?);
    }
    Ok(CompareOp::In(values))
}

/// `like` / `ilike` 的形态门（四形态之外的写法是错，不是宽匹配）。
fn text_like(op_name: &str, pattern: &str) -> Result<CompareOp, Error> {
    like_shape(pattern).ok_or_else(|| {
        entity_frame(format!(
            "`{op_name}` 只接受四种形态：无 `*` / `P*` / `*S` / `*C*`，得到 `{pattern}`"
        ))
    })?;
    Ok(if op_name == "like" {
        CompareOp::Like(pattern.to_owned())
    } else {
        CompareOp::ILike(pattern.to_owned())
    })
}

/// `is` 的四种操作数。
fn text_is(operand: &str) -> Result<IsTest, Error> {
    IsTest::of(operand).ok_or_else(|| {
        entity_frame(format!(
            "`is` 的操作数只能是 `null` / `notnull` / `true` / `false`，得到 `{operand}`"
        ))
    })
}

/// `select`：非空具名出表；空数组 ≡ 缺省（裸 id 数组）。`"id"` 拒（行恒带 id）。
fn entity_projection(list: &Table) -> Result<Vec<String>, Error> {
    let mut out = Vec::new();
    for item in entity_array(list, "select")? {
        let LuaValue::String(text) = item else {
            return Err(entity_frame(format!(
                "`select` 的元素必须是字符串，得到 {}",
                lua_type_name(&item)
            )));
        };
        let name = text
            .to_str()
            .map_err(|_| entity_frame("`select` 元素非 UTF-8"))?
            .to_string();
        if name == "id" {
            return Err(entity_frame("`select` 不接受 `id`（行恒带 id）"));
        }
        out.push(name);
    }
    Ok(out)
}

/// `order = "field" / "field.asc" / "field.desc"`。
/// 属性名可含 `.`：**只有结尾恰为 `.asc` / `.desc` 才当方向**。
fn entity_order(text: &str) -> Result<Order, Error> {
    let (attr, descending) = if let Some(attr) = text.strip_suffix(".desc") {
        (attr, true)
    } else if let Some(attr) = text.strip_suffix(".asc") {
        (attr, false)
    } else {
        (text, false)
    };
    if attr.is_empty() {
        return Err(entity_frame(format!(
            "`order` 的属性名不能为空，得到 `{text}`"
        )));
    }
    Ok(Order {
        attr: attr.to_owned(),
        descending,
    })
}

/// `tag_batch(ops)`：每个元素 `{ id = n, tag = "t", add = true|false }`。
pub fn entity_tag_ops(ops: &Table) -> Result<Vec<(u64, String, bool)>, Error> {
    let mut out = Vec::new();
    for (index, item) in entity_array(ops, "ops")?.iter().enumerate() {
        let LuaValue::Table(op) = item else {
            return Err(entity_frame(format!(
                "ops[{index}] 必须是表，得到 {}",
                lua_type_name(item)
            )));
        };
        let id = match op.raw_get::<LuaValue>("id") {
            Ok(LuaValue::Integer(v)) if v >= 0 => v as u64,
            _ => return Err(entity_frame(format!("ops[{index}].id 必须是非负整数"))),
        };
        let tag = match op.raw_get::<LuaValue>("tag") {
            Ok(LuaValue::String(tag)) => tag
                .to_str()
                .map_err(|_| entity_frame("tag 非 UTF-8"))?
                .to_string(),
            _ => return Err(entity_frame(format!("ops[{index}].tag 必须是字符串"))),
        };
        let add = match op.raw_get::<LuaValue>("add") {
            Ok(LuaValue::Boolean(v)) => v,
            _ => return Err(entity_frame(format!("ops[{index}].add 必须是布尔"))),
        };
        out.push((id, tag, add));
    }
    Ok(out)
}

/// options 必须是字符串的 Lua 序列（数组形态），否则可读报错。
fn options_from_lua(options: &LuaValue) -> Result<Vec<String>, Error> {
    let table = match options {
        LuaValue::Table(t) => t,
        other => {
            return Err(Error::runtime(format!(
                "ygg.choice options must be a sequence of strings, got {}",
                lua_type_name(other)
            )))
        }
    };
    let len = table.raw_len();
    let mut out = Vec::with_capacity(len);
    for i in 1..=len {
        let item: LuaValue = table.raw_get(i)?;
        match &item {
            LuaValue::String(s) => out.push(s.to_str()?.to_owned()),
            other => {
                return Err(Error::runtime(format!(
                    "ygg.choice options[{i}] must be a string, got {}",
                    lua_type_name(other)
                )))
            }
        }
    }
    Ok(out)
}

/// 装载期安装完整 `ygg` 面（D-S9-1）。必须在 `Sandbox` 构造之后调用
/// （依赖 build 创建的 `ygg` 占位表；fuel / inject 之后再装本面不冲突——
/// 各自 raw_set 不同成员）。
/// 未挂载时的统一表述（码在首段，与 `YGG-FS-UNMOUNTED` 同一处置）。
fn sim_unmounted(what: &str) -> String {
    format!(
        "{}: 宿主未注入模拟内核，`{what}` 不可用（注入面见 `KernelFacade::set_sim`）",
        SIM_UNMOUNTED
    )
}

/// `ygg.sim.*` 的未挂载码（Tier 1 面；与 `ygg_fs_code` 分开命名空间以免两族混淆）。
pub const SIM_UNMOUNTED: &str = "YGG-SIM-UNMOUNTED";
/// 定点闸门的拒绝码（非整数值过面 / 除零）。
pub const SIM_FIXED: &str = "YGG-SIM-FIXED";

/// 定点闸门：Lua number → `i64`，**只接受值上等于整数**的数。
///
/// 这是 Tier 2 双精度与 Tier 1 定点之间唯一的闸门：静默取整会把「调用方算错了」变成
/// 「内核算错了」，而后者无法在回放证据里查出来。上限 2^53 是双精度可精确表示的整数边界
/// （超过它「值上等于整数」这个判断本身就不成立了）。
fn sim_fixed_arg(value: LuaValue, what: &str) -> Result<i64, Error> {
    const MAX_EXACT: f64 = 9_007_199_254_740_992.0;
    match value {
        LuaValue::Integer(v) => Ok(v),
        LuaValue::Number(n) if n.fract() == 0.0 && n.abs() <= MAX_EXACT => Ok(n as i64),
        other => Err(Error::runtime(format!(
            "{SIM_FIXED}: `{what}` 必须是值上等于整数的 Lua number（Q32.32），得到 {}",
            other.type_name()
        ))),
    }
}

/// 装 `ygg.sim.*`（10 个键，**一个逐实体 getter 都没有**——批式铁律）。
///
/// 表**始终**注册：未注入实例时每个函数抛 [`SIM_UNMOUNTED`] 而不是返回 `nil` / `false`。
/// `nil` 与「世界是空的」不可分辨，静默降级会让 Mod 把「宿主不支持」读成「世界为空」并据此
/// 写下错误状态。
fn install_sim(lua: &Lua, state: &Rc<RefCell<HostState>>) -> Result<(), Error> {
    let ygg: Table = lua.globals().raw_get("ygg")?;
    let table = lua.create_table()?;

    let st = Rc::clone(state);
    table.raw_set(
        "step",
        lua.create_function(move |lua, dt: LuaValue| -> Result<Table, Error> {
            let dt = sim_fixed_arg(dt, "dt")?;
            let slot = Rc::clone(&st.borrow().sim);
            let (tick, hash) = {
                let mut guard = slot.borrow_mut();
                let sim = guard
                    .as_mut()
                    .ok_or_else(|| Error::runtime(sim_unmounted("sim.step")))?;
                let hash = sim.step(dt).map_err(Error::runtime)?;
                (sim.tick(), hash)
            };
            let out = lua.create_table()?;
            out.raw_set("tick", tick)?;
            out.raw_set("hash", format!("{hash:016x}"))?;
            Ok(out)
        })?,
    )?;

    let st = Rc::clone(state);
    table.raw_set(
        "hash",
        lua.create_function(move |_, ()| -> Result<String, Error> {
            let slot = Rc::clone(&st.borrow().sim);
            let guard = slot.borrow();
            let sim = guard
                .as_ref()
                .ok_or_else(|| Error::runtime(sim_unmounted("sim.hash")))?;
            Ok(format!("{:016x}", sim.hash()))
        })?,
    )?;

    let st = Rc::clone(state);
    table.raw_set(
        "snapshot",
        lua.create_function(move |lua, ()| -> Result<LuaValue, Error> {
            let slot = Rc::clone(&st.borrow().sim);
            let bytes = {
                let guard = slot.borrow();
                let sim = guard
                    .as_ref()
                    .ok_or_else(|| Error::runtime(sim_unmounted("sim.snapshot")))?;
                sim.snapshot()
            };
            Ok(LuaValue::String(lua.create_string(&bytes)?))
        })?,
    )?;

    let st = Rc::clone(state);
    table.raw_set(
        "restore",
        lua.create_function(move |lua, bytes: mlua::String| -> Result<Table, Error> {
            let slot = Rc::clone(&st.borrow().sim);
            let (tick, len, hash) = {
                let mut guard = slot.borrow_mut();
                let sim = guard
                    .as_mut()
                    .ok_or_else(|| Error::runtime(sim_unmounted("sim.restore")))?;
                sim.restore(&bytes.as_bytes()).map_err(Error::runtime)?;
                (sim.tick(), bytes.as_bytes().len(), sim.hash())
            };
            let out = lua.create_table()?;
            out.raw_set("tick", tick)?;
            out.raw_set("bytes", len)?;
            out.raw_set("hash", format!("{hash:016x}"))?;
            Ok(out)
        })?,
    )?;

    let st = Rc::clone(state);
    table.raw_set(
        "spawn_batch",
        lua.create_function(move |lua, frame: mlua::String| -> Result<Table, Error> {
            let slot = Rc::clone(&st.borrow().sim);
            let (spawned, next_id) = {
                let mut guard = slot.borrow_mut();
                let sim = guard
                    .as_mut()
                    .ok_or_else(|| Error::runtime(sim_unmounted("sim.spawn_batch")))?;
                let report = sim.spawn_batch(&frame.as_bytes()).map_err(Error::runtime)?;
                (report.spawned, report.next_id)
            };
            let out = lua.create_table()?;
            out.raw_set("spawned", spawned)?;
            out.raw_set("next_id", next_id)?;
            Ok(out)
        })?,
    )?;

    let st = Rc::clone(state);
    table.raw_set(
        "despawn_batch",
        lua.create_function(move |lua, frame: mlua::String| -> Result<Table, Error> {
            let slot = Rc::clone(&st.borrow().sim);
            let removed = {
                let mut guard = slot.borrow_mut();
                let sim = guard
                    .as_mut()
                    .ok_or_else(|| Error::runtime(sim_unmounted("sim.despawn_batch")))?;
                sim.despawn_batch(&frame.as_bytes())
                    .map_err(Error::runtime)?
            };
            let out = lua.create_table()?;
            out.raw_set("despawned", removed)?;
            Ok(out)
        })?,
    )?;

    let st = Rc::clone(state);
    table.raw_set(
        "insert_batch",
        lua.create_function(move |lua, frame: mlua::String| -> Result<Table, Error> {
            let slot = Rc::clone(&st.borrow().sim);
            let (entities, values) = {
                let mut guard = slot.borrow_mut();
                let sim = guard
                    .as_mut()
                    .ok_or_else(|| Error::runtime(sim_unmounted("sim.insert_batch")))?;
                let report = sim
                    .insert_batch(&frame.as_bytes())
                    .map_err(Error::runtime)?;
                (report.entities, report.values)
            };
            let out = lua.create_table()?;
            out.raw_set("entities", entities)?;
            out.raw_set("values", values)?;
            Ok(out)
        })?,
    )?;

    let st = Rc::clone(state);
    table.raw_set(
        "status",
        lua.create_function(move |lua, ()| -> Result<Table, Error> {
            let slot = Rc::clone(&st.borrow().sim);
            let (tick, entities, archetypes, hash) = {
                let guard = slot.borrow();
                let sim = guard
                    .as_ref()
                    .ok_or_else(|| Error::runtime(sim_unmounted("sim.status")))?;
                (
                    sim.tick(),
                    sim.world().entity_count(),
                    sim.world().archetype_count(),
                    sim.hash(),
                )
            };
            let out = lua.create_table()?;
            out.raw_set("mounted", true)?;
            out.raw_set("tick", tick)?;
            out.raw_set("entities", entities)?;
            out.raw_set("archetypes", archetypes)?;
            out.raw_set("hash", format!("{hash:016x}"))?;
            Ok(out)
        })?,
    )?;

    let st = Rc::clone(state);
    table.raw_set(
        "trace",
        lua.create_function(move |lua, on: bool| -> Result<Table, Error> {
            let slot = Rc::clone(&st.borrow().sim);
            {
                let mut guard = slot.borrow_mut();
                let sim = guard
                    .as_mut()
                    .ok_or_else(|| Error::runtime(sim_unmounted("sim.trace")))?;
                sim.trace(on);
            }
            let out = lua.create_table()?;
            out.raw_set("tracing", on)?;
            Ok(out)
        })?,
    )?;

    let st = Rc::clone(state);
    table.raw_set(
        "replay",
        lua.create_function(
            move |lua, (snapshot, log): (mlua::String, mlua::String)| -> Result<Table, Error> {
                let slot = Rc::clone(&st.borrow().sim);
                let report = {
                    let guard = slot.borrow();
                    let sim = guard
                        .as_ref()
                        .ok_or_else(|| Error::runtime(sim_unmounted("sim.replay")))?;
                    sim.replay(&snapshot.as_bytes(), &log.as_bytes())
                        .map_err(Error::runtime)?
                };
                let out = lua.create_table()?;
                out.raw_set("ticks", report.ticks)?;
                out.raw_set("matched", report.matched)?;
                out.raw_set(
                    "first_mismatch",
                    match report.first_mismatch {
                        Some(tick) => LuaValue::Integer(tick as i64),
                        None => LuaValue::Nil,
                    },
                )?;
                out.raw_set("final_hash", format!("{:016x}", report.final_hash))?;
                Ok(out)
            },
        )?,
    )?;

    ygg.raw_set("sim", table)?;
    Ok(())
}

pub fn install(lua: &Lua, handles: HostHandles) -> Result<HostApi, Error> {
    let state = Rc::new(RefCell::new(HostState {
        registered: Vec::new(),
        calls: Vec::new(),
        handles,
        // VFS 的内存态：`install` 自建（不属注入面），驱通过
        // [`HostApi::vfs_handle`] 拿到同一个 `Rc` 后接入 savepoint 回滚。
        vfs: Rc::new(RefCell::new(VfsStore::default())),
        tool_runner: Rc::new(RefCell::new(None)),
        mount: Rc::new(RefCell::new(None)),
        sim: Rc::new(RefCell::new(None)),
        phase: Phase::Loading,
        sealed: false,
        // per-kernel sentinel：内核种子派生（splitmix64 四轮），Mod 面不可预测
        //（S9 审计代决：伪造首行只会成为普通错误浮出，不会被哨兵吞掉）。
        sentinel: {
            let mut s = crate::inject::KERNEL_RNG_SEED ^ 0x5345_4E54_5E4C_u64;
            let mut token = String::from("\u{1}__ygg_choice_");
            for _ in 0..4 {
                token.push_str(&format!("{:016x}", splitmix64(&mut s)));
            }
            token.push_str("__\u{1}");
            token
        },
        pre_choice_emits: 0,
        warning: None,
        meta: BTreeMap::new(),
    }));

    let ygg: Table = lua.globals().raw_get("ygg")?;

    // ---- ygg.on（装载期注册；seal 后以双检 + 原位桩拒绝，D-S9-1 / D-S9-4）----
    let st = Rc::clone(&state);
    ygg.raw_set(
        "on",
        lua.create_function(
            move |_, (event_name, handler): (String, Function)| -> Result<(), Error> {
                let mut s = st.borrow_mut();
                if s.sealed {
                    return Err(Error::runtime(SEALED_MESSAGE));
                }
                s.registered.push((event_name, handler));
                Ok(())
            },
        )?,
    )?;

    // ---- ygg.emit（收集面：校验 + fail-fast 后追加，D-S9-2）----
    let st = Rc::clone(&state);
    ygg.raw_set(
        "emit",
        lua.create_function(
            move |_, (name, payload): (String, LuaValue)| -> Result<(), Error> {
                let value = value_from_lua(&payload)?;
                // fail-fast：NaN/±∞ 在调用点即以事件名定位拒绝（不进缓冲）。
                Event::derived(name.clone(), value.clone())
                    .map_err(|e| Error::runtime(e.to_string()))?;
                st.borrow_mut().calls.push(HostCall::Emit {
                    name,
                    payload: value,
                });
                Ok(())
            },
        )?,
    )?;

    // ---- ygg.narrate（收集面，D-S9-2）----
    let st = Rc::clone(&state);
    ygg.raw_set(
        "narrate",
        lua.create_function(move |_, line: String| -> Result<(), Error> {
            st.borrow_mut().calls.push(HostCall::Narrate(line));
            Ok(())
        })?,
    )?;

    // ---- ygg.choice（收集 + sentinel 终止，D-S9-3）----
    let st = Rc::clone(&state);
    ygg.raw_set(
        "choice",
        lua.create_function(
            move |_, (prompt, options): (String, LuaValue)| -> Result<(), Error> {
                let opts = options_from_lua(&options)?;
                let sentinel = st.borrow().sentinel.clone();
                let mut state = st.borrow_mut();
                // D1：此刻缓冲中已有的 Emit 条目将因 pending_choice 终结被丢弃
                //（narrate 不丢弃，直接进叙述缓冲，不计数）。
                state.pre_choice_emits = state
                    .calls
                    .iter()
                    .filter(|call| matches!(call, HostCall::Emit { .. }))
                    .count() as u64;
                state
                    .calls
                    .push(HostCall::Choose(PendingChoice::new(prompt, opts)));
                Err(Error::runtime(sentinel))
            },
        )?,
    )?;

    // ---- ygg.meta（结构化回执面，TD1）：不进 calls 缓冲、不回放、不改 core
    //      trait；驱动转换结束后 take_meta 拼进信封。key 长度/类型与非有限值
    //      在调用点 fail-fast 拒绝（与 ygg.emit 同纪律）。----
    let st = Rc::clone(&state);
    ygg.raw_set(
        "meta",
        lua.create_function(
            move |_, (key, payload): (LuaValue, LuaValue)| -> Result<(), Error> {
                let name = meta_key_from_lua(&key)?;
                let value = meta_value_from_lua(&payload, 0)?;
                st.borrow_mut().meta.insert(name, value);
                Ok(())
            },
        )?,
    )?;

    // ---- ygg.fs.*（内容寻址的小文件面：写经 VfsStore，读 / 列是纯读）。
    //      `fs.write` 在模块顶层被影子表拒绍（effects::BANNED_TABLE_MEMBERS），
    //      `fs.read` / `fs.stat` / `fs.list` 是纯读，任何相位都允许。
    //      合约：write 的参数类型必须是 (String, String)——不猜、不隐式转换
    //      （mlua 自己的 arity/type 报错已够人读）；read miss = nil；stat miss
    //      = `{exists=false,bytes=0,sha256=""}`（**恒回表，永不 nil**）；list 是
    //      从 1 开始的**稠密数组**。
    let fs_table = lua.create_table()?;

    let fs_write = Rc::clone(&state);
    fs_table.raw_set(
        "write",
        lua.create_function(
            move |lua, (path, content): (String, String)| -> Result<Table, Error> {
                let out = lua.create_table()?;
                // 首段分派：`mounts` 走挂载桥（写穿磁盘 + 审计），其余走旧内存 VFS。
                if is_mount_path(&path) {
                    let bridge = Rc::clone(&fs_write.borrow().mount);
                    let mut slot = bridge.borrow_mut();
                    let Some(bridge) = slot.as_mut() else {
                        return Err(Error::runtime(unmounted_message("fs.write", &path)));
                    };
                    let written = bridge
                        .write(&path, content.as_bytes())
                        .map_err(Error::runtime)?;
                    out.raw_set("path", written.path)?;
                    out.raw_set("bytes", written.bytes)?;
                    out.raw_set("sha256", written.sha256)?;
                    return Ok(out);
                }
                let vfs = Rc::clone(&fs_write.borrow().vfs);
                let file = vfs
                    .borrow_mut()
                    .put(&path, content.as_bytes())
                    .map_err(Error::runtime)?;
                out.raw_set("path", path)?;
                out.raw_set("bytes", file.bytes.len())?;
                out.raw_set("sha256", file.sha256)?;
                Ok(out)
            },
        )?,
    )?;

    let fs_read = Rc::clone(&state);
    fs_table.raw_set(
        "read",
        lua.create_function(move |lua, path: String| -> Result<LuaValue, Error> {
            if is_mount_path(&path) {
                let bridge = Rc::clone(&fs_read.borrow().mount);
                let slot = bridge.borrow();
                let Some(bridge) = slot.as_ref() else {
                    return Err(Error::runtime(unmounted_message("fs.read", &path)));
                };
                let hit = bridge.read(&path).map_err(Error::runtime)?;
                return Ok(match hit {
                    Some(bytes) => LuaValue::String(lua.create_string(&bytes)?),
                    None => LuaValue::Nil, // miss = nil（与旧面同口径）
                });
            }
            let vfs = Rc::clone(&fs_read.borrow().vfs);
            let hit = vfs.borrow().get(&path).map(|file| file.bytes.clone());
            Ok(match hit {
                Some(bytes) => LuaValue::String(lua.create_string(&bytes)?),
                None => LuaValue::Nil, // miss = nil，不 throw
            })
        })?,
    )?;

    let fs_stat = Rc::clone(&state);
    fs_table.raw_set(
        "stat",
        lua.create_function(move |lua, path: String| -> Result<Table, Error> {
            let out = lua.create_table()?;
            if is_mount_path(&path) {
                let bridge = Rc::clone(&fs_stat.borrow().mount);
                let slot = bridge.borrow();
                let Some(bridge) = slot.as_ref() else {
                    return Err(Error::runtime(unmounted_message("fs.stat", &path)));
                };
                let stat = bridge.stat(&path).map_err(Error::runtime)?;
                out.raw_set("exists", stat.exists)?;
                out.raw_set("bytes", stat.bytes)?;
                out.raw_set("sha256", stat.sha256)?;
                out.raw_set("kind", stat.kind.as_str())?;
                return Ok(out);
            }
            let vfs = Rc::clone(&fs_stat.borrow().vfs);
            let hit = vfs
                .borrow()
                .get(&path)
                .map(|file| (file.bytes.len(), file.sha256.clone()));
            match hit {
                Some((bytes, sha256)) => {
                    out.raw_set("exists", true)?;
                    out.raw_set("bytes", bytes)?;
                    out.raw_set("sha256", sha256)?;
                }
                None => {
                    out.raw_set("exists", false)?;
                    out.raw_set("bytes", 0)?;
                    out.raw_set("sha256", String::new())?;
                }
            }
            Ok(out)
        })?,
    )?;

    let fs_list = Rc::clone(&state);
    fs_table.raw_set(
        "list",
        lua.create_function(move |lua, prefix: Option<String>| -> Result<Table, Error> {
            let out = lua.create_table()?;
            // 无参 = 旧面（字节级不变）；有参且以 `mounts` 起头 = 挂载面；
            // 其余前缀 → 拒（不做「按前缀列 artifacts」的第二套遍历语义）。
            if let Some(path) = prefix {
                if !is_mount_path(&path) {
                    return Err(Error::runtime(format!(
                        "{}: ygg.fs.list 的前缀必须以 `mounts` 起头（当前是 `{path}`）",
                        ygg_fs_code::JAIL
                    )));
                }
                let bridge = Rc::clone(&fs_list.borrow().mount);
                let slot = bridge.borrow();
                let Some(bridge) = slot.as_ref() else {
                    return Err(Error::runtime(unmounted_message("fs.list", &path)));
                };
                let rows = bridge.list(&path).map_err(Error::runtime)?;
                for (index, row) in rows.into_iter().enumerate() {
                    let table = lua.create_table()?;
                    table.raw_set("name", row.name)?;
                    table.raw_set("path", row.path)?;
                    table.raw_set("bytes", row.bytes)?;
                    table.raw_set("sha256", String::new())?; // 不算摘要（键在场、值留白）
                    table.raw_set("kind", row.kind.as_str())?;
                    out.raw_set(index + 1, table)?; // 稠密数组：1..=n
                }
                return Ok(out);
            }
            let vfs = Rc::clone(&fs_list.borrow().vfs);
            let rows = vfs.borrow().snapshot(); // 已按 BTreeMap 序
            for (index, (path, bytes, sha256)) in rows.into_iter().enumerate() {
                let row = lua.create_table()?;
                row.raw_set("path", path)?;
                row.raw_set("bytes", bytes)?;
                row.raw_set("sha256", sha256)?;
                out.raw_set(index + 1, row)?; // **稠密数组**：1..=n
            }
            Ok(out)
        })?,
    )?;

    ygg.raw_set("fs", fs_table)?;

    // ---- ygg.tool.*（同步宿主工具桥：装载期由影子表禁止，转换期按挂载态执行或回退）----
    let tool_table = lua.create_table()?;
    let tool_call = Rc::clone(&state);
    tool_table.raw_set(
        "call",
        lua.create_function(
            move |lua, (tool_id, args_val): (String, Option<LuaValue>)| -> Result<Table, Error> {
                let trimmed = tool_id.trim();
                if trimmed.is_empty() || trimmed.len() > TOOL_ID_MAX_BYTES {
                    return Err(Error::runtime(format!(
                        "ygg.tool.call tool_id must be 1..={TOOL_ID_MAX_BYTES} bytes, got `{tool_id}`"
                    )));
                }
                let args = tool_args_from_lua(args_val.as_ref())?;
                let (phase, runner_rc) = {
                    let s = tool_call.borrow();
                    (s.phase, Rc::clone(&s.tool_runner))
                };
                if phase != Phase::Transition {
                    return Err(Error::runtime(format!(
                        "ygg.tool.call is only allowed during transition phase (current phase: {phase:?})"
                    )));
                }
                let mut runner_slot = runner_rc.borrow_mut();
                let out = lua.create_table()?;
                match runner_slot.as_mut() {
                    None => {
                        out.raw_set("ok", false)?;
                        out.raw_set("unmounted", true)?;
                        out.raw_set("code", -1)?;
                        out.raw_set("error", "no host tool runner mounted")?;
                    }
                    Some(runner) => match runner.call(trimmed, &args) {
                        Ok(data_map) => {
                            let data_table = lua.create_table()?;
                            for (k, v) in data_map {
                                data_table.raw_set(k, v)?;
                            }
                            out.raw_set("ok", true)?;
                            out.raw_set("unmounted", false)?;
                            out.raw_set("code", 0)?;
                            out.raw_set("data", data_table)?;
                        }
                        Err(err_msg) => {
                            out.raw_set("ok", false)?;
                            out.raw_set("unmounted", false)?;
                            out.raw_set("code", 1)?;
                            out.raw_set("error", err_msg)?;
                        }
                    },
                }
                Ok(out)
            },
        )?,
    )?;
    ygg.raw_set("tool", tool_table)?;

    // ---- ygg.kv.*（即席状态写：core 把关方法，门拒绝为可读 mlua 错误；
    //      句柄经 HostState.handles 现取现用，字段即真实读取面）----
    let kv_table = lua.create_table()?;
    let kv_get = Rc::clone(&state);
    kv_table.raw_set(
        "get",
        lua.create_function(move |lua, key: String| -> Result<LuaValue, Error> {
            let kv = Rc::clone(&kv_get.borrow().handles.kv);
            let found = kv.borrow().get(&key).cloned();
            Ok(match found {
                Some(v) => value_to_lua(lua, &v)?,
                None => LuaValue::Nil, // get-miss = nil（D-S9-2 marshal 规则）
            })
        })?,
    )?;
    let kv_set = Rc::clone(&state);
    kv_table.raw_set(
        "set",
        lua.create_function(
            move |_, (key, payload): (String, LuaValue)| -> Result<(), Error> {
                let value = value_from_lua(&payload)?;
                let kv = Rc::clone(&kv_set.borrow().handles.kv);
                let outcome = kv
                    .borrow_mut()
                    .set(key.clone(), value)
                    .map_err(|e| Error::runtime(e.to_string()));
                outcome
            },
        )?,
    )?;
    ygg.raw_set("kv", kv_table)?;

    // ---- ygg.entity.*（即席状态写；get 类 miss = nil / false，写类 miss = 可读错误）----
    let entity_table = lua.create_table()?;
    let ent_spawn = Rc::clone(&state);
    entity_table.raw_set(
        "spawn",
        lua.create_function(move |_, ()| -> Result<i64, Error> {
            let entities = Rc::clone(&ent_spawn.borrow().handles.entities);
            let id = entities
                .borrow_mut()
                .spawn()
                .map_err(|e| Error::runtime(e.to_string()))?;
            i64::try_from(id).map_err(|_| Error::runtime("entity id overflow"))
        })?,
    )?;
    let ent_get = Rc::clone(&state);
    entity_table.raw_set(
        "get_attribute",
        lua.create_function(
            move |lua, (id, name): (u64, String)| -> Result<LuaValue, Error> {
                let entities = Rc::clone(&ent_get.borrow().handles.entities);
                let found = entities
                    .borrow()
                    .get(id)
                    .and_then(|entity| entity.attribute(&name).cloned());
                Ok(match found {
                    Some(v) => value_to_lua(lua, &v)?,
                    None => LuaValue::Nil,
                })
            },
        )?,
    )?;
    let ent_set = Rc::clone(&state);
    entity_table.raw_set(
        "set_attribute",
        lua.create_function(
            move |_, (id, name, payload): (u64, String, LuaValue)| -> Result<(), Error> {
                let value = value_from_lua(&payload)?;
                let entities = Rc::clone(&ent_set.borrow().handles.entities);
                let outcome = entities
                    .borrow_mut()
                    .set_attribute(id, name, value)
                    .map_err(|e| Error::runtime(e.to_string()));
                outcome
            },
        )?,
    )?;
    let ent_has = Rc::clone(&state);
    entity_table.raw_set(
        "has_tag",
        lua.create_function(move |_, (id, tag): (u64, String)| -> Result<bool, Error> {
            let entities = Rc::clone(&ent_has.borrow().handles.entities);
            Ok(ent_get_tag(&entities, id, &tag))
        })?,
    )?;
    let ent_add = Rc::clone(&state);
    entity_table.raw_set(
        "add_tag",
        lua.create_function(move |_, (id, tag): (u64, String)| -> Result<bool, Error> {
            let entities = Rc::clone(&ent_add.borrow().handles.entities);
            let changed = entities
                .borrow_mut()
                .add_tag(id, tag)
                .map_err(|e| Error::runtime(e.to_string()))?;
            Ok(changed)
        })?,
    )?;
    let ent_remove = Rc::clone(&state);
    entity_table.raw_set(
        "remove_tag",
        lua.create_function(move |_, (id, tag): (u64, String)| -> Result<bool, Error> {
            let entities = Rc::clone(&ent_remove.borrow().handles.entities);
            let removed = entities
                .borrow_mut()
                .remove_tag(id, &tag)
                .map_err(|e| Error::runtime(e.to_string()))?;
            Ok(removed)
        })?,
    )?;
    // ---- 批式五面（P-K A1；一个逐实体 getter 都没有）------------------------
    //
    // 入参是 Lua 数组（铁律② 原文「bytes/数组进出」）：n=1 的热路径不必付 pack 成本，
    // 测试夹具也不必写第二份编解码器。五面一律**先校验后改**。
    let ent_spawn_batch = Rc::clone(&state);
    entity_table.raw_set(
        "spawn_batch",
        lua.create_function(move |lua, rows: Table| -> Result<Table, Error> {
            let specs = entity_rows(&rows)?;
            let entities = Rc::clone(&ent_spawn_batch.borrow().handles.entities);
            let (ids, next_id) = {
                let mut store = entities.borrow_mut();
                let mut ids = Vec::with_capacity(specs.len());
                for spec in &specs {
                    let id = store.spawn().map_err(|e| Error::runtime(e.to_string()))?;
                    for (name, value) in &spec.attrs {
                        store
                            .set_attribute(id, name.clone(), value.clone())
                            .map_err(|e| Error::runtime(e.to_string()))?;
                    }
                    for tag in &spec.tags {
                        store
                            .add_tag(id, tag.clone())
                            .map_err(|e| Error::runtime(e.to_string()))?;
                    }
                    ids.push(id);
                }
                (ids, store.next_id())
            };
            let out = lua.create_table()?;
            out.raw_set("spawned", ids.len())?;
            out.raw_set("next_id", next_id)?;
            let list = lua.create_table()?;
            for (index, id) in ids.iter().enumerate() {
                list.raw_set(index + 1, *id)?;
            }
            out.raw_set("ids", list)?;
            Ok(out)
        })?,
    )?;

    let ent_attr_apply = Rc::clone(&state);
    entity_table.raw_set(
        "attr_apply",
        lua.create_function(move |lua, ops: Table| -> Result<Table, Error> {
            let parsed = entity_attr_ops(&ops)?;
            let entities = Rc::clone(&ent_attr_apply.borrow().handles.entities);
            let (applied, changed) = {
                let mut store = entities.borrow_mut();
                // 先校验后改：整批 id 全部在场，否则零改（错误带首个坏 id）。
                for (id, _) in &parsed {
                    if store.get(*id).is_none() {
                        return Err(entity_id_error(*id));
                    }
                }
                let mut applied = 0usize;
                let mut changed = 0usize;
                for (id, kvs) in &parsed {
                    for (name, value) in kvs {
                        // 「值是否真的变了」由内核判（含整/浮身份：`Value::Int(2) != Value::Float(2.0)`），
                        // Tier 2 据 `changed` 决定发不发事件（entity.md I6）。
                        let before = store.get(*id).and_then(|e| e.attribute(name)).cloned();
                        if before.as_ref() != Some(value) {
                            changed += 1;
                        }
                        store
                            .set_attribute(*id, name.clone(), value.clone())
                            .map_err(|e| Error::runtime(e.to_string()))?;
                        applied += 1;
                    }
                }
                (applied, changed)
            };
            let out = lua.create_table()?;
            out.raw_set("applied", applied)?;
            out.raw_set("changed", changed)?;
            Ok(out)
        })?,
    )?;

    let ent_attr_fetch = Rc::clone(&state);
    entity_table.raw_set(
        "attr_fetch",
        lua.create_function(move |lua, queries: Table| -> Result<Table, Error> {
            let parsed = entity_fetch_queries(&queries)?;
            // 先把数据拷出来再建表：不在 create_* 期间持 RefCell 借用（借用纪律同 ygg.sim）。
            let rows: Vec<(Vec<(String, Value)>, Vec<String>)> = {
                let store = entities_of(&ent_attr_fetch);
                let store = store.borrow();
                parsed
                    .iter()
                    .map(|(id, keys)| {
                        let entity = store.get(*id);
                        let values = keys
                            .iter()
                            .filter_map(|key| {
                                entity
                                    .and_then(|e| e.attribute(key))
                                    .map(|v| (key.clone(), v.clone()))
                            })
                            .collect::<Vec<_>>();
                        let tags = entity
                            .map(|e| e.tags().map(str::to_owned).collect::<Vec<_>>())
                            .unwrap_or_default();
                        (values, tags)
                    })
                    .collect()
            };
            let values_table = lua.create_table()?;
            let tags_table = lua.create_table()?;
            for (index, (values, tags)) in rows.iter().enumerate() {
                let row = lua.create_table()?;
                for (key, value) in values {
                    row.raw_set(key.as_str(), value_to_lua(lua, value)?)?;
                }
                values_table.raw_set(index + 1, row)?;
                let list = lua.create_table()?;
                for (position, tag) in tags.iter().enumerate() {
                    list.raw_set(position + 1, tag.as_str())?;
                }
                tags_table.raw_set(index + 1, list)?;
            }
            let out = lua.create_table()?;
            out.raw_set("values", values_table)?;
            out.raw_set("tags", tags_table)?;
            Ok(out)
        })?,
    )?;

    let ent_tag_batch = Rc::clone(&state);
    entity_table.raw_set(
        "tag_batch",
        lua.create_function(move |_, ops: Table| -> Result<usize, Error> {
            let parsed = entity_tag_ops(&ops)?;
            let entities = Rc::clone(&ent_tag_batch.borrow().handles.entities);
            let mut store = entities.borrow_mut();
            for (id, _, _) in &parsed {
                if store.get(*id).is_none() {
                    return Err(entity_id_error(*id));
                }
            }
            let mut changed = 0usize;
            for (id, tag, add) in &parsed {
                let outcome = if *add {
                    store.add_tag(*id, tag.clone())
                } else {
                    store.remove_tag(*id, tag.as_str())
                }
                .map_err(|e| Error::runtime(e.to_string()))?;
                if outcome {
                    changed += 1;
                }
            }
            Ok(changed)
        })?,
    )?;

    let ent_scan = Rc::clone(&state);
    entity_table.raw_set(
        "scan",
        lua.create_function(
            move |lua, (lo, hi, tag): (u64, u64, Option<String>)| -> Result<Table, Error> {
                if hi < lo {
                    return Err(entity_frame(format!("scan 的区间非法：lo={lo} > hi={hi}")));
                }
                let entities = Rc::clone(&ent_scan.borrow().handles.entities);
                // **吃调用方给的 [lo, hi]，绝不读 next_id**：上界是 Tier 2 的 `seq` 水位，
                // 用它挡「本包 before 出生 / 他处直接 spawn」的实体（entity.md §2 冷启动边界）。
                let ids: Vec<u64> = {
                    let store = entities.borrow();
                    store
                        .iter()
                        .filter(|e| e.id >= lo && e.id <= hi)
                        .filter(|e| match &tag {
                            Some(needle) => e.has_tag(needle.as_str()),
                            None => true,
                        })
                        .map(|e| e.id)
                        .collect()
                };
                let out = lua.create_table()?;
                let list = lua.create_table()?;
                for (index, id) in ids.iter().enumerate() {
                    list.raw_set(index + 1, *id)?;
                }
                out.raw_set("ids", list)?;
                Ok(out)
            },
        )?,
    )?;

    // ---- 控制面两键（S9）：存在性 + 真删 ------------------------------------
    //
    // 形状是「单标量进出」：各吃一个 u64、回一个 bool，n=1 热路径不必构造帧
    // （批式化它按 packages/entity.md §9.7 实测是负收益：表形 485 ns/8 值贵过
    // 8 次逐项穿越 ≈320 ns）。两键**不新增错误码**：三族（FRAME / VALUE / ID）
    // 只属批式面——批式拒绝是为了「整批零副作用」，单条删除没有半截状态，
    // 缺失 id 是正常答案（`false`），不是错误（契约：kernel/lua-api.md §3.1）。
    let ent_exists = Rc::clone(&state);
    entity_table.raw_set(
        "exists",
        lua.create_function(move |_, id: u64| -> Result<bool, Error> {
            let entities = entities_of(&ent_exists);
            let present = entities.borrow().contains(id);
            Ok(present)
        })?,
    )?;
    let ent_destroy = Rc::clone(&state);
    entity_table.raw_set(
        "destroy",
        lua.create_function(move |_, id: u64| -> Result<bool, Error> {
            let entities = entities_of(&ent_destroy);
            // 真删：属性 / tag / 记录一并回收；`next_id` 不动 ⇒ id 永不复用（I1）。
            let removed = entities.borrow_mut().remove(id).is_some();
            Ok(removed)
        })?,
    )?;

    // ---- 查询面（S24）：一次跨界、内核全扫（可走索引时走索引）、id 升序 --------
    //
    // 两阶段材质化（借用纪律同 `attr_fetch`）：阶段 1 持借用、零 Lua 调用（拷 id 与
    // 投影值的连续 Vec）；阶段 2 无借用、只建表。`count` 复用阶段 1 长度，不重数。
    let ent_query = Rc::clone(&state);
    entity_table.raw_set(
        "query",
        lua.create_function(move |lua, selector: Table| -> Result<Table, Error> {
            let selector = entity_query_selector(&selector)?;
            let entities = entities_of(&ent_query);
            let (count, ids, values) = {
                let mut store = entities.borrow_mut();
                let rows = store.select(&selector);
                let count = rows.len();
                let stride = selector.select.len();
                let mut ids: Vec<u64> = Vec::with_capacity(count);
                let mut values: Vec<Option<Value>> = Vec::with_capacity(count * stride);
                for entity in &rows {
                    ids.push(entity.id);
                    for key in &selector.select {
                        values.push(entity.attribute(key).cloned());
                    }
                }
                (count, ids, values)
            };
            let out = lua.create_table()?;
            out.raw_set("count", count)?;
            if selector.select.is_empty() {
                let list = lua.create_sequence_from(ids.iter().copied())?;
                out.raw_set("ids", list)?;
            } else {
                let stride = selector.select.len();
                let list = lua.create_table_with_capacity(count, 0)?;
                for (row, id) in ids.iter().enumerate() {
                    let record = lua.create_table_with_capacity(0, 1 + stride)?;
                    record.raw_set("id", *id)?;
                    for (slot, key) in selector.select.iter().enumerate() {
                        // 缺属性 ⇒ 行上**不设键**（`row.hp == nil`，与 `attr_fetch` 同一条纪律）
                        if let Some(value) = &values[row * stride + slot] {
                            record.raw_set(key.as_str(), value_to_lua(lua, value)?)?;
                        }
                    }
                    list.raw_set(row + 1, record)?;
                }
                out.raw_set("rows", list)?;
            }
            Ok(out)
        })?,
    )?;

    ygg.raw_set("entity", entity_table)?;

    install_sim(lua, &state)?;
    // `ygg.codec.*`（Tier 2 pkg.codec 的原生加速面；契约 lua-api.md §3.1.4）。
    // 无状态、无实例，所以没有 `set_*` 注入面，也不需要 CLI 激活。
    crate::codec_face::install(lua)?;

    Ok(HostApi {
        lua: lua.clone(),
        state,
    })
}

/// `ygg.tool.call` 的可选参数表 → `BTreeMap<String, String>`（标量转字符串）。
fn tool_args_from_lua(value: Option<&LuaValue>) -> Result<BTreeMap<String, String>, Error> {
    let mut out = BTreeMap::new();
    let Some(val) = value else {
        return Ok(out);
    };
    let table = match val {
        LuaValue::Nil => return Ok(out),
        LuaValue::Table(t) => t,
        other => {
            return Err(Error::runtime(format!(
                "ygg.tool.call args must be a table or nil, got {}",
                lua_type_name(other)
            )))
        }
    };
    for pair in table.pairs::<LuaValue, LuaValue>() {
        let (k, v) = pair?;
        let key = match &k {
            LuaValue::String(s) => s.to_str()?.to_owned(),
            LuaValue::Integer(i) => i.to_string(),
            other => {
                return Err(Error::runtime(format!(
                    "ygg.tool.call args key must be a string, got {}",
                    lua_type_name(other)
                )))
            }
        };
        let val_str = match &v {
            LuaValue::String(s) => s.to_str()?.to_owned(),
            LuaValue::Integer(i) => i.to_string(),
            LuaValue::Number(n) => {
                if !n.is_finite() {
                    return Err(Error::runtime("ygg.tool.call args value must be finite"));
                }
                n.to_string()
            }
            LuaValue::Boolean(b) => b.to_string(),
            LuaValue::Nil => continue,
            other => {
                return Err(Error::runtime(format!(
                    "ygg.tool.call args[`{key}`] must be a scalar, got {}",
                    lua_type_name(other)
                )))
            }
        };
        out.insert(key, val_str);
    }
    Ok(out)
}

/// `has_tag` 的 store 层读取（store 无 has_tag 时经 Entity 只读面；miss = false）。
fn ent_get_tag(entities: &Rc<RefCell<EntityStore>>, id: u64, tag: &str) -> bool {
    entities
        .borrow()
        .get(id)
        .map(|e| e.has_tag(tag))
        .unwrap_or(false)
}

/// `ygg.meta` 的键校验：必须是长度 1..=`META_KEY_MAX_BYTES` 的字符串
/// （非字符串 / 空串 / 超长均为可读报错；键是回执字段名，不承载数据）。
fn meta_key_from_lua(key: &LuaValue) -> Result<String, Error> {
    let name = match key {
        LuaValue::String(s) => s.to_str()?.to_owned(),
        other => {
            return Err(Error::runtime(format!(
                "ygg.meta key must be a string, got {}",
                lua_type_name(other)
            )))
        }
    };
    if name.is_empty() {
        return Err(Error::runtime("ygg.meta key must not be empty"));
    }
    if name.len() > META_KEY_MAX_BYTES {
        return Err(Error::runtime(format!(
            "ygg.meta key `{name}` is too long: {} bytes exceeds limit {META_KEY_MAX_BYTES}",
            name.len()
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(Error::runtime(format!(
            "ygg.meta key `{name}` must not contain control characters"
        )));
    }
    Ok(name)
}

/// `ygg.meta` 的值转换：标量走 [`value_from_lua`]（同一套四标量校验），
/// 表递归成 JSON 序列/对象（深度封顶 [`META_MAX_DEPTH`]）。
/// 数值纪律与 core 写入门同源：非有限值拒绝（`-0.0` 经 `normalize_f64`
/// 规范化——meta 是回执面，跨进程字节稳定由规范形承担）。
fn meta_value_from_lua(value: &LuaValue, depth: usize) -> Result<MetaValue, Error> {
    if let LuaValue::Table(table) = value {
        if depth >= META_MAX_DEPTH {
            return Err(Error::runtime(format!(
                "ygg.meta value nesting exceeds {META_MAX_DEPTH} levels"
            )));
        }
        return meta_table_to_json(table, depth + 1);
    }
    meta_scalar_from_lua(value)
}

/// 标量 leaf：`value_from_lua` 把关类型，此处补浮点纪律（非有限值拒绝 +
/// 规范形）。
fn meta_scalar_from_lua(value: &LuaValue) -> Result<MetaValue, Error> {
    Ok(match value_from_lua(value)? {
        Value::Int(v) => MetaValue::from(v),
        Value::Float(v) => {
            if !v.is_finite() {
                return Err(Error::runtime(
                    "ygg.meta value is non-finite (NaN / +inf / -inf are rejected)",
                ));
            }
            match serde_json::Number::from_f64(normalize_f64(v)) {
                Some(number) => MetaValue::Number(number),
                None => {
                    return Err(Error::runtime(
                        "ygg.meta value is non-finite (NaN / +inf / -inf are rejected)",
                    ))
                }
            }
        }
        Value::Bool(v) => MetaValue::Bool(v),
        Value::Str(v) => MetaValue::String(v),
    })
}

/// Lua 表 → JSON：稠密 `1..n` 数值键 = 序列；纯字符串键 = 对象；
/// 混合 / 稀疏数值键 / 其他键型都是可读报错（**不做静默猜测**：
/// 猜测会让同一 Mod 在不同输入下产出不同形状的回执）。
///
/// **空表 = 空序列 `[]`**：Lua 无法区分空表 / 空序列，取「空列表」这一更常用
/// 语义（`conflicts = {}` 读成 `[]`）；需要空对象时给一个键（或读侧按缺键处理）。
fn meta_table_to_json(table: &Table, depth: usize) -> Result<MetaValue, Error> {
    let len = table.raw_len();
    let mut int_keys: Vec<i64> = Vec::new();
    let mut str_keys: Vec<String> = Vec::new();
    for pair in table.pairs::<LuaValue, LuaValue>() {
        let (key, _) = pair?;
        match &key {
            LuaValue::Integer(i) => int_keys.push(*i),
            LuaValue::Number(n) if n.fract() == 0.0 && n.is_finite() => {
                int_keys.push(*n as i64);
            }
            LuaValue::String(s) => str_keys.push(s.to_str()?.to_owned()),
            other => {
                return Err(Error::runtime(format!(
                    "ygg.meta table keys must be integers or strings, got {}",
                    lua_type_name(other)
                )))
            }
        }
    }
    if str_keys.is_empty() {
        int_keys.sort_unstable();
        let dense = int_keys.len() == len
            && int_keys
                .iter()
                .enumerate()
                .all(|(position, key)| *key == position as i64 + 1);
        if !dense {
            return Err(Error::runtime(
                "ygg.meta table is not a dense 1..n sequence (numeric keys must be exactly 1..n)",
            ));
        }
        let mut out = Vec::with_capacity(len);
        for index in 1..=len {
            let item: LuaValue = table.raw_get(index)?;
            out.push(meta_value_from_lua(&item, depth)?);
        }
        return Ok(MetaValue::Array(out));
    }
    if !int_keys.is_empty() {
        return Err(Error::runtime(
            "ygg.meta table mixes array indices and string keys",
        ));
    }
    let mut map = JsonMap::new();
    for key in str_keys {
        let item: LuaValue = table.raw_get(key.as_str())?;
        map.insert(key, meta_value_from_lua(&item, depth)?);
    }
    Ok(MetaValue::Object(map))
}

impl HostApi {
    /// 分相收口（D-S9-1）：置 sealed 标志并把 `ygg.on` 原位替换为可读报错桩
    /// （运行期注册禁令）。重复 seal 为可读错误。
    pub fn seal(&self, lua: &Lua) -> Result<(), Error> {
        {
            let mut s = self.state.borrow_mut();
            if s.sealed {
                return Err(Error::runtime("host api is already sealed"));
            }
            s.sealed = true;
            s.phase = Phase::Idle;
        }
        let ygg: Table = lua.globals().raw_get("ygg")?;
        let stub = lua.create_function(|_, _: (String, Function)| -> Result<(), Error> {
            Err(Error::runtime(SEALED_MESSAGE))
        })?;
        ygg.raw_set("on", stub)?;
        Ok(())
    }

    /// 把装载期收集的注册按 `ygg.on` 调用序逐条装入注册表（D-S9-4；
    /// 每条一个 [`LuaHandler`]，注册序 = 派发序）。在 seal 之后调用。
    pub fn register_into(&self, registry: &mut HandlerRegistry, mod_id: &str) {
        let entries: Vec<(String, Function)> = self.state.borrow().registered.clone();
        for (event_name, function) in entries {
            registry.register(
                mod_id,
                event_name.clone(),
                Box::new(LuaHandler {
                    lua: self.lua.clone(),
                    function,
                    state: Rc::clone(&self.state),
                }),
            );
        }
    }

    /// 取走本次 dispatch 累积的人读警告（D1；驱动在事件路径结束时 drain，
    /// 每次 dispatch 至多一条，信封仅在非空时加 `warnings` 键）。
    pub fn take_dispatch_warning(&self) -> Option<String> {
        self.state.borrow_mut().warning.take()
    }

    /// 取走本次 dispatch 累积的结构化 meta（TD1；**取走即清空**）。驱动在
    /// 转换结束后 drain：非空才在信封加 `meta` 键，空则保持既有线格式。
    pub fn take_meta(&self) -> BTreeMap<String, MetaValue> {
        std::mem::take(&mut self.state.borrow_mut().meta)
    }

    /// 清空结构化 meta（驱动在每次 dispatch **入口**调用：上一条的残留
    /// 不得混进本条的收据）。
    pub fn clear_meta(&self) {
        self.state.borrow_mut().meta.clear();
    }

    /// 装载期收集的事件名（按注册序；诊断面）。
    pub fn registered_event_names(&self) -> Vec<String> {
        self.state
            .borrow()
            .registered
            .iter()
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// `ygg.fs` 的内存态句柄（驱动拿去接入 dispatch 级 savepoint 基线）。
    ///
    /// 返回与 Lua 阴面**同一个** `Rc`：驱动克隆出来做基线、回滚时写回，
    /// Lua 侧下一次 `ygg.fs.read` 就读到回滚后的状态。
    pub fn vfs_handle(&self) -> Rc<RefCell<VfsStore>> {
        Rc::clone(&self.state.borrow().vfs)
    }

    /// 挂载或卸载宿主侧同步工具执行器。
    pub fn set_tool_runner(&self, runner: Option<Box<dyn HostToolRunner>>) {
        *self.state.borrow().tool_runner.borrow_mut() = runner;
    }

    /// 挂载或卸载宿主侧文件挂载桥（`ygg.fs.*` 的 `mounts/` 面）。
    pub fn set_mount_fs(&self, bridge: Option<Box<dyn MountFs>>) {
        *self.state.borrow().mount.borrow_mut() = bridge;
    }

    /// 取文件挂载桥句柄（驱动层共享）。
    pub fn mount_fs_handle(&self) -> Rc<RefCell<Option<Box<dyn MountFs>>>> {
        Rc::clone(&self.state.borrow().mount)
    }

    /// 注入或卸载模拟内核实例（`ygg.sim.*` 的宿主面，Tier 1）。
    ///
    /// 未注入时 10 个函数全部抛 `YGG-SIM-UNMOUNTED`——**不是** `nil` / `false`：
    /// `nil` 与「世界是空的」不可分辨，静默降级会让 Mod 把「宿主没这个能力」读成
    /// 「世界为空」并据此写下错误状态。
    ///
    /// 具体类型而不是 trait object：只有一份实现（`ygg_sim::Sim`），`dyn` 只会给
    /// 每次调用加一层 vtable（AGENTS §1.7「能不加就不加」）。
    pub fn set_sim(&self, sim: Option<Box<ygg_sim::Sim>>) {
        *self.state.borrow().sim.borrow_mut() = sim;
    }

    /// 取模拟内核句柄（驱动层共享；宿主批量帧走这条，不经 Lua 边界）。
    pub fn sim_handle(&self) -> Rc<RefCell<Option<Box<ygg_sim::Sim>>>> {
        Rc::clone(&self.state.borrow().sim)
    }

    /// 取同步工具执行器句柄（驱动层共享）。
    pub fn tool_runner_handle(&self) -> Rc<RefCell<Option<Box<dyn HostToolRunner>>>> {
        Rc::clone(&self.state.borrow().tool_runner)
    }
}

// ============================================================================
// §crate 内单测：`VfsStore` 的账本（revision / dirty / load）与路径门向量表
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 路径门向量表——**与 `ygg-store/src/lib.rs` 的同名用例逐字同步**。
    ///
    /// 两份门的存在是有意的（`ygg-store` 是 dev-dependency，运行时无权调用）；
    /// 这份向量表就是漂移锤子：规则只改一边时必有一侧红。
    fn path_gate_vectors() -> (Vec<String>, Vec<String>) {
        let max_ok = format!(
            "artifacts/{}.md",
            "a".repeat(VFS_PATH_MAX_BYTES - VFS_PREFIX.len() - 3)
        );
        let too_long = format!(
            "artifacts/{}.md",
            "a".repeat(VFS_PATH_MAX_BYTES - VFS_PREFIX.len() - 2)
        );
        let accepted = vec![
            "artifacts/a.md".to_owned(),
            "artifacts/notes/x.md".to_owned(),
            "artifacts/a.b-c_d/e.md".to_owned(),
            "artifacts/0.md".to_owned(),
            "artifacts/a/b/c/d.md".to_owned(),
            max_ok,
        ];
        let rejected = vec![
            "artifacts/".to_owned(),
            "artifacts//x".to_owned(),
            "artifacts/x/".to_owned(),
            "artifacts/../x".to_owned(),
            "artifacts/a/../../x".to_owned(),
            "artifacts/a..b".to_owned(),
            "artifact/x".to_owned(),
            "/artifacts/x".to_owned(),
            "artifacts/a b.md".to_owned(),
            "artifacts/a\\b.md".to_owned(),
            "artifacts/a|b.md".to_owned(),
            "artifacts/a@b.md".to_owned(),
            too_long,
            String::new(),
            "artifacts".to_owned(),
        ];
        (accepted, rejected)
    }

    #[test]
    fn path_gate_accepts_legal_shapes() {
        let (accepted, _) = path_gate_vectors();
        for path in accepted {
            validate_vfs_path(&path).unwrap_or_else(|reason| {
                panic!("`{path}`（{} 字节）必须放行：{reason}", path.len())
            });
        }
        // 边界自证：最长向量恰好是上限（否则「刚好 128」根本没被测到）。
        assert_eq!(
            format!(
                "artifacts/{}.md",
                "a".repeat(VFS_PATH_MAX_BYTES - VFS_PREFIX.len() - 3)
            )
            .len(),
            VFS_PATH_MAX_BYTES
        );
    }

    #[test]
    fn path_gate_rejects_illegal_shapes() {
        let (_, rejected) = path_gate_vectors();
        for path in rejected {
            let reason = validate_vfs_path(&path).expect_err(&format!("`{path}` 必须被拒"));
            assert!(!reason.is_empty(), "拒绝必须带人读原因");
        }
    }

    /// `put` 的账本：revision +1、入 dirty、内容与摘要正确。
    #[test]
    fn put_bookkeeps_revision_and_dirty() {
        let mut vfs = VfsStore::default();
        assert!(vfs.is_empty());
        let file = vfs.put("artifacts/a.md", b"first").expect("写入");
        assert_eq!(file.bytes, b"first".to_vec());
        assert_eq!(file.sha256.len(), 64, "小写十六进制 64 字符");
        assert_eq!(vfs.len(), 1);
        assert_eq!(vfs.revision(), 1);
        assert_eq!(
            vfs.get("artifacts/a.md").map(|f| f.bytes.clone()),
            Some(b"first".to_vec())
        );

        // 覆盖写：仍然只有一件，revision 继续增。
        vfs.put("artifacts/a.md", b"second").expect("覆盖");
        assert_eq!(vfs.len(), 1);
        assert_eq!(vfs.revision(), 2);
        assert_eq!(
            vfs.get("artifacts/a.md").map(|f| f.bytes.clone()),
            Some(b"second".to_vec())
        );
    }

    /// `put` 的两道门：坏路径 / 超限内容都不建键、不动 revision。
    #[test]
    fn put_rejects_bad_path_and_oversize_content() {
        let mut vfs = VfsStore::default();
        let err = vfs.put("artifacts/../x", b"y").expect_err("穿越必须被拒");
        assert!(err.starts_with("ygg.fs.write 拒绝："), "{err}");
        let over = vec![0u8; VFS_CONTENT_MAX_BYTES + 1];
        let err = vfs
            .put("artifacts/big.md", &over)
            .expect_err("超限必须被拒");
        assert!(err.contains("256 KiB"), "错误必须给出上限：{err}");
        // 恰好上限放行（否则边界不可验证）。
        vfs.put("artifacts/max.md", &vec![0u8; VFS_CONTENT_MAX_BYTES])
            .expect("恰好上限必须放行");
        assert_eq!(vfs.len(), 1, "被拒的写不得建键");
        assert_eq!(vfs.revision(), 1, "被拒的写不得动 revision");
    }

    /// `load` 是预载面：不 dirty、不动 revision（它不是本次会话的写入）。
    #[test]
    fn load_is_not_dirty_and_does_not_bump_revision() {
        let mut vfs = VfsStore::default();
        vfs.load("artifacts/pre.md", b"preset", "deadbeef");
        assert_eq!(vfs.len(), 1);
        assert_eq!(vfs.revision(), 0, "预载不得当成本次写入");
        assert!(vfs.take_dirty().is_empty(), "预载不得入 dirty");
        assert_eq!(
            vfs.get("artifacts/pre.md").map(|f| f.sha256.clone()),
            Some("deadbeef".to_owned()),
            "预载的摘要原样接受（不重算——盘上那份才是权威）"
        );
        // 非法路径的预载静默跳过：不建键，也不污染键空间。
        vfs.load("../escape.md", b"x", "y");
        assert_eq!(vfs.len(), 1);
    }

    /// `take_dirty` 按 `BTreeMap` 序排空，且排空后不复报。
    #[test]
    fn take_dirty_is_ordered_and_drains() {
        let mut vfs = VfsStore::default();
        for path in ["artifacts/z.md", "artifacts/a.md", "artifacts/m.md"] {
            vfs.put(path, path.as_bytes()).expect("写入");
        }
        let dirty = vfs.take_dirty();
        assert_eq!(
            dirty
                .iter()
                .map(|(path, _)| path.as_str())
                .collect::<Vec<_>>(),
            vec!["artifacts/a.md", "artifacts/m.md", "artifacts/z.md"]
        );
        assert!(vfs.take_dirty().is_empty(), "排空后不得重复上报");
        assert_eq!(vfs.len(), 3, "排空 dirty 不删文件");
    }

    /// `snapshot`（=`ygg.fs.list` 的回执形状）按路径升序，与写入序无关。
    #[test]
    fn snapshot_is_sorted_by_path() {
        let mut vfs = VfsStore::default();
        for path in ["artifacts/b.md", "artifacts/a.md"] {
            vfs.put(path, b"x").expect("写入");
        }
        let rows: Vec<String> = vfs
            .snapshot()
            .into_iter()
            .map(|(path, bytes, sha)| format!("{path}:{bytes}:{}", sha.len()))
            .collect();
        assert_eq!(rows, vec!["artifacts/a.md:1:64", "artifacts/b.md:1:64"]);
    }

    /// 克隆语义：`VfsStore` 整体可克隆（savepoint 基线的加载前提）——
    /// 副本与原件独立，改副本不影响原件。
    #[test]
    fn clone_is_an_independent_snapshot() {
        let mut vfs = VfsStore::default();
        vfs.put("artifacts/a.md", b"one").expect("写入");
        let mut baseline = vfs.clone();
        vfs.put("artifacts/b.md", b"two").expect("后续写入");
        assert_eq!(vfs.len(), 2);
        assert_eq!(baseline.len(), 1, "基线必须冻结在克隆时点");
        assert_eq!(baseline.revision(), 1);
        // dirty 也随基线克隆：基线里只有克隆时点那一条，所以回滚到它
        // **不会**把失败步的路径带进下一次 flush。
        assert_eq!(
            baseline
                .take_dirty()
                .iter()
                .map(|(path, _)| path.as_str())
                .collect::<Vec<_>>(),
            vec!["artifacts/a.md"]
        );
    }

    struct MockRunner;
    impl HostToolRunner for MockRunner {
        fn call(
            &mut self,
            tool_id: &str,
            args: &BTreeMap<String, String>,
        ) -> Result<BTreeMap<String, String>, String> {
            if tool_id == "mock.fail" {
                return Err("boom".to_owned());
            }
            let mut out = BTreeMap::new();
            out.insert("tool".to_owned(), tool_id.to_owned());
            if let Some(v) = args.get("k") {
                out.insert("echo".to_owned(), v.clone());
            }
            Ok(out)
        }
    }

    #[test]
    fn ygg_tool_call_respects_phase_and_runner_mount_state() {
        let sandbox = crate::sandbox::Sandbox::new().expect("sandbox");
        let lua = sandbox.lua().clone();
        let handles = HostHandles {
            kv: Rc::new(RefCell::new(KvStore::new())),
            entities: Rc::new(RefCell::new(EntityStore::new())),
        };
        let host = install(&lua, handles).expect("install host");
        // Loading 相位下直接调用 ygg.tool.call 必须被拒。
        let loading_err = lua
            .load(r#"ygg.tool.call("mock.ok", { k = "v" })"#)
            .exec()
            .expect_err("loading phase must reject ygg.tool.call");
        assert!(
            loading_err.to_string().contains("transition phase"),
            "{loading_err}"
        );

        lua.load(
            r#"
            ygg.on("test.call", function(tool_id)
                local res = ygg.tool.call(tool_id, { k = "val", n = 42, b = true })
                ygg.kv.set("ok", res.ok == true)
                ygg.kv.set("unmounted", res.unmounted == true)
                ygg.kv.set("code", res.code or -99)
                if res.data then
                    ygg.kv.set("echo", res.data.echo or "")
                end
                if res.error then
                    ygg.kv.set("err", res.error)
                end
            end)
        "#,
        )
        .exec()
        .expect("register handler");
        host.seal(&lua).expect("seal");
        let mut registry = HandlerRegistry::new();
        host.register_into(&mut registry, "mod");
        let mut entities = EntityStore::new();
        let mut fuel = crate::fuel::install(&lua, 10_000).expect("fuel");

        // 1) 未挂载：返回 unmounted = true, ok = false, code = -1
        ygg_core::transition::run_transition(
            &mut entities,
            &mut registry,
            &mut fuel,
            Event::external("test.call", Value::Str("mock.ok".to_owned())).expect("event"),
        )
        .expect("unmounted transition");
        assert_eq!(
            host.state.borrow().handles.kv.borrow().get("unmounted"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            host.state.borrow().handles.kv.borrow().get("ok"),
            Some(&Value::Bool(false))
        );
        assert_eq!(
            host.state.borrow().handles.kv.borrow().get("code"),
            Some(&Value::Int(-1))
        );

        // 2) 挂载后成功调用：ok = true, unmounted = false, code = 0, data 映射回传
        host.set_tool_runner(Some(Box::new(MockRunner)));
        fuel.reset(10_000);
        ygg_core::transition::run_transition(
            &mut entities,
            &mut registry,
            &mut fuel,
            Event::external("test.call", Value::Str("mock.ok".to_owned())).expect("event"),
        )
        .expect("mounted ok transition");
        assert_eq!(
            host.state.borrow().handles.kv.borrow().get("unmounted"),
            Some(&Value::Bool(false))
        );
        assert_eq!(
            host.state.borrow().handles.kv.borrow().get("ok"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            host.state.borrow().handles.kv.borrow().get("echo"),
            Some(&Value::Str("val".to_owned()))
        );

        // 3) 挂载后返回 Err：ok = false, unmounted = false, code = 1, error 回传
        fuel.reset(10_000);
        ygg_core::transition::run_transition(
            &mut entities,
            &mut registry,
            &mut fuel,
            Event::external("test.call", Value::Str("mock.fail".to_owned())).expect("event"),
        )
        .expect("mounted err transition");
        assert_eq!(
            host.state.borrow().handles.kv.borrow().get("ok"),
            Some(&Value::Bool(false))
        );
        assert_eq!(
            host.state.borrow().handles.kv.borrow().get("code"),
            Some(&Value::Int(1))
        );
        assert_eq!(
            host.state.borrow().handles.kv.borrow().get("err"),
            Some(&Value::Str("boom".to_owned()))
        );
    }
}
