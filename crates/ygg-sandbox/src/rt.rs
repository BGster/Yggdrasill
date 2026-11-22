//! rt 引擎（S31 R1）：常驻帧循环的确定性核心——定步调度、输入设备、
//! 帧链哈希、录制与回放。契约 `docs/design/host/rt-face.md`。
//!
//! - **定步序列**（D3）：`dt_ms(n) = ⌊n·1000/hz⌋ − ⌊(n−1)·1000/hz⌋`——纯
//!   函数 of (n, hz)，零浮点、零墙钟入值；60Hz 时 16/17 交替，积分恰为
//!   实时。墙钟只决定**一拍之内跑几帧**（落后补帧 ≤ `max_catchup`，超出
//!   部分弃掉并计数——螺旋死亡防护），从不决定 dt 值；计划确定性条款
//!   「dt 入载荷=入状态；墙钟时间戳只入回执元数据」由此成立。
//! - **帧链哈希**（D4）：`H(n) = SHA256(H(n−1) | n | dt | 规范串 | 全状态摘要)`，
//!   genesis = `SHA256("ygg.rt.v1|event=…|hz=…")`——调度参数入链，配置被改
//!   同样红。全状态摘要覆盖实体 + KV + pending_choice（save 信封字节；
//!   [`crate::hash`] 的实体面投影不够——KV 才是 mod 游戏状态的大头）。
//!   回放 = 同序重派 + 逐帧比对，**first_mismatch 证伪语义**（仿
//!   ygg-sim `ReplayLog`）：篡改任一 dt / 键序 / 状态路径必被抓。
//! - **声明收窄**：只证「同 seed + 同录制 → 逐帧同哈希链」，不证「两次
//!   直播同哈希」（直播输入到达本质非确定）。内核 seed 是编译期常量
//!   （`inject::KERNEL_RNG_SEED`），无需进录制文件。（原「燃料耗尽是
//!   确定性事件」条款随 S34 燃料退役移除——链上少一个变量。）

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{json, Value as Json};

use crate::driver::KernelFacade;
use crate::hash::hex_sha256;
use crate::host_api::{RtInput, RtMode, RT_DEFAULT_HZ, RT_DEFAULT_MAX_CATCHUP, RT_KEY_MAX};

/// rt.tick 载荷构造（`n=<n>;dt=<dt>;` + 输入规范串）——录制/回放逐字节
/// 一致的唯一载荷拼串点（**单点收口**：两处拼串必漂移）。
pub fn frame_payload(n: u64, dt_ms: i64, input_canonical: &str) -> String {
    format!("n={n};dt={dt_ms};{input_canonical}")
}

/// 定步 dt 序列（D3）：第 n 帧的整数毫秒，积分恰为实时，无浮点。
pub fn dt_for_frame(n: u64, hz: u32) -> i64 {
    let hz = i64::from(hz.max(1));
    let cur = (n as i64 * 1000) / hz;
    let prev = ((n as i64 - 1) * 1000) / hz;
    cur - prev
}

/// 帧循环配置（`ygg.rt.config` 装载期声明 + CLI 覆盖的合流结果；优先级
/// CLI > Mod 装载期声明 > 默认，见 examples/rt 的合流点）。
#[derive(Clone, Debug)]
pub struct RtConfig {
    pub hz: u32,
    pub max_catchup: u32,
    /// 每帧派发的事件名（缺省 `rt.tick`；`--event` 可指到既有 run 模式
    /// 事件——run 模式兼容的桥）。
    pub event: String,
}

impl Default for RtConfig {
    fn default() -> Self {
        Self {
            hz: RT_DEFAULT_HZ,
            max_catchup: RT_DEFAULT_MAX_CATCHUP,
            event: "rt.tick".to_owned(),
        }
    }
}

