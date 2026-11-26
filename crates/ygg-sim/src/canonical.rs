//! 帧原语：定宽小端读写 + 尾 digest 的唯一实现（快照与批量帧共用）。
//!
//! 契约：sim-kernel §2「编码序澄清」——复用 codec 的**排序规范**（键/组件名按字节序、
//! 整数定宽 LE、无浮点、版本前置），容器是二进制而不是文本。
//!
//! # 为什么只有一份读写器
//!
//! 快照能解的列，批量帧也必须能解——否则「存档里能读出来的世界，帧里写不进去」会变成
//! 一条要人反复核对的隐式规则。共用 [`FrameWriter`] / [`FrameReader`] 之后，这条性质是
//! **结构性质**而不是两套代码的巧合。
//!
//! # 尾 digest 的纪律
//!
//! 每个帧（快照或批量）以 `fnv1a64(前面全部字节)` 的 8 字节小端结尾。于是：
//! - 读取方在 [`FrameReader::finish`] 一次校验，坏字节 fail-closed（不 panic）；
//! - 快照的尾 digest **就是** state hash（见 [`crate::hash::state_hash`]），所以
//!   `restore(b)` 的校验与 `hash()` 的取值是同一个数、同一次计算——不会出现「restore 通过
//!   但 hash 不同」这种需要第三种解释的状态。

use crate::error::{SimError, SimResult};

/// 帧魔数（8 字节，含格式族与主版本位）。
pub const MAGIC: [u8; 8] = *b"YGGSIM\0\x01";

/// 帧格式版本（与 [`MAGIC`] 里的主版本独立：主版本变了魔数变，布局微调变这里）。
///
/// 历史：1 = 首版（列型 1..5，Number/Str/Bool/Ref/Tag）；2 = R1′ 位串列（新增列型 6 =
/// F64Bits，其余布局不变）。读取器收 {1, 2}：v1 帧原语义兼容（组件表 type 字节
/// 断言 ≤ 5，见 `snapshot::decode`），同名组件类型冲突由 `Registry::register` 检查
/// fail-closed。
pub const FMT_VERSION: u16 = 2;

/// 追加式帧写入器（只增字节，最后一次性补 digest）。
#[derive(Debug, Default)]
pub struct FrameWriter {
    buf: Vec<u8>,
}

impl FrameWriter {
    /// 新帧（自动写入魔数与版本头——忘记写头是这类编码最常见的错，所以不给「空帧」入口）。
    pub fn new() -> Self {
        let mut writer = Self { buf: Vec::new() };
        writer.raw(&MAGIC);
        writer.u16(FMT_VERSION);
        writer.u16(0); // flags（保留；非 0 的语义要新增一份契约，不是就地约定）
        writer
    }

    /// 从已存在的字节续写（restore 后重新编码时必须逐字节一致，不允许换编码器）。
    pub fn from_bytes(buf: Vec<u8>) -> Self {
        Self { buf }
    }

    /// 已写入的字节数。
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// 是否空帧。
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// 原始字节（不做 digest）。
    pub fn raw(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// 1 字节。
    pub fn u8(&mut self, value: u8) {
        self.buf.push(value);
    }

    /// 2 字节小端。
    pub fn u16(&mut self, value: u16) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// 4 字节小端。
    pub fn u32(&mut self, value: u32) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// 8 字节小端。
    pub fn u64(&mut self, value: u64) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// 有符号 64 位（补码原样，不做任何归一化）。
    pub fn i64(&mut self, value: i64) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// 长度前缀（`u32` LE）+ 字节。
    pub fn bytes(&mut self, bytes: &[u8]) -> SimResult<()> {
        let len = u32::try_from(bytes.len())
            .map_err(|_| SimError::frame(format!("单段字节超过 4 GiB：{}", bytes.len())))?;
        self.u32(len);
        self.raw(bytes);
        Ok(())
    }

    /// 收尾：补 8 字节尾 digest，返回完整帧。
    pub fn finish(self) -> Vec<u8> {
        let mut buf = self.buf;
        let digest = crate::hash::fnv1a64(&buf);
        buf.extend_from_slice(&digest.to_le_bytes());
        buf
    }
}

/// 只读帧解析器（越界一律 [`SimError::frame`]，不 panic、不返回默认值）。
#[derive(Debug, Clone)]
pub struct FrameReader<'a> {
    buf: &'a [u8],
    pos: usize,
    /// 帧头里读到的 `fmt_version`（快照 decode 按它做 v1/v2 分发）。
    version: u16,
}

