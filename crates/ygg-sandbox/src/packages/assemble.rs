use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::modules::{self, ModuleSet, ModuleTable};
use ygg_schema::manifest::{verify_package_file, ManifestPackage};

use super::closure::ResolvedPackages;
use super::error::PackageError;
// ───────────────────────── 装配面 ─────────────────────────

/// 装配结果：包模块表 + chunk 名覆盖表。
///
/// 两张表必须同时产出（见 [`ModuleTable`] 的理由）：它们由同一份解析数据
/// 生成，拆开就会留下「能 require 但 traceback 指假路径」的中间态。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackageIndex {
    modules: ModuleSet,
    chunk_paths: BTreeMap<String, String>,
}

impl PackageIndex {
    /// 包模块表（名字前缀 `pkg.`）。
    pub fn modules(&self) -> &ModuleSet {
        &self.modules
    }

    /// chunk 名覆盖表（`模块名 → 相对包根的路径`）。
    pub fn chunk_paths(&self) -> &BTreeMap<String, String> {
        &self.chunk_paths
    }

    /// 包模块总数。
    pub fn len(&self) -> usize {
        self.modules.len()
    }

    /// 是否零包模块。
    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }

    /// 转成装载器吃的 [`ModuleTable`]。
    pub fn into_module_table(self) -> ModuleTable {
        ModuleTable::with_chunk_paths(self.modules, self.chunk_paths)
    }
}

/// 声明闭包 → manifest 登记 → 实际字节 三者对账，产出包模块表。
///
/// 顺序即语义（全部在装载前）：① 包集合双向相等 → ② 文件集合双向相等 →
/// ③ 内容寻址（byteLength → sha256）→ ④ 登记规范名 + chunk 路径。
/// 与 [`crate::pack::assemble`] 同一取舍：部分登记一律拒绝——「半份包表」
/// 只会换来一个看似无关的 `NotFound`。
pub fn index(
    root: &Path,
    resolved: &ResolvedPackages,
    declared: &[ManifestPackage],
) -> Result<PackageIndex, PackageError> {
    // ① 包集合双向相等。
    if declared.is_empty() && !resolved.packages().is_empty() {
        return Err(PackageError::ManifestMissingPackages {
            packages: resolved.len(),
        });
    }
    let mut registry: Vec<&ManifestPackage> = Vec::with_capacity(declared.len());
    for entry in declared {
        if registry.iter().any(|known| known.id() == entry.id()) {
            return Err(PackageError::ManifestDuplicatePackage {
                id: entry.id().to_owned(),
            });
        }
        if !resolved.packages().iter().any(|pkg| pkg.id() == entry.id()) {
            return Err(PackageError::ManifestUndeclaredPackage {
                id: entry.id().to_owned(),
            });
        }
        registry.push(entry);
    }
    for pkg in resolved.packages() {
        if !registry.iter().any(|known| known.id() == pkg.id()) {
            return Err(PackageError::ManifestMissingPackage {
                id: pkg.id().to_owned(),
            });
        }
    }

    // ②③④ 逐包：文件集合双向相等 → 内容寻址 → 登记。
    let mut modules_set = ModuleSet::new();
    let mut chunk_paths: BTreeMap<String, String> = BTreeMap::new();
    for pkg in resolved.packages() {
        let entry = registry
            .iter()
            .find(|known| known.id() == pkg.id())
            .ok_or_else(|| PackageError::ManifestMissingPackage {
                id: pkg.id().to_owned(),
            })?;
        let mut seen: Vec<&str> = Vec::with_capacity(entry.files().len());
        for file in entry.files() {
            if seen.contains(&file.path()) {
                return Err(PackageError::ManifestDuplicatePackageFile {
                    path: file.path().to_owned(),
                });
            }
            if !pkg.files().iter().any(|known| known.rel() == file.path()) {
                return Err(PackageError::ManifestUndeclaredPackageFile {
                    path: file.path().to_owned(),
                });
            }
            seen.push(file.path());
        }
        for file in pkg.files() {
            let Some(manifest_file) = entry
                .files()
                .iter()
                .find(|known| known.path() == file.rel())
            else {
                return Err(PackageError::ManifestMissingPackageFile {
                    path: file.rel().to_owned(),
                });
            };
            let path = root.join(file.rel());
            let bytes = fs::read(&path).map_err(|source| PackageError::PackageRead {
                path: path.display().to_string(),
                source,
            })?;
            verify_package_file(&bytes, manifest_file).map_err(|err| {
                PackageError::PackageContent {
                    path: file.rel().to_owned(),
                    details: err.to_string(),
                }
            })?;
            if let Some(name) = file.module() {
                modules_set
                    .insert(name, bytes)
                    .map_err(|err| PackageError::PackageContent {
                        path: file.rel().to_owned(),
                        details: err.to_string(),
                    })?;
                chunk_paths.insert(name.to_owned(), file.rel().to_owned());
            }
        }
    }
    Ok(PackageIndex {
        modules: modules_set,
        chunk_paths,
    })
}

/// 并集：mod 自有模块表 + 包模块表 → 装载器吃的 [`ModuleTable`]。
///
/// 两件事统一判：① **名字冲突**（纵深防御——`pkg.*` 与 `src.*` 不可能撞，
/// 撞了就是内部 bug，必须报而不是静默覆盖）；② **并集预算**（数量与代码总量
/// 按并集算，不分开算——否则两个各自合法的半边合起来超预算）。
pub fn union(base: &ModuleSet, index: PackageIndex) -> Result<ModuleTable, PackageError> {
    let PackageIndex {
        modules,
        chunk_paths,
    } = index;
    let mut set = base.clone();
    for name in modules.names() {
        let code = modules.get(name).map(<[u8]>::to_vec).unwrap_or_default();
        set.insert(name, code).map_err(|err| match err {
            modules::ModuleError::Duplicate { name } => PackageError::ModuleCollision { name },
            other => PackageError::PackageContent {
                path: name.to_owned(),
                details: other.to_string(),
            },
        })?;
    }
    let bytes: usize = set
        .names()
        .map(|name| set.get(name).map_or(0, <[u8]>::len))
        .sum();
    if bytes > crate::pack::MAX_TOTAL_CODE_BYTES {
        return Err(PackageError::UnionBudget {
            total: set.len(),
            bytes,
            max_modules: modules::MAX_MODULES,
            max_bytes: crate::pack::MAX_TOTAL_CODE_BYTES,
        });
    }
    Ok(ModuleTable::with_chunk_paths(set, chunk_paths))
}
