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

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

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

/// packages 装载面的拒绝面（文案风格对齐 [`crate::pack::PackError`]：英文小写、
/// 点名 JSON 位置 / 文件、直接说该写什么）。
///
/// 独立成模块级枚举而不是嵌在 `impl` 里：Rust 的 `impl` 块只容纳关联项，
/// 而这是声明 / 闭包 / 装配三个面共用的错误类型。
#[derive(Debug, ThisError)]
pub enum PackageError {
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
    /// 包根不可读（D4：点名推导值与两个来源，不静默降级）。
    #[error(
        "package root `{root}` is not a readable directory \
         (looked for the `--packages` / `$YGG_PACKAGES_ROOT` value and the default \
         `<mod>/../../packages/libs`)"
    )]
    RootUnreadable { root: String },
    /// 声明的包在包根下不存在。
    #[error("package \"{id}\" is not a package under {root} (found: {found})")]
    Unknown {
        id: String,
        root: String,
        found: String,
    },
    /// 版本不符（声明精确 pin，不符是**声明错**而非完整性错，故报双方值）。
    #[error("package \"{id}\" is required at version {want} but {file} declares {got}")]
    VersionMismatch {
        id: String,
        want: String,
        got: String,
        file: String,
    },
    /// 传递依赖未被本 Mod 的 `packages` 段声明（必须声明完整闭包）。
    #[error(
        "package \"{parent}\" depends on \"{id}\" which the mod's `packages` section \
         does not declare (declare the full closure)"
    )]
    MissingTransitive { parent: String, id: String },
    /// 包依赖成环。
    #[error("package dependency cycle: {chain}")]
    Cycle { chain: String },
    /// 包自描述 `pkg.json` 坏了（JSON / 字段 / name ≠ 目录 / 非允许文件）。
    ///
    /// 一个包的所有局部问题共用这一条：读者先要知道**是哪个包**，再谈细节；
    /// 包内字段路径（`deps[0]`）作为 `reason` 的一部分报出。
    #[error("package \"{id}\" {reason} ({file})")]
    Package {
        id: String,
        reason: String,
        file: String,
    },
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
///   "contract": "docs/design/packages/README.md" }
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

// ───────────────────────── 闭包与包根解析 ─────────────────────────

/// 包根环境变量（最高优先级的显式来源；与 `--packages` 参数同义）。
pub const PACKAGES_ROOT_ENV: &str = "YGG_PACKAGES_ROOT";

/// 包根解析：`explicit`（`--packages`）> `$YGG_PACKAGES_ROOT` > `<mod>/../../packages/libs`。
///
/// 包根是 **host 环境态**而非 run 身份态，所以 `pack.json` 的 `packages` 段只写
/// id / version 不写路径（`pack.rs` 的 `Traversal` 规则与可移植性都要求包根不
/// 落在 pack 声明里）。内容由 manifest 的 sha256 钉死：同字节换根行为等价，异字节
/// 在装载期 hash 失配即失败。
///
/// 不可读即失败并点名推导值——**不静默降级为零包**：声明了包而静默不装，症状是
/// 运行期一个看似无关的 `NotFound`。
pub fn resolve_root(mod_dir: &Path, explicit: Option<&Path>) -> Result<PathBuf, PackageError> {
    let candidate = match explicit {
        Some(dir) => dir.to_path_buf(),
        None => match std::env::var_os(PACKAGES_ROOT_ENV) {
            Some(value) => PathBuf::from(value),
            None => mod_dir.join("../../packages/libs"),
        },
    };
    if candidate.is_dir() {
        return Ok(candidate);
    }
    Err(PackageError::RootUnreadable {
        root: candidate.display().to_string(),
    })
}

/// 已解析的包文件（相对**包根**的路径 + 它作为模块的名字）。
///
/// `pkg.json` 也是包文件（它决定解析结果，必须内容寻址），但它不是模块——
/// `module` 为 `None` 就是那条分界，不需要第二个类型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageFile {
    rel: String,
    module: Option<String>,
}

