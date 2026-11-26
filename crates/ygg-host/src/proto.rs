//! `ygg/1` 信封的字段契约：消息联合 + 支撑类型 + 边界浮点拒收。
//!
//! 契约：`artifacts/w2-m3-protocol-plan.md` §1 消息表 + 本 crate 的
//! `packages/schema/protocol/README.md`（字段表 + 三条不变式）。本模块只做一件事：把信封
//! 的字段形状钉成 Rust 类型，并让**任何**到达世界值的 JSON 数字在边界上被检查一次。
//!
//! # 五条已定决策落在这里的哪一面
//!
//! 1. **两个 crate 边界不变**（`ygg-core` / `ygg-sim`）：本 crate 不依赖任何一个，
//!    连 `EntityId` / `ComponentId` 都是本地别名（[`EntityId`] / [`ComponentId`]），
//!    数值宽度与 `ygg-sim` 的 `component.rs` 逐位一致——协议是**线**契约，不是内核类型面。
//! 2. **v1 只做 JSON**：[`decode`] / [`encode`] 直接用 `serde_json`。`hello` 里的
//!    [`CODEC`] 留字面量，未来 CBOR 走版本协商，不在 v1 加分支。
//! 3. **浮点一律拒收**（[`strict`] 模块）：整数字段全部走 `serde_json::Value` 再判
//!    `as_u64` / `as_i64`，`1.5` 与 `1.0` 都拒（不四舍五入）。错误文本以 `E_PROTOCOL`
//!    开头，[`ProtocolError::code`] 是**结构化**的同一个码。
//! 4. **协议层不持有世界**：本模块没有任何「能不能 spawn / 能不能 step」的判断。
//!    `hello` 的版本 / 编码 / 摘要**格式**在解码期就判（那是信封形状的一部分，本 crate
//!    知道自己说的是 `ygg/1`）；摘要**内容**是否等于本地包则归会话层（`E_PACK`，计划
//!    §3 步 2）——协议层无从得知这个进程装的是哪个包。
//! 5. **一个宿主连接一个世界实例**：信封里因此没有会话 / 世界 id 字段。
//!
//! # 为什么 `deny_unknown_fields` 是纪律而不是洁癖
//!
//! 信封是跨语言契约（TS / GDScript / C# 手写）。少一个字段名的拼写错误在这里是
//! **静默的语义漂移**：`{"type":"step","tick":1,"dt":16}` 会被读成「ticks 缺省 0」，
//! 于是宿主以为推进了一步而内核一步没走。未知字段一律 `E_PROTOCOL`，让错字在第一次
//! 往返就炸出来。
//!
//! # schema 与 Rust 的一致性由构造保证
//!
//! `packages/schema/protocol/ygg-1.schema.json` 是跨语言事实源，本模块的 derive 是它的
//! Rust 投影。两侧的同步不靠约定：`tests/schema_conformance.rs` 把**每个变体的一份样本**
//! 序列化后过 `ygg-schema` 的 draft 2020-12 校验器，另钉 `$defs` 名单。改字段而不同步
//! schema，那条用例当场红。

use serde::{Deserialize, Serialize};

/// 协议版本字面量（`hello.protocol`）。
///
/// 就这一个值：`hello` 里出现别的版本由 `strict::protocol_token` 在解码期拒绝，错误文本
/// 同时给出本 crate 的版本与对端说的版本。解码期拒绝而不是「先收下再回一条 `error`」，
/// 因为 v1 的 crate 本来就解释不了 v2 的信封——把「读不懂」留到会话层只会误报成内容问题。
pub const PROTOCOL: &str = "ygg/1";

/// v1 唯一的线格式（`hello.codec`）。CBOR 走版本协商，不在 v1 加分支（决策 2）。
pub const CODEC: &str = "json";

/// 实体 id：与 `ygg-sim::component::EntityId`（`u64`）同宽。
pub type EntityId = u64;

/// 组件 id（列下标）：与 `ygg-sim::component::ComponentId`（`u32`）同宽。
///
/// **不进快照、不进 hash、不进 Lua 面**——组件 id 是注册序的产物，宿主只应把它当作
/// [`Message::Ready`] 里给出的不透明句柄。
pub type ComponentId = u32;

// ---------------------------------------------------------------------------
// 错误面
// ---------------------------------------------------------------------------

