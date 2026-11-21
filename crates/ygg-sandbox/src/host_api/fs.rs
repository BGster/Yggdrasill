use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use mlua::{Error, Lua, Table, Value as LuaValue};

use super::HostState;
use ygg_schema::validate::sha256_hex;

/// 挂载点里一项的类型（= `ygg.fs.stat` / `ygg.fs.list` 回执里的 `kind` 取值源）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountKind {
    File,
    Dir,
    /// 不存在（只出现在 `stat` 的回执里，`as_str()` 是空串）。
    Missing,
}

impl MountKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MountKind::File => "file",
            MountKind::Dir => "dir",
            MountKind::Missing => "",
        }
    }
}

/// 一次挂载写的回执（`ygg.fs.write` 的 `{ path, bytes, sha256 }`）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountWritten {
    pub path: String,
    pub bytes: usize,
    pub sha256: String,
}

/// 一个挂载路径的探测结果（`ygg.fs.stat` 的 `{ exists, bytes, sha256, kind }`）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountStat {
    pub exists: bool,
    pub bytes: usize,
    pub sha256: String,
    pub kind: MountKind,
}

/// `ygg.fs.list` 的一行（`{ name, path, bytes, sha256, kind }`；`sha256` 恒为 `""`）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountEntry {
    pub name: String,
    pub path: String,
    pub bytes: usize,
    pub kind: MountKind,
}

/// 宿主侧文件挂载桥注入面（与 [`HostToolRunner`] 同型：沙箱只转发，零 IO）。
///
/// 实现方是持盘面（宿主 CLI / 测试桥），错误串必须是 `YGG-FS-*` 首段的人读
/// 文案——沙箱原样包成 Lua 运行时错误，不重新拼码。
pub trait MountFs {
    /// 读一个挂载文件（`miss → None`，与 `artifacts/` 面同口径）。
    fn read(&self, path: &str) -> Result<Option<Vec<u8>>, String>;
    /// 写一个挂载文件（写穿磁盘；成功即已落盘）。
    fn write(&mut self, path: &str, content: &[u8]) -> Result<MountWritten, String>;
    /// 探测一个挂载路径。
    fn stat(&self, path: &str) -> Result<MountStat, String>;
    /// 列一个挂载目录（不递归、不算摘要）。
    fn list(&self, path: &str) -> Result<Vec<MountEntry>, String>;
}

/// 沙箱侧挂载路径的前缀（与 `ygg_store::mounts::MOUNT_PATH_PREFIX` 同值；
/// 沙箱是零依赖面，不能引用 store 的常量）。
pub const MOUNT_PREFIX: &str = "mounts/";

/// 是否落在挂载面（`mounts` 本身或 `mounts/...`）。
pub(super) fn is_mount_path(path: &str) -> bool {
    path == "mounts" || path.starts_with(MOUNT_PREFIX)
}

/// 未挂载桥时的统一表述（码在首段：`nil` 与「文件不存在」不可分辨，静默降级
/// 会把「宿主没挂」读成「文件没了」）。
pub(super) fn unmounted_message(what: &str, path: &str) -> String {
    format!(
        "{}: 宿主未挂载文件桥，`{what}(\"{path}\")` 不可用（旧面 `artifacts/` 不受影响）",
        ygg_fs_code::UNMOUNTED
    )
}

/// 沙箱侧用到的两个错误码（与宿主侧 `ygg_store::mounts::codes` 同值）。
pub mod ygg_fs_code {
    /// 宿主未挂载文件桥。
    pub const UNMOUNTED: &str = "YGG-FS-UNMOUNTED";
    /// 路径首段既不是 `artifacts/` 也不是 `mounts`。
    pub const JAIL: &str = "YGG-FS-JAIL";
}

/// VFS 路径前缀（与 `ygg_store::ARTIFACT_PREFIX` 同值：VFS 的键就是制品的
/// run 内相对路径，宿主 flush 时直接当 `RunStore::artifact_put` 的入参）。
pub const VFS_PREFIX: &str = "artifacts/";
/// VFS 路径字节上限（与 `ygg_store::ARTIFACT_PATH_MAX_BYTES` 同值）。
pub const VFS_PATH_MAX_BYTES: usize = 128;
/// VFS 单件内容字节上限（与 `ygg_store::ARTIFACT_CONTENT_MAX_BYTES` 同值）。
pub const VFS_CONTENT_MAX_BYTES: usize = 256 * 1024;

