//! r3 核心承诺的**运行期证据**：模块只做纯导出，装配集中在入口。
//!
//! # 这条性质为什么值得一组专门的测试
//!
//! 改造前，模块可以在自己的顶层 `ygg.on(...)` 注册 handler，而 `ygg.on` 的
//! **调用序就是 handler 的派发序**。于是「入口先 require 谁」决定了「谁先被
//! 派发」——把一个大文件拆成三个会改变运行时行为，重构不保行为。改造后模块
//! 顶层只剩纯导出（`ygg_modules::effects` + `modules::ShadowBanned` 拦在运行
//! 时，`ygg-lint --pack` 拦在作者侧），派发序唯一的来源是入口里那几行
//! `M.register()` 的**调用序**。
//!
//! 本文件把这条性质钉在两个方向上，缺一个方向就只是"看起来对"：
//!
//! 1. **改 require 顺序 → 派发序不动**（重构安全的正面证据）。为了让这条断言
//!    不沦为空转，同一个用例还断言**模块体执行序确实变了**（`probe.trace`）——
//!    "什么都没改所以什么都没变"是这类测试最容易掉进去的假绿；
//! 2. **改 install 调用序 → 派发序跟着动**（派发序真的由入口决定，不是巧合）。
//!
//! 另外几条盯懒加载与装配前置条件：跨事件 `require` 只执行一次、handler 里
//! require 到的模块照样不许注册、以及「入口必须先 `require` 才有句柄可用」
//! 这个作者最容易踩空的地方（Lua 的 `local` 不跨 chunk，模块不是全局）。
//!
//! 最后两条是围欄：旧写法（模块顶层注册）必须**装载期就红**而不是静默改变
//! 派发序（用例 ① 的等号因此不是靠自觉维持的）；单文件 Mod（无 `code` 段）
//! 必须逐字节照旧——本轮改造不该把它们拖动分毫。
//!
//! 入口本身不受副作用边界约束（它就是装配清单），所以本文件所有用例的入口
//! 都在顶层调 `ygg.on`——它们能装载起来本身就是这条豁免的证据。

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use ygg_sandbox::driver::{KernelError, KernelFacade};
use ygg_sandbox::modload::ModBundle;
use ygg_sandbox::modules::ModuleSet;
use ygg_sandbox::playtest::{playtest, PlaytestOptions, Reached};

use support::{copy_fixture, fixture_kernel, narrate, reseal};

// ───────────────────────── 期望值 ─────────────────────────

/// fixture 的 handler 派发序：由入口的
/// `util.register()` → `shared.register()` → `handlers.register_order()` →
/// 入口自己的 `ygg.on("probe.order", ...)` 决定。
///
/// 与 `modules_lifecycle::ORDER` 同值：`tests/` 下的各文件是独立 crate，不能
/// 互相引用，所以两份只能手抄保持一致——改动 fixture 的 install 序时两处一起改。
const ORDER: [&str; 4] = ["util", "shared", "handlers", "entry"];

/// fixture 的模块体执行序（= 依赖图遍历序，**不是**派发序）。
const TRACE: &str = "trace=>handlers>util>shared>silent";

/// fixture 装载后的「全新状态」：模块各执行一次、模块内状态零值。
const FRESH: &str = "util_execs=1;shared_runs=1;silent_runs=1;util_count=0;shared_count=0";

// ───────────────────────── 脚手架 ─────────────────────────

/// 由 `(名字, 源码)` 构造模块表（内联用例的便捷面）。
fn module_set(pairs: &[(&str, &str)]) -> ModuleSet {
    ModuleSet::from_pairs(
        pairs
            .iter()
            .map(|(name, code)| ((*name).to_owned(), code.as_bytes().to_vec())),
    )
    .expect("module set")
}