/// `ygg/1` 的六类错误码（线上字面量 = `SCREAMING_SNAKE_CASE`）。
///
/// 六类而不是一个大 `E`，因为宿主对它们的反应不同：`E_PROTOCOL` 是宿主自己的 bug
/// （重连无意义），`E_PACK` 是包与存档不匹配（换包或换档），`E_STORE_FORMAT` 是
/// 存档格式不被支持（提示用户升级），`E_BUDGET` / `E_RULE` 是规则代码的问题
/// （报给包作者），`E_STATE` 是内核拒绝（回滚并重试）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    /// 信封本身不合法：未知 `type`、未知字段、浮点数、越界、缺失字段，以及 `hello` 里
    /// 版本 / 编码 / 摘要**格式**不合。
    EProtocol,
    /// 字节属于另一个包：`pack_hash` 与本地包不符、存档与包不符。与前一码的分界是
    /// 「写错了」对「拿错了」。
    EPack,
    /// 指令预算耗尽（沙箱侧的台阶计数）。
    EBudget,
    /// 规则代码运行期失败（Lua 报错、系统返回错误）。
    ERule,
    /// 存档格式不被支持：魔数 / `fmt_version` / flags 不是本内核能读的那一版。
    /// 与 `E_STATE` 分开是因为宿主的话术不同：前者提示用户升级存档，后者提示文件损坏。
    EStoreFormat,
    /// 内核状态拒绝：越界 tick、未 `seal`、结构操作与实体现状冲突。
    EState,
}

impl ErrorCode {
    /// 线上字面量（与 `serde` 的 `SCREAMING_SNAKE_CASE` 逐字一致）。
    ///
    /// 单点声明：[`ErrorCode::as_str`] 与 derive 的序列化必须相等，`tests/proto_roundtrip.rs`
    /// 的 `error_code_wire_labels_match` 逐码断言——两处各写一份字符串就是两份漂移面。
    pub const fn as_str(self) -> &'static str {
        match self {
            ErrorCode::EProtocol => "E_PROTOCOL",
            ErrorCode::EPack => "E_PACK",
            ErrorCode::EBudget => "E_BUDGET",
            ErrorCode::ERule => "E_RULE",
            ErrorCode::EStoreFormat => "E_STORE_FORMAT",
            ErrorCode::EState => "E_STATE",
        }
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 一条结构化错误：`error` 消息的载荷，也是 [`Message::Events`] 里 `errors` 的条目。
///
/// 同一个形状承担两个角色是刻意的——「本步有哪些错误」与「这条连接为什么断」在线上
/// 没有理由长得不一样；宿主因此只需要一个错误渲染函数。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(deny_unknown_fields)]
#[error("{code}: {msg}")]
pub struct ProtocolError {
    /// 错误类别。
    pub code: ErrorCode,
    /// 人类可读细节（中文，含出错字段的原文；不含凭据）。
    pub msg: String,
}

impl ProtocolError {
    /// 构造一条错误。
    pub fn new(code: ErrorCode, msg: impl Into<String>) -> Self {
        Self {
            code,
            msg: msg.into(),
        }
    }

    /// 边界拒收（信封不合法）的错误。
    pub fn protocol(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::EProtocol, msg)
    }
}

// ---------------------------------------------------------------------------
// 世界自述
// ---------------------------------------------------------------------------

/// 一 tick 内的三个阶段，序即执行序（Pre → Rules → Post）。
///
/// 与 `ygg-sim::system::Stage` 逐变体同义；线上用小写（`"pre"` / `"rules"` / `"post"`），
/// 因为整份信封统一 `snake_case`，混两种大小写风格只会让手写宿主多一个坑。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// 输入 / 预处理。
    Pre,
    /// 规则主体。
    Rules,
    /// 收尾。
    Post,
}

