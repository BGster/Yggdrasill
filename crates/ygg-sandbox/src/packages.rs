//! packages 装载面：`packages/libs/<id>/` 是**第二个受控模块命名空间**。
//!
//! # 为什么需要这一层
//!
//! r1 的装载器只认一张模块表（`ModuleSet`），r2 的 `pack::assemble` 只把**本
//! Mod 的** `code.modules` 对成那张表。于是「可复用的能力包」无处落地：把包源码
//! 复制进每个 mod 目录会造成双源（改包不重 pack 即静默漂移），而让 `require`
//! 走文件系统会开出一个内容寻址之外的装载通道。本模块把包变成**同一张
//! `ModuleSet` 的第二段**——名字前缀 `pkg.`，`require` 的查表机制一个字不改。
//!
//! # 与「挂入 `package.path`」的关系（与需求字面的偏差，已登记）
//!
//! 沙箱里**没有** `package` 全局，而且被白名单测试钉死（`sandbox.rs` 的
//! `whitelisted_environment_exposes_only_the_allow_list`：`package == nil`）。
//! 所以「挂入 `package.path`」在本仓的两个载体上是两件不同的实事：
//!
//! | 载体 | 等价机制 | 为什么不能字面实现 |
//! | --- | --- | --- |
//! | 内核沙箱 | 模块表注入（[`crate::modules`] 的受控 `require`） | 造一个 `package` 表会把「解析顺序」交给 mod 可变状态 ⇒ 同一 seed 两次回放可能装到不同模块，破确定性判据 |
//! | 普通 Lua（`examples/selftest`、`ygg-lint` 静态面） | `package.searchers` 追加项 | `package.path` 只认文件路径，而包索引是 `(name → 包内相对路径)`，需要一次查表 |
//!
//! 两个载体读**同一份**包索引（[`PackageIndex`] 的 `name → relpath` 表），所以
//! 两侧不可能漂移。
//!
//! # 三层职责（本模块一条线走完）
//!
//! - **声明面**（[`PackagesSpec`] + [`PackageManifest`]）：`pack.json` 的
//!   `packages[]` 与各包 `pkg.json` 的解析与校验，全部是纯函数（无 IO）。
//! - **闭包与根解析**（[`resolve`]）：把声明展开成「哪些包、哪些文件」，包根
//!   由 [`resolve_root`] 决定（`$YGG_PACKAGES_ROOT` > `<mod>/../../packages/libs`）。
//! - **装配面**（[`index`]）：manifest 内容寻址 ↔ 声明闭包 ↔ 实际字节三方对账，
//!   产出模块表与 chunk 路径映射。
//!
//! # 目录形状（P1 严格面）
//!
//! ```text
//! packages/libs/<id>/          # <id> 必须匹配 ^[a-z][a-z0-9_-]*$
//! ├── pkg.json                # 包自描述（必须存在）
//! ├── init.lua                # 包根模块 → pkg.<id>
//! ├── <sub>.lua               # → pkg.<id>.<sub>
//! └── test/*.lua              # 包自测；**不进模块表、不进 manifest**
//! ```
//!
//! `id` 就是目录名（单源，不另声明）。id → 模块前缀时 `-` 折叠为 `_`：模块名
//! 段字符集只有 `[A-Za-z0-9_]`（[`crate::modules::normalize`]），`pkg.role-skill`
//! 非法而 `pkg.role_skill` 合法——折叠不是风格选择，是唯一可行映射。
//!
//! 只允许 `pkg.json` / `*.lua` / `test/**`；以 `.` 开头的条目静默跳过（`.DS_Store`
//! 之类，与 git 惯例一致）。其它任何文件 = 打包期可读拒绝——「无未声明文件」纪律。
//!
//! # 失败面
//!
//! [`PackageError`] 一律在**进入内核之前**失败（零副作用），文案风格对齐
//! [`crate::pack::PackError`]：英文小写、点名 JSON 位置、直接说该写什么。

use serde_json::Value as Json;
use thiserror::Error as ThisError;

use crate::modules;

