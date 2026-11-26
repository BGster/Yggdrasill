//! 包镜像 → 内核装载：**无文件系统**的一条装载路径（三条绑定共用）。
//!
//! wasm 与 C 侧没有可读的「目录」，四件装载参数只能随字节一起过界；镜像就是
//! 这段 JSON 文本（与 [`YggSession`](crate::Session) 的构造参数同一份形状）：
//!
//! ```text
//! {"modSource": "<Lua 源码>",
//!  "manifest": "<manifest.json 文本；空串 = 无资源>",
//!  "schemas":  ["<schema 文本>", ...],
//!  "resources": {"<path>": "<base64>", ...},
//!  "modules":  {"<模块路径>": "<Lua 源码>", ...},
//!  "chunkPaths": {"<模块路径>": "<包内相对路径>", ...}}
//! ```
//!
//! - `manifest` 空串走无资源路径（与 native [`ModBundle::in_memory`] 同一口径），
//!   此时 `schemas` / `resources` 被忽略；
//! - `schemas` 的注册 id 取自各 schema 文本的 `$id`（与 `ValidatorSet::from_texts`
//!   入参对齐，注册序 = 数组序）；
//! - `resources` 按 path 字典序装载（serde_json 默认 Map = `BTreeMap`，与书写序无关）；
//! - `modules` 是 `require` 的**全部**可见面（登记期判名字规范 / 重复 / 单模块与
//!   总数上限，与 native `pack.json` 的 `code` 段同一套判定）；`chunkPaths` 只喂
//!   traceback 诊断，两者都不进内容寻址。两个键的**键是模块路径**：`src/util`
//!   与 `src/util.lua` 指同一个模块（`pack.json.modules` 写后者），规范名只有
//!   一个，两种拼法撞到同一个模块即拒。
//!
//! **两个新键都可缺省**：缺省与空对象等价——单文件包的装载参数与没有镜像功能时
//! 逐字节相同。
//!
//! 本模块**不碰磁盘**：`ModBundle` 在内存里成型后直接 [`Loaded::from_bundle`]。
//! 因此内容寻址的 `pack_hash`（[`Loaded::manifest_hash`]）是 manifest 文本本身的
//! 摘要——同一份镜像在任何宿主、任何平台上都得到同一串，这正是握手闸要的
//! 「A 包的存档读不进 B 包」。
//!
//! 镜像解析放在内核侧而不是绑定层：解析里每一步都是「这个字段认不认」的判断，
//! 而绑定层的规矩是**只搬字节**。三条绑定（Rust / WASM / C ABI）共用这一份实现，
//! 才有 [`parity`](crate) 可言。

use std::collections::BTreeMap;

use serde_json::{Map, Value as Json};

use ygg_sandbox::modload::{LoadError, Loaded, ModBundle, MINIMAL_MANIFEST};
use ygg_sandbox::modules::{name_of_path, path_of, ModuleSet, MODULE_EXTENSION};

use crate::base64::{base64_decode, base64_encode};
use crate::session::Session;

/// 镜像 → 已装载内核（会话尚未握手）。
///
/// 想要会话直接用 [`Session::from_image`](crate::Session::from_image)；想自己持有
/// `Loaded`（自定会话寿命 / 直接步进）走这条。
pub fn load(image: &[u8]) -> Result<Loaded, String> {
    let bundle = decode_image(image)?;
    Loaded::from_bundle(&bundle).map_err(|err| load_error(&err))
}

/// 已装载内核 → 镜像 JSON（[`load`] 的逆，用于打包给 wasm / C 等无盘宿主）。
///
/// 输出是规范化 JSON（`serde_json::Map` 按键字典序、不依赖遍历顺序）：
/// - `manifest` 等于 [`MINIMAL_MANIFEST`] 且无 `schemas` / `resources` 时写出空串 `""`
///   （与 `decode_image` 的无资源路径一一对应），否则保留 `manifest_bytes` 原文（UTF-8）
///   并输出 `schemas`（数组序 = 注册序）与 `resources`（`path → base64`，按 path 字典序）；
/// - `modules` / `chunkPaths` 仅在非空时写出，键均由 [`path_of`] 还原为模块相对路径
///   （`src.util` → `src/util.lua`），确保 `load(to_image(x))` 与 `to_image(load(to_image(x)))`
///   逐字节稳定。
pub fn to_image(loaded: &Loaded) -> Result<String, String> {
    encode_bundle(loaded.bundle())
}

