//! r1 副作用边界：**模块顶层只做纯导出**。
//!
//! 这一组盯的是一个不变量，而不是某条具体 API：模块 chunk 的顶层不得调用
//! 有副作用的 host API（状态写 / 输出）。理由在 `ygg_modules::effects` 的头注
//! 里——模块顶层写下的状态既不进世界帧、也不进存档，还会随 require 顺序变化。
//!
//! 内核侧的执行面是 `modules::ShadowBanned`（模块 chunk 执行期间把禁止面换成
//! 拒绝桩，`Drop` 恢复）。本文件逐条钉住它的四条性质：
//!
//! 1. 禁止面在模块顶层**一定**被拒，且报错带真实文件与行号；
//! 2. 纯读面**不**被拒（别把只读也一起封了）；
//! 3. 函数体内（`M.install` 里）调用**允许**——这是给作者的出口；
//! 4. chunk 抛错后影子表**一定**恢复（否则拒绝桩会残留、污染整个会话）。
//!
//! 入口 chunk 不受约束——本文件所有用例的观测都在入口顶部写进 KV，它们能
//! 装载起来本身就是这条豁免的证据（事件层已随信封层删除，观测面走 KV）。

use ygg_core::value::Value;
use ygg_sandbox::modload::{LoadError, Loaded, ModBundle};
use ygg_sandbox::modules::{effects, ModuleSet};
use ygg_schema::validate::sha256_hex;

mod support;

use support::{bundle_of, loaded_of};

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

/// 观测行（入口顶部写进 KV 的多行字符串，逐行拆开）。
fn narrate(loaded: &Loaded, topic: &str) -> Vec<String> {
    let kv = loaded.kv();
    let kv = kv.borrow();
    let Some(Value::Str(text)) = kv.get(topic) else {
        panic!("观测值缺失或不是字符串：{topic}");
    };
    text.split('\n').map(str::to_owned).collect()
}

/// 装载失败文本（必须落在入口 chunk 拒绝面——影子表的拒绝经模块 chunk 错误浮出）。
fn load_err_text(mod_source: &str, set: ModuleSet) -> String {
    match loaded_of(mod_source, set) {
        Ok(_) => panic!("expected the mod to fail loading"),
        Err(err) => {
            assert!(
                matches!(err, LoadError::ModChunk(_)),
                "模块副作用拒绝必须经入口 chunk 拒绝浮出，实得：{err:?}"
            );
            err.to_string()
        }
    }
}

// ───────────────────────── ① 禁止面在模块顶层被拒 ─────────────────────────

#[test]
fn a_module_that_writes_state_at_the_top_level_is_rejected_with_file_and_line() {
    let text = load_err_text(
        r#"require("src.handlers.release")"#,
        modules(&[(
            "src.handlers.release",
            "-- 模块只做纯导出\nlocal M = {}\n\nygg.kv.set(\"stage\", \"release\")\n\nreturn M\n",
        )]),
    );

    // 做了什么 + 在哪（chunk 名 = `@<包内相对路径>`，与装载器其它拒绝同形）。
    assert!(
        text.contains("src/handlers/release.lua:4"),
        "报错必须带真实文件与行号：{text}"
    );
    assert!(
        text.contains("模块顶层不能调用 `ygg.kv.set`"),
        "报错必须点名是哪个面：{text}"
    );
    // 该怎么做：模块纯导出 + 入口显式装配。
    assert!(text.contains("M.install(deps)"), "报错必须给出出口：{text}");
    // 为什么：状态写不进存档、还会随 require 顺序漂移 = 重构不安全。
    assert!(
        text.contains("既不进世界帧也不进存档") && text.contains("重构不再安全"),
        "报错必须解释原因：{text}"
    );
}

