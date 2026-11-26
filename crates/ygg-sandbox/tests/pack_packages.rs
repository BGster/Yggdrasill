//! r3 packages 装载面证据（S7）：真沙箱 require、拒绝矩阵、纯度、预算。
//!
//! 测试全部在 `std::env::temp_dir()` 下自造合成包树（复制入库夹具后再改写），
//! 不触碰任何真实数据。
//!
//! # 布局与**缺省推导**这条路
//!
//! 临时根排成 `<R>/packages/libs/<id>/` + `<R>/mods/<mod>/`，于是
//! `<R>/mods/<mod>/../../packages/libs` 正好是 `<R>/packages/libs`——本文件跑的
//! 是 `ModBundle::load` **缺省推导**那条真实路径，不是给测试开后门。`--packages`
//! 与 `$YGG_PACKAGES_ROOT` 两条来源由 `examples/selftest` 的验收覆盖。

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde_json::{json, Value as Json};
use ygg_sandbox::modload::{Loaded, ModBundle};
use ygg_sandbox::modules::ModuleSet;
use ygg_sandbox::packages::{self, PackagesSpec};

use support::{read_json, write_json};

/// 临时布局（见模块文档）。
struct Scenario {
    root: PathBuf,
}

impl Scenario {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("ygg-r3-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("packages/libs")).expect("mkdir package root");
        fs::create_dir_all(root.join("mods")).expect("mkdir mods");
        support::copy_dir(
            &support::fixture_package_dir(),
            &root.join("packages/libs/fixture"),
        );
        support::copy_dir(
            &support::packages_mod_dir(),
            &root.join("mods/packages-mod"),
        );
        Self { root }
    }

    fn libs(&self) -> PathBuf {
        self.root.join("packages/libs")
    }

    fn mod_dir(&self) -> PathBuf {
        self.root.join("mods/packages-mod")
    }

    fn pack_path(&self) -> PathBuf {
        self.mod_dir().join("pack.json")
    }

    fn manifest_path(&self) -> PathBuf {
        self.mod_dir().join("manifest.json")
    }

    /// 改写 `pack.json` 的 `packages` 段并尝试装载，返回拒绝文案。
    #[track_caller]
    fn reject_with(&self, packages: Json) -> String {
        let mut pack = read_json(&self.pack_path());
        pack["packages"] = packages;
        write_json(&self.pack_path(), &pack);
        match ModBundle::load(&self.mod_dir()) {
            Ok(_) => panic!("expected the packages section to be rejected"),
            Err(message) => message,
        }
    }
}

/// 在包根下造一个最小包（`pkg.json` + `init.lua`）。
fn write_pkg(libs: &Path, id: &str, version: &str, deps: &[(&str, &str)]) {
    let dir = libs.join(id);
    fs::create_dir_all(&dir).expect("mkdir package");
    let deps_json: Vec<String> = deps
        .iter()
        .map(|(dep_id, dep_version)| format!(r#"{{"id": "{dep_id}", "version": "{dep_version}"}}"#))
        .collect();
    fs::write(
        dir.join("pkg.json"),
        format!(
            r#"{{"schemaVersion": 1, "name": "{id}", "version": "{version}",
                 "deps": [{}], "contract": "openwiki/contracts/kernel/packaging.md"}}"#,
            deps_json.join(", ")
        ),
    )
    .expect("write pkg.json");
    fs::write(dir.join("init.lua"), "return {}\n").expect("write init.lua");
}

// ───────────────────── E5：真沙箱里 require 包并调用导出 ─────────────────────

#[test]
fn require_pkg_succeeds_and_calls_the_export_in_the_real_sandbox() {
    let scenario = Scenario::new("e5");
    let bundle = ModBundle::load(&scenario.mod_dir()).expect("夹具必须可装载");
    // 包模块与 mod 自有模块进的是**同一张**表（只有前缀不同）。
    assert!(
        bundle.modules.get("pkg.fixture").is_some(),
        "包根模块未登记"
    );
    assert!(
        bundle.modules.get("pkg.fixture.words").is_some(),
        "包子模块未登记"
    );
    assert!(
        bundle.modules.get("src.probe").is_some(),
        "mod 自有模块未登记"
    );
    // chunk 覆盖表用的是**相对包根的真实路径**（不是 path_of 推出的 pkg/fixture.lua）。
    assert_eq!(bundle.chunk_paths["pkg.fixture"], "fixture/init.lua");
    assert_eq!(bundle.chunk_paths["pkg.fixture.words"], "fixture/words.lua");

    let kernel = Loaded::from_bundle(&bundle).expect("装载");
    let lines = support::narrate(&kernel, "probe.pkg");
    assert!(
        lines.iter().any(|line| line.contains("hello, sandbox")),
        "handler 里 require 包并调用导出没有产生叙事行：{lines:?}"
    );
    // 同一张表里 mod 自有模块也照旧可用。
    let lines = support::narrate(&kernel, "probe.mod");
    assert!(
        lines.iter().any(|line| line == "mod module ok"),
        "mod 自有模块被包面挤掉：{lines:?}"
    );
}

