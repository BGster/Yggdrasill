//! ygg-wasm：`ygg/1` 协议的 WASM 绑定（wasm-bindgen）——**只搬字节，不碰语义**。
//!
//! # 导出面（一个类、三个方法）
//!
//! ```text
//! new YggSession(pack_bytes)   // 包镜像（JSON 文本的字节）→ 未握手会话；装载失败 throw
//! session.handle(msg_json)     // 一段 JSON → 一个 JSON 帧数组；永不 throw
//! session.snapshot()           // 世界帧字节（Uint8Array）；失败答空数组
//! ```
//!
//! - **镜像**：`pack_bytes` 是包镜像的 JSON 文本（wasm 侧没有可读的「目录」，四件
//!   装载参数只能随字节一起过界），形状见 [`ygg_host::image`]：
//!   `{"modSource":"<Lua>","manifest":"<manifest.json 文本；空串 = 无资源>",
//!     "schemas":[…],"resources":{…}}`。
//! - **协议**：[`packages/schema/protocol/README.md`] 的 ygg/1 消息表。`handle` 的
//!   回执是**一个 JSON 数组**（`step` 两帧 `events`+`delta`，其余一帧）：先在数组里
//!   遍历，再按 `type` 分派——与 C ABI / Rust 侧同一套帧序。
//! - **永不 throw**：`handle` 的每条失败路径都是一帧 `error{code,msg}`（`E_PROTOCOL`
//!   宿主自己的 bug、`E_PACK` 换包、`E_RULE`/`E_BUDGET` 报给包作者、`E_STATE` 回滚重试）。
//!   JS 侧不需要 `try/catch` 才能继续下一步：错也是协议的一部分。
//! - **装载失败 throw**：镜像不在协议面上（它是装载参数，不是线上消息），所以构造
//!   函数给出的是 `Error`（文案来自内核）。
//!
//! # 这里没有协议逻辑
//!
//! 「这条消息该不该接受」「世界 seal 了没有」「`pack_hash` 对不对」全在
//! `crates/ygg-host`：本文件只有三行 `self.inner.<method>`。绑定层每加一个 `if`，
//! 就多一份与 Rust 宿主对不上的协议实现——`tests/parity.rs` 与
//! `crates/ygg-capi/tests/parity.rs` 用同一份脚本（[`packages/schema/protocol/parity-script.json`]）
//! 把三条载体钉在同一段字节上。
//!
//! # wasm32 构建链
//!
//! （mlua-rs/mlua docs/wasm32 recipe，mlua@ff96ed7 `tests/wasm` 同型）：[`wasi_alloc`]
//! 模块提供委托 WASI libc 的全局分配器（Rust 与 wasi-libc 必须共享同一线性内存
//! 管理）；JS 宿主须提供 WASI preview-1 import shim（本 crate `wasi-shim.js`，
//! init 前安装）。构建单一入口 `scripts/build-wasm.sh`（两步链：cargo build →
//! wasm-bindgen `--reference-types`；wasm-pack 0.15 不透传该旗，而 EH 模块的 catch
//! wrappers 硬要求 externref 表）。Lua 5.4 的错误处理走 C setjmp 面（wasi-libc EH
//! 变体），无需 C++ 异常旗。
//!
//! host 目标（rlib，`cargo test --workspace` 门禁面）下同构编译：分配器等 wasm
//! 专属面全部 `cfg(target_arch = "wasm32")` 门控；[`YggSession`] 的透传面在 host 上
//! 可测（`tests` 模块 + `tests/parity.rs`）。
//!
//! [`packages/schema/protocol/README.md`]: ../../../packages/schema/protocol/README.md
//! [`packages/schema/protocol/parity-script.json`]: ../../../packages/schema/protocol/parity-script.json

#[cfg(target_arch = "wasm32")]
mod wasi_alloc {
    use std::alloc::{GlobalAlloc, Layout};
    use std::ffi::c_void;

    /// 委托 WASI libc 的全局分配器（mlua tests/wasm/src/lib.rs 同型）：
    /// Rust 与 wasi-libc 共享同一分配器，避免双方面独立管理同一线性内存。
    struct WasiAllocator;

