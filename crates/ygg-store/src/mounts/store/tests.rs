use super::super::{codes, MOUNT_DIR_MAX_BYTES};
use super::*;
use serde_json::json;
use serde_json::Value as Json;
use std::fs;
use std::path::{Path, PathBuf};

/// 本用例独占的临时目录（`std::env::temp_dir()` 合成数据，不碰任何业务目录）。
fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ygg-mounts-unit-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// 只有 `mounts` 段的 pack（声明面的最小输入）。
fn pack(mounts: Json) -> Json {
    json!({ "schemaVersion": 1, "mounts": mounts })
}

/// 解析失败时取码 + 原因（码必须在前，调用方按首段分派）。
fn refused(pack: &Json, mod_dir: &Path) -> (String, String) {
    let err = parse_mounts(pack, mod_dir).expect_err("必须拒绍");
    assert_eq!(err.code(), codes::MOUNT, "声明面的拒绍一律 YGG-FS-MOUNT");
    let text = err.to_string();
    let prefix = format!("{}: ", codes::MOUNT);
    assert!(text.starts_with(&prefix), "码必须在前：{text}");
    (err.code().to_owned(), text[prefix.len()..].to_owned())
}

#[test]
fn absent_or_empty_mounts_means_zero_mounts() {
    let dir = temp_dir("absent");
    // 段缺失：旧 mod 100% 兼容。
    let empty = parse_mounts(&json!({ "schemaVersion": 1 }), &dir).expect("段缺失合法");
    assert!(empty.is_empty());
    // 段为空数组：同一语义（显式声明「零挂载」）。
    let none = parse_mounts(&pack(json!([])), &dir).expect("空数组合法");
    assert!(none.is_empty());
}

#[test]
fn non_array_segment_is_refused() {
    let dir = temp_dir("nonarray");
    for bad in [json!(null), json!("x"), json!(1), json!({ "name": "w" })] {
        let (_, detail) = refused(&pack(bad.clone()), &dir);
        assert!(detail.contains("必须是数组"), "{bad}: {detail}");
    }
}

#[test]
fn non_object_element_is_refused() {
    let dir = temp_dir("nonobject");
    let (_, detail) = refused(&pack(json!(["work"])), &dir);
    assert!(detail.contains("mounts[0] 必须是对象"), "{detail}");
}

#[test]
fn name_vectors() {
    let dir = temp_dir("names");
    for good in [
        "a",
        "work",
        "out-2",
        "a_b",
        "z9",
        "abcdefghijklmnopqrstuvwxyz012345",
    ] {
        let specs = parse_mounts(&pack(json!([{ "name": good }])), &dir)
            .unwrap_or_else(|err| panic!("`{good}` 必须合法：{err}"));
        assert_eq!(specs[0].name, good);
    }
    for (bad, hint) in [
        ("", "不得为空"),
        ("Work", "首字符必须是 ASCII 小写字母"),
        ("9x", "首字符必须是 ASCII 小写字母"),
        ("a b", "只允许小写字母"),
        ("a/b", "只允许小写字母"),
        ("_x", "首字符必须是 ASCII 小写字母"),
        ("abcdefghijklmnopqrstuvwxyz0123456", "超过 32 字节"),
    ] {
        let (_, detail) = refused(&pack(json!([{ "name": bad }])), &dir);
        assert!(detail.contains(hint), "`{bad}` 的拒绍理由：{detail}");
    }
    // 缺 name（非字符串也算）：必填字段缺失。
    let (_, detail) = refused(&pack(json!([{ "dir": "/tmp" }])), &dir);
    assert!(detail.contains("缺省必填字段 name"), "{detail}");
}

#[test]
fn duplicate_and_reserved_names_are_refused() {
    let dir = temp_dir("dup");
    let (_, detail) = refused(&pack(json!([{ "name": "work" }, { "name": "work" }])), &dir);
    assert!(detail.contains("重复"), "{detail}");
    let (_, detail) = refused(&pack(json!([{ "name": "artifacts" }])), &dir);
    assert!(detail.contains("保留名"), "{detail}");
}

#[test]
fn dir_must_be_an_existing_directory() {
    let dir = temp_dir("dir");
    // 不存在。
    let (_, detail) = refused(&pack(json!([{ "name": "w", "dir": "missing" }])), &dir);
    assert!(detail.contains("无法 canonicalize"), "{detail}");
    // 存在但是文件。
    let file = dir.join("a.txt");
    fs::write(&file, b"x").expect("write file");
    let (_, detail) = refused(
        &pack(json!([{ "name": "w", "dir": file.display().to_string() }])),
        &dir,
    );
    assert!(detail.contains("不是目录"), "{detail}");
}

