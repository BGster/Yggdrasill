//! r1 `mounts` 面的 Lua 端 e2e（验收五点，`docs/design/kernel/lua-api.md` §3.1.2）。
//!
//! 每个用例在 `std::env::temp_dir()` 里现造一个 Mod 包（不落 fixture 文件、不碰
//! 任何业务目录）与一个 run 目录，把 `ygg_store::MountStore` 经 `MountFs` 注入
//! 到内核上——宿主侧实现与将来 CLI 的接线方式一致，只是这里少了 `egg.rs` 的
//! 六行装配（U1：CLI 激活另立需求）。
//!
//! 断言都以 **Lua 侧可见的东西**为准（回执里的叙述行、`pcall` 捕获的 message、
//! 磁盘上的字节、`fs.jsonl` 的行），而不是 Rust 侧的中间变量。
//!
//! # 一处读法说明（不是降级）
//!
//! mlua 会给 Rust 回调错误套两层装饰：前缀 `runtime error: ` 与尾部的
//! `\nstack traceback: …`。**码仍在 message 首段**——`bare_message` 只剥这两层，
//! 然后按「以 `YGG-FS-*:` 起头」断言。

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value as Json};
use ygg_sandbox::driver::KernelFacade;
use ygg_sandbox::host_api::{MountEntry, MountFs, MountKind, MountStat, MountWritten};
use ygg_sandbox::modload::ModBundle;
use ygg_sandbox::modules::ModuleSet;
use ygg_store::mounts::{MountStore, FS_AUDIT_FILE};

// ───────────────────────── 脚手架 ─────────────────────────

/// 把 `ygg_store::MountStore` 接成沙箱的挂载桥（宿主侧适配器的测试等价物）。
struct StoreBridge {
    store: MountStore,
}

fn kind_of(kind: ygg_store::MountKind) -> MountKind {
    match kind {
        ygg_store::MountKind::File => MountKind::File,
        ygg_store::MountKind::Dir => MountKind::Dir,
        ygg_store::MountKind::Missing => MountKind::Missing,
    }
}

impl MountFs for StoreBridge {
    fn read(&self, path: &str) -> Result<Option<Vec<u8>>, String> {
        self.store.read(path).map_err(|err| err.to_string())
    }

    fn write(&mut self, path: &str, content: &[u8]) -> Result<MountWritten, String> {
        let written = self
            .store
            .write(path, content)
            .map_err(|err| err.to_string())?;
        Ok(MountWritten {
            path: written.path,
            bytes: written.bytes,
            sha256: written.sha256,
        })
    }

    fn stat(&self, path: &str) -> Result<MountStat, String> {
        let stat = self.store.stat(path).map_err(|err| err.to_string())?;
        Ok(MountStat {
            exists: stat.exists,
            bytes: stat.bytes,
            sha256: stat.sha256,
            kind: kind_of(stat.kind),
        })
    }

    fn list(&self, path: &str) -> Result<Vec<MountEntry>, String> {
        let rows = self.store.list(path).map_err(|err| err.to_string())?;
        Ok(rows
            .into_iter()
            .map(|row| MountEntry {
                name: row.name,
                path: row.path,
                bytes: row.bytes,
                kind: kind_of(row.kind),
            })
            .collect())
    }
}

/// 本用例独占的临时目录。
fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ygg-r1-fs-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// 在 `dir` 里写一个最小 Mod 包（`pack.json` 的 `mounts` 段可声明），返回 pack JSON。
fn write_mod(dir: &Path, mounts: Json, entry: &str) -> Json {
    fs::create_dir_all(dir).expect("create mod dir");
    fs::write(dir.join("mod.lua"), entry).expect("write mod.lua");
    let pack = json!({
        "schemaVersion": 1,
        "entries": [],
        "schemaFiles": [],
        "mounts": mounts,
    });
    fs::write(
        dir.join("pack.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&pack).expect("pack json")
        ),
    )
    .expect("write pack.json");
    fs::write(
        dir.join("manifest.json"),
        format!(
            "{}\n",
            serde_json::to_string(&json!({
                "schemaVersion": 1,
                "entries": [],
                "code": [],
            }))
            .expect("manifest json")
        ),
    )
    .expect("write manifest.json");
    pack
}