    unsafe extern "C" {
        fn posix_memalign(ptr: *mut *mut c_void, align: usize, size: usize) -> i32;
        fn free(ptr: *mut c_void);
    }

    unsafe impl GlobalAlloc for WasiAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let mut ptr = std::ptr::null_mut();
            // 对齐下限 = usize（wasi-libc posix_memalign 语义）；失败返回 null。
            if unsafe {
                posix_memalign(
                    &mut ptr,
                    layout.align().max(size_of::<usize>()),
                    layout.size(),
                )
            } != 0
            {
                return std::ptr::null_mut();
            }
            ptr.cast()
        }

        unsafe fn dealloc(&self, ptr: *mut u8, _: Layout) {
            unsafe { free(ptr.cast()) };
        }
    }

    #[global_allocator]
    static ALLOCATOR: WasiAllocator = WasiAllocator;
}

use wasm_bindgen::prelude::*;

use ygg_host::Session;

/// wasm 壳的会话句柄：持有一条 [`Session`]（语义面全部委托）。
#[wasm_bindgen]
pub struct YggSession {
    inner: Session,
}

#[wasm_bindgen]
impl YggSession {
    /// 包镜像 → 未握手会话（镜像形状见模块文档）。
    ///
    /// 装载失败才 throw（镜像不在协议面上，没有帧可答）；握手之后的一切失败都由
    /// [`handle`](Self::handle) 的 `error` 帧表达。
    #[wasm_bindgen(constructor)]
    pub fn new(pack_bytes: Vec<u8>) -> Result<YggSession, JsValue> {
        match Session::from_image(&pack_bytes) {
            Ok(inner) => Ok(Self { inner }),
            Err(text) => Err(throw_value(&text)),
        }
    }

    /// 一条消息 JSON → 帧流 JSON（一个数组：`step` 两帧，其余一帧）。
    ///
    /// 永远给答案：解析失败 / 状态不符 / 包指纹不符都答一帧 `error`，不 throw。
    pub fn handle(&mut self, msg_json: &str) -> String {
        self.inner.handle_json(msg_json)
    }

    /// 当前世界帧字节（`snapshot` 消息的字节面；协议入口仍是 `snapshot` 消息）。
    ///
    /// 失败答空数组——要「为什么失败」就走 `snapshot` 消息（那里有 `error` 帧），
    /// 这条只是给「我要一份存档字节」的宿主省一次 base64 往返。
    pub fn snapshot(&self) -> Vec<u8> {
        self.inner.snapshot_bytes().unwrap_or_default()
    }
}

