//! 位串搬运面：IEEE 754 double 的位级归一与全序键（sim-kernel §3 I2 的唯一豁免文件）。
//!
//! 契约：sim-kernel.md §3 I2「定点政策与位串承载」——位串列（`AttrType::F64Bits`）承载的
//! double **仅搬运不算术**：内核对位串只做存储、拷贝、规范化（写入边界）与字节级 hash。
//! 本文件因此是全 crate 唯一允许出现 `f64` 类型词的地方（`tests/determinism_scan.rs`
//! 的 R1′ 白名单），并被断言**剥注释后不含 `+` `-` `*` `/` `%` 五个算术运算符字符**：
//! 浮点环境差异（FTZ/DAZ、x87 扩展精度中间位）在结构上不可达，确定性由整数运算与
//! 位搬运保证。位运算（`&` `|` `^` `!` `>>` `<<`）与比较不受限——它们不是浮点运算。
//!
//! # 归一规则（写入边界执行，读边界断言）
//!
//! - **任意 NaN 载荷 → 单一 quiet NaN**（[`NAN_BITS`]）。qNaN 载荷是平台/指令来源的
//!   副产物（x86 SSE 与 arm64 NEON 的隐式 NaN 不同），归一才保「同状态同 hash」。
//!   sNaN 经 `to_bits` 原样成字，同样归一到这里。
//! - **±0 → +0**（位串 `0`）。Lua 的 `0.0 == (-0.0)` 为真，codec 文本面本来就单形；
//!   不归一会出现「同值不同 hash」。
//! - **±Inf 是普通位型放行**：内核无算术，无平台风险面。
//!
//! 其余位型（含次正规）原样保留——列内存即 IEEE 754 原始位串，调试直读、
//! hash 字节即位串、每值零变换。

/// 归一化 NaN 位串（任意 NaN 载荷的归一终点）。
pub const NAN_BITS: u64 = 0x7FF8_0000_0000_0000;

/// 指数域掩码。
const EXP_MASK: u64 = 0x7FF0_0000_0000_0000;
/// 尾数域掩码。
const MANT_MASK: u64 = 0x000F_FFFF_FFFF_FFFF;
/// 符号位掩码。
const SIGN_MASK: u64 = 0x8000_0000_0000_0000;

/// `f64` 值 → 归一位串（写入边界的唯一入口）。
///
/// NaN 载荷归一到 [`NAN_BITS`]，±0 归一到 `0`，其余位型原样。输出恒为 canonical
/// 位串（[`is_canonical`] 恒真）——这是 [`pack`] 分支结构的数学事实，不需要运行时断言。
pub fn pack(value: f64) -> u64 {
    let bits = value.to_bits();
    let exp = bits & EXP_MASK;
    let mant = bits & MANT_MASK;
    if exp == EXP_MASK && mant != 0 {
        NAN_BITS
    } else if exp == 0 && mant == 0 {
        0
    } else {
        bits
    }
}

/// 归一位串 → `f64` 值（读边界的唯一出口）。
///
/// 对非 canonical 位串的取值无定义（位串列的读路径各自先做 [`is_canonical`] 校验，
/// 不过即 `YGG-SIM-FRAME`；本函数不做双重检查——搬运面上多一次分支就是多一次要审的语义）。
pub fn unpack(bits: u64) -> f64 {
    f64::from_bits(bits)
}

/// 位串是否为 canonical 形（读边界的 fail-closed 判据，不过即 `YGG-SIM-FRAME`）。
///
/// canonical = [`pack`] 的像：`0`（归一 +0）、NaN 位置的 [`NAN_BITS`]、
/// 其余位型原样（±Inf、次正规、普通数，含一切非 NaN 非 ±0 位型）。
pub fn is_canonical(bits: u64) -> bool {
    let exp = bits & EXP_MASK;
    let mant = bits & MANT_MASK;
    if exp == 0 && mant == 0 {
        bits == 0
    } else if exp == EXP_MASK && mant != 0 {
        bits == NAN_BITS
    } else {
        true
    }
}

