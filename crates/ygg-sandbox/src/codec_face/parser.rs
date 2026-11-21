// ── 解析器与 decode ─────────────────────────────────────────────────────────

/// 原生解析器的递归深度上限。
///
/// Lua 侧**没有**这个上限（`parse_value` 直接递归），所以这里的上限是**保守护栏**：
/// 超过 256 层一律 `Unsupported`，由 Lua 路径接手（它能解多深就多深，结果与今天
/// 完全一致）。护栏只防一件事——恶意深嵌套把 Rust 栈打穿；它不是语义变更。
const MAX_PARSE_DEPTH: usize = 256;

/// `parse_value` 的结果。`Null` 是 JSON `null` 的内部哨兵（Lua 侧是 `NULL` 表）：
/// 对象里的 `null` 值 ⇒ 丢键；数组元素位的 `null` ⇒ 报错；顶层 `null` ⇒ `nil`。
enum Parsed {
    Null,
    Value(mlua::Value),
}

/// 递归下降解析器（输入的每个字节只读一次；与 Lua 侧逐字节 `sub` 的差别只在速度）。
struct Parser<'a> {
    lua: &'a Lua,
    text: &'a [u8],
    /// 0 基光标；报错时 +1 转成人读的 1 基字节偏移。
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(lua: &'a Lua, text: &'a [u8]) -> Self {
        Parser { lua, text, pos: 0 }
    }

    fn fail(&self, detail: &str) -> CodecFaceError {
        unsupported(format!("{detail} at byte {}", self.pos + 1))
    }

    fn peek(&self) -> Option<u8> {
        self.text.get(self.pos).copied()
    }

    fn skip_space(&mut self) {
        while let Some(byte) = self.peek() {
            if byte == b' ' || byte == b'\t' || byte == b'\n' || byte == b'\r' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    /// `parse_value` 的对偶。
    fn value(&mut self, depth: usize) -> Result<Parsed, CodecFaceError> {
        if depth > MAX_PARSE_DEPTH {
            return Err(self.fail("嵌套过深"));
        }
        self.skip_space();
        let byte = match self.peek() {
            Some(byte) => byte,
            None => return Err(self.fail("意外的输入结束")),
        };
        match byte {
            b'"' => Ok(Parsed::Value(mlua::Value::String(self.string()?))),
            b'{' => self.object(depth),
            b'[' => self.array(depth),
            b't' if self.text[self.pos..].starts_with(b"true") => {
                self.pos += 4;
                Ok(Parsed::Value(mlua::Value::Boolean(true)))
            }
            b'f' if self.text[self.pos..].starts_with(b"false") => {
                self.pos += 5;
                Ok(Parsed::Value(mlua::Value::Boolean(false)))
            }
            b'n' if self.text[self.pos..].starts_with(b"null") => {
                self.pos += 4;
                Ok(Parsed::Null)
            }
            b'-' | b'0'..=b'9' => Ok(Parsed::Value(self.number()?)),
            _ => Err(self.fail("无法识别的 token")),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Parsed, CodecFaceError> {
        let table = self
            .lua
            .create_table()
            .map_err(|error| internal(format!("建表失败：{error}")))?;
        self.pos += 1; // '{'
        self.skip_space();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Parsed::Value(mlua::Value::Table(table)));
        }
        loop {
            self.skip_space();
            if self.peek() != Some(b'"') {
                return Err(self.fail("对象的键必须是字符串"));
            }
            let key = self.string()?;
            self.skip_space();
            if self.peek() != Some(b':') {
                return Err(self.fail("键后缺冒号"));
            }
            self.pos += 1;
            let parsed = self.value(depth + 1)?;
            if let Parsed::Value(value) = parsed {
                table
                    .raw_set(key, value)
                    .map_err(|error| internal(format!("对象写键失败：{error}")))?;
            }
            self.skip_space();
            match self.peek() {
                None => return Err(self.fail("对象未闭合")),
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Parsed::Value(mlua::Value::Table(table)));
                }
                Some(b',') => self.pos += 1,
                Some(_) => return Err(self.fail("对象成员之间缺逗号")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Parsed, CodecFaceError> {
        let table = self
            .lua
            .create_table()
            .map_err(|error| internal(format!("建表失败：{error}")))?;
        self.pos += 1; // '['
        self.skip_space();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Parsed::Value(mlua::Value::Table(table)));
        }
        let mut index: i64 = 0;
        loop {
            let parsed = self.value(depth + 1)?;
            match parsed {
                Parsed::Null => return Err(self.fail("数组元素不得为 null（Lua 无 null 槽位）")),
                Parsed::Value(value) => {
                    index += 1;
                    table
                        .raw_set(index, value)
                        .map_err(|error| internal(format!("数组写下标失败：{error}")))?;
                }
            }
            self.skip_space();
            match self.peek() {
                None => return Err(self.fail("数组未闭合")),
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Parsed::Value(mlua::Value::Table(table)));
                }
                Some(b',') => self.pos += 1,
                Some(_) => return Err(self.fail("数组元素之间缺逗号")),
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, CodecFaceError> {
        let mut code = 0u32;
        for _ in 0..4 {
            let digit = match self.peek() {
                Some(byte @ b'0'..=b'9') => byte - b'0',
                Some(byte @ b'a'..=b'f') => byte - b'a' + 10,
                Some(byte @ b'A'..=b'F') => byte - b'A' + 10,
                // 比 Lua 更严：Lua 的 `tonumber(hex, 16)` 容忍空白与前缀符号，
                // 那些写法一律交给 Lua 路径（回退后行为与今天完全一致）。
                _ => return Err(self.fail("\\u 转义需要 4 位十六进制")),
            };
            code = code * 16 + digit as u32;
            self.pos += 1;
        }
        Ok(code)
    }

    /// `parse_string` 的对偶（含代理对合成；UTF-8 四档手写，沙箱里没有 `utf8` 库）。
    fn string(&mut self) -> Result<mlua::LuaString, CodecFaceError> {
        self.pos += 1; // 开引号
        let mut out: Vec<u8> = Vec::new();
        loop {
            let byte = match self.peek() {
                Some(byte) => byte,
                None => return Err(self.fail("字符串未闭合")),
            };
            if byte == b'"' {
                self.pos += 1;
                return self
                    .lua
                    .create_string(&out)
                    .map_err(|error| internal(format!("建字符串失败：{error}")));
            }
            if byte == b'\\' {
                self.pos += 1;
                let escape = match self.peek() {
                    Some(escape) => escape,
                    None => return Err(self.fail("转义序列被截断")),
                };
                self.pos += 1;
                match escape {
                    b'"' => out.push(b'"'),
                    b'\\' => out.push(b'\\'),
                    b'/' => out.push(b'/'),
                    b'b' => out.push(0x08),
                    b'f' => out.push(0x0c),
                    b'n' => out.push(0x0a),
                    b'r' => out.push(0x0d),
                    b't' => out.push(0x09),
                    b'u' => {
                        let mut code = self.hex4()?;
                        if (0xD800..=0xDBFF).contains(&code) {
                            if !self.text[self.pos..].starts_with(b"\\u") {
                                return Err(self.fail("高位代理后缺少低位代理"));
                            }
                            self.pos += 2;
                            let low = self.hex4()?;
                            if !(0xDC00..=0xDFFF).contains(&low) {
                                return Err(self.fail("低位代理非法"));
                            }
                            code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                        }
                        utf8_char(&mut out, code);
                    }
                    _ => return Err(self.fail("非法转义")),
                }
                continue;
            }
            if byte < 32 {
                return Err(self.fail("字符串里有未转义的控制字符"));
            }
            out.push(byte);
            self.pos += 1;
        }
    }

    /// `parse_number` 的对偶：词法决定整/浮（`.`/`e` 在场 ⇒ 浮点），整数溢出回退浮点。
    fn number(&mut self) -> Result<mlua::Value, CodecFaceError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        let mut digits = 0usize;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
            digits += 1;
        }
        if digits == 0 {
            return Err(self.fail("数字缺整数部分"));
        }
        let mut is_float = false;
        if self.peek() == Some(b'.') {
            is_float = true;
            self.pos += 1;
            let mut fraction = 0usize;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
                fraction += 1;
            }
            if fraction == 0 {
                return Err(self.fail("小数点后无数字"));
            }
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            is_float = true;
            self.pos += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.pos += 1;
            }
            let mut exponent = 0usize;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
                exponent += 1;
            }
            if exponent == 0 {
                return Err(self.fail("指数无数字"));
            }
        }
        let lexeme = std::str::from_utf8(&self.text[start..self.pos])
            .map_err(|_| self.fail("数字词法非 UTF-8"))?;
        if !is_float {
            if let Ok(integer) = lexeme.parse::<i64>() {
                return Ok(mlua::Value::Integer(integer));
            }
            // 整数溢出：Lua 的 `tonumber` 会退回浮点（契约 R10）
        }
        match lexeme.parse::<f64>() {
            Ok(number) => Ok(mlua::Value::Number(number)),
            Err(_) => Err(self.fail("数字无法解析")),
        }
    }
}

