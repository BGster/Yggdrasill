//! C ABI 实装：**只做搬运**（解析指针 / 分配 / 归还）。
//!
//! 这是本 workspace 里唯一允许出现 `unsafe` 的文件（`#![forbid(unsafe_code)]` 在其余
//! crate 与其余模块上照旧生效）。每处 `unsafe` 块都带 `// SAFETY:`，说的是同一件事：
//! **谁保证这个指针/长度在本次调用期间有效，以及所有权什么时候回到调用方。**
//!
//! 判断面只有三类——指针为空、字节不是 UTF-8、分配失败。消息语义一律进
//! [`ygg_host::Session::handle_json`]：它进什么出什么，本文件不知道里面有几帧。

use std::cell::RefCell;
use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;

use ygg_host::{encode_frames, ErrorCode, Message, ProtocolError, Session};

/// 不透明会话句柄（C 侧只见 `ygg_session_t *`；内部持有一条 `ygg/1` 会话）。
///
/// 名字跟随头文件的 `ygg_session_t` 而不是 Rust 的驼峰：这是导出面的符号名，
/// 两边不一致就得在文档里维护一张对照表。
#[allow(non_camel_case_types)]
pub struct ygg_session_t {
    inner: Session,
}

/// `ygg_protocol_version()` 的静态返回面（NUL 结尾，**不要释放**）。
static PROTOCOL_VERSION: &[u8] = b"ygg/1\0";

