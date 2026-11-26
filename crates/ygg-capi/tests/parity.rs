//! 三载体等价（C 面）：同一份脚本 + 同一份包镜像，Rust [`Session`] 与 C ABI 的
//! 回执帧流必须**逐字节相同**（规范化重序列化后比较，顺序参与比较）。
//!
//! 脚本是 `packages/schema/protocol/parity-script.json`（编译期嵌入）：
//! hello → ready、submit → delta、step → events + delta、snapshot → reply。
//! `crates/ygg-wasm/tests/parity.rs` 跑同一份脚本的 WASM 载体——三条载体同一份
//! 字节进、同一份字节出，才谈得上「绑定层没有协议逻辑」。
//!
//! `unsafe` 只出现在 C 面调用处：`// SAFETY:` 说的都是同一件事——指针来自本库
//! 的返回值，且所有权按头文件的约定交还。

use std::ffi::{CStr, CString};

use serde_json::Value as Json;

use ygg_capi::{
    ygg_session_free, ygg_session_handle, ygg_session_new, ygg_session_t, ygg_string_free,
};
use ygg_host::Session;

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

/// C 载体：`ygg_session_new` / `ygg_session_handle` / 两个 free。
fn c_stream(image: &[u8], script: &[String]) -> Vec<String> {
    // SAFETY: `image` 的指针与长度来自同一个 Vec，调用期间 Vec 存活且未被改写。
    let session: *mut ygg_session_t = unsafe { ygg_session_new(image.as_ptr(), image.len()) };
    assert!(
        !session.is_null(),
        "C 载体装载失败（ygg_last_error 见 ffi 用例）"
    );
    let mut out = Vec::with_capacity(script.len());
    for msg in script {
        let text = CString::new(msg.as_str()).expect("脚本消息不含 NUL");
        // SAFETY: `session` 是上一行的句柄且尚未释放；`text` 是 NUL 结尾的 C 字符串。
        let reply = unsafe { ygg_session_handle(session, text.as_ptr()) };
        assert!(!reply.is_null(), "ygg_session_handle 分配失败");
        // SAFETY: 返回值由本库分配、NUL 结尾，且只在这里读一次、读后立即交还。
        out.push(
            unsafe { CStr::from_ptr(reply) }
                .to_str()
                .expect("回执是 UTF-8")
                .to_owned(),
        );
        // SAFETY: 同一枚指针，所有权交还一次（头文件约定的唯一释放方式）。
        unsafe { ygg_string_free(reply) };
    }
    // SAFETY: 同一句柄，所有权交还一次；此后不再使用。
    unsafe { ygg_session_free(session) };
    out
}

/// 规范化重序列化：解析成 JSON 再写回（`serde_json::Value` 的 Map 是 `BTreeMap`，
/// 键序确定）。两边的字节如果只在键序/空白上不同，这里会把差异抹平——数组顺序
/// 与数值不变，所以「帧序不同」仍然会被抓到。
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
                .map(|frame| frame["type"].as_str().expect("帧有 type 字段").to_owned())
                .collect()
        })
        .collect()
}

#[test]
fn c_abi_and_rust_session_agree_frame_for_frame() {
    let fixture: Json = serde_json::from_str(FIXTURE).expect("parity-script.json 可解析");
    let image = image_bytes(&fixture);
    let hash = Session::from_image(&image)
        .expect("镜像可装载")
        .loaded()
        .manifest_hash()
        .to_owned();
    let script = script(&fixture, &hash);

    let rust = rust_stream(&image, &script);
    let c = c_stream(&image, &script);

    assert_eq!(rust.len(), script.len(), "一条消息一段回执");
    assert_eq!(c.len(), rust.len(), "两条载体回执段数相同");
    assert_eq!(
        canon(&c),
        canon(&rust),
        "C ABI 与 Rust 会话的回执帧流必须逐帧相同"
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
    assert_eq!(types_of(&c), expected, "C 载体的面与 Rust 载体一致");
}
