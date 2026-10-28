//! 资源校验机器（FR6 / FR12）：schema 注册表 + draft 2020-12 锁定 + 错误信封。
//!
//! - **schema 注册表**：全部 schema 收进 registry（离线 `$ref` 解析面），每份
//!   schema 单独编译为 validator（构造期即拒绝：draft 非 2020-12、未注册的外部
//!   `$ref`、回溯型正则特性——见 crate 文档的 parity 纪律）；
//! - **错误信封**（FR-12）：校验失败归一为
//!   `{file, pointer, schemaLocation, code, params, message, hint}` 七字段——
//!   pointer 指向实例出错位置（JSON Pointer），schemaLocation 指向生效的 schema
//!   关键字（绝对位置），code 为稳定判别码，hint 为作者修复指引；
//! - **内容寻址**（D-A2）：`verify_sha256` / `sha256_hex` 落在本层——内容寻址
//!   是 manifest 契约的组成部分，随 manifest 模型同层交付（sha2 依赖归属见
//!   D-继承-5 的 S6 amendment：状态哈希归沙箱 crate，内容寻址归本 crate，
//!   两处独立引入、互不共享）。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// draft 2020-12 元锚 URI：schema 文本必须携带（parity 守卫，构造期强制）。
pub const DRAFT_2020_12_META: &str = "https://json-schema.org/draft/2020-12/schema";

/// 校验错误（thiserror，白名单内）。
#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    #[error("manifest/resource json parse failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("schema `{uri}` must carry the draft 2020-12 `$schema` anchor")]
    UnsupportedDraft { uri: String },
    #[error("schema `{uri}` compile failed: {message}")]
    SchemaCompile { uri: String, message: String },
    #[error("schema registry build failed: {0}")]
    Registry(String),
    #[error("unknown schema id `{0}` (not in the validator set)")]
    UnknownSchema(String),
    #[error("content size mismatch for `{file}`: expected {expected} bytes, got {actual}")]
    ByteLengthMismatch {
        file: String,
        expected: u64,
        actual: usize,
    },
    #[error("content hash mismatch for `{file}`: expected {expected}, got {actual}")]
    ContentMismatch {
        file: String,
        expected: String,
        actual: String,
    },
    #[error("manifest rejected with {count} violation(s): first — {first}")]
    InvalidManifest {
        count: usize,
        first: String,
        issues: Vec<ValidationIssue>,
    },
    #[error("resource `{file}` rejected with {count} violation(s): first — {first}")]
    InvalidResource {
        file: String,
        count: usize,
        first: String,
        issues: Vec<ValidationIssue>,
    },
}

/// FR-12 错误信封（七字段，作者友好）：序列化键为小驼峰（与 TS 侧信封同形）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationIssue {
    /// 出错文件（manifest.json 或资源路径）。
    pub file: String,
    /// 实例出错位置（JSON Pointer，如 `/entries/0/sha256`）。
    pub pointer: String,
    /// 生效的 schema 关键字绝对位置（作者可据此回查规则）。
    pub schema_location: String,
    /// 稳定判别码（来自校验器错误类别名，如 `Required` / `Pattern`）。
    pub code: String,
    /// 判别参数（错误类别的操作数原文，机器可读）。
    pub params: serde_json::Value,
    /// 人类可读错误消息（校验器原文）。
    pub message: String,
    /// 修复指引（按判别码映射的作者侧提示）。
    pub hint: String,
}

impl std::fmt::Display for ValidationIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} at {}: {} (code: {})",
            self.file, self.pointer, self.message, self.code
        )
    }
}

/// schema 校验器集合：id → validator（确定性容器；校验层无随机化哈希面）。
///
/// 构造期强制 parity 纪律（draft 锚 / 离线 / format 关闭 / 线性正则），
/// 构造失败即 [`SchemaError`]，运行期不再有任何配置分支。
pub struct ValidatorSet {
    registry: jsonschema::Registry<'static>,
    validators: BTreeMap<String, jsonschema::Validator>,
}

impl ValidatorSet {
    /// 以 `(schema id → schema JSON 文本)` 构造。
    ///
    /// 每份 schema 文本：① 必须携带 draft 2020-12 元锚（[`DRAFT_2020_12_META`]）；
    /// ② 解析后与全部同伴一起进 registry（离线 `$ref` 可互相引用）；③ 按锁定
    /// 配置编译为 validator。
    pub fn from_texts<I, S1, S2>(texts: I) -> Result<Self, SchemaError>
    where
        I: IntoIterator<Item = (S1, S2)>,
        S1: Into<String>,
        S2: AsRef<str>,
    {
        let mut values = BTreeMap::new();
        for (uri, text) in texts {
            let uri = uri.into();
            let value: serde_json::Value =
                serde_json::from_str(text.as_ref()).map_err(|e| SchemaError::SchemaCompile {
                    uri: uri.clone(),
                    message: e.to_string(),
                })?;
            values.insert(uri, value);
        }
        Self::from_values(values)
    }