thread_local! {
    /// 最近一次「只能返回 NULL」的失败原因：每线程一份（C 侧不共享线程状态）。
    /// 只由本库写入，读取不消耗（`ygg_last_error` 每次给一份新的副本）。
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

/// 协议版本面：`"ygg/1"`。
///
/// 返回静态区指针：**不要释放**、也不要改写它（C 侧 `const char *`）。
#[no_mangle]
pub extern "C" fn ygg_protocol_version() -> *const c_char {
    PROTOCOL_VERSION.as_ptr().cast()
}

/// 包镜像 → 未握手会话。
///
/// `pack` 是镜像的 JSON 文本（形状见 `ygg_host::image`）；`pack_len` 是字节数
/// （**不必** NUL 结尾）。成功返回句柄（用 `ygg_session_free` 释放）；失败返回
/// NULL，原因见 `ygg_last_error()`。
///
/// # Safety
///
/// `pack` 必须指向 `pack_len` 个可读字节（`pack_len == 0` 时允许为 NULL），且这些
/// 字节在本次调用期间保持有效——本函数只在调用内借用它们。
#[no_mangle]
pub unsafe extern "C" fn ygg_session_new(pack: *const u8, pack_len: usize) -> *mut ygg_session_t {
    let image: &[u8] = if pack_len == 0 {
        &[]
    } else if pack.is_null() {
        set_last_error("ygg_session_new: pack 为空但 pack_len 非零");
        return ptr::null_mut();
    } else {
        // SAFETY: 调用方保证 `pack` 指向 `pack_len` 个可读字节（见函数文档），
        // 且这些字节在本次调用期间有效；切片不越过调用边界。
        unsafe { std::slice::from_raw_parts(pack, pack_len) }
    };
    // 内核装载不 panic（错误都是 `Result`），但一个 panic 跨 `extern "C"` 是 UB：
    // 兜一层，把「不该发生的事」变成可诊断的 NULL。
    match catch_unwind(AssertUnwindSafe(|| Session::from_image(image))) {
        Ok(Ok(session)) => Box::into_raw(Box::new(ygg_session_t { inner: session })),
        Ok(Err(text)) => {
            set_last_error(&text);
            ptr::null_mut()
        }
        Err(_) => {
            set_last_error("ygg_session_new: 内核 panic（装载）");
            ptr::null_mut()
        }
    }
}

/// 一条消息进，帧流出一：回执是**一个 JSON 数组**（`step` 两帧，其余一帧）。
///
/// 返回值是调用方所有的新字符串，用完必须 `ygg_string_free`（不要 `free(3)`）。
/// 坏 JSON / 语义错都不返回 NULL，而是答一帧 `error{code,msg}`；NULL 只在分配失败
/// （或会话指针为空，此时本函数仍然答一帧 `error{E_STATE}`——见实现）。
///
/// # Safety
///
/// `s` 必须是 `ygg_session_new` 返回且尚未释放的指针；`msg_json` 必须是 NUL 结尾的
/// 可读 C 字符串。同一句柄不得并发进入本函数（内部是 `&mut Session`）。
#[no_mangle]
pub unsafe extern "C" fn ygg_session_handle(
    s: *mut ygg_session_t,
    msg_json: *const c_char,
) -> *mut c_char {
    if s.is_null() {
        // 没有会话可问，但「会话不存在」在协议里是一类明确的答案（`E_STATE`）：
        // 调用方拿到的仍是可解析的帧流，不必为 NULL 单开一条分支。
        return into_c_string(&error_frames(
            ErrorCode::EState,
            "ygg_session_handle: 会话指针为空",
        ));
    }
    if msg_json.is_null() {
        return into_c_string(&error_frames(
            ErrorCode::EProtocol,
            "ygg_session_handle: msg_json 指针为空",
        ));
    }
    // SAFETY: 调用方保证 `msg_json` 是 NUL 结尾的可读 C 字符串（见函数文档）。
    let bytes = unsafe { CStr::from_ptr(msg_json) };
    let Ok(text) = bytes.to_str() else {
        // 线协议是 UTF-8（`ygg/1` 只有 JSON）：非 UTF-8 是调用方的编码错误。
        return into_c_string(&error_frames(
            ErrorCode::EProtocol,
            "ygg_session_handle: msg_json 不是 UTF-8",
        ));
    };
    // SAFETY: `s` 由 `ygg_session_new` 返回且未被释放（见函数文档）；本调用独占该
    // 会话（并发进入同一句柄是调用方的 UB，README 与头文件都写明单线程）。
    let session = unsafe { &mut *s };
    let reply = match catch_unwind(AssertUnwindSafe(|| session.inner.handle_json(text))) {
        Ok(reply) => reply,
        Err(_) => error_frames(ErrorCode::EState, "ygg_session_handle: 内核 panic"),
    };
    into_c_string(&reply)
}

/// 释放本库返回的字符串（`ygg_session_handle` / `ygg_last_error` 的返回值）。
///
/// NULL 允许（等价于空操作）。
///
/// # Safety
///
/// `s` 必须是本库某次返回的指针，且只交还一次；不要传静态区指针
/// （`ygg_protocol_version()` 的返回值**不属于**本函数的适用范围）。
#[no_mangle]
pub unsafe extern "C" fn ygg_string_free(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    // SAFETY: 指针来自 `CString::into_raw`（本库唯一的所有权出口），调用方只交还一次。
    drop(unsafe { CString::from_raw(s) });
}

/// 释放会话句柄（`ygg_session_new` 的返回值）。NULL 允许。
///
/// # Safety
///
/// `s` 必须是 `ygg_session_new` 返回且**尚未**释放的指针；释放后再用是 UB。
#[no_mangle]
pub unsafe extern "C" fn ygg_session_free(s: *mut ygg_session_t) {
    if s.is_null() {
        return;
    }
    // SAFETY: 同一枚 `Box::into_raw` 的所有权只交还一次（见函数文档）。
    drop(unsafe { Box::from_raw(s) });
}

/// 最近一次「只能返回 NULL」的失败原因（当前线程）。
///
/// 返回值是调用方所有的新字符串（`ygg_string_free` 释放）；当前线程没有记录时答
/// NULL。成功路径**不会**清空它：NULL 之后先读这一句，就知道为什么。
#[no_mangle]
pub extern "C" fn ygg_last_error() -> *mut c_char {
    LAST_ERROR.with(|slot| match slot.borrow().as_ref() {
        Some(text) => into_c_string(text.to_str().unwrap_or("（错误文案含非 UTF-8 字节）")),
        None => ptr::null_mut(),
    })
}

// -------------------------------------------------------------------------- 搬运

/// 协议错误 → 帧流 JSON（单帧 `error`）：走内核同一台 encoder，本文件不拼 JSON。
fn error_frames(code: ErrorCode, msg: &str) -> String {
    encode_frames(&[Message::Error(ProtocolError::new(code, msg.to_owned()))])
}

/// Rust 字符串 → 调用方所有的 C 字符串（NULL = 分配失败，原因进 `LAST_ERROR`）。
fn into_c_string(text: &str) -> *mut c_char {
    match CString::new(text) {
        Ok(owned) => owned.into_raw(),
        Err(_) => {
            set_last_error("返回值含 NUL 字节，无法过 C 边界");
            ptr::null_mut()
        }
    }
}

/// 记录失败原因（`CString::new` 内部的 NUL 已由调用方保证不存在；再兜一次）。
fn set_last_error(text: &str) {
    let Ok(owned) = CString::new(text) else {
        return;
    };
    LAST_ERROR.with(|slot| *slot.borrow_mut() = Some(owned));
}