/// 一件 VFS 文件（内容 + 摘要；摘要与 `ygg-store` 的制品面同编码）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VfsFile {
    pub bytes: Vec<u8>,
    pub sha256: String,
}

/// `ygg.fs` 的内存态 VFS。
///
/// 三条确定性纪律（改动前先读）：
/// 1. **`BTreeMap`，不是 `HashMap`**：`ygg.fs.list()` 的遍历序 = 键序 = 字典序，
///    同输入同字节（确定性路径一律有序容器——与 `ygg_core` 的 KV / 实体同纪律）。
/// 2. **`revision` 只增**：每次成功写 +1；宿主据此判断「有没有新东西要 flush」。
/// 3. **`dirty` 是落盘水位**：写完即入集合，宿主 `take_dirty()` 排空并写进
///    `RunStore::artifact_put`。它随 dispatch 级 savepoint 基线一起克隆/还原，
///    所以失败 dispatch 里写的文件**不会**漏到盘上（见 `dispatch::cmd_event`）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VfsStore {
    files: BTreeMap<String, VfsFile>,
    dirty: BTreeSet<String>,
    revision: u64,
}

impl VfsStore {
    /// 读一件文件（miss = `None`）。
    pub fn get(&self, path: &str) -> Option<&VfsFile> {
        self.files.get(path)
    }

    /// 全量条目（`BTreeMap` 序 = 字典序；`ygg.fs.list` 与宿主 flush 面共用）。
    pub fn entries(&self) -> impl Iterator<Item = (&str, &VfsFile)> + '_ {
        self.files.iter().map(|(path, file)| (path.as_str(), file))
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// 写代数（每次成功 `put` +1；预载与排空 **不**动它）。
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Lua 写面：先过路径与大小门，再算摘要、入表。
    ///
    /// 报错串以 `ygg.fs.write 拒绝：` 开头（可 grep），原因由门给。
    pub fn put(&mut self, path: &str, content: &[u8]) -> Result<VfsFile, String> {
        validate_vfs_path(path).map_err(|reason| format!("ygg.fs.write 拒绝：{reason}"))?;
        if content.len() > VFS_CONTENT_MAX_BYTES {
            return Err(format!(
                "ygg.fs.write 拒绝：`{path}` 的内容 {} 字节超过上限 {VFS_CONTENT_MAX_BYTES} 字节（256 KiB）",
                content.len()
            ));
        }
        let file = VfsFile {
            bytes: content.to_vec(),
            sha256: sha256_hex(content),
        };
        self.files.insert(path.to_owned(), file.clone());
        self.dirty.insert(path.to_owned());
        self.revision += 1;
        Ok(file)
    }

    /// 宿主预载（开局从 `RunStore::artifact_get` 回灌，或 `--artifact-put`）：
    /// **不校验内容、不入 dirty、不动 revision**——它不是本次会话产生的写入。
    ///
    /// 路径仍过门：预载来源（盘上的制品面）与调用方入参都不该有机会把
    /// `../` 之类的键塞进来，而 `ygg.fs.list` 会把它原样回执出去。
    pub fn load(&mut self, path: &str, content: &[u8], sha256: &str) {
        if validate_vfs_path(path).is_err() {
            return; // 静默跳过：预载面的错路径不是致命错误（不建键）
        }
        self.files.insert(
            path.to_owned(),
            VfsFile {
                bytes: content.to_vec(),
                sha256: sha256.to_owned(),
            },
        );
    }

    /// 排空并返回未落盘条目（`BTreeMap` 序）。宿主 flush 成功后调用。
    pub fn take_dirty(&mut self) -> Vec<(String, VfsFile)> {
        let paths: Vec<String> = self.dirty.iter().cloned().collect();
        self.dirty.clear();
        let mut out = Vec::with_capacity(paths.len());
        for path in paths {
            if let Some(file) = self.files.get(&path) {
                out.push((path, file.clone()));
            }
        }
        out
    }

    /// 把一批路径**重新标脏**（落盘中途失败后的回填）。
    ///
    /// 不动 `revision`、不动内容：没有新的写入发生，只是「这几件还没落盘」。
    /// 内容寻址让重试只重写相同字节，不会产生第二件制品。
    pub fn requeue_dirty(&mut self, paths: &[String]) {
        for path in paths {
            if self.files.contains_key(path) {
                self.dirty.insert(path.clone());
            }
        }
    }

