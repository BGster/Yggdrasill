use std::path::Path;

use serde_json::Value as Json;

use super::error::PackageError;
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
/// `pkg.role-skill` 会被 [`crate::modules::normalize`] 拒掉。反向不成立（`role_skill`
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
    /// `openwiki/contracts/mod/mod-package.md` 的 packages 小节。
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
/// 的 `deps` 决定，打包期与装载期跑同一个 [`crate::packages::resolve`]。
///
/// 段缺失或空数组都返回 `Ok(None)`——两者语义相同（零包），且此时 manifest
/// 逐字节不变（对 `agent-seed` 是可断言的回归）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackagesSpec {
    packages: Vec<PackageRef>,
}

impl PackagesSpec {
    /// 解析 `pack.json` 的 `packages` 段（`None` = 零包）。
    pub fn parse(pack: &Json) -> Result<Option<Self>, PackageError> {
        let Some(raw) = pack.get("packages") else {
            return Ok(None);
        };
        let Some(items) = raw.as_array() else {
            return Err(PackageError::NotArray);
        };
        let refs = parse_refs(items).map_err(|issue| match issue.kind {
            RefIssueKind::NotObject => PackageError::EntryNotObject { index: issue.index },
            RefIssueKind::BadId(id) => PackageError::BadId {
                index: issue.index,
                id,
            },
        })?;
        let mut seen: Vec<&str> = Vec::with_capacity(refs.len());
        for package in &refs {
            if seen.contains(&package.id()) {
                return Err(PackageError::DuplicateId {
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

/// 包自描述 `packages/libs/<id>/pkg.json`（D6）。
///
/// ```json
/// { "schemaVersion": 1, "name": "fixture", "version": "0.1.0", "deps": [],
///   "contract": "openwiki/contracts/packages/README.md" }
/// ```
///
/// - `name` 必须等于目录名（一个包只有一个名字；两份名字就会分裂成两个包）；
/// - `version` 是精确 pin（语义见 [`PackageRef::version`]）；
/// - `deps` 可缺省（= 空数组），是**传递闭包**的唯一来源；
/// - `contract` 是仓库内相对路径，**存在性**由打包期校验（本函数无 IO），
///   **形状**（非绝对路径、无 `.` / `..` 段）在这里先拦下。
///
/// `pkg.json` 自身进 manifest 的 `packages` 段（它决定解析结果，必须内容寻址），
/// 但**不进**模块表（不是 Lua）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageManifest {
    id: String,
    version: String,
    deps: Vec<PackageRef>,
    contract: String,
}

impl PackageManifest {
    /// 解析并校验一个包的自描述（一切失败归为 [`PackageError::Package`]）。
    pub fn parse(id: &str, bytes: &[u8], file: &Path) -> Result<Self, PackageError> {
        let wrap = |reason: String| PackageError::Package {
            id: id.to_owned(),
            reason,
            file: file.display().to_string(),
        };
        let value: Json = serde_json::from_slice(bytes)
            .map_err(|err| wrap(format!("is not valid JSON: {err}")))?;
        if !value.is_object() {
            return Err(wrap("must be a JSON object".to_owned()));
        }
        // schemaVersion：契约演进锚（与 manifest.json 同一个做法）。
        match value.get("schemaVersion").and_then(Json::as_u64) {
            Some(found) if found == PKG_SCHEMA_VERSION => {}
            Some(found) => {
                return Err(wrap(format!(
                    "declares schemaVersion {found} (this build implements {PKG_SCHEMA_VERSION})"
                )))
            }
            None => {
                return Err(wrap(format!(
                    "is missing integer `schemaVersion` (expected {PKG_SCHEMA_VERSION})"
                )))
            }
        }
        // name：与目录名同源。
        let Some(name) = value.get("name").and_then(Json::as_str) else {
            return Err(wrap("is missing string `name`".to_owned()));
        };
        if name != id {
            return Err(wrap(format!(
                "declares name \"{name}\" but lives in directory \"{id}\" \
                 (a package has exactly one name)"
            )));
        }
        // version：精确 pin 的另一半，空字符串不算「没要求版本」。
        let Some(version) = value.get("version").and_then(Json::as_str) else {
            return Err(wrap("is missing string `version`".to_owned()));
        };
        if version.is_empty() {
            return Err(wrap("declares an empty `version`".to_owned()));
        }
        // deps：可缺省；存在即全量校验（与 `packages[]` 同一条规则）。
        let deps = match value.get("deps") {
            None | Some(Json::Null) => Vec::new(),
            Some(raw) => {
                let Some(items) = raw.as_array() else {
                    return Err(wrap(
                        "`deps` must be an array like [{\"id\": \"entity\", \"version\": \"0.1.0\"}]"
                            .to_owned(),
                    ));
                };
                let refs = parse_refs(items).map_err(|issue| {
                    wrap(match issue.kind {
                        RefIssueKind::NotObject => format!(
                            "`deps[{}]` must be an object with string `id` and `version`",
                            issue.index
                        ),
                        RefIssueKind::BadId(bad) => format!(
                            "`deps[{}].id` = \"{bad}\" must match {PACKAGE_ID_RULE}",
                            issue.index
                        ),
                    })
                })?;
                let mut seen: Vec<&str> = Vec::with_capacity(refs.len());
                for dep in &refs {
                    if seen.contains(&dep.id()) {
                        return Err(wrap(format!(
                            "declares `deps` entry \"{}\" twice",
                            dep.id()
                        )));
                    }
                    seen.push(dep.id());
                }
                refs
            }
        };
        // contract：仓库内相对路径（存在性属打包期，本函数无 IO）。
        let Some(contract) = value.get("contract").and_then(Json::as_str) else {
            return Err(wrap(
                "is missing string `contract` (a repo-relative path to the package contract)"
                    .to_owned(),
            ));
        };
        if contract.is_empty() {
            return Err(wrap("declares an empty `contract`".to_owned()));
        }
        if contract.starts_with('/')
            || contract.contains('\\')
            || contract
                .split('/')
                .any(|segment| segment == "." || segment == "..")
        {
            return Err(wrap(format!(
                "`contract` = \"{contract}\" must be a repo-relative path \
                 (no leading `/`, no `.` or `..` segments)"
            )));
        }
        Ok(Self {
            id: id.to_owned(),
            version: version.to_owned(),
            deps,
            contract: contract.to_owned(),
        })
    }

    /// 包 id（= 目录名）。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 包版本（被 `pack.json` / `deps` 精确 pin 的那一侧）。
    pub fn version(&self) -> &str {
        &self.version
    }

    /// 传递依赖清单（缺省为空）。
    pub fn deps(&self) -> &[PackageRef] {
        &self.deps
    }

    /// 契约文档的仓库内相对路径。
    pub fn contract(&self) -> &str {
        &self.contract
    }
}
