//! M2 stdlib 面：`string` / `table` / `utf8` 露出 + `math` 收窄 + `world.isqrt`。
//!
//! 三段断言口径：
//!
//! 1. **裸沙箱**（`Sandbox::new`，未装 host/inject）：`string` / `table` / `utf8`
//!    可达且方法能跑，`string.dump` 已摘；`os` / `io` / `package` / `coroutine` /
//!    `debug` / `math` 仍不可达（deny-by-default 没被这一刀撕开）。
//! 2. **装好 inject**：`math` 从零构造的精确集在、超越函数与浮点 `sqrt` 与
//!    `randomseed` 全不在；`math.random` 消费世界内核流（同种子同序列）。
//! 3. **真实消费者路径**（`Loaded::from_bundle` 装载一个内存 Mod）：入口顶层
//!    就能用这些库并调 `world.isqrt`——这才是「规则作者拿到的面」。

use std::cell::RefCell;
use std::rc::Rc;

use ygg_core::kv::KvStore;
use ygg_core::value::Value;
use ygg_sandbox::host_api::{self, HostApi, HostHandles};
use ygg_sandbox::inject;
use ygg_sandbox::modload::{Loaded, ModBundle};
use ygg_sandbox::sandbox::Sandbox;
use ygg_sim::sim::Sim;

/// 定点的一（Q32.32）。
const ONE: i64 = 1 << 32;

/// 装一个「真宿主面 + 已挂载内核 + 确定性注入」的沙箱。
fn fixture(seed: u64) -> (Sandbox, HostApi) {
    let sandbox = Sandbox::new().expect("沙箱构造");
    let api = host_api::install(
        sandbox.lua(),
        HostHandles::new(Rc::new(RefCell::new(KvStore::new()))),
        sandbox.sim_handle(),
    )
    .expect("host API 必须可装");
    api.set_sim(Some(Box::new(Sim::new(seed))));
    inject::install(sandbox.lua(), sandbox.sim_handle()).expect("确定性注入必须可装");
    (sandbox, api)
}

/// 跑一段 Lua 并取回返回值。
fn eval<T: mlua::FromLuaMulti>(sandbox: &Sandbox, chunk: &str) -> T {
    sandbox.reset_budget();
    sandbox
        .lua()
        .load(chunk)
        .eval()
        .unwrap_or_else(|err| panic!("Lua 必须跑通：{err}"))
}

