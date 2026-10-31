//! r1 副作用边界：**模块顶层只做纯导出**。
//!
//! 这一组盯的是一个不变量，而不是某条具体 API：模块 chunk 的顶层不得调用
//! 有副作用的 host API（注册 / 状态写 / 输出）。理由在 `ygg_modules::effects`
//! 的头注里——`ygg.on` 的注册序就是 handler 派发序，模块顶层能自由注册就等于
//! 「文件怎么拆」决定「运行时行为」。
//!
//! 内核侧的执行面是 `modules::ShadowBanned`（模块 chunk 执行期间把禁止面换成
//! 拒绝桩，`Drop` 恢复）。本文件逐条钉住它的四条性质：
//!
//! 1. 禁止面在模块顶层**一定**被拒，且报错带真实文件与行号；
//! 2. 纯读面**不**被拒（别把只读也一起封了）；
//! 3. 函数体内（`M.install` 里）调用**允许**——这是给作者的出口；
//! 4. chunk 抛错后影子表**一定**恢复（否则拒绝桩会残留、污染整个会话）。
//!
//! 外加两条 r1 打通的能力：handler 内 `require`（懒加载）与「拒绝不污染后续
//! 模块」。入口 chunk 不受约束——本文件所有用例的入口都在顶层调 `ygg.on`，
//! 它们能装载起来本身就是这条豁免的证据。

use serde_json::{json, Value as Json};
use ygg_sandbox::driver::{KernelError, KernelFacade};
use ygg_sandbox::modules::{effects, ModuleSet};
use ygg_schema::validate::sha256_hex;

// ───────────────────────── 脚手架 ─────────────────────────

/// 由 `(名字, 源码)` 构造模块表（测试便捷面）。
fn modules(pairs: &[(&str, &str)]) -> ModuleSet {
    ModuleSet::from_pairs(
        pairs
            .iter()
            .map(|(name, code)| ((*name).to_owned(), code.as_bytes().to_vec())),
    )
    .expect("module set")
}

fn parse(reply: &str) -> Json {
    serde_json::from_str(reply).expect("json envelope")
}

fn reply_ok(reply: &str) -> Json {
    let parsed = parse(reply);
    assert_eq!(parsed["ok"], json!(true), "expected ok reply: {reply}");
    parsed
}

/// 叙述行（每条 handler `ygg.narrate` 一行；顺序本身是契约）。
fn narrate(kernel: &mut KernelFacade, event: &str) -> Vec<String> {
    let parsed = reply_ok(&kernel.dispatch(event, "0"));
    parsed["narrative"]
        .as_array()
        .expect("narrative array")
        .iter()
        .map(|line| line.as_str().expect("narrative line").to_owned())
        .collect()
}

/// 装载失败文本（必须落在 `ModLoad` 面——影子表的拒绝经模块 chunk 错误浮出）。
fn load_err_text(mod_source: &str, set: ModuleSet) -> String {
    match KernelFacade::with_modules(mod_source, set) {
        Ok(_) => panic!("expected the mod to fail loading"),
        Err(err) => {
            assert!(
                matches!(err, KernelError::ModLoad(_)),
                "模块副作用拒绝必须经 ModLoad 浮出，实得：{err:?}"
            );
            err.to_string()
        }
    }
}

// ───────────────────────── ① 禁止面在模块顶层被拒 ─────────────────────────

#[test]
fn a_module_that_registers_at_the_top_level_is_rejected_with_file_and_line() {
    let text = load_err_text(
        r#"require("src.handlers.release")"#,
        modules(&[(
            "src.handlers.release",
            "-- 模块只做纯导出\nlocal M = {}\n\nygg.on(\"event.x\", function() end)\n\nreturn M\n",
        )]),
    );

    // 做了什么 + 在哪（chunk 名 = `@<包内相对路径>`，与装载器其它拒绝同形）。
    assert!(
        text.contains("src/handlers/release.lua:4"),
        "报错必须带真实文件与行号：{text}"
    );
    assert!(
        text.contains("模块顶层不能调用 `ygg.on`"),
        "报错必须点名是哪个面：{text}"
    );
    // 该怎么做：模块纯导出 + 入口显式装配。
    assert!(text.contains("M.install(deps)"), "报错必须给出出口：{text}");
    // 为什么：派发序由文件组织决定 = 重构不安全。
    assert!(
        text.contains("派发序") && text.contains("重构就不再安全"),
        "报错必须解释原因：{text}"
    );
    // 指向作者手册的多文件章节（§5 多文件 Mod）。
    assert!(
        text.contains("docs/mod-authoring.md"),
        "报错必须指向文档：{text}"
    );
}

