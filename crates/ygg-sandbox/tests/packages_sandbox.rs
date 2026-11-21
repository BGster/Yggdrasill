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
use ygg_sandbox::sandbox::Sandbox;

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
            support::copy_dir(
                &repo_libs().join(&id),
                &root.join("packages/libs").join(&id),
            );
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

/// `codec.sha256("abc")` 的已知答案向量（FIPS 180-4 附录 B），driver 与裸沙箱
/// 两边都拿它当锚点。
const SHA256_ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

/// 冒烟叙述行（**逐字**）。八行一包一行，最后一行是收尾。
///
/// 它们与 r2 计划 §6.5 的六包原文逐字节相同（含 `sha=ba7816bf…15ad`、
/// `next32=726839672`、`hash=14b44f969c391c53`），只多了 assetgen / textframe
/// 两行与 N=8
/// ——改口径等价于改契约，必须同提交改这份清单。
fn smoke_lines() -> Vec<String> {
    [
        "pkg.entity ok id=1 hp=2.5 ref_reject=true".to_owned(),
        format!("pkg.codec ok array=[1,2,3] obj={{\"f\":1.5,\"i\":1}} roundtrip=true sha={SHA256_ABC}"),
        "pkg.quest ok id=1 state=open step=2".to_owned(),
        "pkg.dialogue ok history=1".to_owned(),
        "pkg.role-skill ok granted=true has=true atk=3".to_owned(),
        "pkg.procgen ok next32=726839672 hash=14b44f969c391c53".to_owned(),
        "pkg.assetgen ok algos=7 canon=kind=texture algo=texture.fbm v=1 seed=smoke stream=assetgen/texture/fbm w=4 h=4 ch=1 bytes=16".to_owned(),
        "pkg.textframe ok rows=3 cells=A___|___Z|HUD".to_owned(),
        "pkg.smoke packages=8".to_owned(),
    ]
    .into_iter()
    .collect()
}

#[test]
fn packages_load_and_run_in_the_real_sandbox() {
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
    let expected = smoke_lines();
    assert_eq!(lines, expected, "冒烟叙述必须逐字相等：{lines:#?}");

    // R9：收尾行的 N 与声明面、与目录面三方对照——「加包忘改冒烟」与
    // 「冒烟里多写一个不存在的包」两个方向都会红。
    let declared = declared_packages(&tree.mod_dir());
    let products = product_packages();
    assert_eq!(
        declared, products,
        "pack.json 的 packages[] 必须与 packages/libs 下的产品目录恰好相等"
    );
    assert_eq!(
        lines[expected.len() - 1],
        format!("pkg.smoke packages={}", products.len()),
        "收尾行的包数必须等于声明闭包（由 PACKAGES 表长度现算）"
    );
}

// ── 用例 ②：无注入面（裸 `Sandbox::new()`，没有 `inject::install`）────────

#[test]
fn packages_load_in_a_bare_sandbox_without_determinism_injection() {
    let tree = Tree::new("bare");
    let bundle = tree.load();

    // 根模块名从模块表现算：加一个包，这条自动跟上（不手抄包清单）。
    let mut roots: Vec<String> = bundle
        .modules
        .names()
        .filter(|name| name.starts_with("pkg.") && name.matches('.').count() == 1)
        .map(str::to_owned)
        .collect();
    roots.sort();
    assert_eq!(
        roots.len(),
        product_packages().len(),
        "每个产品包应恰好一个根模块：{roots:?}"
    );

    // 裸沙箱：没有 `inject::install`（math 缺席）、没有完整 host API（只有
    // 占位 `ygg = { engine_version }`）——所以这条用例问的是「包的**顶层**
    // 是否真的只依赖 base 白名单 + 模块表」。
    let sandbox = Sandbox::new().expect("裸沙箱构造");
    sandbox.modules().set_modules(bundle.modules.clone());
    sandbox
        .modules()
        .set_chunk_paths(bundle.chunk_paths.clone());

    let names = roots
        .iter()
        .map(|name| format!("\"{name}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let chunk = format!(
        r#"
        assert(math == nil, "裸沙箱不该有 math：本用例的前提是没跑 inject")
        local roots = {{ {names} }}
        for i = 1, #roots do
          require(roots[i])
        end
        local codec = require("pkg.codec")
        local array = codec.encode({{ 1, 2, 3 }})
        local object = codec.encode({{ i = 1, f = 1.5 }})
        local roundtrip = codec.encode(codec.decode(object)) == object
        return #roots .. "|" .. array .. "|" .. object .. "|" .. tostring(roundtrip)
          .. "|" .. codec.sha256("abc")
        "#
    );
    let observed: String = sandbox
        .lua()
        .load(chunk)
        .set_name("@bare/packages")
        .eval()
        .unwrap_or_else(|err| panic!("裸沙箱装包失败（包依赖了只有 driver 才有的面）：{err}"));

    assert_eq!(
        observed,
        format!(
            "{}|[1,2,3]|{{\"f\":1.5,\"i\":1}}|true|{}",
            roots.len(),
            SHA256_ABC
        ),
        "裸沙箱里七个根模块必须全部可 require，且 codec 的输出与 driver 路径同字节"
    );
}

// ── 用例 ③：静态面（包的源码不得出现白名单外的全局）───────────────────

/// 裸沙箱缺席的**命名空间表**（出现 `<名>.` 即视为一次点式调用）。
const NAMESPACE_GLOBALS: [&str; 9] = [
    "coroutine",
    "debug",
    "io",
    "math",
    "os",
    "package",
    "string",
    "table",
    "utf8",
];

/// 裸沙箱缺席的**函数**（出现 `<名>(` 即视为一次调用）。
const FUNCTION_GLOBALS: [&str; 6] = [
    "collectgarbage",
    "dofile",
    "load",
    "loadfile",
    "print",
    "rawset",
];

/// 包内所有**非 `test/`** 的 Lua 源文件（`(包 id, 文件路径)`，按路径升序）。
fn package_sources() -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    for id in product_packages() {
        collect_lua(&repo_libs().join(&id), &id, &mut out);
    }
    out.sort();
    out
}