/// 一份组件的类型（Tier 2 属性袋的投影）。
///
/// 线上字面量与 `ygg-sim::component::AttrType::label` **逐字相同**
/// （`number` / `string` / `boolean` / `ref` / `tag` / `f64bits`）：宿主据它决定用哪种
/// 编码写值（[`Value`] 的四个变体按宽度分类，类型语义由本枚举补足）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    /// 定点整数列（宿主发 [`Value::Int`]）。
    Number,
    /// UTF-8 字节列（[`Value::Str`]）。
    String,
    /// 布尔列（[`Value::Bool`]）。
    Boolean,
    /// `u64` 引用列，`0` = 无引用（[`Value::Uint`]）。
    Ref,
    /// 零宽组件：只进 archetype key，没有列，值是空 [`ComponentValues::values`]。
    Tag,
    /// `u64` 位串承载的 IEEE 754 double（[`Value::Uint`]，**仅搬运不算术**）。
    #[serde(rename = "f64bits")]
    F64Bits,
}

/// 世界自述里的一份组件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Component {
    /// 组件 id（不透明句柄；`submit` 的 [`ComponentValues::cid`] 引用的就是它）。
    #[serde(deserialize_with = "strict::u32")]
    pub id: ComponentId,
    /// 组件名（Tier 2 声明名，如 `hp`）。
    pub name: String,
    /// 组件类型。
    pub kind: ComponentKind,
}

/// 世界自述里的一条系统。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemInfo {
    /// 系统名（同一世界内唯一）。
    pub name: String,
    /// 所属阶段。
    pub stage: Stage,
}

/// 一条日志行的级别。
///
/// v1 固定四级：`world.log` 的文本只有一个来源（规则代码），级别由宿主侧协议统一而非
/// 由包自定义——否则「警告」在 A 包叫 `warn`、B 包叫 `warning`，宿主的过滤逻辑就得
/// 认识每个包的词汇。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    /// 诊断细节（默认不进 UI）。
    Debug,
    /// 常规信息。
    Info,
    /// 可恢复的异常。
    Warn,
    /// 错误（不终止 tick）。
    Error,
}

/// 一条日志行（`log` 只映射 `world.log`，不是内核诊断流的通道）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogLine {
    /// 级别。
    pub level: LogLevel,
    /// 文本。
    pub text: String,
}

// ---------------------------------------------------------------------------
// 值
// ---------------------------------------------------------------------------

/// 一格世界值。**没有浮点变体**，这是决策 3 的类型面表达。
///
/// 四个变体按**列宽**分类，类型语义由 [`ComponentKind`] 补足：
///
/// | 组件类型 | 变体 | 线上形态 |
/// | --- | --- | --- |
/// | `number` | [`Value::Int`] | `{"int": -3}` |
/// | `string` | [`Value::Str`] | `{"str": "hi"}` |
/// | `boolean` | [`Value::Bool`] | `{"bool": true}` |
/// | `ref` / `f64bits` | [`Value::Uint`] | `{"uint": 7}` |
/// | `tag` | （无值） | [`ComponentValues::values`] 为空数组 |
///
/// 外部标签（`{"int": 3}`）而不是 `{"t":"int","v":3}`：一个键既是判别式又是载荷，
/// 少一层嵌套，也少一个可以拼错的键。`f64bits` 走 `uint` 是决策 3 的直接后果——
/// 宿主若要写一个 double，自己取位串，协议层不承接四舍五入。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Value {
    /// 定点整数列的一格（Q32.32 原值或普通整数，由组件声明决定）。
    Int(#[serde(deserialize_with = "strict::i64")] i64),
    /// `u64` 列的一格（`ref` 或 `f64bits`）。
    Uint(#[serde(deserialize_with = "strict::u64")] u64),
    /// 字符串列的一格。
    Str(String),
    /// 布尔列的一格。
    Bool(bool),
}

/// 一份组件在一组实体上的列值（`cid` + 逐行值）。
///
/// `values` 的**行数**必须等于本次操作的行数：`spawn` 是 `n`（每个实体一行），
/// `set` / `insert` 是 `1`。少一行不是「广播到每一行」，而是 `E_PROTOCOL`（
/// `session::column_of` 当场拒）；`tag` 组件没有列，`values` 必须为空。
/// 协议层不校验行数与内核列的对应关系——那是 `submit` 在事务里的事（回 `E_STATE`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentValues {
    /// 组件 id。
    #[serde(deserialize_with = "strict::u32")]
    pub cid: ComponentId,
    /// 逐行值（一个实体一行；`spawn` 时行数 = `n`）。
    pub values: Vec<Value>,
}

// ---------------------------------------------------------------------------
// 变更操作
// ---------------------------------------------------------------------------

