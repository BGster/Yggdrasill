//! rt 渲染面缓冲（S31 R2）：FrameFace 位图缓冲 + 帧 digest + PPM 编码。
//!
//! 契约 `docs/design/host/rt-face.md` 渲染面节。职责边界：
//! - 本文件 = **纯 Rust 缓冲面**（无 mlua）：像素直写语义（零混色）、越界
//!   钳制、脏区并集、全幅 digest 规范序、PPM 字节。
//! - Lua 面（`ygg.rt.frame.*` 五键）住 `host_api/frame.rs`——与 rt 引擎
//!   （`src/rt.rs` 引擎 + `host_api/rt.rs` Lua 面）同一拆分形。
//! - **帧 digest 不入链**（D9）：present 的 digest 走旁路捕获槽
//!   （[`FrameFace::take_presented`]），由引擎在派发成功后收进
//!   `FrameRecord.frame`——链公式与 R1 录制字节零变化。
//!
//! 数据形状（全整数，零浮点）：
//! - 像素 = `u32` 打包 `0xRRGGBBAA`（大端字节即 RGBA8 字节序 R,G,B,A）。
//! - 缓冲 = 行主序、顶行在前；w,h ∈ 1..=4096（4096²×4 = 64MiB 封顶）。
//! - 写入三键（clear/span/blit）全部**直写覆盖、零混色**——Saltline 墙/天/地
//!   全不透明；混色（alpha blend）属 R3+ 裁决，内核最小面不预支。
//! - digest 规范序 = `SHA256("ygg.rt.frame.v1|<w>x<h>|" ++ RGBA8 行主序顶行
//!   在前原始字节)`——尺寸入前像防异形同字节碰撞（KAT 钉值见
//!   `tests/frame_face.rs`，K1–K4 以 python3 hashlib 预算复算钉死）。

use std::path::Path;

use crate::hash::hex_sha256;

/// 单边上限：w,h ∈ 1..=4096（≤64MiB 封顶，内存/GC 理由）。
pub const FRAME_MAX_EDGE: i64 = 4096;

/// 渲染面错误（Lua 面转 `{YGG-RT-SHAPE}|{YGG-RT-OPENED}: 文案` 可读错误）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// 参数形状/状态非法（未 open、handle 不符、负尺寸、长度不整除、越界尺寸）。
    Shape(String),
    /// 面已 open（v1 单面：装载期开一面，再开拒绝）。
    Opened,
}

impl FrameError {
    /// 拒绝码（与 `ygg.rt.*` 既有错误码同族）。
    pub fn code(&self) -> &'static str {
        match self {
            FrameError::Shape(_) => "YGG-RT-SHAPE",
            FrameError::Opened => "YGG-RT-OPENED",
        }
    }
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::Shape(message) => write!(f, "{message}"),
            FrameError::Opened => write!(f, "frame 面已 open（v1 单面）"),
        }
    }
}

/// 矩形（非负整数；钳制后必然落在缓冲界内）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// present 的旁路捕获（不入链——D9）：引擎每派发收进 `FrameRecord.frame`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FramePresent {
    pub w: u32,
    pub h: u32,
    pub digest: String,
}

impl FramePresent {
    /// 录制字段形态：`"<w>x<h>|<digest>"`（尺寸+摘要一串，回放严格对表）。
    pub fn record_key(&self) -> String {
        format!("{}x{}|{}", self.w, self.h, self.digest)
    }
}

/// present 的回执（Lua 面转 `{digest, dirty, frame}` 表）。
pub struct PresentInfo {
    pub digest: String,
    /// 自上帧 present 以来写并集矩形；无写 = `None`（Lua nil）。
    pub dirty: Option<Rect>,
    /// 触发 present 的 rt 帧号（引擎闩锁面读出）。
    pub frame: u64,
}

/// 输出通道快照（PPM/SSE 的唯一数据源；宿主每帧在派发后取）。
#[derive(Clone, Debug)]
pub struct FrameSnapshot {
    pub w: u32,
    pub h: u32,
    pub pixels: Vec<u32>,
}

impl FrameSnapshot {
    /// 全幅 digest（与 present 同规范序——PPM 与 SSE 两通道对账的锚）。
    pub fn digest(&self) -> String {
        frame_digest(self.w, self.h, &self.pixels)
    }
}