fn collect_lua(dir: &Path, id: &str, out: &mut Vec<(String, PathBuf)>) {
    for item in fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("read {}: {err}", dir.display()))
        .filter_map(|item| item.ok())
    {
        let path = item.path();
        if path.is_dir() {
            // `test/**` 跑在普通 Lua 里（有完整标准库），不在沙箱装载面上。
            if path.file_name().and_then(|name| name.to_str()) == Some("test") {
                continue;
            }
            collect_lua(&path, id, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("lua") {
            out.push((id.to_owned(), path));
        }
    }
}

/// 扫一段 Lua 源码，返回**代码区**（注释与字符串之外）首次出现的白名单外全局
/// 调用形态：`(行号, 形态)`。
///
/// 为什么要一个小词法器而不是逐行 `find`：注释与字符串里出现这些名字是**合法的**
/// ——`assetgen/spec.lua` 写着「不用 `math.type`」、`join.lua` 写着「`table.concat`
/// 不可达」、`mod-package.md` 这个文档路径里就含 `package.`。逐行文本匹配会把这
/// 八处**正确**的叙述判成违规，于是门禁只能靠加例外活着——那比没有门禁更糟。
///
/// 判据：命名空间名后面紧跟 `.`、函数名后面紧跟 `(`；前置字符是 `.` / `:` 的视为
/// 字段访问（`a.b`、`a:b`），不算全局。
fn forbidden_global(source: &str) -> Option<(usize, String)> {
    let bytes = source.as_bytes();
    let mut i = 0;
    let mut line = 1usize;
    while i < bytes.len() {
        let byte = bytes[i];
        // 注释：`--` 后可接长括号（`--[[ … ]]`），否则吃到行尾。
        if byte == b'-' && bytes.get(i + 1) == Some(&b'-') {
            i += 2;
            match long_bracket_end(bytes, i) {
                Some(end) => {
                    line += count_newlines(&bytes[i..end]);
                    i = end;
                }
                None => {
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                }
            }
            continue;
        }
        // 长字符串 `[[ … ]]` / `[==[ … ]==]`。
        if byte == b'[' {
            if let Some(end) = long_bracket_end(bytes, i) {
                line += count_newlines(&bytes[i..end]);
                i = end;
                continue;
            }
        }
        // 短字符串：跳掉转义与闭合引号（未闭合就停在行尾，不吞后文）。
        if byte == b'"' || byte == b'\'' {
            i = skip_short_string(bytes, i);
            continue;
        }
        if byte == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        if byte.is_ascii_alphabetic() || byte == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &source[start..i];
            let is_field = matches!(bytes.get(start.wrapping_sub(1)), Some(b'.') | Some(b':'));
            if !is_field {
                let follow = if NAMESPACE_GLOBALS.contains(&word) {
                    Some(b'.')
                } else if FUNCTION_GLOBALS.contains(&word) {
                    Some(b'(')
                } else {
                    None
                };
                if let Some(follow) = follow {
                    let mut next = i;
                    while next < bytes.len() && bytes[next].is_ascii_whitespace() {
                        next += 1;
                    }
                    if bytes.get(next) == Some(&follow) {
                        return Some((line, format!("{word}{}", follow as char)));
                    }
                }
            }
            continue;
        }
        i += 1;
    }
    None
}

fn count_newlines(bytes: &[u8]) -> usize {
    bytes.iter().filter(|byte| **byte == b'\n').count()
}

