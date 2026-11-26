//! 位置工具：字节偏移 → (line, column)。
//!
//! 诊断的 line/column 一律从这里推导（[`Diagnostic`](crate::diagnostic::Diagnostic)
//! 只存 offset，行列是派生量）——bin 侧 codespan 与 `--json` 面的位置因此同源。

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