/// 「一个 run 目录 + 一个宿主目录 + 已接上桥的内核」的三件套。
struct Fixture {
    run: PathBuf,
    host: PathBuf,
    kernel: KernelFacade,
}

/// 装一局：宿主目录已建好（`<root>/mod/biz`，即 `dir: "biz"` 的落点）。
fn attach_fixture(tag: &str, mounts: Json, entry: &str) -> Fixture {
    let root = temp_dir(tag);
    let run = root.join("run");
    fs::create_dir_all(&run).expect("create run dir");
    let mod_dir = root.join("mod");
    let host = mod_dir.join("biz");
    fs::create_dir_all(&host).expect("create host dir");
    let pack = write_mod(&mod_dir, mounts, entry);
    let store = MountStore::attach(&run, &pack, &mod_dir).expect("attach");
    let bundle = ModBundle::load(&mod_dir).expect("装载 Mod 包");
    let mut kernel = bundle.kernel().expect("构造内核");
    kernel.set_mount_fs(Some(Box::new(StoreBridge { store })));
    Fixture { run, host, kernel }
}

fn parse(reply: &str) -> Json {
    serde_json::from_str(reply).expect("回执必须是 JSON")
}

/// 派发并取回叙述行（信封必须 ok）。
fn narrate(kernel: &mut KernelFacade, event: &str) -> Vec<String> {
    let reply = parse(&kernel.dispatch(event, "0"));
    assert_eq!(reply["ok"], json!(true), "{event} 派发失败：{reply}");
    reply["narrative"]
        .as_array()
        .expect("narrative 数组")
        .iter()
        .map(|line| line.as_str().expect("叙述行是字符串").to_owned())
        .collect()
}

/// mlua 的错误串 → 纯 message：剥掉 `runtime error: ` 前缀与 `\nstack traceback: …`。
fn bare_message(text: &str) -> &str {
    let first = text.split('\n').next().unwrap_or("").trim();
    first.strip_prefix("runtime error: ").unwrap_or(first)
}

/// 叙述行里 ` err=` 之后的裸 message（码必须仍在首段）。
#[track_caller]
fn err_message(line: &str) -> &str {
    let tail = line
        .split(" err=")
        .nth(1)
        .unwrap_or_else(|| panic!("叙述行必须带 ` err=`：{line}"));
    bare_message(tail)
}

/// 读 `fs.jsonl` 的各行（审计面的唯一读取方式：行就是事实）。
fn audit_rows(run: &Path) -> Vec<Json> {
    match fs::read_to_string(run.join(FS_AUDIT_FILE)) {
        Ok(text) => text
            .lines()
            .map(|line| serde_json::from_str(line).expect("审计行必须是 JSON"))
            .collect(),
        Err(err) => panic!("审计文件不可读：{err}"),
    }
}

// ───────────────────────── ① 读写列举成功 ─────────────────────────

