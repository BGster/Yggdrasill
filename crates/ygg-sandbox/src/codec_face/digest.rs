// ── sha256：checksum 摘要的宿主面（S19 下沉，第三键）─────────────────────────

/// `ygg.codec.sha256(bytes) -> string` 的原生计算：SHA-256 摘要的 64 位小写 hex。
///
/// 契约：`docs/design/packages/codec.md` §4.1「下沉边界」表的 S19 行与
/// `docs/design/kernel/lua-api.md` §3.1.4。形状与包侧 `pkg.codec.sha256` 逐字
/// 对齐：字节串进（原始字节，不要求 UTF-8）、64 位小写 hex 出——不用「返回
/// raw 32 字节」的反方案，那会让包侧还得用 Lua 补 hex，热路径白下沉。
///
/// 实现复用 `sha2`（RustCrypto，纯 Rust 无 C 依赖，`ygg-store` 的
/// `artifact.rs::sha256_hex` 同 crate 先例），hex 编码复用 [`crate::hash::
/// hex_sha256`]——crate 内单一 hex 源，不引入第二个编码器。**批式铁律**：整串
/// 进出，不做 streaming / incremental。
///
/// 类型门在包侧先行（`runtime.lua` 非字符串直接报 `YGG-PKG-CODEC-TYPE`，文案
/// 单源）；本模块的 `Unsupported` 分支只是面的自保（mlua 包装层的对称防御），
/// 正常消费者永远到不了它。
pub(crate) fn sha256(bytes: &[u8]) -> String {
    crate::hash::hex_sha256(bytes)
}

#[cfg(test)]
mod tests {
    use super::sha256;

    /// FIPS 180-4 附录 B 的两个标准向量 + 跨块边界（55/56/63/64/65 字节，
    /// 与包侧 `test/pkg-codec/sha256-encode.lua` 的向量集同一组钉值）。
    #[test]
    fn nist_vectors_and_block_boundaries() {
        // FIPS 180-4 B.1：空串
        assert_eq!(
            sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        // FIPS 180-4 B.2："abc"
        assert_eq!(
            sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // FIPS 180-4 B.3：448 位双块示例
        assert_eq!(
            sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        // 填充边界（包侧向量集同源钉值）：55 = 填充前一格、56 = 跨块、
        // 64 = 整块无余量、65 = 整块 + 1
        let a55: Vec<u8> = vec![b'a'; 55];
        assert_eq!(
            sha256(&a55),
            "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318"
        );
        let a56: Vec<u8> = vec![b'a'; 56];
        assert_eq!(
            sha256(&a56),
            "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a"
        );
        let a64: Vec<u8> = vec![b'a'; 64];
        assert_eq!(
            sha256(&a64),
            "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb"
        );
        let a65: Vec<u8> = vec![b'a'; 65];
        assert_eq!(
            sha256(&a65),
            "635361c48bb9eab14198e76ea8ab7f1a41685d6ad62aa9146d301d4f17eb0ae0"
        );
    }

    /// 输出形状不变量：恒 64 位、全小写 hex（Saltline 旧档 checksum 逐字节可验
    /// 的前提——大小写或长度漂移会让旧档读档直接 CHECKSUM 报错）。
    #[test]
    fn output_is_always_64_lowercase_hex() {
        for size in [0usize, 1, 31, 63, 64, 65, 128, 4096] {
            let bytes: Vec<u8> = (0..size).map(|index| (index % 251) as u8).collect();
            let hex = sha256(&bytes);
            assert_eq!(hex.len(), 64, "len={size}");
            assert!(
                hex.bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
                "len={size} 出现非小写 hex：{hex}"
            );
        }
    }
}