/// 装载失败的报错文本（入口 chunk 的错误一律经 `KernelError::ModLoad` 浮出）。
fn load_err_text(mod_source: &str, modules: ModuleSet) -> String {
    match KernelFacade::with_modules(mod_source, modules) {
        Ok(_) => panic!("预期装载失败，实际装载成功了：\n{mod_source}"),
        Err(err) => {
            assert!(
                matches!(err, KernelError::ModLoad(_)),
                "入口 chunk 的错误必须经 ModLoad 浮出，实得：{err:?}"
            );
            err.to_string()
        }
    }
}

/// 唯一命中 `needle` 的那一行（多行命中即测试自身的假设坏了）。
fn line_index(lines: &[String], needle: &str) -> usize {
    let mut hits = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.trim() == needle)
        .map(|(index, _)| index);
    let index = hits
        .next()
        .unwrap_or_else(|| panic!("入口里找不到 `{needle}`"));
    assert!(hits.next().is_none(), "`{needle}` 在入口里出现了多次");
    index
}

/// 把入口里所有 `local ... = require("src....")` 行**逆序**写回原位（其余行不动）。
///
/// 只动 require 顺序、不碰任何 `install` 调用序——这正是「模块纯导出之后，
/// require 顺序变成自由变量」的直接用法。
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

/// 复制 fixture、按 `rewrite` 改写**入口**、装载成内核（返回临时目录以便清理）。
///
/// 改写入口不需要重算 `manifest.json`：`code.entry` 不入内容寻址清单（被寻址
/// 的是 `code.modules` 里的模块文件），入口的作者自由度正是这一层要证明的。
fn fixture_with_rewritten_entry(
    tag: &str,
    rewrite: impl Fn(&str) -> String,
) -> (PathBuf, KernelFacade) {
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
    (dir, bundle.kernel().expect("内核构造"))
}

// ───────────────── ① 改 require 序 → 派发序不动（重构安全） ─────────────────

#[test]
fn swapping_the_require_order_leaves_the_dispatch_order_untouched() {
    // 基线：入库件未经改写。
    let mut baseline = fixture_kernel();
    assert_eq!(narrate(&mut baseline, "probe.order"), ORDER);
    assert_eq!(narrate(&mut baseline, "probe.trace"), [TRACE]);

    // 把入口的五条 require 全部逆序：`src.handlers` 从第一个 require 变成最后
    // 一个。改造前那一刻就等同于把 handlers 的 handler 挪到派发链末尾——本用例
    // 断言的正是"现在不会了"。
    let (dir, mut reordered) =
        fixture_with_rewritten_entry("pure-export-require", reverse_require_lines);

    // ① 前提成立：模块体的执行序**真的**变了（不是"没改成功"的假绿）。
    assert_eq!(
        narrate(&mut reordered, "probe.trace"),
        ["trace=>silent>shared>util>handlers"],
        "reshape 之后模块执行序必须按新的 require 序变化；否则本用例失去意义"
    );
    // ② 关键断言：派发序一行都没动。
    assert_eq!(
        narrate(&mut reordered, "probe.order"),
        ORDER,
        "模块只做纯导出 ⇒ require 顺序是自由变量，拆文件/挪 require 都不改派发序"
    );
    // ③ 身份与执行次数也不受 require 序影响（缓存按规范名走，与谁先 require 无关）。
    assert_eq!(
        narrate(&mut reordered, "probe.identity"),
        ["util_same=true;shared_same=true;util_execs=1;shared_runs=1"]
    );
    assert_eq!(narrate(&mut reordered, "probe.stats"), [FRESH]);

    let _ = fs::remove_dir_all(&dir);
}

// ───────────────── ② 改 install 调用序 → 派发序跟着变（反过来钉） ─────────────────

