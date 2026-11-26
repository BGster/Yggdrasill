//! `ygg/1` 会话状态机：**一条连接 ↔ 一个世界实例**（计划稿 §0 决策 5）。
//!
//! # 这一层做什么
//!
//! 信封（[`crate::proto`]）只定义字段；本模块把「一条消息」翻译成「一次内核调用」
//! 并给出回执。任何**决策**仍然在 `ygg-sandbox` / `ygg-sim` 里：能不能写这个世界
//! （[`WorldOp`] 的整批校验）、能不能推进时间（内核步进）、`pack_hash` 对不对
//! （[`Loaded::manifest_hash`]）。本模块只做四件不含判断的事：
//!
//! 1. **状态门**：`Fresh → Ready` 只在握手成功后发生；未握手时除 `hello` 外一律
//!    `error{E_PROTOCOL}`。
//! 2. **值面搬运**：`proto::Value` ↔ 内核列 / 单元格（十进制字节 ↔ `ValueRef`），
//!    base64（快照字节在协议里只是字符串）。
//! 3. **帧切分**：一次 `step` 的答案在线上是**两帧**（`events` + `delta`）——
//!    [`Session::handle_all`] 给全部；[`Session::handle`] 给第一帧（进程内调用方
//!    两条都要看时用前者，别猜）。
//! 4. **错误分类**：内核错误码 → 协议错误码（`E_*`），映射表见 [`step_code`]。
//!
//! # 四条不变式
//!
//! - **永不 panic、总有答案**：任何 `Message` 进来都有 `Message` 出去，包括状态不对、
//!   值类型不符、base64 坏掉——全部变成 `error{code,msg}`，不 `unwrap` 宿主输入。
//! - **不改写内核语义**：批量写只走 [`WorldOp`]（与 Lua `world.*` 同一份实现），
//!   本模块不复制校验、不「顺手修一下」payload。
//! - **增量来自撤销日志**：`delta` 的单元格集合是内核步进时撤销日志的去重投影
//!   （[`StepReport::changed`](ygg_sandbox::sandbox::StepReport)），值在**步进之后**
//!   从世界读——不是另记一份写点，也不是回放前值。
//! - **复算从日志头开始**：`replay` 用 `header.seed` 另起一条内核谱系，逐帧比对
//!   `hash`；第一处不符即答 `replay_result{ok:false, first_bad_tick}` 并停止。
//!
//! # 绑定面（M3.3）：文本进、文本出
//!
//! 非 Rust 宿主不接触 [`Message`]，只走两条入口：[`Session::from_image`]（包镜像 →
//! 会话）与 [`Session::handle_json`]（一段 JSON → 一段 JSON 数组）。绑定层
//! （`ygg-wasm` / `ygg-capi`）因此一行判断都没有：镜像解析在
//! [`crate::image`]，帧流打包在 [`crate::proto::encode_frames`]，而「这条消息
//! 该不该接受」本来就在本模块的状态门里。
//!
//! 绑定层写 `if` 的那一刻就是把协议语义劈成了三份——要加判断，加在这里。
//!
//! # v1 已知缺口（写在这里免得被当成 bug）
//!
//! - `replay` 消息不带起始快照：`header.start_tick != 0` 的日志无法复算（明确拒绝）。
//! - `submit` 没有回执帧：新实体 id 只能靠随后 `step` 的 `delta` 侧看，协议 v1 没有
//!   「本次写了什么」的答案（本模块用 `delta` 代替：`Changed` 面就是本次写的单元格）。
//! - `delta` 只有**单元格终值**：结构变化（despawn / remove 掉的实体与组件）不出现，
//!   因为撤销日志只记录被覆盖的列值。
//! - `string` 组件在 v1 用 `Value::Str` 承载：非 UTF-8 字节写不出线格式，这类单元格
//!   会被从 `delta` 里略去（值类型 v1 没有 bytes 变体）。

use ygg_sandbox::modload::{LoadError, Loaded};
use ygg_sandbox::sandbox::{StepError, StepReport};
use ygg_sandbox::world_ops::WorldOp;
use ygg_sim::archetype::{Column, ValueRef};
use ygg_sim::component::{AttrType, ComponentId, EntityId, Registry};
use ygg_sim::error::SimError;
use ygg_sim::system::Stage as SimStage;