/// 引擎错误（可读文案，循环即停：rt 帧没有「跳过一帧继续跑」的语义）。
#[derive(Debug)]
pub enum RtError {
    /// 帧派发被驱动拒绝（含 VALIDATION 未注册事件等信封错误）。
    Dispatch {
        n: u64,
        code: String,
        message: String,
    },
    /// 全状态摘要 / 载荷构造失败（内部编程错误量级）。
    State { n: u64, message: String },
    /// 输入事件非法（键码越界 / buttons 负）。
    Input(String),
    /// 录制文件 IO / 解析错误。
    Record(String),
}

impl std::fmt::Display for RtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RtError::Dispatch { n, code, message } => {
                write!(f, "第 {n} 帧派发失败 [{code}]：{message}")
            }
            RtError::State { n, message } => write!(f, "第 {n} 帧状态摘要失败：{message}"),
            RtError::Input(message) => write!(f, "输入事件非法：{message}"),
            RtError::Record(message) => write!(f, "录制/回放文件错误：{message}"),
        }
    }
}

impl std::error::Error for RtError {}

/// 一条输入注入事件（调度面：第 `frame` 帧生效；帧内沿折叠在
/// [`InputDevice::snapshot_for`] 里做）。
#[derive(Clone, Debug, Default)]
pub struct InputEvent {
    pub down: Vec<i64>,
    pub up: Vec<i64>,
    pub mouse_dx: i64,
    pub mouse_dy: i64,
    /// `None` = 本帧不变。
    pub mouse_buttons: Option<i64>,
}

/// 输入设备（宿主侧唯一输入源）：按帧调度注入 + 折叠出每帧快照。
///
/// **没有 Lua 侧写入口**——Mod 伪造输入 = 破坏回放证据链（结构性缺席，
/// rt-face.md 不变量 I3）。R1 的输入来自调度脚本（`--input`）与测试；
/// 实时人入（预览端口回注）按计划归 R2。
///
/// 同帧多事件按**注入序**应用；单事件内同时含同键 down 与 up 属歧义
/// 输入（帧内序无定义），注入时点即拒绝——拆成两次注入或换帧。
#[derive(Default)]
pub struct InputDevice {
    down: BTreeSet<i64>,
    mouse_buttons: i64,
    schedule: BTreeMap<u64, Vec<InputEvent>>,
}

impl InputDevice {
    fn validate_code(code: i64) -> Result<(), RtError> {
        if (1..=RT_KEY_MAX).contains(&code) {
            Ok(())
        } else {
            Err(RtError::Input(format!(
                "键码 {code} 越界（合法 1..={RT_KEY_MAX}）"
            )))
        }
    }

    /// 在第 `frame` 帧注入事件（键码越界 / 同帧同键 up+down 歧义即拒绝
    /// ——错误前移到注入时点）。
    pub fn at(&mut self, frame: u64, event: InputEvent) -> Result<(), RtError> {
        for code in event.down.iter().chain(event.up.iter()) {
            Self::validate_code(*code)?;
        }
        if let Some(buttons) = event.mouse_buttons {
            if buttons < 0 {
                return Err(RtError::Input(format!("鼠标按键掩码为负：{buttons}")));
            }
        }
        // 同帧同键 down+up 混在单事件里 = 帧内序无定义（先 down 后 up 与
        // 先 up 后 down 语义相反）——拒绝，拆两次注入。
        for code in &event.down {
            if event.up.contains(code) {
                return Err(RtError::Input(format!(
                    "同帧同键 {code} 同时 down 与 up（帧内序歧义）：拆成两次注入"
                )));
            }
        }
        // 校验全过后才落槽：同帧多事件按注入序应用（snapshot_for 逐个消费）。
        self.schedule.entry(frame).or_default().push(event);
        Ok(())
    }