/// 位图缓冲（v1 单面：`ygg.rt.frame.open` 装载期开一面，handle 恒 1 起）。
///
/// 住 `HostState` 的共享 `Rc`（与 rt 闩锁面同纪律）：Mod 经 Lua 五键写，
/// 引擎/宿主读（旁路捕获槽 + 快照）。跨帧持久——clear/blit 语义全由 Mod
/// 决定（不 clear = 上一帧残留，仍确定）。
#[derive(Default)]
pub struct FrameFace {
    buf: Option<FrameBuf>,
    handle: i64,
    dirty: Option<Rect>,
    presented: Option<FramePresent>,
}

struct FrameBuf {
    w: u32,
    h: u32,
    pixels: Vec<u32>,
}

impl FrameFace {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_open(&self) -> bool {
        self.buf.is_some()
    }

    pub fn handle(&self) -> i64 {
        self.handle
    }

    /// `frame.open`：装载期限定由 Lua 面执行（seal 检查），此处只管形状与
    /// 单面语义。w,h ∈ 1..=4096；再开 = [`FrameError::Opened`]。
    pub fn open(&mut self, w: i64, h: i64) -> Result<i64, FrameError> {
        if !(1..=FRAME_MAX_EDGE).contains(&w) || !(1..=FRAME_MAX_EDGE).contains(&h) {
            return Err(FrameError::Shape(format!(
                "尺寸必须 1..={FRAME_MAX_EDGE}，得到 {w}x{h}"
            )));
        }
        if self.buf.is_some() {
            return Err(FrameError::Opened);
        }
        self.handle += 1; // 整数 1 起
        self.buf = Some(FrameBuf {
            w: w as u32,
            h: h as u32,
            pixels: vec![0; (w * h) as usize],
        });
        Ok(self.handle)
    }

    fn buf_mut(&mut self) -> Result<&mut FrameBuf, FrameError> {
        self.buf
            .as_mut()
            .ok_or_else(|| FrameError::Shape("frame 未 open".to_owned()))
    }

    /// 全缓冲填 rgba；脏区 = 全幅。
    pub fn clear(&mut self, rgba: u32) -> Result<(), FrameError> {
        let buf = self.buf_mut()?;
        buf.pixels.fill(rgba);
        let full = Rect {
            x: 0,
            y: 0,
            w: buf.w,
            h: buf.h,
        };
        union_rect(&mut self.dirty, full);
        Ok(())
    }

