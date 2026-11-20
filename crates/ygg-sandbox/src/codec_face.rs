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

// ── 遍历器与 encode ─────────────────────────────────────────────────────────

/// 嵌套深度上限（与 `init.lua` 的 `MAX_DEPTH` 逐值同源：改一处必改两处）。
const MAX_DEPTH: usize = 64;

/// 确定性 JSON 编码器：一次遍历产出字节，不回头、不分配中间字符串。
struct Encoder {
    out: Vec<u8>,
    /// 正在编码的表的**身份**栈（循环引用检测）。用指针身份而不是结构等价：
    /// 同一个表出现两次是环，两个内容相同的表不是（与 Lua 侧 `stack[value]` 同义）。
    stack: Vec<*const std::ffi::c_void>,
}

impl Encoder {
    fn new(capacity: usize) -> Self {
        Encoder {
            out: Vec::with_capacity(capacity),
            stack: Vec::new(),
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        self.out.extend_from_slice(bytes);
    }

    /// 一个 Lua 值 → 字节。值域拒绝一律 [`CodecFaceError::Unsupported`]
    /// （调用方随后回退 Lua 路径，由那里产出规范文案）。
    fn value(&mut self, value: &mlua::Value, depth: usize) -> Result<(), CodecFaceError> {
        match value {
            mlua::Value::Nil => Err(unsupported("nil 不是可编码的值（对象键用「不存在」表达）")),
            mlua::Value::Boolean(true) => {
                self.push(b"true");
                Ok(())
            }
            mlua::Value::Boolean(false) => {
                self.push(b"false");
                Ok(())
            }
            mlua::Value::Integer(integer) => {
                self.push(integer_text(*integer).as_bytes());
                Ok(())
            }
            mlua::Value::Number(number) => {
                self.push(number_text(*number)?.as_bytes());
                Ok(())
            }
            mlua::Value::String(text) => {
                let bytes = text.as_bytes();
                self.push(&string_text(&bytes));
                Ok(())
            }
            mlua::Value::Table(table) => self.table(table, depth),
            other => Err(unsupported(format!(
                "类型 {} 不可编码",
                other.type_name()
            ))),
        }
    }

    /// 一张表 → 字节：先查环/深度/元方法，再分类（array / object），再发射。
    fn table(&mut self, table: &mlua::Table, depth: usize) -> Result<(), CodecFaceError> {
        let pointer = table.to_pointer() as *const std::ffi::c_void;
        if self.stack.contains(&pointer) {
            return Err(unsupported("表循环引用"));
        }
        if depth > MAX_DEPTH {
            return Err(unsupported(format!("嵌套超过 {MAX_DEPTH} 层")));
        }
        // 元方法整值回退（契约 §3.1.4）：`pairs` 会走 `__pairs`、`t[k]` 会穿
        // `__index`，复刻它们意味着编码过程中执行 Lua 代码——纯函数立刻变成
        // 可重入、可报错、可侧效。所以只要任一层带元方法，整次调用交给 Lua 路径。
        if table.metatable().is_some() {
            return Err(unsupported("带元方法的表交给 Lua 路径（元表不进字节）"));
        }

        self.stack.push(pointer);
        let result = self.table_body(table, depth);
        self.stack.pop();
        result
    }

    fn table_body(&mut self, table: &mlua::Table, depth: usize) -> Result<(), CodecFaceError> {
        let mut array_count = 0usize;
        let mut object_count = 0usize;
        let mut max_index: i64 = 0;
        let mut members: Vec<(Vec<u8>, mlua::Value)> = Vec::new();

        for pair in table.clone().pairs::<mlua::Value, mlua::Value>() {
            let (key, value) = pair.map_err(|error| internal(format!("遍历表失败：{error}")))?;
            match key {
                mlua::Value::Integer(index) => {
                    if index < 1 {
                        return Err(unsupported(format!(
                            "表的键必须是正整数或字符串（遇到下标 {index}）"
                        )));
                    }
                    array_count += 1;
                    if index > max_index {
                        max_index = index;
                    }
                }
                mlua::Value::Number(_) => {
                    return Err(unsupported("表的键必须是正整数或字符串（遇到非整数 number 键）"));
                }
                mlua::Value::String(text) => {
                    object_count += 1;
                    let bytes = text.as_bytes().to_vec();
                    members.push((bytes, value));
                }
                other => {
                    return Err(unsupported(format!(
                        "表的键必须是正整数或字符串（遇到 {} 键）",
                        other.type_name()
                    )));
                }
            }
        }

        if array_count > 0 && object_count > 0 {
            return Err(unsupported("同一张表不得混用数组键与字符串键"));
        }
        if array_count > 0 && max_index as usize != array_count {
            return Err(unsupported(format!(
                "数组有洞（最大下标 {max_index}，条目数 {array_count}）"
            )));
        }

        if array_count > 0 {
            self.push(b"[");
            for index in 1..=array_count as i64 {
                if index > 1 {
                    self.push(b",");
                }
                let item: mlua::Value = table
                    .raw_get(index)
                    .map_err(|error| internal(format!("数组下标 {index} 读取失败：{error}")))?;
                self.value(&item, depth + 1)?;
            }
            self.push(b"]");
        } else {
            // 键序 = UTF-8 字节序升序（契约 I1）。`sort_unstable` 与 Lua 的插入
            // 排序在同一把比较器下给出同一个全序：键是 Lua 字符串，字节相同即同一
            // 个键，因此不存在「相等但不同」的键需要稳定性。
            members.sort_unstable_by(|left, right| left.0.cmp(&right.0));
            self.push(b"{");
            for (position, (key, value)) in members.iter().enumerate() {
                if position > 0 {
                    self.push(b",");
                }
                self.push(&string_text(key));
                self.push(b":");
                self.value(value, depth + 1)?;
            }
            self.push(b"}");
        }
        Ok(())
    }
}

/// 一个 Lua 值 → 确定性 JSON 字节（与 `pkg.codec.encode` 逐字节相等）。
///
/// 这是面的可测入口：测试直接调它，能区分「原生拒绝（合法值被误判）」与
/// 「原生成功」，而不是只看包装层的输出字节（包装层遇 `Unsupported` 会回退，
/// 误拒在字节相等下看不出来，只丢性能）。
pub fn encode(value: &mlua::Value) -> Result<Vec<u8>, CodecFaceError> {
    let mut encoder = Encoder::new(1024);
    encoder.value(value, 0)?;
    Ok(encoder.out)
}

// ── 面安装（decode 在 C5 加入）──────────────────────────────────────────────

/// 装 `ygg.codec.*`（2 键闭集）。
///
/// 与 `ygg.sim.*` 的区别写在契约里：本面**没有** unmounted 态——它无状态、无实例，
/// 缺面时的语义是明确的一条（调用方走 Lua 路径，产出字节相同）。因此这里不需要
/// 「表始终注册 + 抛码」那套；`host_api::install` 装上就一直在场。
pub fn install(lua: &Lua) -> Result<(), mlua::Error> {
    let ygg: mlua::Table = lua.globals().raw_get("ygg")?;
    let table = lua.create_table()?;
    let encode_function = lua.create_function(
        |lua: &Lua, value: mlua::Value| -> Result<mlua::LuaString, mlua::Error> {
            let bytes = encode(&value).map_err(|error| mlua::Error::runtime(error.to_message()))?;
            // 字节过面：`create_string(&[u8])` 而不是 `&str`——合法输出可以含
            // 非 UTF-8 字节（契约 §3.1.4「字节过面不要求 UTF-8」）。
            lua.create_string(&bytes)
        },
    )?;
    table.raw_set("encode", encode_function)?;
    ygg.raw_set("codec", table)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// 字符串向量：`(输入字节, 期望字面量字节)`，期望值同样取自规范执行体。
    #[test]
    fn string_text_matches_the_lua_oracle_vectors() {
        let cases: &[(&[u8], &[u8])] = &[
            (b"", b"\"\""),
            (b"abc", b"\"abc\""),
            (b"quote\"inside", b"\"quote\\\"inside\""),
            (b"back\\slash", b"\"back\\\\slash\""),
            (
                b"tab\tnewline\nreturn\r",
                b"\"tab\\tnewline\\nreturn\\r\"",
            ),
            (b"\x08\x0c", b"\"\\b\\f\""),
            (
                &[0x00, 0x01, 0x1f, 0x7f],
                b"\"\\u0000\\u0001\\u001f\\u007f\"",
            ),
            (&[0x80, 0x9f, 0xff], b"\"\x80\x9f\xff\""),
            (
                "任务 0".as_bytes(),
                b"\"\xe4\xbb\xbb\xe5\x8a\xa1 0\"".as_slice(),
            ),
            (
                "emoji 🙂".as_bytes(),
                b"\"emoji \xf0\x9f\x99\x82\"".as_slice(),
            ),
            (b"sol/forward", b"\"sol/forward\""),
        ];
        for (input, expected) in cases {
            assert_eq!(
                string_text(input),
                expected.to_vec(),
                "输入字节 {input:02x?} 的转义与规范执行体不一致"
            );
        }
    }

    // ── 遍历器与 encode（需要真 Lua 状态）──────────────────────────────────

    fn encode_of(lua: &Lua, chunk: &str) -> Result<Vec<u8>, CodecFaceError> {
        let value: mlua::Value = lua.load(chunk).eval().expect("chunk 必须能求值");
        encode(&value)
    }

    fn encode_text(lua: &Lua, chunk: &str) -> String {
        let bytes = encode_of(lua, chunk).expect("合法值必须编码成功");
        String::from_utf8(bytes).expect("本组用例输出都是 ASCII/UTF-8")
    }

    #[test]
    fn arrays_and_objects_emit_canonical_bytes() {
        let lua = Lua::new();
        assert_eq!(encode_text(&lua, "return {1, 2, 3}"), "[1,2,3]");
        // 空表是 object 不是 array：Lua 侧 `table_kind` 在无数组证据时给 object
        assert_eq!(encode_text(&lua, "return {}"), "{}");
        // 键序 = 字节序：两种插入序必须同一份字节（契约 I1）
        assert_eq!(
            encode_text(&lua, "return {i = 1, f = 1.5}"),
            encode_text(&lua, "return {f = 1.5, i = 1}")
        );
        assert_eq!(encode_text(&lua, "return {i = 1, f = 1.5}"), "{\"f\":1.5,\"i\":1}");
        // 整数与浮点分开写（R2）
        assert_eq!(encode_text(&lua, "return {1, 1.0}"), "[1,1.0]");
        // 嵌套 + 空对象
        assert_eq!(
            encode_text(&lua, "return {z = {b = true, a = false}, a = {}}"),
            "{\"a\":{},\"z\":{\"a\":false,\"b\":true}}"
        );
    }

    #[test]
    fn scalars_at_top_level_encode() {
        let lua = Lua::new();
        assert_eq!(encode_text(&lua, "return 1"), "1");
        assert_eq!(encode_text(&lua, "return 1.0"), "1.0");
        assert_eq!(encode_text(&lua, "return -0.0"), "0.0");
        assert_eq!(encode_text(&lua, "return 'x'"), "\"x\"");
        assert_eq!(encode_text(&lua, "return true"), "true");
    }

    #[test]
    fn value_domain_rejections_are_unsupported() {
        let lua = Lua::new();
        let cases = [
            "return nil",
            "return {1, nil, 3}",   // 洞
            "local t = {1}; t[3] = 2; return t", // 洞（最大下标 3，条目 2）
            "return {1, a = 2}",    // 数组与字符串键混用
            "return {[1.5] = 1}",   // 非整数键
            "return {[true] = 1}",  // 布尔键
            "return {f = function() end}",
            "return 0/0",           // NaN
            "return 1/0",           // +inf
            "local t = {}; t.self = t; return t", // 自环
        ];
        for chunk in cases {
            let error = encode_of(&lua, chunk).expect_err(chunk);
            assert_eq!(error.code(), CODEC_UNSUPPORTED, "chunk 应为值域拒绝：{chunk}");
        }
    }

    #[test]
    fn depth_limit_is_sixty_four() {
        let lua = Lua::new();
        let ok = encode_of(
            &lua,
            "local t = {}; local head = t; for _ = 1, 64 do local n = {}; t[1] = n; t = n end; return head",
        );
        assert!(ok.is_ok(), "64 层必须可编码");
        let too_deep = encode_of(
            &lua,
            "local t = {}; local head = t; for _ = 1, 65 do local n = {}; t[1] = n; t = n end; return head",
        );
        assert_eq!(
            too_deep.expect_err("65 层必须被拒").code(),
            CODEC_UNSUPPORTED
        );
    }

    #[test]
    fn tables_with_metatables_fall_back_to_the_lua_path() {
        let lua = Lua::new();
        for chunk in [
            "return setmetatable({1}, {})",
            "return setmetatable({a = 1}, {__index = function() return 9 end})",
            "return setmetatable({}, {__pairs = function() return function() return nil end end})",
            "return {nested = setmetatable({}, {__index = function() end})}",
        ] {
            let error = encode_of(&lua, chunk).expect_err(chunk);
            assert_eq!(error.code(), CODEC_UNSUPPORTED, "元表表必须整值回退：{chunk}");
        }
    }

    #[test]
    fn non_utf8_bytes_pass_through_unchanged() {
        let lua = Lua::new();
        let bytes = encode_of(&lua, "return string.char(0x80, 0x9f, 0xff)").expect("字节串可编码");
        assert_eq!(bytes, b"\"\x80\x9f\xff\"");
        // 控制字符走 \u00xx，0x7f  тоже
        let control = encode_of(&lua, "return string.char(0, 1, 0x1f, 0x7f)").expect("控制字符可编码");
        assert_eq!(control, b"\"\\u0000\\u0001\\u001f\\u007f\"");
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
