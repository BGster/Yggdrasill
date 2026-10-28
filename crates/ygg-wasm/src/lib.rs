//! ygg-wasm：wasm 壳（B1 实装）——[`KernelFacade`] 的 wasm-bindgen 同步导出面。
//!
//! 对外契约（D-A1 线格式，语义面单一镜像 `crates/ygg-sandbox/src/driver.rs`）：
//! - **唯一同步入口** [`YggKernel::dispatch`]：`dispatch(cmd, payload) -> String`
//!   返回 A1 JSON 信封原文——成功 `{"ok":true,...}`；域错误
//!   `{"ok":false,"error":{"code","message"}}`，稳定码 `FUEL_EXHAUSTED /
//!   SANDBOX_DENIED / VALIDATION / KV_LIMIT / INTERNAL`。**域错误永不 throw**
//!   （返回值即信封）。空壳注记的 `kernel_drain_events()` 自由函数导出面由
//!   `dispatch("drain","")` 承载（保留命令，线格式不变）。
//! - **便捷镜像** [`YggKernel::save`] / [`YggKernel::load`] /
//!   [`YggKernel::state`]：保留命令的一行镜像（= dispatch 同名命令），
//!   语义零新增，仅为 JS 侧提供可发现的类型面。
//! - **哈希度量面** [`YggKernel::state_hash`]：FR-9 wasm 侧确定性度量——
//!   `hex(SHA-256(canonical_state_bytes))`（native `ygg_sandbox::hash::state_hash`
//!   同一函数同一字节序），A1 信封承载；node 侧 `verify-determinism.mjs` 以此
//!   比对 golden-vector fixture 的 expected_state_hash，证 native/wasm 双跑
//!   哈希等价（M1 C2）。
//! - **构造面** [`YggKernel::new`]：`manifest_json` 为空串 →
//!   `KernelFacade::new`（无资源路径）；否则 → `KernelFacade::with_resources`。
//!   `schemas_json` = schema 文本 JSON 数组（壳逐文本解析 `$id` 作注册 id，
//!   与 `ValidatorSet::from_texts` 的 `(id, text)` 对齐）；`resources_json` =
//!   `path → base64(bytes)` JSON 对象（壳内置 base64 解码，标准字母表 + 结尾
//!   padding）。manifest 为空串时两资源参数被忽略（无资源装载路径）。
//! - **错误跨界双通道（A1）**：构造期的 marshal 失败（坏 base64 / 缺 `$id` /
//!   非法 JSON）与域失败（ModLoad / 资源包拒绝）**throw**——构造无信封回程
//!   通道，域失败抛出的值仍是 A1 错误信封 JSON 文本（消费方在 init 处
//!   catch 一次即可统一按信封分支）；`dispatch` / `save` / `load` / `state`
//!   **永不 throw**。
//! - **wasm32 构建链**（mlua-rs/mlua docs/wasm32 recipe，mlua@ff96ed7
//!   tests/wasm 同型）：`wasi_alloc` 模块提供委托 WASI libc 的全局分配器
//!   （Rust 与 wasi-libc 必须共享线性内存管理）；JS 宿主须提供 WASI
//!   preview-1 import shim（本 crate `wasi-shim.js`，init 前安装）。构建
//!   单一入口 `scripts/build-wasm.sh`（两步链：cargo build → wasm-bindgen
//!   `--reference-types`；wasm-pack 0.15 不透传该旗而 EH 模块的 catch
//!   wrappers 硬要求 externref 表）。Lua 5.4 错误处理走 C setjmp 面
//!   （wasi-libc EH 变体），无需 C++ 异常旗（与 mlua tests/wasm 对 lua54
//!   的旗面一致）。
//!
//! host 目标（rlib，`cargo test --workspace` 门禁面）下同构编译：分配器与
//! wasm 专属面全部 `cfg(target_arch = "wasm32")` 门控；[`YggKernel`] 及其
//! 解析/编解码面在 host 上可测（本文件 `tests` 模块）。

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

use serde_json::Value as Json;
use wasm_bindgen::prelude::*;

use ygg_sandbox::driver::{KernelError, KernelFacade};

/// wasm 壳导出结构：持有一个 [`KernelFacade`]（语义面全部委托）。
#[wasm_bindgen]
pub struct YggKernel {
    inner: KernelFacade,
}

