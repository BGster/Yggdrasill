//! S11 ygg.data 资源面（D-资源-*）：manifest + 内容寻址 + schema 分发校验的
//! **只读数据**装载面。
//!
//! - **资源是数据不是状态**（本模块的单一不变量）：装载期一次性构建、构建后
//!   不可变（[`ResourceTable`] 无任何写面）；**不进** canonical store、**不进**
//!   存档、**不进** 状态哈希——读档不改资源，`save` 不携带资源，两轮 dispatch
//!   的 reply/canonical 字节与资源内容无关（测试面锁定）。
//! - **预算门（最廉价失败优先）**：manifest + 全部资源字节总量 ≤
//!   [`RESOURCES_MAX_BYTES`]（1 MiB），超限即 [`ResourceError::BudgetExceeded`]
//!   （驱动侧单点映射 `KV_LIMIT`——预留码激活点，见 `driver::KernelError::code`）。
//! - **装载管线**（[`ResourceTable::build`] 单点收口，顺序即语义）：
//!   ① 预算 → ② [`parse_manifest`]（manifest 自身 schema 校验，A2）→
//!   ③ [`ValidatorSet::from_texts`]（draft 锚 / 离线 / 线性正则，构造期拒绝）→
//!   ④ 按 [`Manifest::entries_in_load_order`]（D-A2 sorted 序）逐条：
//!   取字节（缺失 = Content，文案带 path）→ [`verify_entry`]（byteLength
//!   廉价预检先于 sha256）→ JSON 解析 → schema 分发校验（未知 schema id =
//!   [`ResourceError::UnknownSchema`]；非空违规 = [`ResourceError::Payload`]）
//!   → 注册进 BTreeMap（确定性容器，无随机化哈希面）。
//! - **Lua 面**（[`install`]）：`ygg.data(id)` 单函数——id 命中返回资源表
//!   （JSON→Lua 确定性转换：对象→表且按**排序键序**插入、数组→表 1..n、
//!   字符串/布尔直通、数字整数优先 i64、null→Lua nil）；id 缺失 / 非字符串 /
//!   空串均以**可读错误**拒绝（文案带 id）。
//! - **接线序**（S11 锁定，`driver::build_kernel` 承载）：`host_api::install`
//!   **之后**（依赖 ygg 占位表就位）、`stdlib::install` **之前**、Mod chunk
//!   之前——Mod 顶层即可读 `ygg.data`（装载期快照，handler 经 local 闭包读取）。

use std::collections::BTreeMap;

use mlua::{Error as LuaError, Lua, Table, Value as LuaValue};
use thiserror::Error as ThisError;

use ygg_schema::manifest::{parse_manifest, verify_entry, ManifestEntry};
use ygg_schema::validate::{SchemaError, ValidationIssue, ValidatorSet};

use serde_json::Value as Json;

/// 资源包预算上限：manifest + 全部资源字节总量（D-资源-S11 锁定 1 MiB）。
pub const RESOURCES_MAX_BYTES: usize = 1 << 20;

/// 资源包构建错误（thiserror，白名单内；驱动侧经 `KernelError` 单点映射
/// A1 稳定码——本枚举不直接出现在 Lua 信封）。
#[derive(Debug, ThisError)]
pub enum ResourceError {
    /// manifest 解析 / schema 集构造失败（含 UnsupportedDraft / SchemaCompile）。
    #[error("resource manifest rejected: {0}")]
    Manifest(SchemaError),
    /// 内容寻址失败（字节面：文件缺失 / byteLength 不符 / sha256 不符）。
    #[error("resource content rejected: {0}")]
    Content(SchemaError),
    /// payload schema 校验失败（FR-12 信封逐条携带；file = 资源路径）。
    #[error(
        "resource `{file}` rejected with {} violation(s): first — {}",
        .issues.len(),
        .issues.first().map(std::string::ToString::to_string).unwrap_or_else(|| "unknown".to_owned())
    )]
    Payload {
        file: String,
        issues: Vec<ValidationIssue>,
    },
    /// 分发 schema id 未在 supplied schema 文本集中注册。
    #[error("unknown resource schema id `{uri}` (not among the supplied schema texts)")]
    UnknownSchema { uri: String },
    /// 预算超限（total = manifest + 全部资源字节；驱动侧映射 KV_LIMIT）。
    #[error("resource budget exceeded: {total} bytes requested, limit {limit}")]
    BudgetExceeded { total: usize, limit: usize },
}

/// 只读资源表：资源 id → 已解析的 JSON 值（BTreeMap 确定性容器）。
///
/// 构建后不可变（无写面）；**不属于状态**——不入 canonical store / 存档 /
/// 状态哈希（见模块文档）。
#[derive(Debug)]
pub struct ResourceTable {
    entries: BTreeMap<String, Json>,
}

