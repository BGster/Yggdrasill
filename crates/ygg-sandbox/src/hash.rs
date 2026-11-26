//! S12 状态哈希（D-继承-5 归位：状态哈希归 ygg-sandbox）。
//!
//! - **哈希输入** = [`Loaded::snapshot`]（crate::modload）：世界内核的快照帧
//!   字节（`Sim::snapshot`）。M1-D 删掉行存后，帧字节就是
//!   canonical 的唯一序列化面：`magic / fmt_version / tick / next_entity /
//!   组件表（名字升序）/ resources（rng 流）/ archetype 段 / 尾 digest`，
//!   自带逐字节稳定性与往返幂等性，且**名字键**与实体 id 解耦。
//!   **不含 engine_version**（引擎版本不入帧，golden-vector 基线跨引擎补丁
//!   版本稳定；ENGINE_VERSION 门在存档编排层，不在内核里）。
//! - **与 Lua 构建细节完全解耦**（D-继承-5 铁律）：绝不从 Lua `pairs()`
//!   递归走查——`luai_makeseed` 构建覆盖（D-继承-3）与字符串哈希序是
//!   纵深防御面，不是本哈希的依赖面。
//! - **确定性链**（FR-9 native 侧证据链）：`KERNEL_RNG_SEED`（splitmix64，
//!   D-B5）+ `luai_makeseed` 常量覆盖（D-继承-3）+ 有序容器纪律
//!   （research §3 威胁 #3：确定性路径一律 `BTreeMap`）→ 同 Mod + 同外部
//!   事件序列 → 逐字节同 canonical bytes → 同 SHA-256（跨进程成立——
//!   `tests/probe_determinism.rs::probe_golden_vector` 实证）。
//! - **哈希覆盖面声明**：canonical 帧 = 世界面（实体 + 组件 + tick + 内核
//!   rng 流）；KV 不入哈希（其保真由宿主读回 /
//!   反复快照往返测试承载）——本函数是快照帧的 SHA-256 投影，不另立状态面。
//!   写断言时的口径：**「状态哈希不变」只能证明世界面未被改动**；要一并
//!   证明 KV 面也未变，必须经 [`Loaded::kv`] 读回 KV（或比较两次快照字节）。

use sha2::{Digest, Sha256};

use crate::modload::Loaded;
use crate::sandbox::StepError;

/// 状态哈希：`hex(SHA-256(世界帧字节))`（小写 64 位十六进制串；FR-9 native
/// 侧「同 Mod + 同步进序列 → 同哈希」的度量面）。
pub fn state_hash(loaded: &Loaded) -> Result<String, StepError> {
    let bytes = loaded.sandbox.snapshot()?;
    Ok(hex_sha256(&bytes))
}

/// SHA-256 摘要的小写十六进制编码（零依赖面：不引入 hex crate）。
///
/// 两个消费者：状态哈希（[`state_hash`]）与 `ygg.codec.sha256` 原生键
/// （`codec_face/digest.rs`，S19 起）——crate 内单一 hex 源。
pub(crate) fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