    /// 以 `(schema id → schema JSON 值)` 构造（同 [`ValidatorSet::from_texts`]）。
    pub fn from_values<I, S1>(values: I) -> Result<Self, SchemaError>
    where
        I: IntoIterator<Item = (S1, serde_json::Value)>,
        S1: Into<String>,
    {
        // Parity 守卫一：draft 元锚必须为 2020-12（不用自动探测）。
        let mut checked: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        for (uri, value) in values {
            let uri = uri.into();
            let anchor = value
                .get("$schema")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if anchor != DRAFT_2020_12_META {
                return Err(SchemaError::UnsupportedDraft { uri });
            }
            checked.insert(uri, value);
        }
        // Parity 守卫二：全部资源进 registry，未注册的外部 `$ref` 构造期失败
        //（离线化——永不发起网络 / 文件取用）。
        let mut builder = jsonschema::Registry::new();
        for (uri, value) in &checked {
            let resource = jsonschema::Resource::from_contents(value.clone());
            builder = builder
                .add(uri.as_str(), resource)
                .map_err(|e| SchemaError::Registry(format!("{uri}: {e}")))?;
        }
        let registry = builder
            .prepare()
            .map_err(|e| SchemaError::Registry(e.to_string()))?;
        let mut validators = BTreeMap::new();
        for (uri, schema) in &checked {
            // Parity 守卫三/四：draft 锁定 2020-12、format 断言显式关闭、
            // 正则用线性时间引擎（回溯型特性构造期即拒绝）。
            let validator = jsonschema::draft202012::options()
                .with_registry(&registry)
                .offline()
                .should_validate_formats(false)
                .with_pattern_options(jsonschema::PatternOptions::regex())
                .build(schema)
                .map_err(|e| SchemaError::SchemaCompile {
                    uri: uri.clone(),
                    message: e.to_string(),
                })?;
            validators.insert(uri.clone(), validator);
        }
        Ok(Self {
            registry,
            validators,
        })
    }

    /// 注册表中是否含该 schema id。
    pub fn contains(&self, schema_id: &str) -> bool {
        self.validators.contains_key(schema_id)
    }

    /// registry 中是否含该 URI 的资源（`$ref` 解析面诊断）。
    pub fn contains_resource(&self, uri: &str) -> bool {
        self.registry.contains_resource(uri)
    }

    /// 校验实例：未知 schema id 显式报错；违规时逐条归一为
    /// [`ValidationIssue`]（`file` 由调用方提供——校验器只认值不认文件）。
    pub fn issues(
        &self,
        schema_id: &str,
        file: &str,
        instance: &serde_json::Value,
    ) -> Result<Vec<ValidationIssue>, SchemaError> {
        let Some(validator) = self.validators.get(schema_id) else {
            return Err(SchemaError::UnknownSchema(schema_id.to_owned()));
        };
        let issues: Vec<ValidationIssue> = validator
            .iter_errors(instance)
            .map(|err| normalize_issue(schema_id, file, &err))
            .collect();
        Ok(issues)
    }

    /// 校验入口（FR6 / FR12）：合规 `Ok(())`；违规 [`SchemaError::InvalidResource`]
    /// 携带全部信封条目。
    pub fn validate_checked(
        &self,
        schema_id: &str,
        file: &str,
        instance: &serde_json::Value,
    ) -> Result<(), SchemaError> {
        let issues = self.issues(schema_id, file, instance)?;
        if issues.is_empty() {
            return Ok(());
        }
        let count = issues.len();
        let first = issues
            .first()
            .map(std::string::ToString::to_string)
            .unwrap_or_else(|| "unknown".to_owned());
        Err(SchemaError::InvalidResource {
            file: file.to_owned(),
            count,
            first,
            issues,
        })
    }
}

/// 单条校验器错误 → FR-12 信封（从 instance_path / absolute_keyword_location /
/// kind 三面归一）。
fn normalize_issue(
    schema_id: &str,
    file: &str,
    err: &jsonschema::ValidationError<'_>,
) -> ValidationIssue {
    // 稳定判别码：错误类别 Debug 形态的首段（如 `Required { property: "…" }` →
    // `Required`）——零维护的稳定映射面，操作数原文整体进 params。
    let debug = format!("{:?}", err.kind());
    let code = debug
        .split([' ', '{', '('])
        .next()
        .unwrap_or("Unknown")
        .to_owned();
    ValidationIssue {
        file: file.to_owned(),
        pointer: err.instance_path().to_string(),
        // schemaLocation：优先绝对关键字位置；校验器未给出时回落到契约根
        //（schema id）——FR-12 要求作者可回查生效规则，不留空指针。
        schema_location: err
            .absolute_keyword_location()
            .map(|uri| uri.as_str().to_owned())
            .unwrap_or_else(|| schema_id.to_owned()),
        params: serde_json::json!({ "kind": debug }),
        code: code.clone(),
        message: err.to_string(),
        hint: hint_for(&code).to_owned(),
    }
}