/// 装载参数 → 镜像 JSON 文本（[`decode_image`] 的逆）。
fn encode_bundle(bundle: &ModBundle) -> Result<String, String> {
    let mut obj = Map::new();
    obj.insert(
        "modSource".to_owned(),
        Json::String(bundle.mod_source.clone()),
    );

    let no_resources = bundle.manifest_bytes == MINIMAL_MANIFEST.as_bytes()
        && bundle.schemas.is_empty()
        && bundle.resources.is_empty();
    if no_resources {
        obj.insert("manifest".to_owned(), Json::String(String::new()));
    } else {
        let manifest = std::str::from_utf8(&bundle.manifest_bytes)
            .map_err(|err| format!("manifest 不是合法 UTF-8，无法写入镜像：{err}"))?;
        obj.insert("manifest".to_owned(), Json::String(manifest.to_owned()));

        let schemas: Vec<Json> = bundle
            .schemas
            .iter()
            .map(|(_, text)| Json::String(text.clone()))
            .collect();
        obj.insert("schemas".to_owned(), Json::Array(schemas));

        let mut resources = Map::new();
        for (path, bytes) in &bundle.resources {
            resources.insert(path.clone(), Json::String(base64_encode(bytes)));
        }
        obj.insert("resources".to_owned(), Json::Object(resources));
    }

    if !bundle.modules.is_empty() {
        let mut modules = Map::new();
        for name in bundle.modules.names() {
            let bytes = bundle
                .modules
                .get(name)
                .ok_or_else(|| format!("模块表缺条目 {name:?}"))?;
            let text = std::str::from_utf8(bytes)
                .map_err(|err| format!("模块 {name:?} 不是合法 UTF-8，无法写入镜像：{err}"))?;
            let path =
                path_of(name).map_err(|err| format!("模块名 {name:?} 无法还原路径：{err}"))?;
            modules.insert(path, Json::String(text.to_owned()));
        }
        obj.insert("modules".to_owned(), Json::Object(modules));
    }

    if !bundle.chunk_paths.is_empty() {
        let mut chunk_paths = Map::new();
        for (name, rel) in &bundle.chunk_paths {
            let path = path_of(name)
                .map_err(|err| format!("chunkPaths 键 {name:?} 无法还原路径：{err}"))?;
            chunk_paths.insert(path, Json::String(rel.clone()));
        }
        obj.insert("chunkPaths".to_owned(), Json::Object(chunk_paths));
    }

    serde_json::to_string(&Json::Object(obj)).map_err(|err| format!("镜像 JSON 序列化失败：{err}"))
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
    // 模块表与诊断覆盖表在两条路径上都生效：它们与 manifest 无关（单文件包也能带）。
    let modules = parse_modules(image.get("modules"))?;
    let chunk_paths = parse_chunk_paths(image.get("chunkPaths"))?;
    if manifest.is_empty() {
        // 无资源路径：schema / resources 参数被忽略（与 native 同口径）。
        return Ok(ModBundle {
            mod_source: mod_source.to_owned(),
            modules,
            chunk_paths,
            ..ModBundle::in_memory(String::new())
        });
    }
    let schemas = parse_schemas(&image["schemas"].to_string())?;
    let resources = parse_resources(&image["resources"].to_string())?;
    Ok(ModBundle {
        mod_source: mod_source.to_owned(),
        manifest_bytes: manifest.as_bytes().to_vec(),
        schemas,
        resources,
        modules,
        chunk_paths,
    })
}

