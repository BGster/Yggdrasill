//! C 面的边界行为：空指针、坏输入、装载失败、版本常量、释放。
//!
//! 这些用例只问「边界会不会骗人」：协议语义（状态门、帧切分、错误分类）由
//! `crates/ygg-host` 的用例与 `tests/parity.rs` 钉住。每条 `unsafe` 的
//! `// SAFETY:` 都在说明「这枚指针是谁给的、什么时候还」。

use std::ffi::{c_char, CStr, CString};

use serde_json::Value as Json;

use ygg_capi::{
    ygg_last_error, ygg_protocol_version, ygg_session_free, ygg_session_handle, ygg_session_new,
    ygg_session_t, ygg_string_free,
};

/// 空转包镜像：不声明组件、不生成实体（装载必然成功）。
const IDLE_IMAGE: &[u8] = br#"{"modSource":"-- idle mod: no world declarations","manifest":""}"#;

/// 取一枚本库返回的字符串并交还所有权。
fn take(ptr: *mut c_char) -> String {
    assert!(!ptr.is_null(), "本库返回了 NULL（不该在这里发生）");
    // SAFETY: 指针由本库分配、NUL 结尾；读一次后立即交还，不再使用。
    let text = unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .expect("回执是 UTF-8")
        .to_owned();
    // SAFETY: 同一枚指针，所有权交还一次。
    unsafe { ygg_string_free(ptr) };
    text
}

/// 回执的帧类型列表（顺带断言「回执是一个 JSON 数组」）。
fn frame_types(reply: &str) -> Vec<String> {
    let value: Json = serde_json::from_str(reply).expect("回执是 JSON");
    value
        .as_array()
        .expect("回执是一个 JSON 数组")
        .iter()
        .map(|frame| frame["type"].as_str().expect("帧有 type").to_owned())
        .collect()
}

/// 回执里第一帧的错误码（没有 `error` 帧时返回 `None`）。
fn error_code(reply: &str) -> Option<String> {
    let value: Json = serde_json::from_str(reply).expect("回执是 JSON");
    value
        .as_array()?
        .iter()
        .find(|frame| frame["type"] == "error")?
        .get("code")?
        .as_str()
        .map(str::to_owned)
}

#[test]
fn version_is_the_protocol_constant() {
    // SAFETY: 静态区指针，本库保证 NUL 结尾且不过期。
    let text = unsafe { CStr::from_ptr(ygg_protocol_version()) }
        .to_str()
        .expect("版本串是 UTF-8");
    assert_eq!(text, ygg_host::PROTOCOL);
    assert_eq!(text, "ygg/1");
}

#[test]
fn new_answers_null_with_a_reason_when_the_image_is_broken() {
    let broken = b"not json at all";
    // SAFETY: `broken` 的指针与长度来自同一数组，调用期间存活。
    let session = unsafe { ygg_session_new(broken.as_ptr(), broken.len()) };
    assert!(session.is_null(), "坏镜像必须得到 NULL");
    let reason = take(ygg_last_error());
    assert!(
        reason.contains("JSON"),
        "ygg_last_error 应指出镜像不是 JSON：{reason}"
    );
}

#[test]
fn new_accepts_a_zero_length_image_only_as_a_loading_error() {
    // 长度 0 + 空指针不是 UB：装载失败，NULL + 原因（而不是崩溃）。
    // SAFETY: `pack_len == 0` 时 `pack` 允许为 NULL（头文件约定）。
    let session = unsafe { ygg_session_new(std::ptr::null(), 0) };
    assert!(session.is_null());
    assert!(!take(ygg_last_error()).is_empty(), "失败必须留下原因");
}

#[test]
fn bad_message_gets_an_error_frame_instead_of_null() {
    // SAFETY: 镜像指针/长度来自同一常量，调用期间存活。
    let session: *mut ygg_session_t =
        unsafe { ygg_session_new(IDLE_IMAGE.as_ptr(), IDLE_IMAGE.len()) };
    assert!(!session.is_null(), "空转镜像必须装载成功");
    let msg = CString::new("this is not json").expect("文案不含 NUL");
    // SAFETY: 句柄尚未释放；`msg` 是 NUL 结尾的 C 字符串。
    let reply = unsafe { ygg_session_handle(session, msg.as_ptr()) };
    let text = take(reply);
    assert_eq!(frame_types(&text), ["error"], "坏 JSON 答一帧 error");
    assert_eq!(
        error_code(&text).as_deref(),
        Some("E_PROTOCOL"),
        "坏 JSON 归协议错误（宿主自己的 bug）"
    );
    // SAFETY: 同一句柄，所有权交还一次；此后不再使用。
    unsafe { ygg_session_free(session) };
}

#[test]
fn null_session_still_answers_a_parseable_frame() {
    let msg = CString::new("{}").expect("文案不含 NUL");
    // SAFETY: 空句柄是本用例的被测输入（头文件允许传 NULL，答案是一帧 E_STATE）。
    let reply = unsafe { ygg_session_handle(std::ptr::null_mut(), msg.as_ptr()) };
    let text = take(reply);
    assert_eq!(frame_types(&text), ["error"]);
    assert_eq!(error_code(&text).as_deref(), Some("E_STATE"));
}

#[test]
fn non_utf8_message_is_a_protocol_error() {
    // SAFETY: 镜像指针/长度来自同一常量，调用期间存活。
    let session = unsafe { ygg_session_new(IDLE_IMAGE.as_ptr(), IDLE_IMAGE.len()) };
    assert!(!session.is_null());
    let raw = CString::new(vec![0xffu8, 0xfe, 0x7b]).expect("无 NUL 字节");
    // SAFETY: 句柄尚未释放；`raw` 是 NUL 结尾、字节可读（只是不是 UTF-8）。
    let reply = unsafe { ygg_session_handle(session, raw.as_ptr()) };
    let text = take(reply);
    assert_eq!(error_code(&text).as_deref(), Some("E_PROTOCOL"));
    // SAFETY: 同一句柄，所有权交还一次。
    unsafe { ygg_session_free(session) };
}

#[test]
fn free_accepts_null() {
    // SAFETY: 头文件写明 NULL 允许（空操作）。
    unsafe { ygg_session_free(std::ptr::null_mut()) };
    // SAFETY: 同上。
    unsafe { ygg_string_free(std::ptr::null_mut()) };
}
