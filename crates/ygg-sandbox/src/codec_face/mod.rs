//! `ygg.codec.*` —— 确定性 JSON 编解码的宿主面（原生加速面）。
//!
//! 契约：`openwiki/contracts/kernel/lua-api.md` §3.1.4（面形状 / 字节过面 / 两码）与
//! `openwiki/contracts/packages/codec.md` §4.1（下沉边界：谁下沉、谁回退、差分门禁）。
//!
//! # 这个面是什么、不是什么
//!
//! 它是 `pkg.codec` **Lua 实现的符合实现**，不是第二个规范：规范在包契约 I1 与
//! `packages/libs/codec/init.lua` 那四个函数上，本模块的每一个字节都必须与它相等
//! （判据 = `crates/ygg-sandbox/tests/codec_face.rs` 的差分字节相等）。
//!
//! 面的主项是 `encode(value) -> string`、`decode(text) -> value`（S19 起另带
//! 摘要键 `sha256(bytes) -> 64 位小写 hex`，见 [`digest`]）。**一个逐节点
//! getter 都没有**（批式铁律：一个值整包进出，一次调用一次穿越）。
//!
//! # 为什么放在 `ygg-sandbox` 而不是 `ygg-sim`
//!
//! 三条结构证据（计划稿 §2.3/§2.4，逐条可复现）：① `ygg-sim` 不依赖 mlua（Tier 1
//! 必须无 VM 可编译可基准），拿不到 Lua 值；② `ygg-sim/src/**` 禁浮点 token
//! （`tests/determinism_scan.rs` R1），而 JSON 数字的最短往返格式化必须有 `f64`；
//! ③ 快照是带 magic 与尾 digest 的二进制列帧，本面的容器是 JSON 文本（存档兼容面）。
//! 复用 Tier 1 的是**规范序纪律**（键按 UTF-8 字节序升序）与其实测证据，不是它的函数。
//!
//! # 与 Lua 实现的逐条对应（改任一侧都必须跑差分门禁）
//!
//! | 本模块 | `init.lua` | 规则 |
//! | --- | --- | --- |
//! | [`integer_text`] | `is_integer` + `tostring` | 整数子类型走 `%d` 形态 |
//! | [`number_text`] | `number_text` | 最短往返 `%g` 精度试探（1..17） |
//! | `format_g` | `("%.Pg"):format` | C `printf` 的 `%g` 语义 |
//! | [`string_text`] | `string_text` + `ESCAPES` | JSON 转义（非 ASCII 原样字节） |
//!
//! 表分类、键域、深度、环、元方法回退与解析器在后续块里加入（同一份契约）。

use mlua::Lua;

mod digest;
mod encoder;
mod number;
mod parser;

pub use encoder::{encode, string_text};
pub use number::{integer_text, number_text};
pub use parser::decode;

/// 值域拒绝 / 带元方法的表：调用方（`pkg.codec`）据此**回退 Lua 路径**。
pub const CODEC_UNSUPPORTED: &str = "YGG-CODEC-UNSUPPORTED";

/// 原生实现内部失败：**不回退**（缺陷必须响，静默降级会把 bug 藏成性能问题）。
pub const CODEC_INTERNAL: &str = "YGG-CODEC-INTERNAL";

/// 面的错误：两态，映射到 [`CODEC_UNSUPPORTED`] / [`CODEC_INTERNAL`] 两码。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecFaceError {
    /// 输入落在原生路径之外（值域拒绝 / 带元方法的表）⇒ 调用方回退。
    Unsupported(String),
    /// 实现内部失败 ⇒ 调用方报 `YGG-PKG-CODEC-HOST` 并不回退。
    Internal(String),
}

impl CodecFaceError {
    /// 错误码（消息首段，与 `YGG-SIM-*` / `YGG-FS-*` 同一处置）。
    pub fn code(&self) -> &'static str {
        match self {
            CodecFaceError::Unsupported(_) => CODEC_UNSUPPORTED,
            CodecFaceError::Internal(_) => CODEC_INTERNAL,
        }
    }

    /// 细节（不含码前缀）。
    pub fn message(&self) -> &str {
        match self {
            CodecFaceError::Unsupported(message) | CodecFaceError::Internal(message) => message,
        }
    }

    /// `"<码>: <细节>"`：过 mlua 边界时作为 runtime error 的消息。
    pub fn to_message(&self) -> String {
        format!("{}: {}", self.code(), self.message())
    }
}