#[test]
fn every_banned_surface_is_rejected_from_a_module_top_level() {
    // 逐条覆盖 `ygg_modules::effects::BANNED_*`：清单改了而这里没跟上，
    // 下一条断言会直接指出漏了哪一项。
    let mut surfaces: Vec<(String, String)> = effects::BANNED_CALLS
        .iter()
        .map(|name| ((*name).to_owned(), (*name).to_owned()))
        .collect();
    surfaces.extend(
        effects::BANNED_TABLE_MEMBERS
            .iter()
            .map(|(table, member)| (format!("{table}.{member}"), format!("{table}.{member}"))),
    );
    // 登记面随事件层退役后，直接成员全空；只靠表成员承重。清单变空是设计变更，
    // 不是本用例失效——所以这里显式断言「至少还有一条」。
    assert!(
        !surfaces.is_empty(),
        "禁止面清单不能为空，否则影子表与 lint 一起失效"
    );

    for (call, dotted) in &surfaces {
        let call = call.replace("kv.set", "kv.set(\"k\", 1)");
        let source = format!("local M = {{}}\nygg.{call}\n\nreturn M\n");
        let text = load_err_text(
            r#"require("src.mod")"#,
            modules(&[("src.mod", source.as_str())]),
        );
        assert!(
            text.contains(&format!("模块顶层不能调用 `ygg.{dotted}`")),
            "`ygg.{call}` 必须被拒并点名 `ygg.{dotted}`：{text}"
        );
        assert!(
            text.contains("src/mod.lua:2"),
            "`ygg.{call}` 的报错必须指向模块自己那一行：{text}"
        );
        // 清单同源：报错点的名字必须能在 `effects` 里查出来。
        let path: Vec<&str> = dotted.split('.').collect();
        assert_eq!(
            effects::banned(&path).as_deref(),
            Some(dotted.as_str()),
            "`{dotted}` 必须在 ygg_modules::effects 的禁止面清单里"
        );
    }

    // 别名逃逸拿到的也是拒绝桩（影子表换的是表里的成员，不是某个局部）。
    let text = load_err_text(
        r#"require("src.mod")"#,
        modules(&[(
            "src.mod",
            "local M = {}\nlocal e = ygg.kv.set\ne(\"k\", 1)\n\nreturn M\n",
        )]),
    );
    assert!(
        text.contains("模块顶层不能调用 `ygg.kv.set`"),
        "别名逃逸也必须被拒：{text}"
    );
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
    let mut bundle: ModBundle = bundle_of(
        r#"
local probe = require("src.readonly")
ygg.kv.set("probe", "kv_absent=" .. tostring(probe.kv_absent)
  .. "\nresource=" .. probe.resource
  .. "\ndigest=" .. probe.digest)
"#,
        modules(&[(
            "src.readonly",
            // 顶层只读：KV miss、资源表、摘要。三者都不写状态、不产生输出。
            r#"local missing = ygg.kv.get("absent")
local loot = ygg.data("ygg:data/probe.loot")
return {
  kv_absent = (missing == nil),
  resource = loot.name,
  digest = ygg.codec.sha256("probe"),
}
"#,
        )]),
    );
    bundle.manifest_bytes = manifest().into_bytes();
    bundle.schemas = vec![(SCHEMA_ID.to_owned(), SCHEMA.to_owned())];
    bundle.resources = vec![(RES_PATH.to_owned(), RES_PAYLOAD.as_bytes().to_vec())];

    let loaded = Loaded::from_bundle(&bundle).expect("纯读模块必须能装载");
    assert_eq!(
        narrate(&loaded, "probe"),
        [
            "kv_absent=true".to_owned(),
            "resource=probe".to_owned(),
            format!("digest={}", sha256_hex(b"probe")),
        ]
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
fn a_module_may_write_state_when_the_entry_asks_it_to() {
    let loaded = loaded_of(
        r#"
local m = require("src.installable")
m.install()            -- 装配点：写状态由这一行决定，不由 require 序决定
ygg.kv.set("probe", "flag=" .. tostring(ygg.kv.get("installed")))
"#,
        modules(&[(
            "src.installable",
            r#"local M = {}

-- 函数体内调用禁止面是**允许**的：影子表只在 chunk 顶层生效，
-- 而这段代码由入口在装载期显式调用（运行时同理）。
function M.install()
  ygg.kv.set("installed", "yes")
end

return M
"#,
        )]),
    )
    .expect("M.install 形态必须能装载");

    // 模块在函数体里写下的状态真的进了 KV。
    assert_eq!(narrate(&loaded, "probe"), ["flag=yes"]);
}

// ───────────────────────── ④ Drop guard：拒绝后一定恢复 ─────────────────────────

#[test]
fn a_throwing_module_chunk_restores_the_shadowed_surface() {
    // 模块顶层写状态 → chunk 抛错；入口 pcall 掉它，然后自己照常写 KV。
    // 如果影子表没恢复，入口这一句 ygg.kv.set 会撞上残留的拒绝桩（装载期直接
    // 失败）——这条用例一次性盯住「错误可读 + 影子表已还原」。
    let loaded = loaded_of(
        r#"
local ok, err = pcall(require, "src.boom")
ygg.kv.set("probe", "ok=" .. tostring(ok) .. "\n" .. tostring(err))
ygg.kv.set("restored", "yes")
"#,
        modules(&[("src.boom", "local M = {}\nygg.kv.set(\"k\", 1)\nreturn M\n")]),
    )
    .expect("抛错的模块不能把整包装载拖垮（入口 pcall 得住）");

    let lines = narrate(&loaded, "probe");
    assert_eq!(lines[0], "ok=false", "模块顶层写状态必须失败：{lines:?}");
    assert!(
        lines[1].contains("模块顶层不能调用 `ygg.kv.set`"),
        "pcall 住的错误必须仍是可读的副作用拒绝：{lines:?}"
    );
    assert!(
        lines[1].contains("src/boom.lua:2"),
        "pcall 住的错误必须带模块自己的行号：{lines:?}"
    );
    assert_eq!(
        narrate(&loaded, "restored"),
        ["yes"],
        "影子表必须还原，否则入口自己的写会被拒绝桩挡住"
    );
}

// ───────────────────────── ⑤ 懒加载：模块代码里的 require ─────────────────────────

#[test]
fn a_module_can_be_required_lazily_from_another_module() {
    let loaded = loaded_of(
        r#"
local lib = require("src.host")
ygg.kv.set("probe", lib.report())
"#,
        modules(&[
            (
                "src.host",
                r#"local M = {}
function M.report()
  local lib = require("src.lazy")
  local again = require("src.lazy")
  return "tag=" .. lib.tag .. "\nexecs=" .. tostring(execs) .. ";same=" .. tostring(lib == again)
end
return M
"#,
            ),
            (
                "src.lazy",
                "execs = (execs or 0) + 1\nreturn { tag = \"lazy\" }\n",
            ),
        ]),
    )
    .expect("多文件 Mod 装载");

    // 装载期没 require 过它：第一次发生在模块自己的函数里（require 没有第二道闸）。
    assert_eq!(
        narrate(&loaded, "probe"),
        ["tag=lazy", "execs=1;same=true"],
        "模块内 require 正常工作，且同模块只执行一次、对象同一"
    );
}

#[test]
fn a_lazy_module_is_still_subject_to_the_effect_boundary() {
    // 懒加载放开的是**时机**，不是**约束**：任何模块被 require 到时，
    // 它的顶层照样是纯导出面。
    let loaded = loaded_of(
        r#"
local ok = pcall(require, "src.greedy")
ygg.kv.set("probe", "ok=" .. tostring(ok))
"#,
        modules(&[(
            "src.greedy",
            "local M = {}\nygg.kv.set(\"sneaky\", 1)\nreturn M\n",
        )]),
    )
    .expect("入口本身不受约束");

    assert_eq!(narrate(&loaded, "probe"), ["ok=false"]);
}