/// 镜像里的模块键 → 规范名（`require` 用的就是它）。
///
/// 两种写法都收：`pack.json.modules` 写的是路径（`src/util.lua`），而手写或浏览器
/// 侧生成的镜像常只写 `src/util`。两者落到同一个规范名，因此两种拼法指向同一模块时
/// 会被 [`ModuleSet`] / [`parse_chunk_paths`] 当重复拒掉——不静默取后者。
fn module_name(key: &str) -> Result<String, String> {
    let path = if key.ends_with(MODULE_EXTENSION) {
        key.to_owned()
    } else {
        format!("{key}{MODULE_EXTENSION}")
    };
    name_of_path(&path).map_err(|err| format!("镜像的模块键 {key:?} 不是模块路径：{err}"))
}

/// `modules` 解析：`模块路径 → Lua 源码` 对象 → [`ModuleSet`]（登记期校验名字 / 重复 / 上限）。
///
/// 缺省与空对象等价：单文件包不必为了「能不能进浏览器」多写一个键。
fn parse_modules(modules: Option<&Json>) -> Result<ModuleSet, String> {
    let Some(value) = modules else {
        return Ok(ModuleSet::new());
    };
    let entries = value
        .as_object()
        .ok_or("镜像的 `modules` 必须是 模块路径 → Lua 源码 文本的对象")?;
    let pairs = entries
        .iter()
        .map(|(name, item)| {
            let text = item
                .as_str()
                .ok_or_else(|| format!("镜像的 `modules[{name:?}]` 不是字符串"))?;
            Ok((module_name(name)?, text.as_bytes().to_vec()))
        })
        .collect::<Result<Vec<_>, String>>()?;
    ModuleSet::from_pairs(pairs).map_err(|err| format!("镜像的 `modules` 登记失败：{err}"))
}