#[test]
fn dir_resolves_relative_to_the_mod_dir() {
    let mod_dir = temp_dir("reldir");
    let host = mod_dir.join("biz");
    fs::create_dir_all(&host).expect("create host dir");
    let specs = parse_mounts(&pack(json!([{ "name": "w", "dir": "biz" }])), &mod_dir)
        .expect("相对路径合法");
    assert_eq!(
        specs[0].dir.as_deref(),
        Some(host.canonicalize().expect("canonical").as_path()),
        "相对路径对 <mod dir> 解析并 canonicalize"
    );
    // 绝对路径原样（仍然 canonicalize：符号链接 / `..` 都被折叠）。
    let specs = parse_mounts(
        &pack(json!([{ "name": "w", "dir": host.display().to_string() }])),
        &mod_dir,
    )
    .expect("绝对路径合法");
    assert_eq!(
        specs[0].dir.as_deref(),
        Some(host.canonicalize().expect("canonical").as_path())
    );
}

#[test]
fn dir_shape_limits_are_enforced() {
    let dir = temp_dir("dirlimits");
    let (_, detail) = refused(&pack(json!([{ "name": "w", "dir": "" }])), &dir);
    assert!(detail.contains("不得为空串"), "{detail}");
    let long = "x".repeat(MOUNT_DIR_MAX_BYTES + 1);
    let (_, detail) = refused(&pack(json!([{ "name": "w", "dir": long }])), &dir);
    assert!(detail.contains("超过 4096 字节"), "{detail}");
    let (_, detail) = refused(&pack(json!([{ "name": "w", "dir": "a\u{0}b" }])), &dir);
    assert!(detail.contains("不得含 NUL"), "{detail}");
    // 非字符串的 dir：类型不对也是声明错误。
    let (_, detail) = refused(&pack(json!([{ "name": "w", "dir": 3 }])), &dir);
    assert!(detail.contains("dir 必须是字符串"), "{detail}");
}

#[test]
fn mode_defaults_to_ro_and_rejects_unknown_values() {
    let dir = temp_dir("mode");
    let specs = parse_mounts(&pack(json!([{ "name": "w" }])), &dir).expect("缺省合法");
    assert_eq!(specs[0].mode, MountMode::Ro, "缺省是 ro（唯一减压阀）");
    let specs = parse_mounts(&pack(json!([{ "name": "w", "mode": "rw" }])), &dir).expect("rw 合法");
    assert_eq!(specs[0].mode, MountMode::Rw);
    let (_, detail) = refused(&pack(json!([{ "name": "w", "mode": "readonly" }])), &dir);
    assert!(detail.contains("只能是 `ro` / `rw`"), "{detail}");
    let (_, detail) = refused(&pack(json!([{ "name": "w", "mode": 1 }])), &dir);
    assert!(detail.contains("mode 必须是字符串"), "{detail}");
}

#[test]
fn unknown_keys_are_ignored() {
    // 键集宽松：旧宿主忽略未知顶层段，新宿主同样忽略未知元素键。
    let dir = temp_dir("unknown");
    let specs = parse_mounts(
        &json!({
            "schemaVersion": 1,
            "mounts": [{ "name": "w", "futureField": { "nested": true } }],
            "panel": { "futureKey": 1 },
        }),
        &dir,
    )
    .expect("未知键忽略");
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].dir, None, "无 dir = 私有挂载");
}

/// attach 的两个入参：一个空 run 目录 + 一个空 mod 目录。
fn attach_env(tag: &str) -> (PathBuf, PathBuf) {
    let root = temp_dir(tag);
    let run = root.join("run");
    fs::create_dir_all(&run).expect("create run dir");
    let mod_dir = root.join("mod");
    fs::create_dir_all(&mod_dir).expect("create mod dir");
    (run, mod_dir)
}

/// `MountStore::attach` 的测试便捷面（pack 只含 `mounts` 段）。
fn attach(run: &Path, mod_dir: &Path, mounts: Json) -> Result<MountStore, MountError> {
    MountStore::attach(run, &pack(mounts), mod_dir)
}

