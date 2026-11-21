// ── 数字：C `printf` 的 `%g` 口径 ─────────────────────────────────────────────

/// 去掉尾部的 `'0'`（`%g` 的「去尾零」规则），返回可显示的字节切片。
fn trimmed(value: &[u8]) -> &[u8] {
    let mut end = value.len();
    while end > 0 && value[end - 1] == b'0' {
        end -= 1;
    }
    &value[..end]
}

/// 按 C `printf("%.<precision>g", value)` 的语义渲染**正有限非零**值。
///
/// 三条规则来自 C 标准（`%g` 的定义），逐条对应：
/// 1. 先把值舍入到 `precision` 位有效数字，取十进制指数 `e10`；
/// 2. `e10 < -4 || e10 >= precision` ⇒ 指数形（`%e`，小数位 `precision-1`），
///    否则定点形（`%f`，小数位 `precision-1-e10`）；
/// 3. 两种形态都去尾零、无小数则不留小数点；指数至少两位（`e+05` / `e-07`）。
///
/// 舍入本身交给 Rust 的 `{:.*e}`（正确舍入、与 C 一致，含 round-half-to-even），
/// 本函数只做**形态选择**——那是 `%g` 与 `%e` 唯一不同的地方。
fn format_g(value: f64, precision: usize) -> String {
    debug_assert!((1..=17).contains(&precision));
    debug_assert!(value.is_finite() && value > 0.0);

    let scientific = format!("{:.*e}", precision - 1, value);
    let (mantissa, exponent) = scientific
        .split_once('e')
        .expect("Rust 的 LowerExp 一定带 e");
    let e10: i32 = exponent.parse().expect("指数是十进制整数");
    let digits: Vec<u8> = mantissa.bytes().filter(|byte| *byte != b'.').collect();
    debug_assert_eq!(digits.len(), precision);

    let mut out = String::with_capacity(24);
    if e10 < -4 || e10 >= precision as i32 {
        out.push(digits[0] as char);
        let fraction = trimmed(&digits[1..]);
        if !fraction.is_empty() {
            out.push('.');
            out.push_str(std::str::from_utf8(fraction).expect("ASCII 数字"));
        }
        out.push('e');
        let magnitude = e10.unsigned_abs();
        out.push(if e10 < 0 { '-' } else { '+' });
        if magnitude < 10 {
            out.push('0');
        }
        out.push_str(&magnitude.to_string());
    } else {
        let integer_len = e10 + 1;
        if integer_len <= 0 {
            // 0.000ddd…：小数点后补 (e10 的绝对值) 个零
            out.push_str("0.");
            for _ in 0..(-integer_len) {
                out.push('0');
            }
            out.push_str(std::str::from_utf8(&digits).expect("ASCII 数字"));
        } else {
            let integer_len = integer_len as usize;
            out.push_str(std::str::from_utf8(&digits[..integer_len]).expect("ASCII 数字"));
            if let Some(fraction) = digits.get(integer_len..) {
                let fraction = trimmed(fraction);
                if !fraction.is_empty() {
                    out.push('.');
                    out.push_str(std::str::from_utf8(fraction).expect("ASCII 数字"));
                }
            }
        }
    }
    out
}

/// 整数子类型 → 字节：Lua `%d` 与 Rust `i64::to_string` 是同一个十进制形态。
pub fn integer_text(value: i64) -> String {
    value.to_string()
}