#[test]
fn bare_sandbox_exposes_the_pure_computation_libraries() {
    let sb = Sandbox::new().expect("沙箱构造");
    let lua = sb.lua();
    // 露出面。
    for name in ["string", "table", "utf8"] {
        let kind: String = lua.load(format!("return type({name})")).eval().unwrap();
        assert_eq!(kind, "table", "{name} 必须露出");
    }
    // 真的能用（不是一张空表）。
    let (upper, len, joined, code_points): (String, usize, String, usize) = lua
        .load(
            r#"local t = { "b", "c", "a" }
               table.sort(t)
               return ("abc"):upper(), string.len("abcd"), table.concat(t, ","), utf8.len("你好")"#,
        )
        .eval()
        .unwrap();
    assert_eq!(upper, "ABC");
    assert_eq!(len, 4);
    assert_eq!(joined, "a,b,c", "table.sort + concat 必须按字节序稳定");
    assert_eq!(code_points, 2, "utf8.len 数的是码点");
    // 摘掉面与仍未露出的面。
    let dump: bool = lua.load("return string.dump == nil").eval().unwrap();
    assert!(dump, "string.dump 必须摘掉（字节码不是脚本能力）");
    for name in ["os", "io", "package", "coroutine", "debug", "math"] {
        let absent: bool = lua.load(format!("return {name} == nil")).eval().unwrap();
        assert!(absent, "{name} 不得在裸沙箱里露出");
    }
    // 字符串值方法（走 string 元表）一直可用——本刀不动它，只确认没被误伤。
    let (found, sub): (bool, String) = lua
        .load(r#"return ("hello"):find("ll") ~= nil, ("hello"):sub(2, 3)"#)
        .eval()
        .unwrap();
    assert!(found);
    assert_eq!(sub, "el");
}

#[test]
fn math_is_the_exact_op_subset_only() {
    let (sb, _api) = fixture(inject::KERNEL_RNG_SEED);
    let present: bool = eval(
        &sb,
        r#"return math.floor(3.7) == 3 and math.abs(-2) == 2 and math.maxinteger + 1 == math.mininteger
                  and math.fmod(-7, 3) == -1 and math.type(math.ceil(1.2)) == "integer""#,
    );
    assert!(present, "精确集必须全部按 Lua 5.4 语义工作");
    // 结构性缺席：超越函数（libm 位差）+ 浮点 sqrt（定量只在定点域）+ randomseed。
    for name in [
        "sin",
        "cos",
        "tan",
        "asin",
        "acos",
        "atan",
        "exp",
        "log",
        "log10",
        "pow",
        "sqrt",
        "randomseed",
    ] {
        let absent: bool = sb
            .lua()
            .load(format!("return math.{name} == nil"))
            .eval()
            .unwrap();
        assert!(absent, "math.{name} 必须缺席");
    }
}

#[test]
fn math_random_follows_the_world_stream() {
    let (a, _api_a) = fixture(inject::KERNEL_RNG_SEED);
    let (b, _api_b) = fixture(inject::KERNEL_RNG_SEED);
    let chunk =
        "local r = {} for i = 1, 5 do r[i] = math.random() end return table.concat(r, \",\")";
    let seq_a: String = eval(&a, chunk);
    let seq_b: String = eval(&b, chunk);
    assert_eq!(seq_a, seq_b, "同世界种子必须同序列");
    let in_unit: bool = eval(&a, "local x = math.random() return x >= 0 and x < 1");
    assert!(in_unit, "math.random() 留在 [0,1)");
    let bounded: bool = eval(
        &a,
        "local ok = true for _ = 1, 20 do local v = math.random(6) if v < 1 or v > 6 then ok = false end end return ok",
    );
    assert!(bounded, "math.random(n) 留在 1..=n");
}

#[test]
fn world_isqrt_is_fixed_point_floor() {
    let (sb, _api) = fixture(inject::KERNEL_RNG_SEED);
    // 精确平方：isqrt(4.0) == 2.0（Q32.32 原值就是 2 * ONE）。
    let exact: i64 = eval(&sb, &format!("return world.isqrt(4 * {ONE})"));
    assert_eq!(exact, 2 * ONE);
    // 非整数平方：floor 语义。不变式在**标度域**比：r² ≤ x·2^32 < (r+1)²
    //（先移位再比较会把 (r+1)² 的严格性舍掉，上界断言就会假红）。
    let r: i64 = eval(&sb, &format!("return world.isqrt(2 * {ONE})"));
    assert!(
        r > ONE && r < 2 * ONE,
        "isqrt(2.0) 必须落在 (1,2) 之间，得到 {r}"
    );
    let x_scaled = (2 * ONE as i128) << 32;
    let lo = (r as i128) * (r as i128);
    let hi = ((r as i128) + 1) * ((r as i128) + 1);
    assert!(lo <= x_scaled, "下界：{lo} vs {x_scaled}");
    assert!(hi > x_scaled, "上界：{hi} vs {x_scaled}");
    // 负值与浮点实参都是一等一的拒绝（可读错误，码在首段）。
    for (chunk, code) in [
        ("return world.isqrt(-1)", "YGG-SIM-FIXED"),
        ("return world.isqrt(1.5)", "YGG-SIM-FIXED"),
        ("return world.isqrt('4')", "YGG-SIM-FIXED"),
    ] {
        let msg = sb
            .lua()
            .load(chunk)
            .exec()
            .expect_err("拒绝面必须报错")
            .to_string();
        // mlua 在 runtime error 前面多包一层前缀，码落在文本里而不是行首。
        assert!(msg.contains(code), "{chunk} → {msg}");
    }
}

/// 内存 Mod 的入口：顶层直接用库面 + `world.isqrt`，结果写进 KV 供断言。
///
/// 这是**真实消费者路径**：不是「裸沙箱能跑」，而是「装载一个 Mod 时这些面
/// 就在入口顶层」。
const MOD_ENTRY: &str = r#"
local ONE = 4294967296
local parts = {}
parts[#parts + 1] = ("abc"):upper()
parts[#parts + 1] = table.concat({ 3, 1, 2 }, "-")
parts[#parts + 1] = tostring(utf8.len("世界"))
parts[#parts + 1] = tostring(math.floor(3.7))
parts[#parts + 1] = tostring(math.random(10) >= 1)
parts[#parts + 1] = tostring(world.isqrt(9 * ONE) == 3 * ONE)
ygg.kv.set("probe.surface", table.concat(parts, "|"))
"#;

#[test]
fn a_loaded_mod_sees_the_whole_surface_at_entry() {
    let loaded = Loaded::from_bundle(&ModBundle::in_memory(MOD_ENTRY)).expect("Mod 必须可装载");
    let store = loaded.kv();
    let store = store.borrow();
    let Some(Value::Str(text)) = store.get("probe.surface") else {
        panic!("观测值缺失或不是字符串");
    };
    assert_eq!(
        text, "ABC|3-1-2|2|3|true|true",
        "入口顶层必须同时看到 string/table/utf8/math/world.isqrt"
    );
}