impl ResourceTable {
    /// 空资源表（无资源装载路径的驱动基线，`KernelFacade::new` 复用）。
    pub fn empty() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// 按锁定管线构建资源表（见模块文档；装载序 = manifest sorted 序）。
    ///
    /// `schemas` = `(schema id → schema JSON 文本)`（文本必须携带 2020-12 元锚）；
    /// `resources` = `(包内相对路径 → 资源字节)`（路径 = manifest `entry.path`
    /// 的查找键）。
    pub fn build(
        manifest_bytes: &[u8],
        schemas: &[(String, String)],
        resources: &[(String, Vec<u8>)],
    ) -> Result<Self, ResourceError> {
        // ① 预算门（最廉价失败优先：不做任何解析先量字节）。
        let total = manifest_bytes.len()
            + resources
                .iter()
                .map(|(_, bytes)| bytes.len())
                .sum::<usize>();
        if total > RESOURCES_MAX_BYTES {
            return Err(ResourceError::BudgetExceeded {
                total,
                limit: RESOURCES_MAX_BYTES,
            });
        }
        // ② manifest 解析 + 自身 schema 校验（A2）。
        let manifest = parse_manifest(manifest_bytes).map_err(ResourceError::Manifest)?;
        // ③ schema 集构造（draft 锚 / 离线 $ref / 线性正则，构造期即拒绝）。
        let set =
            ValidatorSet::from_texts(schemas.iter().cloned()).map_err(ResourceError::Manifest)?;
        // ④ 装载序逐条（D-A2 sorted 序：native/wasm 装载序一致）。
        let mut entries = BTreeMap::new();
        for entry in manifest.entries_in_load_order() {
            let bytes = lookup_bytes(resources, entry)?;
            // 内容寻址：byteLength 廉价预检先于 sha256（ygg-schema 单点）。
            verify_entry(bytes, entry).map_err(ResourceError::Content)?;
            // JSON 解析 + schema 分发校验（作者哈希过但内容非 JSON 的退化路径
            // 归 Content——内容寻址已过、语义面仍拒绝，文案落在 Json 解析面）。
            let value: Json = serde_json::from_slice(bytes)
                .map_err(|e| ResourceError::Content(SchemaError::Json(e)))?;
            let issues =
                set.issues(entry.schema(), entry.path(), &value)
                    .map_err(|err| match err {
                        SchemaError::UnknownSchema(uri) => ResourceError::UnknownSchema { uri },
                        other => ResourceError::Manifest(other),
                    })?;
            if !issues.is_empty() {
                return Err(ResourceError::Payload {
                    file: entry.path().to_owned(),
                    issues,
                });
            }
            entries.insert(entry.id().to_owned(), value);
        }
        Ok(Self { entries })
    }

    /// 资源 id 的只读读取面（miss = None；Lua 侧经 `ygg.data` 以可读错误拒绝）。
    pub fn get(&self, id: &str) -> Option<&Json> {
        self.entries.get(id)
    }

    /// 是否无资源（诊断面）。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// 按条目 path 取资源字节（缺失 = Content，文案带 path——用
/// ByteLengthMismatch{actual: 0} 表达「期望 N 字节、实得 0（文件缺失）」）。
fn lookup_bytes<'a>(
    resources: &'a [(String, Vec<u8>)],
    entry: &ManifestEntry,
) -> Result<&'a [u8], ResourceError> {
    resources
        .iter()
        .find(|(path, _)| path.as_str() == entry.path())
        .map(|(_, bytes)| bytes.as_slice())
        .ok_or_else(|| {
            ResourceError::Content(SchemaError::ByteLengthMismatch {
                file: entry.path().to_owned(),
                expected: entry.byte_length(),
                actual: 0,
            })
        })
}

/// 在 Lua 状态上安装 `ygg.data(id)`（S11 接线序见模块文档；调用方为
/// `driver::build_kernel` 单点）。
///
/// `ygg.data` 为**单函数**：id 命中 → 返回资源表（JSON→Lua 确定性转换）；
/// id 缺失 / 非字符串 / 空串 → 可读错误（文案带 id / 实参类型）。资源数据
/// 在 install 时捕获进闭包——装载期快照，构建后与 Rust 侧表无共享可变面。
pub fn install(lua: &Lua, table: &ResourceTable) -> Result<(), LuaError> {
    let entries = table.entries.clone();
    let ygg: Table = lua.globals().raw_get("ygg")?;
    ygg.raw_set(
        "data",
        lua.create_function(move |lua, arg: LuaValue| -> Result<LuaValue, LuaError> {
            let id = match &arg {
                LuaValue::String(s) => s.to_str()?.to_owned(),
                other => {
                    return Err(LuaError::runtime(format!(
                        "ygg.data(id) requires a string resource id, got {}",
                        lua_value_kind(other)
                    )));
                }
            };
            if id.is_empty() {
                return Err(LuaError::runtime("ygg.data: resource id must not be empty"));
            }
            let value = entries.get(&id).ok_or_else(|| {
                LuaError::runtime(format!("ygg.data: unknown resource id `{id}`"))
            })?;
            json_to_lua(lua, value)
        })?,
    )?;
    Ok(())
}