/// `start` 指向 `[` 时返回长括号的结束下标（`None` = 不是长括号）。
fn long_bracket_end(bytes: &[u8], start: usize) -> Option<usize> {
    if bytes.get(start) != Some(&b'[') {
        return None;
    }
    let mut i = start + 1;
    let mut level = 0usize;
    while bytes.get(i) == Some(&b'=') {
        level += 1;
        i += 1;
    }
    if bytes.get(i) != Some(&b'[') {
        return None;
    }
    i += 1;
    while i < bytes.len() {
        if bytes[i] == b']' {
            let mut j = i + 1;
            let mut seen = 0usize;
            while bytes.get(j) == Some(&b'=') {
                seen += 1;
                j += 1;
            }
            if seen == level && bytes.get(j) == Some(&b']') {
                return Some(j + 1);
            }
        }
        i += 1;
    }
    Some(bytes.len())
}

fn skip_short_string(bytes: &[u8], start: usize) -> usize {
    let quote = bytes[start];
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'\n' => return i,
            byte if byte == quote => return i + 1,
            _ => i += 1,
        }
    }
    i
}

#[test]
fn package_sources_use_no_global_outside_the_base_whitelist() {
    let sources = package_sources();
    let visited: Vec<String> = {
        let mut ids: Vec<String> = sources.iter().map(|(id, _)| id.clone()).collect();
        ids.dedup();
        ids
    };
    assert_eq!(
        visited,
        product_packages(),
        "扫描面必须恰好是产品包目录（扫不到 = 这条用例空转）"
    );

    let mut offenders: Vec<String> = Vec::new();
    for (_, path) in &sources {
        let source =
            fs::read_to_string(path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        let name = path
            .strip_prefix(repo_libs())
            .map(|rel| rel.display().to_string())
            .unwrap_or_else(|_| path.display().to_string());
        if let Some((line, form)) = forbidden_global(&source) {
            offenders.push(format!("{name}:{line}: {form} 不在 base 白名单"));
        }
    }
    assert!(
        offenders.is_empty(),
        "包源码出现白名单外的全局（裸沙箱装载期必炸）：\n{}",
        offenders.join("\n")
    );

    // 禁名列自检：这些名字在裸沙箱里确实全为 nil。沙箱哪天真的放出
    // `math`（或别的名字），这里会红并提醒复核——否则上面那段扫描就只是
    // 一句「相信白名单」的断言。
    let sandbox = Sandbox::new().expect("裸沙箱构造");
    for global in NAMESPACE_GLOBALS.iter().chain(FUNCTION_GLOBALS.iter()) {
        let absent: bool = sandbox
            .lua()
            .load(format!("return {global} == nil"))
            .eval()
            .unwrap_or_else(|err| panic!("探测 {global} 失败：{err}"));
        assert!(
            absent,
            "{global} 出现在沙箱 globals 里了：白名单变了，禁名列需要重新复核"
        );
    }
}

// ── 用例 ④：`pkg.entity` 导出面键集恰等（I6 的宿主面硬钉）─────────────────
//
// 为什么放在真沙箱而不是只在包自测里：`pkg.entity` 的导出面在沙箱里可能少键——
// 少了就是七包消费方拿到 nil，而这在普通 Lua 的 selftest 里看不出来（那里 `string`
// 表还在、模块装载路径也不同）。这条与包侧的 `pkg.entity.exports` 是**两道钉子**，
// 不是一份断言抄两遍：一个钉「实现者改了表」，一个钉「沙箱里可见性与源码一致」。
//
// 键集**恰等**：多一个键 = 未申报的能力面（下沉后最容易发生的漂移），少一个键 = 契约
// §4 的 API 面断裂。
#[test]
fn entity_exports_are_exactly_the_seventeen_documented_keys() {
    let tree = Tree::new("entity-exports");
    let bundle = tree.load();
    let sandbox = Sandbox::new().expect("裸沙箱构造");
    sandbox.modules().set_modules(bundle.modules.clone());
    sandbox
        .modules()
        .set_chunk_paths(bundle.chunk_paths.clone());

    // `pairs` 在 base 白名单里；`table.sort` / `string` 不在了（这正是本用例的前提），
    // 所以排序交给 Rust 侧做，Lua 只负责把键名拼出来。
    let chunk = r#"
        local entity = require("pkg.entity")
        local out = ""
        for key in pairs(entity) do
          out = out .. key .. ","
        end
        return out
    "#;
    let observed: String = sandbox
        .lua()
        .load(chunk)
        .set_name("@bare/entity-exports")
        .eval()
        .unwrap_or_else(|err| panic!("裸沙箱里 require(\"pkg.entity\") 失败：{err}"));

    let mut seen: Vec<&str> = observed.split(',').filter(|s| !s.is_empty()).collect();
    seen.sort_unstable();

    let mut expected = vec![
        "VERSION",
        "attrs",
        "count",
        "define",
        "destroy",
        "exists",
        "get",
        "has",
        "ids",
        "install",
        "schema",
        "schema_of",
        "set",
        "spawn",
        "tag",
        "tags",
        "untag",
    ];
    expected.sort_unstable();

    assert_eq!(
        seen, expected,
        "pkg.entity 的导出面必须恰为契约 §4 的 17 个键（多/少都红）：{seen:?}"
    );
}