/// 包 id 的字符集规则（`packages/libs/<id>/` 的目录名，也是 `pkg.<id>.*` 的 id）。
///
/// 点号是结构分隔符（`pkg` / `<id>` / 子模块段），所以 id 里不许有点；连字符
/// 合法但会折叠成下划线（见 [`module_prefix`]）。
pub const PACKAGE_ID_RULE: &str = "^[a-z][a-z0-9_-]*$";

/// 包自描述文件名（每个包的目录下必须存在）。
pub const PKG_FILE: &str = "pkg.json";

/// 包内自测目录名（`test/*.lua`；不进模块表、不进 manifest）。
pub const PKG_TEST_DIR: &str = "test";

/// 包模块的模块名前缀根（`pkg.<id>.*`）。
pub const PACKAGE_PREFIX: &str = "pkg";

/// `pkg.json` 的 `schemaVersion` 现值（契约演进锚，与本构建实现的形状一一对应）。
pub const PKG_SCHEMA_VERSION: u64 = 1;

/// 包 id 是否合法（[`PACKAGE_ID_RULE`] 的手写判定）。
///
/// 手写而不引入 `regex`：本仓零新增依赖是常态（`examples/pack.rs` 记着同一条
/// 取舍），而这条规则只有「首字符小写字母 + 其余小写字母/数字/下划线/连字符」。
pub fn is_valid_package_id(id: &str) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        Some(first) if first.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-')
}

/// 包 id → 模块名前缀（`role-skill` → `pkg.role_skill`；`fixture` → `pkg.fixture`）。
///
/// 连字符折叠为下划线是**唯一可行映射**：模块名段字符集只有 `[A-Za-z0-9_]`，
/// `pkg.role-skill` 会被 [`modules::normalize`] 拒掉。反向不成立（`role_skill`
/// 与 `role-skill` 同前缀），所以 id 仍是单源——前缀只用于装载，不用于回查 id。
pub fn module_prefix(id: &str) -> String {
    format!(
        "{PACKAGE_PREFIX}.{}",
        id.replace('-', "_") // 折叠是映射，不是清洗：id 已过 is_valid_package_id
    )
}

/// 一条包引用（`{ "id": ..., "version": ... }`）。
///
/// `pack.json` 的 `packages[]` 与各包 `pkg.json` 的 `deps[]` 是**同一形状**，
/// 因此共用这一个类型与一套校验——两条面写两套规则必然漂移。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageRef {
    id: String,
    version: String,
}

impl PackageRef {
    pub fn new(id: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            version: version.into(),
        }
    }

    /// 包 id。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 要求的版本（P1 语义 = **精确 pin**，字符串相等）。
    ///
    /// 不做范围匹配的理由：本仓无 `semver` 依赖且零新增依赖是惯例；「同一包在
    /// 两个 mod 里被不同版本需求消费」的真实场景尚未出现。范围语义等到有第二个
    /// 消费者再立，届时不新开文档，落 `packages/README.md` §5 与
    /// `docs/design/mod/mod-package.md` 的 packages 小节。
    pub fn version(&self) -> &str {
        &self.version
    }
}

/// 一条引用的形状错误（局部面）。
///
/// 由调用方包成它自己文档的错误变体：`pack.json` 的 `packages[]` 与 `pkg.json`
/// 的 `deps[]` 在**结构**上共用规则，在**归因**上分别是两份文档。
struct RefIssue {
    index: usize,
    kind: RefIssueKind,
}

enum RefIssueKind {
    /// 不是对象 / 缺 `id` 或 `version` / 字段非字符串或为空。
    NotObject,
    /// `id` 不匹配 [`PACKAGE_ID_RULE`]。
    BadId(String),
}

/// 解析一组 `{id, version}` 条目（`packages[]` 与 `deps[]` 的同一条规则）。
fn parse_refs(items: &[Json]) -> Result<Vec<PackageRef>, RefIssue> {
    let mut refs: Vec<PackageRef> = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let (Some(id), Some(version)) = (
            item.get("id").and_then(Json::as_str),
            item.get("version").and_then(Json::as_str),
        ) else {
            return Err(RefIssue {
                index,
                kind: RefIssueKind::NotObject,
            });
        };
        if id.is_empty() || version.is_empty() {
            return Err(RefIssue {
                index,
                kind: RefIssueKind::NotObject,
            });
        }
        if !is_valid_package_id(id) {
            return Err(RefIssue {
                index,
                kind: RefIssueKind::BadId(id.to_owned()),
            });
        }
        refs.push(PackageRef::new(id, version));
    }
    Ok(refs)
}