#[test]
fn jail_rejects_everything_outside_the_two_roots() {
    let (run, mod_dir) = attach_env("jail-basic");
    let store = attach(&run, &mod_dir, json!([{ "name": "w" }])).expect("attach");
    for bad in [
        "artifacts/x.md",
        "mounts",
        "mounts/",
        "mounts/other/x",
        "mounts/w/",
        "mounts/w//x",
        "mounts/w/./x",
        "mounts/w/../x",
        "mounts/w/a\u{0}b",
        "/mounts/w/x",
    ] {
        match store.locate(bad, Need::File) {
            Ok(_) => panic!("`{bad}` 必须拒"),
            Err(err) => assert_eq!(err.code(), codes::JAIL, "`{bad}` → {err}"),
        }
    }
    // File 面拒挂载根本身；Dir 面接受它。
    assert!(store.locate("mounts/w", Need::File).is_err());
    let root = store.locate("mounts/w", Need::Dir).expect("挂载根是目录");
    assert_eq!(root.mount, "w");
    assert_eq!(
        root.target,
        run.join(MOUNTS_DIR)
            .join("w")
            .canonicalize()
            .expect("canonical")
    );
    // 父目录不存在 → 拒（r1 不做挂载点内的 mkdir）。
    assert!(store.locate("mounts/w/nope/x.txt", Need::File).is_err());
    // 超出段数 / 字节上限 → 拒。
    let deep = format!(
        "mounts/w/{}",
        vec!["d"; MOUNT_PATH_MAX_SEGMENTS + 1].join("/")
    );
    assert!(store.locate(&deep, Need::File).is_err());
    let long = format!("mounts/w/{}", "x".repeat(MOUNT_PATH_MAX_BYTES));
    assert!(store.locate(&long, Need::File).is_err());
}

#[test]
fn attach_private_mount_creates_a_real_dir() {
    let (run, mod_dir) = attach_env("attach-private");
    let store = attach(&run, &mod_dir, json!([{ "name": "out" }])).expect("attach");
    let slot = run.join(MOUNTS_DIR).join("out");
    let meta = fs::symlink_metadata(&slot).expect("挂载点必须存在（声明面 = 目录面）");
    assert!(
        meta.is_dir() && !meta.file_type().is_symlink(),
        "私有挂载是真目录"
    );
    let declared: Vec<_> = store.declared().collect();
    assert_eq!(declared.len(), 1);
    assert_eq!(declared[0].0, "out");
    assert_eq!(
        declared[0].1,
        slot.canonicalize().expect("canonical").as_path()
    );
    assert_eq!(declared[0].2, MountMode::Ro, "缺省是 ro");
    assert_eq!(store.budget_left(), MOUNT_RUN_WRITE_BUDGET);
    assert_eq!(store.run_dir(), run.as_path());
    assert_eq!(store.mounts_root(), run.join(MOUNTS_DIR).as_path());
    assert_eq!(store.audit_path(), run.join(FS_AUDIT_FILE));
}

#[cfg(unix)]
#[test]
fn attach_dir_mount_binds_the_host_dir_by_symlink() {
    let (run, mod_dir) = attach_env("attach-link");
    let host = mod_dir.join("biz");
    fs::create_dir_all(&host).expect("create host dir");
    fs::write(host.join("a.txt"), b"host").expect("write host file");
    let store = attach(
        &run,
        &mod_dir,
        json!([{ "name": "work", "dir": "biz", "mode": "rw" }]),
    )
    .expect("attach");
    let slot = run.join(MOUNTS_DIR).join("work");
    assert!(
        fs::symlink_metadata(&slot)
            .expect("slot")
            .file_type()
            .is_symlink(),
        "目录挂载是符号链接（bind 语义）"
    );
    assert_eq!(
        fs::read_link(&slot).expect("read_link"),
        host.canonicalize().expect("canonical")
    );
    // 不拷贝内容：挂载点里读到的就是宿主目录那一份。
    assert_eq!(fs::read(slot.join("a.txt")).expect("read through"), b"host");
    assert_eq!(
        store.declared().next().expect("一个挂载点").2,
        MountMode::Rw
    );
}

