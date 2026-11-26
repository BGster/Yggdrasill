//! ygg-wasm：wasm 壳（B1 实装）——[`Loaded`] 的 wasm-bindgen 同步导出面。
//!
//! 对外契约（M1-E 收紧：**只有装载 / 推进 / 快照三件事**，没有事件面）：
//! - **装载** [`YggWorld::open`]：`open(image_bytes) -> world`。`image_bytes` 是
//!   Mod 镜像的 JSON 文本（无文件系统的 wasm 侧没有「目录」可读，四件装载参数
//!   只能随字节一起过界）：
//!   `{"modSource": "<Lua 源码>", "manifest": "<manifest.json 文本；空串 = 无资源>",
//!     "schemas": ["<schema 文本>", ...], "resources": {"<path>": "<base64>", ...}}`。
//!   `manifest` 空串走无资源路径（资源参数被忽略）——与 native
//!   `ModBundle::in_memory` 同一口径。
//! - **推进时间** [`YggWorld::step`]：`step(ticks, dt) -> "<64 位小写 hex>"`，
//!   即推进后的状态哈希 `hex(SHA-256(世界帧字节))`（与 native
//!   `ygg_sandbox::hash::state_hash` 同一函数同一字节序）——时间只由宿主推进，
//!   脚本里没有任何时钟入口。
//! - **快照** [`YggWorld::snapshot`]：`snapshot() -> Uint8Array`，世界帧规范字节；
//!   **重建** [`YggWorld::restore`]：`restore(image_bytes, frame) -> world`——帧只含
//!   世界面，脚本依旧是世界定义来源，所以重建必须同时给出镜像（对应 native
//!   `Loaded::restore(dir, bytes)` 的两个参数）。
//! - **没有** `submit` / `events` / `drive` / `dispatch` 导出面：协议层（ygg/1）在
//!   M3 落成后才经类型化接口加回来，现在不留占位壳。
//! - **错误跨界单通道**：装载失败 / 步进失败 / 坏帧一律 throw（构造与推进都没有
//!   信封回程通道，也还没有一个稳定的错误线格式值得钉死——信封是 M3 协议层的事）。
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
//! wasm 专属面全部 `cfg(target_arch = "wasm32")` 门控；[`YggWorld`] 及其
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

use ygg_sandbox::hash::state_hash;
use ygg_sandbox::modload::{Loaded, ModBundle};

/// wasm 壳导出结构：持有一个 [`Loaded`]（语义面全部委托）。
#[wasm_bindgen]
pub struct YggWorld {
    inner: Loaded,
}

#[wasm_bindgen]
impl YggWorld {
    /// 装载镜像（`image_bytes` 形状见模块文档）：读镜像 → 对账 → 全序装载。
    ///
    /// throw 通道：镜像非法（坏 JSON / 缺 `modSource` / 元素类型不符 / 坏 base64）
    /// 与装载域失败（manifest / 资源包 / 入口 chunk / 模块表）一律携带定位文案。
    #[wasm_bindgen(constructor)]
    pub fn open(image_bytes: Vec<u8>) -> Result<YggWorld, JsValue> {
        let bundle = decode_image(&image_bytes).map_err(|message| throw_value(&message))?;
        let inner = Loaded::from_bundle(&bundle).map_err(|err| throw_value(&err.to_string()))?;
        Ok(YggWorld { inner })
    }

    /// 推进 `ticks` 个 tick（每 tick `dt` 整数秒），返回推进后的状态哈希
    /// （64 位小写十六进制；与 native `hash::state_hash` 同一字节序）。
    pub fn step(&mut self, ticks: u32, dt: u32) -> Result<String, JsValue> {
        self.inner
            .step(ticks, dt)
            .map_err(|err| throw_value(&err.to_string()))?;
        state_hash(&self.inner).map_err(|err| throw_value(&err.to_string()))
    }

    /// 世界帧规范字节（同一状态 ⇒ 同一字节；存档与哈希共用这一个序列化面）。
    pub fn snapshot(&self) -> Result<Vec<u8>, JsValue> {
        self.inner
            .snapshot()
            .map_err(|err| throw_value(&err.to_string()))
    }

    /// 按世界帧重建：镜像（世界定义来源）+ 帧 → 新世界。坏帧先校验后改。
    pub fn restore(image_bytes: Vec<u8>, frame: Vec<u8>) -> Result<YggWorld, JsValue> {
        let bundle = decode_image(&image_bytes).map_err(|message| throw_value(&message))?;
        let inner = Loaded::restore_from_bundle(&bundle, &frame)
            .map_err(|err| throw_value(&err.to_string()))?;
        Ok(YggWorld { inner })
    }
}

/// throw 值（wasm 侧是 JS 字符串；host 无 JS 值空间——`JsValue::from_str`
/// 在 host 上 panic，故返回 NULL 占位。构造失败面为 `Err` 本身，文案内容由
/// 下方纯函数单测覆盖）。
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

