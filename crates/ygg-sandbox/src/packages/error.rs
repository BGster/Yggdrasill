use thiserror::Error as ThisError;

use super::declare::PACKAGE_ID_RULE;
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
    /// 声明了包但 manifest 没有 `packages` 数组（忘了跑打包工具）。
    #[error(
        "manifest.json has no `packages` array while pack.json declares {packages} package(s): \
         run the pack tool to regenerate the manifest"
    )]
    ManifestMissingPackages { packages: usize },
    /// manifest 登记了但声明里没有。
    #[error(
        "package `{id}` is registered in manifest.json but not declared in pack.json \
         (run the pack tool after editing the declaration)"
    )]
    ManifestUndeclaredPackage { id: String },
    /// 声明了但 manifest 未登记。
    #[error("package `{id}` is declared in pack.json but not registered in manifest.json")]
    ManifestMissingPackage { id: String },
    /// manifest 同一包登记两次。
    #[error("manifest.json registers package `{id}` more than once")]
    ManifestDuplicatePackage { id: String },
    /// manifest 同一文件登记两次。
    #[error("manifest.json registers package file `{path}` more than once")]
    ManifestDuplicatePackageFile { path: String },
    /// 登记了但声明闭包里没有这个文件。
    #[error(
        "package file `{path}` is registered in manifest.json but not part of the declared \
         closure (run the pack tool after editing the package)"
    )]
    ManifestUndeclaredPackageFile { path: String },
    /// 闭包里有但 manifest 未登记。
    #[error(
        "package file `{path}` is part of the declared closure but not registered in manifest.json"
    )]
    ManifestMissingPackageFile { path: String },
    /// 读包文件失败。
    #[error("cannot read package file `{path}`: {source}")]
    PackageRead {
        path: String,
        #[source]
        source: std::io::Error,
    },
    /// 包文件内容寻址失败（byteLength / sha256；文案带路径与期望值）。
    #[error("package file `{path}` content rejected: {details}")]
    PackageContent { path: String, details: String },
    /// 包模块名与 mod 模块名冲突（纵深防御：前缀不同，撞了就是内部 bug）。
    #[error(
        "package module `{name}` collides with a mod module of the same name \
         (this is an internal bug: `pkg.*` and `src.*` cannot collide)"
    )]
    ModuleCollision { name: String },
    /// 并集预算超限（`MAX_MODULES` / 代码总量）。
    #[error(
        "declared code totals {total} modules / {bytes} bytes across the mod and its \
         packages (limits: {max_modules} modules, {max_bytes} bytes)"
    )]
    UnionBudget {
        total: usize,
        bytes: usize,
        max_modules: usize,
        max_bytes: usize,
    },
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