    /// 第 `frame` 帧的输入快照：消费调度表里 `≤ frame` 的全部事件，取
    /// **帧初/帧末差集**为沿（帧内同键 down+up 或 up+down = 净零，两侧都
    /// 不出现——一帧一条规范记录，帧是沿的粒度），键序升序（BTreeSet
    /// 天然序 = 规范序）。
    pub fn snapshot_for(&mut self, frame: u64) -> Result<RtInput, RtError> {
        let due: Vec<u64> = self.schedule.range(..=frame).map(|(k, _)| *k).collect();
        let before: BTreeSet<i64> = self.down.clone();
        let mut dx = 0i64;
        let mut dy = 0i64;
        let mut buttons = self.mouse_buttons;
        for key in due {
            // due 取自同一张表的 range 快照，缺席只能来自并发改写（不存在：
            // 单线程 Rc 面）；缺席即跳过而非 panic。键码已在注入时校验。
            let Some(events) = self.schedule.remove(&key) else {
                continue;
            };
            // 注入序逐事件应用；单事件内先 down 后 up（歧义组合已在
            // 注入时拒绝，这里剩下的是无歧义序列）。
            for event in events {
                for code in event.down {
                    self.down.insert(code);
                }
                for code in event.up {
                    self.down.remove(&code);
                }
                dx += event.mouse_dx;
                dy += event.mouse_dy;
                if let Some(b) = event.mouse_buttons {
                    buttons = b;
                }
            }
        }
        self.mouse_buttons = buttons;
        Ok(RtInput {
            held: self.down.iter().copied().collect(),
            pressed: self.down.difference(&before).copied().collect(),
            released: before.difference(&self.down).copied().collect(),
            mouse_dx: dx,
            mouse_dy: dy,
            mouse_buttons: buttons,
        })
    }
}

/// 一帧的录制记录（JSONL 逐行；字段序 = 字典序 = 确定性字节）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameRecord {
    pub n: u64,
    pub dt_ms: i64,
    /// 输入规范串（`k=…;p=…;r=…;m=…`）。
    pub input: String,
    /// 全状态摘要（实体+KV+pending_choice 的 save 信封 SHA-256）。
    pub state: String,
    /// 帧链哈希（含前帧链、调度参数与输入字节）。
    pub hash: String,
    /// 帧渲染证据（S31 R2，**可选**；present 过才有）：本帧末次 present 的
    /// `<w>x<h>|<全幅 digest>`。旁路证据线（D9）——**不入链**：链公式与
    /// R1 录制字节零变化；回放严格对表（录有实无/录无实有均 mismatch）。
    pub frame: Option<String>,
    /// 帧混音证据（S31 R3，**可选**；本帧有活动 voice 才有）：本帧窗口的
    /// `SHA256("ygg.rt.audio.v1|<rate>x1|" ++ s16 字节)`。旁路证据线（I9，
    /// D9 同构）——**不入链**：链公式与 R1 录制字节零变化；回放严格对表
    /// （录有实无/录无实有均 mismatch）。无活动 voice → 整键缺席。
    pub audio: Option<String>,
}

impl FrameRecord {
    /// JSONL 行（serde_json 默认 Map = BTreeMap 字典序，同记录同字节——
    /// 与驱动信封同一确定性口径；不引 serde derive 依赖，字段手写）。
    /// `frame`/`audio` 有则写入（字典序落在各自位：audio < dt_ms < frame <
    /// hash）；无则整键缺席——R1/R2 录制（无渲染/音频面）行字节逐字节不变。
    pub fn to_line(&self) -> Result<String, RtError> {
        let mut line = json!({
            "n": self.n,
            "dt_ms": self.dt_ms,
            "input": self.input,
            "state": self.state,
            "hash": self.hash,
        });
        if let Some(frame) = &self.frame {
            line["frame"] = json!(frame);
        }
        if let Some(audio) = &self.audio {
            line["audio"] = json!(audio);
        }
        serde_json::to_string(&line)
            .map_err(|err| RtError::Record(format!("记录序列化失败：{err}")))
    }

