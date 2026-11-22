//! rt 音频面（S31 R3）：AudioFace 单混音面 + 帧窗混音 + WAV 编码。
//!
//! 契约 `docs/design/host/rt-face.md` 音频面节。职责边界（与渲染面同拆分形）：
//! - 本文件 = **纯 Rust 混音面**（无 mlua）：open/play/stop 语义、帧窗样本
//!   序列（I1 镜像）、饱和整数混音（assetgen `sound.mix` 先例）、逐帧 digest
//!   规范序、WAV 44 字节头编码。
//! - Lua 面（`ygg.rt.audio.*` 三键）住 `host_api/audio.rs`；引擎挂点在
//!   `src/rt.rs` 的 `dispatch_frame` 帧末（present 槽旁）。
//! - **audio digest 不入链**（I9，D9 同构旁路）：混音证据走
//!   `FrameRecord.audio` 加性可选键——链公式与 R1/R2 录制字节零变化。
//!
//! 数据形状（全整数，零浮点）：
//! - 样本 = `i16` s16 小端；单混音面 v1，ch 固定 1（Saltline bundle 10 条
//!   全单声道，mono 下 pan 无定义——ch=2+pan 同轮才有意义，推 S30+）。
//! - rate ∈ 8000..=48000，缺省 22050（= Saltline bundle sr）。
//! - 窗口样本数 = I1 镜像：`samples(n) = ⌊logical_n·rate/1000⌋ −
//!   ⌊logical_{n−1}·rate/1000⌋`——纯整数、零漂移；22050@60Hz 每 1000ms
//!   恰 22050 样本（KAT 钉值见 `tests/audio_face.rs`）。
//! - 混音 = **饱和整数加法**（assetgen `sound.mix` 先例：各层整数和、
//!   [-32768, 32767] 钳制、不做二次归一）；gain 千分比整数在**入队时烘焙**
//!   （后改不追溯 = 确定性）。
//! - 逐帧 digest 规范序 = `SHA256("ygg.rt.audio.v1|<rate>x1|" ++ 本帧窗口
//!   s16 小端字节)`；本帧无活动 voice → `FrameRecord.audio` 整键缺席
//!   （镜像 frame 键规则）——整局流仍照写静音窗口（WAV 时长不失真）。

use std::path::Path;

use crate::hash::hex_sha256;

/// 采样率合法区间下界。
pub const AUDIO_RATE_MIN: i64 = 8000;
/// 采样率合法区间上界。
pub const AUDIO_RATE_MAX: i64 = 48000;
/// 缺省采样率（= Saltline bundle 10 条 sfx 的 sr）。
pub const AUDIO_DEFAULT_RATE: i64 = 22050;
/// 并发 voice 上限（超限 `YGG-RT-AUDIO-CAP`；混完或 stop 释放后可再 play）。
pub const AUDIO_MAX_VOICES: usize = 64;
/// gain 缺省千分比（1000 = 原样）。
pub const AUDIO_GAIN_DEFAULT: i64 = 1000;
/// gain 合法区间（千分比整数：0 = 静音，2000 = 2 倍烘焙钳制）。
pub const AUDIO_GAIN_MIN: i64 = 0;
pub const AUDIO_GAIN_MAX: i64 = 2000;

/// 音频面错误（Lua 面转 `{code}: 文案` 可读错误，与渲染面同族）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioError {
    /// 参数形状/状态非法（未 open、rate 越界、pcm 非法、gain 越界）。
    Shape(String),
    /// 面已 open（v1 单混音面：装载期开一面，再开拒绝）。
    Opened,
    /// 并发 voice 超上限。
    Cap(String),
}

impl AudioError {
    /// 拒绝码（与 `ygg.rt.*` 既有错误码同族）。
    pub fn code(&self) -> &'static str {
        match self {
            AudioError::Shape(_) => "YGG-RT-SHAPE",
            AudioError::Opened => "YGG-RT-OPENED",
            AudioError::Cap(_) => "YGG-RT-AUDIO-CAP",
        }
    }
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioError::Shape(message) => write!(f, "{message}"),
            AudioError::Opened => write!(f, "audio 面已 open（v1 单混音面）"),
            AudioError::Cap(message) => write!(f, "{message}"),
        }
    }
}

/// 一帧的混音证据（引擎收进 `FrameRecord.audio`；不入链——I9）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioMix {
    pub rate: u32,
    pub digest: String,
}

/// 一条活跃 voice：s16 样本在**入队时烘焙** gain，cursor 指向下一帧窗口
/// 要消费的样本（voice 跨帧续播 = 帧序即播放序，录制即真源）。
struct Voice {
    id: i64,
    samples: Vec<i16>,
    cursor: usize,
}