#[wasm_bindgen]
impl YggKernel {
    /// 构造内核：装载 Mod + （可选）资源包。装载序与语义面 =
    /// `KernelFacade::new` / `KernelFacade::with_resources`（S10/S11 锁定序）。
    ///
    /// - `mod_source`：Lua Mod 源码全文；
    /// - `manifest_json`：资源 manifest JSON 文本；**空串 = 无资源路径**
    ///   （此时 `schemas_json` / `resources_json` 被忽略）；
    /// - `schemas_json`：schema 文本 JSON 数组（注册 id 取自各文本的 `$id`）；
    /// - `resources_json`：`path → base64` JSON 对象（path = manifest
    ///   `entry.path` 查找键）。
    ///
    /// throw 通道：marshal 失败（非法 JSON / 元素非字符串 / 缺 `$id` /
    /// 坏 base64）携带定位文案；装载域失败（ModLoad / 资源包拒绝）携带
    /// A1 错误信封 JSON 文本（`{"ok":false,"error":{code,message}}`）。
    #[wasm_bindgen(constructor)]
    pub fn new(
        mod_source: String,
        manifest_json: String,
        schemas_json: String,
        resources_json: String,
    ) -> Result<YggKernel, JsValue> {
        let outcome = if manifest_json.is_empty() {
            KernelFacade::new(&mod_source)
        } else {
            let schemas = parse_schemas(&schemas_json).map_err(|message| throw_value(&message))?;
            let resources =
                parse_resources(&resources_json).map_err(|message| throw_value(&message))?;
            KernelFacade::with_resources(
                &mod_source,
                manifest_json.as_bytes(),
                &schemas,
                &resources,
            )
        };
        // 域失败（构造期）→ throw A1 错误信封文本：构造无信封回程通道，
        // 信封内容与 dispatch 面逐字段一致，消费方 catch 后按码分支不变。
        let inner = outcome.map_err(|err| throw_value(&kernel_error_envelope(&err)))?;
        Ok(YggKernel { inner })
    }

    /// 唯一同步入口（A1）：命令分发，返回信封 JSON 原文。**永不 throw**。
    pub fn dispatch(&mut self, cmd: String, payload: String) -> String {
        self.inner.dispatch(&cmd, &payload)
    }

    /// `save` 镜像（= `dispatch("save","")`）：完整状态快照信封。
    pub fn save(&mut self) -> String {
        self.dispatch("save".to_owned(), String::new())
    }

    /// `load` 镜像（= `dispatch("load", save_json)`）：存档 JSON 载入，
    /// 返回 `{"ok":true,"restored":true,"pendingChoice":...}` 信封或
    /// VALIDATION 错误信封。
    pub fn load(&mut self, save_json: String) -> String {
        self.dispatch("load".to_owned(), save_json)
    }

    /// `state` 镜像（= `dispatch("state","")`）：诊断信封（引擎/实体/KV
    /// 版本号 + 快照字节数 + 待决选择）。
    pub fn state(&mut self) -> String {
        self.dispatch("state".to_owned(), String::new())
    }

    /// 状态哈希（FR-9 wasm 侧度量面）：`hex(SHA-256(canonical_state_bytes))`，
    /// 与 native 侧 `ygg_sandbox::hash::state_hash` 同一函数、同一字节序。
    /// node 侧 `verify-determinism.mjs` 以此比对 golden-vector fixture 的
    /// `expected_state_hash`（native 钉死基线），证 native/wasm 双跑哈希等价。
    ///
    /// A1 信封承载：成功 `{"ok":true,"stateHash":"<64 位小写 hex>"}`；失败
    /// 路径（快照序列化错误，驱动 bug 面）= `SnapshotError` 经 `KernelError`
    /// 单点码映射的 INTERNAL 错误信封（与 dispatch 错误信封同构）。**永不
    /// throw**（与 dispatch / save / load / state 同约）。
    pub fn state_hash(&self) -> String {
        match ygg_sandbox::hash::state_hash(&self.inner) {
            Ok(hash) => serde_json::json!({ "ok": true, "stateHash": hash }).to_string(),
            Err(err) => kernel_error_envelope(&KernelError::from(err)),
        }
    }
}

/// marshal 失败 → throw 值（构造期输入契约违反：非域错误，不进信封）。
/// 非 wasm 目标无 JS 值空间（`JsValue::from_str` 在 host 上 panic）——host
/// 返回 NULL 占位（构造失败面为 Err 本身；信封文本内容由下方纯函数单测 +
/// node smoke 在真 wasm 上验证）。
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