    /// 从 JSON 行重建（缺字段/类型不符即可读错误——录制文件可能被手改，
    /// 解析不静默吞）。`frame` 容缺失（R1 录制无此字段）。
    pub fn from_line(line: &str) -> Result<Self, RtError> {
        let value: Json = serde_json::from_str(line)
            .map_err(|err| RtError::Record(format!("不是合法帧记录：{err}")))?;
        let field = |name: &str| -> Result<String, RtError> {
            value[name]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| RtError::Record(format!("记录缺 `{name}` 字段或非字符串")))
        };
        let n = value["n"]
            .as_u64()
            .ok_or_else(|| RtError::Record("记录缺 `n` 字段或非非负整数".to_owned()))?;
        let dt_ms = value["dt_ms"]
            .as_i64()
            .ok_or_else(|| RtError::Record("记录缺 `dt_ms` 字段或非整数".to_owned()))?;
        let frame = match &value["frame"] {
            Json::Null => None,
            Json::String(text) => Some(text.clone()),
            _ => {
                return Err(RtError::Record(
                    "记录 `frame` 字段非字符串（渲染证据为 `w>x<h>|<64hex>` 串）".to_owned(),
                ))
            }
        };
        let audio = match &value["audio"] {
            Json::Null => None,
            Json::String(text) => Some(text.clone()),
            _ => {
                return Err(RtError::Record(
                    "记录 `audio` 字段非字符串（混音证据为 64hex 窗口摘要）".to_owned(),
                ))
            }
        };
        Ok(Self {
            n,
            dt_ms,
            input: field("input")?,
            state: field("state")?,
            hash: field("hash")?,
            frame,
            audio,
        })
    }
}

/// 回放单步结论：匹配或首处失配（first_mismatch 证伪语义）。
#[derive(Clone, Debug)]
pub struct ReplayStep {
    pub n: u64,
    pub matched: bool,
    pub expected: String,
    pub actual: String,
}

/// 回放终局报告。
#[derive(Clone, Debug)]
pub struct ReplayResult {
    pub frames_checked: u64,
    pub first_mismatch: Option<ReplayStep>,
    /// 回放中途的派发错误（录制合法但重放撞上信封错误等）。
    pub dispatch_error: Option<String>,
}

impl ReplayResult {
    pub fn ok(&self) -> bool {
        self.first_mismatch.is_none() && self.dispatch_error.is_none()
    }
}

/// rt 引擎：定步 + 输入 + 帧链（录制与回放共用同一条哈希链构造）。
pub struct RtHost {
    pub cfg: RtConfig,
    device: InputDevice,
    chain: String,
    next_n: u64,
}

impl RtHost {
    /// 链起源：调度参数入链——配置被改（hz / 事件名）回放即红在第 1 帧。
    pub fn genesis(cfg: &RtConfig) -> String {
        hex_sha256(format!("ygg.rt.v1|event={}|hz={}", cfg.event, cfg.hz).as_bytes())
    }

    pub fn new(cfg: RtConfig) -> Self {
        let chain = Self::genesis(&cfg);
        Self {
            cfg,
            device: InputDevice::default(),
            chain,
            next_n: 1,
        }
    }

    /// 注入输入事件（透传 [`InputDevice::at`]）。
    pub fn input_at(&mut self, frame: u64, event: InputEvent) -> Result<(), RtError> {
        self.device.at(frame, event)
    }

    pub fn next_n(&self) -> u64 {
        self.next_n
    }

    pub fn chain_head(&self) -> &str {
        &self.chain
    }

    /// 录制模式一步：dt 来自定步公式，输入来自设备调度。
    pub fn step_record(&mut self, facade: &mut KernelFacade) -> Result<FrameRecord, RtError> {
        let n = self.next_n;
        let dt = dt_for_frame(n, self.cfg.hz);
        let input = self.device.snapshot_for(n)?;
        let record = self.dispatch_frame(facade, n, dt, &input)?;
        self.next_n += 1;
        Ok(record)
    }

