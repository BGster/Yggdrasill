//! r4 负向面：模块装载的**每一条拒绝路径**都必须可读、可定位、且失败在最前面。
//!
//! 纪律：每例都断言错误文本里的**关键片段**（模块路径 / 行号 / 完整链条 / 上限
//! 常量），不用 `is_err()` 糊过去——"失败了但说不清为什么"和"没失败"对作者是同
//! 一种折磨。装载器自判的拒绝带调用点（`src/a.lua:3:`）与 require 链；模块 chunk
//! 自身的错误按 Lua 原生错误链浮出（每层 `path:line:` 前缀）。
//!
//! 内容寻址一例另有硬要求：**在任何模块执行前失败**。判据是把每个模块都写成
//! `error("SHOULD-NOT-RUN: …")`——若装载真的跑到了模块，错误文本里就会出现这句；
//! 只出现内容寻址拒绝，才算"装载前失败"。

mod support;

use std::fs;
use std::path::Path;

use ygg_sandbox::driver::{KernelError, KernelFacade};
use ygg_sandbox::modload::ModBundle;
use ygg_sandbox::modules::{ModuleSet, MAX_DEPTH, MAX_MODULES};

use support::{modules, read_json, temp_dir, write_json, write_package};

// ───────────────────────── 断言脚手架 ─────────────────────────

/// 错误文本必须同时含这些片段（逐条报出缺了哪一条，不要"第一条命中就过"）。
#[track_caller]
fn assert_has(text: &str, needles: &[&str]) {
    for needle in needles {
        assert!(
            text.contains(needle),
            "错误文本缺少片段 {needle:?}，实际：\n{text}"
        );
    }
}

/// 错误文本必须**不含**这些片段（证伪"失败得太晚"）。
#[track_caller]
fn assert_lacks(text: &str, needles: &[&str]) {
    for needle in needles {
        assert!(
            !text.contains(needle),
            "错误文本不该出现片段 {needle:?}，实际：\n{text}"
        );
    }
}

/// 磁盘包的装载失败文本（`ModBundle::load` 或内核构造，两者都算"装载期"）。
fn load_err_text(dir: &Path) -> String {
    match ModBundle::load(dir) {
        Err(message) => message,
        Ok(bundle) => match bundle.kernel() {
            Err(err) => err.to_string(),
            Ok(_) => panic!("期望装载失败，实际装载成功：{}", dir.display()),
        },
    }
}

/// 模块表装载失败文本（`KernelFacade::with_modules` 路径）。
#[track_caller]
fn loader_err_text(mod_source: &str, set: ModuleSet) -> String {
    match KernelFacade::with_modules(mod_source, set) {
        Ok(_) => panic!("期望装载失败，实际装载成功"),
        Err(err) => {
            assert!(
                matches!(err, KernelError::ModLoad(_)),
                "模块面失败必须浮出为 ModLoad，实际：{err:?}"
            );
            err.to_string()
        }
    }
}

/// `(名字, 源码)` → [`ModuleSet`]。
fn module_set(pairs: &[(&str, &str)]) -> ModuleSet {
    ModuleSet::from_pairs(
        pairs
            .iter()
            .map(|(name, code)| ((*name).to_owned(), code.as_bytes().to_vec())),
    )
    .expect("module set")
}

// ───────────────────────── ① 未登记模块 ─────────────────────────

