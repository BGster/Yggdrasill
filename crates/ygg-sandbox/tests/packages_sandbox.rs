//! r2 · 包在**真沙箱**里的可装载面（三层证据，三个 `#[test]`）。
//!
//! # 为什么需要这一层
//!
//! `packages/libs/*/test/*.lua` 跑的是**普通 Lua 解释器**（`examples/selftest.rs`
//! 头部自己这么写），它自带 mock `ygg`，于是「普通 Lua 能跑」与「包能在核心里跑」
//! 是两件事。三层缺一即有死角：
//!
//! | 层 | 走哪条路 | 覆盖什么 | 缺了会怎样 |
//! | --- | --- | --- | --- |
//! | ① 生产真值 | `ModBundle::load` → `build_kernel` | 真 host API、inject 的 math、单张模块表 | 会随 inject 一起漂：inject 有 math 时冒烟永远绿 |
//! | ② 无注入面 | 裸 `Sandbox::new()` + 同一张模块表 | 包的**纯导出**是否真的不依赖驱动注入 | 没人逼包在无 inject 的沙箱里跑得动 |
//! | ③ 静态面 | 纯源码扫描 + 裸沙箱名字自检 | 新增一个非 base 全局的瞬间 | 注入面被裁掉时只能在运行期炸 |
//!
//! # 布局：**缺省推导**那条真实路径
//!
//! 仓库布局下 `packages/mods/pkg-selftest/../../packages/libs` 是
//! `packages/packages/libs`（不存在），所以仓库里的夹具只能靠 `--packages` 显式
//! 指路，而 `ModBundle::load` 没有显式包根入参。照 `pack_packages.rs` 的 E5 先例，
//! 临时根排成 `<R>/packages/libs/<id>/` + `<R>/mods/pkg-selftest/`——于是
//! `resolve_root` 走的是**真实推导**，不是给测试开后门。
//!
//! 合成树不碰任何真实数据：源目录是入库的 `packages/libs` 与
//! `packages/mods/pkg-selftest`。

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value as Json};
use ygg_sandbox::driver::KernelFacade;
use ygg_sandbox::modload::ModBundle;

/// 合成树：`<R>/packages/libs/<产品包…>` + `<R>/mods/pkg-selftest`。
struct Tree {
    root: PathBuf,
}

impl Tree {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("ygg-r2-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("packages/libs")).expect("mkdir package root");
        fs::create_dir_all(root.join("mods")).expect("mkdir mods");
        for id in product_packages() {
            support::copy_dir(&repo_libs().join(&id), &root.join("packages/libs").join(&id));
        }
        support::copy_dir(&repo_fixture(), &root.join("mods/pkg-selftest"));
        Self { root }
    }

    fn mod_dir(&self) -> PathBuf {
        self.root.join("mods/pkg-selftest")
    }

    fn load(&self) -> ModBundle {
        ModBundle::load(&self.mod_dir()).unwrap_or_else(|err| {
            panic!(
                "{} 必须可经 `ModBundle::load` 装载：{err}",
                self.mod_dir().display()
            )
        })
    }
}

/// 仓库里的包根（`packages/libs`）。
fn repo_libs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/libs")
}

/// 仓库里的消费夹具（`packages/mods/pkg-selftest`）。
fn repo_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/mods/pkg-selftest")
}

/// 包根下的**产品包**目录名（升序）：排除装载面活体夹具 `fixture`
/// （它显式豁免「≥2 个游戏用点」那道闸，见 `packages/README.md` §5）。
fn product_packages() -> Vec<String> {
    let mut ids: Vec<String> = fs::read_dir(repo_libs())
        .unwrap_or_else(|err| panic!("read {}: {err}", repo_libs().display()))
        .filter_map(|item| item.ok())
        .filter(|item| item.path().is_dir())
        .filter_map(|item| item.file_name().into_string().ok())
        .filter(|id| id != "fixture")
        .collect();
    ids.sort();
    ids
}