    /// 实心矩形段：x,y 可负（钳制），w,hh ≥ 0（负 = [`FrameError::Shape`]）；
    /// 越界部分静默裁掉（射线柱超屏是常态非错误），钳后零面积 = no-op。
    pub fn span(&mut self, x: i64, y: i64, w: i64, hh: i64, rgba: u32) -> Result<(), FrameError> {
        if w < 0 || hh < 0 {
            return Err(FrameError::Shape(format!(
                "span 宽高必须非负，得到 w={w},h={hh}"
            )));
        }
        let buf = self.buf_mut()?;
        let (bw, bh) = (i64::from(buf.w), i64::from(buf.h));
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + w).min(bw);
        let y1 = (y + hh).min(bh);
        if x1 <= x0 || y1 <= y0 {
            return Ok(());
        }
        for yy in y0..y1 {
            let base = (yy * bw) as usize;
            let px = &mut buf.pixels[base + x0 as usize..base + x1 as usize];
            px.fill(rgba);
        }
        union_rect(
            &mut self.dirty,
            Rect {
                x: x0 as u32,
                y: y0 as u32,
                w: (x1 - x0) as u32,
                h: (y1 - y0) as u32,
            },
        );
        Ok(())
    }

    /// 行串 blit：bytes = RGBA8 行主序，len 必须是 w×4 的正整数倍
    /// （rows = len/(w·4) ≥ 1；不整除 = [`FrameError::Shape`]）；直写覆盖、
    /// 越界钳制（半出界 blit 的入界部分照写）。
    pub fn blit(&mut self, x: i64, y: i64, w: i64, bytes: &[u8]) -> Result<(), FrameError> {
        if w <= 0 {
            return Err(FrameError::Shape(format!("blit 宽必须正，得到 {w}")));
        }
        let stride = w as usize * 4;
        if bytes.len() < stride || !bytes.len().is_multiple_of(stride) {
            return Err(FrameError::Shape(format!(
                "bytes 长度 {} 不是 {w}×4={stride} 的正整数倍",
                bytes.len()
            )));
        }
        let rows = bytes.len() / stride;
        let buf = self.buf_mut()?;
        let (bw, bh) = (i64::from(buf.w), i64::from(buf.h));
        // 目标可见窗口；源窗口与之同形（源列/行偏移 = 目标 − 原点）。
        let vx0 = x.max(0);
        let vx1 = (x + w).min(bw);
        let vy0 = y.max(0);
        let vy1 = (y + rows as i64).min(bh);
        if vx1 <= vx0 || vy1 <= vy0 {
            return Ok(());
        }
        for ty in vy0..vy1 {
            let srow = ((ty - y) as usize) * stride;
            let tbase = (ty * bw) as usize;
            for tx in vx0..vx1 {
                let so = srow + (tx - x) as usize * 4;
                let be = [bytes[so], bytes[so + 1], bytes[so + 2], bytes[so + 3]];
                buf.pixels[tbase + tx as usize] = u32::from_be_bytes(be);
            }
        }
        union_rect(
            &mut self.dirty,
            Rect {
                x: vx0 as u32,
                y: vy0 as u32,
                w: (vx1 - vx0) as u32,
                h: (vy1 - vy0) as u32,
            },
        );
        Ok(())
    }

    /// present：全幅 digest + 脏区**读后复位** + 旁路捕获（同帧多次 present
    /// 合法，末次为准——捕获槽覆盖写）。
    pub fn present(&mut self, frame_n: u64) -> Result<PresentInfo, FrameError> {
        let buf = self
            .buf
            .as_ref()
            .ok_or_else(|| FrameError::Shape("frame 未 open".to_owned()))?;
        let digest = frame_digest(buf.w, buf.h, &buf.pixels);
        let dirty = self.dirty.take();
        self.presented = Some(FramePresent {
            w: buf.w,
            h: buf.h,
            digest: digest.clone(),
        });
        Ok(PresentInfo {
            digest,
            dirty,
            frame: frame_n,
        })
    }

    /// 旁路捕获槽取走（引擎两个调用点：派发**入口**清残——失败帧不留半帧
    /// 证据；派发成功后收证据进 `FrameRecord.frame`）。
    pub fn take_presented(&mut self) -> Option<FramePresent> {
        self.presented.take()
    }

    /// 输出通道快照（presented 与否由调用方以 `FrameRecord.frame` 判断）。
    pub fn snapshot(&self) -> Option<FrameSnapshot> {
        let buf = self.buf.as_ref()?;
        Some(FrameSnapshot {
            w: buf.w,
            h: buf.h,
            pixels: buf.pixels.clone(),
        })
    }
}

/// 脏区并集（最小外包矩形；None = 首写）。
fn union_rect(slot: &mut Option<Rect>, r: Rect) {
    *slot = Some(match *slot {
        None => r,
        Some(a) => {
            let x0 = a.x.min(r.x);
            let y0 = a.y.min(r.y);
            Rect {
                x: x0,
                y: y0,
                w: (a.x + a.w).max(r.x + r.w) - x0,
                h: (a.y + a.h).max(r.y + r.h) - y0,
            }
        }
    });
}

/// 帧 digest（D9 证据线锚）：
/// `SHA256("ygg.rt.frame.v1|<w>x<h>|" ++ RGBA8 行主序顶行在前原始字节)`。
pub fn frame_digest(w: u32, h: u32, pixels: &[u32]) -> String {
    let mut bytes = Vec::with_capacity(32 + pixels.len() * 4);
    bytes.extend_from_slice(format!("ygg.rt.frame.v1|{w}x{h}|").as_bytes());
    for p in pixels {
        bytes.extend_from_slice(&p.to_be_bytes());
    }
    hex_sha256(&bytes)
}

/// PPM（P6）落盘：头 `P6\n<w> <h>\n255\n` + RGB 三元组（弃 alpha）。
/// headless/selftest/KAT 通道；帧号即名由调用方拼（`f%06d.ppm`）。
pub fn write_ppm(path: &Path, snap: &FrameSnapshot) -> std::io::Result<()> {
    let mut bytes = format!("P6\n{} {}\n255\n", snap.w, snap.h).into_bytes();
    bytes.reserve(snap.pixels.len() * 3);
    for p in &snap.pixels {
        bytes.extend_from_slice(&p.to_be_bytes()[..3]);
    }
    std::fs::write(path, bytes)
}