#[test]
fn swapping_the_install_calls_moves_the_dispatch_order() {
    // 反向验证：如果上一条只是"派发序根本不看任何东西"的巧合，这一条会红。
    // 派发序的唯一来源就是入口里 `M.register()` 的调用序。
    let (dir, mut swapped) = fixture_with_rewritten_entry("pure-export-install", |source| {
        let mut lines: Vec<String> = source.lines().map(str::to_owned).collect();
        let util = line_index(&lines, "util.register()");
        let handlers = line_index(&lines, "handlers.register_order()");
        lines.swap(util, handlers);
        let mut text = lines.join("\n");
        if source.ends_with('\n') {
            text.push('\n');
        }
        text
    });

    // 对调 `util.register()` 与 `handlers.register_order()` ⇒ handlers 抢到派发链首位。
    assert_eq!(
        narrate(&mut swapped, "probe.order"),
        ["handlers", "shared", "util", "entry"],
        "派发序必须跟着入口的调用序走（这是唯一来源）"
    );
    // 而模块执行序纹丝未动：两个轴互相独立，各有唯一来源。
    assert_eq!(
        narrate(&mut swapped, "probe.trace"),
        [TRACE],
        "只改 install 调用序不该动模块执行序（那是 require 序的事）"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ───────────────── ③ 入口必须先 require 才有句柄 ─────────────────

/// 纯导出模块：`register` 只在入口调用时才注册 handler。
const UTIL: &str = r#"local M = {}

function M.register()
  ygg.on("probe", function()
    ygg.narrate("from-module")
  end)
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

    // ① 正确形态：require → 句柄 → install，handler 真的进了派发表。
    let mut kernel = KernelFacade::with_modules(OK_ENTRY, module_set(&[("src.util", UTIL)]))
        .expect("require + install 必须装载成功");
    assert_eq!(narrate(&mut kernel, "probe"), ["from-module"]);

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

    // ⑤ 只 require 不 install：装载成功，但派发面是空的——注册**只**由入口那
    //    一行决定，模块装载不会顺手把自己的 handler 塞进去（纯导出的直接后果）。
    let mut idle = KernelFacade::with_modules(
        "local util = require(\"src.util\")\n",
        module_set(&[("src.util", UTIL)]),
    )
    .expect("只 require 必须能装载");
    assert!(
        narrate(&mut idle, "probe").is_empty(),
        "没调用 M.register() 就不该有 handler（模块顶层不注册任何东西）"
    );
}

// ───────────────── ④ 懒加载：跨事件只执行一次 ─────────────────

/// 纯库模块：`execs` 是执行次数观测面（重复装载会立刻显形）。
const LAZY_LIB: &str = r#"execs = (execs or 0) + 1
return { tag = "lazy", execs = execs }
"#;

/// 两个**不同**事件各 require 一次同一个库：装载期不碰它，运行期缓存命中。
const LAZY_ENTRY: &str = r#"local seen = nil

local function lazy()
  local lib = require("src.lazy")
  local same = (seen == nil) or (seen == lib)
  seen = lib
  return lib, same
end

ygg.on("probe.at_load", function()
  ygg.narrate("execs_at_load=" .. tostring(execs))
end)

ygg.on("probe.one", function()
  local lib, same = lazy()
  ygg.narrate("tag=" .. lib.tag .. ";execs=" .. tostring(lib.execs) .. ";same=" .. tostring(same))
end)

ygg.on("probe.two", function()
  local lib, same = lazy()
  ygg.narrate("tag=" .. lib.tag .. ";execs=" .. tostring(lib.execs) .. ";same=" .. tostring(same))
end)
"#;

#[test]
fn a_library_required_lazily_runs_once_across_different_events() {
    let mut kernel = KernelFacade::with_modules(LAZY_ENTRY, module_set(&[("src.lazy", LAZY_LIB)]))
        .expect("懒加载入口必须装载成功");

    // 装载期没碰它：模块体的全局计数器还不存在。
    assert_eq!(
        narrate(&mut kernel, "probe.at_load"),
        ["execs_at_load=nil"],
        "装载期不得 require 这个库（懒加载才成立）"
    );
    // 事件 A：第一次 require → 执行一次。
    assert_eq!(
        narrate(&mut kernel, "probe.one"),
        ["tag=lazy;execs=1;same=true"]
    );
    // 事件 B（**另一个**事件）：缓存命中 → 不再执行，且拿到同一对象。
    // r1 的 `a_module_can_be_required_from_inside_a_handler` 只连发同一事件两次；
    // 这里换事件，防的是"缓存按事件分桶"这类看似合理的错误实现。
    assert_eq!(
        narrate(&mut kernel, "probe.two"),
        ["tag=lazy;execs=1;same=true"],
        "跨事件复用同一份缓存：不重复执行、对象同一"
    );
    // 再回事件 A 也一样（两条路径共用一个缓存）。
    assert_eq!(
        narrate(&mut kernel, "probe.one"),
        ["tag=lazy;execs=1;same=true"]
    );
}

// ───────────────── ⑤ 懒加载不放宽边界：handler 里 require 的模块也不许注册 ─────────────────

#[test]
fn a_module_required_from_a_handler_cannot_register_itself() {
    // r1 的 `a_lazy_module_is_still_subject_to_the_effect_boundary` 用 `ygg.narrate`
    // 证明"懒加载模块仍在边界内"；本用例补的是**这条性质的正主**：注册。
    // 否则「懒加载打通了 require」会被误读成「懒加载打通了注册」。
    let mut kernel = KernelFacade::with_modules(
        r#"ygg.on("probe.greedy", function()
  local ok, err = pcall(require, "src.greedy")
  ygg.narrate("ok=" .. tostring(ok))
  ygg.narrate("err=" .. tostring(err))
end)

ygg.on("probe.after", function()
  ygg.kv.set("after", "yes")
  ygg.narrate("after=" .. tostring(ygg.kv.get("after")))
end)
"#,
        module_set(&[(
            "src.greedy",
            "local M = {}\n\nygg.on(\"sneaky\", function() end)\n\nreturn M\n",
        )]),
    )
    .expect("入口本身不受副作用边界约束");

    let lines = narrate(&mut kernel, "probe.greedy");
    assert_eq!(
        lines[0], "ok=false",
        "懒加载模块顶层注册必须失败：{lines:?}"
    );
    let error = &lines[1];
    // 三件事都要说清：做了什么 + 在哪 + 该怎么做。
    assert!(
        error.contains("模块顶层不能调用 `ygg.on`"),
        "拒绝必须点名被禁的 API：{error}"
    );
    assert!(
        error.contains("src/greedy.lua:3"),
        "拒绝必须带模块自己的文件与行号：{error}"
    );
    assert!(
        error.contains("M.install(deps)"),
        "拒绝必须给出允许的出口：{error}"
    );

    // 影子表已恢复：同一次运行里 kv 与 narrate 都能用（泄漏的桩会在这里炸）。
    assert_eq!(narrate(&mut kernel, "probe.after"), ["after=yes"]);
    // 失败的模块不写缓存：再 require 一次仍是同一个可读拒绝（不会留下半个模块）。
    let again = narrate(&mut kernel, "probe.greedy");
    assert_eq!(again[0], "ok=false");
    assert!(
        again[1].contains("src/greedy.lua:3"),
        "重试必须是同一个可读拒绝：{again:?}"
    );
}