#[test]
fn test_mod_writes_reads_and_lists_a_mount() {
    let entry = r#"
ygg.on("probe.mount", function()
  local written = ygg.fs.write("mounts/work/b.txt", "22")
  ygg.narrate("write path=" .. written.path .. " bytes=" .. written.bytes)
  ygg.narrate("write sha=" .. written.sha256)
  ygg.fs.write("mounts/work/a.txt", "hello")
  ygg.narrate("read=" .. tostring(ygg.fs.read("mounts/work/a.txt")))
  ygg.narrate("miss=" .. tostring(ygg.fs.read("mounts/work/none.txt")))
  local st = ygg.fs.stat("mounts/work/a.txt")
  ygg.narrate("stat exists=" .. tostring(st.exists) .. " kind=" .. st.kind
    .. " bytes=" .. st.bytes .. " sha=" .. st.sha256)
  local rows = ygg.fs.list("mounts/work")
  ygg.narrate("list n=" .. #rows)
  for _, row in ipairs(rows) do
    ygg.narrate("row " .. row.name .. " " .. row.kind .. " " .. row.path
      .. " bytes=" .. row.bytes .. " sha=[" .. row.sha256 .. "]")
  end
  local root = ygg.fs.stat("mounts/work")
  ygg.narrate("root kind=" .. root.kind .. " bytes=" .. root.bytes)
end)
"#;
    let mut fixture = attach_fixture(
        "e2e-rw",
        json!([{ "name": "work", "dir": "biz", "mode": "rw" }]),
        entry,
    );
    let lines = narrate(&mut fixture.kernel, "probe.mount");

    // 回执形状 { path, bytes, sha256 }。
    assert_eq!(
        lines[0], "write path=mounts/work/b.txt bytes=2",
        "{lines:?}"
    );
    assert_eq!(
        lines[1],
        format!("write sha={}", ygg_store::sha256_hex(b"22")),
        "{lines:?}"
    );
    // read 命中 / miss（miss = nil，与 artifacts/ 面同口径）。
    assert_eq!(lines[2], "read=hello", "{lines:?}");
    assert_eq!(lines[3], "miss=nil", "{lines:?}");
    // stat：kind / bytes / 摘要 = sha256(content)。
    assert_eq!(
        lines[4],
        format!(
            "stat exists=true kind=file bytes=5 sha={}",
            ygg_store::sha256_hex(b"hello")
        ),
        "{lines:?}"
    );
    // list：按 name 升序、kind 正确、sha256 留白、路径是沙箱路径。
    assert_eq!(lines[5], "list n=2", "{lines:?}");
    assert_eq!(
        lines[6], "row a.txt file mounts/work/a.txt bytes=5 sha=[]",
        "{lines:?}"
    );
    assert_eq!(
        lines[7], "row b.txt file mounts/work/b.txt bytes=2 sha=[]",
        "{lines:?}"
    );
    // 挂载根本身是目录。
    assert_eq!(lines[8], "root kind=dir bytes=0", "{lines:?}");

    // 写穿磁盘：字节逐字落在宿主目录里（不是沙箱里的副本）。
    assert_eq!(
        fs::read(fixture.host.join("a.txt")).expect("读宿主侧 a.txt"),
        b"hello"
    );
    assert_eq!(
        fs::read(fixture.host.join("b.txt")).expect("读宿主侧 b.txt"),
        b"22"
    );
}

// ───────────────────────── ② 路径狱逃逸拒绝 ─────────────────────────

#[test]
fn jail_rejects_escape_vectors() {
    let entry = r#"
local vectors = {
  "mounts",
  "mounts/",
  "mounts/other/a.txt",
  "mounts/work/",
  "mounts/work//a.txt",
  "mounts/work/./a.txt",
  "mounts/work/../a.txt",
  "mounts/work/a/../../b.txt",
  "mounts/work/nope/a.txt",
}
ygg.on("probe.jail", function()
  for _, path in ipairs(vectors) do
    local ok, err = pcall(function() ygg.fs.write(path, "x") end)
    ygg.narrate(path .. "|ok=" .. tostring(ok) .. " err=" .. tostring(err))
  end
  -- `fs.list` 的前缀只能是 mounts：artifacts/ 前缀不提供第二套遍历语义。
  local ok_list, err_list = pcall(function() ygg.fs.list("artifacts/") end)
  ygg.narrate("list-artifacts|ok=" .. tostring(ok_list) .. " err=" .. tostring(err_list))
  -- 旧面不受影响：非 mounts 首段仍走既有 VFS 门（文案与行为不变）。
  local ok_old, err_old = pcall(function() ygg.fs.write("notes/a.txt", "x") end)
  ygg.narrate("old-face|ok=" .. tostring(ok_old) .. " err=" .. tostring(err_old))
end)
"#;
    let mut fixture = attach_fixture(
        "e2e-jail",
        json!([{ "name": "work", "dir": "biz", "mode": "rw" }]),
        entry,
    );
    let lines = narrate(&mut fixture.kernel, "probe.jail");
    assert_eq!(lines.len(), 11, "9 个路径向量 + list + 旧面：{lines:?}");
    for line in lines.iter().take(10) {
        assert!(
            line.contains("ok=false"),
            "每个向量都必须被拒：{line}"
        );
        let message = err_message(line);
        assert!(
            message.starts_with("YGG-FS-JAIL: "),
            "message 必须以 `YGG-FS-JAIL: ` 起头：{line}"
        );
    }
    // 旧面的门上没有 YGG-FS 码——旧面逐字节不变的代价，明确钉住别被“顺手统一”。
    let old = err_message(&lines[10]);
    assert!(
        old.contains("必须以 `artifacts/` 起头"),
        "非 mounts 首段仍归既有 VFS 门：{old}"
    );
    assert!(
        !old.starts_with("YGG-FS-"),
        "旧面不得被改写成挂载码：{old}"
    );
    // 越狱后的落点一个都不存在（磁盘零写入）。
    for name in ["a.txt", "b.txt", "a", "b"] {
        assert!(
            !fixture.host.join(name).exists(),
            "虚写不得在宿主侧留下 `{name}`"
        );
    }
    assert!(
        !fixture.host.join("nope").exists(),
        "不存在的父目录不得被顺手建出来"
    );
}

// ───────────────────────── ③ 审计行 ─────────────────────────

#[test]
fn audit_rows_are_appended_per_write() {
    let entry = r#"
ygg.on("probe.audit", function()
  ygg.fs.write("mounts/work/a.txt", "hello")
  ygg.fs.write("mounts/work/b.txt", "22")
  local ok, err = pcall(function() ygg.fs.write("mounts/work/../x", "x") end)
  ygg.narrate("rejected|ok=" .. tostring(ok) .. " err=" .. tostring(err))
end)
"#;
    let mut fixture = attach_fixture(
        "e2e-audit",
        json!([{ "name": "work", "dir": "biz", "mode": "rw" }]),
        entry,
    );
    let lines = narrate(&mut fixture.kernel, "probe.audit");
    let message = err_message(&lines[0]);
    assert!(
        message.starts_with("YGG-FS-JAIL: "),
        "被拒的写必须浮出码：{message}"
    );

    let rows = audit_rows(&fixture.run);
    assert_eq!(rows.len(), 3, "2 次成功 + 1 次被拒：{rows:?}");
    assert_eq!(rows[0]["seq"], json!(1));
    assert_eq!(rows[0]["event"], json!("fs.write"));
    assert_eq!(rows[0]["mount"], json!("work"));
    assert_eq!(rows[0]["path"], json!("mounts/work/a.txt"));
    assert_eq!(rows[0]["bytes"], json!(5));
    assert_eq!(rows[0]["sha256"], json!(ygg_store::sha256_hex(b"hello")));
    assert_eq!(rows[1]["seq"], json!(2));
    assert_eq!(rows[1]["path"], json!("mounts/work/b.txt"));
    assert_eq!(rows[2]["seq"], json!(3));
    assert_eq!(rows[2]["event"], json!("fs.write.rejected"));
    assert_eq!(rows[2]["reason"], json!("YGG-FS-JAIL"));
    assert_eq!(rows[2]["bytes"], json!(0));
    assert_eq!(rows[2]["sha256"], json!(""));
    // 审计行住在 run 目录的独立文件里——不进 events.jsonl（后者一行 = 一次 dispatch）。
    assert!(fixture.run.join("fs.jsonl").exists());
}

// ───────────────────────── ④ 未挂载降级 ─────────────────────────

#[test]
fn unmounted_bridge_reports_ygg_fs_unmounted() {
    let entry = r#"
ygg.on("probe.unmounted", function()
  local ok_write, err_write = pcall(function() ygg.fs.write("mounts/work/a.txt", "x") end)
  ygg.narrate("write|ok=" .. tostring(ok_write) .. " err=" .. tostring(err_write))
  local ok_read, err_read = pcall(function() ygg.fs.read("mounts/work/a.txt") end)
  ygg.narrate("read|ok=" .. tostring(ok_read) .. " err=" .. tostring(err_read))
  local ok_stat, err_stat = pcall(function() ygg.fs.stat("mounts/work") end)
  ygg.narrate("stat|ok=" .. tostring(ok_stat) .. " err=" .. tostring(err_stat))
  local ok_list, err_list = pcall(function() ygg.fs.list("mounts/work") end)
  ygg.narrate("list|ok=" .. tostring(ok_list) .. " err=" .. tostring(err_list))
  -- 旧面不受影响（回归钉）：artifacts/ 写读仍成功。
  local written = ygg.fs.write("artifacts/note.md", "keep")
  ygg.narrate("artifacts|bytes=" .. written.bytes .. " read=" .. tostring(ygg.fs.read("artifacts/note.md")))
end)
"#;
    // 不注入桥：只有 Mod 包 + 内核。
    let root = temp_dir("e2e-unmounted");
    let mod_dir = root.join("mod");
    write_mod(
        &mod_dir,
        json!([{ "name": "work", "dir": "biz", "mode": "rw" }]),
        entry,
    );
    let bundle = ModBundle::load(&mod_dir).expect("装载 Mod 包");
    let mut kernel = bundle.kernel().expect("构造内核");
    let lines = narrate(&mut kernel, "probe.unmounted");
    assert_eq!(lines.len(), 5, "{lines:?}");
    for (op, line) in ["write", "read", "stat", "list"].iter().zip(lines.iter()) {
        let tag = format!("{op}|");
        let body = line.strip_prefix(&tag).expect("叙述行带操作名");
        assert!(body.contains("ok=false"), "`{op}` 必须降级：{line}");
        let message = err_message(line);
        assert!(
            message.starts_with("YGG-FS-UNMOUNTED: "),
            "`{op}` 的降级必须是码在首段的可读错误：{message}"
        );
    }
    assert_eq!(
        lines[4], "artifacts|bytes=4 read=keep",
        "旧面必须逐字节不变：{lines:?}"
    );
}

// ───────────────────────── ⑤ 模块顶层禁写继承 ─────────────────────────

#[test]
fn module_top_level_mount_write_is_refused() {
    // 模块 chunk 的顶层调 `ygg.fs.write("mounts/…")` → 影子表拒绝。这条钉住
    // 「挂载分支继承了既有禁止面」：`crates/ygg-modules` 的
    // `BANNED_TABLE_MEMBERS` 里本来就有 `("fs", "write")`，零改动即覆盖新面。
    //
    // 注：`YGG-MOD-EFFECT` 是**静态侧**的规则码（`ygg-lint --pack`），运行期
    // 影表的拒绝是人读文案 + 调用点行号（同一份 `ygg_modules::effects` 清单）。
    let mut kernel = KernelFacade::with_modules(
        r#"
local ok, err = pcall(require, "src.boom")
ygg.on("probe.top", function()
  ygg.narrate("ok=" .. tostring(ok))
  ygg.narrate(tostring(err))
end)
"#,
        ModuleSet::from_pairs([(
            "src.boom".to_owned(),
            b"local M = {}\nygg.fs.write(\"mounts/work/a.txt\", \"x\")\nreturn M\n".to_vec(),
        )])
        .expect("module set"),
    )
    .expect("抛错的模块不能把整包装载拖垮（入口 pcall 得住）");
    let lines = narrate(&mut kernel, "probe.top");
    assert_eq!(lines[0], "ok=false", "模块顶层写挂载点必须失败：{lines:?}");
    assert!(
        lines[1].contains("模块顶层不能调用 `ygg.fs.write`"),
        "拒绍必须点名被禁的 API：{lines:?}"
    );
    assert!(
        lines[1].contains("src/boom.lua:2"),
        "拒绍必须带模块自己的行号：{lines:?}"
    );
}