/// 一条世界变更操作（`submit` 的载荷）。
///
/// 五种操作与内核的批量 API 一一对应（`spawn_columns` / `despawn_ids` / 值写入 /
/// `insert_columns` / `remove_components`），**不推进时间**：结构操作在 tick 边界之外
/// 由宿主驱动，与 `Sim::step` 里的 `defer_*` 是两条路径，共享同一份预检。
///
/// 判别键是 `op`（不是 `type`）：一个 `submit` 里既有 `type` 又有 `op` 会让两条判别
/// 轴在同一个对象里并存，宿主的手写解码器迟早把 `op:"spawn"` 写成 `type:"spawn"`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Op {
    /// 新建 `n` 个实体，每个都带同一份组件集。
    ///
    /// `n` 是显式的：批量建同一模板的实体（棋盘、50 格小人）是宿主最常做的动作，
    /// 逐实体重复一条 `spawn` 会把信封体积乘上 n 倍。新实体的 id 不在任何回执里
    /// （计划的 v1 消息表没有 `submit` 回执行），宿主靠随后的 `delta` / 快照对齐——
    /// 见 `packages/schema/protocol/README.md`「未定面」。
    Spawn {
        /// 新建实体数（≥ 1；0 是无效指令）。
        #[serde(deserialize_with = "strict::non_zero_u32")]
        n: u32,
        /// 每个新实体共用的组件集。
        comps: Vec<ComponentValues>,
    },
    /// 删除若干实体（不存在的 id 不算错误，见内核 `despawn_ids` 的返回计数）。
    Despawn {
        /// 要删除的实体 id。
        ids: Vec<EntityId>,
    },
    /// 往**已有**实体上写值（组件必须已存在；不存在是 `E_STATE`）。
    Set {
        /// 目标实体。
        #[serde(deserialize_with = "strict::u64")]
        id: EntityId,
        /// 要写值的组件集。
        comps: Vec<ComponentValues>,
    },
    /// 往**已有**实体上补组件（已存在的组件是 `E_STATE`）。
    Insert {
        /// 目标实体。
        #[serde(deserialize_with = "strict::u64")]
        id: EntityId,
        /// 要补的组件集。
        comps: Vec<ComponentValues>,
    },
    /// 从**已有**实体上摘组件（不存在的组件不算错误）。
    Remove {
        /// 目标实体。
        #[serde(deserialize_with = "strict::u64")]
        id: EntityId,
        /// 要摘的组件 id。
        cids: Vec<ComponentId>,
    },
}

/// `delta` 的一条：一个实体上被写过的那几个组件。
///
/// 比计划表更具体一层：表里写 `{eid, cids, values}`（两个平行数组），但一份组件是
/// **多列**的，平行数组无法表达「第 i 个 cid 的第 j 列」。这里收成
/// `{eid, comps:[{cid, values}]}`，长度靠结构表达，不靠下标对齐——`$data` 之外
/// JSON Schema 也没有便宜的办法约束平行数组等长。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Changed {
    /// 被写过的实体。
    #[serde(deserialize_with = "strict::u64")]
    pub eid: EntityId,
    /// 该实体上被写过的组件（只列写过的，不是全部组件）。
    pub comps: Vec<ComponentValues>,
}

// ---------------------------------------------------------------------------
// 回放
// ---------------------------------------------------------------------------

/// 回放日志的表头（`ReplayLog` 的 `seed` / `start_tick`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogHeader {
    /// 世界的随机流派生种子。
    #[serde(deserialize_with = "strict::u64")]
    pub seed: u64,
    /// 回放起点：`0` = 从创世，否则 = 快照所在 tick。
    #[serde(deserialize_with = "strict::u64")]
    pub start_tick: u64,
}

/// 回放日志的一帧：该 tick 的输入 + 录制时算出的 hash。
///
/// `dt` 在计划稿的分支类型表里没有，这里必须有：`ygg-sim::input::TickRecord` 是
/// `{tick, dt, hash, ops}`，少了 `dt` 就复现不出那一 tick 的输入，`replay` 只能证明
/// 「跑完了」而不能证明「跑对了」——与 `input.rs` 头注的论证同源。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    /// 该 tick 的序号。
    #[serde(deserialize_with = "strict::u64")]
    pub tick: u64,
    /// 该 tick 的步长（定点原值）。
    #[serde(deserialize_with = "strict::u32")]
    pub dt: u32,
    /// 该 tick 开始前的结构操作（按录制序）。
    pub ops: Vec<Op>,
    /// 录制时该 tick 结束的 state hash。
    #[serde(deserialize_with = "strict::u64")]
    pub hash: u64,
}