#[cfg(unix)]
#[test]
fn re_attach_is_silent_and_writes_nothing() {
    use std::os::unix::fs::MetadataExt;
    let (run, mod_dir) = attach_env("attach-idem");
    let host = mod_dir.join("biz");
    fs::create_dir_all(&host).expect("create host dir");
    let spec = json!([{ "name": "work", "dir": "biz" }, { "name": "out" }]);
    attach(&run, &mod_dir, spec.clone()).expect("first attach");
    let slot = run.join(MOUNTS_DIR).join("work");
    let private = run.join(MOUNTS_DIR).join("out");
    let link_ino = fs::symlink_metadata(&slot).expect("slot").ino();
    let dir_ino = fs::symlink_metadata(&private).expect("out").ino();
    fs::write(private.join("keep.txt"), b"k").expect("write into private mount");
    attach(&run, &mod_dir, spec).expect("re-attach 必须静默通过");
    assert_eq!(
        fs::symlink_metadata(&slot).expect("slot").ino(),
        link_ino,
        "重载不得重建符号链接（零写入）"
    );
    assert_eq!(
        fs::symlink_metadata(&private).expect("out").ino(),
        dir_ino,
        "重载不得重建私有挂载目录（零写入）"
    );
    assert!(
        private.join("keep.txt").exists(),
        "已存在的挂载内容不得被清空"
    );
}

#[cfg(unix)]
#[test]
fn attach_conflicts_refuse_with_zero_writes() {
    let (run, mod_dir) = attach_env("attach-conflict");
    let host = mod_dir.join("biz");
    fs::create_dir_all(&host).expect("create host dir");
    // 槽位已是真目录，声明却是目录挂载 → 冲突；同一 pack 里的另一条
    // 合法声明也不得被建出来（先全量算现状、再落地）。
    fs::create_dir_all(run.join(MOUNTS_DIR).join("work")).expect("pre-create slot");
    match attach(
        &run,
        &mod_dir,
        json!([{ "name": "okmount" }, { "name": "work", "dir": "biz" }]),
    ) {
        Ok(_) => panic!("真目录 + dir 声明必须拒"),
        Err(err) => assert_eq!(err.code(), codes::MOUNT, "{err}"),
    }
    assert!(
        !run.join(MOUNTS_DIR).join("okmount").exists(),
        "冲突时其余挂载点也不得被创建（零写入）"
    );
    // 槽位是软链，声明却是私有挂载 → 冲突。
    let other = attach_env("attach-conflict-link");
    fs::create_dir_all(other.0.join(MOUNTS_DIR)).expect("mounts root");
    std::os::unix::fs::symlink(other.1.join("biz"), other.0.join(MOUNTS_DIR).join("work"))
        .expect("pre-create link");
    match attach(&other.0, &other.1, json!([{ "name": "work" }])) {
        Ok(_) => panic!("软链 + 私有声明必须拒"),
        Err(err) => assert_eq!(err.code(), codes::MOUNT, "{err}"),
    }
    // 已存在的软链指向别处 → 冲突（不一致）。
    let third = attach_env("attach-conflict-elsewhere");
    fs::create_dir_all(third.0.join(MOUNTS_DIR)).expect("mounts root");
    fs::create_dir_all(third.1.join("biz")).expect("declared host dir");
    fs::create_dir_all(third.1.join("biz2")).expect("other host dir");
    std::os::unix::fs::symlink(third.1.join("biz2"), third.0.join(MOUNTS_DIR).join("work"))
        .expect("pre-create link");
    match attach(
        &third.0,
        &third.1,
        json!([{ "name": "work", "dir": "biz" }]),
    ) {
        Ok(_) => panic!("指向别处的软链必须拒"),
        Err(err) => {
            assert_eq!(err.code(), codes::MOUNT, "{err}");
            assert!(err.to_string().contains("不一致"), "{err}");
        }
    }
}

#[test]
fn read_hits_misses_and_caps() {
    let (run, mod_dir) = attach_env("read");
    let store = attach(&run, &mod_dir, json!([{ "name": "w" }])).expect("attach");
    let root = run.join(MOUNTS_DIR).join("w");
    fs::write(root.join("a.txt"), b"hello").expect("write fixture");
    assert_eq!(
        store.read("mounts/w/a.txt").expect("read").as_deref(),
        Some(b"hello".as_slice())
    );
    // miss = None（与 artifacts/ 面同口径），不是错误。
    assert_eq!(store.read("mounts/w/none.txt").expect("miss"), None);
    // 目标是目录 → 狱拒（路径形状不对）。
    match store.read("mounts/w") {
        Ok(_) => panic!("挂载根本身不能 read"),
        Err(err) => assert_eq!(err.code(), codes::JAIL, "{err}"),
    }
    match store.read("mounts/w/") {
        Ok(_) => panic!("结尾 `/` 必须拒"),
        Err(err) => assert_eq!(err.code(), codes::JAIL, "{err}"),
    }
    // 超过单件上限 → 帽拒（拒绝而不截断）。
    let big = vec![b'x'; MOUNT_FILE_MAX_BYTES + 1];
    fs::write(root.join("big.bin"), &big).expect("write big");
    match store.read("mounts/w/big.bin") {
        Ok(_) => panic!("超过 4 MiB 必须拒"),
        Err(err) => assert_eq!(err.code(), codes::CAP, "{err}"),
    }
    // 恰好等于上限：收（边界是闭区间）。
    fs::write(root.join("edge.bin"), vec![b'y'; MOUNT_FILE_MAX_BYTES]).expect("write edge");
    assert_eq!(
        store
            .read("mounts/w/edge.bin")
            .expect("边缘合法")
            .expect("必须命中")
            .len(),
        MOUNT_FILE_MAX_BYTES
    );
}

