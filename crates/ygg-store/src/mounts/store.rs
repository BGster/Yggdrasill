use std::collections::BTreeMap;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{json, Value as Json};

use super::{
    parse_mounts, MountEntry, MountError, MountKind, MountMode, MountSpec, MountStat, MountWritten,
    FS_AUDIT_FILE, MOUNTS_DIR, MOUNT_FILE_MAX_BYTES, MOUNT_LIST_MAX_ENTRIES, MOUNT_PATH_MAX_BYTES,
    MOUNT_PATH_MAX_SEGMENTS, MOUNT_PATH_PREFIX, MOUNT_RUN_WRITE_BUDGET,
};

/// 一个已冻结的挂载根（attach 时 `canonicalize` 一次，之后不再解析）。
#[derive(Clone, Debug, PartialEq, Eq)]
struct MountRoot {
    root: PathBuf,
    mode: MountMode,
}

/// 落地的待执行动作（先算全部现状再写，保证「不一致 → 零写入」/ 两条声明
/// 里后一条冲突时前一条也不会被顺手建出来）。
enum Landing<'a> {
    /// 无需动盘（已存在且与声明一致）。
    Keep,
    /// 建真目录（私有挂载）。
    MakeDir,
    /// 建指向声明目录的符号链接（bind 语义，不拷贝内容）。
    MakeLink(&'a Path),
}

/// 狱的「要什么」：文件面要求 `<rel>` 非空；目录面接受挂载根本身。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Need {
    File,
    Dir,
}

/// 一次成功的狱裁决。
struct Located {
    mount: String,
    target: PathBuf,
}

/// 文件挂载桥的宿主侧句柄（`pack.json` `mounts` 段的运行时形）。
pub struct MountStore {
    run_dir: PathBuf,
    mounts_root: PathBuf,
    audit: PathBuf,
    roots: BTreeMap<String, MountRoot>,
    /// 剩余写预算（字节；**进程级、不落盘**）。
    budget_left: u64,
    /// 审计行的下一个 `seq`（懒加载：首次 append 时数非空行）。
    next_audit_seq: Option<u64>,
}

