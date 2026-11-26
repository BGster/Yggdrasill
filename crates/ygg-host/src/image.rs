//! 包镜像 → 内核装载：**无文件系统**的一条装载路径（三条绑定共用）。
//!
//! wasm 与 C 侧没有可读的「目录」，四件装载参数只能随字节一起过界；镜像就是
//! 这段 JSON 文本（与 [`YggSession`](crate::Session) 的构造参数同一份形状）：
//!
//! ```text
//! {"modSource": "<Lua 源码>",
//!  "manifest": "<manifest.json 文本；空串 = 无资源>",
//!  "schemas":  ["<schema 文本>", ...],
//!  "resources": {"<path>": "<base64>", ...}}
//! ```
//!
//! - `manifest` 空串走无资源路径（与 native [`ModBundle::in_memory`] 同一口径），
//!   此时 `schemas` / `resources` 被忽略；
//! - `schemas` 的注册 id 取自各 schema 文本的 `$id`（与 `ValidatorSet::from_texts`
//!   入参对齐，注册序 = 数组序）；
//! - `resources` 按 path 字典序装载（serde_json 默认 Map = `BTreeMap`，与书写序无关）。
//!
//! 本模块**不碰磁盘**：`ModBundle` 在内存里成型后直接 [`Loaded::from_bundle`]。
//! 因此内容寻址的 `pack_hash`（[`Loaded::manifest_hash`]）是 manifest 文本本身的
//! 摘要——同一份镜像在任何宿主、任何平台上都得到同一串，这正是握手闸要的
//! 「A 包的存档读不进 B 包」。
//!
//! 镜像解析放在内核侧而不是绑定层：解析里每一步都是「这个字段认不认」的判断，
//! 而绑定层的规矩是**只搬字节**。三条绑定（Rust / WASM / C ABI）共用这一份实现，
//! 才有 [`parity`](crate) 可言。

use serde_json::Value as Json;

use ygg_sandbox::modload::{LoadError, Loaded, ModBundle};

use crate::base64::base64_decode;
use crate::session::Session;

/// 镜像 → 已装载内核（会话尚未握手）。
///
/// 想要会话直接用 [`Session::from_image`](crate::Session::from_image)；想自己持有
/// `Loaded`（自定会话寿命 / 直接步进）走这条。
pub fn load(image: &[u8]) -> Result<Loaded, String> {
    let bundle = decode_image(image)?;
    Loaded::from_bundle(&bundle).map_err(|err| load_error(&err))
}

/// 镜像 → 未握手会话（[`Session::from_image`] 的实现面）。
pub(crate) fn open(image: &[u8]) -> Result<Session, String> {
    load(image).map(Session::new)
}

/// 装载错误 → 绑定层文案（这里的出口是 `String`，不是协议帧：镜像不是线上消息）。
fn load_error(err: &LoadError) -> String {
    format!("装载失败：{err}")
}

/// 镜像 → 装载参数（唯一的镜像解析面）。
fn decode_image(bytes: &[u8]) -> Result<ModBundle, String> {
    let text = std::str::from_utf8(bytes).map_err(|err| format!("镜像不是合法 UTF-8：{err}"))?;
    let image: Json =
        serde_json::from_str(text).map_err(|err| format!("镜像不是合法 JSON：{err}"))?;
    let mod_source = image
        .get("modSource")
        .and_then(Json::as_str)
        .ok_or("镜像缺字符串字段 `modSource`")?;
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

/// `schemas` 解析：JSON 字符串数组 → `(schema id, schema 文本)` 对。
fn parse_schemas(schemas_json: &str) -> Result<Vec<(String, String)>, String> {
    let value: Json = serde_json::from_str(schemas_json)
        .map_err(|err| format!("镜像的 `schemas` 不是合法 JSON：{err}"))?;
    let items = value
        .as_array()
        .ok_or("镜像的 `schemas` 必须是 schema 文本数组")?;
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let text = item
                .as_str()
                .ok_or_else(|| format!("镜像的 `schemas[{index}]` 不是字符串"))?;
            let id = serde_json::from_str::<Json>(text)
                .ok()
                .and_then(|schema| schema.get("$id").and_then(Json::as_str).map(str::to_owned))
                .ok_or_else(|| {
                    format!("镜像的 `schemas[{index}]` 缺字符串 `$id`（注册 id 取自 `$id`）")
                })?;
            Ok((id, text.to_owned()))
        })
        .collect()
}

/// `resources` 解析：`path → base64` JSON 对象 → `(path, bytes)` 对（path 字典序）。
fn parse_resources(resources_json: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
    let value: Json = serde_json::from_str(resources_json)
        .map_err(|err| format!("镜像的 `resources` 不是合法 JSON：{err}"))?;
    let entries = value
        .as_object()
        .ok_or("镜像的 `resources` 必须是 path → base64 文本的对象")?;
    entries
        .iter()
        .map(|(path, item)| {
            let text = item
                .as_str()
                .ok_or_else(|| format!("镜像的 `resources[{path:?}]` 不是 base64 字符串"))?;
            let bytes = base64_decode(text)
                .map_err(|err| format!("镜像的 `resources[{path:?}]`：{err}"))?;
            Ok((path.clone(), bytes))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{decode_image, load, open, parse_resources, parse_schemas};
    use ygg_sandbox::modload::MINIMAL_MANIFEST;

    /// 空转 Mod：不声明组件、不生成实体——世界帧就是空世界。
    const IDLE_MOD: &str = "-- idle mod: no world declarations";

    fn image(mod_source: &str) -> Vec<u8> {
        serde_json::json!({ "modSource": mod_source, "manifest": "" })
            .to_string()
            .into_bytes()
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
    fn load_keeps_the_manifest_hash_content_addressed() {
        // 同一份镜像两次装载 → 同一串 pack_hash（握手闸的可比性）。
        let first = load(&image(IDLE_MOD)).unwrap();
        let second = load(&image(IDLE_MOD)).unwrap();
        assert_eq!(first.manifest_hash(), second.manifest_hash());
        // 换 Lua 源码但 manifest 不变：pack_hash 也不变（摘要只覆盖 manifest）。
        let other = load(&image("world.seal()")).unwrap();
        assert_eq!(first.manifest_hash(), other.manifest_hash());
    }

    #[test]
    fn load_rejects_broken_mod_source() {
        assert!(
            load(&image("syntax error here (((")).is_err(),
            "broken mod source must fail loading"
        );
        assert!(open(&image(IDLE_MOD)).is_ok());
    }
}