/// Unicode 码位 → UTF-8 字节（`init.lua` 的 `utf8_char` 四档，逐档同值）。
fn utf8_char(out: &mut Vec<u8>, code: u32) {
    if code < 0x80 {
        out.push(code as u8);
    } else if code < 0x800 {
        out.push(0xc0 | (code >> 6) as u8);
        out.push(0x80 | (code & 0x3f) as u8);
    } else if code < 0x10000 {
        out.push(0xe0 | (code >> 12) as u8);
        out.push(0x80 | ((code >> 6) & 0x3f) as u8);
        out.push(0x80 | (code & 0x3f) as u8);
    } else {
        out.push(0xf0 | (code >> 18) as u8);
        out.push(0x80 | ((code >> 12) & 0x3f) as u8);
        out.push(0x80 | ((code >> 6) & 0x3f) as u8);
        out.push(0x80 | (code & 0x3f) as u8);
    }
}

/// 确定性 JSON 文本 → Lua 值（与 `pkg.codec.decode` 同构）。
///
/// 顶层 `null` 返回 `nil`；对象里的 `null` 值丢键；数组元素位的 `null` 报错
/// （契约 R9）。任何**语法**问题都是 `Unsupported`：调用方回退 Lua 路径，由那里
/// 产出规范文案（`at byte N`）——本函数不回写第二套错误字符串。
pub fn decode(lua: &Lua, text: &[u8]) -> Result<mlua::Value, CodecFaceError> {
    let mut parser = Parser::new(lua, text);
    let parsed = parser.value(0)?;
    parser.skip_space();
    if parser.pos < text.len() {
        return Err(parser.fail("顶层值之后还有残留字节"));
    }
    Ok(match parsed {
        Parsed::Null => mlua::Value::Nil,
        Parsed::Value(value) => value,
    })
}