impl std::fmt::Display for CodecFaceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.to_message())
    }
}

impl std::error::Error for CodecFaceError {}

/// 值域拒绝（回退）。
pub(crate) fn unsupported(detail: impl Into<String>) -> CodecFaceError {
    CodecFaceError::Unsupported(detail.into())
}

/// 实现内部失败（不回退）。
pub(crate) fn internal(detail: impl Into<String>) -> CodecFaceError {
    CodecFaceError::Internal(detail.into())
}

// ── 面安装 ───────────────────────────────────────────────────────────────────

/// 装 `ygg.codec.*`（3 键闭集：`encode` / `decode` / `sha256`）。
///
/// 与 `ygg.sim.*` 的区别写在契约里：本面**没有** unmounted 态——它无状态、无实例，
/// 缺面时的语义是明确的一条（调用方走 Lua 路径，产出字节相同）。因此这里不需要
/// 「表始终注册 + 抛码」那套；`host_api::install` 装上就一直在场。
///
/// `sha256` 是按键独立的第三键（S19）：包侧 `runtime.lua` 对它单独探测
/// `face.sha256` 为 function 才走原生——旧宿主只有 encode/decode 两键时，sha256
/// 自动落回包侧纯 Lua 实现，不会因「半个面」而报错。
pub fn install(lua: &Lua) -> Result<(), mlua::Error> {
    let ygg: mlua::Table = lua.globals().raw_get("ygg")?;
    let table = lua.create_table()?;
    let encode_function = lua.create_function(
        |lua: &Lua, value: mlua::Value| -> Result<mlua::LuaString, mlua::Error> {
            let bytes = encode(&value).map_err(|error| mlua::Error::runtime(error.to_message()))?;
            // 字节过面：`create_string(&[u8])` 而不是 `&str`——合法输出可以含
            // 非 UTF-8 字节（契约 §3.1.4「字节过面不要求 UTF-8」）。
            lua.create_string(&bytes)
        },
    )?;
    table.raw_set("encode", encode_function)?;

    let decode_function = lua.create_function(
        |lua: &Lua, text: mlua::Value| -> Result<mlua::Value, mlua::Error> {
            let bytes = match &text {
                // 非字符串一律 Unsupported（不是 INTERNAL）：包的 Lua 路径会给出
                // 规范的 `decode 只收字符串，得到 <type>` 文案。
                mlua::Value::String(text) => text.as_bytes(),
                _ => {
                    return Err(mlua::Error::runtime(
                        CodecFaceError::Unsupported(format!(
                            "decode 只收字符串，得到 {}",
                            text.type_name()
                        ))
                        .to_message(),
                    ))
                }
            };
            decode(lua, &bytes).map_err(|error| mlua::Error::runtime(error.to_message()))
        },
    )?;
    table.raw_set("decode", decode_function)?;

    // sha256（S19 第三键）：字节串进、64 位小写 hex 出。类型门在包侧先行，
    // 这里的 Unsupported 分支只是面的自保（与 decode 同一条对称防御）。
    let sha256_function = lua.create_function(
        |lua: &Lua, bytes: mlua::Value| -> Result<mlua::LuaString, mlua::Error> {
            let bytes = match &bytes {
                // 字节过面：原始字节，不要求 UTF-8（摘要的输入域是字节串）。
                mlua::Value::String(bytes) => bytes.as_bytes().to_vec(),
                _ => {
                    return Err(mlua::Error::runtime(
                        CodecFaceError::Unsupported(format!(
                            "sha256 只收字符串，得到 {}",
                            bytes.type_name()
                        ))
                        .to_message(),
                    ))
                }
            };
            let hex = digest::sha256(&bytes);
            lua.create_string(&hex)
        },
    )?;
    table.raw_set("sha256", sha256_function)?;
    ygg.raw_set("codec", table)?;
    Ok(())
}