    /// `ygg.fs.list()` 的回执形状（路径 / 字节数 / 摘要，`BTreeMap` 序）。
    pub fn snapshot(&self) -> Vec<(String, usize, String)> {
        self.files
            .iter()
            .map(|(path, file)| (path.clone(), file.bytes.len(), file.sha256.clone()))
            .collect()
    }
}

/// VFS 路径门（**本地、私有、fail-fast 面**）。规则与
/// `ygg_store::validate_artifact_path` **逐条相同**，却是另一份实现——这是
/// 有意的双门：`ygg-store` 是本 crate 的 **dev-dependency**，运行时无权调用
///（否则会把 `rusqlite` 拉进 wasm32）；而且两份门的角色不同：
///
/// | 门 | 位置 | 作用 |
/// | --- | --- | --- |
/// | 本函数 | Lua 调用点 | `ygg.fs.write("…/../x")` 当场可读报错，不等落盘 |
/// | `ygg_store::validate_artifact_path` | 端口内 | 权威持久化门（兼盖 `--artifact-put` 这类不经 Lua 的入口） |
///
/// **漂移守卫**：两边的测试向量表逐字相同（本文件的单测 ↔
/// `ygg-store/src/lib.rs` 的同名用例）。改规则必须一次提交改两边。
pub(super) fn validate_vfs_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.len() > VFS_PATH_MAX_BYTES {
        return Err(format!(
            "路径长度必须在 1..={VFS_PATH_MAX_BYTES} 字节之间（当前 {} 字节：`{path}`）",
            path.len()
        ));
    }
    let Some(rest) = path.strip_prefix(VFS_PREFIX) else {
        return Err(format!("路径必须以 `{VFS_PREFIX}` 起头（当前是 `{path}`）"));
    };
    if rest.is_empty() {
        return Err(format!(
            "`{VFS_PREFIX}` 之后必须还有文件名（当前是 `{path}`）"
        ));
    }
    if rest.ends_with('/') {
        return Err(format!("路径不得以 `/` 结尾（当前是 `{path}`）"));
    }
    if rest.split('/').any(str::is_empty) {
        return Err(format!(
            "路径不得含空路径段（`{VFS_PREFIX}` 之后是 `{rest}`）"
        ));
    }
    if let Some(ch) = path
        .chars()
        .find(|ch| !(ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '/' | '-')))
    {
        return Err(format!("路径含非法字符 `{ch}`（只允许 `[A-Za-z0-9._/-]`）"));
    }
    if path.contains("..") {
        return Err(format!("路径不得含 `..`（目录穿越；当前是 `{path}`）"));
    }
    Ok(())
}