/// 一整个回放日志：表头 + 逐帧。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Log {
    /// 表头。
    pub header: LogHeader,
    /// 逐 tick 的帧（按 tick 升序）。
    pub frames: Vec<Frame>,
}

// ---------------------------------------------------------------------------
// 存档往返（`snapshot` 是双向同名消息，靠 `direction` 判别）
// ---------------------------------------------------------------------------

/// `snapshot` 的两个方向。
///
/// 为什么不是 `SnapshotRequest` / `SnapshotReply` 两个顶层变体：`Message` 用
/// `tag = "type"` 的内部标签，**两个变体不能共用一个标签**，而计划表里请求与回执都叫
/// `snapshot`。于是把方向做成内层判别键：请求 `{"type":"snapshot","direction":"request"}`，
/// 回执 `{"type":"snapshot","direction":"reply","tick":…,"hash":…,"bytes":…}`。
/// 请求没有载荷字段，回执的字段全部必填——`oneOf` 两支各自 `additionalProperties:false`，
/// 所以 `{"direction":"request","bytes":"…"}` 在 schema 与 Rust 两侧都拒。
///
/// 一个 serde 陷阱记在这里：`Request` 必须是**空 struct 变体** `Request {}`，不能是
/// unit 变体 `Request`——内部标签枚举的 unit 变体**不施加** `deny_unknown_fields`
/// （实测：`{"direction":"request","bytes":"AA"}` 会被静默读成 `Request` 并丢掉 `bytes`），
/// 而 struct 变体才走带字段访问器的路径。多一个 `{}` 换来的是「错字必炸」。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "direction", rename_all = "snake_case", deny_unknown_fields)]
pub enum Snapshot {
    /// H→K：请给一份存档（无载荷字段；写成空 struct 变体，未知字段才会被拒）。
    Request {},
    /// K→H：存档字节（与 `modload::Loaded::snapshot` 同源，base64）。
    Reply {
        /// 存档所在 tick。
        #[serde(deserialize_with = "strict::u64")]
        tick: u64,
        /// 该 tick 的 state hash。
        #[serde(deserialize_with = "strict::u64")]
        hash: u64,
        /// 存档字节（base64；协议层不解码，只搬运）。
        bytes: String,
    },
}

// ---------------------------------------------------------------------------
// 消息联合
// ---------------------------------------------------------------------------