/// 浮点子类型 → 最短往返字节（与 `init.lua` 的 `number_text` 逐字节相等）。
///
/// 三条规则（缺一破字节稳定，见包契约 §3.2 R3）：
/// 1. 取**第一个能往返的精度** `%.1g … %.17g`（固定 17 位会把 `0.1` 写成
///    `0.10000000000000001`）；
/// 2. 量级在 `[1e-4, 1e15)` 内时优先十进制形态——该区间里 `%g` 若给出指数形，
///    继续试更高精度直到出现定点形（这是 `1e8` 编码成 `100000000.0` 而不是
///    `1e08` 的唯一原因）；
/// 3. `-0.0` 归一到 `0.0`；无小数点也无指数则补 `.0` 保住「浮点」身份。
pub fn number_text(value: f64) -> Result<String, CodecFaceError> {
    if value.is_nan() {
        return Err(unsupported("NaN 不是可编码的值"));
    }
    if value.is_infinite() {
        return Err(unsupported("±inf 不是可编码的值"));
    }
    if value == 0.0 {
        // 同时覆盖 -0.0：零只有一个存诸形（与内核 normalize_f64 同口径）
        return Ok("0.0".to_string());
    }

    let magnitude = value.abs();
    let allow_exponent = !(1e-4..1e15).contains(&magnitude);

    // 一次定位「第一个能往返的精度」，不做 1..17 的逐个试探。
    //
    // 依据：`%.Pg` 能往返 ⇔ P 位有效数字的十进制串解析回来还是原值 ⇔ P ≥ 该值的
    // **最短往返位数**。而 Rust 的 `{:e}`（无精度）给的就是最短往返数字，所以
    // 「第一个能往返的 P」= 它的有效位数——一次格式化即可定位，不必 17 次。
    let scientific = format!("{magnitude:e}");
    let (mantissa, exponent) = scientific
        .split_once('e')
        .expect("Rust 的 LowerExp 一定带 e");
    let shortest = mantissa
        .bytes()
        .filter(|byte| *byte != b'.')
        .count()
        .clamp(1, 17);
    let e10: i32 = exponent.parse().expect("指数是十进制整数");

    // 量级落在 `[1e-4, 1e15)` 时须用定点形（契约 R3 第 2 条）：若最短位数的形态
    // 是指数形，就提到 `e10 + 1` 位（多出来的位是零，仍是同一个值）。
    let mut precision = shortest;
    if !allow_exponent && (e10 >= precision as i32 || e10 < -4) {
        precision = ((e10 + 1).max(0) as usize).max(shortest).min(17);
    }
    let mut text = format_g(magnitude, precision);
    text = text.replace('E', "e");
    text = text.replace('+', "");
    if !text.contains('.') && !text.contains('e') {
        text.push_str(".0");
    }
    let mut text = text;
    if value < 0.0 {
        text.insert(0, '-');
    }
    Ok(text)
}