impl MountStore {
    /// 把 `pack.json` 的 `mounts` 段落到 `<run-dir>/mounts/`，并把每个挂载根
    /// canonicalize 一次后冻结。
    ///
    /// 三态幂等（重载 / 续局走同一个入口）：
    /// - 不存在 → 创建（私有 = 真目录；`{name,dir}` = 指向 `canonicalize(dir)` 的软链）；
    /// - 已存在且与声明一致（私有 = 真目录 / 目录挂载 = 软链指向同一 canonical 目标）
    ///   → 静默通过，**零写入**；
    /// - 已存在但不一致 → `YGG-FS-MOUNT` 硬拒且**零写入**（先全部算现状、再落地）。
    ///
    /// 挂载点内的条目仍是懒创建（父目录不存在即拒，见 §狱的规则 7）。
    pub fn attach(run_dir: &Path, pack: &Json, mod_dir: &Path) -> Result<Self, MountError> {
        let specs = parse_mounts(pack, mod_dir)?; // 声明坏了：零写入
        let mounts_root = run_dir.join(MOUNTS_DIR);

        // ① 全量算现状（这一阶段不碰盘任何写）。
        let mut plan: Vec<(&MountSpec, Landing<'_>)> = Vec::with_capacity(specs.len());
        for spec in &specs {
            let slot = mounts_root.join(&spec.name);
            let landing = match fs::symlink_metadata(&slot) {
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => match &spec.dir {
                    None => Landing::MakeDir,
                    Some(dir) => Landing::MakeLink(dir),
                },
                Err(err) => {
                    return Err(MountError::mount(format!(
                        "挂载点 `{}` 不可探测（{err}）",
                        slot.display()
                    )))
                }
                Ok(meta) => {
                    if meta.file_type().is_symlink() {
                        match &spec.dir {
                            None => {
                                return Err(MountError::mount(format!(
                                    "挂载点 `{}` 已存在为软链，但声明是私有挂载（不一致）",
                                    slot.display()
                                )))
                            }
                            Some(dir) => {
                                let actual = fs::read_link(&slot)
                                    .map_err(|err| {
                                        MountError::mount(format!(
                                            "挂载点 `{}` 的链接目标不可读（{err}）",
                                            slot.display()
                                        ))
                                    })?
                                    .canonicalize()
                                    .map_err(|err| {
                                        MountError::mount(format!(
                                            "挂载点 `{}` 的链接目标不可 canonicalize（{err}）",
                                            slot.display()
                                        ))
                                    })?;
                                if actual != *dir {
                                    return Err(MountError::mount(format!(
                                        "挂载点 `{}` 指向 `{}`，与声明的 `{}` 不一致",
                                        slot.display(),
                                        actual.display(),
                                        dir.display()
                                    )));
                                }
                                Landing::Keep
                            }
                        }
                    } else if meta.is_dir() && spec.dir.is_none() {
                        Landing::Keep
                    } else {
                        return Err(MountError::mount(format!(
                            "挂载点 `{}` 已存在，但与声明不一致（{}）",
                            slot.display(),
                            if meta.is_dir() {
                                "声明了 dir」却已存在为真目录"
                            } else {
                                "已存在且不是目录"
                            }
                        )));
                    }
                }
            };
            plan.push((spec, landing));
        }

        // ② 落地（到这里所有声明都已验证过，不会半途才发现冲突）。
        fs::create_dir_all(&mounts_root).map_err(|err| {
            MountError::mount(format!("创建 {} 失败（{err}）", mounts_root.display()))
        })?;
        for (spec, landing) in &plan {
            let slot = mounts_root.join(&spec.name);
            match landing {
                Landing::Keep => {}
                Landing::MakeDir => fs::create_dir_all(&slot).map_err(|err| {
                    MountError::mount(format!("创建 {} 失败（{err}）", slot.display()))
                })?,
                Landing::MakeLink(dir) => link_dir(dir, &slot)?,
            }
        }

        // ③ 冻结每个挂载根（一次 canonicalize；之后的路径裁决只用这个值）。
        let mut roots = BTreeMap::new();
        for spec in &specs {
            let slot = mounts_root.join(&spec.name);
            let root = slot.canonicalize().map_err(|err| {
                MountError::mount(format!(
                    "挂载点 {} 不可 canonicalize（{err}）",
                    slot.display()
                ))
            })?;
            roots.insert(
                spec.name.clone(),
                MountRoot {
                    root,
                    mode: spec.mode,
                },
            );
        }

        Ok(Self {
            run_dir: run_dir.to_path_buf(),
            mounts_root,
            audit: run_dir.join(FS_AUDIT_FILE),
            roots,
            budget_left: MOUNT_RUN_WRITE_BUDGET,
            next_audit_seq: None,
        })
    }

    /// run 目录（调用方拼产物路径用）。
    pub fn run_dir(&self) -> &Path {
        &self.run_dir
    }

    /// 挂载点根（`<run-dir>/mounts`；运维观察面）。
    pub fn mounts_root(&self) -> &Path {
        &self.mounts_root
    }

    /// 审计行文件（`<run-dir>/fs.jsonl`）。
    pub fn audit_path(&self) -> &Path {
        &self.audit
    }

    /// 声明的挂载点（名字 → （冻结根，模式）；声明序无关，`BTreeMap` 序即名字序）。
    pub fn declared(&self) -> impl Iterator<Item = (&str, &Path, MountMode)> {
        self.roots
            .iter()
            .map(|(name, entry)| (name.as_str(), entry.root.as_path(), entry.mode))
    }

    /// 剩余写预算（字节）。
    pub fn budget_left(&self) -> u64 {
        self.budget_left
    }

    /// 路径狱（**唯一实现**）：`mounts/<name>` 与 `mounts/<name>/<rel>` 之外一律拒。
    ///
    /// 拒绝条件（顺序即报错顺序，逐条对应 `docs/design/kernel/lua-api.md` §3.1.2）：
    /// ① 不以 `mounts/` 起头；② `<name>` 未声明；③ 任一段为空 / `.` / `..` / 含
    /// NUL 或 ASCII 控制字符；④ 路径 > 512 字节或 `<rel>` 段数 > 24；⑤ canonicalize
    /// 后必须按**路径分量**落在冻结根内；⑥ 末段不得是软链；⑦ 父目录必须存在。
    /// `<rel>` 为空（`mounts/<name>`）只被 `Need::Dir` 接受。
    fn locate(&self, path: &str, need: Need) -> Result<Located, MountError> {
        if path.is_empty() {
            return Err(MountError::jail("路径不得为空"));
        }
        if path.len() > MOUNT_PATH_MAX_BYTES {
            return Err(MountError::jail(format!(
                "路径超过 {MOUNT_PATH_MAX_BYTES} 字节（当前 {} 字节）",
                path.len()
            )));
        }
        let Some(rest) = path.strip_prefix(MOUNT_PATH_PREFIX) else {
            return Err(MountError::jail(format!(
                "挂载路径必须以 `{MOUNT_PATH_PREFIX}` 起头（当前是 `{path}`）"
            )));
        };
        let mut segments = rest.split('/');
        let name = segments.next().unwrap_or_default();
        let Some(entry) = self.roots.get(name) else {
            return Err(MountError::jail(format!(
                "挂载点 `{name}` 未在 pack.json 的 mounts 段声明"
            )));
        };
        let rel: Vec<&str> = segments.collect();
        for segment in &rel {
            if segment.is_empty() {
                return Err(MountError::jail(format!(
                    "路径不得含空段（`//` 或结尾 `/`；当前是 `{path}`）"
                )));
            }
            if *segment == "." || *segment == ".." {
                return Err(MountError::jail(format!(
                    "路径段不得是 `.` 或 `..`（当前段 `{segment}`）"
                )));
            }
            if let Some(ch) = segment
                .chars()
                .find(|ch| *ch == '\u{0}' || (*ch as u32) < 0x20 || *ch == '\u{7f}')
            {
                return Err(MountError::jail(format!(
                    "路径段含 NUL 或 ASCII 控制字符（U+{:04X}）",
                    ch as u32
                )));
            }
        }
        if rel.len() > MOUNT_PATH_MAX_SEGMENTS {
            return Err(MountError::jail(format!(
                "路径段数超过 {MOUNT_PATH_MAX_SEGMENTS}（当前 {} 段）",
                rel.len()
            )));
        }
        if rel.is_empty() {
            if need == Need::File {
                return Err(MountError::jail(format!(
                    "`{path}` 是挂载根本身（目录），不能当文件路径用"
                )));
            }
            return Ok(Located {
                mount: name.to_owned(),
                target: entry.root.clone(),
            });
        }

        // 父目录：canonicalize 一次（同时杀掉 `..` 与中间段软链的两条歧义），
        // 再按路径分量确认它仍在冻结根内。
        let (last, parents) = rel.split_last().expect("rel 非空");
        let mut parent = entry.root.clone();
        for segment in parents {
            parent.push(segment);
        }
        let canonical_parent = parent.canonicalize().map_err(|err| {
            MountError::jail(format!(
                "父目录 {} 不存在或不可达（{err}）",
                parent.display()
            ))
        })?;
        let meta = fs::metadata(&canonical_parent).map_err(|err| {
            MountError::jail(format!(
                "父路径 {} 不可读（{err}）",
                canonical_parent.display()
            ))
        })?;
        if !meta.is_dir() {
            return Err(MountError::jail(format!(
                "父路径 {} 不是目录",
                canonical_parent.display()
            )));
        }
        if !canonical_parent.starts_with(&entry.root) {
            return Err(MountError::jail(format!(
                "路径越界：`{}` 不在挂载根 `{}` 内",
                canonical_parent.display(),
                entry.root.display()
            )));
        }
        let target = canonical_parent.join(last);
        // 末段永不跟随：已存在的软链一律拒（读穿与写穿两条逃逸同时关闭）。
        if let Ok(existing) = fs::symlink_metadata(&target) {
            if existing.file_type().is_symlink() {
                return Err(MountError::jail(format!(
                    "路径末段不得是符号链接（`{}`）",
                    target.display()
                )));
            }
        }
        Ok(Located {
            mount: name.to_owned(),
            target,
        })
    }
}

impl MountStore {
    /// 读一个挂载文件（`miss → None`，与 `artifacts/` 面同口径：miss 不报错）。
    pub fn read(&self, path: &str) -> Result<Option<Vec<u8>>, MountError> {
        let located = self.locate(path, Need::File)?;
        let meta = match fs::symlink_metadata(&located.target) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(MountError::mount(format!(
                    "读取 {} 失败（{err}）",
                    located.target.display()
                )))
            }
            Ok(meta) => meta,
        };
        if meta.file_type().is_symlink() {
            return Err(MountError::jail(format!(
                "路径末段不得是符号链接（`{}`）",
                located.target.display()
            )));
        }
        if meta.is_dir() {
            return Err(MountError::jail(format!("`{path}` 是目录，不能当文件读")));
        }
        if meta.len() > MOUNT_FILE_MAX_BYTES as u64 {
            return Err(MountError::cap(format!(
                "`{path}` 的 {} 字节超过单件上限 {MOUNT_FILE_MAX_BYTES} 字节",
                meta.len()
            )));
        }
        fs::read(&located.target).map(Some).map_err(|err| {
            MountError::mount(format!("读取 {} 失败（{err}）", located.target.display()))
        })
    }

    /// 探一个挂载路径（目录 `bytes = 0`、`sha256 = ""`；文件算真实摘要）。
    pub fn stat(&self, path: &str) -> Result<MountStat, MountError> {
        let located = self.locate(path, Need::Dir)?;
        let missing = MountStat {
            exists: false,
            bytes: 0,
            sha256: String::new(),
            kind: MountKind::Missing,
        };
        let meta = match fs::symlink_metadata(&located.target) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(missing),
            Err(err) => {
                return Err(MountError::mount(format!(
                    "探测 {} 失败（{err}）",
                    located.target.display()
                )))
            }
            Ok(meta) => meta,
        };
        if meta.file_type().is_symlink() {
            return Err(MountError::jail(format!(
                "路径末段不得是符号链接（`{}`）",
                located.target.display()
            )));
        }
        if meta.is_dir() {
            return Ok(MountStat {
                exists: true,
                bytes: 0,
                sha256: String::new(),
                kind: MountKind::Dir,
            });
        }
        if meta.len() > MOUNT_FILE_MAX_BYTES as u64 {
            return Err(MountError::cap(format!(
                "`{path}` 的 {} 字节超过单件上限 {MOUNT_FILE_MAX_BYTES} 字节",
                meta.len()
            )));
        }
        let bytes = fs::read(&located.target).map_err(|err| {
            MountError::mount(format!("读取 {} 失败（{err}）", located.target.display()))
        })?;
        Ok(MountStat {
            exists: true,
            bytes: bytes.len(),
            sha256: crate::sha256_hex(&bytes),
            kind: MountKind::File,
        })
    }

    /// 列一个挂载目录（不递归、不算摘要、按 `name` 字节序升序）。
    ///
    /// **跳过软链条目**：「末段永不是软链」的同一纪律——列表不会把链接目标
    /// 冒充成 `file` / `dir`（非 UTF-8 名字同理跳过：沙箱路径是 Rust `String`，
    /// 它们本来就不可寻址）。
    pub fn list(&self, path: &str) -> Result<Vec<MountEntry>, MountError> {
        let located = self.locate(path, Need::Dir)?;
        let meta = fs::symlink_metadata(&located.target).map_err(|err| {
            MountError::jail(format!(
                "目录 {} 不存在或不可达（{err}）",
                located.target.display()
            ))
        })?;
        if meta.file_type().is_symlink() {
            return Err(MountError::jail(format!(
                "路径末段不得是符号链接（`{}`）",
                located.target.display()
            )));
        }
        if !meta.is_dir() {
            return Err(MountError::jail(format!("`{path}` 不是目录")));
        }
        let read_dir = fs::read_dir(&located.target).map_err(|err| {
            MountError::mount(format!("目录 {} 不可读（{err}）", located.target.display()))
        })?;
        let mut entries = Vec::new();
        for item in read_dir {
            let item = item.map_err(|err| {
                MountError::mount(format!("目录 {} 不可读（{err}）", located.target.display()))
            })?;
            let Ok(name) = item.file_name().into_string() else {
                continue; // 非 UTF-8 名字：沙箱不可寻址
            };
            let item_meta = match item.metadata() {
                Ok(meta) => meta,
                Err(_) => continue, // 列目录中途消失的条目：不是错误
            };
            if item_meta.file_type().is_symlink() {
                continue; // 末段永不是软链（同一条纪律）
            }
            let kind = if item_meta.is_dir() {
                MountKind::Dir
            } else {
                MountKind::File
            };
            entries.push(MountEntry {
                path: format!("{path}/{name}"),
                name,
                bytes: if kind == MountKind::Dir {
                    0
                } else {
                    item_meta.len() as usize
                },
                kind,
            });
        }
        entries.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
        if entries.len() > MOUNT_LIST_MAX_ENTRIES {
            return Err(MountError::cap(format!(
                "目录 `{path}` 有 {} 条目，超过单次 list 上限 {MOUNT_LIST_MAX_ENTRIES} 条（无分页）",
                entries.len()
            )));
        }
        Ok(entries)
    }
}

