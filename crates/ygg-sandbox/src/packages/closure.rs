use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::modules;

use super::declare::{
    is_valid_package_id, module_prefix, PackageManifest, PackageRef, PackagesSpec, PKG_FILE,
    PKG_TEST_DIR,
};
use super::error::PackageError;
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