use super::{unsupported, CodecFaceError};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec_face::CODEC_UNSUPPORTED;

    /// 数字向量：`(输入位型, 期望字节)`。期望值由**规范执行体**（Lua 侧
    /// `pkg.codec`）产出后逐字抄入——方法见计划稿 §3.5（用公开面当 oracle，
    /// 不在测试里复制第三份口径）。
    const NUMBER_VECTORS: &[(u64, &str)] = &[
        (0x0000_0000_0000_0000, "0.0"),
        (0x8000_0000_0000_0000, "0.0"),
        (0x3ff8_0000_0000_0000, "1.5"),
        (0xbff8_0000_0000_0000, "-1.5"),
        (0x3fb9_9999_9999_999a, "0.1"),
        (0x4004_0000_0000_0000, "2.5"),
        (0x3fe0_0000_0000_0000, "0.5"),
        (0x3fd0_0000_0000_0000, "0.25"),
        (0x3fd3_3333_3333_3334, "0.30000000000000004"),
        (0x4197_d784_0000_0000, "100000000.0"),
        (0x42d6_bcc4_1e90_0000, "100000000000000.0"),
        (0x430c_6bf5_2634_0000, "1e15"),
        (0x4341_c379_37e0_8000, "1e16"),
        (0x444b_1ae4_d6e2_ef50, "1e21"),
        (0x54b2_49ad_2594_c37d, "1e100"),
        (0x7e37_e43c_8800_759c, "1e300"),
        (0x3f1a_36e2_eb1c_432d, "0.0001"),
        (0x3ee4_f8b5_88e3_68f1, "1e-05"),
        (0x3b92_e3b4_0a0e_9b4f, "1e-21"),
        (0x2b2b_ff2e_e48e_0530, "1e-100"),
        (0x01a5_6e1f_c2f8_f359, "1e-300"),
        (0x3e84_21f5_f40d_8376, "1.5e-07"),
        (0x0000_0000_0000_0001, "5e-324"),
        (0x7fef_ffff_ffff_ffff, "1.7976931348623157e308"),
        (0x0010_0000_0000_0000, "2.2250738585072014e-308"),
        (0x419d_6f34_5400_0000, "123456789.0"),
        (0x437b_69b4_ba63_0f35, "1.2345678901234568e17"),
        (0x4340_0000_0000_0000, "9007199254740992.0"),
        (0x3ff0_0000_0000_0001, "1.0000000000000002"),
        (0x4009_21fb_5444_2d18, "3.141592653589793"),
        (0x4005_bf0a_8b14_5769, "2.718281828459045"),
        (0x4023_fae1_47ae_147b, "9.99"),
        (0x4058_f999_9999_999a, "99.9"),
        (0x408f_3ffd_f3b6_45a2, "999.999"),
        (0x3ff0_147a_e147_ae14, "1.005"),
        (0x4005_6666_6666_6666, "2.675"),
        (0x3ee9_e3fe_580f_5494, "1.23456e-05"),
        (0xc005_6666_6666_6666, "-2.675"),
        (0xc023_fae1_47ae_147b, "-9.99"),
        (0x44df_e185_ca57_c517, "6.02214076e23"),
        (0x3c07_a4da_290c_1653, "1.602176634e-19"),
        (0x3f50_624d_d2f1_a9fc, "0.001"),
        (0x3f1a_3637_1ea5_31a8, "9.999e-05"),
        (0x42d6_bcc4_1e8f_fffa, "99999999999999.9"),
        (0x430c_6bf5_2633_ffff, "999999999999999.9"),
        (0x3f20_2c9d_edbc_309d, "0.0001234"),
        (0x4029_0000_0000_0000, "12.5"),
        (0x405f_4000_0000_0000, "125.0"),
    ];

    #[test]
    fn number_text_matches_the_lua_oracle_vectors() {
        for (bits, expected) in NUMBER_VECTORS {
            let value = f64::from_bits(*bits);
            assert_eq!(
                number_text(value).expect("有限值必然可编码"),
                *expected,
                "位型 0x{bits:016x} 的编码形态与规范执行体不一致"
            );
        }
    }

    #[test]
    fn integer_text_is_plain_decimal() {
        assert_eq!(integer_text(0), "0");
        assert_eq!(integer_text(1), "1");
        assert_eq!(integer_text(-1), "-1");
        assert_eq!(integer_text(42), "42");
        assert_eq!(integer_text(i64::MAX), "9223372036854775807");
        assert_eq!(integer_text(i64::MIN), "-9223372036854775808");
        assert_eq!(integer_text(9007199254740993), "9007199254740993");
    }

    #[test]
    fn non_finite_and_nan_are_rejected_as_unsupported() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let error = number_text(value).expect_err("非有限值必须被拒");
            assert_eq!(error.code(), CODEC_UNSUPPORTED);
        }
    }

    #[test]
    fn format_g_follows_the_c_printf_style_rule() {
        // 形态选择的三条边界（C `%g`：P > X ≥ -4 走定点，否则指数）
        assert_eq!(format_g(1.0, 1), "1");
        assert_eq!(format_g(100.0, 1), "1e+02");
        assert_eq!(format_g(1e-4, 1), "0.0001");
        assert_eq!(format_g(1e-5, 1), "1e-05");
        assert_eq!(
            format_g(1.2345678901234568e17, 17),
            "1.2345678901234568e+17"
        );
        assert_eq!(format_g(0.1, 17), "0.10000000000000001");
        assert_eq!(format_g(1234.5, 17), "1234.5");
    }
}