/// `ygg/1` 的一条消息。
///
/// 判别键 `type`（`snake_case`），**每个变体的每个字段都是必填**（`replay_result` 的
/// `first_bad_tick` 除外，它在成功时缺省）。方向不是字段：同一个 `type` 在表里只有一个
/// 方向（`snapshot` 与 `error` 是例外，前者靠内层 `direction`，后者两个方向同形）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Message {
    /// H→K：配对请求。`pack_hash` 不符即拒（防 A 包存档进 B 包）。
    Hello {
        /// 协议版本，恒为 [`PROTOCOL`]（别的版本在解码处即拒）。
        #[serde(deserialize_with = "strict::protocol_token")]
        protocol: String,
        /// 线格式，恒为 [`CODEC`]（别的编码在解码处即拒）。
        #[serde(deserialize_with = "strict::codec_token")]
        codec: String,
        /// 包 id（`ygg:…` 或包名）。
        pack_id: String,
        /// 包内容摘要（小写十六进制 sha256，与 `manifest` 的内容寻址同口径；
        /// 格式不合在解码处即拒）。
        #[serde(deserialize_with = "strict::sha256_hex")]
        pack_hash: String,
        /// 可选：创世随机流种子。
        ///
        /// 缺省（字段不在）沿用内核默认种子；显式给出则进入 `ready` **之前**按该
        /// 种子重建一条干净世界谱系（创世重播）。只在 `Fresh` 态有意义——握手是
        /// 唯一的 `Fresh → Ready` 边，`Ready` 之后再发 `hello` 照旧被拒。
        ///
        /// 显式 `null` 不是缺省而是错（`strict::opt_u64`），与 schema 的 `integer`
        /// 口径一致：浮点 / 负数 / 超 `u64` 一律在解码期拒为 `E_PROTOCOL`。
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "strict::opt_u64"
        )]
        seed: Option<u64>,
    },
    /// K→H：世界自述；`open` 成功后同样回这一条（带新的 tick / hash）。
    Ready {
        /// 当前 tick。
        #[serde(deserialize_with = "strict::u64")]
        tick: u64,
        /// 当前 state hash。
        #[serde(deserialize_with = "strict::u64")]
        hash: u64,
        /// 系统是否已 `seal`（未 seal 的世界不能 `step`）。
        sealed: bool,
        /// 组件注册表（宿主据此生成类型 / UI）。
        components: Vec<Component>,
        /// 系统表（含阶段）。
        systems: Vec<SystemInfo>,
    },
    /// H→K：载入存档；成功回 [`Message::Ready`]。
    Open {
        /// 存档字节（base64）。
        bytes: String,
    },
    /// H→K：变更世界；不推进时间；失败整批回滚。
    Submit {
        /// 按序应用的操作（整批一个事务）。
        ops: Vec<Op>,
    },
    /// H→K：**表达意图**——按名字调用 Mod 导出的命令面；不推进时间。
    ///
    /// 与 [`Message::Submit`] 的分界是「谁决定规则」：`submit` 提交的是宿主自己
    /// 算出来的结构变更，规则在宿主手里；`command` 只说要做什么，**写什么由包里的
    /// 命令函数决定**。内核**不解释 `name`**——它只是命令面这张表的键，`args` 只是
    /// 原样转成 Lua 表的一段参数；信封与内核两侧都没有任何游戏词。
    ///
    /// 回 [`Message::Delta`]（与 `submit` 同一份 `Changed` 面）或 [`Message::Error`]
    /// （Mod 拒绝 / 名字不在命令面上 / 包未导出命令面）；每次调用一个事务，失败时
    /// `tick` / `hash` / 世界逐字节不变。
    Command {
        /// 命令名：Mod 入口 chunk 返回值里 `commands` 表的键（内核不认识它）。
        name: String,
        /// 命令参数：任意 JSON 对象，原样递进命令函数（键与值都是包的词汇，内核不读）。
        args: serde_json::Map<String, serde_json::Value>,
    },
    /// H→K：推进时间；回 [`Message::Events`] + [`Message::Delta`]。
    Step {
        /// 推进多少个 tick（≥ 1；0 是无效指令）。
        #[serde(deserialize_with = "strict::non_zero_u32")]
        ticks: u32,
        /// 每 tick 的步长（定点原值）。
        #[serde(deserialize_with = "strict::u32")]
        dt: u32,
    },
    /// K→H：本步结果。
    Events {
        /// 推进后的 tick。
        #[serde(deserialize_with = "strict::u64")]
        tick: u64,
        /// 推进后的 state hash。
        #[serde(deserialize_with = "strict::u64")]
        hash: u64,
        /// 本步产生的日志行（只映射 `world.log`）。
        log: Vec<LogLine>,
        /// 本步产生的结构化错误（非空不代表 step 失败——失败会整批回滚**并**回错误）。
        errors: Vec<ProtocolError>,
    },
    /// K→H：本步被写过的组件（增量刷新用；不是全量）。
    Delta {
        /// 这些变更落在哪个 tick。
        #[serde(deserialize_with = "strict::u64")]
        tick: u64,
        /// 逐实体的写入清单。
        changed: Vec<Changed>,
    },
    /// 双向：存档请求 / 回执，靠 [`Snapshot::Request`] / [`Snapshot::Reply`] 判别方向。
    Snapshot(Snapshot),
    /// H→K：回放校验请求；逐帧比对 hash，首个不符即停。
    Replay {
        /// 回放日志。
        log: Log,
    },
    /// K→H：回放结论。
    ///
    /// 线上 `type` 是 `replay_result` 而不是 `replay`：内部标签要求请求与回执的标签
    /// 不同，而回执没有 `direction` 之外的字段可借（与 `snapshot` 不同，回执有载荷、
    /// 请求没有）。
    ReplayResult {
        /// 全部帧逐一对上。
        ok: bool,
        /// 首个不符的 tick（`ok` 为真时缺省）。
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "strict::opt_u64"
        )]
        first_bad_tick: Option<u64>,
    },
    /// 双向：错误信封（两个方向同形）。
    Error(ProtocolError),
}