#[test]
fn stat_reports_kind_bytes_and_sha() {
    let (run, mod_dir) = attach_env("stat");
    let store = attach(&run, &mod_dir, json!([{ "name": "w" }])).expect("attach");
    let root = run.join(MOUNTS_DIR).join("w");
    fs::write(root.join("a.txt"), b"hello").expect("write fixture");
    fs::create_dir_all(root.join("sub")).expect("mkdir");
    let file = store.stat("mounts/w/a.txt").expect("stat file");
    assert!(file.exists);
    assert_eq!(file.bytes, 5);
    assert_eq!(file.sha256, crate::sha256_hex(b"hello"));
    assert_eq!(file.kind, MountKind::File);
    assert_eq!(file.kind.as_str(), "file");
    let dir = store.stat("mounts/w/sub").expect("stat dir");
    assert!(dir.exists);
    assert_eq!((dir.bytes, dir.sha256.as_str()), (0, ""));
    assert_eq!(dir.kind, MountKind::Dir);
    assert_eq!(dir.kind.as_str(), "dir");
    // 挂载根本身是目录（stat 接受空 rel）。
    assert_eq!(
        store.stat("mounts/w").expect("stat root").kind,
        MountKind::Dir
    );
    let miss = store.stat("mounts/w/none.txt").expect("stat miss");
    assert!(!miss.exists);
    assert_eq!(
        (miss.bytes, miss.sha256.as_str(), miss.kind.as_str()),
        (0, "", "")
    );
}

#[cfg(unix)]
#[test]
fn list_is_sorted_and_skips_symlinks() {
    let (run, mod_dir) = attach_env("list");
    let store = attach(&run, &mod_dir, json!([{ "name": "w" }])).expect("attach");
    let root = run.join(MOUNTS_DIR).join("w");
    fs::write(root.join("b.txt"), b"22").expect("write b");
    fs::write(root.join("a.txt"), b"1").expect("write a");
    fs::create_dir_all(root.join("sub")).expect("mkdir sub");
    fs::create_dir_all(mod_dir.join("outside")).expect("mkdir outside");
    std::os::unix::fs::symlink(mod_dir.join("outside"), root.join("escape")).expect("symlink");
    let rows = store.list("mounts/w").expect("list");
    let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["a.txt", "b.txt", "sub"],
        "按名字字节序升序，不递归，跳过软链"
    );
    assert_eq!(rows[0].path, "mounts/w/a.txt");
    assert_eq!((rows[2].bytes, rows[2].kind), (0, MountKind::Dir));
    assert_eq!(rows[1].kind, MountKind::File);
    assert_eq!(rows[1].bytes, 2);
    // 子目录：路径按请求路径拼
    let sub = store.list("mounts/w/sub").expect("list sub");
    assert!(sub.is_empty());
    // 列一个不存在 / 是文件的路径 → 狱拒
    for bad in ["mounts/w/nope", "mounts/w/a.txt"] {
        match store.list(bad) {
            Ok(_) => panic!("`{bad}` 必须拒"),
            Err(err) => assert_eq!(err.code(), codes::JAIL, "{err}"),
        }
    }
}