impl PackageFile {
    /// 相对包根的路径（包 id 打头，如 `fixture/init.lua`）。
    pub fn rel(&self) -> &str {
        &self.rel
    }

    /// 作为模块注册的名字（`pkg.fixture` / `pkg.fixture.words`）；`pkg.json` 为 `None`。
    pub fn module(&self) -> Option<&str> {
        self.module.as_deref()
    }
}

/// 一个包的解析结果：自描述 + 纳入闭包的文件（按 `rel` 升序）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPackage {
    manifest: PackageManifest,
    files: Vec<PackageFile>,
}

impl ResolvedPackage {
    pub fn id(&self) -> &str {
        self.manifest.id()
    }

    pub fn version(&self) -> &str {
        self.manifest.version()
    }

    pub fn deps(&self) -> &[PackageRef] {
        self.manifest.deps()
    }

    pub fn contract(&self) -> &str {
        self.manifest.contract()
    }

    /// 纳入闭包的文件（`pkg.json` + 非 `test/**` 的 `*.lua`，按 `rel` 升序）。
    pub fn files(&self) -> &[PackageFile] {
        &self.files
    }
}

/// 整个闭包的解析结果（包按 id 升序：确定性，不依赖目录枚举序）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResolvedPackages {
    packages: Vec<ResolvedPackage>,
}

impl ResolvedPackages {
    /// 声明的包（id 升序）。
    pub fn packages(&self) -> &[ResolvedPackage] {
        &self.packages
    }

    pub fn len(&self) -> usize {
        self.packages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }

    /// 全部模块文件：`(模块名, 相对包根的路径)`（名字序）。
    ///
    /// 这份表就是 D3 的 `name → path` 覆盖表：装载器的 chunk 名从它取（未命中
    /// 则回落 `path_of`，即现有 `src.*` 行为逐字节不变），selftest 的
    /// `package.searchers` 也从它生成——**一份解析数据，两个宿主面**。
    pub fn chunk_paths(&self) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        for package in &self.packages {
            for file in &package.files {
                if let Some(module) = file.module() {
                    map.insert(module.to_owned(), file.rel().to_owned());
                }
            }
        }
        map
    }
}

/// 声明 → 包闭包（目录枚举 + `pkg.json` 解析 + 版本 / 缺失 / 循环校验）。
///
/// # 闭包必须是**完整声明**
///
/// `packages` 段是唯一的装载清单：包的 `deps` 只**声明**依赖关系（用于校验与
/// 循环检测），不把未声明的包"拉进来"。一个 dep 未在 `packages` 段出现即
/// [`PackageError::MissingTransitive`]。这条与本仓既有纪律同形：`code.modules`
/// 也必须枚举每一个可 `require` 的模块，没有任何隐式发现面。
///
/// 全部校验在**进入内核之前**完成（零副作用）。
pub fn resolve(root: &Path, spec: &PackagesSpec) -> Result<ResolvedPackages, PackageError> {
    let found = list_package_ids(root);
    let found_list = if found.is_empty() {
        "-".to_owned()
    } else {
        found.join(", ")
    };

    // ① 逐个声明：目录存在 → pkg.json 可解析 → 版本相符。
    let mut manifests: BTreeMap<String, PackageManifest> = BTreeMap::new();
    for package in spec.packages() {
        let id = package.id();
        let dir = root.join(id);
        if !dir.is_dir() {
            return Err(PackageError::Unknown {
                id: id.to_owned(),
                root: root.display().to_string(),
                found: found_list.clone(),
            });
        }
        let file = dir.join(PKG_FILE);
        let bytes = fs::read(&file).map_err(|err| PackageError::Package {
            id: id.to_owned(),
            reason: format!("cannot be read: {err}"),
            file: file.display().to_string(),
        })?;
        let manifest = PackageManifest::parse(id, &bytes, &file)?;
        if manifest.version() != package.version() {
            return Err(PackageError::VersionMismatch {
                id: id.to_owned(),
                want: package.version().to_owned(),
                got: manifest.version().to_owned(),
                file: file.display().to_string(),
            });
        }
        manifests.insert(id.to_owned(), manifest);
    }

    // ② 每个 dep 必须在声明集内（完整闭包），且版本与它自己的声明一致。
    for manifest in manifests.values() {
        for dep in manifest.deps() {
            let Some(declared) = spec.packages().iter().find(|item| item.id() == dep.id()) else {
                return Err(PackageError::MissingTransitive {
                    parent: manifest.id().to_owned(),
                    id: dep.id().to_owned(),
                });
            };
            let target = &manifests[dep.id()];
            if target.version() != dep.version() || target.version() != declared.version() {
                return Err(PackageError::VersionMismatch {
                    id: dep.id().to_owned(),
                    want: dep.version().to_owned(),
                    got: target.version().to_owned(),
                    file: root.join(dep.id()).join(PKG_FILE).display().to_string(),
                });
            }
        }
    }

    // ③ 循环（声明图；递归深度 = 包数，P1 的十几个包构不成栈风险）。
    let mut marks: BTreeMap<String, u8> = BTreeMap::new();
    let mut path: Vec<String> = Vec::new();
    for id in manifests.keys() {
        visit(id, &manifests, &mut marks, &mut path)?;
    }

    // ④ 文件枚举（按 id 升序：确定性）。
    let mut packages = Vec::with_capacity(manifests.len());
    for (id, manifest) in manifests {
        let dir = root.join(&id);
        let files = enumerate_files(&id, &dir, &dir.join(PKG_FILE))?;
        packages.push(ResolvedPackage { manifest, files });
    }
    Ok(ResolvedPackages { packages })
}