/// 位串 → 全序键：`order_key` 升序 ⟺ IEEE 754 `totalOrder` 升序。
///
/// 证明要点：①非负区间 IEEE 位型按（偏置指数, 尾数）高位到低位排布，无符号序 =
/// 数值序；②负区间无符号序与数值序相反，按位取反翻转；③接缝 −0 对应 `0x7FFF…F`、
/// +0 对应 `0x8000…0`，两键相邻；④canonical NaN 居 +Inf 之上（`totalOrder`）。
/// ⇒ 双射且单调。输入约定为 canonical 位串（±0 已在写入边界归一）。
pub fn order_key(bits: u64) -> u64 {
    if bits & SIGN_MASK != 0 {
        !bits
    } else {
        bits | SIGN_MASK
    }
}

#[cfg(test)]
mod tests {
    use super::{is_canonical, order_key, pack, unpack, EXP_MASK, MANT_MASK, NAN_BITS, SIGN_MASK};

    /// 表驱动归一向量：值一律用 `f64::from_bits` 构造（本文件禁浮点字面量，
    /// 常量一律 u64 hex——扫描 R1′ 钉死）。
    #[test]
    fn pack_normalizes_nan_and_signed_zero() {
        let cases: &[(u64, u64)] = &[
            // +0 与 -0 都归一到 0
            (0x0000_0000_0000_0000, 0),
            (0x8000_0000_0000_0000, 0),
            // 普通数原样（正、负、小数位型）
            (0x3FF0_0000_0000_0000, 0x3FF0_0000_0000_0000),
            (0xBFF0_0000_0000_0000, 0xBFF0_0000_0000_0000),
            (0xC008_0000_0000_0000, 0xC008_0000_0000_0000),
            // ±Inf 原样放行
            (0x7FF0_0000_0000_0000, 0x7FF0_0000_0000_0000),
            (0xFFF0_0000_0000_0000, 0xFFF0_0000_0000_0000),
            // 一切 NaN 载荷归一到 NAN_BITS（qNaN、sNaN、带载荷、负号）
            (0x7FF8_0000_0000_0000, NAN_BITS),
            (0x7FF8_0000_0000_0001, NAN_BITS),
            (0x7FF0_0000_0000_0001, NAN_BITS),
            (0xFFF8_0000_0000_0001, NAN_BITS),
            // 次正规原样
            (0x0000_0000_0000_0001, 0x0000_0000_0000_0001),
            (0x000F_FFFF_FFFF_FFFF, 0x000F_FFFF_FFFF_FFFF),
        ];
        for &(raw, want) in cases {
            assert_eq!(pack(f64::from_bits(raw)), want, "pack(0x{raw:016X})");
        }
    }

    /// pack 的输出恒 canonical；unpack 往返恒等于原位串。
    #[test]
    fn pack_output_is_always_canonical_and_round_trips() {
        let probes: &[u64] = &[
            0,
            0x8000_0000_0000_0000,
            NAN_BITS,
            0x7FF8_DEAD_BEEF_0001,
            0x7FF0_0000_0000_0001,
            0x3FF0_0000_0000_0000,
            0xBFF0_0000_0000_0000,
            0x7FF0_0000_0000_0000,
            0xFFF0_0000_0000_0000,
            0x0000_0000_0000_0001,
        ];
        for bits in probes.iter().copied() {
            let packed = pack(f64::from_bits(bits));
            assert!(is_canonical(packed), "pack(0x{bits:016X}) 非 canonical");
            // unpack 是位串恒等出口：往返后位串必须回到原位（不能比较 f64 值——
            // NaN 的 PartialEq 恒为 false，那是 IEEE 语义不是失败）。
            assert_eq!(unpack(packed).to_bits(), packed);
            if is_canonical(bits) {
                assert_eq!(packed, bits, "canonical 位串必须原样往返");
            }
        }
    }

