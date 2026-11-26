//! r1 纯导出的**收益**：装配序与 require 序是两个独立的轴。
//!
//! 改造前，模块可以在自己的顶层用 `ygg` 的 `on(...)` 注册 handler，而注册的
//! 调用序就是派发序——于是「文件怎么拆、require 写在第几行」直接决定运行时
//! 行为。模块顶层改成纯导出、装配集中在入口之后，两个轴各有一个唯一来源：
//!
//! - **require 序** → 模块体的**执行序**（`probe.trace`）；
//! - **入口的 install 调用序** → **装配序 / 派发序**（`probe.order`）。
//!
//! 装配序为什么需要 `probe.order` 这条观测：装配的动作在装载期就发生完了，
//! 之后不再有「派发」这一步——入口把装配点按调用序记进 KV，顺序本身就是契约。
//!
//! 本文件所有用例的入口都在顶层写 KV（观测面）；入口 chunk 豁免 MOD_EFFECT，
//! 它们能装载起来本身就是这条豁免的证据。

mod support;

use std::fs;
use std::path::PathBuf;

use ygg_sandbox::modload::{LoadError, Loaded, ModBundle};
use ygg_sandbox::modules::ModuleSet;

use support::{copy_fixture, fixture_loaded, loaded_of, narrate};

// 以下三个常量与 `modules_lifecycle.rs` 同名同值：`tests/` 下各文件是独立
// crate，不能互相引用，所以只能手抄——改动 fixture 的装配序时两处一起改。

/// 入口的装配序（= 入口的调用序，唯一来源）。
const ORDER: [&str; 4] = ["util", "shared", "handlers", "entry"];

/// fixture 的模块体执行序（= 依赖图遍历序，**不是**装配序）。
const TRACE: &str = "trace=>handlers>util>shared>silent";

/// fixture 装载后的「全新状态」：模块各执行一次、模块内状态零值。
const FRESH: &str = "util_execs=1;shared_runs=1;silent_runs=1;util_count=0;shared_count=0";

/// 由 `(名字, 源码)` 构造模块表（内联用例的便捷面）。
fn module_set(pairs: &[(&str, &str)]) -> ModuleSet {
    ModuleSet::from_pairs(
        pairs
            .iter()
            .map(|(name, code)| ((*name).to_owned(), code.as_bytes().to_vec())),
    )
    .expect("module set")
}

/// 装载失败的报错文本（入口 chunk 的错误一律经 `LoadError::ModChunk` 浮出）。
fn load_err_text(mod_source: &str, modules: ModuleSet) -> String {
    match loaded_of(mod_source, modules) {
        Ok(_) => panic!("期望装载失败，实际装载成功"),
        Err(err) => {
            assert!(
                matches!(err, LoadError::ModChunk(_)),
                "入口失败必须浮出为 chunk 拒绝，实得：{err:?}"
            );
            err.to_string()
        }
    }
}

/// 唯一命中 `needle` 的那一行（多行命中即测试自身的假设坏了）。
fn line_index(lines: &[String], needle: &str) -> usize {
    let hits: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.contains(needle))
        .map(|(index, _)| index)
        .collect();
    assert_eq!(hits.len(), 1, "`{needle}` 必须唯一命中，实得 {hits:?}");
    hits[0]
}

/// 把入口里所有 `local ... = require("src....")` 行**逆序**写回原位（其余行不动）。
///
/// 只动 require 顺序、不碰任何 `util.register(note)` 调用序——这正是「模块纯
/// 导出之后，require 顺序变成自由变量」的直接用法。
fn reverse_require_lines(source: &str) -> String {
    let mut lines: Vec<String> = source.lines().map(str::to_owned).collect();
    let slots: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| {
            let trimmed = line.trim_start();
            trimmed.starts_with("local ") && trimmed.contains("require(\"src.")
        })
        .map(|(index, _)| index)
        .collect();
    assert_eq!(
        slots.len(),
        5,
        "fixture 入口应有 5 个模块 require 行，实得 {slots:?}（fixture 形态变了？）"
    );
    let originals: Vec<String> = slots.iter().map(|&index| lines[index].clone()).collect();
    for (&slot, line) in slots.iter().zip(originals.iter().rev()) {
        lines[slot] = line.clone();
    }
    assert_ne!(
        lines, originals,
        "require 行的内容必须两两不同，否则逆序不产生任何变化"
    );
    let mut text = lines.join("\n");
    if source.ends_with('\n') {
        text.push('\n');
    }
    text
}

/// 复制 fixture、按 `rewrite` 改写**入口**、装载（返回临时目录以便清理）。
///
/// 改写入口不需要重算 `manifest.json`：`code.entry` 不入内容寻址清单（被寻址
/// 的是 `code.modules` 里的模块文件），入口的作者自由度正是这一层要证明的。
fn fixture_with_rewritten_entry(tag: &str, rewrite: impl Fn(&str) -> String) -> (PathBuf, Loaded) {
    let dir = copy_fixture(tag);
    let path = dir.join("mod.lua");
    let source = fs::read_to_string(&path).expect("读入口");
    let rewritten = rewrite(&source);
    assert_ne!(
        rewritten, source,
        "改写必须真的落到入口源码上（否则整条用例只是空转）"
    );
    fs::write(&path, &rewritten).expect("写入口");
    let bundle = ModBundle::load(&dir).expect("改写后的包必须可装载");
    assert_eq!(bundle.modules.len(), 4, "模块表不因入口改写而变化");
    (
        dir,
        Loaded::from_bundle(&bundle).expect("改写后的包必须能装载"),
    )
}

// ───────────────── ① 改 require 序 → 装配序不动（重构安全） ─────────────────