/// 修复指引（按判别码映射；未覆盖的判别码回落到通用指引）。
fn hint_for(code: &str) -> &'static str {
    match code {
        "Required" => "缺少必需字段：对照 schema 的 required 列表补齐（注意字段名大小写）",
        "Type" => "值类型不符：该位置 schema 限定了 type（字符串 / 数字 / 布尔等）",
        "Pattern" => "字符串不满足 pattern 约束：按 schema 文档的格式约定书写",
        "Enum" => "取值不在 enum 列表内：使用 schema 允许的枚举值",
        "Const" => "取值被 const 固定：使用 schema 指定的常量值",
        "Minimum" | "Maximum" => "数值超出 schema 限定的范围",
        "MinLength" | "MaxLength" => "字符串长度超出 schema 限定范围",
        "AdditionalProperties" => "出现 schema 未定义的字段：删除或改名（契约字段白名单制）",
        _ => "对照 schema 定义修正该位置（pointer 指向出错值，schemaLocation 指向生效规则）",
    }
}

/// sha256 小写十六进制摘要（manifest 条目生成侧与校验侧共用同一编码口径）。
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// 逐字节内容哈希校验（D-A2 内容寻址锚）：不符即带期望/实际摘要的错误。
pub fn verify_sha256(bytes: &[u8], expected: &str) -> Result<(), SchemaError> {
    let actual = sha256_hex(bytes);
    if actual != expected {
        return Err(SchemaError::ContentMismatch {
            file: String::new(),
            expected: expected.to_owned(),
            actual,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SAMPLE_SCHEMA_ID: &str = "https://yggdrasill.dev/schemas/v1/sample.json";
    const SAMPLE_SCHEMA: &str = r#"{
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://yggdrasill.dev/schemas/v1/sample.json",
        "type": "object",
        "additionalProperties": false,
        "required": ["name"],
        "properties": {
            "name": { "type": "string", "pattern": "^[a-z]+$" },
            "level": { "type": "integer", "minimum": 1 }
        }
    }"#;

    fn set() -> ValidatorSet {
        ValidatorSet::from_texts([(SAMPLE_SCHEMA_ID, SAMPLE_SCHEMA)]).unwrap()
    }

    /// 构造失败提取器（ValidatorSet 无 Debug，`unwrap_err` 不可用）。
    fn compile_err(built: Result<ValidatorSet, SchemaError>) -> SchemaError {
        match built {
            Err(e) => e,
            Ok(_) => panic!("expected construction failure"),
        }
    }

    #[test]
    fn valid_instance_passes_and_unknown_schema_is_reported() {
        let set = set();
        let ok = json!({"name": "feni", "level": 3});
        assert!(set
            .validate_checked(SAMPLE_SCHEMA_ID, "sample.json", &ok)
            .is_ok());
        // 公开诊断面自测（contains / contains_resource）：已注册 id 可见。
        assert!(set.contains(SAMPLE_SCHEMA_ID));
        let err = set
            .validate_checked(
                "https://yggdrasill.dev/schemas/v1/missing.json",
                "x.json",
                &ok,
            )
            .unwrap_err();
        assert!(err.to_string().contains("unknown schema id"), "err: {err}");
    }

    #[test]
    fn envelope_carries_all_seven_fields_with_locations() {
        let set = set();
        let bad = json!({"name": "Feni", "extra": true});
        let err = set
            .validate_checked(SAMPLE_SCHEMA_ID, "sample.json", &bad)
            .unwrap_err();
        assert!(err.to_string().contains("sample.json"));
        let SchemaError::InvalidResource { issues, .. } = err else {
            panic!("expected InvalidResource");
        };
        let codes: Vec<_> = issues.iter().map(|i| i.code.as_str()).collect();
        assert!(codes.contains(&"Pattern"), "codes: {codes:?}");
        assert!(codes.contains(&"AdditionalProperties"), "codes: {codes:?}");
        for issue in &issues {
            assert_eq!(issue.file, "sample.json");
            assert!(issue.schema_location.starts_with("https://"));
            assert!(!issue.params.is_null());
            assert!(!issue.message.is_empty());
            assert!(!issue.hint.is_empty());
        }
        // 根级违规 pointer 为空串（实例根）；属性级违规 pointer 指向属性。
        let pattern_issue = issues.iter().find(|i| i.code == "Pattern").unwrap();
        assert_eq!(pattern_issue.pointer, "/name");
        assert!(pattern_issue.params["kind"]
            .as_str()
            .unwrap()
            .starts_with("Pattern"));
        assert!(pattern_issue.hint.contains("pattern"));
    }

    #[test]
    fn construction_rejects_schemas_without_2020_12_anchor() {
        let old = r#"{"$schema": "http://json-schema.org/draft-07/schema#", "type": "string"}"#;
        let err = compile_err(ValidatorSet::from_texts([(SAMPLE_SCHEMA_ID, old)]));
        assert!(matches!(err, SchemaError::UnsupportedDraft { .. }));
        let no_anchor = r#"{"type": "string"}"#;
        let err = compile_err(ValidatorSet::from_texts([(SAMPLE_SCHEMA_ID, no_anchor)]));
        assert!(matches!(err, SchemaError::UnsupportedDraft { .. }));
    }

    #[test]
    fn offline_rejects_unregistered_external_ref_at_construction() {
        let external = format!(
            r#"{{"$schema": "{DRAFT_2020_12_META}", "$ref": "https://example.com/elsewhere.json"}}"#
        );
        let err = compile_err(ValidatorSet::from_texts([(
            SAMPLE_SCHEMA_ID,
            external.as_str(),
        )]));
        // 构造必须失败（离线化；错误落 Registry 或 SchemaCompile 均可——
        // 不变量 = 未注册引用永不去取用、构造期即拒绝）。
        assert!(
            matches!(
                err,
                SchemaError::Registry(_) | SchemaError::SchemaCompile { .. }
            ),
            "err: {err}"
        );
    }

    #[test]
    fn backtracking_only_regex_constructs_are_rejected_linear_engine() {
        // lookbehind 是回溯型引擎特性、线性引擎不支持——parity 纪律要求
        // 正则限双端交集，此类构造在编译期即拒绝。
        let lookaround = format!(
            r#"{{"$schema": "{DRAFT_2020_12_META}", "type": "string", "pattern": "(?<=a)b"}}"#
        );
        let err = compile_err(ValidatorSet::from_texts([(
            SAMPLE_SCHEMA_ID,
            lookaround.as_str(),
        )]));
        assert!(
            matches!(err, SchemaError::SchemaCompile { .. }),
            "err: {err}"
        );
    }

    #[test]
    fn format_keywords_are_annotations_not_assertions() {
        // format 断言显式关闭（双端 parity）：非法 date-time 不被拒绝。
        let schema = format!(
            r#"{{"$schema": "{DRAFT_2020_12_META}", "type": "string", "format": "date-time"}}"#
        );
        let set = ValidatorSet::from_texts([(SAMPLE_SCHEMA_ID, schema.as_str())]).unwrap();
        assert!(set
            .validate_checked(SAMPLE_SCHEMA_ID, "f.json", &json!("not-a-date"))
            .is_ok());
    }

    #[test]
    fn registry_enables_offline_cross_schema_refs() {
        let base = format!(
            r#"{{"$schema": "{DRAFT_2020_12_META}", "$id": "{SAMPLE_SCHEMA_ID}", "type": "object", "required": ["tag"], "properties": {{"tag": {{"type": "string"}}}}}}"#
        );
        let derived =
            format!(r#"{{"$schema": "{DRAFT_2020_12_META}", "$ref": "{SAMPLE_SCHEMA_ID}"}}"#);
        let derived_id = "https://yggdrasill.dev/schemas/v1/derived.json";
        let set =
            ValidatorSet::from_texts([(SAMPLE_SCHEMA_ID, base), (derived_id, derived)]).unwrap();
        // registry 诊断面：两条 id 均已注册（$ref 离线解析面可见）。
        assert!(set.contains_resource(SAMPLE_SCHEMA_ID));
        assert!(set.contains_resource(derived_id));
        assert!(set
            .validate_checked(derived_id, "d.json", &json!({"tag": "ok"}))
            .is_ok());
        assert!(set
            .validate_checked(derived_id, "d.json", &json!({}))
            .is_err());
    }

    #[test]
    fn sha256_helpers_agree_on_encoding() {
        let bytes = b"ygg content";
        let expected = sha256_hex(bytes);
        assert_eq!(expected.len(), 64);
        assert!(expected
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert!(verify_sha256(bytes, &expected).is_ok());
        let wrong = "0".repeat(64);
        let err = verify_sha256(bytes, &wrong).unwrap_err();
        let SchemaError::ContentMismatch { actual, .. } = err else {
            panic!("expected ContentMismatch");
        };
        assert_eq!(actual, expected);
    }
}