/// 单混音面（v1：`ygg.rt.audio.open` 装载期开一面，handle 恒 1 起）。
///
/// 住 `HostState` 的共享 `Rc`（与 rt 闩锁面/渲染面同纪律）：Mod 经 Lua
/// 三键写（open/play/stop），引擎每帧帧末读（[`AudioFace::mix_frame`]），
/// 宿主关停时取整局流出 WAV。
#[derive(Default)]
pub struct AudioFace {
    rate: Option<u32>,
    handle: i64,
    voices: Vec<Voice>,
    next_voice: i64,
    /// 上次混音的逻辑钟（窗口公式下界；`mix_frame` 每帧推进）。
    logical_ms: i64,
    /// 整局混音流（s16le 单声道字节 = WAV data 段；含静音窗口）。
    session: Vec<u8>,
}

impl AudioFace {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_open(&self) -> bool {
        self.rate.is_some()
    }

    pub fn handle(&self) -> i64 {
        self.handle
    }

    /// 当前采样率（未 open = `None`）。
    pub fn rate(&self) -> Option<u32> {
        self.rate
    }

    /// 整局混音流（WAV data 段字节；宿主关停时写 `--audio` WAV）。
    pub fn session(&self) -> &[u8] {
        &self.session
    }

    /// `audio.open`：装载期限定由 Lua 面执行（seal 检查），此处只管形状与
    /// 单面语义。rate 缺省 22050，合法 8000..=48000；再开 =
    /// [`AudioError::Opened`]。
    pub fn open(&mut self, rate: Option<i64>) -> Result<i64, AudioError> {
        let rate = rate.unwrap_or(AUDIO_DEFAULT_RATE);
        if !(AUDIO_RATE_MIN..=AUDIO_RATE_MAX).contains(&rate) {
            return Err(AudioError::Shape(format!(
                "rate 必须 {AUDIO_RATE_MIN}..={AUDIO_RATE_MAX}，得到 {rate}"
            )));
        }
        if self.rate.is_some() {
            return Err(AudioError::Opened);
        }
        self.handle += 1; // 整数 1 起
        self.rate = Some(rate as u32);
        Ok(self.handle)
    }

    /// `audio.play`：pcm = s16 小端单声道串（0 < 样本数 ≤ rate×10s），
    /// gain 千分比整数（入队时烘焙进样本：`s·gain/1000` 截断除法后饱和钳
    /// 制到 i16——gain=2000 的峰值翻倍在此定形，混音层不再缩放）。
    /// 返回 voice 整数 1 起；并发 ≥ 64 超限 [`AudioError::Cap`]。
    ///
    /// play→混入延迟 = **0 帧**：handler 内 play 的 voice 当帧帧末窗口即
    /// 混出（帧同步混音 = 确定性；混音序 = play 序 = 录制序）。
    pub fn play(&mut self, pcm: &[u8], gain: i64) -> Result<i64, AudioError> {
        let rate = self
            .rate
            .ok_or_else(|| AudioError::Shape("audio 未 open（open 为装载期限定）".to_owned()))?;
        if pcm.is_empty() || !pcm.len().is_multiple_of(2) {
            return Err(AudioError::Shape(format!(
                "pcm 必须是非零偶数字节的 s16 小端串，得到 {} 字节",
                pcm.len()
            )));
        }
        let samples = pcm.len() / 2;
        if samples as i64 > rate as i64 * 10 {
            return Err(AudioError::Shape(format!(
                "pcm 时长超限（≤ rate×10s = {} 样本），得到 {samples}",
                rate as i64 * 10
            )));
        }
        if !(AUDIO_GAIN_MIN..=AUDIO_GAIN_MAX).contains(&gain) {
            return Err(AudioError::Shape(format!(
                "gain 必须 {AUDIO_GAIN_MIN}..={AUDIO_GAIN_MAX} 的千分比整数，得到 {gain}"
            )));
        }
        if self.voices.len() >= AUDIO_MAX_VOICES {
            return Err(AudioError::Cap(format!(
                "并发 voice 已达 {AUDIO_MAX_VOICES}（混完或 stop 释放后再 play）"
            )));
        }
        // 入队时烘焙（后改不追溯 = 确定性）：s32 中间量不溢出
        // （32767×2000 = 6.55e7 < 2^31），除法截断向零、末端饱和钳制。
        let baked: Vec<i16> = pcm
            .as_chunks::<2>()
            .0
            .iter()
            .map(|chunk| {
                let s = i16::from_le_bytes([chunk[0], chunk[1]]) as i32;
                (s * gain as i32 / 1000).clamp(-32768, 32767) as i16
            })
            .collect();
        self.next_voice += 1;
        self.voices.push(Voice {
            id: self.next_voice,
            samples: baked,
            cursor: 0,
        });
        Ok(self.next_voice)
    }