/// 构造期域失败 → A1 错误信封 JSON 文本（码 = `KernelError::code` 单点映射）。
fn kernel_error_envelope(err: &KernelError) -> String {
    serde_json::json!({
        "ok": false,
        "error": {"code": err.code(), "message": err.to_string()},
    })
    .to_string()
}

/// `schemas_json` 解析：JSON 字符串数组 → `(schema id, schema 文本)` 对，
/// id 取自各 schema 文本的 `$id`（与 `ValidatorSet::from_texts` 入参对齐；
/// 注册序 = 数组序）。
fn parse_schemas(schemas_json: &str) -> Result<Vec<(String, String)>, String> {
    let value: Json = serde_json::from_str(schemas_json)
        .map_err(|err| format!("schemas_json is not valid JSON: {err}"))?;
    let items = value
        .as_array()
        .ok_or("schemas_json must be a JSON array of schema texts")?;
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let text = item
                .as_str()
                .ok_or_else(|| format!("schemas_json[{index}] is not a string"))?;
            let id = serde_json::from_str::<Json>(text)
                .ok()
                .and_then(|schema| schema.get("$id").and_then(Json::as_str).map(str::to_owned))
                .ok_or_else(|| {
                    format!("schemas_json[{index}] has no string `$id` (registration id is read from `$id`)")
                })?;
            Ok((id, text.to_owned()))
        })
        .collect()
}

/// `resources_json` 解析：`path → base64` JSON 对象 → `(path, bytes)` 对。
/// serde_json 默认 Map = BTreeMap：遍历序 = path 字典序（装载序确定，
/// 与 JS 侧键序无关）。
fn parse_resources(resources_json: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
    let value: Json = serde_json::from_str(resources_json)
        .map_err(|err| format!("resources_json is not valid JSON: {err}"))?;
    let entries = value
        .as_object()
        .ok_or("resources_json must be a JSON object of path → base64 text")?;
    entries
        .iter()
        .map(|(path, item)| {
            let text = item
                .as_str()
                .ok_or_else(|| format!("resources_json[{path:?}] is not a base64 string"))?;
            let bytes =
                base64_decode(text).map_err(|err| format!("resources_json[{path:?}]: {err}"))?;
            Ok((path.clone(), bytes))
        })
        .collect()
}