// ---------------------------------------------------------------------------
// 编解码入口
// ---------------------------------------------------------------------------

/// 解一条消息。
///
/// 任何失败都归为 [`ErrorCode::EProtocol`]：信封层能报的只有「你发的东西不是 `ygg/1`」，
/// 而具体是哪一类（浮点、未知字段、缺字段、越界）在 [`ProtocolError::msg`] 里逐字带出，
/// 并保留 `serde_json` 的 `line/column`。
pub fn decode(text: &str) -> Result<Message, ProtocolError> {
    serde_json::from_str(text).map_err(|e| ProtocolError::protocol(e.to_string()))
}

/// 编一条消息（规范序 = serde 的声明序，`type` 在首）。
pub fn encode(message: &Message) -> Result<String, ProtocolError> {
    serde_json::to_string(message).map_err(|e| ProtocolError::protocol(e.to_string()))
}

/// 一帧或多帧 → 线上回执：**一个 JSON 数组**，顺序即线上顺序（`step` 两帧）。
///
/// 绑定层（WASM / C ABI）只有一条回执通道：一次请求一段文本，于是帧序必须装
/// 进同一段文本——数组而不是「一行一帧」。这条约定是协议的一部分，所以它住在
/// 信封层：绑定层拿到的永远是一段可直接回给宿主的 JSON，不需要知道里面有
/// 几帧、也不参与拼接。
///
/// 空帧流答 `[]`（结构合法）；真正的「至少一帧」由 [`Session::handle_all`]
/// 保证。单帧编码失败也不让线上闭嘴：退化成那一帧的 `error{E_STATE}`。
///
/// [`Session::handle_all`]: crate::Session::handle_all
pub fn encode_frames(frames: &[Message]) -> String {
    let mut out = String::from("[");
    for (index, frame) in frames.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        match encode(frame) {
            Ok(text) => out.push_str(&text),
            Err(err) => {
                let fallback = Message::Error(ProtocolError::new(ErrorCode::EState, err.msg));
                // 兜底帧走同一台 encoder：它再失败只可能是内存耗尽。
                out.push_str(&encode(&fallback).unwrap_or_default());
            }
        }
    }
    out.push(']');
    out
}

// ---------------------------------------------------------------------------
// 边界浮点拒收
// ---------------------------------------------------------------------------

/// 整数字段的严格解码器（决策 3：浮点一律拒收，不做四舍五入）。
///
/// 为什么不是 `u64::deserialize`：`serde_json` 的 `deserialize_u64` 对 `1.0` 会**截断**
/// （`1.0` 走 `visit_f64`，serde 的 `u64` 访问器接受整值浮点），于是 `{"ticks":1.0}` 会
/// 被读成 `1`——一个「宿主以为发了小数、内核走了整数」的静默分歧。这里先落到
/// [`serde_json::Value`] 再判 `as_u64` / `as_i64`：`1.0` 与 `1.5` 都拿不到整数，
/// 两者都被拒，且错误里带上原文，宿主看得见自己发的是什么。
///
/// 代价只在非自描述格式上：本模块要求 deserializer 能 `deserialize_any`。v1 唯一的
/// 线格式是 JSON（决策 2），这条约束不构成限制；引入第二种格式时它必须自带同样的
/// 严格整数面，而不是绕过本模块。
mod strict {
    use serde::de::{Deserializer, Error as DeError};
    use serde::Deserialize;
    use serde_json::Value;

    use super::ErrorCode;
    use crate::{CODEC, PROTOCOL};

    fn rejected(found: &Value, want: &str) -> String {
        format!(
            "{}: 世界值必须是整数（期望 {want}），收到 {found}",
            ErrorCode::EProtocol.as_str()
        )
    }

    fn as_u64(value: Value) -> Result<u64, String> {
        match &value {
            Value::Number(n) => match n.as_u64() {
                Some(v) => Ok(v),
                None => Err(rejected(&value, "u64")),
            },
            _ => Err(rejected(&value, "u64")),
        }
    }

