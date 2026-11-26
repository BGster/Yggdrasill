//! S12 状态哈希（D-继承-5 归位：状态哈希归 ygg-sandbox）。
//!
//! - **哈希输入** = [`KernelFacade::canonical_state_bytes`]（crate::driver）：
//!   世界内核的快照帧字节（`Sim::snapshot`）。M1-D 删掉行存后，帧字节就是
//!   canonical 的唯一序列化面：`magic / fmt_version / tick / next_entity /
//!   组件表（名字升序）/ resources（rng 流）/ archetype 段 / 尾 digest`，
//!   自带逐字节稳定性与往返幂等性，且**名字键**与实体 id 解耦。
//!   **不含 engine_version**（引擎版本不入帧，golden-vector 基线跨引擎补丁
//!   版本稳定；ENGINE_VERSION 门在存档面，见 `driver::KernelFacade::restore`）。
//! - **与 Lua 构建细节完全解耦**（D-继承-5 铁律）：绝不从 Lua `pairs()`
//!   递归走查——`luai_makeseed` 构建覆盖（D-继承-3）与字符串哈希序是
//!   纵深防御面，不是本哈希的依赖面。
//! - **确定性链**（FR-9 native 侧证据链）：`KERNEL_RNG_SEED`（splitmix64，
//!   D-B5）+ `luai_makeseed` 常量覆盖（D-继承-3）+ 有序容器纪律
//!   （research §3 威胁 #3：确定性路径一律 `BTreeMap`）→ 同 Mod + 同外部
//!   事件序列 → 逐字节同 canonical bytes → 同 SHA-256（跨进程成立——
//!   `tests/probe_determinism.rs::probe_golden_vector` 实证）。
//! - **哈希覆盖面声明**：canonical 帧 = 世界面（实体 + 组件 + tick + 内核
//!   rng 流）；KV / pending_choice 不入哈希（其保真由存读档往返测试承载）——
//!   本函数是 canonical_state_bytes 的 SHA-256 投影，不另立状态面。
//!   写断言时的口径：**「状态哈希不变」只能证明世界面未被改动**；要一并
//!   证明 KV 面也未变，必须读 `save` 回执（驱动 `dispatch("save","")` 的
//!   `save.kv.entries`）或比较 save 字节——dispatch 级 savepoint 的回滚回归
//!   正靠这对证据（`tests/kernel_facade/rollback.rs::failed_dispatch_leaves_`
//!   `state_hash_and_save_bytes_unchanged`）。

use sha2::{Digest, Sha256};

use crate::driver::KernelFacade;
use crate::sandbox::StepError;

/// 状态哈希：`hex(SHA-256(canonical_state_bytes))`（小写 64 位十六进制串；
/// FR-9 native 侧「同 Mod + 同事件序列 → 同哈希」的度量面）。
pub fn state_hash(facade: &KernelFacade) -> Result<String, StepError> {
    let bytes = facade.canonical_state_bytes()?;
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