impl<'a> FrameReader<'a> {
    /// 打开帧并校验魔数与版本（写入器只产出 [`FMT_VERSION`]；读取器收历史版本集
    /// {1, 2}——旧档原语义兼容，更老/更新的版本一律拒：静默兼容会让旧档读成新世界）。
    ///
    /// 头三类不过报 [`SimError::format`]（「这不是我们能读的帧」），与后续的
    /// 截断 / 长度 / digest 不符（[`SimError::frame`]，「帧坏了」）分开：宿主对
    /// 两者的处置不同。
    pub fn open(buf: &'a [u8]) -> SimResult<Self> {
        let mut reader = Self {
            buf,
            pos: 0,
            version: 0,
        };
        let magic = reader.raw(8)?;
        if magic != MAGIC {
            return Err(SimError::format(format!(
                "魔数不符：得到 {magic:02X?}，期望 {MAGIC:02X?}"
            )));
        }
        let version = reader.u16()?;
        if version != 1 && version != 2 {
            return Err(SimError::format(format!(
                "帧格式版本不符：得到 {version}，可读 {{1, 2}}（本写入器产出 {FMT_VERSION}）"
            )));
        }
        reader.version = version;
        let flags = reader.u16()?;
        if flags != 0 {
            return Err(SimError::format(format!(
                "未知 flags：{flags}（保留位必须为 0）"
            )));
        }
        Ok(reader)
    }

    /// 帧头里的 `fmt_version`（1 或 2；[`FrameReader::open`] 已验过）。
    pub const fn version(&self) -> u16 {
        self.version
    }

    /// 已消费字节数。
    pub fn position(&self) -> usize {
        self.pos
    }

    /// 剩余字节数。
    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    /// 取 `len` 字节（越界即拒）。
    pub fn raw(&mut self, len: usize) -> SimResult<&'a [u8]> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| SimError::frame("帧长度溢出"))?;
        if end > self.buf.len() {
            return Err(SimError::frame(format!(
                "帧提前结束：需要 {len} 字节，剩余 {}",
                self.remaining()
            )));
        }
        let slice = &self.buf[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    /// 1 字节。
    pub fn u8(&mut self) -> SimResult<u8> {
        Ok(self.raw(1)?[0])
    }

    /// 2 字节小端。
    pub fn u16(&mut self) -> SimResult<u16> {
        let bytes = self.raw(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    /// 4 字节小端。
    pub fn u32(&mut self) -> SimResult<u32> {
        let bytes = self.raw(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// 8 字节小端。
    pub fn u64(&mut self) -> SimResult<u64> {
        let bytes = self.raw(8)?;
        let mut buf = [0u8; 8];
        buf.copy_from_slice(bytes);
        Ok(u64::from_le_bytes(buf))
    }

    /// 有符号 64 位。
    pub fn i64(&mut self) -> SimResult<i64> {
        Ok(self.u64()? as i64)
    }

    /// 长度前缀 + 字节。
    pub fn bytes(&mut self) -> SimResult<&'a [u8]> {
        let len = self.u32()? as usize;
        self.raw(len)
    }

    /// 长度前缀 + UTF-8 文本（非法 UTF-8 即拒）。
    pub fn text(&mut self) -> SimResult<&'a str> {
        let bytes = self.bytes()?;
        std::str::from_utf8(bytes)
            .map_err(|err| SimError::frame(format!("帧里的文本不是合法 UTF-8：{err}")))
    }

    /// 收尾：校验尾 digest 并确认**没有多余字节**。
    ///
    /// 「没有多余字节」是硬要求：尾随垃圾要么是版本错位、要么是被拼接的帧，两种都不该
    /// 被静默忽略（静默忽略会让「加了一个字段但忘了升版本」永远不被发现）。
    pub fn finish(mut self) -> SimResult<()> {
        if self.remaining() < 8 {
            return Err(SimError::frame(format!(
                "帧缺少尾 digest：剩余 {} 字节",
                self.remaining()
            )));
        }
        let digest_at = self.buf.len() - 8;
        let expected = crate::hash::fnv1a64(&self.buf[..digest_at]);
        let mut tail = [0u8; 8];
        tail.copy_from_slice(&self.buf[digest_at..]);
        let found = u64::from_le_bytes(tail);
        if found != expected {
            return Err(SimError::frame(format!(
                "尾 digest 不符：帧内 {found:016x}，实算 {expected:016x}"
            )));
        }
        if self.pos != digest_at {
            return Err(SimError::frame(format!(
                "帧有 {} 字节尾随数据未消费",
                digest_at - self.pos
            )));
        }
        self.pos = self.buf.len();
        Ok(())
    }
}
