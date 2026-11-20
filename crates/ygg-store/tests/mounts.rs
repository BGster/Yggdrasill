//! r1 挂载桥的**狱向量表**（`docs/design/kernel/lua-api.md` §3.1.2 的七条拒绝
//! 条件 × 每条的现实逃逸样本）。
//!
//! 这一组的价值在「反例必须被拒」：每个向量都是一个**看起来能通**的写法——
//! 前缀相同但不同根的路径、`..` 折叠、中间段软链、字符串前缀陷阱
//! （`/a/bc` 与 `/a/b`）、末段软链、父缺失。四个入口（`read` / `write` /
//! `stat` / `list`）共用同一个路径狱实现，所以逐向量四入口同时断言。
//!
//! 全程 `std::env::temp_dir()` 合成数据（数据红线：不碰任何业务目录）。

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value as Json};
use ygg_store::mounts::{
    codes, MountError, MountKind, MountMode, MountStore, MOUNT_FILE_MAX_BYTES,
    MOUNT_LIST_MAX_ENTRIES,
};

/// 本用例独占的临时目录。
fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ygg-r1-jail-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// 一个 run 目录 + 一个 mod 目录（attach 的两个入参）。
fn attach_env(tag: &str) -> (PathBuf, PathBuf) {
    let root = temp_dir(tag);
    let run = root.join("run");
    fs::create_dir_all(&run).expect("create run dir");
    let mod_dir = root.join("mod");
    fs::create_dir_all(&mod_dir).expect("create mod dir");
    (run, mod_dir)
}

fn attach(run: &Path, mod_dir: &Path, mounts: Json) -> MountStore {
    MountStore::attach(run, &json!({ "schemaVersion": 1, "mounts": mounts }), mod_dir)
        .expect("attach 必须成功")
}

/// 四个入口对同一个路径都必须 `YGG-FS-JAIL`，且码在 message 首段。
#[track_caller]
fn assert_jail(store: &mut MountStore, path: &str, hint: &str) {
    let results: [(&str, Result<(), MountError>); 4] = [
        ("read", store.read(path).map(|_hit| ())),
        ("write", store.write(path, b"x").map(|_written| ())),
        ("stat", store.stat(path).map(|_stat| ())),
        ("list", store.list(path).map(|_rows| ())),
    ];
    for (op, result) in results {
        match result {
            Ok(()) => panic!("{op}(`{path}`) 必须拒（{hint}）"),
            Err(err) => {
                assert_eq!(
                    err.code(),
                    codes::JAIL,
                    "{op}(`{path}`) 的码不对（{hint}）：{err}"
                );
                let text = err.to_string();
                assert!(
                    text.starts_with("YGG-FS-JAIL: "),
                    "{op}(`{path}`) 的码必须在首段：{text}"
                );
                assert!(
                    !text[err.code().len()..].trim().is_empty(),
                    "拒绍必须带人读原因：{text}"
                );
            }
        }
    }
}