impl MountStore {
    /// 写一个挂载文件：**同步写穿磁盘、不回滚**，并追加一行审计。
    ///
    /// 拒绝面（每一条都**先登记审计行**再返回错误）：
    /// - 路径狱 / 目标是目录 → `YGG-FS-JAIL`；
    /// - 只读挂载 → `YGG-FS-MOUNT`；
    /// - 超单件 4 MiB 或超剩余写预算 → `YGG-FS-CAP`。
    ///
    /// 写成功而审计 append 失败 → `YGG-FS-AUDIT`（诚实报「已写未审」）。
    pub fn write(&mut self, path: &str, content: &[u8]) -> Result<MountWritten, MountError> {
        let mount = mount_name_of(path);
        let located = match self.locate(path, Need::File) {
            Ok(located) => located,
            Err(err) => {
                self.log_rejection(&mount, path, err.code());
                return Err(err);
            }
        };
        let mode = self
            .roots
            .get(&located.mount)
            .map(|entry| entry.mode)
            .unwrap_or(MountMode::Ro);
        if mode == MountMode::Ro {
            let err = MountError::mount(format!(
                "挂载点 `{}` 是只读（pack.json 的 mounts[].mode 缺省是 `ro`）",
                located.mount
            ));
            self.log_rejection(&located.mount, path, err.code());
            return Err(err);
        }
        if content.len() > MOUNT_FILE_MAX_BYTES {
            let err = MountError::cap(format!(
                "`{path}` 的 {} 字节超过单件上限 {MOUNT_FILE_MAX_BYTES} 字节",
                content.len()
            ));
            self.log_rejection(&located.mount, path, err.code());
            return Err(err);
        }
        if content.len() as u64 > self.budget_left {
            let err = MountError::cap(format!(
                "写入 {} 字节会超过本轮剩余写预算 {} 字节（每次 attach 重计）",
                content.len(),
                self.budget_left
            ));
            self.log_rejection(&located.mount, path, err.code());
            return Err(err);
        }
        if let Err(err) = fs::write(&located.target, content) {
            let err = MountError::mount(format!("写入 {} 失败（{err}）", located.target.display()));
            self.log_rejection(&located.mount, path, err.code());
            return Err(err);
        }
        self.budget_left -= content.len() as u64;
        let sha256 = crate::sha256_hex(content);
        // 已写未审：审计 append 失败必须诚实报错（`YGG-FS-AUDIT`），不假装成功。
        self.append_audit(json!({
            "event": "fs.write",
            "mount": located.mount,
            "path": path,
            "bytes": content.len(),
            "sha256": sha256,
        }))?;
        Ok(MountWritten {
            path: path.to_owned(),
            bytes: content.len(),
            sha256,
        })
    }