use crate::base64::{base64_decode, base64_encode};
use crate::proto::{
    decode, encode_frames, Changed, Component, ComponentKind, ComponentValues, ErrorCode, Log,
    LogLevel, LogLine, Message, Op, ProtocolError, Snapshot, Stage, SystemInfo, Value, CODEC,
    PROTOCOL,
};

/// 会话状态：`Fresh` 未握手 / `Ready` 可调用 / `Closed` 已关闭。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// 已装载、未握手：只接受 `hello`。
    Fresh,
    /// 握手完成：接受世界面全部消息。
    Ready,
    /// 已关闭（传输层判定连接结束）：任何消息都答 `error{E_STATE}`。
    Closed,
}

/// 一条连接上的会话（持有一个已装载内核）。
///
/// 一个会话 = 一个世界实例：快照之外的跨连接状态不存在（决策 5）。关闭由传输层
/// 显式调用 [`close`](Self::close)——协议 v1 没有 `bye`，本模块不自行猜测连接寿命。
pub struct Session {
    state: State,
    loaded: Loaded,
    /// `ready.sealed` 的缓存：装载后由脚本的 `world.seal()` 决定，`open` 不动它。
    sealed: bool,
}

impl Session {
    /// 用一个已装载内核开一条**未握手**会话。
    pub fn new(loaded: Loaded) -> Self {
        let sealed = loaded.sealed();
        Self {
            state: State::Fresh,
            loaded,
            sealed,
        }
    }

    /// 当前状态（绑定层用来做连接生命周期断言）。
    pub fn state(&self) -> State {
        self.state
    }

    /// 已装载内核的只读视图（诊断 / 测试用；写面一律经消息）。
    pub fn loaded(&self) -> &Loaded {
        &self.loaded
    }

    /// 关闭会话：之后任何消息答 `error{E_STATE}`。
    pub fn close(&mut self) {
        self.state = State::Closed;
    }

    /// 处理一条消息，返回**第一帧**答案（`step` 的第二帧是 `delta`，见
    /// [`handle_all`](Self::handle_all)）。
    ///
    /// 永不 panic：内部帧数为零时兜底一条 `error{E_STATE}`（构造上不可达，
    /// 但绑定层不该因为本模块的将来改动而崩）。
    pub fn handle(&mut self, msg: Message) -> Message {
        self.handle_all(msg)
            .into_iter()
            .next()
            .unwrap_or_else(|| error(ErrorCode::EState, "会话未产生答案帧（内部不变量破坏）"))
    }

    /// 处理一条消息，返回线上真实的**全部**帧（顺序即线上顺序）。
    ///
    /// 只有 `step` 出两帧（`events` 然后 `delta`），其余一帧。
    pub fn handle_all(&mut self, msg: Message) -> Vec<Message> {
        match self.state {
            State::Closed => vec![error(ErrorCode::EState, "会话已关闭")],
            State::Fresh => match msg {
                Message::Hello {
                    protocol,
                    codec,
                    pack_id,
                    pack_hash,
                    seed,
                } => self.hello(&protocol, &codec, &pack_id, &pack_hash, seed),
                other => vec![error(
                    ErrorCode::EProtocol,
                    format!("未握手：先发送 hello 再发 {}", name_of(&other)),
                )],
            },
            State::Ready => self.call(msg),
        }
    }

    // ---------------------------------------------------------------- 绑定面

    /// 用一份**包镜像**装载内核并开一条未握手会话（三条绑定共用的唯一入口）。
    ///
    /// 镜像形状见 [`crate::image`]；装载失败答 `Err(文案)`——镜像不是线上消息，
    /// 所以这里没有协议帧可答（握手之后的一切失败才走 `error` 帧）。
    pub fn from_image(image: &[u8]) -> Result<Self, String> {
        crate::image::open(image)
    }

