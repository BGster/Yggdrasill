//! 输入日志（`ReplayLog`）的帧编解码：`{ seed, 逐 tick (dt, hash, 结构操作) }`。
//!
//! 契约：sim-kernel §2（`ReplayLog` 形状）+ I1/I3（同 seed + 同日志 ⇒ 逐 tick 字节一致）。
//!
//! # 为什么日志里要带「当时算出的 hash」
//!
//! 回放的价值是**证伪**：拿录下的逐 tick hash 与新算的比，第一个不等处就是失配点。
//! 如果日志只记输入不记 hash，回放就只能证明「跑完了」，证明不了「跑对了」——
//! 那是恒真的断言（`tests/replay_tick_hash.rs` 的负例正是为此存在：篡改一帧 dt
//! 必须被抓成 `first_mismatch = Some(该 tick)`）。
//!
//! # 结构操作为什么按原字节存
//!
//! 批量帧已经是自描述的（列式 + 尾 digest），回放时**原样重投**给同一个解码器。
//! 若在日志里重编码一遍，就有两个编码器要同步——而回放要证明的恰恰是「同一个解码器
//! 给出同一结果」。
//!
//! 帧布局：
//!
//! ```text
//! magic+version+flags（canonical::FrameWriter::new）
//! seed        8B   LE
//! start_tick  8B   LE   （0 = 从创世回放；否则 = 快照所在 tick）
//! n           4B   LE
//!   × n: tick 8B | dt 8B | hash 8B | op_n 4B
//!        × op_n: kind 1B（1=spawn 2=despawn 3=insert）| len 4B | 原始批帧字节
//! digest      8B   LE
//! ```

use crate::canonical::{FrameReader, FrameWriter};
use crate::error::{SimError, SimResult};
use crate::fixed::Fixed;

/// 结构操作的类型字节（与批量帧的种类一一对应）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    /// `spawn_batch`。
    Spawn = 1,
    /// `despawn_batch`。
    Despawn = 2,
    /// `insert_batch`。
    Insert = 3,
}

impl OpKind {
    fn from_wire(byte: u8) -> SimResult<Self> {
        Ok(match byte {
            1 => OpKind::Spawn,
            2 => OpKind::Despawn,
            3 => OpKind::Insert,
            other => {
                return Err(SimError::frame(format!(
                    "日志里的操作类型字节未知：{other}（合法 1..=3）"
                )))
            }
        })
    }
}

/// 一条被录下的结构操作（原始批帧字节）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpRecord {
    /// 操作种类。
    pub kind: OpKind,
    /// 原始帧字节（回放时原样重投给同一个解码器）。
    pub frame: Vec<u8>,
}

/// 一个 tick 的完整输入与期望结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TickRecord {
    /// 该 tick 的序号。
    pub tick: u64,
    /// 步长。
    pub dt: Fixed,
    /// 录制时该 tick 结束的 state hash。
    pub hash: u64,
    /// 该 tick 开始前要应用的结构操作（按录制序）。
    pub ops: Vec<OpRecord>,
}

/// 输入日志（录制 / 编解码 / 回放共用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayLog {
    seed: u64,
    start_tick: u64,
    records: Vec<TickRecord>,
    /// 已录制但尚未归属到某个 tick 的操作（下一次 `push_tick` 收编）。
    pending: Vec<OpRecord>,
}

impl ReplayLog {
    /// 新日志（`start_tick = 0` 表示从创世开始）。
    pub fn new(seed: u64, start_tick: u64) -> Self {
        Self {
            seed,
            start_tick,
            records: Vec::new(),
            pending: Vec::new(),
        }
    }

    /// 种子。
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// 日志起点 tick（快照所在 tick；创世 = 0）。
    pub fn start_tick(&self) -> u64 {
        self.start_tick
    }

    /// 已录 tick 记录。
    pub fn records(&self) -> &[TickRecord] {
        &self.records
    }

    /// 可写记录面（宿主工具与负例用例要靠它篡改一帧来证明断言不是恒真）。
    pub fn records_mut(&mut self) -> &mut Vec<TickRecord> {
        &mut self.records
    }

    /// 截到前 `len` 条（step 失败回滚用：本 tick 没发生，就不该留下它的记录）。
    ///
    /// 只截 `records`，不碰 `pending`：待归属操作是本 tick **之前**投的（`spawn_batch` 类
    /// 宿主帧立即生效），它们与失败的那一 tick 无关。（`push_tick` 只在 step 成功后才调，
    /// 所以失败路径上不会把 `pending` 收编进 `records`。）
    pub(crate) fn truncate_records(&mut self, len: usize) {
        self.records.truncate(len);
    }

    /// 录一条结构操作（归属到下一个被录的 tick）。
    pub fn push_op(&mut self, kind: OpKind, frame: Vec<u8>) {
        self.pending.push(OpRecord { kind, frame });
    }

    /// 录一个 tick（把待归属操作收编进来）。
    pub fn push_tick(&mut self, tick: u64, dt: Fixed, hash: u64) {
        let ops = std::mem::take(&mut self.pending);
        self.records.push(TickRecord {
            tick,
            dt,
            hash,
            ops,
        });
    }

    /// 编码成帧（含尾 digest）。
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = FrameWriter::new();
        writer.u64(self.seed);
        writer.u64(self.start_tick);
        writer.u32(self.records.len() as u32);
        for record in &self.records {
            writer.u64(record.tick);
            writer.i64(record.dt);
            writer.u64(record.hash);
            writer.u32(record.ops.len() as u32);
            for op in &record.ops {
                writer.u8(op.kind as u8);
                // 帧长用 u32 前缀；`bytes()` 会自己写长度，这里要的是「长度 + 原字节」，
                // 语义相同故直接复用。
                let _ = writer.bytes(&op.frame);
            }
        }
        // 尚未归属的操作不允许静默丢弃（录制器 bug 会在回放时表现成「少应用一帧」）。
        debug_assert!(self.pending.is_empty(), "编码时仍有未归属的结构操作");
        writer.finish()
    }

    /// 解码（全量校验后返回；任何不符即 `YGG-SIM-FRAME`）。
    pub fn decode(bytes: &[u8]) -> SimResult<Self> {
        let mut reader = FrameReader::open(bytes)?;
        let seed = reader.u64()?;
        let start_tick = reader.u64()?;
        let n = reader.u32()? as usize;
        let mut records = Vec::with_capacity(n);
        for _ in 0..n {
            let tick = reader.u64()?;
            let dt = reader.i64()?;
            let hash = reader.u64()?;
            let op_n = reader.u32()? as usize;
            let mut ops = Vec::with_capacity(op_n);
            for _ in 0..op_n {
                let kind = OpKind::from_wire(reader.u8()?)?;
                let frame = reader.bytes()?.to_vec();
                ops.push(OpRecord { kind, frame });
            }
            records.push(TickRecord {
                tick,
                dt,
                hash,
                ops,
            });
        }
        reader.finish()?;
        // 逐 tick 单调递增（乱序日志会让「第一个失配」失去意义）
        for pair in records.windows(2) {
            if pair[0].tick >= pair[1].tick {
                return Err(SimError::frame(format!(
                    "日志的 tick 未严格递增：{} 之后是 {}",
                    pair[0].tick, pair[1].tick
                )));
            }
        }
        Ok(Self {
            seed,
            start_tick,
            records,
            pending: Vec::new(),
        })
    }
}