// ───────────────── ⑥ 旧写法必须装载期就红（而不是静默改派发序） ─────────────────

#[test]
fn the_old_style_module_cannot_sneak_back_in_and_reorder_dispatch() {
    // 用例 ① 的等号有前提：模块**不再**在顶层注册。这个前提不是靠自觉维持的——
    // 谁把某个模块改回改造前的写法，装载期就直接红，行为不会"悄悄变了"。
    // 这里改的是**中间层** `src/shared.lua`（不是入口、也不是第一个 require），
    // 证明拒绝覆盖整条依赖链，而不只是入口周边。
    let dir = copy_fixture("pure-export-old-style");
    let mut baseline = fixture_kernel();
    assert_eq!(narrate(&mut baseline, "probe.order"), ORDER);

    let path = dir.join("src/shared.lua");
    let mut lines: Vec<String> = fs::read_to_string(&path)
        .expect("读模块源码")
        .lines()
        .map(str::to_owned)
        .collect();
    // 插在模块**顶层**的 `return shared` 之前（列 0 才是顶层返回；模块内函数体
    // 里也有 `return`，插错了就成了死代码，测不到东西）。
    let return_at = lines
        .iter()
        .position(|line| line.starts_with("return "))
        .expect("模块顶层必须以 return 收尾");
    lines.insert(
        return_at,
        "ygg.on(\"probe.order\", function() ygg.narrate(\"sneaky\") end)".to_owned(),
    );
    let line = return_at + 1; // 1-based：报错里的行号
    fs::write(&path, format!("{}\n", lines.join("\n"))).expect("写模块源码");
    reseal(&dir); // 模块字节变了：内容寻址必须重算（打包工具职责）

    // `ModBundle::load` 只读盘与对账（零执行）；模块顶层注册在**构造内核**（执行入口
    // chunk）时才被影子表拦下。
    let bundle = ModBundle::load(&dir).expect("字节面合法：拒绝来自运行时边界，不是内容寻址");
    let err = match bundle.kernel() {
        Ok(_) => panic!("模块顶层注册必须让内核构造失败（否则旧写法可以静默改派发序）"),
        Err(err) => err,
    };
    assert!(
        matches!(err, KernelError::ModLoad(_)),
        "模块副作用拒绝必须经 ModLoad 浮出，实得：{err:?}"
    );
    let err = err.to_string();
    assert!(
        err.contains(&format!("src/shared.lua:{line}")),
        "拒绝必须指向模块自己那一行（{line}）：{err}"
    );
    assert!(
        err.contains("模块顶层不能调用 `ygg.on`"),
        "拒绝必须点名被禁的 API：{err}"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ───────────────── ⑦ 回归：无 code 段的 Mod 行为不变 ─────────────────

/// 仓库里剩下的单文件 Mod（`pack.json` 没有 `code` 段）。
/// 两套 agent-seed（`.ygg/mods/agent-seed` / `packages/mods/agent-seed`）已迁成多文件，
/// 不再是本用例的对象——它们的机制面由 `ygg-lint --pack` + playtest + 骨架守卫覆盖。
const SINGLE_FILE_MODS: [&str; 1] = ["packages/mods/test-adventure"];

#[test]
fn mods_without_a_code_segment_still_load_and_play_through() {
    // 本轮改造只动多文件装背面：单文件 Mod 的 `pack.json` 没有 `code` 段，
    // 模块表为空——它们必须逐字节照旧。这里每个包都真装载 + 真走查到【结局】，
    // 而不是只看"能不能解析"。
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for relative in SINGLE_FILE_MODS {
        let dir = root.join(relative);
        assert!(dir.is_dir(), "仓库 Mod 目录缺失：{}", dir.display());
        let pack = support::read_json(&dir.join("pack.json"));
        assert!(
            pack.get("code").is_none(),
            "{relative} 不该有 code 段（有的话它就不是单文件 Mod，本用例前提不成立）"
        );

        let bundle =
            ModBundle::load(&dir).unwrap_or_else(|err| panic!("{relative} 装载失败：{err}"));
        assert_eq!(bundle.modules.len(), 0, "{relative} 的模块表必须为空");
        assert!(
            bundle.mod_source.contains("ygg.on"),
            "{relative} 的入口必须是入口（顶层注册正是它该做的事）"
        );

        let report = playtest(&dir, &PlaytestOptions::default());
        assert!(
            report.ok && report.reached == Some(Reached::Ending),
            "{relative} 走查必须到【结局】：reached={:?} errors={:?}",
            report.reached,
            report.errors
        );
        assert!(report.errors.is_empty(), "{relative}：{:?}", report.errors);
        assert!(
            report.warnings.is_empty(),
            "{relative} 不该出现装载/派发告警：{:?}",
            report.warnings
        );
    }
}