#[test]
fn swapping_the_require_order_leaves_the_assembly_order_untouched() {
    // 基线：入库件未经改写。
    let baseline = fixture_loaded();
    assert_eq!(narrate(&baseline, "probe.order"), ORDER);
    assert_eq!(narrate(&baseline, "probe.trace"), [TRACE]);

    // 把入口的五条 require 全部逆序：`src.handlers` 从第一个 require 变成最后
    // 一个。改造前那一刻就等同于把 handlers 的 handler 挪到派发链末尾——本用例
    // 断言的正是"现在不会了"。
    let (dir, reordered) =
        fixture_with_rewritten_entry("pure-export-require", reverse_require_lines);

    // ① 前提成立：模块体的执行序**真的**变了（不是"没改成功"的假绿）。
    assert_eq!(
        narrate(&reordered, "probe.trace"),
        ["trace=>silent>shared>util>handlers"],
        "reshape 之后模块执行序必须按新的 require 序变化；否则本用例失去意义"
    );
    // ② 关键断言：装配序一行都没动。
    assert_eq!(
        narrate(&reordered, "probe.order"),
        ORDER,
        "模块只做纯导出 ⇒ require 顺序是自由变量，拆文件/挪 require 都不改装配序"
    );
    // ③ 身份与执行次数也不受 require 序影响（缓存按规范名走，与谁先 require 无关）。
    assert_eq!(
        narrate(&reordered, "probe.identity"),
        ["util_same=true;shared_same=true;util_execs=1;shared_runs=1"]
    );
    assert_eq!(narrate(&reordered, "probe.stats"), [FRESH]);

    let _ = fs::remove_dir_all(&dir);
}

// ───────────────── ② 改 install 调用序 → 装配序跟着变（反过来钉） ─────────────────

#[test]
fn swapping_the_install_calls_moves_the_assembly_order() {
    // 反向验证：如果上一条只是"装配序根本不看任何东西"的巧合，这一条会红。
    // 装配序的唯一来源就是入口里 `M.register(note)` 的调用序。
    let (dir, swapped) = fixture_with_rewritten_entry("pure-export-install", |source| {
        let mut lines: Vec<String> = source.lines().map(str::to_owned).collect();
        let util = line_index(&lines, "util.register(note)");
        let handlers = line_index(&lines, "handlers.register_order(note)");
        lines.swap(util, handlers);
        let mut text = lines.join("\n");
        if source.ends_with('\n') {
            text.push('\n');
        }
        text
    });

    // 对调 `util.register(note)` 与 `handlers.register_order(note)` ⇒ handlers 抢到首位。
    assert_eq!(
        narrate(&swapped, "probe.order"),
        ["handlers", "shared", "util", "entry"],
        "装配序必须跟着入口的调用序走（这是唯一来源）"
    );
    // 而模块执行序纹丝未动：两个轴互相独立，各有唯一来源。
    assert_eq!(
        narrate(&swapped, "probe.trace"),
        [TRACE],
        "只改 install 调用序不该动模块执行序（那是 require 序的事）"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ───────────────── ③ 入口必须先 require 才有句柄 ─────────────────

/// 纯导出模块：`register` 只在入口调用时才写状态，顶层不碰任何 host API。
const UTIL: &str = r#"local M = {}

function M.register()
  ygg.kv.set("probe", "from-module")
end

return M
"#;

#[test]
fn the_entry_must_require_a_module_to_get_a_usable_handle() {
    // 模块返回的 table **不是全局**（Lua 的 `local` 不跨 chunk）：入口少了那行
    // require，句柄就是 nil，`util.register()` 当场炸在入口自己那一行。把这条
    // 失败形态断言下来，作者踩空时看到的是"我忘了 require"，而不是"register
    // 静默没生效"。
    const OK_ENTRY: &str = "local util = require(\"src.util\")\nutil.register()\n";

    // ① 正确形态：require → 句柄 → install，状态真的写进去了。
    let loaded = loaded_of(OK_ENTRY, module_set(&[("src.util", UTIL)]))
        .expect("require + install 必须装载成功");
    assert_eq!(narrate(&loaded, "probe"), ["from-module"]);

    // ② 忘 require（把模块当全局）：报错点名 global 与入口行号。
    let text = load_err_text("util.register()\n", module_set(&[("src.util", UTIL)]));
    assert!(
        text.contains("mod.lua:1") && text.contains("attempt to index a nil value (global 'util')"),
        "忘 require 必须在入口第 1 行就报 global 为 nil：{text}"
    );

    // ③ 声明了 local 却忘了赋值：同样可读，但指向 local。
    let text = load_err_text(
        "local util\nutil.register()\n",
        module_set(&[("src.util", UTIL)]),
    );
    assert!(
        text.contains("mod.lua:2") && text.contains("attempt to index a nil value (local 'util')"),
        "`local util` 未赋值必须报 local 为 nil：{text}"
    );

    // ④ require 拼错路径：装载器自判的拒绝（点名缺哪个模块），不是 nil-index。
    let text = load_err_text(
        "local util = require(\"src.utill\")\nutil.register()\n",
        module_set(&[("src.util", UTIL)]),
    );
    assert!(
        text.contains("mod.lua:1") && text.contains("'src.utill' is not available"),
        "require 拼错必须点名缺的模块名：{text}"
    );

    // ⑤ 只 require 不 install：装载成功，但没有任何装配发生——装配**只**由入口
    //    那一行决定，模块装载不会顺手把自己的副作用塞进去（纯导出的直接后果）。
    let idle = loaded_of(
        "local util = require(\"src.util\")\n",
        module_set(&[("src.util", UTIL)]),
    )
    .expect("只 require 必须能装载");
    assert!(
        idle.kv().borrow().get("probe").is_none(),
        "没调用 M.register() 就不该有任何装配（模块顶层不产生副作用）"
    );
}