// ───────────────────── E9：8 条声明面拒绝（D9 表格逐条） ─────────────────────

#[test]
fn rejects_bad_packages_declarations() {
    // ① 缺 `version`（半声明一律拒）。
    let err = Scenario::new("r-missing-version").reject_with(json!([{ "id": "fixture" }]));
    assert!(
        err.contains("`packages[0]` must be an object with string `id` and `version`"),
        "{err}"
    );

    // ② 缺 `id`。
    let err = Scenario::new("r-missing-id").reject_with(json!([{ "version": "0.1.0" }]));
    assert!(err.contains("`packages[0]` must be an object"), "{err}");

    // ③ id 非法（点号是结构分隔符）。
    let err =
        Scenario::new("r-bad-id").reject_with(json!([{ "id": "role.skill", "version": "0.1.0" }]));
    assert!(err.contains("must match ^[a-z][a-z0-9_-]*$"), "{err}");

    // ④ 同一 id 声明两次。
    let err = Scenario::new("r-dup").reject_with(json!([
        { "id": "fixture", "version": "0.1.0" },
        { "id": "fixture", "version": "0.1.0" }
    ]));
    assert!(err.contains("declares \"fixture\" twice"), "{err}");
}

#[test]
fn rejects_unresolvable_package_declarations() {
    // ⑤ 包不存在：同时点名包根与实际有哪些（作者不用猜目录名）。
    let err =
        Scenario::new("r-unknown").reject_with(json!([{ "id": "quest", "version": "0.1.0" }]));
    assert!(
        err.contains("package \"quest\" is not a package under"),
        "{err}"
    );
    assert!(err.contains("(found: fixture)"), "{err}");

    // ⑥ 版本不符：报双方值（声明错，不是完整性错）。
    let err =
        Scenario::new("r-version").reject_with(json!([{ "id": "fixture", "version": "9.9.9" }]));
    assert!(err.contains("is required at version 9.9.9"), "{err}");
    assert!(err.contains("declares 0.1.0"), "{err}");

    // ⑦ 缺传递依赖：quest 依赖 codec，但 `packages` 段只列了 quest。
    let scenario = Scenario::new("r-closure");
    write_pkg(&scenario.libs(), "quest", "0.1.0", &[("codec", "0.1.0")]);
    write_pkg(&scenario.libs(), "codec", "0.1.0", &[]);
    let err = scenario.reject_with(json!([{ "id": "quest", "version": "0.1.0" }]));
    assert!(
        err.contains("depends on \"codec\" which the mod's `packages` section"),
        "{err}"
    );

    // ⑧ 循环：a → b → a。
    let scenario = Scenario::new("r-cycle");
    write_pkg(&scenario.libs(), "a", "0.1.0", &[("b", "0.1.0")]);
    write_pkg(&scenario.libs(), "b", "0.1.0", &[("a", "0.1.0")]);
    let err = scenario.reject_with(json!([
        { "id": "a", "version": "0.1.0" },
        { "id": "b", "version": "0.1.0" }
    ]));
    assert!(
        err.contains("package dependency cycle: a -> b -> a"),
        "{err}"
    );
}

// ───────────────────── manifest 纳管（内容寻址的第二段） ─────────────────────

#[test]
fn rejects_manifest_drift_against_the_declared_closure() {
    // ① manifest 没有 `packages` 段：忘了跑打包工具。
    let scenario = Scenario::new("drift-missing");
    let mut manifest = read_json(&scenario.manifest_path());
    manifest
        .as_object_mut()
        .expect("manifest object")
        .remove("packages");
    write_json(&scenario.manifest_path(), &manifest);
    let err = ModBundle::load(&scenario.mod_dir()).expect_err("缺 packages 段必须被拒");
    assert!(err.contains("has no `packages` array"), "{err}");

    // ② 包文件字节被改而 manifest 仍说旧哈希：内容寻址拒绝。
    let scenario = Scenario::new("drift-bytes");
    let words = scenario.libs().join("fixture/words.lua");
    let mut text = fs::read_to_string(&words).expect("read words.lua");
    text.push_str("\n-- tampered\n");
    fs::write(&words, text).expect("write words.lua");
    let err = ModBundle::load(&scenario.mod_dir()).expect_err("字节漂移必须被拒");
    assert!(err.contains("content rejected"), "{err}");
    assert!(err.contains("fixture/words.lua"), "{err}");
}

