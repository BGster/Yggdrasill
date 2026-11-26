//! FNV-1a 64：Tier 1 state hash 的唯一实现。
//!
//! 契约：sim-kernel §2.5（`state_hash` 的规范序与算法）+ §4（`hash()` 跨面形状）。
//!
//! # 为什么是 FNV-1a 64 而不是 SHA-256
//!
//! 三个理由，按分量排序：
//! 1. **它不是安全面**。state hash 用于「同 seed 同输入 ⇒ 同字节」的等价判定与回放对账，
//!    不用于防伪。密码学性质在这里是纯成本（SHA-256 在 0.7 MB/帧上比 FNV 慢一个数量级，
//!    而 §5 的快照预算只有 100 ms）。
//! 2. **一个算法两个消费者不再必要**。v1 里没有人另算一份 hash：原生与 wasm 共用本实现，
//!    等价判定只比较内核给出的 64 位值。若将来需要 Lua 侧算同一个 hash，FNV-1a 只需十行
//!    常量与循环，比 SHA-256 容易对齐——这是选择它的前瞻理由，不是当前依赖。
//! 3. **十行可审**。常量 `0xCBF29CE484222325` / `0x100000001B3` 与 `h ^= b; h *= prime`
//!    的循环没有可争议的写法。
//!
//! # 与 Tier 2 state hash 的分工
//!
//! `crates/ygg-sandbox/src/hash.rs` 的 SHA-256 覆盖 **Tier 2 实体面**（canonical JSON）；
//! 本模块覆盖 **Tier 1 模拟态**（快照列帧）。两者**不互算、不互认**：同一次运行里它们是
//! 两个不同对象的摘要，各自有独立用例。把任何一个换成另一个都会让「同一份数据的两个
//! 摘要」变成一条要人背的规则。

/// FNV-1a 64（逐字节；`h0` 与质数都是十六进制字面量，乘法按 2^64 回绕）。
///
/// **不用 32 位变体**：32 位与 64 位的 FNV 取值不同，混用会让快照字节与 hash 对不上。
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xCBF2_9CE4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01B3;
    let mut hash = OFFSET;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// 快照帧 → state hash：**帧尾 8 字节小端就是它**（所以 restore 的校验与 hash 同一来源）。
///
/// 输入必须是**完整帧**（含尾 digest）。返回的值等于 `fnv1a64(帧去尾 digest)`：
/// 快照写出时先把 digest 追加到末尾，读回时再从末尾取——两次计算同一个数，
/// 于是一条等式统摄快照与哈希（`state_hash == fnv1a64(snapshot_payload)`）。
pub fn state_hash(snapshot: &[u8]) -> u64 {
    if snapshot.len() < 8 {
        return fnv1a64(snapshot);
    }
    fnv1a64(&snapshot[..snapshot.len() - 8])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 公开参考向量（FNV-1a 64 的标准测试串）——不是自算的派生值。
    #[test]
    fn fnv1a64_matches_published_vectors() {
        assert_eq!(fnv1a64(b""), 0xCBF2_9CE4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xAF63_DC4C_8601_EC8C);
        assert_eq!(fnv1a64(b"foobar"), 0x85944171F73967E8);
        // 逐字节累积：长度不同即 hash 不同（防「把整帧当字符串处理」的等价实现）
        assert_ne!(fnv1a64(b"ab"), fnv1a64(b"ba"));
    }

    #[test]
    fn state_hash_is_the_trailing_digest() {
        let payload = b"YGGSIM-payload";
        let mut frame = payload.to_vec();
        frame.extend_from_slice(&fnv1a64(payload).to_le_bytes());
        assert_eq!(state_hash(&frame), fnv1a64(payload));
        // 帧太短时不 panic（只有调用方跳过 canonical 校验才可能走到这里）
        assert_eq!(state_hash(b"abc"), fnv1a64(b"abc"));
    }
}