/// JSON → mlua 值（确定性转换面，模块文档锁定）：
/// 对象 → 表（BTreeMap **排序键序**插入，native/wasm 序一致）；
/// 数组 → 表（1..n）；字符串 / 布尔直通；数字整数优先 i64（其余 f64）；
/// null → Lua nil。递归深度受 serde_json 128 层解析上限约束（构造期即拒）。
fn json_to_lua(lua: &Lua, value: &Json) -> Result<LuaValue, LuaError> {
    Ok(match value {
        Json::Null => LuaValue::Nil,
        Json::Bool(b) => LuaValue::Boolean(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) => LuaValue::Integer(i),
            None => LuaValue::Number(n.as_f64().unwrap_or_default()),
        },
        Json::String(s) => LuaValue::String(lua.create_string(s)?),
        Json::Array(items) => {
            let out = lua.create_table()?;
            for (index, item) in items.iter().enumerate() {
                out.raw_set((index + 1) as i64, json_to_lua(lua, item)?)?;
            }
            LuaValue::Table(out)
        }
        Json::Object(map) => {
            // 排序键序插入（serde_json Map 迭代序不作依赖面——显式 BTreeMap）。
            let sorted: BTreeMap<String, Json> =
                map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            let out = lua.create_table()?;
            for (key, item) in &sorted {
                out.raw_set(key.as_str(), json_to_lua(lua, item)?)?;
            }
            LuaValue::Table(out)
        }
    })
}

/// mlua 值的 Lua 类型名（可读报错文案；host_api 内有同形私有面，此处独立
/// 不外溢——两处均为 &'static 字面量映射，无漂移风险）。
fn lua_value_kind(value: &LuaValue) -> &'static str {
    match value {
        LuaValue::Nil => "nil",
        LuaValue::Boolean(_) => "boolean",
        LuaValue::Integer(_) | LuaValue::Number(_) => "number",
        LuaValue::String(_) => "string",
        LuaValue::Table(_) => "table",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMA_ID: &str = "https://yggdrasill.dev/schemas/v1/unit.json";
    const SCHEMA_TEXT: &str = r#"{
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://yggdrasill.dev/schemas/v1/unit.json",
        "type": "object",
        "required": ["name"],
        "properties": { "name": { "type": "string" } }
    }"#;

    #[test]
    fn budget_gate_fires_before_any_parsing() {
        // 预算门最廉价失败优先：fixture 用**非法 JSON manifest**——若门在
        // 解析之后，本案会先以 Manifest(Json) 失败；实测报 BudgetExceeded
        // 即证「先量字节、后解析」的顺序契约（S11 审计代决 (a) 补强）。
        let big = vec![b'x'; RESOURCES_MAX_BYTES];
        let manifest: &[u8] = b"{ not json at all";
        let err = ResourceTable::build(
            manifest,
            &[(SCHEMA_ID.to_owned(), SCHEMA_TEXT.to_owned())],
            &[("data/big.json".to_owned(), big)],
        )
        .unwrap_err();
        let ResourceError::BudgetExceeded { total, limit } = err else {
            panic!("expected BudgetExceeded");
        };
        assert_eq!(limit, RESOURCES_MAX_BYTES);
        assert_eq!(total, manifest.len() + RESOURCES_MAX_BYTES);
        assert!(total > limit);
    }

    #[test]
    fn missing_resource_file_names_the_path() {
        let content = b"{}";
        let sha = ygg_schema::validate::sha256_hex(content);
        let manifest = format!(
            r#"{{"schemaVersion": 1, "entries": [{{"id": "ygg:data/a", "kind": "k", "schema": "{SCHEMA_ID}", "path": "data/a.json", "sha256": "{sha}", "byteLength": {}}}]}}"#,
            content.len()
        );
        let err = ResourceTable::build(
            manifest.as_bytes(),
            &[(SCHEMA_ID.to_owned(), SCHEMA_TEXT.to_owned())],
            // 未提供 data/a.json 对应字节 → Content 且文案带路径。
            &[],
        )
        .unwrap_err();
        let ResourceError::Content(SchemaError::ByteLengthMismatch { file, actual, .. }) = err
        else {
            panic!("expected Content(ByteLengthMismatch)");
        };
        assert_eq!(file, "data/a.json");
        assert_eq!(actual, 0);
    }

    #[test]
    fn build_get_roundtrip_and_empty_baseline() {
        let content = br#"{"name": "unit"}"#;
        let sha = ygg_schema::validate::sha256_hex(content);
        let manifest = format!(
            r#"{{"schemaVersion": 1, "entries": [{{"id": "ygg:data/a", "kind": "k", "schema": "{SCHEMA_ID}", "path": "data/a.json", "sha256": "{sha}", "byteLength": {}}}]}}"#,
            content.len()
        );
        let table = ResourceTable::build(
            manifest.as_bytes(),
            &[(SCHEMA_ID.to_owned(), SCHEMA_TEXT.to_owned())],
            &[("data/a.json".to_owned(), content.to_vec())],
        )
        .expect("build");
        assert!(!table.is_empty());
        assert_eq!(table.get("ygg:data/a").unwrap()["name"], "unit");
        assert!(table.get("ygg:data/missing").is_none());
        assert!(ResourceTable::empty().is_empty());
    }
}