/// `pack.json` 的可选 `packages` 段：本 Mod 声明的**直接依赖**清单。
///
/// 段存在即**全量校验**（对齐 [`crate::pack::CodeSpec::parse`] 的半声明拒绝
/// 纪律）：不允许「写了个 packages 但字段缺一半」。传递闭包由各包 `pkg.json`
/// 的 `deps` 决定，打包期与装载期跑同一个 [`resolve`]。
///
/// 段缺失或空数组都返回 `Ok(None)`——两者语义相同（零包），且此时 manifest
/// 逐字节不变（对 `agent-seed` 是可断言的回归）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackagesSpec {
    packages: Vec<PackageRef>,
}

/// `pack.json` 的 `packages` 段的拒绝面（文案点名 JSON 位置）。
///
/// 独立成模块级枚举而不是嵌在 `impl` 里：Rust 的 `impl` 块只容纳关联项，
/// 而这是两个面（声明面）共用的错误类型。
#[derive(Debug, ThisError)]
pub enum PackagesSpecError {
    /// 段不是数组。
    #[error(
        "pack.json `packages` must be an array like \
         [{{\"id\": \"entity\", \"version\": \"0.1.0\"}}]"
    )]
    NotArray,
    /// 项不是对象 / 缺字段 / 字段非字符串或为空。
    #[error("pack.json `packages[{index}]` must be an object with string `id` and `version`")]
    EntryNotObject { index: usize },
    /// `id` 不匹配 [`PACKAGE_ID_RULE`]。
    #[error(
        "pack.json `packages[{index}].id` = \"{id}\" must match {rule} \
         (dots are structural, hyphens fold to `_` in module names)",
        rule = PACKAGE_ID_RULE
    )]
    BadId { index: usize, id: String },
    /// 同一 id 声明两次。
    #[error("pack.json `packages` declares \"{id}\" twice")]
    DuplicateId { id: String },
}

impl PackagesSpec {
    /// 解析 `pack.json` 的 `packages` 段（`None` = 零包）。
    pub fn parse(pack: &Json) -> Result<Option<Self>, PackagesSpecError> {
        let Some(raw) = pack.get("packages") else {
            return Ok(None);
        };
        let Some(items) = raw.as_array() else {
            return Err(PackagesSpecError::NotArray);
        };
        let refs = parse_refs(items).map_err(|issue| match issue.kind {
            RefIssueKind::NotObject => PackagesSpecError::EntryNotObject { index: issue.index },
            RefIssueKind::BadId(id) => PackagesSpecError::BadId {
                index: issue.index,
                id,
            },
        })?;
        let mut seen: Vec<&str> = Vec::with_capacity(refs.len());
        for package in &refs {
            if seen.contains(&package.id()) {
                return Err(PackagesSpecError::DuplicateId {
                    id: package.id().to_owned(),
                });
            }
            seen.push(package.id());
        }
        if refs.is_empty() {
            return Ok(None);
        }
        Ok(Some(Self { packages: refs }))
    }

    /// 直接依赖清单（声明序）。
    pub fn packages(&self) -> &[PackageRef] {
        &self.packages
    }

    /// 直接依赖数量。
    pub fn len(&self) -> usize {
        self.packages.len()
    }

    /// 是否零依赖（构造面不会给出空表，保留以便调用方免去 `len() == 0`）。
    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// 声明面解析的拒绝文案（断言可读性：文案必须点名位置与该写什么）。
    fn spec_err(pack: &Json) -> String {
        match PackagesSpec::parse(pack) {
            Ok(_) => panic!("expected the packages section to be rejected"),
            Err(err) => err.to_string(),
        }
    }

    #[test]
    fn packages_section_is_optional_so_mods_without_packages_parse_unchanged() {
        // 段缺失 = 零包（现有 Mod 零迁移；manifest 逐字节不变）。
        assert!(PackagesSpec::parse(&json!({ "schemaVersion": 1 }))
            .expect("parse")
            .is_none());
        // 空数组 = 零包（同一语义，不制造「声明了 0 个」这种第三态）。
        assert!(PackagesSpec::parse(&json!({ "packages": [] }))
            .expect("parse")
            .is_none());
    }

