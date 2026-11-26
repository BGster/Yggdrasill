//! 三载体等价（WASM 面）：同一份脚本 + 同一份包镜像，wasm 壳 `YggSession` 与
//! Rust [`Session`] 的回执帧流必须**逐字节相同**（规范化重序列化后比较，顺序参与
//! 比较）。C 面在 `crates/ygg-capi/tests/parity.rs` 跑同一份脚本——三条载体同一份
//! 字节进、同一份字节出，才谈得上「绑定层没有协议逻辑」。
//!
//! 脚本是 `packages/schema/protocol/parity-script.json`（编译期嵌入）：
//! hello → ready、submit → delta、step → events + delta、snapshot → reply。
//!
//! 本文件在 **host 目标**上跑（`cargo test -p ygg-wasm`）：wasm-bindgen 在 host 上
//! 同构编译，`handle`/`snapshot` 的透传面与 wasm32 上同一份代码。

use serde_json::Value as Json;

use ygg_host::Session;
use ygg_wasm::YggSession;

/// 三载体共用脚本（同一份文件；这里编译期嵌入，运行时不依赖 cwd）。
const FIXTURE: &str = include_str!("../../../packages/schema/protocol/parity-script.json");

/// 包镜像字节（fixture 的 `image` 字段原样序列化）。
fn image_bytes(fixture: &Json) -> Vec<u8> {
    serde_json::to_vec(&fixture["image"]).expect("fixture.image 是 JSON")
}

/// 脚本消息（`<manifest-hash>` 占位符用载体自己算出的指纹替换）。
fn script(fixture: &Json, manifest_hash: &str) -> Vec<String> {
    fixture["script"]
        .as_array()
        .expect("fixture.script 是数组")
        .iter()
        .map(|item| {
            item.as_str()
                .expect("脚本条目是字符串")
                .replace("<manifest-hash>", manifest_hash)
        })
        .collect()
}

/// Rust 载体：`Session::handle_json` 一段进一段出。
fn rust_stream(image: &[u8], script: &[String]) -> Vec<String> {
    let mut session = Session::from_image(image).expect("Rust 载体装载失败");
    script.iter().map(|msg| session.handle_json(msg)).collect()
}

/// wasm 载体：`YggSession::new` / `handle` / `snapshot`。
fn wasm_stream(image: &[u8], script: &[String]) -> Vec<String> {
    let mut session = YggSession::new(image.to_vec()).expect("wasm 载体装载失败");
    script.iter().map(|msg| session.handle(msg)).collect()
}

/// 规范化重序列化：解析成 JSON 再写回（`serde_json::Value` 的 Map 是 `BTreeMap`，
/// 键序确定）。两边只在键序/空白上不同时会被抹平；数组顺序与数值不变，所以
/// 「帧序不同」仍然会被抓到。
fn canon(replies: &[String]) -> Vec<String> {
    replies
        .iter()
        .map(|reply| {
            let value: Json = serde_json::from_str(reply).expect("回执是 JSON");
            value.to_string()
        })
        .collect()
}

/// 逐帧 `type` 序列（钉住脚本确实打到了四个面，而不是退化成「都答 error」）。
fn types_of(replies: &[String]) -> Vec<Vec<String>> {
    replies
        .iter()
        .map(|reply| {
            let value: Json = serde_json::from_str(reply).expect("回执是 JSON");
            value
                .as_array()
                .expect("回执是一个 JSON 数组")
                .iter()
                .map(|frame| frame["type"].as_str().expect("帧有 type").to_owned())
                .collect()
        })
        .collect()
}

#[test]
fn wasm_shell_and_rust_session_agree_frame_for_frame() {
    let fixture: Json = serde_json::from_str(FIXTURE).expect("parity-script.json 可解析");
    let image = image_bytes(&fixture);
    let hash = Session::from_image(&image)
        .expect("镜像可装载")
        .loaded()
        .manifest_hash()
        .to_owned();
    let script = script(&fixture, &hash);

    let rust = rust_stream(&image, &script);
    let wasm = wasm_stream(&image, &script);

    assert_eq!(rust.len(), script.len(), "一条消息一段回执");
    assert_eq!(wasm.len(), rust.len(), "两条载体回执段数相同");
    assert_eq!(
        canon(&wasm),
        canon(&rust),
        "wasm 壳与 Rust 会话的回执帧流必须逐帧相同"
    );

    let expected: Vec<Vec<String>> = fixture["expected_types"]
        .as_array()
        .expect("expected_types 是数组")
        .iter()
        .map(|row| {
            row.as_array()
                .expect("expected_types 每行是数组")
                .iter()
                .map(|name| name.as_str().expect("帧名是字符串").to_owned())
                .collect()
        })
        .collect();
    assert_eq!(types_of(&rust), expected, "脚本打到的面与 fixture 声明一致");
    assert_eq!(types_of(&wasm), expected, "wasm 载体的面与 Rust 载体一致");
}