/// 读审计文件的各行（测试面：直接读 JSON，不绕过实现）。
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
fn write_appends_audit_rows_for_success_and_rejection() {
    let (run, mod_dir) = attach_env("write-audit");
    let mut store = attach(&run, &mod_dir, json!([{ "name": "w", "mode": "rw" }])).expect("attach");
    let written = store.write("mounts/w/a.txt", b"hello").expect("write");
    assert_eq!(written.path, "mounts/w/a.txt");
    assert_eq!(written.bytes, 5);
    assert_eq!(written.sha256, crate::sha256_hex(b"hello"));
    store.write("mounts/w/b.txt", b"22").expect("write 2");
    // 被拒也登记。
    match store.write("mounts/w/../x", b"x") {
        Ok(_) => panic!("`..` 必须拒"),
        Err(err) => assert_eq!(err.code(), codes::JAIL, "{err}"),
    }
    // 写穿磁盘：文件就在挂载目录里。
    assert_eq!(
        fs::read(run.join(MOUNTS_DIR).join("w").join("a.txt")).expect("read back"),
        b"hello"
    );
    let rows = audit_rows(&store);
    assert_eq!(rows.len(), 3, "2 成功 + 1 被拒：{rows:?}");
    assert_eq!(rows[0]["seq"], json!(1));
    assert_eq!(rows[0]["event"], json!("fs.write"));
    assert_eq!(rows[0]["mount"], json!("w"));
    assert_eq!(rows[0]["path"], json!("mounts/w/a.txt"));
    assert_eq!(rows[0]["bytes"], json!(5));
    assert_eq!(rows[0]["sha256"], json!(crate::sha256_hex(b"hello")));
    assert_eq!(rows[1]["seq"], json!(2));
    assert_eq!(rows[2]["seq"], json!(3));
    assert_eq!(rows[2]["event"], json!("fs.write.rejected"));
    assert_eq!(rows[2]["reason"], json!(codes::JAIL));
    assert_eq!(rows[2]["bytes"], json!(0));
    assert_eq!(rows[2]["sha256"], json!(""));
}

#[test]
fn write_refuses_read_only_mounts() {
    let (run, mod_dir) = attach_env("write-ro");
    let mut store = attach(&run, &mod_dir, json!([{ "name": "w" }])).expect("attach");
    match store.write("mounts/w/a.txt", b"x") {
        Ok(_) => panic!("ro 缺省下写必须拒"),
        Err(err) => assert_eq!(err.code(), codes::MOUNT, "{err}"),
    }
    assert!(
        !run.join(MOUNTS_DIR).join("w").join("a.txt").exists(),
        "只读拒写：磁盘零写入"
    );
    let rows = audit_rows(&store);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["event"], json!("fs.write.rejected"));
    assert_eq!(rows[0]["reason"], json!(codes::MOUNT));
}

#[test]
fn write_caps_single_file_and_per_attach_budget() {
    let (run, mod_dir) = attach_env("write-budget");
    let mut store = attach(&run, &mod_dir, json!([{ "name": "w", "mode": "rw" }])).expect("attach");
    let oversize = vec![b'x'; MOUNT_FILE_MAX_BYTES + 1];
    match store.write("mounts/w/big.bin", &oversize) {
        Ok(_) => panic!("超单件帽必须拒"),
        Err(err) => assert_eq!(err.code(), codes::CAP, "{err}"),
    }
    // 预算 = 16 次满帽写；用完后再写 1 字节 → 帽拒。
    let full = vec![b'y'; MOUNT_FILE_MAX_BYTES];
    for index in 0..16 {
        store
            .write(&format!("mounts/w/f{index}.bin"), &full)
            .unwrap_or_else(|err| panic!("第 {index} 次满帽写必须成功：{err}"));
    }
    assert_eq!(store.budget_left(), 0);
    match store.write("mounts/w/one.txt", b"z") {
        Ok(_) => panic!("预算耗尽后必须拒"),
        Err(err) => assert_eq!(err.code(), codes::CAP, "{err}"),
    }
    // 续局拿到新预算（预算不落盘）。
    let mut again =
        attach(&run, &mod_dir, json!([{ "name": "w", "mode": "rw" }])).expect("re-attach");
    assert_eq!(again.budget_left(), MOUNT_RUN_WRITE_BUDGET);
    again.write("mounts/w/one.txt", b"z").expect("新预算可写");
}

#[test]
fn error_display_puts_the_code_first() {
    // 码在前是调用方契约（Lua 侧与 CLI 回执按首段分派）。
    assert_eq!(
        MountError::jail("x").to_string(),
        "YGG-FS-JAIL: x".to_owned()
    );
    assert_eq!(MountError::cap("x").code(), codes::CAP);
    assert_eq!(MountError::mount("x").code(), codes::MOUNT);
    assert_eq!(MountError::audit("x").code(), codes::AUDIT);
}