#[test]
fn jail_rejects_escape_vector_table() {
    let (run, mod_dir) = attach_env("table");
    let mut store = attach(
        &run,
        &mod_dir,
        json!([{ "name": "w", "mode": "rw" }, { "name": "work", "mode": "rw" }]),
    );

    // 规则 1：首段必须是 mounts/ 且名字必须完整（`mountsfoo` / `Mounts` / 裸 `mounts`）。
    for (path, hint) in [
        ("artifacts/x.md", "旧面根不是挂载根"),
        ("/mounts/w/a.txt", "绝对路径不是沙箱路径"),
        ("mountsfoo/w/a.txt", "字面前缀不是段前缀"),
        ("Mounts/w/a.txt", "大小写不同"),
        ("mounts", "必须带挂载点名"),
        ("", "空路径"),
    ] {
        assert_jail(&mut store, path, hint);
    }

    // 规则 2：`<name>` 必须在声明集内。
    for (path, hint) in [
        ("mounts/other/a.txt", "未声明的挂载点"),
        ("mounts/W/a.txt", "名字区分大小写"),
        ("mounts/artifacts/a.txt", "保留名不是挂载点"),
    ] {
        assert_jail(&mut store, path, hint);
    }

    // 规则 3：空段 / `.` / `..` / NUL / ASCII 控制字符。
    for (path, hint) in [
        ("mounts/w/", "结尾斜杠 = 空段"),
        ("mounts/w//a.txt", "双斜杠 = 空段"),
        ("mounts/w/./a.txt", "`.` 段"),
        ("mounts/w/../a.txt", "`..` 段（向上逃逸）"),
        ("mounts/w/a/../../b.txt", "折叠后向上逃逸"),
        ("mounts/w/a\u{0}b", "NUL"),
        ("mounts/w/a\u{1}b", "C0 控制字符"),
        ("mounts/w/a\u{7f}b", "DEL"),
    ] {
        assert_jail(&mut store, path, hint);
    }

    // 规则 4：路径 > 512 字节 / 段数 > 24。
    let long = format!("mounts/w/{}", "x".repeat(600));
    assert_jail(&mut store, &long, "超过 512 字节");
    let deep = format!("mounts/w/{}", vec!["d"; 25].join("/"));
    assert_jail(&mut store, &deep, "超过 24 段");
    // 恰好 24 段是合法的文法（父目录不存在由规则 7 拒，不是段数）。
    let edge = format!("mounts/w/{}", vec!["d"; 24].join("/"));
    let err = store
        .read(&edge)
        .expect_err("24 段但父缺失，必须拒");
    assert_eq!(err.code(), codes::JAIL, "{err}");
    assert!(
        err.to_string().contains("父目录"),
        "24 段不该被段数规则拒（应落在父缺失）：{err}"
    );

    // 规则 7：父目录不存在（r1 不做挂载点内的 mkdir）。
    assert_jail(&mut store, "mounts/w/nope/a.txt", "父目录不存在");
    assert_jail(&mut store, "mounts/work/out/deep/a.txt", "多级父缺失");

    // 合法面：挂载根本身是目录（stat / list 收，read / write 拒）。
    assert_eq!(store.stat("mounts/w").expect("stat root").kind.as_str(), "dir");
    assert!(store.list("mounts/w").expect("list root").is_empty());
    match store.read("mounts/w") {
        Ok(_) => panic!("挂载根不能当文件读"),
        Err(err) => assert_eq!(err.code(), codes::JAIL, "{err}"),
    }
}

#[cfg(unix)]
#[test]
fn jail_rejects_symlink_and_prefix_traps() {
    let (run, mod_dir) = attach_env("symlink");
    let mut store = attach(
        &run,
        &mod_dir,
        json!([{ "name": "w", "mode": "rw" }]),
    );
    let root = run.join("mounts").join("w");
    fs::create_dir_all(root.join("sub")).expect("mkdir sub");
    fs::write(root.join("sub/real.txt"), b"real").expect("write real");
    fs::create_dir_all(mod_dir.join("outside")).expect("mkdir outside");
    fs::write(mod_dir.join("outside/secret.txt"), b"secret").expect("write secret");
    // 字符串前缀陷阱：`<run>/mounts/w-evil` 与 `<run>/mounts/w` 共享字符串前缀，
    // 但**不是**它的路径分量前缀——按字符串比较的实现会在这里放行。
    let evil = run.join("mounts").join("w-evil");
    fs::create_dir_all(&evil).expect("mkdir evil");
    fs::write(evil.join("trap.txt"), b"trap").expect("write trap");

    // 规则 6：末段是软链（含断链）→ 一律拒。
    std::os::unix::fs::symlink(mod_dir.join("outside"), root.join("escape")).expect("symlink");
    std::os::unix::fs::symlink(mod_dir.join("nope"), root.join("dangling")).expect("dangling");
    assert_jail(&mut store, "mounts/w/escape", "末段是指向外的软链");
    assert_jail(&mut store, "mounts/w/dangling", "末段是断链");
    // 写穿也拒：外部目标文件不得被创建。
    assert!(!mod_dir.join("outside/secret2.txt").exists());

    // 规则 5：中间段是软链时，canonical 结果必须仍在冻结根内。
    std::os::unix::fs::symlink(mod_dir.join("outside"), root.join("outdir")).expect("outdir");
    assert_jail(&mut store, "mounts/w/outdir/secret.txt", "中间段逃逸到根外");
    std::os::unix::fs::symlink(&evil, root.join("evil")).expect("evil link");
    assert_jail(
        &mut store,
        "mounts/w/evil/trap.txt",
        "字符串前缀陷阱（w-evil 不是 w 的分量前缀）",
    );

    // 正面：中间段软链**留在根内**是允许的（bind 语义的现实用法）。
    std::os::unix::fs::symlink(root.join("sub"), root.join("alias")).expect("alias");
    assert_eq!(
        store.read("mounts/w/alias/real.txt").expect("read through"),
        Some(b"real".to_vec())
    );
    assert_eq!(store.stat("mounts/w/alias/real.txt").expect("stat").bytes, 4);
    // 末段是软链的目录不能 list（「末段永不是软链」对最后一个段一律生效）。
    match store.list("mounts/w/alias") {
        Ok(_) => panic!("末段是软链的目录不能 list"),
        Err(err) => assert_eq!(err.code(), codes::JAIL, "{err}"),
    }
    let rows = store.list("mounts/w/sub").expect("list sub");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].path, "mounts/w/sub/real.txt", "路径按请求路径拼");

    // 未被拒的写确实落了盘（「拒」不是「一律不写」的反面证据）。
    store.write("mounts/w/sub/written.txt", b"ok").expect("write");
    assert_eq!(
        fs::read(root.join("sub/written.txt")).expect("read back"),
        b"ok"
    );
    // 而所有逃逸写都没有在根外留下任何字节。
    assert_eq!(
        fs::read(mod_dir.join("outside/secret.txt")).expect("secret intact"),
        b"secret"
    );
    assert!(!mod_dir.join("outside/written.txt").exists());
}

