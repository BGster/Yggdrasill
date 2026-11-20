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
///
/// `code`（r2）是**代码模块**的内容寻址清单，与 `entries` 同构但独立：entries
/// 管数据资源（manifest 必填），code 管可 `require` 的模块源码（可选，缺失即
/// 单文件 Mod）。两者不混入同一数组——资源的 schema 分发校验与模块的
/// `ModuleSet` 登记是两条装载面，混装会让「谁是模块」要靠 kind 猜。
///
/// `packages`（r3）是**第二个受控模块命名空间**（`pkg.<id>.*`）的内容寻址清单：
/// 每个声明的包一条，`files` 是包内非 `test/**` 文件 + `pkg.json`。它与 `code`
/// 分开的理由同上（mod 自有模块 vs 可复用的包模块是两个装载身份），且 `path`
/// 相对**包根**、以包 id 打头，与 `codeEntry.path` 的相对**mod 根**不同源。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    schema_version: u32,
    entries: Vec<ManifestEntry>,
    /// 代码模块清单（r2；缺省 = 单文件 Mod，旧 manifest 逐字节可解析）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    code: Vec<ManifestCodeEntry>,
    /// 包文件清单（r3；缺省 = 不声明包，旧 manifest 逐字节可解析）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    packages: Vec<ManifestPackage>,
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

    /// 代码模块清单（r2；空 = 单文件 Mod，require 对任何名字可读拒绝）。
    ///
    /// 顺序不作语义面：装载方按 `path` 查表（`ygg-sandbox::pack::assemble`），
    /// 不依赖本数组的排列。
    pub fn code(&self) -> &[ManifestCodeEntry] {
        &self.code
    }

    /// 包文件清单（r3；空 = 本 Mod 不声明任何包）。
    ///
    /// 顺序不作语义面：装载方按 `(id, path)` 查表（`ygg-sandbox::packages`），
    /// 不依赖本数组的排列。
    pub fn packages(&self) -> &[ManifestPackage] {
        &self.packages
    }

    /// 装载序 = id 升序（D-A2 sorted manifest 序：与文件系统枚举序解耦，
    /// native/wasm 装载序一致）。
    pub fn entries_in_load_order(&self) -> Vec<&ManifestEntry> {
        let mut refs: Vec<&ManifestEntry> = self.entries.iter().collect();
        refs.sort_by(|a, b| a.id.cmp(&b.id));
        refs
    }
}

/// 单条代码模块条目（r2）：包内相对路径 + 内容寻址（sha256/byteLength）。
///
/// 与 [`ManifestEntry`] 的差别只在**没有** id/kind/schema——模块源码是代码不是
/// 数据，没有 schema 分发面（它的合法性由装载器在装载期按 Lua 语法判定）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestCodeEntry {
    path: String,
    sha256: String,
    byte_length: u64,
}

impl ManifestCodeEntry {
    pub fn new(path: impl Into<String>, sha256: impl Into<String>, byte_length: u64) -> Self {
        Self {
            path: path.into(),
            sha256: sha256.into(),
            byte_length,
        }
    }

    /// 包内相对路径（如 `src/util.lua`）。
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

/// 单条包文件条目（r3）：相对**包根**的路径 + 内容寻址（sha256/byteLength）。
///
/// `path` 以包 id 打头（`fixture/init.lua`）——这条落在 schema pattern 与
/// 装载期校验里，不是命名建议：载荷哈希只对「哪个包的哪个文件」有意义，
/// 丢掉 id 前缀就只剩一个包内相对路径。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestPackage {
    id: String,
    files: Vec<ManifestPackageFile>,
}

impl ManifestPackage {
    pub fn new(id: impl Into<String>, files: Vec<ManifestPackageFile>) -> Self {
        Self {
            id: id.into(),
            files,
        }
    }

    /// 包 id（= 包根下的目录名，`packages/libs/<id>/`）。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 本包纳入内容寻址的文件（按 `path` 序；装载查表不依赖顺序）。
    pub fn files(&self) -> &[ManifestPackageFile] {
        &self.files
    }
}

/// 单条包文件条目（r3）：`path` 相对包根且以包 id 打头（`fixture/init.lua`）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestPackageFile {
    path: String,
    sha256: String,
    byte_length: u64,
}

impl ManifestPackageFile {
    pub fn new(path: impl Into<String>, sha256: impl Into<String>, byte_length: u64) -> Self {
        Self {
            path: path.into(),
            sha256: sha256.into(),
            byte_length,
        }
    }

    /// 相对包根的路径（如 `fixture/init.lua`）。
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
    verify_content(entry.path(), entry.sha256(), entry.byte_length(), bytes)
}

/// 代码模块的内容寻址校验（r2，同一套判定：byteLength 先于 sha256）。
pub fn verify_code_entry(bytes: &[u8], entry: &ManifestCodeEntry) -> Result<(), SchemaError> {
    verify_content(entry.path(), entry.sha256(), entry.byte_length(), bytes)
}