    fn as_u32(value: Value) -> Result<u32, String> {
        let raw = as_u64(value)?;
        u32::try_from(raw).map_err(|_| {
            format!(
                "{}: 数值超出 u32 范围：{raw}",
                ErrorCode::EProtocol.as_str()
            )
        })
    }

    fn as_i64(value: Value) -> Result<i64, String> {
        match &value {
            Value::Number(n) => match n.as_i64() {
                Some(v) => Ok(v),
                None => Err(rejected(&value, "i64")),
            },
            _ => Err(rejected(&value, "i64")),
        }
    }

    pub(super) fn u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
    where
        D: Deserializer<'de>,
    {
        as_u64(Value::deserialize(deserializer)?).map_err(D::Error::custom)
    }

    pub(super) fn u32<'de, D>(deserializer: D) -> Result<u32, D::Error>
    where
        D: Deserializer<'de>,
    {
        as_u32(Value::deserialize(deserializer)?).map_err(D::Error::custom)
    }

    pub(super) fn i64<'de, D>(deserializer: D) -> Result<i64, D::Error>
    where
        D: Deserializer<'de>,
    {
        as_i64(Value::deserialize(deserializer)?).map_err(D::Error::custom)
    }

    /// `hello.protocol`：必须是 [`PROTOCOL`]。版本面在**解码**处判，不在会话层判——
    /// 一个 v1 的 crate 本来就读不懂 v2 的信封，让它先读进来再比字符串，只会把「读不懂」
    /// 误报成「内容不对」。
    pub(super) fn protocol_token<'de, D>(deserializer: D) -> Result<String, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        if raw != PROTOCOL {
            return Err(D::Error::custom(format!(
                "E_PROTOCOL: 协议版本不符：本 crate 说 {PROTOCOL}，对端说 {raw}"
            )));
        }
        Ok(raw)
    }

    /// `hello.codec`：必须是 [`CODEC`]。v1 只有 JSON；这个字段的存在是为了让未来的 CBOR
    /// 走**同一套协商**（先声明、后换编码），而不是让 v1 悄悄收下一个它解释不了的声明。
    pub(super) fn codec_token<'de, D>(deserializer: D) -> Result<String, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        if raw != CODEC {
            return Err(D::Error::custom(format!(
                "E_PROTOCOL: 编码不符：v1 只认 {CODEC}，对端说 {raw}"
            )));
        }
        Ok(raw)
    }

    /// `hello.pack_hash`：64 位小写十六进制（内容寻址口径，与 `manifest.json` 的
    /// `sha256` 同一形态）。**格式**不合是 `E_PROTOCOL`（写错了字节），格式合但与本地包
    /// 不符才是 `E_PACK`（拿错了包）——两件事别混。
    pub(super) fn sha256_hex<'de, D>(deserializer: D) -> Result<String, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        let shaped = raw.len() == 64
            && raw
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !shaped {
            return Err(D::Error::custom(format!(
                "E_PROTOCOL: pack_hash 必须是 64 位小写十六进制，收到 {raw:?}"
            )));
        }
        Ok(raw)
    }

    /// 「至少一个」的 `u32`：`step.ticks` 与 `spawn.n`。0 是**无效指令**而不是空操作——
    /// 推进 0 步不构成一次推进，生成 0 个实体也没必要过线；schema 的 `minimum: 1` 与这里
    /// 同位，两侧对 0 都拒（成对用例 `schema_and_rust_reject_the_same_shape` 钉着）。
    pub(super) fn non_zero_u32<'de, D>(deserializer: D) -> Result<u32, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = as_u32(Value::deserialize(deserializer)?).map_err(D::Error::custom)?;
        if value == 0 {
            return Err(D::Error::custom(
                "E_PROTOCOL: 这个字段必须 ≥ 1（0 是无效指令，不是空操作）",
            ));
        }
        Ok(value)
    }

    /// `Option<u64>`：缺省（字段不在）由 `#[serde(default)]` 处理，**显式 `null` 不是缺省**
    /// 而是错——schema 那一侧也只有 `integer`，两侧必须拒同一批输入。出现即非 `Some` 的
    /// 路径只有「字段不在」一条。
    pub(super) fn opt_u64<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
    where
        D: Deserializer<'de>,
    {
        as_u64(Value::deserialize(deserializer)?)
            .map(Some)
            .map_err(D::Error::custom)
    }
}