    /// 回放模式一步：dt 与输入都来自录制记录（同序重派），重算链哈希与
    /// 渲染/混音证据并与录制比对——链哈希、frame、audio 任一失配即首处
    /// 失配（三闸：I2 链 ∧ D9 渲染严格对表 ∧ I9 混音严格对表）。
    pub fn step_replay(
        &mut self,
        facade: &mut KernelFacade,
        record: &FrameRecord,
    ) -> Result<ReplayStep, RtError> {
        let input = RtInput::parse(&record.input).map_err(RtError::Record)?;
        let actual = self.dispatch_frame(facade, record.n, record.dt_ms, &input)?;
        self.next_n = record.n + 1;
        // 链哈希比对（I2）+ 帧渲染证据严格对表（D9）+ 帧混音证据严格对表
        // （I9）：录有实无/录无实有/同有不同值均 mismatch——Mod 渲染或音频
        // 行为漂移 = 该红的证据。
        let hash_match = actual.hash == record.hash;
        let frame_match = actual.frame == record.frame;
        let audio_match = actual.audio == record.audio;
        // 失配报告按闸序取第一处：链哈希 → frame → audio（链哈希全等时
        // frame 键单独成证；frame 同样全等时 audio 键单独成证）。
        let (expected, actual_key) = if !hash_match {
            (record.hash.clone(), actual.hash)
        } else if !frame_match {
            (
                record.frame.clone().unwrap_or_else(|| "-".to_owned()),
                actual.frame.clone().unwrap_or_else(|| "-".to_owned()),
            )
        } else {
            (
                record.audio.clone().unwrap_or_else(|| "-".to_owned()),
                actual.audio.clone().unwrap_or_else(|| "-".to_owned()),
            )
        };
        Ok(ReplayStep {
            n: record.n,
            matched: hash_match && frame_match && audio_match,
            expected,
            actual: actual_key,
        })
    }

    /// 单帧核心：闩锁 rt 面 → 派发 → 全状态摘要 → 链推进（旁：帧渲染证据
    /// 捕获，D9 不入链）。
    ///
    /// 失败时 rt 面的帧号/逻辑钟回退（失败帧不留残渣——循环即停，但
    /// 面上不留半步状态）；渲染面同理**不留半帧证据**：派发入口先清 present
    /// 捕获槽，失败路径不产生 FrameRecord。
    fn dispatch_frame(
        &mut self,
        facade: &mut KernelFacade,
        n: u64,
        dt_ms: i64,
        input: &RtInput,
    ) -> Result<FrameRecord, RtError> {
        // 渲染面旁路：先清上一帧残留的 present 捕获槽（上一帧已收走；
        // 这里防御 run 模式同进程混派的时序——槽内只能有本帧的 present）。
        {
            let face = facade.frame_handle();
            face.borrow_mut().take_presented();
        }
        let canonical = input.canonical();
        // 闩锁 rt 面（mode/input/n/逻辑钟都在派发**前**就位：handler 看到的
        // 是本帧自己的帧号与含本帧 dt 的逻辑时刻）；prev 值捕获供失败回退。
        let (prev_n, prev_logical) = {
            let face = facade.rt_handle();
            let mut frame = face.borrow_mut();
            frame.mode = RtMode::Rt;
            frame.input = input.clone();
            let (prev_n, prev_logical) = (frame.n, frame.logical_ms);
            frame.n = n;
            frame.logical_ms = prev_logical + dt_ms;
            (prev_n, prev_logical)
        };
        let payload = frame_payload(n, dt_ms, &canonical);
        let payload_json = match serde_json::to_string(&payload) {
            Ok(text) => text,
            Err(err) => {
                // String 序列化实际上不可能失败；错误路径只为不 panic。
                let face = facade.rt_handle();
                let mut frame = face.borrow_mut();
                frame.n = prev_n;
                frame.logical_ms = prev_logical;
                return Err(RtError::State {
                    n,
                    message: format!("载荷序列化失败：{err}"),
                });
            }
        };
        let reply = facade.dispatch(&self.cfg.event, &payload_json);
        let parsed: Result<Json, _> = serde_json::from_str(&reply);
        let envelope = match parsed {
            Ok(envelope) => envelope,
            Err(err) => {
                let face = facade.rt_handle();
                let mut frame = face.borrow_mut();
                frame.n = prev_n;
                frame.logical_ms = prev_logical;
                return Err(RtError::Dispatch {
                    n,
                    code: "INTERNAL".to_owned(),
                    message: format!("信封解析失败：{err}"),
                });
            }
        };
        if envelope["ok"] != Json::Bool(true) {
            // 面上不留失败帧的残渣（帧号/逻辑钟回退到帧前）。
            let face = facade.rt_handle();
            let mut frame = face.borrow_mut();
            frame.n = prev_n;
            frame.logical_ms = prev_logical;
            return Err(RtError::Dispatch {
                n,
                code: envelope["error"]["code"]
                    .as_str()
                    .unwrap_or("UNKNOWN")
                    .to_owned(),
                message: envelope["error"]["message"]
                    .as_str()
                    .unwrap_or("")
                    .to_owned(),
            });
        }
        let state = state_digest(facade, n)?;
        // 帧渲染证据（D9 旁路，不入链）：本帧末次 present 的 `<w>x<h>|<digest>`；
        // present 未达 = None（半帧不留证据）。
        let frame = {
            let face = facade.frame_handle();
            let taken = face.borrow_mut().take_presented().map(|p| p.record_key());
            taken
        };
        // 帧混音（S31 R3 旁路，不入链——I9）：帧末按 I1 镜像窗口混出本帧
        // s16 样本；handler 内 play 的 voice 当帧窗口即混出（play→混入 = 0
        // 帧），stop 帧内生效。失败路径不会走到这里（提前 return）——失败
        // 帧不留半帧混音证据，voice 残留随循环即停自然终结。
        let audio = {
            let logical = facade.rt_handle().borrow().logical_ms;
            let face = facade.audio_handle();
            let mut mix = face.borrow_mut();
            mix.mix_frame(logical).map(|m| m.digest)
        };
        self.chain =
            hex_sha256(format!("{}|{n}|{dt_ms}|{canonical}|{state}", self.chain).as_bytes());
        Ok(FrameRecord {
            n,
            dt_ms,
            input: canonical,
            state,
            hash: self.chain.clone(),
            frame,
            audio,
        })
    }
}