use super::{internal, unsupported, CodecFaceError};
use mlua::Lua;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec_face::encode;
    use crate::codec_face::CODEC_UNSUPPORTED;

    // ── 解析器与 decode ────────────────────────────────────────────────────

    fn decode_text(lua: &Lua, text: &[u8]) -> Result<mlua::Value, CodecFaceError> {
        decode(lua, text)
    }

    #[test]
    fn decode_round_trips_through_encode() {
        let lua = Lua::new();
        for text in [
            &b"[1,2,3]"[..],
            b"{}",
            b"{\"f\":1.5,\"i\":1}",
            b"[true,false,\"x\",{},\"a\"]",
            b"{\"nested\":{\"deep\":[1,2,{\"k\":3}]}}",
            "[\"任务 0\",\"emoji 🙂\"]".as_bytes(),
        ] {
            let value = decode_text(&lua, text).expect("合法 JSON 必须可解");
            let bytes = encode(&value).expect("解出来的值必须可编码");
            assert_eq!(
                bytes,
                text,
                "往返字节不同：{}",
                String::from_utf8_lossy(text)
            );
        }
        // `[]` 与 `{}` 是同一个 Lua 值（空表），再编码只能是 `{}`——这是规范执行体
        // 的既有非对称性（Lua 的 `table_kind` 无数组证据时给 object），不是本面的偏差。
        let empty_array = decode_text(&lua, b"[]").expect("空数组");
        assert_eq!(encode(&empty_array).expect("编码"), b"{}".to_vec());
        // 嵌套位置的 `null` 值同样丢键（R9 对整棵树生效，不只顶层对象）
        let nested_null = decode_text(&lua, b"{\"a\":[{\"k\":null,\"z\":1}]}").expect("嵌套 null");
        assert_eq!(
            encode(&nested_null).expect("编码"),
            b"{\"a\":[{\"z\":1}]}".to_vec()
        );
    }

    #[test]
    fn decode_preserves_number_subtypes() {
        let lua = Lua::new();
        let integer = decode_text(&lua, b"100").expect("整数");
        assert!(matches!(integer, mlua::Value::Integer(100)));
        for text in [&b"1e2"[..], b"100.0", b"-0.0"] {
            let lexeme = String::from_utf8_lossy(text);
            match decode_text(&lua, text).expect("浮点") {
                mlua::Value::Number(number) => {
                    let expected: f64 = std::str::from_utf8(text).unwrap().parse().unwrap();
                    assert_eq!(number.to_bits(), expected.to_bits(), "{lexeme}");
                }
                other => panic!("{lexeme} 必须解成浮点，得到 {}", other.type_name()),
            }
        }
        // 整数溢出退回浮点（契约 R10）
        match decode_text(&lua, b"99999999999999999999").expect("溢出") {
            mlua::Value::Number(number) => assert_eq!(number, 1e20),
            other => panic!("溢出必须退回浮点，得到 {}", other.type_name()),
        }
        // "-0" 是整数零（与 Lua 的 tonumber 一致），再编码成 "0"
        assert!(matches!(
            decode_text(&lua, b"-0").expect("负零整数"),
            mlua::Value::Integer(0)
        ));
    }

    #[test]
    fn decode_handles_null_positions_and_escapes() {
        let lua = Lua::new();
        assert!(matches!(
            decode_text(&lua, b"null").expect("顶层 null"),
            mlua::Value::Nil
        ));
        let dropped = decode_text(&lua, b"{\"a\":null,\"b\":1}").expect("对象 null 丢键");
        assert_eq!(encode(&dropped).expect("编码"), b"{\"b\":1}".to_vec());
        assert_eq!(
            decode_text(&lua, b"[1,null]")
                .expect_err("数组 null 必须被拒")
                .code(),
            CODEC_UNSUPPORTED
        );
        // 代理对合成 + 单字节转义
        let value = decode_text(&lua, b"\"\\ud83d\\ude42\\/\\u00e9\"").expect("转义");
        assert_eq!(
            encode(&value).expect("编码"),
            "\"🙂/é\"".as_bytes().to_vec()
        );
        for bad in [
            &b"\"\\ud83d\""[..], // 高位代理无低位
            b"\"\\ud83dx\"",     // 低位位置不是 \u
            b"\"\\q\"",          // 非法转义
            b"\"\\u12\"",        // \u 不足 4 位
            b"\"\x01\"",         // 未转义控制字符
            b"\"abc",            // 未闭合
        ] {
            assert_eq!(
                decode_text(&lua, bad)
                    .expect_err("非法转义/控制字符")
                    .code(),
                CODEC_UNSUPPORTED,
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
    }

    #[test]
    fn decode_rejects_malformed_documents() {
        let lua = Lua::new();
        for bad in [
            &b""[..],
            b" ",
            b"[1,2",
            b"{\"a\":1",
            b"{\"a\" 1}",
            b"{a:1}",
            b"[1 2]",
            b"1 2",
            b"trueX",
            b"tru",
            b"+1",
            b"1.",
            b"1e",
            b"01x",
            b"{}extra",
        ] {
            assert_eq!(
                decode_text(&lua, bad).expect_err("坏文档").code(),
                CODEC_UNSUPPORTED,
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
        // 深度护栏：超过 256 层交回 Lua 路径（不是语义变更）
        let deep = format!("{}{}", "[".repeat(300), "]".repeat(300));
        assert_eq!(
            decode_text(&lua, deep.as_bytes()).expect_err("过深").code(),
            CODEC_UNSUPPORTED
        );
        let ok_depth = format!("{}{}", "[".repeat(32), "]".repeat(32));
        assert!(decode_text(&lua, ok_depth.as_bytes()).is_ok());
    }
}