/// 包根下的目录名清单（升序；用于「找不到该包时告诉你有哪些」）。
fn list_package_ids(root: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            is_valid_package_id(&name).then_some(name)
        })
        .collect();
    ids.sort();
    ids
}

/// 声明图的 DFS：`0` 未访问 / `1` 在栈上 / `2` 已完成。
fn visit(
    id: &str,
    manifests: &BTreeMap<String, PackageManifest>,
    marks: &mut BTreeMap<String, u8>,
    path: &mut Vec<String>,
) -> Result<(), PackageError> {
    match marks.get(id).copied().unwrap_or(0) {
        2 => return Ok(()),
        1 => {
            let start = path.iter().position(|item| item == id).unwrap_or(0);
            let mut chain: Vec<String> = path[start..].to_vec();
            chain.push(id.to_owned());
            return Err(PackageError::Cycle {
                chain: chain.join(" -> "),
            });
        }
        _ => {}
    }
    marks.insert(id.to_owned(), 1);
    path.push(id.to_owned());
    if let Some(manifest) = manifests.get(id) {
        for dep in manifest.deps() {
            visit(dep.id(), manifests, marks, path)?;
        }
    }
    path.pop();
    marks.insert(id.to_owned(), 2);
    Ok(())
}

/// 枚举一个包纳入闭包的文件：`pkg.json` + 非 `test/**` 的 `*.lua`。
///
/// 其它任何文件 = 可读拒绝（「无未声明文件」纪律）；以 `.` 开头的条目静默跳过
/// （`.DS_Store` 之类，与 git 惯例一致）。`test/**` 不进模块表也不进 manifest——
/// 包自测是开发面，不是装载面。
fn enumerate_files(
    id: &str,
    dir: &Path,
    pkg_file: &Path,
) -> Result<Vec<PackageFile>, PackageError> {
    let reject = |rel: &str| PackageError::Package {
        id: id.to_owned(),
        reason: format!(
            "contains `{rel}` which is not an allowed file \
             (allowed: `{PKG_FILE}`, `*.lua`, `{PKG_TEST_DIR}/**`)"
        ),
        file: pkg_file.display().to_string(),
    };
    let mut files: Vec<PackageFile> = vec![PackageFile {
        rel: format!("{id}/{PKG_FILE}"),
        module: None,
    }];
    let mut pending: Vec<(PathBuf, String)> = vec![(dir.to_path_buf(), String::new())];
    while let Some((current, prefix)) = pending.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            return Err(reject(&prefix));
        };
        for entry in entries.filter_map(|entry| entry.ok()) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let rel_in_pkg = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            let path = entry.path();
            if path.is_dir() {
                // 只有包根的 `test/` 整棵子树跳过（子目录里的 test 仍是非法面）。
                if prefix.is_empty() && name == PKG_TEST_DIR {
                    continue;
                }
                pending.push((path, rel_in_pkg));
                continue;
            }
            if prefix.is_empty() && name == PKG_FILE {
                continue; // 已作为非模块条目入册
            }
            if !name.ends_with(modules::MODULE_EXTENSION) {
                return Err(reject(&rel_in_pkg));
            }
            let module = module_name(id, &rel_in_pkg).map_err(|cause| PackageError::Package {
                id: id.to_owned(),
                reason: format!(
                    "contains `{rel_in_pkg}` which is not a valid module path: {cause}"
                ),
                file: pkg_file.display().to_string(),
            })?;
            files.push(PackageFile {
                rel: format!("{id}/{rel_in_pkg}"),
                module: Some(module),
            });
        }
    }
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(files)
}