#[test]
fn declared_modes_are_reported_as_declared() {
    let (run, mod_dir) = attach_env("modes");
    let store = attach(
        &run,
        &mod_dir,
        json!([{ "name": "ro_one" }, { "name": "rw_one", "mode": "rw" }]),
    );
    let modes: Vec<(&str, MountMode)> = store
        .declared()
        .map(|(name, root, mode)| {
            assert!(root.is_dir(), "冻结根必须是目录：{}", root.display());
            (name, mode)
        })
        .collect();
    assert_eq!(
        modes,
        vec![("ro_one", MountMode::Ro), ("rw_one", MountMode::Rw)],
        "缺省 ro；declared() 按名字序"
    );
}

// ============================================================================
// §行为面：三态幂等 / 往返落盘 / 帽 / 审计行 / list 形状（公开 API 口径）
// ============================================================================

/// 审计文件的各行（测试面直接读 JSON）。
fn audit_rows(store: &MountStore) -> Vec<Json> {
    match fs::read_to_string(store.audit_path()) {
        Ok(text) => text
            .lines()
            .map(|line| serde_json::from_str(line).expect("审计行必须是 JSON"))
            .collect(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(err) => panic!("读审计文件失败：{err}"),
    }
}

#[test]
fn attach_is_idempotent_and_conflicts_write_nothing() {
    let (run, mod_dir) = attach_env("idem");
    let host = mod_dir.join("biz");
    fs::create_dir_all(&host).expect("create host dir");
    let pack = json!({ "schemaVersion": 1, "mounts": [
        { "name": "work", "dir": "biz", "mode": "rw" },
        { "name": "out" },
    ] });
    let first = MountStore::attach(&run, &pack, &mod_dir).expect("first attach");
    let roots: Vec<(String, PathBuf)> = first
        .declared()
        .map(|(name, root, _mode)| (name.to_owned(), root.to_path_buf()))
        .collect();
    // 第二个实例（等价于「续局 / 另一进程」）：静默通过，冻结根一致。
    let second = MountStore::attach(&run, &pack, &mod_dir).expect("second attach");
    let again: Vec<(String, PathBuf)> = second
        .declared()
        .map(|(name, root, _mode)| (name.to_owned(), root.to_path_buf()))
        .collect();
    assert_eq!(roots, again, "重载必须落到同一批冻结根");
    assert_eq!(second.budget_left(), ygg_store::mounts::MOUNT_RUN_WRITE_BUDGET);

    // 把私有挂载点换成真目录而声明仍是目录挂载 → 冲突，且另一条声明也不落地。
    let other = temp_dir("idem-conflict");
    let other_run = other.join("run");
    fs::create_dir_all(other_run.join("mounts").join("work")).expect("pre-create");
    match MountStore::attach(&other_run, &pack, &mod_dir) {
        Ok(_) => panic!("冲突必须拒"),
        Err(err) => assert_eq!(err.code(), codes::MOUNT, "{err}"),
    }
    assert!(
        !other_run.join("mounts").join("out").exists(),
        "冲突时其余挂载点不得被创建（零写入）"
    );
}

#[test]
fn write_read_roundtrip_lands_on_the_host_dir_and_survives_reattach() {
    let (run, mod_dir) = attach_env("roundtrip");
    let host = mod_dir.join("biz");
    fs::create_dir_all(&host).expect("create host dir");
    let pack = json!({ "schemaVersion": 1, "mounts": [
        { "name": "work", "dir": "biz", "mode": "rw" },
    ] });
    let mut store = MountStore::attach(&run, &pack, &mod_dir).expect("attach");
    let content = b"# note\nline 2\n";
    let written = store.write("mounts/work/note.md", content).expect("write");
    assert_eq!(written.path, "mounts/work/note.md");
    assert_eq!(written.bytes, content.len());
    assert_eq!(written.sha256, ygg_store::sha256_hex(content));
    // 写穿：字节就是落在宿主目录里的那一份（不拷贝、不另存）。
    assert_eq!(fs::read(host.join("note.md")).expect("read host file"), content);
    // 摘要 / 大小 / 种类三面一致。
    assert_eq!(
        store.stat("mounts/work/note.md").expect("stat"),
        ygg_store::MountStat {
            exists: true,
            bytes: content.len(),
            sha256: ygg_store::sha256_hex(content),
            kind: MountKind::File,
        }
    );
    assert_eq!(
        store.read("mounts/work/note.md").expect("read"),
        Some(content.to_vec())
    );
    // 续局（新实例，预算与审计 seq 都重新装载）之后内容仍在。
    let mut again = MountStore::attach(&run, &pack, &mod_dir).expect("reattach");
    assert_eq!(
        again.read("mounts/work/note.md").expect("read after reattach"),
        Some(content.to_vec())
    );
    assert_eq!(audit_rows(&again).len(), 1, "审计行跨 attach 累积");
    again
        .write("mounts/work/note2.md", b"x")
        .expect("write after reattach");
    assert_eq!(
        audit_rows(&again)[1]["seq"],
        json!(2),
        "新实例从磁盘上的非空行数继续编号（不是从 1 重来）"
    );
}

#[test]
fn caps_refuse_instead_of_truncating() {
    let (run, mod_dir) = attach_env("caps");
    let mut store = attach(&run, &mod_dir, json!([{ "name": "w", "mode": "rw" }]));
    let root = run.join("mounts").join("w");
    // 读侧：磁盘上的超大文件（他处写入）→ 帽拒，不截断。
    fs::write(root.join("big.bin"), vec![b'x'; MOUNT_FILE_MAX_BYTES + 1]).expect("write big");
    match store.read("mounts/w/big.bin") {
        Ok(_) => panic!("超单件帽必须拒"),
        Err(err) => assert_eq!(err.code(), codes::CAP, "{err}"),
    }
    // 写侧：同样帽（超限时磁盘零写入）。
    match store.write("mounts/w/big2.bin", &vec![b'y'; MOUNT_FILE_MAX_BYTES + 1]) {
        Ok(_) => panic!("超单件帽必须拒"),
        Err(err) => assert_eq!(err.code(), codes::CAP, "{err}"),
    }
    assert!(!root.join("big2.bin").exists(), "被拒的写不得落盘");
    // stat 也受同一帽（算摘要要读全文件）。
    match store.stat("mounts/w/big.bin") {
        Ok(_) => panic!("超单件帽必须拒"),
        Err(err) => assert_eq!(err.code(), codes::CAP, "{err}"),
    }
}

#[test]
fn list_shape_is_dense_sorted_and_capped() {
    let (run, mod_dir) = attach_env("list-shape");
    let mut store = attach(&run, &mod_dir, json!([{ "name": "w", "mode": "rw" }]));
    let root = run.join("mounts").join("w");
    for name in ["c.txt", "a.txt", "b.txt"] {
        fs::write(root.join(name), b"1").expect("write");
    }
    fs::create_dir_all(root.join("dir")).expect("mkdir");
    store.write("mounts/w/d.txt", b"22").expect("write via store");
    let rows = store.list("mounts/w").expect("list");
    let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(names, vec!["a.txt", "b.txt", "c.txt", "d.txt", "dir"]);
    assert_eq!(rows[3].path, "mounts/w/d.txt");
    assert_eq!(rows[3].bytes, 2);
    assert_eq!(rows[4].kind, MountKind::Dir);
    assert_eq!(rows[4].bytes, 0, "目录的 bytes 恒为 0（无递归、无树扫描）");

    // 条目帽：16385 个条目 > 16384 → 帽拒（无分页）。
    let big = temp_dir("list-cap");
    let big_run = big.join("run");
    let big_mod = big.join("mod");
    fs::create_dir_all(&big_mod).expect("mod dir");
    let big_store = attach(&big_run, &big_mod, json!([{ "name": "w" }]));
    let big_root = big_run.join("mounts").join("w");
    for index in 0..=MOUNT_LIST_MAX_ENTRIES {
        fs::write(big_root.join(format!("f{index:05}.txt")), b"").expect("write entry");
    }
    match big_store.list("mounts/w") {
        Ok(rows) => panic!("{} 条目必须超过上限", rows.len()),
        Err(err) => {
            assert_eq!(err.code(), codes::CAP, "{err}");
            assert!(err.to_string().contains("16384"), "{err}");
        }
    }
}