    /// 文本进、文本出的一次往返：解码 → [`handle_all`](Self::handle_all) → 帧流 JSON。
    ///
    /// 回执是**一个 JSON 数组**（线上帧序：`step` 两帧，其余一帧）。坏 JSON 不
    /// 抛错也不答 `null`，答一帧 `error`——绑定层因此不必（也不许）自己判断消息
    /// 语义，它只负责把这段文本搬到对端。
    pub fn handle_json(&mut self, text: &str) -> String {
        let mut frames = match decode(text) {
            Ok(msg) => self.handle_all(msg),
            Err(err) => vec![Message::Error(err)],
        };
        if frames.is_empty() {
            // 构造上不可达（`handle_all` 永远至少一帧）；兜底与 `handle` 同一句话。
            frames.push(error(
                ErrorCode::EState,
                "会话未产生答案帧（内部不变量破坏）",
            ));
        }
        encode_frames(&frames)
    }

    /// 当前世界帧字节（`snapshot` 消息的字节面）。
    ///
    /// 绑定层的 `snapshot()` 近路走这里：失败映射成 `error{load_code}`（与
    /// `snapshot` 消息同一分类）。**协议入口仍是 `snapshot` 消息**——这条只是给
    /// 「我要一份存档字节」的宿主省一次 base64 往返。名字带 `_bytes` 是为了与
    /// 消息处理面 `snapshot(request)` 分开：一个是字节近路，一个是线上消息。
    pub fn snapshot_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        self.loaded
            .snapshot()
            .map_err(|err| ProtocolError::new(load_code(&err), err.to_string()))
    }

    // ---------------------------------------------------------------- 门与回执

    /// 握手：对账协议名 / 编码 / 声明指纹；不符**不改变状态**（仍可重试握手）。
    /// `seed` 是可选的**创世种子**：`None` 沿用内核默认种子；`Some(seed)` 在对账全过后、
    /// 进入 `Ready` 前按该种子重建一条干净世界谱系，重建失败不改变状态（仍可重试握手）。
    fn hello(
        &mut self,
        protocol: &str,
        codec: &str,
        pack_id: &str,
        pack_hash: &str,
        seed: Option<u64>,
    ) -> Vec<Message> {
        // `proto` 的严格词面已在解码期拒绝这两项；此处仍判定一次——会话可能被
        // 进程内调用方手搓消息喂进来，状态门不能依赖「上游一定会校验」。
        if protocol != PROTOCOL {
            return vec![error(
                ErrorCode::EProtocol,
                format!("协议名不符：要 {PROTOCOL}，收到 {protocol}"),
            )];
        }
        if codec != CODEC {
            return vec![error(
                ErrorCode::EProtocol,
                format!("编码不符：要 {CODEC}，收到 {codec}"),
            )];
        }
        if pack_hash != self.loaded.manifest_hash() {
            // 状态留在 `Fresh`：宿主可以改正后重试（换包重载 or 修指纹）。
            return vec![error(
                ErrorCode::EPack,
                format!(
                    "pack_hash 不符：装载的声明是 {}，收到 {pack_hash}（pack_id={pack_id}）",
                    self.loaded.manifest_hash()
                ),
            )];
        }
        // 创世重播：对账全过之后才换种子——换谱系是一次装载，失败要能重试握手。
        if let Some(seed) = seed {
            match self.loaded.fork_with_seed(seed) {
                Ok(loaded) => self.loaded = loaded,
                Err(err) => return vec![error(load_code(&err), err.to_string())],
            }
        }
        self.state = State::Ready;
        vec![self.ready()]
    }

    /// `ready` 回执：时钟 + 声明面 + 组件表 + 系统表。
    fn ready(&mut self) -> Message {
        self.sealed = self.loaded.sealed();
        Message::Ready {
            tick: self.tick(),
            hash: self.hash(),
            sealed: self.sealed,
            components: self.components(),
            systems: self.systems(),
        }
    }

    /// `Ready` 态的消息分派（世界面）。
    fn call(&mut self, msg: Message) -> Vec<Message> {
        match msg {
            Message::Submit { ops } => self.submit(&ops),
            Message::Command { name, args } => self.command(&name, &args),
            Message::Step { ticks, dt } => self.step(ticks, dt),
            Message::Open { bytes } => self.open(&bytes),
            Message::Snapshot(request) => self.snapshot(request),
            Message::Replay { log } => vec![self.replay(&log)],
            // 重复握手：状态已经 `Ready`，再来一次是调用方的错（不静默重放）。
            Message::Hello { .. } => vec![error(ErrorCode::EProtocol, "已握手：不重复 hello")],
            // 宿主侧的 `error` 视为「传输层报告了一次失败」：答 `ready` 让双方回到
            // 可继续的状态（不推断是谁的错）。
            Message::Error(_) => vec![self.ready()],
            // 内核 → 宿主的回执帧被送回来：方向错，协议错。
            other => vec![error(
                ErrorCode::EProtocol,
                format!("{} 是内核回执帧，宿主不该发送", name_of(&other)),
            )],
        }
    }

    // ---------------------------------------------------------------- 世界面

    /// `submit`：一批 [`WorldOp`] 交给内核的「先全验、后落笔」实现。
    ///
    /// 协议 v1 没有 submit 回执帧，本模块答**本次写入的 `delta`**（同一份
    /// `Changed` 面，值取写后终值）：调用方立刻知道这次批量的效果，不必等到
    /// 下一次 `step`。整批失败时零写入，答 `error`，世界哈希不变。
    fn submit(&mut self, ops: &[Op]) -> Vec<Message> {
        let converted = {
            let sim = self.loaded.sim();
            let Some(sim) = sim.as_ref() else {
                return vec![error(ErrorCode::EState, "内核未挂载")];
            };
            convert_ops(sim.world().registry(), ops)
        };
        let world_ops = match converted {
            Ok(world_ops) => world_ops,
            Err(err) => return vec![Message::Error(err)],
        };
        match self.loaded.apply(&world_ops) {
            Ok(_writes) => {
                let cells = self.loaded.changed();
                vec![Message::Delta {
                    tick: self.tick(),
                    changed: self.changed_of(&cells),
                }]
            }
            Err(err) => vec![error(step_code(&err), err.to_string())],
        }
    }

    /// `command`：宿主**表达意图**，写什么由 Mod 的命令面决定。
    ///
    /// 与 `submit` 的分界是「谁决定规则」：`submit` 直接交结构变更，规则在宿主
    /// 手里；`command` 只说要做什么，规则在包里。内核因此不认识任何命令名——它
    /// 只把 `args` 换成 Lua 表递进去，再按 `true` / `(false, 原因)` 解读结果。
    ///
    /// 回执：成功答 `delta`（与 `submit` 同一份 `Changed` 面，值取写后终值），
    /// 拒绝 / 名字不存在 / 命令面缺失答 `error`，世界零写入（`tick` 与 `hash` 不动）。
    fn command(
        &mut self,
        name: &str,
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> Vec<Message> {
        // `args` 原样过界：它是包的词汇，内核不读（也不该读）其中任何键。
        let args_json = match serde_json::to_string(args) {
            Ok(text) => text,
            Err(err) => {
                return vec![error(
                    ErrorCode::EProtocol,
                    format!("command.args 不能序列化：{err}"),
                )]
            }
        };
        match self.loaded.command(name, &args_json) {
            Ok(cells) => vec![Message::Delta {
                tick: self.tick(),
                changed: self.changed_of(&cells),
            }],
            Err(err) => vec![error(step_code(&err), err.to_string())],
        }
    }

    /// `step`：推进时间 → `events`（日志）→ `delta`（增量）。
    ///
    /// 未 `seal` 的世界拒绝推进（协议 `ready.sealed` 的承诺；内核自己不在 `step`
    /// 里拦，见 `Loaded::sealed`）。
    fn step(&mut self, ticks: u32, dt: u32) -> Vec<Message> {
        if !self.sealed {
            return vec![error(
                ErrorCode::ERule,
                "世界未 seal（脚本未调用 world.seal()），不能推进时间",
            )];
        }
        let report: StepReport = match self.loaded.step(ticks, dt) {
            Ok(report) => report,
            Err(err) => return vec![error(load_code(&err), err.to_string())],
        };
        let log = self
            .loaded
            .take_world_log()
            .into_iter()
            .map(|text| LogLine {
                level: LogLevel::Info,
                text,
            })
            .collect();
        vec![
            Message::Events {
                tick: report.tick,
                hash: report.hash,
                log,
                errors: Vec::new(),
            },
            Message::Delta {
                tick: report.tick,
                changed: self.changed_of(&report.changed),
            },
        ]
    }

    /// `open`：按快照字节整体覆写世界面，答 `ready`（协议消息表的口径）。
    fn open(&mut self, bytes: &str) -> Vec<Message> {
        let raw = match base64_decode(bytes) {
            Ok(raw) => raw,
            Err(err) => return vec![Message::Error(err)],
        };
        match self.loaded.sandbox.restore(&raw) {
            Ok(()) => vec![self.ready()],
            Err(err) => vec![error(step_code(&err), err.to_string())],
        }
    }

    /// `snapshot`：世界帧字节 → base64（协议层不解码，只搬运）。
    fn snapshot(&mut self, request: Snapshot) -> Vec<Message> {
        match request {
            Snapshot::Request {} => match self.loaded.snapshot() {
                Ok(bytes) => vec![Message::Snapshot(Snapshot::Reply {
                    tick: self.tick(),
                    hash: self.hash(),
                    bytes: base64_encode(&bytes),
                })],
                Err(err) => vec![error(load_code(&err), err.to_string())],
            },
            // `reply` 是内核 → 宿主的方向：送回来是调用方的错。
            Snapshot::Reply { .. } => vec![error(
                ErrorCode::EProtocol,
                "snapshot 的 reply 帧是内核回执，宿主不该发送",
            )],
        }
    }

    /// `replay`：用日志头的种子另起一条谱系，逐帧重演并比对哈希。
    ///
    /// 判据是**帧哈希**（内核每步的状态摘要），不是「步进没报错」：篡改过的
    /// `ops` 只要改变了世界，哈希就对不上；不改变世界的篡改（例如把 `dt` 改小到
    /// 系统无感知）在 v1 不可检测，这是日志面本来的强度。
    ///
    /// **帧 tick 是「该帧开跑前」的内核时钟**（与内核录制器 `push_tick` 同一口径：
    /// 创世后的第一帧是 `tick = 0`），所以复算要求「当前 tick == `frame.tick`」，
    /// 跳号/倒序都算第一处不符；`replay_result.first_bad_tick` 报的就是这个字段的
    /// 原值，不是「步进之后的 tick」。
    fn replay(&self, log: &Log) -> Message {
        let start_tick = log.header.start_tick;
        if start_tick != 0 {
            // v1 的 `replay` 不带起始快照：非零起点无从复算——明确拒绝，不猜。
            return Message::ReplayResult {
                ok: false,
                first_bad_tick: Some(start_tick),
            };
        }
        let mut forked = match self.loaded.fork_with_seed(log.header.seed) {
            Ok(forked) => forked,
            Err(err) => return error(load_code(&err), err.to_string()),
        };
        // 组件类型表在两条谱系里同一份（同一装载参数），转换借用原内核的 registry。
        let sim = self.loaded.sim();
        let Some(source) = sim.as_ref() else {
            return error(ErrorCode::EState, "内核未挂载");
        };
        let registry = source.world().registry();
        for frame in &log.frames {
            if forked.tick().unwrap_or(u64::MAX) != frame.tick {
                return bad_frame(frame.tick);
            }
            let ops = match convert_ops(registry, &frame.ops) {
                Ok(ops) => ops,
                // 帧里的 payload 连转换都过不了：这一帧复算不出来，就是这一帧坏。
                Err(_) => return bad_frame(frame.tick),
            };
            if forked.apply(&ops).is_err() {
                return bad_frame(frame.tick);
            }
            match forked.step(1, frame.dt) {
                Ok(report) if report.hash == frame.hash => {}
                _ => return bad_frame(frame.tick),
            }
        }
        Message::ReplayResult {
            ok: true,
            first_bad_tick: None,
        }
    }

    // ---------------------------------------------------------------- 读出面

    /// 当前 tick（内核未挂载时 0——构造上不可达）。
    fn tick(&self) -> u64 {
        self.loaded.tick().unwrap_or_default()
    }

    /// 当前状态哈希（同上）。
    fn hash(&self) -> u64 {
        self.loaded.hash().unwrap_or_default()
    }

    /// 组件表（注册序即 id 序）。
    fn components(&self) -> Vec<Component> {
        let sim = self.loaded.sim();
        let Some(sim) = sim.as_ref() else {
            return Vec::new();
        };
        let registry = sim.world().registry();
        (0..registry.len())
            .filter_map(|index| {
                let id = u32::try_from(index).ok()?;
                let name = registry.name_of(id).ok()?.to_string();
                let kind = component_kind(registry.type_of(id).ok()?);
                Some(Component { id, name, kind })
            })
            .collect()
    }

    /// 系统表（stage 先后的执行序）。
    fn systems(&self) -> Vec<SystemInfo> {
        let sim = self.loaded.sim();
        let Some(sim) = sim.as_ref() else {
            return Vec::new();
        };
        sim.systems()
            .into_iter()
            .map(|(name, stage)| SystemInfo {
                name: name.to_string(),
                stage: stage_of(stage),
            })
            .collect()
    }

    /// 单元格集合 → `delta.changed`（按实体归并；值取**现在**的世界值）。
    fn changed_of(&self, cells: &[(EntityId, ComponentId)]) -> Vec<Changed> {
        let sim = self.loaded.sim();
        let Some(sim) = sim.as_ref() else {
            return Vec::new();
        };
        let world = sim.world();
        let mut out: Vec<Changed> = Vec::new();
        for &(eid, cid) in cells {
            // 结构操作可能已经让这个单元格不存在（despawn / remove 之后的同一批），
            // 或值在线格式里没有表示（非 UTF-8 字节）：略过，不编造。
            let Ok(reference) = world.value_of(eid, cid) else {
                continue;
            };
            let Ok(value) = cell_value(reference) else {
                continue;
            };
            let cell = ComponentValues {
                cid,
                values: vec![value],
            };
            match out.last_mut() {
                Some(last) if last.eid == eid => last.comps.push(cell),
                _ => out.push(Changed {
                    eid,
                    comps: vec![cell],
                }),
            }
        }
        out
    }
}

