//! 随机数：seed 流派生 + xoshiro128\*\*——与 `packages/libs/procgen/rng.lua` **位级同源**。
//!
//! 契约：sim-kernel I5（模拟态不读外部随机源，随机数只由 seed 流派生）+ §3.3。
//!
//! # 为什么是「镜像」而不是「另一份实现」
//!
//! `pkg.procgen.rng` 已经是本仓 PRNG 的唯一契约（assetgen 明令不得复制常量）。Tier 1
//! 侧必须给出同样的数，否则「Lua 侧生成的种子 → Rust 侧复算」会分叉。所以本模块的规格
//! 逐字取自那个文件：
//!
//! - `fnv64(text)`：FNV-1a 64（复用 [`crate::hash::fnv1a64`]，同一个十行实现）。
//! - `mix64(z)`：splitmix64 终结器，常量 `0xBF58476D1CE4E5B9` / `0x94D049BB133111EB`。
//! - `derive_state(seed, stream)`：`w[i] = mix64(fnv64("{seed}/{stream}/{i}")) & 0xFFFFFFFF`，
//!   `i = 0..3`；四字全 0（xoshiro 退化态）⇒ `w[0] = 0x9E3779B9`（黄金比例常数）。
//! - `next32`：xoshiro128\*\*，`result = rotl32(mul32(s[1], 5), 7) * 9`——**用第二个字**
//!   （`s[1]`），不是 `s[0]`；这一步与参考 C 实现一致，KAT 见 `tests/rng_golden.rs`。
//!
//! 同源证据不是声明而是用例：`crates/ygg-sim/fixtures/fnv64-vectors.json` 由**第三实现**
//! （Python，独立于 Rust 与 Lua）生成，Rust 侧 `rng_golden` 与 Lua 侧
//! `pkg-procgen-digest.lua` 各断言一次——任一侧漂移即红。
//!
//! # 状态随快照往返（I1）
//!
//! [`RngRegistry`] 持有每条流的四字状态与 seed，快照里有一段 `rng` 记录它们。理由：
//! 「存档后续跑」要求随机序列**接着往下走**而不是从头重播——否则同一个世界在存档前后
//! 会给出不同的随机数，而这类错误在短用例里看不出来。

use std::collections::BTreeMap;

use crate::error::SimResult;

const M32: u32 = 0xFFFF_FFFF;
const MIX_A: u64 = 0xBF58_476D_1CE4_E5B9;
const MIX_B: u64 = 0x94D0_49BB_1331_11EB;
const GOLDEN: u32 = 0x9E37_79B9;

/// 32 位循环左移（`k` 取 1..31）。
fn rotl32(value: u32, k: u32) -> u32 {
    value.rotate_left(k) & M32
}

/// FNV-1a 64（文本口径；与 `pkg.procgen.rng.fnv64` 同源）。
pub fn fnv64(text: &str) -> u64 {
    crate::hash::fnv1a64(text.as_bytes())
}

/// splitmix64 终结器（雪崩）。
pub fn mix64(z: u64) -> u64 {
    let mut z = z;
    z ^= z >> 30;
    z = z.wrapping_mul(MIX_A);
    z ^= z >> 27;
    z = z.wrapping_mul(MIX_B);
    z ^= z >> 31;
    z
}

/// `(seed 文本, 流名)` → 四字状态（纯函数：同参数永远同结果）。
pub fn derive_state(seed_text: &str, stream: &str) -> [u32; 4] {
    let mut words = [0u32; 4];
    for (index, slot) in words.iter_mut().enumerate() {
        let text = format!("{seed_text}/{stream}/{index}");
        *slot = (mix64(fnv64(&text)) & u64::from(M32)) as u32;
    }
    if words.iter().all(|word| *word == 0) {
        words[0] = GOLDEN;
    }
    words
}

/// xoshiro128\*\* 的一步（**原地**推进四字状态）。
pub fn next32(state: &mut [u32; 4]) -> u32 {
    let result = rotl32(state[1].wrapping_mul(5), 7).wrapping_mul(9);
    let t = (state[1] << 9) & M32;
    state[2] ^= state[0];
    state[3] ^= state[1];
    state[1] ^= state[2];
    state[0] ^= state[3];
    state[2] ^= t;
    state[3] = rotl32(state[3], 11);
    result
}

/// 整数 seed → 文本（与 `pkg.procgen` 的整数 seed 口径一致：十进制、无前导零）。
pub fn seed_text(seed: u64) -> String {
    seed.to_string()
}

/// 流的命名约定（**单点**在这里，别在调用点拼字符串）。
///
/// `sim/<system>/<channel>`：`channel` 从 0 起，一个系统可以有多个独立通道（例如「位置抖动」
/// 与「暴击判定」各一条），这样新增一条通道不会移动既有通道的序列。
pub fn stream_name(system: &str, channel: u32) -> String {
    format!("sim/{system}/{channel}")
}

/// 流状态表：`seed` + 每条流一份四字状态（**随快照往返**）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RngRegistry {
    seed: u64,
    /// 流名 → 四字状态（`BTreeMap`：快照按名称升序写出，与注册/使用顺序无关）。
    streams: BTreeMap<String, [u32; 4]>,
}

impl RngRegistry {
    /// 新建（`seed` 是**唯一**随机源入口；I5）。
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            streams: BTreeMap::new(),
        }
    }

    /// 原始 seed。
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// 已派生（被抽过）的流数。
    pub fn stream_count(&self) -> usize {
        self.streams.len()
    }

    /// 某条流的当前状态（未派生 ⇒ `None`）。
    pub fn state_of(&self, stream: &str) -> Option<[u32; 4]> {
        self.streams.get(stream).copied()
    }

    /// 全部流（名称升序；快照写出用）。
    pub fn entries(&self) -> impl Iterator<Item = (&str, [u32; 4])> + '_ {
        self.streams
            .iter()
            .map(|(name, state)| (name.as_str(), *state))
    }

    /// 取一个 32 位随机数（首次使用该流时按 `(seed, stream)` 派生）。
    pub fn next32(&mut self, stream: &str) -> u32 {
        let state = self
            .streams
            .entry(stream.to_string())
            .or_insert_with(|| derive_state(&seed_text(self.seed), stream));
        next32(state)
    }

    /// 从快照段重建（seed + 流状态；状态表保持名称升序）。
    pub fn from_parts(seed: u64, streams: BTreeMap<String, [u32; 4]>) -> SimResult<Self> {
        for (name, state) in &streams {
            if state.iter().all(|word| *word == 0) {
                return Err(crate::error::SimError::frame(format!(
                    "快照里的流 `{name}` 四字全 0（xoshiro 退化态）"
                )));
            }
        }
        Ok(Self { seed, streams })
    }
}
