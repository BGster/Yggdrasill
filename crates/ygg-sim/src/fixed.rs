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