/// 复算失败的一帧。
fn bad_frame(tick: u64) -> Message {
    Message::ReplayResult {
        ok: false,
        first_bad_tick: Some(tick),
    }
}

/// `error` 帧的构造器（本模块唯一的错误出口）。
fn error(code: ErrorCode, msg: impl Into<String>) -> Message {
    Message::Error(ProtocolError::new(code, msg))
}

/// 消息名（诊断文案用；不含 payload）。
fn name_of(msg: &Message) -> &'static str {
    match msg {
        Message::Hello { .. } => "hello",
        Message::Ready { .. } => "ready",
        Message::Open { .. } => "open",
        Message::Submit { .. } => "submit",
        Message::Command { .. } => "command",
        Message::Step { .. } => "step",
        Message::Events { .. } => "events",
        Message::Delta { .. } => "delta",
        Message::Snapshot(_) => "snapshot",
        Message::Replay { .. } => "replay",
        Message::ReplayResult { .. } => "replay_result",
        Message::Error(_) => "error",
    }
}

// -------------------------------------------------------------------- 错误映射

/// 装载 / 恢复 / 步进错误 → 协议错误码。
///
/// 分类口径：**写入面拒绝 = `E_RULE`**（世界规则），**帧/状态 = `E_STATE`**，
/// **预算 = `E_BUDGET`**，**装载参数 = `E_PACK`**。`E_PROTOCOL` 只给「消息本身
/// 不合协议」（值类型不符、base64 坏、方向错）。
fn load_code(err: &LoadError) -> ErrorCode {
    match err {
        LoadError::Step(err) => step_code(err),
        LoadError::Bundle(_) | LoadError::Manifest(_) => ErrorCode::EPack,
        LoadError::Sandbox(_) | LoadError::ModChunk(_) | LoadError::Resources(_) => {
            ErrorCode::EState
        }
    }
}

