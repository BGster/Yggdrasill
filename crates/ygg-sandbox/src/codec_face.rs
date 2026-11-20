//! `ygg.codec.*` —— 确定性 JSON 编解码的宿主面（原生加速面）。
//!
//! 契约：`docs/design/kernel/lua-api.md` §3.1.4（面形状 / 字节过面 / 两码）与
//! `docs/design/packages/codec.md` §4.1（下沉边界：谁下沉、谁回退、差分门禁）。
//!
//! # 这个面是什么、不是什么
//!
//! 它是 `pkg.codec` **Lua 实现的符合实现**，不是第二个规范：规范在包契约 I1 与
//! `packages/libs/codec/init.lua` 那四个函数上，本模块的每一个字节都必须与它相等
//! （判据 = `crates/ygg-sandbox/tests/codec_face.rs` 的差分字节相等）。
//!
//! 面只做两件事：`encode(value) -> string`、`decode(text) -> value`。**一个逐节点
//! getter 都没有**（批式铁律：一个值整包进出，一次调用一次穿越）。
//!
//! # 为什么放在 `ygg-sandbox` 而不是 `ygg-sim`
//!
//! 三条结构证据（计划稿 §2.3/§2.4，逐条可复现）：① `ygg-sim` 不依赖 mlua（Tier 1
//! 必须无 VM 可编译可基准），拿不到 Lua 值；② `ygg-sim/src/**` 禁浮点 token
//! （`tests/determinism_scan.rs` R1），而 JSON 数字的最短往返格式化必须有 `f64`；
//! ③ 快照是带 magic 与尾 digest 的二进制列帧，本面的容器是 JSON 文本（存档兼容面）。
//! 复用 Tier 1 的是**规范序纪律**（键按 UTF-8 字节序升序）与其实测证据，不是它的函数。
//!
//! # 与 Lua 实现的逐条对应（改任一侧都必须跑差分门禁）
//!
//! | 本模块 | `init.lua` | 规则 |
//! | --- | --- | --- |
//! | [`integer_text`] | `is_integer` + `tostring` | 整数子类型走 `%d` 形态 |
//! | [`number_text`] | `number_text` | 最短往返 `%g` 精度试探（1..17） |
//! | [`format_g`] | `("%.Pg"):format` | C `printf` 的 `%g` 语义 |
//! | [`string_text`] | `string_text` + `ESCAPES` | JSON 转义（非 ASCII 原样字节） |
//!
//! 表分类、键域、深度、环、元方法回退与解析器在后续块里加入（同一份契约）。

use mlua::Lua;

/// 值域拒绝 / 带元方法的表：调用方（`pkg.codec`）据此**回退 Lua 路径**。
pub const CODEC_UNSUPPORTED: &str = "YGG-CODEC-UNSUPPORTED";

/// 原生实现内部失败：**不回退**（缺陷必须响，静默降级会把 bug 藏成性能问题）。
pub const CODEC_INTERNAL: &str = "YGG-CODEC-INTERNAL";

/// 面的错误：两态，映射到 [`CODEC_UNSUPPORTED`] / [`CODEC_INTERNAL`] 两码。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecFaceError {
    /// 输入落在原生路径之外（值域拒绝 / 带元方法的表）⇒ 调用方回退。
    Unsupported(String),
    /// 实现内部失败 ⇒ 调用方报 `YGG-PKG-CODEC-HOST` 并不回退。
    Internal(String),
}

impl CodecFaceError {
    /// 错误码（消息首段，与 `YGG-SIM-*` / `YGG-FS-*` 同一处置）。
    pub fn code(&self) -> &'static str {
        match self {
            CodecFaceError::Unsupported(_) => CODEC_UNSUPPORTED,
            CodecFaceError::Internal(_) => CODEC_INTERNAL,
        }
    }

    /// 细节（不含码前缀）。
    pub fn message(&self) -> &str {
        match self {
            CodecFaceError::Unsupported(message) | CodecFaceError::Internal(message) => message,
        }
    }

    /// `"<码>: <细节>"`：过 mlua 边界时作为 runtime error 的消息。
    pub fn to_message(&self) -> String {
        format!("{}: {}", self.code(), self.message())
    }
}