/// 实时节拍器（D3 的墙钟面）：一拍之内算出应跑帧数，落后补帧封顶
/// `max_catchup`，超出欠账弃掉并计数（螺旋死亡防护）。dt 永远来自定步
/// 公式——墙钟只决定跑几帧，从不决定 dt 值。
///
/// 整数口径：`target = 1e9/hz ns`；`due = 欠账 ns / target ns`（floor）。
/// 封顶时清空欠账（弃帧含零头——下一拍回到正常节拍，不追旧账）。
pub struct Pacer {
    target: Duration,
    max_catchup: u32,
    deficit: Duration,
    last: Option<Instant>,
    dropped_frames: u64,
}

impl Pacer {
    pub fn new(hz: u32, max_catchup: u32) -> Self {
        Self {
            target: Duration::from_nanos(1_000_000_000 / u64::from(hz.max(1))),
            max_catchup: max_catchup.max(1),
            deficit: Duration::ZERO,
            last: None,
            dropped_frames: 0,
        }
    }

    /// 一拍：给当前墙钟时刻，返回本拍应跑帧数（首拍 bootstrap 1 帧）。
    pub fn tick(&mut self, now: Instant) -> u32 {
        let Some(last) = self.last else {
            self.last = Some(now);
            return 1;
        };
        self.deficit += now.saturating_duration_since(last);
        self.last = Some(now);
        let target_ns = self.target.as_nanos().max(1);
        let due = u32::try_from(self.deficit.as_nanos() / target_ns).unwrap_or(u32::MAX);
        if due == 0 {
            return 0;
        }
        if due >= self.max_catchup {
            self.dropped_frames += u64::from(due - self.max_catchup);
            self.deficit = Duration::ZERO; // 弃账含零头：下一拍回到正常节拍
            self.max_catchup
        } else {
            self.deficit -= self.target * due;
            due
        }
    }

