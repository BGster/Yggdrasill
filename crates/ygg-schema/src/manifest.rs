//! 资源包 manifest（D-A2）：模型 + manifest.json 自身 schema 校验 + 内容寻址校验。
//!
//! - 结构：`{schemaVersion, entries:[{id, kind, schema, path, sha256, byteLength}]}`；
//! - id 用 URI 风格（`ygg:data/act1.dialogues`），schema pattern 校验；
//! - manifest.json 本身过 schema 校验（A2 硬性要求）——先 schema 校验（作者友好
//!   定位，FR-12），后 serde 具型；
//! - 装载序 = sorted manifest 序（D-A2：native 文件系统枚举序与浏览器 bundle
//!   目录序不同，是 native/wasm 分歧通道——装载方绝不按文件序装载，本模型以
//!   [`Manifest::entries_in_load_order`] 提供排序后的确定性装载序）。

use serde::{Deserialize, Serialize};

use crate::validate::{sha256_hex, SchemaError, ValidationIssue, ValidatorSet};

/// manifest schema 的契约 id（registry 与 `$ref` 解析面）。
pub const MANIFEST_SCHEMA_ID: &str = "https://yggdrasill.dev/schemas/v1/manifest.json";

/// manifest schema（draft 2020-12）——单一事实源是
/// `packages/schema/schemas/manifest.json`（D15 跨语言契约源），本常量编译期
/// include_str! 同一文件，禁止在此另写内联副本（铁律 2：不双写 Schema）。
pub const MANIFEST_SCHEMA: &str = include_str!("../../../packages/schema/schemas/manifest.json");

/// manifest 条目引用的资源 payload schema id 前缀（kind→schema 分发面）。
pub const RESOURCE_SCHEMA_PREFIX: &str = "https://yggdrasill.dev/schemas/v1/";

/// 资源包 manifest 模型（D-A2）：schemaVersion + entries（serde 小驼峰同形）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    schema_version: u32,
    entries: Vec<ManifestEntry>,
}

/// 单条资源条目：URI 风格 id + 资源分发 schema id + 内容寻址（sha256/byteLength）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestEntry {
    id: String,
    kind: String,
    schema: String,
    path: String,
    sha256: String,
    byte_length: u64,
}

impl ManifestEntry {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        kind: impl Into<String>,
        schema: impl Into<String>,
        path: impl Into<String>,
        sha256: impl Into<String>,
        byte_length: u64,
    ) -> Self {
        Self {
            id: id.into(),
            kind: kind.into(),
            schema: schema.into(),
            path: path.into(),
            sha256: sha256.into(),
            byte_length,
        }
    }

    /// 资源 id（URI 风格，如 `ygg:data/act1.dialogues`）。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 资源种类标签（自由标注；分发校验以 `schema` id 为准）。
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// 资源 payload 的 schema id（ValidatorSet 分发键）。
    pub fn schema(&self) -> &str {
        &self.schema
    }

    /// 包内相对路径（schema pattern 禁绝绝对路径）。
    pub fn path(&self) -> &str {
        &self.path
    }

    /// 期望内容摘要（小写十六进制 sha256）。
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    /// 期望字节数（廉价预检，先于哈希）。
    pub fn byte_length(&self) -> u64 {
        self.byte_length
    }
}

impl Manifest {
    /// schema 版本（契约演进锚；M0 恒为 1，schema const 校验）。
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// 条目（manifest 原文序；装载请用 [`Manifest::entries_in_load_order`]）。
    pub fn entries(&self) -> &[ManifestEntry] {
        &self.entries
    }

    /// 装载序 = id 升序（D-A2 sorted manifest 序：与文件系统枚举序解耦，
    /// native/wasm 装载序一致）。
    pub fn entries_in_load_order(&self) -> Vec<&ManifestEntry> {
        let mut refs: Vec<&ManifestEntry> = self.entries.iter().collect();
        refs.sort_by(|a, b| a.id.cmp(&b.id));
        refs
    }
}

/// 解析并校验 manifest.json（A2：manifest 自身过 schema）。
///
/// 流程：JSON 解析 → manifest schema 校验（违规 [`SchemaError::InvalidManifest`]
/// 携带 FR-12 信封，`file` 恒为 `manifest.json`）→ serde 具型。
pub fn parse_manifest(bytes: &[u8]) -> Result<Manifest, SchemaError> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    let set = ValidatorSet::from_values([(
        MANIFEST_SCHEMA_ID,
        serde_json::from_str(MANIFEST_SCHEMA).expect("embedded manifest schema is valid JSON"),
    )])?;
    let issues: Vec<ValidationIssue> = set
        .issues(MANIFEST_SCHEMA_ID, "manifest.json", &value)
        .expect("manifest schema id is registered in the one-entry set");
    if !issues.is_empty() {
        let count = issues.len();
        let first = issues
            .first()
            .map(std::string::ToString::to_string)
            .unwrap_or_else(|| "unknown".to_owned());
        return Err(SchemaError::InvalidManifest {
            count,
            first,
            issues,
        });
    }
    Ok(serde_json::from_value(value)?)
}

