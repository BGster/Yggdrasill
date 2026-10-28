//! 位置工具：字节偏移 → (line, column)；R3 的 parse 前文本预扫。
//!
//! R3 必须站在 parser 之外（research Architecture Insight 3）：点形态
//! goto 在 Lua 5.4 是语法错误，full_moon（lua54）会在该处直接解析失败、
//! AST 无法构建——先预扫捕获全部命中（含精确位置），无命中才进解析。

/// 字节偏移 → (line, column)。均 1 起；column 为字节列（与 codespan 的
/// 字节 range 口径一致，多字节字符行不做显示列换算）。
pub fn line_col(source: &str, offset: usize) -> (usize, usize) {
    let mut line = 1usize;
    let mut col = 1usize;
    for (index, byte) in source.bytes().enumerate() {
        if index >= offset {
            break;
        }
        if byte == b'\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// R3 预扫：`ygg.narrative.goto` + 可选空白/制表 + `(` 的字节偏移列表。
/// 引号感知（单/双引号 + 反斜杠转义）跳过字符串字面量内的命中；多行
/// `[[ ]]` 字符串不追踪（生成物场景概率极低，已知近似，research Q3 同口径）。
pub fn find_goto_dot_calls(source: &str) -> Vec<usize> {
    const NEEDLE: &[u8] = b"ygg.narrative.goto";
    let bytes = source.as_bytes();
    let mut hits = Vec::new();
    let mut in_string: Option<u8> = None;
    let mut i = 0usize;
    while i < bytes.len() {
        let byte = bytes[i];
        match in_string {
            Some(quote) => {
                if byte == b'\\' {
                    i += 2; // 转义对整体跳过（末尾越界由循环条件兜住）。
                    continue;
                }
                if byte == quote {
                    in_string = None;
                }
                i += 1;
            }
            None => {
                if byte == b'"' || byte == b'\'' {
                    in_string = Some(byte);
                    i += 1;
                } else if bytes[i..].starts_with(NEEDLE) {
                    let mut j = i + NEEDLE.len();
                    while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                        j += 1;
                    }
                    if j < bytes.len() && bytes[j] == b'(' {
                        hits.push(i);
                    }
                    i += NEEDLE.len();
                } else {
                    i += 1;
                }
            }
        }
    }
    hits
}