    #[test]
    fn packages_section_parses_ids_and_versions_in_declaration_order() {
        let spec = PackagesSpec::parse(&json!({
            "packages": [
                { "id": "codec", "version": "0.1.0" },
                { "id": "role-skill", "version": "1.2.3" }
            ]
        }))
        .expect("parse")
        .expect("section present");
        assert_eq!(spec.len(), 2);
        let ids: Vec<&str> = spec.packages().iter().map(PackageRef::id).collect();
        assert_eq!(ids, ["codec", "role-skill"]);
        assert_eq!(spec.packages()[1].version(), "1.2.3");
    }

    #[test]
    fn packages_section_rejects_non_array_and_partial_entries() {
        let err = spec_err(&json!({ "packages": "fixture" }));
        assert_eq!(
            err,
            "pack.json `packages` must be an array like [{\"id\": \"entity\", \"version\": \"0.1.0\"}]"
        );
        // 缺 version：半声明一律拒绝（对齐 CodeSpec 的「段存在即全量校验」）。
        let err = spec_err(&json!({ "packages": [{ "id": "fixture" }] }));
        assert_eq!(
            err,
            "pack.json `packages[0]` must be an object with string `id` and `version`"
        );
        // version 非字符串。
        let err = spec_err(&json!({ "packages": [{ "id": "fixture", "version": 1 }] }));
        assert!(err.contains("packages[0]"), "{err}");
        // 空字符串同样算缺失：`""` 不是「没要求版本」。
        let err = spec_err(&json!({
            "packages": [{ "id": "fixture", "version": "0.1.0" }, { "id": "", "version": "1" }]
        }));
        assert_eq!(
            err,
            "pack.json `packages[1]` must be an object with string `id` and `version`"
        );
    }

    #[test]
    fn packages_section_rejects_illegal_ids_and_duplicates() {
        // 点号是结构分隔符：`role.skill` 会把 id 变成两级模块名。
        let err = spec_err(&json!({ "packages": [{ "id": "role.skill", "version": "0.1.0" }] }));
        assert_eq!(
            err,
            "pack.json `packages[0].id` = \"role.skill\" must match ^[a-z][a-z0-9_-]*$ \
             (dots are structural, hyphens fold to `_` in module names)"
        );
        // 大写：id 与目录名同源，大小写敏感的文件系统会分裂成两个包。
        let err = spec_err(&json!({ "packages": [{ "id": "Fixture", "version": "0.1.0" }] }));
        assert!(err.contains("= \"Fixture\" must match"), "{err}");
        // 首字符必须是字母（`1pkg` 会是非法模块名段）。
        let err = spec_err(&json!({ "packages": [{ "id": "1pkg", "version": "0.1.0" }] }));
        assert!(err.contains("= \"1pkg\" must match"), "{err}");
        // 同一 id 两次：一条声明说了两件事，取哪个都是猜。
        let err = spec_err(&json!({
            "packages": [
                { "id": "fixture", "version": "0.1.0" },
                { "id": "fixture", "version": "0.2.0" }
            ]
        }));
        assert_eq!(err, "pack.json `packages` declares \"fixture\" twice");
    }

    #[test]
    fn hyphenated_id_folds_to_underscores_so_the_module_prefix_normalizes() {
        assert!(is_valid_package_id("role-skill"));
        let prefix = module_prefix("role-skill");
        assert_eq!(prefix, "pkg.role_skill");
        // 折叠前的形态是**非法模块名**——折叠不是风格选择，是唯一可行映射。
        assert!(modules::normalize("pkg.role-skill").is_err());
        assert_eq!(modules::normalize(&prefix).expect("prefix"), prefix);
        // 无反斜杠/空段的 id 同样合法，前缀保形。
        assert_eq!(module_prefix("fixture"), "pkg.fixture");
        // 首字符规则与模块名段同口径。
        assert!(!is_valid_package_id("-lead"));
        assert!(!is_valid_package_id(""));
        assert!(!is_valid_package_id("Fixture"));
        assert!(is_valid_package_id("a"));
        assert!(is_valid_package_id("a_b-c9"));
    }
}