/// `ygg.fs.*` 注册(内容寻址小文件面;原文迁入)。
pub(super) fn install_fs(
    lua: &Lua,
    state: &Rc<RefCell<HostState>>,
    ygg: &Table,
) -> Result<(), Error> {
    // ---- ygg.fs.*（内容寻址的小文件面：写经 VfsStore，读 / 列是纯读）。
    //      `fs.write` 在模块顶层被影子表拒绍（effects::BANNED_TABLE_MEMBERS），
    //      `fs.read` / `fs.stat` / `fs.list` 是纯读，任何相位都允许。
    //      合约：write 的参数类型必须是 (String, String)——不猜、不隐式转换
    //      （mlua 自己的 arity/type 报错已够人读）；read miss = nil；stat miss
    //      = `{exists=false,bytes=0,sha256=""}`（**恒回表，永不 nil**）；list 是
    //      从 1 开始的**稠密数组**。
    let fs_table = lua.create_table()?;

    let fs_write = Rc::clone(&state);
    fs_table.raw_set(
        "write",
        lua.create_function(
            move |lua, (path, content): (String, String)| -> Result<Table, Error> {
                let out = lua.create_table()?;
                // 首段分派：`mounts` 走挂载桥（写穿磁盘 + 审计），其余走旧内存 VFS。
                if is_mount_path(&path) {
                    let bridge = Rc::clone(&fs_write.borrow().mount);
                    let mut slot = bridge.borrow_mut();
                    let Some(bridge) = slot.as_mut() else {
                        return Err(Error::runtime(unmounted_message("fs.write", &path)));
                    };
                    let written = bridge
                        .write(&path, content.as_bytes())
                        .map_err(Error::runtime)?;
                    out.raw_set("path", written.path)?;
                    out.raw_set("bytes", written.bytes)?;
                    out.raw_set("sha256", written.sha256)?;
                    return Ok(out);
                }
                let vfs = Rc::clone(&fs_write.borrow().vfs);
                let file = vfs
                    .borrow_mut()
                    .put(&path, content.as_bytes())
                    .map_err(Error::runtime)?;
                out.raw_set("path", path)?;
                out.raw_set("bytes", file.bytes.len())?;
                out.raw_set("sha256", file.sha256)?;
                Ok(out)
            },
        )?,
    )?;

    let fs_read = Rc::clone(&state);
    fs_table.raw_set(
        "read",
        lua.create_function(move |lua, path: String| -> Result<LuaValue, Error> {
            if is_mount_path(&path) {
                let bridge = Rc::clone(&fs_read.borrow().mount);
                let slot = bridge.borrow();
                let Some(bridge) = slot.as_ref() else {
                    return Err(Error::runtime(unmounted_message("fs.read", &path)));
                };
                let hit = bridge.read(&path).map_err(Error::runtime)?;
                return Ok(match hit {
                    Some(bytes) => LuaValue::String(lua.create_string(&bytes)?),
                    None => LuaValue::Nil, // miss = nil（与旧面同口径）
                });
            }
            let vfs = Rc::clone(&fs_read.borrow().vfs);
            let hit = vfs.borrow().get(&path).map(|file| file.bytes.clone());
            Ok(match hit {
                Some(bytes) => LuaValue::String(lua.create_string(&bytes)?),
                None => LuaValue::Nil, // miss = nil，不 throw
            })
        })?,
    )?;

    let fs_stat = Rc::clone(&state);
    fs_table.raw_set(
        "stat",
        lua.create_function(move |lua, path: String| -> Result<Table, Error> {
            let out = lua.create_table()?;
            if is_mount_path(&path) {
                let bridge = Rc::clone(&fs_stat.borrow().mount);
                let slot = bridge.borrow();
                let Some(bridge) = slot.as_ref() else {
                    return Err(Error::runtime(unmounted_message("fs.stat", &path)));
                };
                let stat = bridge.stat(&path).map_err(Error::runtime)?;
                out.raw_set("exists", stat.exists)?;
                out.raw_set("bytes", stat.bytes)?;
                out.raw_set("sha256", stat.sha256)?;
                out.raw_set("kind", stat.kind.as_str())?;
                return Ok(out);
            }
            let vfs = Rc::clone(&fs_stat.borrow().vfs);
            let hit = vfs
                .borrow()
                .get(&path)
                .map(|file| (file.bytes.len(), file.sha256.clone()));
            match hit {
                Some((bytes, sha256)) => {
                    out.raw_set("exists", true)?;
                    out.raw_set("bytes", bytes)?;
                    out.raw_set("sha256", sha256)?;
                }
                None => {
                    out.raw_set("exists", false)?;
                    out.raw_set("bytes", 0)?;
                    out.raw_set("sha256", String::new())?;
                }
            }
            Ok(out)
        })?,
    )?;

    let fs_list = Rc::clone(&state);
    fs_table.raw_set(
        "list",
        lua.create_function(move |lua, prefix: Option<String>| -> Result<Table, Error> {
            let out = lua.create_table()?;
            // 无参 = 旧面（字节级不变）；有参且以 `mounts` 起头 = 挂载面；
            // 其余前缀 → 拒（不做「按前缀列 artifacts」的第二套遍历语义）。
            if let Some(path) = prefix {
                if !is_mount_path(&path) {
                    return Err(Error::runtime(format!(
                        "{}: ygg.fs.list 的前缀必须以 `mounts` 起头（当前是 `{path}`）",
                        ygg_fs_code::JAIL
                    )));
                }
                let bridge = Rc::clone(&fs_list.borrow().mount);
                let slot = bridge.borrow();
                let Some(bridge) = slot.as_ref() else {
                    return Err(Error::runtime(unmounted_message("fs.list", &path)));
                };
                let rows = bridge.list(&path).map_err(Error::runtime)?;
                for (index, row) in rows.into_iter().enumerate() {
                    let table = lua.create_table()?;
                    table.raw_set("name", row.name)?;
                    table.raw_set("path", row.path)?;
                    table.raw_set("bytes", row.bytes)?;
                    table.raw_set("sha256", String::new())?; // 不算摘要（键在场、值留白）
                    table.raw_set("kind", row.kind.as_str())?;
                    out.raw_set(index + 1, table)?; // 稠密数组：1..=n
                }
                return Ok(out);
            }
            let vfs = Rc::clone(&fs_list.borrow().vfs);
            let rows = vfs.borrow().snapshot(); // 已按 BTreeMap 序
            for (index, (path, bytes, sha256)) in rows.into_iter().enumerate() {
                let row = lua.create_table()?;
                row.raw_set("path", path)?;
                row.raw_set("bytes", bytes)?;
                row.raw_set("sha256", sha256)?;
                out.raw_set(index + 1, row)?; // **稠密数组**：1..=n
            }
            Ok(out)
        })?,
    )?;

    ygg.raw_set("fs", fs_table)?;

    Ok(())
}

