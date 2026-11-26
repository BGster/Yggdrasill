//! M1-C `world.*` 面的 Lua 端 e2e：组件声明 / 增删改查 / 迭代序与实参序 / 系统声明与
//! `seal` 上浮 / 随机流 / kv 复用 / 无浮点闸门。
//!
//! 断言都以 **Lua 侧可见的东西**为准（`pcall` 捕获的 message 首段、`world.get` 读回的值、
//! `each` 收到的实参），而不是 Rust 侧的中间变量——「Rust 面能跑」与「Lua 面能用」是
//! 两件事，这里只证明后者。
//!
//! 每个用例都**真的装宿主面**（`host_api::install` + `set_sim`）：`world.*` 的重入/未挂载
//! 分支只有在真句柄下才走得到。

use std::cell::RefCell;
use std::rc::Rc;

use mlua::Table;
use ygg_core::kv::KvStore;
use ygg_sandbox::host_api::{self, HostApi, HostHandles};
use ygg_sandbox::sandbox::Sandbox;
use ygg_sim::sim::Sim;

/// 装一个「真宿主面 + 已挂载内核」的沙箱（不装 Mod 包：`world` 面与包无关）。
///
/// M1-D 之后**时间只由宿主推**（`Sandbox::step`）：Lua 侧没有任何推进时钟的
/// 入口（`ygg` 的 `sim.*` 面已删），所以每个用例都是「跑配置脚本 → 宿主 step →
/// 跑断言脚本」三段。
fn fixture(seed: u64) -> (Sandbox, HostApi) {
    let sandbox = Sandbox::new().expect("沙箱构造");
    let api = host_api::install(
        sandbox.lua(),
        HostHandles::new(Rc::new(RefCell::new(KvStore::new()))),
        sandbox.sim_handle(),
    )
    .expect("host API 必须可装");
    api.set_sim(Some(Box::new(Sim::new(seed))));
    (sandbox, api)
}

/// 跑一段 Lua 并取回它的表（脚本自身必须以 `return { ... }` 收尾）。
fn run(sandbox: &Sandbox, script: &str) -> Table {
    sandbox
        .lua()
        .load(script)
        .eval::<Table>()
        .expect("Lua 脚本必须跑通")
}

fn text(table: &Table, key: &str) -> String {
    table.get::<String>(key).expect("字段必须是字符串")
}

fn int(table: &Table, key: &str) -> i64 {
    table.get::<i64>(key).expect("字段必须是整数")
}

/// Lua 侧共用前缀：`probe` 把 `pcall` 捕获的错误截成首行（多行是 traceback），
/// `join` 手写拼接（本文件不装 `inject`，所以 `math` 仍是 `nil`；`string` / `table`
/// 自 M2 起已是沙箱全局，但这里的脚手架保持手写——它要在「有无库表」两种沙箱下
/// 都跑得动）。
const HELPERS: &str = r#"
local function first_line(text)
  local line = tostring(text)
  local cut = line:find("\n", 1, true)
  if cut ~= nil then
    line = line:sub(1, cut - 1)
  end
  return line
end

local function probe(body)
  local ok, err = pcall(body)
  if ok then
    return "ok"
  end
  return first_line(err)
end

local function join(items, sep)
  local out = ""
  for i = 1, #items do
    if i > 1 then
      out = out .. sep
    end
    out = out .. tostring(items[i])
  end
  return out
end
"#;

// ---------------------------------------------------------------------------
// 迭代：序与实参序
// ---------------------------------------------------------------------------