    /// `audio.stop`：即刻静默（帧内生效——handler 内 stop 的 voice 不进
    /// 本帧窗口；混音序 = 帧序 = 录制序）。已混完/未知的 voice = no-op
    /// Ok（voice 混完即出队，v1 不留墓碑：与「从未 play 过」不可分辨，
    /// 静默已静默者成功；确定性不受影响）。
    pub fn stop(&mut self, voice: i64) {
        self.voices.retain(|v| v.id != voice);
    }

    /// 活跃 voice 数（诊断面/容量口径）。
    pub fn active_voices(&self) -> usize {
        self.voices.len()
    }

    /// 帧末混音（引擎在 dispatch 成功后、链推进前调用；每帧恰一次）。
    ///
    /// 窗口 = I1 镜像（整数零漂移）；voice 按播放序取头饱和累加、跨帧
    /// 续播。本帧**无活动 voice** → 窗口照写静音进整局流（WAV 时长不
    /// 失真）、返回 `None`（`FrameRecord.audio` 整键缺席）；有 → 返回
    /// 本帧窗口 digest（规范序见 [`audio_digest`]）。未 open = no-op
    /// `None`（R1/R2 mod 无音频面 = 引擎挂点全帧静默通过）。
    pub fn mix_frame(&mut self, logical_ms: i64) -> Option<AudioMix> {
        let rate = self.rate?;
        let prev = self.logical_ms;
        self.logical_ms = logical_ms;
        let n = window_samples(prev, logical_ms, rate).max(0) as usize;
        let mut window = vec![0i16; n];
        // 有无活动 voice 以**循环前队列**为准：voice 恰在本帧混完的帧
        // 有贡献（digest 在场），不算缺席。
        let had_voices = !self.voices.is_empty();
        for voice in &mut self.voices {
            let take = (voice.samples.len() - voice.cursor).min(n);
            for (slot, sample) in window.iter_mut().zip(&voice.samples[voice.cursor..]) {
                let sum = i32::from(*slot) + i32::from(*sample);
                *slot = sum.clamp(-32768, 32767) as i16;
            }
            voice.cursor += take;
        }
        self.voices.retain(|v| v.cursor < v.samples.len());
        let mut bytes = Vec::with_capacity(n * 2);
        for sample in &window {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        self.session.extend_from_slice(&bytes);
        if !had_voices {
            return None;
        }
        Some(AudioMix {
            rate,
            digest: audio_digest(rate, &bytes),
        })
    }
}

/// 窗口样本数（I1 镜像，纯函数）：`⌊logical·rate/1000⌋ − ⌊prev·rate/1000⌋`
/// ——纯整数、零漂移；60Hz×22050 每 60 帧积分恰 22050 样本。
pub fn window_samples(prev_logical_ms: i64, logical_ms: i64, rate: u32) -> i64 {
    let rate = i64::from(rate);
    (logical_ms * rate) / 1000 - (prev_logical_ms * rate) / 1000
}

/// 音频 digest（I9 证据线锚）：
/// `SHA256("ygg.rt.audio.v1|<rate>x1|" ++ 本帧窗口 s16 小端字节)`——rate 入
/// 前像防异率同字节碰撞；ch 固定 1 入前像（`x1`），ch=2+ 属 S30+ 轮。
pub fn audio_digest(rate: u32, window: &[u8]) -> String {
    let mut bytes = Vec::with_capacity(24 + window.len());
    bytes.extend_from_slice(format!("ygg.rt.audio.v1|{rate}x1|").as_bytes());
    bytes.extend_from_slice(window);
    hex_sha256(&bytes)
}

/// WAV 字节（44 字节 RIFF 头 + s16 小端交织；assetgen `wav16` 恒等先例：
/// u32le/u16le 手摆、ch=1、`byteRate = sr·ch·2`、`blockAlign = ch·2`）。
pub fn wav_bytes(rate: u32, pcm: &[u8]) -> Vec<u8> {
    let data = pcm.len() as u32;
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&data.wrapping_add(36).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // ch = 1
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes()); // byteRate
    out.extend_from_slice(&2u16.to_le_bytes()); // blockAlign
    out.extend_from_slice(&16u16.to_le_bytes()); // bitsPerSample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

/// WAV 落盘（`examples/rt --audio` 关停时定稿：44 字节头 + 整局流）。
pub fn write_wav(path: &Path, rate: u32, pcm: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, wav_bytes(rate, pcm))
}