#[test]
fn an_unregistered_module_is_denied_at_its_call_site() {
    // 文件在盘上但**没声明**：装载期只查已登记模块表，不做路径拼接——盘上有
    // `src/ghost.lua` 也不能 require 到它（声明面是唯一放行依据）。
    let dir = temp_dir("neg-unregistered");
    write_package(
        &dir,
        "require(\"src.ghost\")\n",
        &modules(&[("src/util.lua", "return {}\n")]),
    );
    fs::write(dir.join("src/ghost.lua"), "return { leaked = true }\n").expect("write ghost");

    let text = load_err_text(&dir);
    assert_has(
        &text,
        &[
            "sandbox denied",
            "module 'src.ghost' is not available",
            "require chain: src.ghost",
            // 拒绝带调用点（入口 chunk 名 = `@mod.lua`）。
            "mod.lua:1",
            "only the mod's own registered module set is loadable",
        ],
    );
    assert_lacks(&text, &["leaked"]);

    // 深层未登记同理：链条自外向内。
    let nested = loader_err_text(
        "require(\"src.a\")\n",
        module_set(&[("src.a", "require(\"src.zzz\")\n")]),
    );
    assert_has(
        &nested,
        &[
            "require chain: src.a -> src.zzz",
            "src/a.lua:1",
            "module 'src.zzz' is not available",
        ],
    );
}

// ───────────────────────── ② 可达循环 ─────────────────────────

#[test]
fn a_reachable_cycle_reports_the_whole_chain() {
    // entry → src.a → src.b → src.c → src.a（三节点环，从首次出现处起算）。
    let dir = temp_dir("neg-cycle");
    write_package(
        &dir,
        "require(\"src.a\")\n",
        &modules(&[
            ("src/a.lua", "require(\"src.b\")\n"),
            ("src/b.lua", "require(\"src.c\")\n"),
            ("src/c.lua", "require(\"src.a\")\n"),
        ]),
    );

    let text = load_err_text(&dir);
    assert_has(
        &text,
        &[
            // 完整链条（不是"发现循环"四个字）。
            "circular require: src.a -> src.b -> src.c -> src.a",
            // 拒绝发生在最后一个 require 的调用点（发现处即报出的位置）。
            "src/c.lua:1",
        ],
    );

    // 自环同样报完整链条（不退化成一个名字）。
    let self_cycle = loader_err_text(
        "require(\"src.a\")\n",
        module_set(&[("src.a", "require(\"src.a\")\n")]),
    );
    assert_has(&self_cycle, &["circular require: src.a -> src.a"]);

    // 两节点互依赖（契约里点名的那条链）——入口只是外部引用者，不在链里。
    let mutual = loader_err_text(
        "require(\"src.a\")\n",
        module_set(&[
            ("src.a", "require(\"src.b\")\n"),
            ("src.b", "require(\"src.a\")\n"),
        ]),
    );
    assert_has(
        &mutual,
        &[
            "circular require: src.a -> src.b -> src.a",
            // 报在“要求装载已存活帧”的那一跳（src.b）。
            "src/b.lua:1",
        ],
    );
}

// ───────────────────────── ③④ 模块自身错误可定位 ─────────────────────────

#[test]
fn a_syntax_error_names_the_module_path_and_line() {
    // 语法错：模块路径 + 行号（chunk 名 = `@<相对路径>`，不是 `[C]` / `@mod.lua`）。
    let dir = temp_dir("neg-syntax");
    write_package(
        &dir,
        "require(\"src.broken\")\n",
        &modules(&[("src/broken.lua", "local a = 1\nlocal b = = 2\nreturn a\n")]),
    );

    let text = load_err_text(&dir);
    assert_has(
        &text,
        &["syntax error", "src/broken.lua:2", "unexpected symbol"],
    );
    // 位置必须指向**出错的那一行**（第 2 行），不是 chunk 起始行。
    assert_lacks(&text, &["src/broken.lua:1"]);
}

#[test]
fn a_runtime_error_names_the_module_path_and_line() {
    // 运行期错：同一套定位口径（`path:line:` 前缀 + 原始消息）。
    let dir = temp_dir("neg-runtime");
    write_package(
        &dir,
        "require(\"src.boom\")\n",
        &modules(&[("src/boom.lua", "local a = 1\n\nerror(\"boom\")\nreturn a\n")]),
    );

    let text = load_err_text(&dir);
    assert_has(&text, &["runtime error", "src/boom.lua:3", "boom"]);
    // 顶层（入口）的调用点也在链条里——排障要知道是谁 require 的。
    assert_has(&text, &["mod.lua:1"]);
}