impl std::fmt::Display for CodecFaceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.to_message())
    }
}

impl std::error::Error for CodecFaceError {}

/// 值域拒绝（回退）。
pub(crate) fn unsupported(detail: impl Into<String>) -> CodecFaceError {
    CodecFaceError::Unsupported(detail.into())
}

/// 实现内部失败（不回退）。
pub(crate) fn internal(detail: impl Into<String>) -> CodecFaceError {
    CodecFaceError::Internal(detail.into())
}

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
    debug_assert!(precision >= 1 && precision <= 17);
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
    let allow_exponent = !(magnitude >= 1e-4 && magnitude < 1e15);
    let mut chosen: Option<String> = None;
    for precision in 1..=17 {
        let candidate = format_g(magnitude, precision);
        if candidate.parse::<f64>().ok() == Some(magnitude) {
            let uses_exponent = candidate.contains('e') || candidate.contains('E');
            if allow_exponent || !uses_exponent {
                chosen = Some(candidate);
                break;
            }
        }
    }
    let mut text = chosen.unwrap_or_else(|| format_g(magnitude, 17));
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

// ── 字符串：JSON 字面量（非 ASCII 原样字节）────────────────────────────────

/// 字节 → JSON 字符串字面量（含首尾引号）。与 `init.lua` 的 `string_text` 逐字节相等。
///
/// 非 ASCII **原样直通**（UTF-8 也好、非法字节也好）：多一层 `\uXXXX` 只多一个
/// 出错面，确定性判据不需要它。控制字符是 ASCII 定义（`< 0x20` 与 `0x7f`，
/// 实测 `0x80–0x9f` 不在 Lua `%c` 内）⇒ 走 `\u00xx`（小写四位）。
pub fn string_text(bytes: &[u8]) -> Vec<u8> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = Vec::with_capacity(bytes.len() + 2);
    out.push(b'"');
    let mut start = 0;
    for (index, byte) in bytes.iter().copied().enumerate() {
        let short: Option<&[u8]> = match byte {
            b'"' => Some(b"\\\""),
            b'\\' => Some(b"\\\\"),
            b'\n' => Some(b"\\n"),
            b'\r' => Some(b"\\r"),
            b'\t' => Some(b"\\t"),
            0x08 => Some(b"\\b"),
            0x0c => Some(b"\\f"),
            0x00..=0x1f | 0x7f => None, // 走 \u00xx
            _ => continue,
        };
        if index > start {
            out.extend_from_slice(&bytes[start..index]);
        }
        match short {
            Some(escape) => out.extend_from_slice(escape),
            None => {
                out.extend_from_slice(b"\\u00");
                out.push(HEX[(byte >> 4) as usize]);
                out.push(HEX[(byte & 0x0f) as usize]);
            }
        }
        start = index + 1;
    }
    if start < bytes.len() {
        out.extend_from_slice(&bytes[start..]);
    }
    out.push(b'"');
    out
}

// ── 面安装（后续块补 decode / 遍历器；本块只留骨架）────────────────────────

/// 装 `ygg.codec.*`（2 键闭集）。
///
/// 与 `ygg.sim.*` 的区别写在契约里：本面**没有** unmounted 态——它无状态、无实例，
/// 缺面时的语义是明确的一条（调用方走 Lua 路径，产出字节相同）。因此这里不需要
/// 「表始终注册 + 抛码」那套；`host_api::install` 装上就一直在场。
pub fn install(lua: &Lua) -> Result<(), mlua::Error> {
    let ygg: mlua::Table = lua.globals().raw_get("ygg")?;
    let table = lua.create_table()?;
    ygg.raw_set("codec", table)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(format_g(1.2345678901234568e17, 17), "1.2345678901234568e+17");
        assert_eq!(format_g(0.1, 17), "0.10000000000000001");
        assert_eq!(format_g(1234.5, 17), "1234.5");
    }
}