fn step_code(err: &StepError) -> ErrorCode {
    match err {
        // 消息本身不合契约（含命令返回值不是 `true` / `(false, 原因)`）。
        StepError::ZeroTicks | StepError::CommandValue { .. } => ErrorCode::EProtocol,
        StepError::Budget { .. } => ErrorCode::EBudget,
        StepError::Sim(err) => sim_code(err),
        // 调用与包对不上：包没导出命令面，或名字不在面上——宿主侧要改的是名字。
        StepError::NoCommandSurface | StepError::UnknownCommand { .. } => ErrorCode::EState,
        // Mod 的规则拒绝或运行期报错：世界规则说了算。
        StepError::Refused { .. } | StepError::Raised { .. } => ErrorCode::ERule,
    }
}

fn sim_code(err: &SimError) -> ErrorCode {
    match err {
        SimError::Schema(_) | SimError::Entity(_) | SimError::Fixed(_) | SimError::Sealed(_) => {
            ErrorCode::ERule
        }
        SimError::Frame(_) | SimError::State(_) | SimError::Unmounted(_) => ErrorCode::EState,
        // 存档格式不被支持单独成码：宿主的话术是「升级存档」而不是「文件损坏」。
        SimError::Format(_) => ErrorCode::EStoreFormat,
    }
}