/// 内容寻址校验（D-A2）：先 byteLength 廉价预检，再 sha256 摘要比对。
///
/// S11 装载面按此逐文件把关：`verify_entry(bytes, entry)` 通过后，payload 才
/// 进入对应 schema 的分发校验。
pub fn verify_entry(bytes: &[u8], entry: &ManifestEntry) -> Result<(), SchemaError> {
    if bytes.len() as u64 != entry.byte_length {
        return Err(SchemaError::ByteLengthMismatch {
            file: entry.path.clone(),
            expected: entry.byte_length,
            actual: bytes.len(),
        });
    }
    let actual = sha256_hex(bytes);
    if actual != entry.sha256 {
        return Err(SchemaError::ContentMismatch {
            file: entry.path.clone(),
            expected: entry.sha256.clone(),
            actual,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_json(id: &str, sha: &str) -> String {
        format!(
            r#"{{"id": "{id}", "kind": "dialogues", "schema": "{RESOURCE_SCHEMA_PREFIX}dialogues.json", "path": "data/act1.dialogues.json", "sha256": "{sha}", "byteLength": 11}}"#
        )
    }

    fn sha_of(content: &str) -> String {
        sha256_hex(content.as_bytes())
    }

    #[test]
    fn manifest_schema_carries_draft_2020_12_anchor() {
        // Parity 守卫锚：契约文本必须携带 draft 2020-12 元锚（构造期另强制）。
        assert!(MANIFEST_SCHEMA
            .contains(r#""$schema": "https://json-schema.org/draft/2020-12/schema""#));
        assert!(MANIFEST_SCHEMA.contains("draft/2020-12/schema"));
    }

    #[test]
    fn valid_manifest_parses_and_load_order_is_sorted_by_id() {
        let c1 = "hello world"; // 11 字节
        let c2 = "hello again"; // 11 字节
        let manifest_json = format!(
            r#"{{"schemaVersion": 1, "entries": [{}, {}]}}"#,
            entry_json("ygg:data/b.txt", &sha_of(c2)),
            entry_json("ygg:data/a.txt", &sha_of(c1)),
        );
        let manifest = parse_manifest(manifest_json.as_bytes()).unwrap();
        assert_eq!(manifest.schema_version(), 1);
        assert_eq!(manifest.entries().len(), 2);
        // 原文序 = b 在前；装载序 = id 升序（a 在前）
        assert_eq!(manifest.entries()[0].id(), "ygg:data/b.txt");
        let order: Vec<_> = manifest
            .entries_in_load_order()
            .iter()
            .map(|e| e.id().to_owned())
            .collect();
        assert_eq!(order, ["ygg:data/a.txt", "ygg:data/b.txt"]);
    }

    #[test]
    fn violations_carry_file_pointer_schema_location_and_hint() {
        let bad_sha = "Z".repeat(64); // pattern 违规（非小写十六进制）
        let manifest_json = format!(
            r#"{{"schemaVersion": 1, "entries": [{}]}}"#,
            entry_json("ygg:data/a.txt", &bad_sha)
        );
        let err = parse_manifest(manifest_json.as_bytes()).unwrap_err();
        let SchemaError::InvalidManifest {
            issues,
            count,
            first,
            ..
        } = err
        else {
            panic!("expected InvalidManifest");
        };
        assert_eq!(count, 1);
        assert!(first.contains("manifest.json"));
        let issue = &issues[0];
        assert_eq!(issue.file, "manifest.json");
        assert_eq!(issue.pointer, "/entries/0/sha256");
        assert!(issue.schema_location.starts_with("https://"));
        assert_eq!(issue.code, "Pattern");
        assert!(!issue.hint.is_empty());
        assert!(!issue.message.is_empty());
    }

    #[test]
    fn missing_field_and_unknown_keywords_are_rejected() {
        // 缺 byteLength：Required 违规 + 指针指向条目
        let no_len = format!(
            r#"{{"schemaVersion": 1, "entries": [{{"id": "ygg:a", "kind": "k", "schema": "{RESOURCE_SCHEMA_PREFIX}x.json", "path": "p", "sha256": "{}"}}]}}"#,
            "0".repeat(64)
        );
        let err = parse_manifest(no_len.as_bytes()).unwrap_err();
        let SchemaError::InvalidManifest { issues, .. } = err else {
            panic!("expected InvalidManifest");
        };
        assert_eq!(issues[0].code, "Required");
        assert!(issues[0].pointer.starts_with("/entries/0"));
        // 未知顶层字段：additionalProperties 关死
        let unknown = r#"{"schemaVersion": 1, "entries": [], "bonus": true}"#;
        let err = parse_manifest(unknown.as_bytes()).unwrap_err();
        assert!(matches!(err, SchemaError::InvalidManifest { .. }));
        // 错误 schemaVersion：const 违规
        let bad_version = r#"{"schemaVersion": 2, "entries": []}"#;
        assert!(matches!(
            parse_manifest(bad_version.as_bytes()),
            Err(SchemaError::InvalidManifest { .. })
        ));
    }

    #[test]
    fn verify_entry_checks_byte_length_then_hash() {
        let content = "hello world"; // 11 字节
        let sha = sha_of(content);
        let entry = ManifestEntry::new(
            "ygg:data/act1.dialogues",
            "dialogues",
            format!("{RESOURCE_SCHEMA_PREFIX}dialogues.json"),
            "data/act1.dialogues.json",
            &sha,
            content.len() as u64,
        );
        // 全对：通过
        assert!(verify_entry(content.as_bytes(), &entry).is_ok());
        // 字节数不符：先于哈希报 ByteLengthMismatch
        let err = verify_entry(b"hello", &entry).unwrap_err();
        assert!(matches!(
            err,
            SchemaError::ByteLengthMismatch {
                expected: 11,
                actual: 5,
                ..
            }
        ));
        // 字节数同但内容被篡改：ContentMismatch 带文件路径
        let err = verify_entry(b"hell0 world", &entry).unwrap_err();
        let SchemaError::ContentMismatch { file, .. } = err else {
            panic!("expected ContentMismatch");
        };
        assert_eq!(file, "data/act1.dialogues.json");
        // 摘要生成与校验同口径：sha256_hex 产出可作 manifest sha256 字段
        assert_eq!(entry.sha256(), &sha);
    }
}
