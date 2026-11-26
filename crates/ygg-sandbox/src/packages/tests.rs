use serde_json::json;
use serde_json::Value as Json;
use std::fs;
use std::path::{Path, PathBuf};

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
                 "contract": "openwiki/contracts/kernel/packaging.md"{extra}}}"#
    )
}

#[test]
fn package_manifest_parses_the_documented_shape_and_defaults_deps_to_empty() {
    let file = Path::new("packages/libs/fixture/pkg.json");
    let bare =
        PackageManifest::parse("fixture", pkg_json("").as_bytes(), file).expect("minimal pkg.json");
    assert_eq!(bare.id(), "fixture");
    assert_eq!(bare.version(), "0.1.0");
    assert!(bare.deps().is_empty());
    assert_eq!(bare.contract(), "openwiki/contracts/kernel/packaging.md");

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
                       "contract": "openwiki/contracts/kernel/packaging.md"}"#;
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
                "contract": "openwiki/contracts/kernel/packaging.md"}"#,
    );
    assert!(
        err.contains("declares schemaVersion 2 (this build implements 1)"),
        "{err}"
    );
    // 缺 version：半声明一律拒绕（与 packages[] 同一条纪律）。
    let err = pkg_err(
        "fixture",
        r#"{"schemaVersion": 1, "name": "fixture",
                "contract": "openwiki/contracts/kernel/packaging.md"}"#,
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
fn write_pkg(root: &Path, id: &str, version: &str, deps: &[(&str, &str)], files: &[(&str, &str)]) {
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
                 "deps": [{}], "contract": "openwiki/contracts/kernel/packaging.md" }}"#,
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
    let resolved = resolve(&root, &spec_of(&[("fixture", "0.1.0")])).expect("dotfiles are skipped");
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