// ───────────────────────── ⑤ 内容寻址先于执行 ─────────────────────────

#[test]
fn a_manifest_mismatch_fails_before_any_module_executes() {
    // 每个模块一执行就炸：若装载真的跑到模块，错误文本里会出现 SHOULD-NOT-RUN。
    let dir = temp_dir("neg-manifest");
    write_package(
        &dir,
        "require(\"src.first\")\nrequire(\"src.second\")\n",
        &modules(&[
            (
                "src/first.lua",
                "error(\"SHOULD-NOT-RUN: src/first.lua\")\n",
            ),
            (
                "src/second.lua",
                "error(\"SHOULD-NOT-RUN: src/second.lua\")\n",
            ),
        ]),
    );
    // 前置：这份包本来是合法的（证明下面的失败是被改出来的）。
    assert!(
        ModBundle::load(&dir).is_ok(),
        "改动前的包必须合法，否则本用例证明不了任何事"
    );

    // ① byteLength 不符（廉价预检，先于哈希）。
    let mut manifest = read_json(&dir.join("manifest.json"));
    let index = manifest["code"]
        .as_array()
        .expect("code 数组")
        .iter()
        .position(|entry| entry["path"] == "src/first.lua")
        .expect("src/first.lua 必须在 manifest 的 code 里");
    let declared = manifest["code"][index]["byteLength"]
        .as_u64()
        .expect("byteLength");
    manifest["code"][index]["byteLength"] = (declared + 1).into();
    write_json(&dir.join("manifest.json"), &manifest);

    let text = load_err_text(&dir);
    assert_has(
        &text,
        &[
            "src/first.lua",
            "content rejected",
            "content size mismatch",
            // 期望值 / 实际值都报出来（作者不用去数字节）。
            &format!("expected {} bytes", declared + 1),
        ],
    );
    assert_lacks(&text, &["SHOULD-NOT-RUN", "runtime error"]);

    // ② 字节数恢复、换掉 sha256（内容换过 / 手改记录）。
    support::reseal(&dir);
    let mut manifest = read_json(&dir.join("manifest.json"));
    manifest["code"][index]["sha256"] = "0".repeat(64).into();
    write_json(&dir.join("manifest.json"), &manifest);

    let text = load_err_text(&dir);
    assert_has(
        &text,
        &["src/first.lua", "content rejected", "content hash mismatch"],
    );
    assert_lacks(&text, &["SHOULD-NOT-RUN", "runtime error"]);
}

// ───────────────────────── ⑥ 模块数上限 ─────────────────────────

#[test]
fn more_modules_than_the_loader_can_register_are_rejected_before_loading() {
    // 129 个模块（上限 128）：两层的判定都要可读——打包声明面与模块表登记面。
    let dir = temp_dir("neg-too-many");
    let overflow: Vec<(String, String)> = (0..=MAX_MODULES)
        .map(|index| {
            (
                format!("src/m{index:03}.lua"),
                format!("return {{ index = {index} }}\n"),
            )
        })
        .collect();
    write_package(&dir, "return true\n", &overflow);

    let text = load_err_text(&dir);
    assert_has(
        &text,
        &[
            "pack.json",
            &format!("{} code modules", MAX_MODULES + 1),
            &format!("limit {MAX_MODULES}"),
        ],
    );

    // 模块表自身也拒绝第 129 个（装载器入口的兜底面，不依赖打包声明）。
    let mut set = ModuleSet::new();
    for index in 0..MAX_MODULES {
        set.insert(&format!("m{index:03}"), b"return true".to_vec())
            .expect("上限内的登记必须成功");
    }
    let err = set
        .insert("m_overflow", b"return true".to_vec())
        .expect_err("第 129 个模块必须被拒绝")
        .to_string();
    assert_has(
        &err,
        &[
            &format!("too many modules: {}", MAX_MODULES + 1),
            &format!("limit {MAX_MODULES}"),
        ],
    );
}