// -------------------------------------------------------------------- 值面搬运

/// `AttrType` → 线格式的组件类型字面量（一一对应，**单点声明**）。
fn component_kind(ty: AttrType) -> ComponentKind {
    match ty {
        AttrType::Number { .. } => ComponentKind::Number,
        AttrType::Str => ComponentKind::String,
        AttrType::Bool => ComponentKind::Boolean,
        AttrType::Ref => ComponentKind::Ref,
        AttrType::Tag => ComponentKind::Tag,
        AttrType::F64Bits => ComponentKind::F64Bits,
    }
}

/// 内核 stage → 线格式 stage。
fn stage_of(stage: SimStage) -> Stage {
    match stage {
        SimStage::Pre => Stage::Pre,
        SimStage::Rules => Stage::Rules,
        SimStage::Post => Stage::Post,
    }
}

/// 单元格值 → 线格式值（`f64bits` 以无符号位串过面；v1 没有浮点）。
fn cell_value(value: ValueRef<'_>) -> Result<Value, ProtocolError> {
    Ok(match value {
        ValueRef::I64(v) => Value::Int(v),
        ValueRef::U64(v) | ValueRef::Bits(v) => Value::Uint(v),
        ValueRef::U8(v) => Value::Bool(v != 0),
        ValueRef::Bytes(bytes) => Value::Str(
            std::str::from_utf8(bytes)
                .map_err(|_| protocol("string 组件的字节不是 UTF-8，v1 线格式无从表达"))?
                .to_string(),
        ),
    })
}