/// `pack.json` 的 `packages[].id`（升序）。
fn declared_packages(mod_dir: &Path) -> Vec<String> {
    let pack = support::read_json(&mod_dir.join("pack.json"));
    let mut ids: Vec<String> = pack["packages"]
        .as_array()
        .unwrap_or_else(|| panic!("{}/pack.json 缺 packages 段", mod_dir.display()))
        .iter()
        .map(|item| item["id"].as_str().expect("packages[].id").to_owned())
        .collect();
    ids.sort();
    ids
}

/// 派发一个事件并取回整份信封（`ok` 必须为真，否则 panic 带原文）。
#[track_caller]
fn dispatch(kernel: &mut KernelFacade, event: &str) -> Json {
    let reply: Json = serde_json::from_str(&kernel.dispatch(event, "0"))
        .unwrap_or_else(|err| panic!("{event} 的回执不是 JSON：{err}"));
    assert_eq!(reply["ok"], json!(true), "{event} 派发失败：{reply}");
    reply
}

/// 信封里的叙事行。
fn lines_of(reply: &Json) -> Vec<String> {
    reply["narrative"]
        .as_array()
        .unwrap_or_else(|| panic!("narrative 不是数组：{reply}"))
        .iter()
        .map(|line| {
            line.as_str()
                .unwrap_or_else(|| panic!("叙事行不是字符串：{line:?}"))
                .to_owned()
        })
        .collect()
}

// ── 用例 ①：生产真值（driver 路径）─────────────────────────────────────────

/// 冒烟叙述行（**逐字**）。七行一包一行，最后一行是收尾。
///
/// 它们与 r2 计划 §6.5 的六包原文逐字节相同（含 `sha=ba7816bf…15ad`、
/// `next32=726839672`、`hash=14b44f969c391c53`），只多了 assetgen 一行与 N=7
/// ——改口径等价于改契约，必须同提交改这条常量。
const SMOKE_LINES: [&str; 8] = [
    "pkg.entity ok id=1 hp=2.5 ref_reject=true",
    "pkg.codec ok array=[1,2,3] obj={\"f\":1.5,\"i\":1} roundtrip=true sha=ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    "pkg.quest ok id=1 state=open step=2",
    "pkg.dialogue ok history=1",
    "pkg.role-skill ok granted=true has=true atk=3",
    "pkg.procgen ok next32=726839672 hash=14b44f969c391c53",
    "pkg.assetgen ok algos=7 canon=kind=texture algo=texture.fbm v=1 seed=smoke stream=assetgen/texture/fbm w=4 h=4 ch=1 bytes=16",
    "pkg.smoke packages=7",
];

#[test]
fn seven_packages_load_and_run_in_the_real_sandbox() {
    let tree = Tree::new("driver");
    let bundle = tree.load();
    let mut kernel = bundle.kernel().expect("内核构造");

    let reply = dispatch(&mut kernel, "pkg.smoke");
    assert_eq!(
        reply["pendingChoice"],
        Json::Null,
        "冒烟不得留下待选（pendingChoice 必须是 nil）：{reply}"
    );
    // 派生事件真的走到了内核：entity.created / entity.attr_changed ×4 /
    // quest.opened / quest.advanced / role.granted。它们全经 `ygg.emit` →
    // 派生队列 → 空订阅 handler，所以这个数字是「载荷过了 value_from_lua」
    // 的直接证据（表载荷会在这里之前就红）。
    assert_eq!(
        reply["eventsDerived"],
        json!(8),
        "派生事件数不对（载荷被拒或 emit 没入队）：{reply}"
    );

    let lines = lines_of(&reply);
    assert_eq!(lines, SMOKE_LINES, "冒烟叙述必须逐字相等：{lines:#?}");

    // R9：收尾行的 N 与声明面、与目录面三方对照——「加包忘改冒烟」与
    // 「冒烟里多写一个不存在的包」两个方向都会红。
    let declared = declared_packages(&tree.mod_dir());
    let products = product_packages();
    assert_eq!(
        declared, products,
        "pack.json 的 packages[] 必须与 packages/libs 下的产品目录恰好相等"
    );
    assert_eq!(
        lines[SMOKE_LINES.len() - 1],
        format!("pkg.smoke packages={}", products.len()),
        "收尾行的包数必须等于声明闭包（由 PACKAGES 表长度现算）"
    );
}