/// throw 值（wasm 侧是 JS 字符串；host 无 JS 值空间——`JsValue::from_str`
/// 在 host 上 panic，故返回 NULL 占位。文案内容由内核侧纯函数单测覆盖）。
fn throw_value(text: &str) -> JsValue {
    #[cfg(target_arch = "wasm32")]
    {
        JsValue::from_str(text)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = text;
        JsValue::NULL
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value as Json};

    use super::YggSession;
    use ygg_sandbox::hash::state_hash;

    /// 声明一个组件 + 一个系统并 seal（不 seal 的世界不允许推进时间）。
    const SEALED_MOD: &str = "world.component{ name = \"hp\", kind = \"i64\" }\n\
        world.system{\n  name = \"damage\", stage = \"rules\", reads = {\"hp\"}, writes = {\"hp\"},\n\
        \x20 run = function(dt, ctx)\n    ctx.each_mut({\"hp\"}, function(id, hp) return hp - 1 end)\n  end,\n}\n\
        world.spawn{ hp = 3 }\nworld.seal()";

    fn image(mod_source: &str) -> Vec<u8> {
        json!({ "modSource": mod_source, "manifest": "" })
            .to_string()
            .into_bytes()
    }

    fn open(mod_source: &str) -> YggSession {
        YggSession::new(image(mod_source)).expect("镜像应装载成功")
    }

    /// 握手消息（`pack_hash` 取会话自己的 manifest 指纹——否则答 `E_PACK`）。
    fn hello(session: &YggSession) -> String {
        json!({
            "type": "hello",
            "protocol": ygg_host::PROTOCOL,
            "codec": ygg_host::CODEC,
            "pack_id": "ygg:test",
            "pack_hash": session.inner.loaded().manifest_hash(),
        })
        .to_string()
    }

    /// 回执帧的 `type` 列表（顺带断言「回执是一个 JSON 数组」）。
    fn frame_types(reply: &str) -> Vec<String> {
        let value: Json = serde_json::from_str(reply).expect("回执是 JSON");
        value
            .as_array()
            .expect("回执是一个 JSON 数组")
            .iter()
            .map(|frame| frame["type"].as_str().expect("帧有 type").to_owned())
            .collect()
    }

    #[test]
    fn handle_answers_a_frame_array_and_never_throws() {
        let mut session = open(SEALED_MOD);
        assert_eq!(frame_types(&session.handle(&hello(&session))), ["ready"]);
        assert_eq!(
            frame_types(&session.handle(r#"{"type":"step","ticks":1,"dt":1}"#)),
            ["events", "delta"],
            "step 两帧，顺序即线上顺序"
        );
        assert_eq!(
            frame_types(&session.handle("not json")),
            ["error"],
            "坏 JSON 答 error 帧而不是抛异常"
        );
    }

    #[test]
    fn step_before_seal_is_a_rule_error() {
        let mut session = open("world.component{ name = \"hp\", kind = \"i64\" }");
        assert_eq!(frame_types(&session.handle(&hello(&session))), ["ready"]);
        let reply = session.handle(r#"{"type":"step","ticks":1,"dt":1}"#);
        let value: Json = serde_json::from_str(&reply).expect("回执是 JSON");
        assert_eq!(value[0]["type"], "error");
        assert_eq!(value[0]["code"], "E_RULE");
    }

    #[test]
    fn broken_mod_source_fails_loading() {
        assert!(
            YggSession::new(image("syntax error here (((")).is_err(),
            "坏 Lua 必须装载失败（构造面 throw）"
        );
    }

    #[test]
    fn two_sessions_from_one_image_produce_identical_streams() {
        let script = [
            hello(&open(SEALED_MOD)),
            r#"{"type":"submit","ops":[{"op":"spawn","n":2,"comps":[{"cid":0,"values":[{"int":5}]}]}]}"#
                .to_owned(),
            r#"{"type":"step","ticks":2,"dt":1}"#.to_owned(),
        ];
        let run = |script: &[String]| -> Vec<String> {
            let mut session = open(SEALED_MOD);
            script
                .iter()
                .map(|msg| session.handle(msg))
                .collect::<Vec<_>>()
        };
        assert_eq!(run(&script), run(&script), "同镜像同脚本 → 同回执");
    }

    #[test]
    fn snapshot_bytes_agree_between_two_sessions() {
        let advance = |session: &mut YggSession| {
            session.handle(&hello(session));
            session.handle(r#"{"type":"step","ticks":2,"dt":1}"#);
        };
        let mut first = open(SEALED_MOD);
        let mut second = open(SEALED_MOD);
        advance(&mut first);
        advance(&mut second);
        assert_eq!(first.snapshot(), second.snapshot(), "同一段脚本同一帧字节");
        assert!(!first.snapshot().is_empty(), "世界帧非空");
    }

    #[test]
    fn load_path_is_the_native_one() {
        // FR-9 壳面锚（host 同构编译）：壳的装载仍然走 native 装载路径——golden 向量
        // （同一份文件、编译期嵌入）的状态哈希必须原样复现。壳换成协议绑定之后这条
        // 依然成立，说明「薄壳」没有偷偷换掉装载面。
        let golden: Json = serde_json::from_str(include_str!(
            "../../ygg-sandbox/tests/fixtures/golden_vector.json"
        ))
        .expect("golden 向量可解析");
        let session = open(golden["mod_source"].as_str().expect("mod_source 是字符串"));
        assert_eq!(
            state_hash(session.inner.loaded()).expect("live world"),
            golden["expected_state_hash"]
                .as_str()
                .expect("expected hash"),
            "壳必须复现钉死的 golden 状态哈希"
        );
    }
}