/// 包内相对路径 → 模块名（复用 [`modules::name_of_path`]，不另写一套字符规则）。
///
/// `init.lua` → `pkg.<id>`（包根模块）；`words.lua` → `pkg.<id>.words`；
/// `a/b.lua` → `pkg.<id>.a.b`。
fn module_name(id: &str, rel_in_pkg: &str) -> Result<String, modules::ModuleError> {
    let path_relative = modules::name_of_path(rel_in_pkg)?;
    let prefix = module_prefix(id);
    let name = if path_relative == "init" {
        prefix
    } else {
        format!("{prefix}.{path_relative}")
    };
    modules::normalize(&name)?;
    Ok(name)
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
        assert!(crate::modules::normalize("pkg.role-skill").is_err());
        assert_eq!(crate::modules::normalize(&prefix).expect("prefix"), prefix);
        // 无反斜杠/空段的 id 同样合法，前缀保形。
        assert_eq!(module_prefix("fixture"), "pkg.fixture");
        // 首字符规则与模块名段同口径。
        assert!(!is_valid_package_id("-lead"));
        assert!(!is_valid_package_id(""));
        assert!(!is_valid_package_id("Fixture"));
        assert!(is_valid_package_id("a"));
        assert!(is_valid_package_id("a_b-c9"));
    }

    /// `pkg.json` 拒绝文案（断言「先知道是哪个包，再谈细节」）。
    fn pkg_err(id: &str, text: &str) -> String {
        let file = Path::new("packages/libs").join(id).join(PKG_FILE);
        match PackageManifest::parse(id, text.as_bytes(), &file) {
            Ok(_) => panic!("expected the package manifest to be rejected"),
            Err(err) => err.to_string(),
        }
    }

    /// 合格 `pkg.json` 的便捷构造函数（只写出差异字段）。
    fn pkg_json(extra: &str) -> String {
        format!(
            r#"{{"schemaVersion": 1, "name": "fixture", "version": "0.1.0",
                 "contract": "docs/design/packages/README.md"{extra}}}"#
        )
    }

    #[test]
    fn package_manifest_parses_the_documented_shape_and_defaults_deps_to_empty() {
        let file = Path::new("packages/libs/fixture/pkg.json");
        let bare = PackageManifest::parse("fixture", pkg_json("").as_bytes(), file)
            .expect("minimal pkg.json");
        assert_eq!(bare.id(), "fixture");
        assert_eq!(bare.version(), "0.1.0");
        assert!(bare.deps().is_empty());
        assert_eq!(bare.contract(), "docs/design/packages/README.md");

        // deps 与 packages[] 同形（共用 PackageRef 与 parse_refs）。
        let with_deps = PackageManifest::parse(
            "fixture",
            pkg_json(r#", "deps": [{"id": "role-skill", "version": "1.2.3"}]"#).as_bytes(),
            file,
        )
        .expect("pkg.json with deps");
        assert_eq!(with_deps.deps().len(), 1);
        assert_eq!(with_deps.deps()[0].id(), "role-skill");
        assert_eq!(with_deps.deps()[0].version(), "1.2.3");
    }

    #[test]
    fn package_manifest_rejects_a_name_that_differs_from_its_directory() {
        let text = r#"{"schemaVersion": 1, "name": "other", "version": "0.1.0",
                       "contract": "docs/design/packages/README.md"}"#;
        let err = pkg_err("fixture", text);
        assert_eq!(
            err,
            "package \"fixture\" declares name \"other\" but lives in directory \"fixture\" \
             (a package has exactly one name) (packages/libs/fixture/pkg.json)"
        );
    }

    #[test]
    fn package_manifest_rejects_bad_deps_and_contract_shape() {
        // deps 不是数组。
        let err = pkg_err("fixture", &pkg_json(r#", "deps": "none""#));
        assert!(
            err.starts_with("package \"fixture\" `deps` must be an array"),
            "{err}"
        );
        // deps 项缺 version。
        let err = pkg_err("fixture", &pkg_json(r#", "deps": [{"id": "codec"}]"#));
        assert!(
            err.contains("`deps[0]` must be an object with string `id` and `version`"),
            "{err}"
        );
        // deps id 非法（点号）。
        let err = pkg_err(
            "fixture",
            &pkg_json(r#", "deps": [{"id": "role.skill", "version": "1"}]"#),
        );
        assert!(
            err.contains("`deps[0].id` = \"role.skill\" must match"),
            "{err}"
        );
        // deps 同一 id 两次。
        let err = pkg_err(
            "fixture",
            &pkg_json(
                r#", "deps": [{"id": "codec", "version": "1"}, {"id": "codec", "version": "2"}]"#,
            ),
        );
        assert!(
            err.contains("declares `deps` entry \"codec\" twice"),
            "{err}"
        );
        // contract 上跳。
        let err = pkg_err(
            "fixture",
            r#"{"schemaVersion": 1, "name": "fixture", "version": "0.1.0",
                "contract": "../escape.md"}"#,
        );
        assert!(err.contains("must be a repo-relative path"), "{err}");
        // contract 绝对路径。
        let err = pkg_err(
            "fixture",
            r#"{"schemaVersion": 1, "name": "fixture", "version": "0.1.0",
                "contract": "/abs.md"}"#,
        );
        assert!(err.contains("must be a repo-relative path"), "{err}");
    }

    #[test]
    fn package_manifest_rejects_bad_json_schema_version_and_missing_fields() {
        let err = pkg_err("fixture", "{ not json");
        assert!(err.contains("is not valid JSON"), "{err}");
        // schemaVersion 是本构建实现的锚。
        let err = pkg_err(
            "fixture",
            r#"{"schemaVersion": 2, "name": "fixture", "version": "0.1.0",
                "contract": "docs/design/packages/README.md"}"#,
        );
        assert!(
            err.contains("declares schemaVersion 2 (this build implements 1)"),
            "{err}"
        );
        // 缺 version：半声明一律拒绕（与 packages[] 同一条纪律）。
        let err = pkg_err(
            "fixture",
            r#"{"schemaVersion": 1, "name": "fixture",
                "contract": "docs/design/packages/README.md"}"#,
        );
        assert!(err.contains("is missing string `version`"), "{err}");
        // 缺 contract。
        let err = pkg_err(
            "fixture",
            r#"{"schemaVersion": 1, "name": "fixture", "version": "0.1.0"}"#,
        );
        assert!(err.contains("is missing string `contract`"), "{err}");
    }

    // ───────────────────── 闭包与包根解析（S3） ─────────────────────

    /// 本用例独占的临时包根（先清空，重跑不受上次残留影响）。
    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ygg-r3-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create package root");
        dir
    }

    /// 写一个包：`pkg.json` + 给定文件（`rel_in_pkg` → 源码）。
    fn write_pkg(
        root: &Path,
        id: &str,
        version: &str,
        deps: &[(&str, &str)],
        files: &[(&str, &str)],
    ) {
        let dir = root.join(id);
        fs::create_dir_all(&dir).expect("create package dir");
        let deps_json: Vec<String> = deps
            .iter()
            .map(|(dep_id, dep_version)| {
                format!(r#"{{ "id": "{dep_id}", "version": "{dep_version}" }}"#)
            })
            .collect();
        let pkg = format!(
            r#"{{ "schemaVersion": 1, "name": "{id}", "version": "{version}",
                 "deps": [{}], "contract": "docs/design/packages/README.md" }}"#,
            deps_json.join(", ")
        );
        fs::write(dir.join(PKG_FILE), pkg).expect("write pkg.json");
        for (rel, source) in files {
            let target = dir.join(rel);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).expect("create file dir");
            }
            fs::write(&target, source).unwrap_or_else(|err| panic!("write {rel}: {err}"));
        }
    }

    /// `[{id, version}]` → [`PackagesSpec`]（走真实解析面，不绕开校验）。
    fn spec_of(pairs: &[(&str, &str)]) -> PackagesSpec {
        let declared: Vec<Json> = pairs
            .iter()
            .map(|(id, version)| json!({ "id": id, "version": version }))
            .collect();
        PackagesSpec::parse(&json!({ "packages": declared }))
            .expect("parse")
            .expect("section present")
    }

    #[test]
    fn resolve_maps_package_files_to_module_names_and_skips_tests() {
        let root = temp_root("resolve-ok");
        write_pkg(
            &root,
            "fixture",
            "0.1.0",
            &[],
            &[
                ("init.lua", "return {}"),
                ("words.lua", "return {}"),
                ("nested/deep.lua", "return {}"),
                ("test/pkg-fixture.lua", "-- self test"),
            ],
        );
        let resolved = resolve(&root, &spec_of(&[("fixture", "0.1.0")])).expect("resolve");
        assert_eq!(resolved.len(), 1);
        let package = &resolved.packages()[0];
        assert_eq!(package.id(), "fixture");
        assert_eq!(package.version(), "0.1.0");
        // 非 test/** 的 lua + pkg.json，按 rel 升序；test/** 不在其中。
        let rels: Vec<&str> = package.files().iter().map(PackageFile::rel).collect();
        assert_eq!(
            rels,
            [
                "fixture/init.lua",
                "fixture/nested/deep.lua",
                "fixture/pkg.json",
                "fixture/words.lua",
            ]
        );
        // pkg.json 是包文件但不是模块。
        assert_eq!(package.files()[2].module(), None);
        // init.lua 是包根模块（不带 `.init`）。
        let chunks = resolved.chunk_paths();
        assert_eq!(chunks["pkg.fixture"], "fixture/init.lua");
        assert_eq!(chunks["pkg.fixture.words"], "fixture/words.lua");
        assert_eq!(chunks["pkg.fixture.nested.deep"], "fixture/nested/deep.lua");
    }

    #[test]
    fn resolve_hyphenated_id_becomes_an_underscore_module_prefix() {
        let root = temp_root("resolve-hyphen");
        write_pkg(
            &root,
            "role-skill",
            "0.1.0",
            &[],
            &[("init.lua", "return {}")],
        );
        let resolved = resolve(&root, &spec_of(&[("role-skill", "0.1.0")])).expect("resolve");
        assert_eq!(
            resolved.chunk_paths()["pkg.role_skill"],
            "role-skill/init.lua"
        );
    }

    #[test]
    fn resolve_reports_a_missing_package_with_the_ids_it_found() {
        let root = temp_root("resolve-unknown");
        write_pkg(&root, "fixture", "0.1.0", &[], &[("init.lua", "return {}")]);
        let err = resolve(&root, &spec_of(&[("quest", "0.1.0")])).unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("package \"quest\" is not a package under"),
            "{text}"
        );
        // 「找不到」必须告诉作者有哪些——否则只能猜目录名。
        assert!(text.contains("(found: fixture)"), "{text}");
    }

    #[test]
    fn resolve_rejects_version_drift_and_requires_the_full_closure() {
        let root = temp_root("resolve-version");
        write_pkg(&root, "fixture", "0.2.0", &[], &[("init.lua", "return {}")]);
        // 声明 0.1.0，pkg.json 是 0.2.0：报双方值（声明错，不是完整性错）。
        let err = resolve(&root, &spec_of(&[("fixture", "0.1.0")])).unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("package \"fixture\" is required at version 0.1.0"),
            "{text}"
        );
        assert!(text.contains("declares 0.2.0"), "{text}");

        // 传递依赖未声明：quest 依赖 codec，但 packages 段只列了 quest。
        let root = temp_root("resolve-closure");
        write_pkg(
            &root,
            "quest",
            "0.1.0",
            &[("codec", "0.1.0")],
            &[("init.lua", "return {}")],
        );
        write_pkg(&root, "codec", "0.1.0", &[], &[("init.lua", "return {}")]);
        let err = resolve(&root, &spec_of(&[("quest", "0.1.0")])).unwrap_err();
        assert_eq!(
            err.to_string(),
            "package \"quest\" depends on \"codec\" which the mod's `packages` section \
             does not declare (declare the full closure)"
        );
        // 声明完整闭包即通过。
        let resolved = resolve(&root, &spec_of(&[("quest", "0.1.0"), ("codec", "0.1.0")]))
            .expect("full closure resolves");
        assert_eq!(resolved.len(), 2);
    }

    #[test]
    fn resolve_detects_dependency_cycles() {
        let root = temp_root("resolve-cycle");
        write_pkg(
            &root,
            "a",
            "0.1.0",
            &[("b", "0.1.0")],
            &[("init.lua", "return {}")],
        );
        write_pkg(
            &root,
            "b",
            "0.1.0",
            &[("a", "0.1.0")],
            &[("init.lua", "return {}")],
        );
        let err = resolve(&root, &spec_of(&[("a", "0.1.0"), ("b", "0.1.0")])).unwrap_err();
        assert_eq!(err.to_string(), "package dependency cycle: a -> b -> a");
    }

    #[test]
    fn resolve_rejects_files_that_are_not_allowed() {
        let root = temp_root("resolve-files");
        write_pkg(
            &root,
            "fixture",
            "0.1.0",
            &[],
            &[("init.lua", "return {}"), ("README.md", "# not allowed")],
        );
        let err = resolve(&root, &spec_of(&[("fixture", "0.1.0")])).unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("contains `README.md` which is not an allowed file"),
            "{text}"
        );
        // 点开头的条目静默跳过（.DS_Store 不该拦住整包）。
        let root = temp_root("resolve-dotfile");
        write_pkg(
            &root,
            "fixture",
            "0.1.0",
            &[],
            &[("init.lua", "return {}"), (".DS_Store", "junk")],
        );
        let resolved =
            resolve(&root, &spec_of(&[("fixture", "0.1.0")])).expect("dotfiles are skipped");
        assert_eq!(resolved.packages()[0].files().len(), 2);
    }

    #[test]
    fn resolve_root_fails_loudly_instead_of_degrading_to_zero_packages() {
        let mod_dir = temp_root("resolve-root/mod/a/b");
        let err = resolve_root(&mod_dir, Some(&mod_dir.join("absent"))).unwrap_err();
        assert!(
            err.to_string().contains("is not a readable directory"),
            "{err}"
        );
        // 显式参数优先：存在即采纳。
        let good = temp_root("resolve-root-good");
        assert_eq!(resolve_root(&mod_dir, Some(&good)).unwrap(), good);
    }
}
