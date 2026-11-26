//! Q32.32 定点（`i64` 载体）——模拟态里唯一的数值类型。
//!
//! 契约：sim-kernel §3 I2「模拟数值默认 i64 定点（Q32.32 或按面声明）；浮点禁入模拟态」。
//!
//! # 为什么是 i64 与 32 位小数位
//!
//! 32 位小数位给出 ±2³¹ 的整数域与 ~2.3e-10 的分辨率，足以覆盖坐标 / 速度 / 血量；
//! `i64` 的加减是单指令，溢出可以用 `saturating_*` 定义成**确定行为**。
//!
//! # 四条运算语义（逐条钉死，别改）
//!
//! - [`add`] / [`sub`]：**饱和**。不 panic（panic 的行为取决于 debug assertion：release
//!   里静默 wrap、debug 里中止，等于同一表达式有两套行为），也不 wrap（会符号翻转）。
//! - [`mul`]：`(a as i128 * b as i128) >> 32`，右移对负数为**向 −∞ 取整**（Rust 的有符号
//!   算术右移语义确定），结果 clamp 到 `i64` 域。
//! - [`div`]：`((a as i128) << 32) / b`，向零截断；`b == 0` 报 [`SimError::fixed`]。
//!   **不定义除零的近似值**：给 `i64::MAX` 会把除零藏成数值异常，而静默错答比报错贵得多。
//! - **禁浮点中间量**：`as f64` 在本 crate 的静态扫描里属 R1 命中。这条纪律与「注释里
//!   可以解释 f64」不冲突（扫描先剥注释，见 `lib.rs` 头注）。
//!
//! # `frac_bits` 不在本模块里
//!
//! 内核**不解释小数位**：它只存 `i64`。组件 schema 里的 `frac_bits` 是元数据，供 Lua 面
//! 换算助手与将来的面声明使用——今天是**声明面**，不是运算面。

use crate::error::{SimError, SimResult};

/// 定点载体：Q32.32 的 `i64`。
pub type Fixed = i64;

/// 默认小数位（Q32.32）。
pub const FRAC_BITS: u8 = 32;

/// 1.0 的表示。
pub const ONE: Fixed = 1 << FRAC_BITS;

/// 小数位掩码（取 `a` 的小数部分）。
pub const FRAC_MASK: Fixed = ONE - 1;

/// 整数 → 定点（饱和：`i64::MAX >> 32` 已在域外）。
pub fn from_int(value: i64) -> Fixed {
    value.saturating_mul(ONE)
}

/// 定点 → 整数（向 −∞ 取整；小数部分被丢弃，不做四舍五入——舍入会引入第二套语义）。
pub fn to_int(value: Fixed) -> i64 {
    value >> FRAC_BITS
}

/// 饱和加法。
pub fn add(a: Fixed, b: Fixed) -> Fixed {
    a.saturating_add(b)
}

/// 饱和减法。
pub fn sub(a: Fixed, b: Fixed) -> Fixed {
    a.saturating_sub(b)
}

/// 乘法：`(a * b) >> 32`，经 `i128` 中间量后 clamp。
///
/// `i128` 不是「更宽所以更慢」的妥协：`i64 × i64` 的完整积本来就是 128 位，截断到 64 位
/// 再做移位会丢精度；`i128` 乘法在 x86_64/aarch64 上都是一条 `mul` 加一条 `imul` 的高半字，
/// 没有任何分支或平台差异。
pub fn mul(a: Fixed, b: Fixed) -> Fixed {
    let wide = (a as i128) * (b as i128);
    clamp_i64(wide >> FRAC_BITS)
}

/// 除法：`(a << 32) / b`，向零截断；除零报错。
pub fn div(a: Fixed, b: Fixed) -> SimResult<Fixed> {
    if b == 0 {
        return Err(SimError::fixed("div-by-zero"));
    }
    let wide = (a as i128) << FRAC_BITS;
    Ok(clamp_i64(wide / (b as i128)))
}