// ───────────────────── 纯度边界与 chunk 名（D3 的直接证据） ─────────────────────

#[test]
fn package_module_top_level_effects_are_refused_and_named_by_real_path() {
    let scenario = Scenario::new("impure");
    fs::write(
        scenario.libs().join("fixture/init.lua"),
        "ygg.kv.set(\"bad\", 1)\nreturn {}\n",
    )
    .expect("write impure init.lua");
    // 改包内容必须重算 manifest，否则会先撞内容寻址（那是另一条用例的被测面）。
    support::reseal_with_packages(&scenario.mod_dir(), &scenario.libs());
    // 纯度拒绝发生在**执行**面（入口 chunk 里的 require），不在读盘面：
    // `ModBundle::load` 只做 I/O 与对账，Lua 到 `Loaded::from_bundle` 才跑。
    let bundle = ModBundle::load(&scenario.mod_dir()).expect("装载（I/O 面）应通过");
    let err = match Loaded::from_bundle(&bundle) {
        Ok(_) => panic!("包模块顶层副作用必须被拒，但内核构造成功了"),
        Err(err) => err.to_string(),
    };
    assert!(
        err.contains("模块顶层不能调用 `ygg.kv.set`"),
        "纯度边界没生效：{err}"
    );
    // 同一条错误也是 chunk 覆盖表的证据：路径是**相对包根的真实文件**，
    // 而不是 `path_of("pkg.fixture")` 会推出的 `pkg/fixture.lua`。
    assert!(
        err.contains("fixture/init.lua:1"),
        "chunk 名没有指向真实文件（D3 未生效）：{err}"
    );
}

// ───────────────────── E12：包加载预算（Goal 原则二 <10ms） ─────────────────────

#[test]
fn pkg_load_budget_stays_under_ten_milliseconds() {
    // 合成树：6 包 × 3 文件（`init.lua` + 2 子模块），与 P2 的六包规模同量级。
    let scenario = Scenario::new("budget");
    let libs = scenario.libs();
    let ids = [
        "codec",
        "dialogue",
        "entity",
        "procgen",
        "quest",
        "role-skill",
    ];
    for id in ids {
        let dir = libs.join(id);
        fs::create_dir_all(&dir).expect("mkdir package");
        fs::write(
            dir.join("pkg.json"),
            format!(
                r#"{{"schemaVersion": 1, "name": "{id}", "version": "0.1.0", "deps": [],
                     "contract": "openwiki/contracts/kernel/packaging.md"}}"#
            ),
        )
        .expect("write pkg.json");
        for file in ["init.lua", "a.lua", "b.lua"] {
            fs::write(
                dir.join(file),
                format!("return {{ name = \"{id}/{file}\" }}\n"),
            )
            .expect("write module");
        }
    }
    let declared: Vec<Json> = ids
        .iter()
        .map(|id| json!({ "id": id, "version": "0.1.0" }))
        .collect();
    let spec = PackagesSpec::parse(&json!({ "packages": declared }))
        .expect("parse")
        .expect("section present");
    // manifest 侧（内容寻址的期望值）在计时区**之外**构造：被测的是装载器自身。
    let entries = support::manifest_packages_for(&libs, &spec);
    let manifest_packages: Vec<ygg_schema::manifest::ManifestPackage> = entries
        .iter()
        .map(|entry| serde_json::from_value(entry.clone()).expect("ManifestPackage 反序列化"))
        .collect();

    let mut samples: Vec<f64> = Vec::new();
    for _ in 0..3 {
        let start = Instant::now();
        // 口径 = ②′ 闭包解析 + ④′ 三方对账 + ⑦⑧ 并集（不含 mod 自身 code 与资源读盘）。
        let resolved = packages::resolve(&libs, &spec).expect("resolve");
        let index = packages::index(&libs, &resolved, &manifest_packages).expect("index");
        let table = packages::union(&ModuleSet::new(), index).expect("union");
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(table.set().len(), ids.len() * 3, "并集模块数");
    }
    samples.sort_by(f64::total_cmp);
    let median = samples[1];
    println!("pkg_load_ms={median:.3} (n=3, 6 packages x 3 files)");
    // 阈值是 10ms 的**目标口径**（Goal 原则二），不是「永远为真」的形式断言：
    // 实测在几十 µs 量级，两个数量级余量；真超了就是真的回归。
    assert!(median < 10.0, "包加载超预算：{median:.3}ms");
}