#[test]
fn every_banned_surface_is_rejected_from_a_module_top_level() {
    // 逐条覆盖 `ygg_modules::effects::BANNED_*`：清单改了而这里没跟上，
    // 下一条断言会直接指出漏了哪一项。
    for (call, dotted) in [
        (r#"ygg.on("event.x", function() end)"#, "on"),
        (r#"ygg.emit("event.x", 1)"#, "emit"),
        (r#"ygg.narrate("line")"#, "narrate"),
        (r#"ygg.choice("prompt", { "a", "b" })"#, "choice"),
        (r#"ygg.meta("k", 1)"#, "meta"),
        (r#"ygg.kv.set("k", 1)"#, "kv.set"),
        (r#"ygg.entity.spawn()"#, "entity.spawn"),
        (
            r#"ygg.entity.set_attribute(1, "hp", 1)"#,
            "entity.set_attribute",
        ),
        (r#"ygg.entity.add_tag(1, "t")"#, "entity.add_tag"),
        (r#"ygg.entity.remove_tag(1, "t")"#, "entity.remove_tag"),
        (
            r#"ygg.fsm.define({ name = "m", initial = "a", event = "e", transitions = {} })"#,
            "fsm.define",
        ),
        (
            r#"ygg.narrative.node("n", function() end)"#,
            "narrative.node",
        ),
        // `goto` 是 Lua 5.4 保留字：只可写括号形态（点号是语法错误）。
        (r#"ygg.narrative["goto"]("n")"#, "narrative.goto"),
        // 别名逃逸拿到的也是拒绝桩（影子表换的是表里的成员，不是某个局部）。
        (r#"local e = ygg.emit; e("event.x", 1)"#, "emit"),
    ] {
        let source = format!("local M = {{}}\n{call}\n\nreturn M\n");
        let text = load_err_text(
            r#"require("src.mod")"#,
            modules(&[("src.mod", source.as_str())]),
        );
        assert!(
            text.contains(&format!("模块顶层不能调用 `ygg.{dotted}`")),
            "`{call}` 必须被拒并点名 `ygg.{dotted}`：{text}"
        );
        assert!(
            text.contains("src/mod.lua:2"),
            "`{call}` 的报错必须指向模块自己那一行：{text}"
        );
        // 清单同源：报错点的名字必须能在 `effects` 里查出来。
        let path: Vec<&str> = dotted.split('.').collect();
        assert_eq!(
            effects::banned(&path).as_deref(),
            Some(dotted),
            "`{dotted}` 必须在 ygg_modules::effects 的禁止面清单里"
        );
    }
}

// ───────────────────────── ② 纯读面放行 ─────────────────────────

/// 资源包脚手架（只为一个目的：让 `ygg.data` **真实存在**——不装资源时它是
/// nil，那样「顶层读 ygg.data 不被拒」就退化成「nil 不可调用」，测不出东西）。
const SCHEMA_ID: &str = "https://yggdrasill.dev/schemas/v1/loot.json";
const SCHEMA: &str = r#"{
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "$id": "https://yggdrasill.dev/schemas/v1/loot.json",
    "type": "object",
    "additionalProperties": false,
    "required": ["name"],
    "properties": { "name": { "type": "string" } }
}"#;
const RES_ID: &str = "ygg:data/probe.loot";
const RES_PATH: &str = "data/probe.loot.json";
const RES_PAYLOAD: &str = r#"{"name":"probe"}"#;

fn manifest() -> String {
    let bytes = RES_PAYLOAD.as_bytes();
    format!(
        r#"{{"schemaVersion": 1, "entries": [{{"id": "{RES_ID}", "kind": "loot", "schema": "{SCHEMA_ID}", "path": "{RES_PATH}", "sha256": "{}", "byteLength": {}}}]}}"#,
        sha256_hex(bytes),
        bytes.len()
    )
}

#[test]
fn module_top_level_is_allowed_to_read() {
    let kernel = KernelFacade::with_resources_and_modules(
        r#"
local probe = require("src.readonly")
ygg.on("probe", function()
  ygg.narrate("kv_absent=" .. tostring(probe.kv_absent))
  ygg.narrate("resource=" .. probe.resource)
  ygg.narrate("ticks=" .. tostring(probe.ticks_ok))
end)
"#,
        modules(&[(
            "src.readonly",
            // 顶层只读：KV miss、资源表、时钟。三者都不写状态、不产生输出。
            r#"local missing = ygg.kv.get("absent")
local loot = ygg.data("ygg:data/probe.loot")
local ticks = ygg.ticks()
return {
  kv_absent = (missing == nil),
  resource = loot.name,
  ticks_ok = (type(ticks) == "number"),
}
"#,
        )]),
        manifest().as_bytes(),
        &[(SCHEMA_ID.to_owned(), SCHEMA.to_owned())],
        &[(RES_PATH.to_owned(), RES_PAYLOAD.as_bytes().to_vec())],
    )
    .expect("纯读模块必须能装载");

    let mut kernel = kernel;
    assert_eq!(
        narrate(&mut kernel, "probe"),
        ["kv_absent=true", "resource=probe", "ticks=true"]
    );
}

#[test]
fn the_banned_list_never_contains_a_pure_read() {
    // 单一事实源的自洽性：两条清单不得重叠（否则 lint 会在「允许」与「禁止」
    // 之间给出两个互相矛盾的判定）。
    for name in effects::PURE_READ_CALLS {
        assert_eq!(
            effects::banned(&[name]),
            None,
            "纯读 `ygg.{name}` 不得出现在禁止面里"
        );
    }
    for (table, member) in effects::PURE_READ_TABLE_MEMBERS {
        assert_eq!(
            effects::banned(&[table, member]),
            None,
            "纯读 `ygg.{table}.{member}` 不得出现在禁止面里"
        );
    }
    // 禁止面反查：清单里的每一条都必须能被查询面命中（拼错名字会在这里露）。
    for name in effects::BANNED_CALLS {
        assert_eq!(effects::banned(&[name]).as_deref(), Some(*name));
    }
    for (table, member) in effects::BANNED_TABLE_MEMBERS {
        assert_eq!(
            effects::banned(&[table, member]).as_deref(),
            Some(format!("{table}.{member}").as_str())
        );
    }
}

// ───────────────────────── ③ 函数体内调用允许 ─────────────────────────

#[test]
fn a_module_may_install_into_the_host_when_the_entry_asks_it_to() {
    let mut kernel = KernelFacade::with_modules(
        r#"
local m = require("src.installable")
m.install()            -- 装配点：派发序由这一行决定，不由 require 序决定
ygg.on("readback", function()
  ygg.narrate("flag=" .. tostring(ygg.kv.get("installed")))
end)
"#,
        modules(&[(
            "src.installable",
            r#"local M = {}

-- 函数体内调用禁止面是**允许**的：影子表只在 chunk 顶层生效，
-- 而这段代码由入口在装载期显式调用（运行时同理）。
function M.install()
  ygg.kv.set("installed", "yes")
  ygg.on("installed", function()
    ygg.narrate("installed-handler")
  end)
end

return M
"#,
        )]),
    )
    .expect("M.install 形态必须能装载");

    // 模块在函数体里注册的 handler 真的进了派发表。
    assert_eq!(narrate(&mut kernel, "installed"), ["installed-handler"]);
    assert_eq!(narrate(&mut kernel, "readback"), ["flag=yes"]);
}

// ───────────────────────── ④ Drop guard：拒绝后一定恢复 ─────────────────────────

#[test]
fn a_throwing_module_chunk_restores_the_shadowed_surface() {
    // 模块顶层写状态 → chunk 抛错；入口 pcall 掉它，然后自己照常注册。
    // 如果影子表没恢复，第二条 ygg.on 会撞上残留的拒绝桩（装载期直接失败），
    // 而 handler 里的 ygg.kv.set / ygg.narrate 也会炸——这条用例一次性盯住。
    let mut kernel = KernelFacade::with_modules(
        r#"
local ok, err = pcall(require, "src.boom")
ygg.on("probe", function()
  ygg.kv.set("restored", "yes")
  ygg.narrate("ok=" .. tostring(ok))
  ygg.narrate(tostring(err))
end)
ygg.on("readback", function()
  ygg.narrate("restored=" .. tostring(ygg.kv.get("restored")))
end)
"#,
        modules(&[("src.boom", "local M = {}\nygg.kv.set(\"k\", 1)\nreturn M\n")]),
    )
    .expect("抛错的模块不能把整包装载拖垮（入口 pcall 得住）");

    let lines = narrate(&mut kernel, "probe");
    assert_eq!(lines[0], "ok=false", "模块顶层写状态必须失败：{lines:?}");
    assert!(
        lines[1].contains("模块顶层不能调用 `ygg.kv.set`"),
        "pcall 住的错误必须仍是可读的副作用拒绝：{lines:?}"
    );
    assert!(
        lines[1].contains("src/boom.lua:2"),
        "pcall 住的错误必须带模块自己的行号：{lines:?}"
    );
    // 真恢复的三重证据：handler 里的 kv.set 写进去了、narrate 出得来、
    // 后面还能再注册 handler（上面两条 ygg.on 已经成立）。
    assert_eq!(narrate(&mut kernel, "readback"), ["restored=yes"]);
}

// ───────────────────────── ⑤ 懒加载：handler 内 require ─────────────────────────

#[test]
fn a_module_can_be_required_from_inside_a_handler() {
    let mut kernel = KernelFacade::with_modules(
        r#"
ygg.on("lazy", function()
  local lib = require("src.lazy")
  local again = require("src.lazy")
  ygg.narrate("tag=" .. lib.tag)
  ygg.narrate("execs=" .. tostring(execs) .. ";same=" .. tostring(lib == again))
end)
"#,
        modules(&[(
            "src.lazy",
            "execs = (execs or 0) + 1\nreturn { tag = \"lazy\" }\n",
        )]),
    )
    .expect("多文件 Mod 装载");

    // 装载期没 require 过它：第一次发生在 handler 里（require 不再有装载期边界）。
    assert_eq!(
        narrate(&mut kernel, "lazy"),
        ["tag=lazy", "execs=1;same=true"],
        "handler 内 require 正常工作，且同模块只执行一次、对象同一"
    );
    // 再触发一次也还是同一份缓存（不是每次都重新装载）。
    assert_eq!(
        narrate(&mut kernel, "lazy"),
        ["tag=lazy", "execs=1;same=true"],
        "跨事件复用缓存，不重复执行"
    );
}

#[test]
fn a_lazy_module_is_still_subject_to_the_effect_boundary() {
    // 懒加载放开的是**时机**，不是**约束**：handler 里 require 到的模块，
    // 顶层照样是纯导出面。
    let mut kernel = KernelFacade::with_modules(
        r#"
ygg.on("lazy", function()
  local ok = pcall(require, "src.greedy")
  ygg.narrate("ok=" .. tostring(ok))
end)
"#,
        modules(&[(
            "src.greedy",
            "local M = {}\nygg.narrate(\"sneaky\")\nreturn M\n",
        )]),
    )
    .expect("入口本身不受约束");

    // handler 里 pcall 住 BadModule 的 chunk 错误 → dispatch 成功、ok=false。
    // （handler 报错会变成 VALIDATION；这里不报错说明拒绝是 chunk 级的。）
    assert_eq!(narrate(&mut kernel, "lazy"), ["ok=false"]);
}