/// 协议错误（值类型 / 载荷形状不符协议）。
fn protocol(msg: impl Into<String>) -> ProtocolError {
    ProtocolError::new(ErrorCode::EProtocol, msg)
}

/// 值变体的名字（诊断文案用）。
fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Int(_) => "int",
        Value::Uint(_) => "uint",
        Value::Str(_) => "str",
        Value::Bool(_) => "bool",
    }
}

/// `proto::Op` 序列 → 内核 [`WorldOp`] 序列（纯转译，不做可行性判断）。
///
/// 组件类型取自 registry：`tag`（无列）走 `tags`，其余按列型逐值转换。
fn convert_ops(registry: &Registry, ops: &[Op]) -> Result<Vec<WorldOp>, ProtocolError> {
    let mut out = Vec::with_capacity(ops.len());
    for op in ops {
        match op {
            Op::Spawn { n, comps } => {
                let (cols, tags) = payload(registry, comps, *n)?;
                out.push(WorldOp::Spawn {
                    rows: *n,
                    cols,
                    tags,
                });
            }
            Op::Despawn { ids } => out.push(WorldOp::Despawn { ids: ids.clone() }),
            Op::Insert { id, comps } => {
                let (cols, tags) = payload(registry, comps, 1)?;
                out.push(WorldOp::Insert {
                    ids: vec![*id],
                    cols,
                    tags,
                });
            }
            Op::Remove { id, cids } => out.push(WorldOp::Remove {
                ids: vec![*id],
                cids: cids.clone(),
            }),
            Op::Set { id, comps } => {
                let (cols, tags) = payload(registry, comps, 1)?;
                if !tags.is_empty() {
                    return Err(protocol("set 不能写 tag 组件（tag 没有列）"));
                }
                out.extend(cols.into_iter().map(|(col, values)| WorldOp::Set {
                    ids: vec![*id],
                    col,
                    values,
                }));
            }
        }
    }
    Ok(out)
}