// ───────────────────────── ⑦ 依赖深度上限 ─────────────────────────

#[test]
fn a_dependency_chain_deeper_than_the_limit_is_rejected_with_the_chain() {
    /// `n` 层链（`m0` → `m1` → … → 末端 `return true`）。
    fn chain(n: usize) -> ModuleSet {
        let pairs: Vec<(String, Vec<u8>)> = (0..n)
            .map(|index| {
                let code = if index + 1 < n {
                    format!("return require(\"m{}\")\n", index + 1)
                } else {
                    "return true\n".to_owned()
                };
                (format!("m{index}"), code.into_bytes())
            })
            .collect();
        ModuleSet::from_pairs(pairs).expect("chain module set")
    }

    // 上限内（64 层）正常装载——拒绝必须是显式判栈，不是"碰巧能跑 64"。
    assert!(
        KernelFacade::with_modules("require(\"m0\")\n", chain(MAX_DEPTH)).is_ok(),
        "{MAX_DEPTH} 层嵌套必须能装载"
    );

    // 65 层：显式判栈拒绝，链条带中段省略（首尾各 3 段）。
    let text = loader_err_text("require(\"m0\")\n", chain(MAX_DEPTH + 1));
    assert_has(
        &text,
        &[
            &format!("require depth exceeded ({MAX_DEPTH} nested modules)"),
            "m0 -> m1 -> m2 -> … -> m62 -> m63 -> m64",
            // 拒绝带调用点：深度在 push 时判，所以报的是**要求装载 m64 的那一跳**。
            "m63.lua:1",
        ],
    );
    assert_lacks(&text, &["stack overflow"]);
}

// ───────────────────────── ⑧ 非法模块名（五种） ─────────────────────────

#[test]
fn invalid_module_names_are_all_rejected_readably() {
    // 五种形态：绝对路径 / `..` 上跳 / 反斜杠 / 空段 / NUL 字节。
    // 每种都给**专用**拒绝（不是笼统的"名字非法"），且带调用点。
    let cases: &[(&str, &str, &str)] = &[
        (
            "绝对路径",
            "require(\"/abs/util\")\n",
            "module name must be relative",
        ),
        ("上跳 ..", "require(\"..\")\n", "empty segment"),
        (
            "反斜杠",
            "require(\"src\\\\util\")\n",
            "contains a backslash",
        ),
        ("空段", "require(\"src..util\")\n", "empty segment"),
        (
            "NUL 字节",
            "require(\"src\\0util\")\n",
            "contains a NUL byte",
        ),
    ];

    for (label, source, needle) in cases {
        let text = loader_err_text(source, ModuleSet::new());
        assert_has(&text, &[needle, "mod.lua:1"]);
        // 拒绝必须发生在装载期（ModLoad），且不是裸 nil-call。
        assert_lacks(&text, &["attempt to call a nil value"]);
        assert!(
            !text.is_empty(),
            "{label} 的拒绝文案不得为空（每条都要能读）"
        );
    }

    // 附带：字符集外的段落（`/` 不是点分分隔符——点分名与路径形态不能混用）。
    let bad_char = loader_err_text("require(\"src/a\")\n", ModuleSet::new());
    assert_has(
        &bad_char,
        &["contains a forbidden character", "'/'", "mod.lua:1"],
    );

    // 反例锚：`src.a.b` 是**合法**规范名（三段），不得被误伤（双射面的正向侧）。
    assert!(
        KernelFacade::with_modules(
            "require(\"src.a.b\")\n",
            module_set(&[("src.a.b", "return true\n")])
        )
        .is_ok(),
        "三段点分名必须能装载"
    );
}
