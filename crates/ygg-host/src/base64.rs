//! 标准 base64（`+/` 字母表，`=` 填充）：零依赖的编码面。
//!
//! 协议层只把 base64 当**字符串搬运**：快照帧（`snapshot.bytes`）与包镜像里的
//! `resources` 值都在这一处编解码，`session` 与 `image` 两个模块共用同一台实现
//! ——两份实现会在「哪个更宽松」上慢慢分叉，而那正好是包镜像与快照帧互相
//! 不认对方的起点。
//!
//! 解码是**严格**的：长度必须是 4 的倍数、填充只能落在末尾、字母表之外的字节
//! 一律拒（不 trim 空白、不容忍 URL-safe 变体）。宽松解码会把「对端写错了
//! 字节」变成「对端写得差不多」。

use crate::proto::ProtocolError;

/// 标准 base64 字母表（`+/`，带 `=` 填充）。
const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// 字节 → 标准 base64（快照帧可能几 MB，一次线性扫描，不引依赖）。
pub(crate) fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// 标准 base64 → 字节（严格：长度、填充位置、字母表都校验）。
pub(crate) fn base64_decode(text: &str) -> Result<Vec<u8>, ProtocolError> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return Err(protocol("base64 长度必须是 4 的倍数"));
    }
    let quads = bytes.len() / 4;
    let mut out = Vec::with_capacity(quads * 3);
    for (index, quad) in bytes.chunks(4).enumerate() {
        let last = index + 1 == quads;
        let mut n = 0u32;
        let mut pad = 0usize;
        for (slot, &byte) in quad.iter().enumerate() {
            if byte == b'=' {
                if !last || slot < 2 {
                    return Err(protocol("base64 填充位置非法"));
                }
                pad += 1;
                n <<= 6;
                continue;
            }
            // 填充只能落在末尾：`ab=c` 这种「中间挖洞」不是合法 base64。
            if pad > 0 {
                return Err(protocol("base64 填充必须在末尾"));
            }
            n = (n << 6) | base64_value(byte).ok_or_else(|| protocol("base64 含非法字符"))?;
        }
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    Ok(out)
}

fn base64_value(byte: u8) -> Option<u32> {
    let value = match byte {
        b'A'..=b'Z' => byte - b'A',
        b'a'..=b'z' => byte - b'a' + 26,
        b'0'..=b'9' => byte - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => return None,
    };
    Some(u32::from(value))
}

fn protocol(msg: impl Into<String>) -> ProtocolError {
    ProtocolError::protocol(msg)
}

#[cfg(test)]
mod tests {
    use super::{base64_decode, base64_encode};

    #[test]
    fn base64_round_trips_every_tail_length() {
        for len in 0..=32usize {
            let bytes: Vec<u8> = (0..len).map(|i| (i as u8).wrapping_mul(37)).collect();
            let text = base64_encode(&bytes);
            assert_eq!(base64_decode(&text).unwrap(), bytes, "len={len}");
        }
    }

    #[test]
    fn base64_rejects_bad_input() {
        for bad in ["a", "a===", "=AAA", "ab=c", "ab$d", "AAAA="] {
            assert!(base64_decode(bad).is_err(), "{bad}");
        }
    }

    /// 标准向量（RFC 4648 §10）与镜像/快照两侧的常见错法：严格解码器必须
    /// 认得出「合法的」、也拒得掉「像合法但不合法」的。
    #[test]
    fn base64_matches_standard_vectors_and_keeps_rejections() {
        for (text, bytes) in [
            ("", b"".as_slice()),
            ("Zg==", b"f".as_slice()),
            ("Zm8=", b"fo".as_slice()),
            ("Zm9v", b"foo".as_slice()),
            ("Zm9vYg==", b"foob".as_slice()),
            ("Zm9vYmE=", b"fooba".as_slice()),
            ("Zm9vYmFy", b"foobar".as_slice()),
        ] {
            assert_eq!(base64_decode(text).unwrap(), bytes, "{text}");
            assert_eq!(base64_encode(bytes), text, "{text}");
        }
        for bad in ["Zm 9v", "Zm9v=", "Z===", "A", "Zm9*", "Zg==Zm8="] {
            assert!(base64_decode(bad).is_err(), "{bad}");
        }
    }
}
