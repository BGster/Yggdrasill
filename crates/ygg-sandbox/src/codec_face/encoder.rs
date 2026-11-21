// ── 字符串：JSON 字面量（非 ASCII 原样字节）────────────────────────────────

/// 字节 → JSON 字符串字面量（含首尾引号）。与 `init.lua` 的 `string_text` 逐字节相等。
///
/// 非 ASCII **原样直通**（UTF-8 也好、非法字节也好）：多一层 `\uXXXX` 只多一个
/// 出错面，确定性判据不需要它。控制字符是 ASCII 定义（`< 0x20` 与 `0x7f`，
/// 实测 `0x80–0x9f` 不在 Lua `%c` 内）⇒ 走 `\u00xx`（小写四位）。
fn push_json_string(out: &mut Vec<u8>, bytes: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.reserve(bytes.len() + 2);
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
}

/// 字节 → JSON 字符串字面量（独立分配版；面向测试与单值使用）。
pub fn string_text(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 2);
    push_json_string(&mut out, bytes);
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
                push_json_string(&mut self.out, &bytes);
                Ok(())
            }
            mlua::Value::Table(table) => self.table(table, depth),
            other => Err(unsupported(format!("类型 {} 不可编码", other.type_name()))),
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
                    return Err(unsupported(
                        "表的键必须是正整数或字符串（遇到非整数 number 键）",
                    ));
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
                push_json_string(&mut self.out, key);
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

use super::number::{integer_text, number_text};
use super::{internal, unsupported, CodecFaceError};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec_face::CODEC_UNSUPPORTED;
    use mlua::Lua;

    /// 字符串向量：`(输入字节, 期望字面量字节)`，期望值同样取自规范执行体。
    #[test]
    fn string_text_matches_the_lua_oracle_vectors() {
        let cases: &[(&[u8], &[u8])] = &[
            (b"", b"\"\""),
            (b"abc", b"\"abc\""),
            (b"quote\"inside", b"\"quote\\\"inside\""),
            (b"back\\slash", b"\"back\\\\slash\""),
            (b"tab\tnewline\nreturn\r", b"\"tab\\tnewline\\nreturn\\r\""),
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
        assert_eq!(
            encode_text(&lua, "return {i = 1, f = 1.5}"),
            "{\"f\":1.5,\"i\":1}"
        );
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
            "return {1, nil, 3}",                // 洞
            "local t = {1}; t[3] = 2; return t", // 洞（最大下标 3，条目 2）
            "return {1, a = 2}",                 // 数组与字符串键混用
            "return {[1.5] = 1}",                // 非整数键
            "return {[true] = 1}",               // 布尔键
            "return {f = function() end}",
            "return 0/0",                         // NaN
            "return 1/0",                         // +inf
            "local t = {}; t.self = t; return t", // 自环
        ];
        for chunk in cases {
            let error = encode_of(&lua, chunk).expect_err(chunk);
            assert_eq!(
                error.code(),
                CODEC_UNSUPPORTED,
                "chunk 应为值域拒绝：{chunk}"
            );
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
            assert_eq!(
                error.code(),
                CODEC_UNSUPPORTED,
                "元表表必须整值回退：{chunk}"
            );
        }
    }

    #[test]
    fn non_utf8_bytes_pass_through_unchanged() {
        let lua = Lua::new();
        let bytes = encode_of(&lua, "return string.char(0x80, 0x9f, 0xff)").expect("字节串可编码");
        assert_eq!(bytes, b"\"\x80\x9f\xff\"");
        // 控制字符走 \u00xx，0x7f  тоже
        let control =
            encode_of(&lua, "return string.char(0, 1, 0x1f, 0x7f)").expect("控制字符可编码");
        assert_eq!(control, b"\"\\u0000\\u0001\\u001f\\u007f\"");
    }
}