/// 一份载荷的转译结果：**有列的组件** + **零宽（tag）成员声明**。
type Payload = (Vec<(ComponentId, Column)>, Vec<ComponentId>);

/// 一份载荷（`ComponentValues` 序列）→ 列 + 零宽成员声明。
fn payload(
    registry: &Registry,
    comps: &[ComponentValues],
    rows: u32,
) -> Result<Payload, ProtocolError> {
    let mut cols = Vec::with_capacity(comps.len());
    let mut tags = Vec::new();
    for cell in comps {
        let ty = registry
            .type_of(cell.cid)
            .map_err(|err| protocol(format!("组件 id {}：{err}", cell.cid)))?;
        if ty.has_column() {
            cols.push((cell.cid, column_of(ty, cell.cid, &cell.values, rows)?));
        } else {
            if !cell.values.is_empty() {
                return Err(protocol(format!(
                    "组件 id {} 是 tag（没有列），不能带载荷",
                    cell.cid
                )));
            }
            tags.push(cell.cid);
        }
    }
    Ok((cols, tags))
}

/// 一条 `ComponentValues` → 内核列（行数必须与 `rows` 一致）。
fn column_of(
    ty: AttrType,
    cid: ComponentId,
    values: &[Value],
    rows: u32,
) -> Result<Column, ProtocolError> {
    if values.len() != rows as usize {
        return Err(protocol(format!(
            "组件 id {cid} 的载荷有 {} 行，消息声明 {rows} 行",
            values.len()
        )));
    }
    let wrong = |expected: &str| {
        let got = values.first().map_or("空载荷", value_kind);
        protocol(format!("组件 id {cid} 要 {expected}，载荷里是 {got}"))
    };
    Ok(match ty {
        AttrType::Number { .. } => Column::I64(
            values
                .iter()
                .map(|v| match v {
                    Value::Int(v) => Ok(*v),
                    Value::Uint(v) => i64::try_from(*v)
                        .map_err(|_| protocol(format!("组件 id {cid} 的值 {v} 超出 i64"))),
                    _ => Err(wrong("int")),
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
        AttrType::Ref | AttrType::F64Bits => Column::U64(
            values
                .iter()
                .map(|v| match v {
                    Value::Uint(v) => Ok(*v),
                    Value::Int(v) => u64::try_from(*v).map_err(|_| {
                        protocol(format!("组件 id {cid} 的值 {v} 为负，ref 要求 u64"))
                    }),
                    _ => Err(wrong("uint")),
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
        AttrType::Bool => Column::U8(
            values
                .iter()
                .map(|v| match v {
                    Value::Bool(v) => Ok(u8::from(*v)),
                    _ => Err(wrong("bool")),
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
        AttrType::Str => {
            let mut off = Vec::with_capacity(values.len() + 1);
            let mut buf = Vec::new();
            off.push(0u32);
            for v in values {
                let Value::Str(text) = v else {
                    return Err(wrong("str"));
                };
                buf.extend_from_slice(text.as_bytes());
                let end = u32::try_from(buf.len())
                    .map_err(|_| protocol(format!("组件 id {cid} 的字符串列超过 4GiB")))?;
                off.push(end);
            }
            Column::Bytes { off, buf }
        }
        // `payload` 已经把 tag 分流出去了：走到这里说明调用方漏判，按协议错答。
        AttrType::Tag => return Err(protocol(format!("组件 id {cid} 是 tag，没有列"))),
    })
}