/// 整数平方根（floor）：`isqrt(x)` 是满足 `r*r ≤ x < (r+1)*(r+1)` 的 `r`。
///
/// **为什么需要它**：规则代码里的距离判定（`dx*dx + dy*dy ≤ range*range`）要开方，
/// 而 [`ONE`] 以下的根用 `mul` 是做不到的；绕过它的唯一办法是浮点 `sqrt`，那正是
/// 本 crate 静态扫描（R1）禁掉的东西。
///
/// 输入输出都是 Q32.32：`r_raw = floor(sqrt(v_raw · 2^32))`（推导：真值 `x = v_raw / 2^32`，
/// 结果 `r_raw = sqrt(x) · 2^32 = sqrt(v_raw · 2^32)`）。`v_raw ≤ i64::MAX < 2^63` ⇒
/// `v_raw · 2^32 < 2^95` ⇒ `r_raw < 2^48`，必落在 `i64` 域内，故无需饱和。
///
/// 负值拒绝（报 [`SimError::fixed`]）：`sqrt(-1)` 不是「接近 0 的数」，给它一个近似值
/// 等于把调用方的符号错误藏成数值异常——与 [`div`] 的除零处置同一条纪律。
pub fn isqrt(value: Fixed) -> SimResult<Fixed> {
    if value < 0 {
        return Err(SimError::fixed(format!(
            "isqrt: 负值没有实数平方根（得到 {value}）"
        )));
    }
    Ok(isqrt_u128((value as u128) << FRAC_BITS) as i64)
}

/// 非负 `u128` 的整数平方根（位对法：逐两位定一位，只用移位/比较/减法）。
///
/// 不用浮点、不用 `f64` 近似、无除法迭代的收敛判定——每一位的结论都是精确比较，
/// 所以 native 与 wasm 逐位一致（确定性优先于速度：一个 tick 里调用次数是个位数）。
fn isqrt_u128(value: u128) -> u128 {
    let mut remainder = value;
    let mut root: u128 = 0;
    // 最高偶数位：2^126（`u128` 域内最大的平方数位）。
    let mut bit: u128 = 1 << 126;
    while bit > value {
        bit >>= 2;
    }
    while bit != 0 {
        if remainder >= root + bit {
            remainder -= root + bit;
            root = (root >> 1) + bit;
        } else {
            root >>= 1;
        }
        bit >>= 2;
    }
    root
}

/// `i128` → `i64`，越界即饱和。
fn clamp_i64(value: i128) -> i64 {
    if value > i64::MAX as i128 {
        i64::MAX
    } else if value < i64::MIN as i128 {
        i64::MIN
    } else {
        value as i64
    }
}

/// 定点 → 十进制文本（9 位小数截断，无四舍五入、无 locale、无浮点）。
///
/// 只用于诊断与用例可读性（错误消息、基准打印）；**不是线格式**——快照里数值是
/// 定宽 LE 二进制，文本形态不进任何契约。
pub fn to_decimal(value: Fixed) -> String {
    let negative = value < 0;
    // 取绝对值走 i128：`i64::MIN` 的绝对值在 i64 里不可表（饱和会把它变成 MAX，
    // 打印出错误的符号量级）。
    let magnitude = (value as i128).unsigned_abs();
    let int_part = magnitude >> FRAC_BITS;
    let frac_part = magnitude & (FRAC_MASK as u128);
    // 9 位小数 = frac * 10^9 / 2^32（向下取整，确定）。
    let scaled = frac_part * 1_000_000_000u128 / (1u128 << FRAC_BITS);
    let sign = if negative { "-" } else { "" };
    format!("{sign}{int_part}.{scaled:09}")
}