    /// 读边界 fail-closed 判据：只拒 -0 与非规范 NaN 载荷。
    #[test]
    fn is_canonical_rejects_signed_zero_and_payload_nan() {
        let rejected: &[u64] = &[
            0x8000_0000_0000_0000, // -0
            0x7FF0_0000_0000_0001, // sNaN
            0x7FF8_0000_0000_0042, // 带载荷 qNaN
            0xFFF8_0000_0000_0000, // 负 qNaN
            0xFFF8_0000_DEAD_BEEF, // 负带载荷 NaN
        ];
        for &bits in rejected {
            assert!(!is_canonical(bits), "0x{bits:016X} 应判非 canonical");
        }
        let accepted: &[u64] = &[
            0,                     // +0
            NAN_BITS,              // 规范 NaN
            0x7FF0_0000_0000_0000, // +Inf
            0xFFF0_0000_0000_0000, // -Inf
            0x0000_0000_0000_0001, // 次正规
            0x000F_FFFF_FFFF_FFFF, // 最大次正规
            0x3FF0_0000_0000_0000, // 1.0
            0xBFF0_0000_0000_0000, // -1.0
            0x7FEF_FFFF_FFFF_FFFF, // 最大有限
            0xFFEF_FFFF_FFFF_FFFF, // 最小有限
        ];
        for &bits in accepted {
            assert!(is_canonical(bits), "0x{bits:016X} 应判 canonical");
        }
    }

    /// 全序：order_key 在采样集上与 `f64::total_cmp` 同序（单调），且双射。
    #[test]
    fn order_key_agrees_with_total_cmp_and_is_bijective() {
        let samples: &[u64] = &[
            0x0000_0000_0000_0000, // +0
            0x0000_0000_0000_0001, // 最小次正规
            0x36A0_0000_0000_0000, // 深次正规区的小数
            0x3FF0_0000_0000_0000, // 1.0
            0x4000_0000_0000_0000, // 2.0
            0x7FEF_FFFF_FFFF_FFFF, // 最大有限
            0x7FF0_0000_0000_0000, // +Inf
            NAN_BITS,              // canonical NaN（居 +Inf 之上）
            0xBFF0_0000_0000_0000, // -1.0
            0xC000_0000_0000_0000, // -2.0
            0xFFEF_FFFF_FFFF_FFFF, // 最小有限
            0xFFF0_0000_0000_0000, // -Inf
        ];
        for &a in samples {
            assert!(is_canonical(a), "采样必须 canonical：0x{a:016X}");
            for &b in samples {
                let key_order = order_key(a).cmp(&order_key(b));
                let value_order = f64::from_bits(a).total_cmp(&f64::from_bits(b));
                assert_eq!(key_order, value_order, "0x{a:016X} vs 0x{b:016X}");
                if a != b {
                    assert_ne!(order_key(a), order_key(b), "双射被破坏");
                }
            }
        }
        // 接缝：位型序里的相邻对，键也相邻（正区键随位型递增；负区按位取反反向，
        // 用「清末位」表达相邻，避免引入减法）。
        assert_eq!(order_key(0) | 1, order_key(0x0000_0000_0000_0001));
        assert_eq!(
            order_key(0xBFF0_0000_0000_0001),
            order_key(0xBFF0_0000_0000_0000) & !1
        );
    }

    /// 掩码自检：三域不重叠、并起来铺满全字。
    #[test]
    fn masks_partition_the_word() {
        assert_eq!(EXP_MASK | MANT_MASK | SIGN_MASK, 0xFFFF_FFFF_FFFF_FFFF);
        assert_eq!(EXP_MASK & MANT_MASK, 0);
        assert_eq!(EXP_MASK & SIGN_MASK, 0);
        assert_eq!(MANT_MASK & SIGN_MASK, 0);
    }
}
