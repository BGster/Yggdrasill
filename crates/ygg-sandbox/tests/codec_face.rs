//! `ygg.codec.*` 原生面的**绝对值**门禁。
//!
//! M2.5 删掉 `pkg.codec` 的 Lua 实现后，本文件原先那套「原生 ↔ Lua 逐字节差分」
//! 失去了对照物：差分要求两份实现同时在场，只剩一份时它只能自比，不能自证。
//! 因此留下的是原生面自己的判据：
//!
//! - **键集闭集**：`ygg.codec` 恰好 `encode` / `decode` / `sha256` 三个键，
//!   没有逐节点 getter；
//! - **KAT**：FIPS 180-4 附录 B 的发布向量逐字节对上（不是「两条路径互相比」）；
//! - **拒绝面**：坏档与非字符串一律落在 `YGG-CODEC-UNSUPPORTED`；
//! - **规范序**：键序跟着 `crates/ygg-sim/fixtures/canonical-order.json` 的
//!   `expected` 走——那份 fixture 的另一个消费者是
//!   `crates/ygg-sim/tests/canonical_order.rs`，两边都等于同一份契约序。
use mlua::{Function, Lua, Table, Value};
use std::path::Path;
/// 装出一个只有 `ygg.codec` 的状态（不装任何 Lua 侧实现）。
fn build() -> Lua {
    let lua = Lua::new();
    let ygg = lua.create_table().expect("建 ygg 表");
    lua.globals().raw_set("ygg", ygg).expect("挂 ygg");
    ygg_sandbox::codec_face::install(&lua).expect("装 ygg.codec");
    lua
}
/// 原生径：直接调面函数（不经包的分派——这样「合法值被原生误拒」才是硬错）。
fn native_encode(value: &Value) -> Result<Vec<u8>, ygg_sandbox::codec_face::CodecFaceError> {
    ygg_sandbox::codec_face::encode(value)
}
/// 面径的 `ygg.codec.sha256`（走 mlua 包装，原始字节进）。
fn face_sha256(lua: &Lua, bytes: &[u8]) -> Result<String, mlua::Error> {
    let function: Function = lua
        .load("return function(b) return ygg.codec.sha256(b) end")
        .eval()?;
    let text = lua.create_string(bytes)?;
    function.call(text)
}
#[test]
fn key_set_is_exactly_encode_decode_and_sha256() {
    let lua = build();
    let ygg: Table = lua.globals().get("ygg").expect("ygg 表");
    let face: Table = ygg.get("codec").expect("ygg.codec 表");
    let mut keys: Vec<String> = face
        .pairs::<String, Value>()
        .map(|pair| pair.expect("键值对").0)
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        vec![
            "decode".to_string(),
            "encode".to_string(),
            "sha256".to_string()
        ],
        "ygg.codec 的键集是闭集（契约 §3.1.4：恰好 3 个，没有逐节点 getter）"
    );
}
/// FIPS 180-4 附录 B 的发布值：差分测的是相等，这条钉的是绝对值。
#[test]
fn sha256_known_answer_vectors() {
    let lua = build();
    let vectors: &[(&str, &str)] = &[
        (
            "",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (
            "abc",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        (
            "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
        ),
    ];
    for (input, expected) in vectors {
        let face = face_sha256(&lua, input.as_bytes()).expect("面径");
        assert_eq!(&face, expected, "面径 KAT（input={input:?}）");
    }
}
/// 面的自保分支：非字符串进面 ⇒ `YGG-CODEC-UNSUPPORTED`。
#[test]
fn sha256_rejects_non_strings() {
    let lua = build();
    let message: String = lua
        .load(
            r#"
            local ok, err = pcall(ygg.codec.sha256, 42)
            if ok then error("面必须拒绝数字", 0) end
            return tostring(err)
            "#,
        )
        .eval()
        .expect("面径拒绝");
    assert!(
        message.contains("YGG-CODEC-UNSUPPORTED"),
        "面径错误码必须是 UNSUPPORTED：{message}"
    );
}
#[test]
fn malformed_documents_are_rejected_by_the_native_path() {
    let lua = build();
    let bad: &[&[u8]] = &[
        b"",
        b"[1,2",
        b"{\"a\":1",
        b"{\"a\" 1}",
        b"{a:1}",
        b"[1 2]",
        b"1 2",
        b"tru",
        b"1.",
        b"1e",
        b"\"\\q\"",
        b"\"\\u12\"",
        b"{\"a\":null,\"b\":[null]}",
    ];
    for text in bad {
        let error = ygg_sandbox::codec_face::decode(&lua, text).expect_err("坏档必须被原生拒");
        assert_eq!(
            error.code(),
            ygg_sandbox::codec_face::CODEC_UNSUPPORTED,
            "{}",
            String::from_utf8_lossy(text)
        );
    }
}
/// 规范序 fixture 的消费者之一（另一个在 `crates/ygg-sim/tests/canonical_order.rs`）。
///
/// 判据是「等于 fixture 的 `expected`」，而不是「两条路径互相比」——后者只能
/// 证明两者一致，不能证明它们等于契约那条序。
#[test]
fn codec_key_order_follows_the_canonical_order_fixture() {
    let lua = build();
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/ygg-sim/fixtures/canonical-order.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读 {} 失败：{error}", path.display()));
    let fixture: serde_json::Value = serde_json::from_str(&raw).expect("fixture 是 JSON");
    let names: Vec<&str> = fixture["names"]
        .as_array()
        .expect("names 数组")
        .iter()
        .map(|item| item.as_str().expect("名字"))
        .collect();
    let expected: Vec<&str> = fixture["expected"]
        .as_array()
        .expect("expected 数组")
        .iter()
        .map(|item| item.as_str().expect("名字"))
        .collect();
    // 洗序插入（fixture 的 names 本身就是乱序），输出必须仍是契约序
    let table = lua.create_table().expect("建表");
    for (index, name) in names.iter().enumerate() {
        table.set(*name, index as i64 + 1).expect("写键");
    }
    let bytes = native_encode(&Value::Table(table)).expect("合法表");
    let text = String::from_utf8(bytes).expect("键都是 UTF-8");
    // 输出形如 {"0":..,"9":..}：按键字面量在输出里的位置排序，得到出现序
    let mut seen: Vec<(usize, &str)> = names
        .iter()
        .filter_map(|name| {
            text.find(&format!("\"{name}\":"))
                .map(|position| (position, *name))
        })
        .collect();
    seen.sort_by_key(|(position, _)| *position);
    let seen: Vec<&str> = seen.into_iter().map(|(_, name)| name).collect();
    assert_eq!(
        seen, expected,
        "原生径的键序必须等于 fixture 的期望序（codec.md §4.1 / sim-kernel §2）"
    );
}