/// 十进制文本 → 定点。接受 `[-]int[.frac]`，`frac` 最多 9 位；超出即报错（不静默截断）。
///
/// 与 [`to_decimal`] 配对：`parse_decimal(&to_decimal(v))` 在 9 位小数可精确表示的范围内
/// 往返一致（`2^-32` 的十进制展开有 32 位，故 9 位是**有损**的，往返只在 `v` 的小数部分
/// 本身就是 10^-9 的整数倍时严格成立——用例只断言该范围内的一致性）。
pub fn parse_decimal(text: &str) -> SimResult<Fixed> {
    let (negative, body) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let (int_text, frac_text) = match body.split_once('.') {
        Some((int_text, frac_text)) => (int_text, frac_text),
        None => (body, ""),
    };
    if int_text.is_empty() && frac_text.is_empty() {
        return Err(SimError::fixed(format!("parse-decimal: 空数：{text}")));
    }
    if frac_text.len() > 9 {
        return Err(SimError::fixed(format!(
            "parse-decimal: 小数位超过 9 位（会被静默截断）：{text}"
        )));
    }
    let mut int_value: i128 = 0;
    for byte in int_text.bytes() {
        if !byte.is_ascii_digit() {
            return Err(SimError::fixed(format!("parse-decimal: 非数字：{text}")));
        }
        int_value = int_value * 10 + i128::from(byte - b'0');
        if int_value > (i64::MAX as i128) >> FRAC_BITS {
            return Err(SimError::fixed(format!("parse-decimal: 整数溢出：{text}")));
        }
    }
    let mut frac_value: i128 = 0;
    for byte in frac_text.bytes() {
        if !byte.is_ascii_digit() {
            return Err(SimError::fixed(format!("parse-decimal: 非数字：{text}")));
        }
        frac_value = frac_value * 10 + i128::from(byte - b'0');
    }
    // 小数部分按 10^-len 展开到 Q32.32：frac * 2^32 / 10^len（向下取整，与 to_decimal 同向）。
    let mut denom: i128 = 1;
    for _ in 0..frac_text.len() {
        denom *= 10;
    }
    let frac_fixed = if frac_text.is_empty() {
        0
    } else {
        (frac_value << FRAC_BITS) / denom
    };
    let total = (int_value << FRAC_BITS) + frac_fixed;
    let total = if negative { -total } else { total };
    Ok(clamp_i64(total))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::codes;

    #[test]
    fn isqrt_exact_squares_round_trip() {
        assert_eq!(isqrt(0), Ok(0));
        assert_eq!(isqrt(ONE), Ok(ONE));
        assert_eq!(isqrt(4 * ONE), Ok(2 * ONE));
        assert_eq!(isqrt(9 * ONE), Ok(3 * ONE));
        // 非整数平方：floor 到下一个可表示的 Q32.32 根（2.25 → 1.5 精确）。
        let nine_quarters = 9 * (ONE / 4);
        assert_eq!(isqrt(nine_quarters), Ok(ONE + ONE / 2));
    }

    #[test]
    fn isqrt_floor_semantics_hold() {
        // `ONE - 1`（1.0 以下的最近值）的根仍是 0.999…，Q32.32 的 floor 是
        // `ONE - 1`；`to_int` 才把它读成整数 0——两个断言一起钉住「floor 在哪一级」。
        let below_one = ONE - 1;
        let root = isqrt(below_one).expect("non-negative");
        assert_eq!(root, ONE - 1);
        assert_eq!(to_int(root), 0);
        // 不变式：r² ≤ x·2^32 < (r+1)² —— 两边都放大 2^32（**标度域**）再比。
        // 先移位后比较会丢严格性：(r+1)² 不是 2^32 的倍数时，floor 后的商
        // 可以正好等于 x，上界断言会假红。
        for raw in [1i64, 2, 12345, ONE / 3, ONE - 1, 7 * ONE, 1_000_000 * ONE] {
            let r = isqrt(raw).expect("non-negative");
            let x_scaled = (raw as i128) << FRAC_BITS;
            let lo = (r as i128) * (r as i128);
            let hi = ((r as i128) + 1) * ((r as i128) + 1);
            assert!(lo <= x_scaled, "floor too high for {raw}: {r}");
            assert!(hi > x_scaled, "floor too low for {raw}: {r}");
        }
    }

    #[test]
    fn isqrt_rejects_negative() {
        let err = isqrt(-ONE).expect_err("negative must be rejected");
        assert_eq!(err.code(), codes::FIXED);
        assert!(err.to_string().starts_with(codes::FIXED), "{err}");
    }
}