    /// 已弃帧数（回执元数据，不入哈希）。
    pub fn dropped_frames(&self) -> u64 {
        self.dropped_frames
    }

    /// 名义帧间隔（调用方 sleep 用）。
    pub fn target(&self) -> Duration {
        self.target
    }
}

/// 全状态摘要：save 信封的 `save` 字段重新序列化字节取 SHA-256。
///
/// 覆盖实体 + KV + pending_choice（`canonical_state_bytes` 只有实体面，
/// KV 才是 mod 游戏状态的大头）；serde_json 对象键序 = BTreeMap 字典序
/// （驱动既定确定性口径），同状态同字节。每帧一次 `dispatch("save","")`
/// ——保留命令、只读、不动状态与燃料，60Hz 下成本实测见 rt-face.md。
fn state_digest(facade: &mut KernelFacade, n: u64) -> Result<String, RtError> {
    let reply = facade.dispatch("save", "");
    let envelope: Json = serde_json::from_str(&reply).map_err(|err| RtError::State {
        n,
        message: format!("save 信封解析失败：{err}"),
    })?;
    if envelope["ok"] != Json::Bool(true) {
        return Err(RtError::State {
            n,
            message: envelope["error"]["message"]
                .as_str()
                .unwrap_or("")
                .to_owned(),
        });
    }
    let save = &envelope["save"];
    let bytes = serde_json::to_vec(save).map_err(|err| RtError::State {
        n,
        message: err.to_string(),
    })?;
    Ok(hex_sha256(&bytes))
}

// ============================================================================
// 录制文件（JSONL：一行一帧，字段序即字节序）
// ============================================================================

/// 写录制文件（父目录须已存在）。
pub fn write_records(path: &Path, records: &[FrameRecord]) -> Result<(), RtError> {
    let mut text = String::new();
    for record in records {
        text.push_str(&record.to_line()?);
        text.push('\n');
    }
    std::fs::write(path, text).map_err(|err| RtError::Record(format!("写入失败：{err}")))
}

/// 读录制文件（空文件 = 空录制；任何一行不合法即错误——不静默跳）。
pub fn read_records(path: &Path) -> Result<Vec<FrameRecord>, RtError> {
    let text =
        std::fs::read_to_string(path).map_err(|err| RtError::Record(format!("读取失败：{err}")))?;
    let mut records = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        records.push(
            FrameRecord::from_line(line)
                .map_err(|err| RtError::Record(format!("第 {} 行：{}", index + 1, err)))?,
        );
    }
    Ok(records)
}

/// 录制 `count` 帧（无墙钟节拍——测试与 `--frames` 批跑共用；遇错即停，
/// 返回已完成的帧：截断录制仍可回放）。
pub fn record_frames(
    facade: &mut KernelFacade,
    host: &mut RtHost,
    count: u64,
    sink: &mut Vec<FrameRecord>,
) -> Result<u64, RtError> {
    let mut done = 0u64;
    for _ in 0..count {
        let record = host.step_record(facade)?;
        sink.push(record);
        done += 1;
    }
    Ok(done)
}

/// 回放一段录制（同序重派 + 逐帧链比对；首处失配即停——证伪语义只需
/// 第一个反例）。回放不产生录制副作用。
pub fn replay_records(
    facade: &mut KernelFacade,
    host: &mut RtHost,
    records: &[FrameRecord],
) -> ReplayResult {
    let mut frames_checked = 0u64;
    for record in records {
        match host.step_replay(facade, record) {
            Ok(step) => {
                frames_checked += 1;
                if !step.matched {
                    return ReplayResult {
                        frames_checked,
                        first_mismatch: Some(step),
                        dispatch_error: None,
                    };
                }
            }
            Err(err) => {
                return ReplayResult {
                    frames_checked,
                    first_mismatch: None,
                    dispatch_error: Some(err.to_string()),
                };
            }
        }
    }
    ReplayResult {
        frames_checked,
        first_mismatch: None,
        dispatch_error: None,
    }
}