// ============================================================================
// §crate 内单测：`VfsStore` 的账本（revision / dirty / load）与路径门向量表
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 路径门向量表——**与 `ygg-store/src/lib.rs` 的同名用例逐字同步**。
    ///
    /// 两份门的存在是有意的（`ygg-store` 是 dev-dependency，运行时无权调用）；
    /// 这份向量表就是漂移锤子：规则只改一边时必有一侧红。
    fn path_gate_vectors() -> (Vec<String>, Vec<String>) {
        let max_ok = format!(
            "artifacts/{}.md",
            "a".repeat(VFS_PATH_MAX_BYTES - VFS_PREFIX.len() - 3)
        );
        let too_long = format!(
            "artifacts/{}.md",
            "a".repeat(VFS_PATH_MAX_BYTES - VFS_PREFIX.len() - 2)
        );
        let accepted = vec![
            "artifacts/a.md".to_owned(),
            "artifacts/notes/x.md".to_owned(),
            "artifacts/a.b-c_d/e.md".to_owned(),
            "artifacts/0.md".to_owned(),
            "artifacts/a/b/c/d.md".to_owned(),
            max_ok,
        ];
        let rejected = vec![
            "artifacts/".to_owned(),
            "artifacts//x".to_owned(),
            "artifacts/x/".to_owned(),
            "artifacts/../x".to_owned(),
            "artifacts/a/../../x".to_owned(),
            "artifacts/a..b".to_owned(),
            "artifact/x".to_owned(),
            "/artifacts/x".to_owned(),
            "artifacts/a b.md".to_owned(),
            "artifacts/a\\b.md".to_owned(),
            "artifacts/a|b.md".to_owned(),
            "artifacts/a@b.md".to_owned(),
            too_long,
            String::new(),
            "artifacts".to_owned(),
        ];
        (accepted, rejected)
    }

    #[test]
    fn path_gate_accepts_legal_shapes() {
        let (accepted, _) = path_gate_vectors();
        for path in accepted {
            validate_vfs_path(&path).unwrap_or_else(|reason| {
                panic!("`{path}`（{} 字节）必须放行：{reason}", path.len())
            });
        }
        // 边界自证：最长向量恰好是上限（否则「刚好 128」根本没被测到）。
        assert_eq!(
            format!(
                "artifacts/{}.md",
                "a".repeat(VFS_PATH_MAX_BYTES - VFS_PREFIX.len() - 3)
            )
            .len(),
            VFS_PATH_MAX_BYTES
        );
    }

    #[test]
    fn path_gate_rejects_illegal_shapes() {
        let (_, rejected) = path_gate_vectors();
        for path in rejected {
            let reason = validate_vfs_path(&path).expect_err(&format!("`{path}` 必须被拒"));
            assert!(!reason.is_empty(), "拒绝必须带人读原因");
        }
    }

    /// `put` 的账本：revision +1、入 dirty、内容与摘要正确。
    #[test]
    fn put_bookkeeps_revision_and_dirty() {
        let mut vfs = VfsStore::default();
        assert!(vfs.is_empty());
        let file = vfs.put("artifacts/a.md", b"first").expect("写入");
        assert_eq!(file.bytes, b"first".to_vec());
        assert_eq!(file.sha256.len(), 64, "小写十六进制 64 字符");
        assert_eq!(vfs.len(), 1);
        assert_eq!(vfs.revision(), 1);
        assert_eq!(
            vfs.get("artifacts/a.md").map(|f| f.bytes.clone()),
            Some(b"first".to_vec())
        );

        // 覆盖写：仍然只有一件，revision 继续增。
        vfs.put("artifacts/a.md", b"second").expect("覆盖");
        assert_eq!(vfs.len(), 1);
        assert_eq!(vfs.revision(), 2);
        assert_eq!(
            vfs.get("artifacts/a.md").map(|f| f.bytes.clone()),
            Some(b"second".to_vec())
        );
    }

    /// `put` 的两道门：坏路径 / 超限内容都不建键、不动 revision。
    #[test]
    fn put_rejects_bad_path_and_oversize_content() {
        let mut vfs = VfsStore::default();
        let err = vfs.put("artifacts/../x", b"y").expect_err("穿越必须被拒");
        assert!(err.starts_with("ygg.fs.write 拒绝："), "{err}");
        let over = vec![0u8; VFS_CONTENT_MAX_BYTES + 1];
        let err = vfs
            .put("artifacts/big.md", &over)
            .expect_err("超限必须被拒");
        assert!(err.contains("256 KiB"), "错误必须给出上限：{err}");
        // 恰好上限放行（否则边界不可验证）。
        vfs.put("artifacts/max.md", &vec![0u8; VFS_CONTENT_MAX_BYTES])
            .expect("恰好上限必须放行");
        assert_eq!(vfs.len(), 1, "被拒的写不得建键");
        assert_eq!(vfs.revision(), 1, "被拒的写不得动 revision");
    }

    /// `load` 是预载面：不 dirty、不动 revision（它不是本次会话的写入）。
    #[test]
    fn load_is_not_dirty_and_does_not_bump_revision() {
        let mut vfs = VfsStore::default();
        vfs.load("artifacts/pre.md", b"preset", "deadbeef");
        assert_eq!(vfs.len(), 1);
        assert_eq!(vfs.revision(), 0, "预载不得当成本次写入");
        assert!(vfs.take_dirty().is_empty(), "预载不得入 dirty");
        assert_eq!(
            vfs.get("artifacts/pre.md").map(|f| f.sha256.clone()),
            Some("deadbeef".to_owned()),
            "预载的摘要原样接受（不重算——盘上那份才是权威）"
        );
        // 非法路径的预载静默跳过：不建键，也不污染键空间。
        vfs.load("../escape.md", b"x", "y");
        assert_eq!(vfs.len(), 1);
    }

    /// `take_dirty` 按 `BTreeMap` 序排空，且排空后不复报。
    #[test]
    fn take_dirty_is_ordered_and_drains() {
        let mut vfs = VfsStore::default();
        for path in ["artifacts/z.md", "artifacts/a.md", "artifacts/m.md"] {
            vfs.put(path, path.as_bytes()).expect("写入");
        }
        let dirty = vfs.take_dirty();
        assert_eq!(
            dirty
                .iter()
                .map(|(path, _)| path.as_str())
                .collect::<Vec<_>>(),
            vec!["artifacts/a.md", "artifacts/m.md", "artifacts/z.md"]
        );
        assert!(vfs.take_dirty().is_empty(), "排空后不得重复上报");
        assert_eq!(vfs.len(), 3, "排空 dirty 不删文件");
    }

    /// `snapshot`（=`ygg.fs.list` 的回执形状）按路径升序，与写入序无关。
    #[test]
    fn snapshot_is_sorted_by_path() {
        let mut vfs = VfsStore::default();
        for path in ["artifacts/b.md", "artifacts/a.md"] {
            vfs.put(path, b"x").expect("写入");
        }
        let rows: Vec<String> = vfs
            .snapshot()
            .into_iter()
            .map(|(path, bytes, sha)| format!("{path}:{bytes}:{}", sha.len()))
            .collect();
        assert_eq!(rows, vec!["artifacts/a.md:1:64", "artifacts/b.md:1:64"]);
    }

    /// 克隆语义：`VfsStore` 整体可克隆（savepoint 基线的加载前提）——
    /// 副本与原件独立，改副本不影响原件。
    #[test]
    fn clone_is_an_independent_snapshot() {
        let mut vfs = VfsStore::default();
        vfs.put("artifacts/a.md", b"one").expect("写入");
        let mut baseline = vfs.clone();
        vfs.put("artifacts/b.md", b"two").expect("后续写入");
        assert_eq!(vfs.len(), 2);
        assert_eq!(baseline.len(), 1, "基线必须冻结在克隆时点");
        assert_eq!(baseline.revision(), 1);
        // dirty 也随基线克隆：基线里只有克隆时点那一条，所以回滚到它
        // **不会**把失败步的路径带进下一次 flush。
        assert_eq!(
            baseline
                .take_dirty()
                .iter()
                .map(|(path, _)| path.as_str())
                .collect::<Vec<_>>(),
            vec!["artifacts/a.md"]
        );
    }
}