/// 镜像 → 装载参数（唯一的镜像解析面；字段形状见模块文档）。
fn decode_image(bytes: &[u8]) -> Result<ModBundle, String> {
    let text =
        std::str::from_utf8(bytes).map_err(|err| format!("image is not valid UTF-8: {err}"))?;
    let image: Json =
        serde_json::from_str(text).map_err(|err| format!("image is not valid JSON: {err}"))?;
    let mod_source = image
        .get("modSource")
        .and_then(Json::as_str)
        .ok_or("image has no string `modSource`")?;
    let manifest = image.get("manifest").and_then(Json::as_str).unwrap_or("");
    if manifest.is_empty() {
        // 无资源路径：schema / resources 参数被忽略（与 native 同口径）。
        return Ok(ModBundle::in_memory(mod_source));
    }
    let schemas = parse_schemas(&image["schemas"].to_string())?;
    let resources = parse_resources(&image["resources"].to_string())?;
    Ok(ModBundle {
        mod_source: mod_source.to_owned(),
        manifest_bytes: manifest.as_bytes().to_vec(),
        schemas,
        resources,
        ..ModBundle::in_memory(String::new())
    })
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
    use ygg_sandbox::modload::MINIMAL_MANIFEST;

    /// 空转 Mod：不声明组件、不生成实体——世界帧就是空世界。
    const IDLE_MOD: &str = "-- idle mod: no world declarations";

    fn image(mod_source: &str) -> Vec<u8> {
        serde_json::json!({ "modSource": mod_source, "manifest": "" })
            .to_string()
            .into_bytes()
    }

    fn state_hash_of(world: &YggWorld) -> String {
        state_hash(&world.inner).expect("hash on live world")
    }

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
    fn decode_image_requires_mod_source_and_takes_empty_manifest_as_no_resources() {
        let bundle = decode_image(&image(IDLE_MOD)).unwrap();
        assert_eq!(bundle.mod_source, IDLE_MOD);
        assert_eq!(bundle.manifest_bytes, MINIMAL_MANIFEST.as_bytes());
        assert!(bundle.schemas.is_empty() && bundle.resources.is_empty());
        assert!(
            decode_image(br#"{"manifest":""}"#).is_err(),
            "missing modSource must be rejected"
        );
        assert!(decode_image(b"not json").is_err());
    }

    #[test]
    fn open_rejects_broken_mod_source() {
        assert!(
            YggWorld::open(image("syntax error here (((")).is_err(),
            "broken mod source must fail loading"
        );
    }

    #[test]
    fn step_advances_and_returns_stable_hash() {
        let mut world = YggWorld::open(image(IDLE_MOD)).unwrap();
        let before = state_hash_of(&world);
        let after = world.step(1, 1).unwrap();
        assert_ne!(after, before, "one tick must move the world frame");
        assert_eq!(world.step(1, 1).unwrap().len(), 64, "hex sha-256");
        assert!(
            world.step(0, 1).is_err(),
            "zero ticks must be rejected (same gate as native)"
        );
    }

    #[test]
    fn snapshot_restore_roundtrip_preserves_state_hash() {
        let mut world = YggWorld::open(image(
            "world.component{ name = \"hp\", kind = \"i64\" }\nworld.spawn{ hp = 7 }",
        ))
        .unwrap();
        world.step(2, 1).unwrap();
        let frame = world.snapshot().unwrap();
        let restored = YggWorld::restore(
            image("world.component{ name = \"hp\", kind = \"i64\" }\nworld.spawn{ hp = 7 }"),
            frame,
        )
        .unwrap();
        assert_eq!(state_hash_of(&restored), state_hash_of(&world));
        assert!(
            YggWorld::restore(image(IDLE_MOD), vec![0u8; 4]).is_err(),
            "坏帧必须被拒"
        );
    }

    #[test]
    fn step_reproduces_golden_state_hash_on_host() {
        // FR-9 壳面锚（host 同构编译）：壳的装载 + 步进经 golden-vector fixture
        // （同一份文件、include_str 编译期嵌入）必须复现钉死的 expected_state_hash
        // ——本测试锁的是**壳方法的委托面**（native 等值本身由
        // probe_determinism 承载）。
        let golden: Json = serde_json::from_str(include_str!(
            "../../ygg-sandbox/tests/fixtures/golden_vector.json"
        ))
        .unwrap();
        let mut world = YggWorld::open(image(golden["mod_source"].as_str().unwrap())).unwrap();
        assert_eq!(
            state_hash_of(&world),
            golden["expected_state_hash"].as_str().unwrap(),
            "shell must reproduce the pinned golden hash"
        );
        assert_eq!(
            world.step(1, 1).unwrap(),
            state_hash_of(&world),
            "step returns the state hash it just produced"
        );
    }
}