/// 标准 base64 解码（自实现，零新依赖）：标准字母表 + 结尾 `=` padding；
/// 非法字符 / 非法组长度（1 字符尾组）/ padding 位置错误 → Err 文案。
fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    fn sextet(byte: u8) -> Result<u32, String> {
        match byte {
            b'A'..=b'Z' => Ok((byte - b'A') as u32),
            b'a'..=b'z' => Ok((byte - b'a' + 26) as u32),
            b'0'..=b'9' => Ok((byte - b'0' + 52) as u32),
            b'+' => Ok(62),
            b'/' => Ok(63),
            other => Err(format!("invalid base64 byte {other:#04x}")),
        }
    }
    let trimmed = input.trim_end();
    let body = trimmed.trim_end_matches('=');
    // padding 只允许结尾，且总长须为 4 的倍数（padding 已剥除后 body 补算）。
    if trimmed.len() != body.len() + (4 - body.len() % 4) % 4 {
        return Err("misplaced base64 padding".to_owned());
    }
    let mut out = Vec::with_capacity(body.len() * 3 / 4 + 3);
    for chunk in body.as_bytes().chunks(4) {
        let n = match chunk {
            [a, b, c, d] => {
                (sextet(*a)? << 18) | (sextet(*b)? << 12) | (sextet(*c)? << 6) | sextet(*d)?
            }
            [a, b, c] => (sextet(*a)? << 18) | (sextet(*b)? << 12) | (sextet(*c)? << 6),
            [a, b] => (sextet(*a)? << 18) | (sextet(*b)? << 12),
            _ => return Err("truncated base64 group".to_owned()),
        };
        out.push((n >> 16) as u8);
        if chunk.len() >= 3 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() >= 4 {
            out.push(n as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply_code(reply: &str) -> String {
        let parsed: Json = serde_json::from_str(reply).unwrap();
        parsed["error"]["code"].as_str().unwrap_or("").to_owned()
    }

    /// 空转 Mod：无任何注册——事件无 handler，转换空跑即成功。
    const IDLE_MOD: &str = "-- idle mod: no handlers";

    #[test]
    fn base64_decode_matches_standard_vectors() {
        assert_eq!(base64_decode("").unwrap(), b"");
        assert_eq!(base64_decode("Zg==").unwrap(), b"f");
        assert_eq!(base64_decode("Zm8=").unwrap(), b"fo");
        assert_eq!(base64_decode("Zm9v").unwrap(), b"foo");
        assert_eq!(base64_decode("Zm9vYg==").unwrap(), b"foob");
        assert_eq!(base64_decode("Zm9vYmE=").unwrap(), b"fooba");
        assert_eq!(base64_decode("Zm9vYmFy").unwrap(), b"foobar");
        // JSON 线格式常见转义面：无空白容忍（严格标准字母表）。
        assert!(base64_decode("Zm 9v").is_err(), "space must be rejected");
        assert!(
            base64_decode("Zm9v=").is_err(),
            "misplaced padding must be rejected"
        );
        assert!(
            base64_decode("Z===").is_err(),
            "overlong padding must be rejected"
        );
        assert!(
            base64_decode("A").is_err(),
            "single-char group must be rejected"
        );
        assert!(
            base64_decode("Zm9*").is_err(),
            "non-alphabet byte must be rejected"
        );
    }

    #[test]
    fn base64_decode_rejects_inner_padding() {
        // 中间 padding：总长校验（body+padding≡0 mod 4）+ 位置校验双门。
        assert!(base64_decode("Zg==Zm8=").is_err());
    }

    #[test]
    fn parse_schemas_reads_id_from_schema_text() {
        let schemas = parse_schemas(
            r#"["{\"$id\":\"https://x/y.json\",\"type\":\"object\"}","{\"$schema\":\"https://json-schema.org/draft/2020-12/schema\",\"$id\":\"https://x/z.json\"}"]"#,
        )
        .unwrap();
        assert_eq!(schemas.len(), 2);
        assert_eq!(schemas[0].0, "https://x/y.json");
        assert_eq!(schemas[1].0, "https://x/z.json");
        assert!(schemas[0].1.contains("https://x/y.json"));
    }

    #[test]
    fn parse_schemas_rejects_malformed_input() {
        assert!(parse_schemas("not json").is_err());
        assert!(
            parse_schemas(r#"{"a":1}"#).is_err(),
            "object is not an array"
        );
        assert!(parse_schemas("[42]").is_err(), "element must be a string");
        assert!(
            parse_schemas(r#"["{\"type\":\"object\"}"]"#).is_err(),
            "missing `$id` must be rejected"
        );
        assert!(
            parse_schemas(r#"["{\"$id\":42}"]"#).is_err(),
            "non-string `$id` must be rejected"
        );
    }

    #[test]
    fn parse_resources_decodes_base64_values_in_sorted_path_order() {
        // serde_json 默认 Map = BTreeMap：输出序 = path 字典序（与书写序无关）。
        let resources = parse_resources(r#"{"data/b.bin":"Zm9v","data/a.bin":"Zg=="}"#).unwrap();
        assert_eq!(resources[0].0, "data/a.bin");
        assert_eq!(resources[0].1, b"f");
        assert_eq!(resources[1].0, "data/b.bin");
        assert_eq!(resources[1].1, b"foo");
    }

    #[test]
    fn parse_resources_rejects_malformed_input() {
        assert!(parse_resources("not json").is_err());
        assert!(parse_resources("[]").is_err(), "array is not an object");
        assert!(
            parse_resources(r#"{"p":42}"#).is_err(),
            "value must be a string"
        );
        assert!(
            parse_resources(r#"{"p":"Zm9*"}"#).is_err(),
            "bad base64 must be rejected"
        );
    }

    #[test]
    fn constructor_empty_manifest_ignores_resource_args_and_dispatches() {
        // 空串 manifest = 无资源路径；资源参数占位被忽略（契约口径）。
        let mut kernel = YggKernel::new(
            IDLE_MOD.to_owned(),
            String::new(),
            "[]".to_owned(),
            "{}".to_owned(),
        )
        .unwrap();
        let reply = kernel.dispatch("event.any".to_owned(), "0".to_owned());
        let parsed: Json = serde_json::from_str(&reply).unwrap();
        assert_eq!(parsed["ok"], Json::Bool(true));
        assert_eq!(parsed["event"], Json::String("event.any".to_owned()));
    }

    #[test]
    fn constructor_mod_load_failure_returns_err() {
        // 域失败（ModLoad）：构造 Err（host 上 throw 值为 NULL 占位——无 JS
        // 值空间；信封文本格式由 kernel_error_envelope 单测锁定，真 wasm 上的
        // throw 内容由 node smoke 实证）。
        let outcome = YggKernel::new(
            "syntax error here (((".to_owned(),
            String::new(),
            "[]".to_owned(),
            "{}".to_owned(),
        );
        assert!(outcome.is_err(), "broken mod source must fail construction");
    }

    #[test]
    fn kernel_error_envelope_carries_stable_code_and_message() {
        let err = match KernelFacade::new("syntax error (((") {
            Err(err) => err,
            Ok(_) => panic!("broken mod source must fail construction"),
        };
        let parsed: Json = serde_json::from_str(&kernel_error_envelope(&err)).unwrap();
        assert_eq!(parsed["ok"], Json::Bool(false));
        assert_eq!(parsed["error"]["code"], Json::String("INTERNAL".to_owned()));
        assert!(
            parsed["error"]["message"]
                .as_str()
                .unwrap()
                .contains("mod load failed"),
            "message must carry the kernel error text"
        );
    }

    #[test]
    fn save_load_helpers_roundtrip_through_reserved_commands() {
        let mut kernel = YggKernel::new(
            IDLE_MOD.to_owned(),
            String::new(),
            "[]".to_owned(),
            "{}".to_owned(),
        )
        .unwrap();
        let save_reply = kernel.save();
        let parsed: Json = serde_json::from_str(&save_reply).unwrap();
        assert_eq!(parsed["ok"], Json::Bool(true));
        let save_json = parsed["save"].to_string();
        let load_reply = kernel.load(save_json);
        let parsed: Json = serde_json::from_str(&load_reply).unwrap();
        assert_eq!(parsed["restored"], Json::Bool(true));
        let state_reply = kernel.state();
        let parsed: Json = serde_json::from_str(&state_reply).unwrap();
        assert_eq!(parsed["engineVersion"], parsed["engineVersion"]);
        assert!(parsed["snapshotBytes"].is_number());
    }

    #[test]
    fn state_hash_envelope_reproduces_golden_hash_on_host() {
        // FR-9 壳面锚（host 同构编译）：壳 state_hash() 经 golden-vector
        // fixture（同一份文件、include_str 编译期嵌入）必须复现钉死的
        // expected_state_hash——本测试锁的是**壳方法的信封与委托面**
        // （native 等值本身由 probe_golden_vector 承载）；真 wasm 上的同一
        // 断言由 node verify-determinism.mjs 在 pkg 产物上活链承载（M1 C2）。
        let golden: Json = serde_json::from_str(include_str!(
            "../../ygg-sandbox/tests/fixtures/golden_vector.json"
        ))
        .unwrap();
        let mut kernel = YggKernel::new(
            golden["mod_source"].as_str().unwrap().to_owned(),
            String::new(),
            "[]".to_owned(),
            "{}".to_owned(),
        )
        .unwrap();
        for step in golden["event_sequence"].as_array().unwrap() {
            let reply = kernel.dispatch(
                step["cmd"].as_str().unwrap().to_owned(),
                step["payload"].as_str().unwrap().to_owned(),
            );
            let parsed: Json = serde_json::from_str(&reply).unwrap();
            assert_eq!(
                parsed["ok"],
                Json::Bool(true),
                "golden step must succeed: {reply}"
            );
        }
        let reply = kernel.state_hash();
        let parsed: Json = serde_json::from_str(&reply).unwrap();
        assert_eq!(
            parsed["ok"],
            Json::Bool(true),
            "state_hash envelope: {reply}"
        );
        assert_eq!(
            parsed["stateHash"],
            Json::String(golden["expected_state_hash"].as_str().unwrap().to_owned()),
            "shell state_hash must reproduce the pinned golden hash"
        );
    }

    #[test]
    fn dispatch_invalid_payload_returns_envelope_never_throws() {
        let mut kernel = YggKernel::new(
            IDLE_MOD.to_owned(),
            String::new(),
            "[]".to_owned(),
            "{}".to_owned(),
        )
        .unwrap();
        let reply = kernel.dispatch("event.x".to_owned(), "{\"nested\":{}}".to_owned());
        assert_eq!(reply_code(&reply), "VALIDATION");
        let reply = kernel.load("not json".to_owned());
        assert_eq!(reply_code(&reply), "VALIDATION");
    }
}
