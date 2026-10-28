//! S12 状态哈希（D-继承-5 归位：状态哈希归 ygg-sandbox）。
//!
//! - **哈希输入** = [`KernelFacade::canonical_state_bytes`]（crate::driver）：
//!   `ygg_core::snapshot::canonical_snapshot` 的排序键 canonical JSON——
//!   覆盖 `schema_version / revision / next_entity_id / entities`（实体 ID
//!   升序、属性键升序），**不含 engine_version**（引擎版本不入快照字节，
//!   golden-vector 基线跨引擎补丁版本稳定；ENGINE_VERSION 门在存档面，
//!   见 `driver::KernelFacade::restore`）。
//! - **与 Lua 构建细节完全解耦**（D-继承-5 铁律）：绝不从 Lua `pairs()`
//!   递归走查——`luai_makeseed` 构建覆盖（D-继承-3）与字符串哈希序是
//!   纵深防御面，不是本哈希的依赖面。
//! - **确定性链**（FR-9 native 侧证据链）：`KERNEL_RNG_SEED`（splitmix64，
//!   D-B5）+ `luai_makeseed` 常量覆盖（D-继承-3）+ 有序容器纪律
//!   （research §3 威胁 #3：确定性路径一律 `BTreeMap`）→ 同 Mod + 同外部
//!   事件序列 → 逐字节同 canonical bytes → 同 SHA-256（跨进程成立——
//!   `tests/probe_determinism.rs::probe_golden_vector` 实证）。
//! - **哈希覆盖面声明**：canonical 快照 = 实体表面（core `canonical_snapshot`
//!   既定面）；KV / pending_choice 不入哈希（其保真由存读档往返测试承载，
//!   S5/S10）——本函数是 canonical_state_bytes 的 SHA-256 投影，不另立状态面。

use sha2::{Digest, Sha256};

use ygg_core::snapshot::SnapshotError;

use crate::driver::KernelFacade;

/// 状态哈希：`hex(SHA-256(canonical_state_bytes))`（小写 64 位十六进制串；
/// FR-9 native 侧「同 Mod + 同事件序列 → 同哈希」的度量面）。
pub fn state_hash(facade: &KernelFacade) -> Result<String, SnapshotError> {
    let bytes = facade.canonical_state_bytes()?;
    Ok(hex_sha256(&bytes))
}

/// SHA-256 摘要的小写十六进制编码（零依赖面：不引入 hex crate）。
fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