/// 同种子同脚本两次运行 ⇒ `each` 的访问序逐项相同（且就是 spawn 序）。
///
/// 这是本面的**确定序契约**：序不来自 Lua 的表遍历（`pairs` 无序），而来自内核
/// `for_each_mut`（桶下标升序 → 行号升序）。
#[test]
fn spawn_then_each_order_is_stable_across_runs() {
    let script = format!(
        r#"{HELPERS}
world.component{{ name = "hp", kind = "i64" }}
local ids = {{}}
for i = 1, 3 do
  ids[i] = world.spawn{{ hp = i * 10 }}
end
local seen = {{}}
world.each({{ "hp" }}, function(id, hp)
  seen[#seen + 1] = id .. ":" .. hp
end)
local listed = {{}}
for _, id in ipairs(world.ids({{ "hp" }})) do
  listed[#listed + 1] = tostring(id)
end
return {{
  ids = join(ids, " "),
  order = join(seen, " "),
  listed = join(listed, " "),
  rows = #seen,
}}
"#
    );
    let (one, _a) = fixture(11);
    let (two, _b) = fixture(11);
    let first = run(&one, &script);
    let second = run(&two, &script);

    for key in ["ids", "order", "listed"] {
        assert_eq!(
            text(&first, key),
            text(&second, key),
            "`{key}` 两次运行的序必须逐项相同"
        );
    }
    assert_eq!(int(&first, "rows"), int(&second, "rows"));
    assert_eq!(text(&first, "ids"), "1 2 3", "id 从 1 起、按分配序");
    assert_eq!(
        text(&first, "order"),
        "1:10 2:20 3:30",
        "each 的序 = spawn 序"
    );
    assert_eq!(
        text(&first, "listed"),
        "1 2 3",
        "ids 的序 = each 的序（同一个遍历面）"
    );
    assert_eq!(int(&first, "rows"), 3);
}

/// `each` 按**调用方声明的顺序**逐个传值（多个返回值，不是一行一张表）。
#[test]
fn each_passes_values_in_declared_order() {
    let script = format!(
        r#"{HELPERS}
world.component{{ name = "hp", kind = "i64" }}
world.component{{ name = "atk", kind = "i64" }}
local id = world.spawn{{ hp = 7, atk = 3 }}
local out = {{}}
world.each({{ "atk", "hp" }}, function(e, atk, hp)
  out[#out + 1] = "atk-first:" .. e .. ":" .. atk .. ":" .. hp
end)
local rev = {{}}
world.each({{ "hp", "atk" }}, function(e, hp, atk)
  rev[#rev + 1] = "hp-first:" .. e .. ":" .. hp .. ":" .. atk
end)
return {{ id = id, forward = join(out, " "), reverse = join(rev, " ") }}
"#
    );
    let (sandbox, _api) = fixture(7);
    let out = run(&sandbox, &script);
    assert_eq!(int(&out, "id"), 1);
    assert_eq!(text(&out, "forward"), "atk-first:1:3:7", "实参序 = 声明序");
    assert_eq!(
        text(&out, "reverse"),
        "hp-first:1:7:3",
        "换一个声明序，实参跟着换"
    );
}

// ---------------------------------------------------------------------------
// 未知组件名：每一条入口都点名报错（不是 panic，不是静默空集）
// ---------------------------------------------------------------------------

#[test]
fn unknown_component_name_errors_on_every_entry() {
    let script = format!(
        r#"{HELPERS}
world.component{{ name = "hp", kind = "i64" }}
local id = world.spawn{{ hp = 1 }}
return {{
  spawn = probe(function() world.spawn{{ nope = 1 }} end),
  insert = probe(function() world.insert(id, {{ nope = 1 }}) end),
  set = probe(function() world.set(id, {{ nope = 1 }}) end),
  get = probe(function() return world.get(id, "nope") end),
  remove = probe(function() world.remove(id, {{ "nope" }}) end),
  each = probe(function() world.each({{ "nope" }}, function() end) end),
  each_mut = probe(function() world.each_mut({{ "nope" }}, function() end) end),
  ids = probe(function() return world.ids({{ "nope" }}) end),
  system = probe(function()
    world.system{{ name = "s", stage = "rules", reads = {{ "nope" }}, run = function() end }}
  end),
}}
"#
    );
    let (sandbox, _api) = fixture(1);
    let out = run(&sandbox, &script);
    for key in [
        "spawn", "insert", "set", "get", "remove", "each", "each_mut", "ids", "system",
    ] {
        let message = text(&out, key);
        assert!(
            message.contains("YGG-SIM-SCHEMA:"),
            "`{key}` 必须带内核码，得到 {message}"
        );
        assert!(
            message.contains("nope"),
            "`{key}` 必须点名组件，得到 {message}"
        );
    }
}

// ---------------------------------------------------------------------------
// 系统声明与 seal
// ---------------------------------------------------------------------------

/// 同阶段两个写者 ⇒ `seal` 拒绝，且消息点名**两个**系统。
#[test]
fn seal_rejects_write_write_in_one_stage() {
    let script = format!(
        r#"{HELPERS}
world.component{{ name = "hp", kind = "i64" }}
world.system{{ name = "alpha", stage = "rules", writes = {{ "hp" }}, run = function() end }}
world.system{{ name = "beta", stage = "rules", writes = {{ "hp" }}, run = function() end }}
return {{ seal = probe(function() return world.seal() end) }}
"#
    );
    let (sandbox, _api) = fixture(1);
    let seal = text(&run(&sandbox, &script), "seal");
    assert!(seal.contains("YGG-SIM-SEALED:"), "得到 {seal}");
    assert!(
        seal.contains("alpha") && seal.contains("beta"),
        "得到 {seal}"
    );
}

/// 读写同一组件是**合法**的（`reads ∩ writes` 不是冲突，冲突只判同阶段写写）；
/// 封盘后系统体真的跑，`ctx.each_mut` 的写回落进世界，`dt` 原样漏传。
///
/// `dt` 是 Q32.32 裸值（宿主 `step(1, 1)` 的「整数秒 1」换出来的同一个值），
/// 体里不拿 `dt` 做组件算术（单位不同），只把它写进 kv 供断言——这条注释就是
/// 单位契约的回归钉。
#[test]
fn seal_accepts_read_write_and_system_body_runs() {
    let setup = format!(
        r#"{HELPERS}
world.component{{ name = "hp", kind = "i64" }}
world.system{{
  name = "damage",
  stage = "rules",
  reads = {{ "hp" }},
  writes = {{ "hp" }},
  run = function(dt, ctx)
    ygg.kv.set("dt", dt)
    ygg.kv.set("tick", ctx.tick())
    ctx.each_mut({{ "hp" }}, function(id, hp) return hp - 1 end)
  end,
}}
local seal = probe(function() return world.seal() end)
local id = world.spawn{{ hp = 5 }}
ygg.kv.set("id", id)
return {{ seal = seal }}
"#
    );
    let probe = format!(
        r#"{HELPERS}
return {{
  after = world.get(ygg.kv.get("id"), "hp"),
  tick = world.tick(),
  seen_dt = ygg.kv.get("dt"),
  seen_tick = ygg.kv.get("tick"),
}}
"#
    );
    let (mut sandbox, _api) = fixture(1);
    let out = run(&sandbox, &setup);
    assert_eq!(text(&out, "seal"), "ok");
    let report = sandbox.step(1, 1).expect("宿主推进必须成功");
    assert_eq!(report.tick, 1, "推进一 tick 后内核时钟是 1");
    assert_eq!(
        sandbox.tick().expect("世界已挂载"),
        report.tick,
        "推进后的时钟与步进回执同源"
    );
    let out = run(&sandbox, &probe);
    assert_eq!(int(&out, "after"), 4, "ctx.each_mut 的写回必须落进世界");
    assert_eq!(int(&out, "tick"), 1, "world.tick 与内核 tick 同源");
    assert_eq!(
        int(&out, "seen_dt"),
        4_294_967_296,
        "dt 是 Q32.32 裸值，原样进系统体"
    );
    assert_eq!(int(&out, "seen_tick"), 0, "首 tick 执行时 tick 形参是 0");
}

/// `seal` 之后再声明系统 ⇒ `YGG-SIM-SEALED`（内核 `SystemSet::add` 的拒绝原样上浮）。
#[test]
fn system_after_seal_errors() {
    let script = format!(
        r#"{HELPERS}
world.component{{ name = "hp", kind = "i64" }}
world.system{{ name = "alpha", stage = "rules", writes = {{ "hp" }}, run = function() end }}
local seal = probe(function() return world.seal() end)
local late = probe(function()
  world.system{{ name = "late", stage = "rules", writes = {{ "hp" }}, run = function() end }}
end)
return {{ seal = seal, late = late }}
"#
    );
    let (sandbox, _api) = fixture(1);
    let out = run(&sandbox, &script);
    assert_eq!(text(&out, "seal"), "ok");
    let late = text(&out, "late");
    assert!(late.contains("YGG-SIM-SEALED:"), "得到 {late}");
}

/// 系统体内的 Lua 错误以 `YGG-SIM-STATE: 系统 `名` 执行失败：…` 上浮（点名系统）。
///
/// 错误从**宿主** `step` 出来（不是 Lua 侧）：这就是「宿主拿得到失败、脚本拿不到
/// 时钟」的直接体现——脚本没有推进入口，也就没有「把推进错误吞掉」的自由。
#[test]
fn system_body_error_names_the_system() {
    let script = format!(
        r#"{HELPERS}
world.component{{ name = "hp", kind = "i64" }}
world.system{{
  name = "boom",
  stage = "rules",
  writes = {{ "hp" }},
  run = function(dt, ctx) error("炸了") end,
}}
world.seal()
return {{ seal = "ok" }}
"#
    );
    let (mut sandbox, _api) = fixture(1);
    assert_eq!(text(&run(&sandbox, &script), "seal"), "ok");
    let err = sandbox.step(1, 1).expect_err("系统体炸了，推进必须报错");
    let text = err.to_string();
    assert!(text.contains("YGG-SIM-STATE:"), "得到 {text}");
    assert!(text.contains("boom"), "必须点名系统，得到 {text}");
}

/// 系统体里再调 `world.*` ⇒ `YGG-WORLD-BUSY`（内核槽已被 `step` 独占），不是 panic。
#[test]
fn world_reentry_inside_system_is_busy() {
    let setup = format!(
        r#"{HELPERS}
world.component{{ name = "hp", kind = "i64" }}
world.system{{
  name = "reenter",
  stage = "rules",
  writes = {{ "hp" }},
  run = function(dt, ctx)
    local ok, err = pcall(function() return world.tick() end)
    ygg.kv.set("inner", ok and "ok" or first_line(err))
  end,
}}
world.seal()
world.spawn{{ hp = 1 }}
return {{ seal = "ok" }}
"#
    );
    let (mut sandbox, _api) = fixture(1);
    assert_eq!(text(&run(&sandbox, &setup), "seal"), "ok");
    sandbox
        .step(1, 1)
        .expect("系统体自己吞了错误，推进本身必须成功");
    let inner = text(
        &run(&sandbox, "return { inner = ygg.kv.get('inner') }"),
        "inner",
    );
    assert!(inner.contains("YGG-WORLD-BUSY:"), "得到 {inner}");
}

// ---------------------------------------------------------------------------
// 随机流与 kv：都复用内核/宿主的既有实现
// ---------------------------------------------------------------------------

#[test]
fn rng_same_seed_same_sequence() {
    let script = format!(
        r#"{HELPERS}
local out = {{}}
for i = 1, 8 do
  out[i] = tostring(world.rng("roll/d6"))
end
return {{ sequence = join(out, ",") }}
"#
    );
    let (one, _a) = fixture(0x5EED_1234);
    let (two, _b) = fixture(0x5EED_1234);
    let (other, _c) = fixture(0x5EED_1235);
    let a = text(&run(&one, &script), "sequence");
    let b = text(&run(&two, &script), "sequence");
    let c = text(&run(&other, &script), "sequence");
    assert_eq!(a, b, "同种子 ⇒ 同序列");
    assert_ne!(a, c, "换种子 ⇒ 换序列");
    assert_eq!(a.split(',').count(), 8);
    for value in a.split(',') {
        let parsed: u64 = value.parse().expect("必须是十进制整数");
        assert!(parsed <= u32::MAX as u64, "world.rng 的取值域是 u32");
    }
}

#[test]
fn kv_is_the_same_table_as_ygg_kv() {
    let script = format!(
        r#"{HELPERS}
world.kv.set("k", "v")
world.kv.set("n", 42)
return {{
  text = world.kv.get("k"),
  number = world.kv.get("n"),
  miss = tostring(world.kv.get("nope")),
  same = world.kv == ygg.kv,
}}
"#
    );
    let (sandbox, _api) = fixture(1);
    let out = run(&sandbox, &script);
    assert_eq!(text(&out, "text"), "v");
    assert_eq!(int(&out, "number"), 42);
    assert_eq!(text(&out, "miss"), "nil", "get-miss 是 nil（不是 false）");
    assert!(
        out.get::<bool>("same").expect("字段必须是布尔"),
        "world.kv 必须是 ygg.kv 的同一张表（没有第二个 kv）"
    );
}

// ---------------------------------------------------------------------------
// kind 面：u8 / u64 / bytes 与无浮点闸门
// ---------------------------------------------------------------------------

#[test]
fn kinds_round_trip_and_float_never_crosses() {
    let script = format!(
        r#"{HELPERS}
world.component{{ name = "name", kind = "bytes" }}
world.component{{ name = "small", kind = "u8" }}
world.component{{ name = "link", kind = "u64" }}
local id = world.spawn{{ name = "hero", small = 255, link = 7 }}
local bytes_write = probe(function()
  world.each_mut({{ "name" }}, function(e, name) return name .. "!" end)
end)
local range = probe(function() world.set(id, {{ small = 256 }}) end)
local float = probe(function() world.set(id, {{ small = 1.5 }}) end)
local negative = probe(function() world.set(id, {{ link = -1 }}) end)
world.set(id, {{ small = 0, link = 9007199254740992 }})
return {{
  name = world.get(id, "name"),
  small = world.get(id, "small"),
  link = world.get(id, "link"),
  bytes_write = bytes_write,
  range = range,
  float = float,
  negative = negative,
}}
"#
    );
    let (sandbox, _api) = fixture(1);
    let out = run(&sandbox, &script);
    assert_eq!(text(&out, "name"), "hero");
    assert_eq!(int(&out, "small"), 0, "small 被 set 回 0");
    assert_eq!(int(&out, "link"), 9_007_199_254_740_992);

    let bytes_write = text(&out, "bytes_write");
    assert!(
        bytes_write.contains("YGG-SIM-SCHEMA:"),
        "bytes 列在内核查询面上没有写回口，必须显式拒绝（得到 {bytes_write}）"
    );
    assert!(bytes_write.contains("world.set"), "错误消息要给出替代路径");

    let range = text(&out, "range");
    assert!(
        range.contains("YGG-SIM-FIXED:"),
        "u8 值域外必须拒（得到 {range}）"
    );
    let float = text(&out, "float");
    assert!(
        float.contains("YGG-SIM-FIXED:"),
        "浮点过不了面（得到 {float}）"
    );
    let negative = text(&out, "negative");
    assert!(
        negative.contains("YGG-SIM-FIXED:"),
        "u64 负数必须拒（得到 {negative}）"
    );
}

#[test]
fn duplicate_declaration_and_unknown_kind_error() {
    let script = format!(
        r#"{HELPERS}
world.component{{ name = "hp", kind = "i64" }}
local duplicate = probe(function() world.component{{ name = "hp", kind = "i64" }} end)
local kind = probe(function() world.component{{ name = "hp2", kind = "f64" }} end)
return {{ duplicate = duplicate, kind = kind }}
"#
    );
    let (sandbox, _api) = fixture(1);
    let out = run(&sandbox, &script);
    for key in ["duplicate", "kind"] {
        let message = text(&out, key);
        assert!(
            message.contains("YGG-SIM-SCHEMA:"),
            "`{key}` 必须带内核码，得到 {message}"
        );
    }
    assert!(text(&out, "duplicate").contains("hp"));
    assert!(text(&out, "kind").contains("f64"));
}

// ---------------------------------------------------------------------------
// remove：摘组件后索引必须仍然一致（`from_parts` 重建面）
// ---------------------------------------------------------------------------

#[test]
fn remove_component_keeps_index_consistent() {
    let script = format!(
        r#"{HELPERS}
world.component{{ name = "hp", kind = "i64" }}
world.component{{ name = "atk", kind = "i64" }}
local a = world.spawn{{ hp = 1, atk = 2 }}
local b = world.spawn{{ hp = 3, atk = 4 }}
local removed = world.remove(a, {{ "atk" }})
local missing = probe(function() world.remove(a, {{ "atk" }}) end)
local both = {{}}
world.each({{ "hp", "atk" }}, function(id, hp, atk) both[#both + 1] = id .. ":" .. hp .. ":" .. atk end)
return {{
  removed = removed,
  a_hp = world.get(a, "hp"),
  a_atk = tostring(world.get(a, "atk")),
  b_hp = world.get(b, "hp"),
  b_atk = world.get(b, "atk"),
  ids_hp = join(world.ids({{ "hp" }}), ","),
  ids_both = join(world.ids({{ "hp", "atk" }}), ","),
  both = join(both, " "),
  missing = missing,
}}
"#
    );
    let (sandbox, _api) = fixture(1);
    let out = run(&sandbox, &script);
    assert_eq!(int(&out, "removed"), 1);
    assert_eq!(int(&out, "a_hp"), 1, "留下的组件值不变");
    assert_eq!(text(&out, "a_atk"), "nil", "摘掉的组件读回 nil");
    assert_eq!(int(&out, "b_hp"), 3, "别的实体值不变");
    assert_eq!(int(&out, "b_atk"), 4);
    assert_eq!(
        text(&out, "ids_hp"),
        "2,1",
        "规范序 = 桶序（建桶先后）：1 号实体换到后建的 (hp) 桶，于是排在 2 号之后"
    );
    assert_eq!(text(&out, "ids_both"), "2", "摘过组件的实体换到更小的桶");
    assert_eq!(text(&out, "both"), "2:3:4");
    let missing = text(&out, "missing");
    assert!(
        missing.contains("YGG-SIM-SCHEMA:"),
        "重复移除必须拒（得到 {missing}）"
    );
}

// ---------------------------------------------------------------------------
// 日志环（宿主 drain；不进叙述流/事件流）
// ---------------------------------------------------------------------------

#[test]
fn log_lines_are_drained_by_the_host() {
    let script = format!(
        r#"{HELPERS}
world.log("hp=%d atk=%s", 7, "sword")
world.log("second")
return {{ done = true }}
"#
    );
    let (sandbox, api) = fixture(1);
    run(&sandbox, &script);
    assert_eq!(
        api.take_world_log(),
        vec!["hp=7 atk=sword".to_string(), "second".to_string()]
    );
    assert!(api.take_world_log().is_empty(), "drain 之后清空");
}
