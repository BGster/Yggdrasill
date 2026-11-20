//! 定点（Q32.32）语义用例：四条运算语义逐条钉死（sim-kernel §3 / I2）。
//!
//! 这些用例的价值不在「跑通了」，而在**把语义写成了可执行的断言**：饱和 vs wrap、
//! 向 −∞ 取整 vs 向零截断、除零必须报错。改其中任何一条都会让回放跨平台失配，
//! 所以它们不能只活在注释里。

use ygg_sim::error::codes;
use ygg_sim::fixed::{self, FRAC_BITS, ONE};

#[test]
fn one_is_two_to_the_thirty_two() {
    assert_eq!(ONE, 1 << 32);
    assert_eq!(FRAC_BITS, 32);
    assert_eq!(fixed::from_int(1), ONE);
    assert_eq!(fixed::from_int(-1), -ONE);
    // 0.5 = 2^31
    assert_eq!(ONE / 2, 1 << 31);
}

#[test]
fn add_and_sub_saturate_instead_of_wrapping() {
    assert_eq!(fixed::add(i64::MAX, ONE), i64::MAX);
    assert_eq!(fixed::add(i64::MIN, -ONE), i64::MIN);
    assert_eq!(fixed::sub(i64::MIN, ONE), i64::MIN);
    // 普通算术不受影响
    assert_eq!(fixed::add(ONE, ONE), 2 * ONE);
    assert_eq!(fixed::sub(ONE, 3 * ONE), -2 * ONE);
}

#[test]
fn mul_uses_i128_intermediate_and_floors_toward_negative_infinity() {
    // 0.5 × 0.5 = 0.25
    assert_eq!(fixed::mul(ONE / 2, ONE / 2), (1 << 32) / 4);
    // 1.5 × 2.0 = 3.0
    assert_eq!(fixed::mul(ONE + ONE / 2, 2 * ONE), 3 * ONE);
    // 最小正值 2^-32 × 1.0 = 2^-32（不丢到 0）
    assert_eq!(fixed::mul(-1, ONE), -1);
    // 负数右移是「向 −∞ 取整」而不是「向零截断」：floor(-0.5 ulp) = -1 ulp
    assert_eq!(fixed::mul(-1, ONE / 2), -1);
    // 溢出经 i128 后 clamp（不 wrap、不 panic）
    assert_eq!(fixed::mul(i64::MAX, i64::MAX), i64::MAX);
    assert_eq!(fixed::mul(i64::MIN, 2 * ONE), i64::MIN);
}

#[test]
fn div_truncates_toward_zero_and_rejects_zero_divisor() {
    // 1.0 / 3.0 = 0.333… → 2^32/3 = 1431655765（向零截断）
    assert_eq!(fixed::div(ONE, 3 * ONE).unwrap(), 1_431_655_765);
    // 负数向零截断（不是向 −∞）：-1 ulp / 2.0 = 0
    assert_eq!(fixed::div(-1, 2 * ONE).unwrap(), 0);
    // 溢出 clamp
    assert_eq!(fixed::div(i64::MAX, 1).unwrap(), i64::MAX);
    // 除零必须报错，且码是 YGG-SIM-FIXED——静默给 MAX 会把除零藏成数值异常
    let err = fixed::div(ONE, 0).unwrap_err();
    assert_eq!(err.code(), codes::FIXED);
    assert!(err.to_string().contains("div-by-zero"), "{err}");
}

#[test]
fn to_int_floors() {
    assert_eq!(fixed::to_int(ONE), 1);
    assert_eq!(fixed::to_int(ONE - 1), 0);
    // -1 ulp 的整数部分按向下取整是 -1（不是 0）——这是「floor」与「trunc」的分界处
    assert_eq!(fixed::to_int(-1), -1);
    assert_eq!(fixed::to_int(-ONE), -1);
}

#[test]
fn decimal_text_round_trips_without_floating_point() {
    // 3.5 的 Q32.32 表示是精确的（2^31）
    let three_point_five = fixed::from_int(3) + (1 << 31);
    assert_eq!(fixed::to_decimal(three_point_five), "3.500000000");
    assert_eq!(fixed::parse_decimal("3.5").unwrap(), three_point_five);
    assert_eq!(fixed::parse_decimal("-1.25").unwrap(), -(ONE + ONE / 4));
    assert_eq!(fixed::to_decimal(-(ONE + ONE / 4)), "-1.250000000");
    // i64::MIN 的绝对值在 i64 里不可表（取绝对值必须走 i128）
    assert_eq!(fixed::to_decimal(i64::MIN), "-2147483648.000000000");
    // 整数域的顶端：2147483647.5 = (2^31-1)<<32 + 2^31（i64::MAX 是 …79999…，
    // 两者不是同一个数——差 1 ulp 的断言会把「接近」误当「相等」）
    assert_eq!(
        fixed::parse_decimal("2147483647.5").unwrap(),
        (2_147_483_647i64 << 32) + (1 << 31)
    );
}

#[test]
fn decimal_parse_is_idempotent_and_error_cases_are_explicit() {
    // 9 位小数是**有损**展开（2^-32 有 32 位十进制），所以往返只断言「误差 < 1e-9」
    // 与「文本形幂等」，不断言逐位相等——把有损说成无损才是错的。
    for text in ["0.5", "-7.25", "123.000000001", "-0.000000001"] {
        let value = fixed::parse_decimal(text).unwrap();
        let again = fixed::parse_decimal(&fixed::to_decimal(value)).unwrap();
        assert!((again - value).abs() <= 5, "{text}: {value} vs {again}");
        assert_eq!(
            fixed::to_decimal(again),
            fixed::to_decimal(fixed::parse_decimal(&fixed::to_decimal(again)).unwrap())
        );
    }
    // 错误一律 YGG-SIM-FIXED，且不静默截断/回退为 0
    for bad in ["", "-", "abc", "1.2.3", "1.1234567890", "2147483648"] {
        let err = fixed::parse_decimal(bad).unwrap_err();
        assert_eq!(err.code(), codes::FIXED, "{bad} 应当被拒");
    }
}