/// `chunkPaths` 解析：`模块路径 → 包内路径` 对象 → traceback 覆盖表（键插到规范名）。
///
/// 为什么要规范化：`require` 按规范名查覆盖表，作者若写 `src/util` 而表里存
/// `src.util` 就会静默不命中，traceback 退回推导路径。
fn parse_chunk_paths(paths: Option<&Json>) -> Result<BTreeMap<String, String>, String> {
    let Some(value) = paths else {
        return Ok(BTreeMap::new());
    };
    let entries = value
        .as_object()
        .ok_or("镜像的 `chunkPaths` 必须是 模块路径 → 路径 文本的对象")?;
    let mut table: BTreeMap<String, String> = BTreeMap::new();
    for (name, item) in entries {
        let path = item
            .as_str()
            .ok_or_else(|| format!("镜像的 `chunkPaths[{name:?}]` 不是字符串"))?;
        let canonical = module_name(name)?;
        if table.contains_key(&canonical) {
            return Err(format!(
                "镜像的 `chunkPaths` 里 {name:?} 与另一个键同指模块 {canonical:?}"
            ));
        }
        table.insert(canonical, path.to_owned());
    }
    Ok(table)
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
    use std::path::PathBuf;

    use super::{
        decode_image, load, open, parse_chunk_paths, parse_modules, parse_resources, parse_schemas,
        to_image,
    };
    use ygg_sandbox::modload::{Loaded, MINIMAL_MANIFEST};

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

    /// 两个模块的镜像：第二个模块能 `require` 到（wasm / C 侧的唯一装载路径）。
    ///
    /// 探针走命令面：`require` 到的值只在 VM 内可见，能观察它的通道就是「入口把
    /// 结果交给命令面」——顺带把 `modules` 与 `commands` 两条新增面串起来测。
    #[test]
    fn load_injects_the_module_table_so_a_second_module_is_requireable() {
        let image = serde_json::json!({
            "modSource": "local util = require('src.util')\nreturn { commands = { probe = function(_) return util.size == 2 end } }\n",
            "manifest": "",
            "modules": { "src/util": "return { size = 2 }\n" },
            "chunkPaths": { "src/util": "packs/two/src/util.lua" }
        })
        .to_string()
        .into_bytes();
        let mut loaded = load(&image).expect("两模块镜像应装载成功");
        assert_eq!(loaded.bundle().modules.len(), 1);
        assert_eq!(
            loaded
                .bundle()
                .chunk_paths
                .get("src.util")
                .map(String::as_str),
            Some("packs/two/src/util.lua"),
            "chunkPaths 的键应规范化为 require 用的名字"
        );
        assert_eq!(
            loaded.command("probe", "{}").expect("命令面应可达"),
            Vec::new(),
            "probe 只断言、不写世界"
        );
    }

    #[test]
    fn module_entries_are_validated_before_the_loader_sees_them() {
        assert!(
            parse_modules(Some(&serde_json::json!(7))).is_err(),
            "不是对象"
        );
        assert!(
            parse_modules(Some(&serde_json::json!({ "src/a": 1 }))).is_err(),
            "源码必须是字符串"
        );
        assert!(
            parse_modules(Some(&serde_json::json!({ "../a": "return {}" }))).is_err(),
            "名字必须可规范化"
        );
        assert!(parse_modules(None).expect("缺省即空表").is_empty());
        assert!(
            parse_chunk_paths(Some(&serde_json::json!({ "src/a": 1 }))).is_err(),
            "路径必须是字符串"
        );
        assert!(parse_chunk_paths(None).expect("缺省即空表").is_empty());
    }

    /// 互逆与稳定性（单文件 + 模块 + chunkPaths）：
    /// `load(to_image(x))` 与 `x` 同 tick / 同 hash / 同 manifest_hash，
    /// 再 `to_image` 一次字节完全一致。
    #[test]
    fn to_image_round_trips_in_memory_and_is_byte_stable() {
        let raw = serde_json::json!({
            "modSource": "local util = require('src.util')\nworld.component{ name = 'hp', kind = 'u8' }\nworld.spawn{ hp = util.hp }\nworld.seal()\n",
            "manifest": "",
            "modules": { "src/util": "return { hp = 7 }\n" },
            "chunkPaths": { "src/util": "packs/two/src/util.lua" }
        })
        .to_string()
        .into_bytes();

        let first = load(&raw).expect("首次装载");
        let img1 = to_image(&first).expect("首次导出镜像");
        let second = load(img1.as_bytes()).expect("镜像二次装载");
        assert_eq!(second.tick().unwrap(), first.tick().unwrap());
        assert_eq!(second.hash().unwrap(), first.hash().unwrap());
        assert_eq!(second.manifest_hash(), first.manifest_hash());

        let img2 = to_image(&second).expect("二次导出镜像");
        assert_eq!(img1, img2, "连续两次 to_image 必须逐字节稳定");
    }

    /// 真实多文件参考包 `packs/example` 往返：
    /// 带 manifest / schemas / resources / 6 个 Lua 模块的目录装载 → `to_image` → `load`
    /// 后 tick、hash、manifest_hash 全部一致，且再 `to_image` 字节相同。
    #[test]
    fn to_image_round_trips_example_pack_with_stable_bytes() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packs/example");
        let mut from_dir = Loaded::from_dir(&dir).expect("装载 packs/example");
        let img1 = to_image(&from_dir).expect("导出 packs/example 镜像");

        let mut from_img = load(img1.as_bytes()).expect("从镜像装载 packs/example");
        assert_eq!(from_img.tick().unwrap(), from_dir.tick().unwrap());
        assert_eq!(from_img.hash().unwrap(), from_dir.hash().unwrap());
        assert_eq!(from_img.manifest_hash(), from_dir.manifest_hash());

        // 推进几步也必须保持完全一致（证明模块/资源/规则全部等价装入）。
        let r1 = from_dir.step(5, 1).expect("目录装载步进");
        let r2 = from_img.step(5, 1).expect("镜像装载步进");
        assert_eq!(r1.tick, r2.tick);
        assert_eq!(r1.hash, r2.hash);

        let img2 = to_image(&from_img).expect("二次导出镜像");
        assert_eq!(img1, img2, "再导出一次镜像必须逐字节相同");
    }
}