    /// 被拒写也登记（审计的价值一半在「谁试图做了什么」）。
    ///
    /// 审计 append 在此路径上的失败被**吞掉**：主事实是「这次写被拒」，
    /// 用 `YGG-FS-AUDIT` 盖掉它反而丢掉主事实。
    fn log_rejection(&mut self, mount: &str, path: &str, reason: &str) {
        let _ = self.append_audit(json!({
            "event": "fs.write.rejected",
            "mount": mount,
            "path": path,
            "bytes": 0,
            "sha256": "",
            "reason": reason,
        }));
    }

    /// 追加一行审计（`<run-dir>/fs.jsonl`，append-only、懒创建）。
    ///
    /// `seq` = 文件内非空行数 + 1（首次懒加载），**无墙钟**——行序即执行序。
    fn append_audit(&mut self, mut row: Json) -> Result<(), MountError> {
        let seq = self.audit_next_seq()?;
        row["seq"] = json!(seq);
        let mut line = serde_json::to_string(&row)
            .map_err(|err| MountError::audit(format!("审计行序列化失败（{err}）")))?;
        line.push('\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.audit)
            .map_err(|err| {
                MountError::audit(format!("审计文件 {} 不可写（{err}）", self.audit.display()))
            })?;
        file.write_all(line.as_bytes()).map_err(|err| {
            MountError::audit(format!("审计文件 {} 写失败（{err}）", self.audit.display()))
        })
    }

    /// 下一个审计 `seq`（懒加载：首次数非空行——与 `events.jsonl` 的 `next_seq`
    /// 同口径：不可解析的行同样占一个号）。
    fn audit_next_seq(&mut self) -> Result<u64, MountError> {
        if self.next_audit_seq.is_none() {
            let count = match fs::read_to_string(&self.audit) {
                Ok(text) => text.lines().filter(|line| !line.trim().is_empty()).count() as u64,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => 0,
                Err(err) => {
                    return Err(MountError::audit(format!(
                        "审计文件 {} 不可读（{err}）",
                        self.audit.display()
                    )))
                }
            };
            self.next_audit_seq = Some(count + 1);
        }
        let seq = self.next_audit_seq.unwrap_or(1);
        self.next_audit_seq = Some(seq + 1);
        Ok(seq)
    }
}

/// 从沙箱路径里取挂载点名（审计行的 `mount` 字段；解不出就是空串）。
fn mount_name_of(path: &str) -> String {
    path.strip_prefix(MOUNT_PATH_PREFIX)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or_default()
        .to_owned()
}

/// 建「目录挂载」的符号链接（bind 语义；不拷贝内容）。
#[cfg(unix)]
fn link_dir(dir: &Path, slot: &Path) -> Result<(), MountError> {
    std::os::unix::fs::symlink(dir, slot).map_err(|err| {
        MountError::mount(format!(
            "建挂载软链 {} -> {} 失败（{err}）",
            slot.display(),
            dir.display()
        ))
    })
}

/// 本平台没有符号链接 ⇒ 目录挂载不可用（诚实地拒，而不是默默拷贝一份内容）。
#[cfg(not(unix))]
fn link_dir(dir: &Path, slot: &Path) -> Result<(), MountError> {
    let _ = (dir, slot);
    Err(MountError::mount(
        "本平台不支持符号链接，目录挂载（{name, dir}）不可用；请改用私有挂载",
    ))
}

#[cfg(test)]
mod tests;
