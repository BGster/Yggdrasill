//! r4 正向面：多文件 Mod 的 require 契约端到端（`tests/fixtures/multi-script`）。
//!
//! 每个用例都从**入库 fixture**装载（不是内联字符串）：fixture 是三层依赖
//! `entry → src.handlers → src.util` + 一处重复 require + 一个 `return nil`
//! 模块的真跑包，装载期就把观测值写进 KV——「装载通过但装配没跑」这类假绿在这里
//! 无处藏身。
//!
//! 断言口径：**对象身份与执行次数**用模块自己留的痕（`util_execs` / `runs` /
//! `load_trace`）观测，不靠"看起来对"；观测值逐条比对，不用 `contains` 糊过去
//! （顺序本身是契约）。

mod support;

use std::fs;

use ygg_sandbox::modload::ModBundle;

use support::{declared_modules, fixture_dir, fixture_loaded, narrate};

#[test]
fn fixture_declares_four_modules_and_the_entry_is_not_one_of_them() {
    let bundle = ModBundle::load(&fixture_dir()).expect("fixture 必须可装载");
    assert_eq!(
        bundle.modules.len(),
        4,
        "code 段声明的四个模块必须全部进模块表：{:?}",
        bundle.modules.names().collect::<Vec<_>>()
    );
    // 入口由宿主作为 Mod 源读取，不登记为模块（同一份源码两个装载身份会让缓存
    // 与错误定位产生歧义）。
    assert!(
        !bundle.modules.names().any(|name| name == "mod"),
        "入口不得出现在模块表里：{:?}",
        bundle.modules.names().collect::<Vec<_>>()
    );
    assert!(
        bundle.mod_source.contains("require(\"src.handlers\")"),
        "入口取 code.entry（mod.lua）"
    );
}

#[test]
fn a_module_can_export_a_table_and_the_entry_registers_handlers_from_it() {
    // 正向 1：模块返回 table，入口能拿其中的函数做装配并调用。
    let kernel = fixture_loaded();
    assert_eq!(
        narrate(&kernel, "probe.table"),
        ["handler_from_table=util;bumped=2"],
        "入口经 src.handlers 取到 src.util 表里的函数并调用"
    );
    // 模块内状态真的被写动（不是把常量抄进 handler）。
    assert_eq!(narrate(&kernel, "probe.bump"), ["util_count=5"]);
}

#[test]
fn three_layer_dependencies_run_in_a_deterministic_order() {
    // 正向 2：entry → src.handlers → src.util（另含 src.shared / src.silent）都执行，
    // 且执行序由依赖图决定。
    let kernel = fixture_loaded();
    assert_eq!(
        narrate(&kernel, "probe.trace"),
        ["trace=>handlers>util>shared>silent"],
        "模块体执行序：handlers 先进入，再是它 require 的两个叶子，最后是入口的 silent"
    );

    // 声明序不是装载序：pack.json 把 src/util.lua 排在第一位，实际最先执行的是
    // src.handlers（它是入口的第一个 require）。装载序来自 require 图，不来自
    // 声明数组的排列——这条断言就是"依赖图遍历不得用无序容器"的正向锚。
    let declared = declared_modules();
    assert_eq!(
        declared.first().map(String::as_str),
        Some("src/util.lua"),
        "fixture 的声明序（{declared:?}）必须与执行序不同，否则本用例失去意义"
    );
    let trace = narrate(&kernel, "probe.trace").remove(0);
    let first_executed = trace.trim_start_matches("trace=>").split('>').next();
    assert_eq!(first_executed, Some("handlers"), "实际首跳：{trace}");
}

#[test]
fn one_module_required_from_two_sites_runs_once_and_keeps_table_identity() {
    // 正向 3：入口与 src.handlers 分别 require src.util / src.shared——只执行一次，
    // 且返回同一个 table 对象（table 身份，不是"内容相等"）。
    let kernel = fixture_loaded();
    assert_eq!(
        narrate(&kernel, "probe.identity"),
        ["util_same=true;shared_same=true;util_execs=1;shared_runs=1"],
        "三处 require 同一模块：执行一次、身份同一"
    );
}

#[test]
fn a_nil_module_is_cached_as_true_and_never_runs_twice() {
    // 正向 4：`return nil` 的模块只执行一次（缓存 `true`），重复 require 不重跑。
    let kernel = fixture_loaded();
    assert_eq!(
        narrate(&kernel, "probe.nil"),
        ["silent=true;again=true;same=true;runs=1"]
    );
}

#[test]
fn the_same_mod_loads_the_same_way_from_a_copied_directory() {
    // 装载结果与目录位置无关（模块表按声明序读、按名字查表；无路径拼接面）。
    let original = ModBundle::load(&fixture_dir()).expect("原目录装载");
    let copy = support::copy_fixture("e2e-copy");
    let moved = ModBundle::load(&copy).expect("副本装载");
    assert_eq!(
        original.modules.names().collect::<Vec<_>>(),
        moved.modules.names().collect::<Vec<_>>(),
        "模块集合与位置无关"
    );
    assert_eq!(original.mod_source, moved.mod_source, "入口源码与位置无关");
    let _ = fs::remove_dir_all(&copy);
}
