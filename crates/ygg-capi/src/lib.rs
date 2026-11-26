//! ygg-capi：`ygg/1` 协议的 **C ABI**（`cdylib` + `staticlib`）。
//!
//! 契约：`artifacts/w2-m3-protocol-plan.md` §3 步 3；字段面见
//! `packages/schema/protocol/README.md`。头文件：`include/ygg.h`（与[`ffi`]的导出面
//! 逐字一致，`Cargo.toml` 的 `cdylib` 产物与它配套发布）。
//!
//! # 六个导出符号
//!
//! | 符号 | 面 |
//! | --- | --- |
//! | `ygg_protocol_version()` | `"ygg/1"`（静态区，**不要释放**） |
//! | `ygg_session_new(pack, pack_len)` | 包镜像 → 未握手会话（NULL + `ygg_last_error()`） |
//! | `ygg_session_handle(s, msg_json)` | 一段 JSON 进、一帧或多帧 JSON 数组出 |
//! | `ygg_session_free(s)` | 释放会话 |
//! | `ygg_string_free(s)` | 释放本库返回的任一字符串 |
//! | `ygg_last_error()` | 最近一次「只能返回 NULL」的原因（可释放） |
//!
//! 边界上**没有结构体过境**：没有 out 参数、没有回调、没有指针数组——一次进一段
//! 字符缓冲区，一次出一段字符缓冲区。这条约束不是为了省事：结构体一过境，语言 ABI
//! 差异（对齐、位域、`bool` 宽度、`size_t` 宽度）就会在宿主的第二个平台上变成
//! 「协议对不上」。
//!
//! # 三条边界规则
//!
//! 1. **进的是 UTF-8 字节，出的是调用方所有的 NUL 结尾字符串**——用完一律
//!    [`ygg_string_free`]，不要 `free(3)`（分配器必须成对）。
//! 2. **协议错误不用 NULL 表达**：会话自己在回执里答一帧 `error{code,msg}`
//!    （`E_PROTOCOL` / `E_PACK` / `E_RULE` / `E_STATE` / `E_BUDGET`），NULL 只留给
//!    「分配不出来」与「会话指针为空」——后者也可以答一帧 `error`，本库就是这么做的。
//! 3. **一个会话只在一个线程上用**：句柄内部可变（`&mut Session`），并发调用同一
//!    句柄是未定义行为（Rust 的别名规则，不是本库的额外限制）。要并发，一连接一会话。
//!
//! # 为什么这里没有协议逻辑
//!
//! 本 crate 的全部判断只有三类：指针空不空、字节是不是 UTF-8、分配成不成功。
//! 「这条消息该不该接受」「世界有没有 seal」都是内核面（`ygg-host`）的事——绑定层
//! 每加一个 `if`，就等于多一份与 Rust 宿主对不上的协议实现，而三份实现会在第一个
//! 边角用例上分叉（`packages/schema/protocol/parity-script.json` 就是来钉这件事的）。

mod ffi;

pub use ffi::{
    ygg_last_error, ygg_protocol_version, ygg_session_free, ygg_session_handle, ygg_session_new,
    ygg_session_t, ygg_string_free,
};