/// 包文件的内容寻址校验（r3，同一套判定：byteLength 先于 sha256）。
///
/// `path` 是包根相对路径（`fixture/init.lua`），所以文案里的 `file` 也是它
/// ——与数据资源（mod 根相对）、mod 模块（mod 根相对）三条面但**一套口径**。
pub fn verify_package_file(bytes: &[u8], entry: &ManifestPackageFile) -> Result<(), SchemaError> {
    verify_content(entry.path(), entry.sha256(), entry.byte_length(), bytes)
}

/// 内容寻址判定单点（[`verify_entry`] / [`verify_code_entry`] /
/// [`verify_package_file`] 共用——三条装载面但一套口径，避免「数据校验严、
/// 代码校验松」这类分叉）。
fn verify_content(
    path: &str,
    expected_sha256: &str,
    expected_len: u64,
    bytes: &[u8],
) -> Result<(), SchemaError> {
    if bytes.len() as u64 != expected_len {
        return Err(SchemaError::ByteLengthMismatch {
            file: path.to_owned(),
            expected: expected_len,
            actual: bytes.len(),
        });
    }
    let actual = sha256_hex(bytes);
    if actual != expected_sha256 {
        return Err(SchemaError::ContentMismatch {
            file: path.to_owned(),
            expected: expected_sha256.to_owned(),
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

    #[test]
    fn code_array_is_optional_and_round_trips() {
        // 缺 code：旧 manifest 逐字节不变（多文件 Mod 上线前的形态）。
        let legacy = format!(
            r#"{{"schemaVersion": 1, "entries": [{}]}}"#,
            entry_json("ygg:data/a.txt", &sha_of("hello world"))
        );
        let manifest = parse_manifest(legacy.as_bytes()).unwrap();
        assert!(manifest.code().is_empty());

        // 带 code：schema 放行，具型后可按 path/sha256/byteLength 读。
        let code_text = "return {}";
        let with_code = format!(
            r#"{{"schemaVersion": 1, "entries": [{}], "code": [{{"path": "src/util.lua", "sha256": "{}", "byteLength": {}}}]}}"#,
            entry_json("ygg:data/a.txt", &sha_of("hello world")),
            sha_of(code_text),
            code_text.len()
        );
        let manifest = parse_manifest(with_code.as_bytes()).unwrap();
        assert_eq!(manifest.code().len(), 1);
        assert_eq!(manifest.code()[0].path(), "src/util.lua");
        assert_eq!(manifest.code()[0].byte_length(), code_text.len() as u64);
        // 条目与模块是两个独立数组：code 不污染 entries 装载面。
        assert_eq!(manifest.entries().len(), 1);
    }

    #[test]
    fn code_entry_violations_point_at_the_module_array() {
        let bad = format!(
            r#"{{"schemaVersion": 1, "entries": [{}], "code": [{{"path": "src/util.lua", "sha256": "{}", "byteLength": 11}}]}}"#,
            entry_json("ygg:data/a.txt", &sha_of("hello world")),
            "Z".repeat(64)
        );
        let err = parse_manifest(bad.as_bytes()).unwrap_err();
        let SchemaError::InvalidManifest { issues, .. } = err else {
            panic!("expected InvalidManifest");
        };
        assert_eq!(issues[0].pointer, "/code/0/sha256");

        // 非 .lua 路径：schema pattern 拦下（模块清单只装 Lua 源码）。
        let not_lua = format!(
            r#"{{"schemaVersion": 1, "entries": [{}], "code": [{{"path": "src/util.txt", "sha256": "{}", "byteLength": 11}}]}}"#,
            entry_json("ygg:data/a.txt", &sha_of("hello world")),
            sha_of("hello world")
        );
        let err = parse_manifest(not_lua.as_bytes()).unwrap_err();
        let SchemaError::InvalidManifest { issues, .. } = err else {
            panic!("expected InvalidManifest");
        };
        assert_eq!(issues[0].pointer, "/code/0/path");
    }

    #[test]
    fn verify_code_entry_uses_the_same_content_addressing_rules() {
        let content = b"return {}\n";
        let entry =
            ManifestCodeEntry::new("src/util.lua", sha256_hex(content), content.len() as u64);
        assert!(verify_code_entry(content, &entry).is_ok());
        // byteLength 先于 sha256（廉价预检优先）。
        let err = verify_code_entry(b"return {}", &entry).unwrap_err();
        assert!(matches!(err, SchemaError::ByteLengthMismatch { .. }));
        // 字节数同、内容换：ContentMismatch 带模块路径。
        let err = verify_code_entry(b"return []\n", &entry).unwrap_err();
        let SchemaError::ContentMismatch { file, .. } = err else {
            panic!("expected ContentMismatch");
        };
        assert_eq!(file, "src/util.lua");
    }

    #[test]
    fn packages_array_is_optional_so_old_manifests_parse_unchanged() {
        // 缺 packages：旧 manifest 逐字节可解析，packages() 为空。
        let legacy = format!(
            r#"{{"schemaVersion": 1, "entries": [{}]}}"#,
            entry_json("ygg:data/a.txt", &sha_of("hello world"))
        );
        let manifest = parse_manifest(legacy.as_bytes()).unwrap();
        assert!(manifest.packages().is_empty());
    }

    #[test]
    fn packages_array_round_trips_and_carries_id_prefixed_paths() {
        let init = "return {}";
        let pkg = "{\"schemaVersion\": 1}";
        let with_pkgs = format!(
            r#"{{"schemaVersion": 1, "entries": [{}], "packages": [{{"id": "fixture", "files": [
                {{"path": "fixture/init.lua", "sha256": "{}", "byteLength": {}}},
                {{"path": "fixture/pkg.json", "sha256": "{}", "byteLength": {}}}
            ]}}]}}"#,
            entry_json("ygg:data/a.txt", &sha_of("hello world")),
            sha_of(init),
            init.len(),
            sha_of(pkg),
            pkg.len(),
        );
        let manifest = parse_manifest(with_pkgs.as_bytes()).unwrap();
        assert_eq!(manifest.packages().len(), 1);
        let package = &manifest.packages()[0];
        assert_eq!(package.id(), "fixture");
        assert_eq!(package.files().len(), 2);
        assert_eq!(package.files()[0].path(), "fixture/init.lua");
        assert_eq!(package.files()[0].byte_length(), init.len() as u64);
        // 包清单与 entries / code 是三个独立数组：互不污染。
        assert_eq!(manifest.entries().len(), 1);
        assert!(manifest.code().is_empty());
    }

    #[test]
    fn package_entries_violations_point_at_the_packages_section() {
        // 段内不允许 `.` / `..` 段：pattern 要求 `^<id>/…[.](lua|json)$`，
        // 而 `../` 既不以小写 id 起头也不结尾于扩展名。
        let traversal = format!(
            r#"{{"schemaVersion": 1, "entries": [{}], "packages": [{{"id": "fixture", "files": [
                {{"path": "../escape.lua", "sha256": "{}", "byteLength": 11}}
            ]}}]}}"#,
            entry_json("ygg:data/a.txt", &sha_of("hello world")),
            sha_of("hello world")
        );
        let err = parse_manifest(traversal.as_bytes()).unwrap_err();
        let SchemaError::InvalidManifest { issues, .. } = err else {
            panic!("expected InvalidManifest");
        };
        assert_eq!(issues[0].pointer, "/packages/0/files/0/path");
        assert_eq!(issues[0].code, "Pattern");

        // 大写 id：包 id 与目录名同源，只允许 `^[a-z][a-z0-9_-]*$`。
        let bad_id = format!(
            r#"{{"schemaVersion": 1, "entries": [{}], "packages": [{{"id": "Fixture", "files": []}}]}}"#,
            entry_json("ygg:data/a.txt", &sha_of("hello world"))
        );
        let err = parse_manifest(bad_id.as_bytes()).unwrap_err();
        let SchemaError::InvalidManifest { issues, .. } = err else {
            panic!("expected InvalidManifest");
        };
        assert_eq!(issues[0].pointer, "/packages/0/id");

        // 路径必须以包 id 打头：`other/init.lua` 在一个 `fixture` 包里是错位登记。
        let mismatched = format!(
            r#"{{"schemaVersion": 1, "entries": [{}], "packages": [{{"id": "fixture", "files": [
                {{"path": "init.lua", "sha256": "{}", "byteLength": 11}}
            ]}}]}}"#,
            entry_json("ygg:data/a.txt", &sha_of("hello world")),
            sha_of("hello world")
        );
        let err = parse_manifest(mismatched.as_bytes()).unwrap_err();
        let SchemaError::InvalidManifest { issues, .. } = err else {
            panic!("expected InvalidManifest");
        };
        assert_eq!(issues[0].pointer, "/packages/0/files/0/path");
    }

    #[test]
    fn verify_package_file_reuses_the_one_content_addressing_ruleset() {
        let content = b"return {}\n";
        let entry = ManifestPackageFile::new(
            "fixture/init.lua",
            sha256_hex(content),
            content.len() as u64,
        );
        assert!(verify_package_file(content, &entry).is_ok());
        // byteLength 先于 sha256。
        let err = verify_package_file(b"return {}", &entry).unwrap_err();
        assert!(matches!(err, SchemaError::ByteLengthMismatch { .. }));
        // 字节数同、内容换：ContentMismatch 带包根相对路径。
        let err = verify_package_file(b"return []\n", &entry).unwrap_err();
        let SchemaError::ContentMismatch { file, .. } = err else {
            panic!("expected ContentMismatch");
        };
        assert_eq!(file, "fixture/init.lua");
    }
}
