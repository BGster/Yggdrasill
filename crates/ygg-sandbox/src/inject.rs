//! 确定性注入（D-B5 / research §3）：从零构造白名单 `math` 表，`random` 走世界内核流。
//!
//! 沙箱 globals 已被 `Sandbox::build` 整体替换（ALL_SAFE 加载的旧 math 库自此
//! 不可达），本模块**从零构造** `math` 表——绝不 raw_get 旧库（替换后不可达，
//! 也杜绝把非白名单成员夹带进来），全部函数为 Rust `create_function` 闭包：
//!
//! - **精确集**（D-B5）：`abs / ceil / floor / fmod / modf / tointeger / ult /
//!   min / max / type` + 常量面 `huge / maxinteger / mininteger`——IEEE 754
//!   精确运算或整数运算，native/wasm 位一致；超越函数（sin/exp/log/…，走 C
//!   libm 有 glibc vs wasi-libc 位差，research §3 威胁 #1）结构性缺席。
//! - **`sqrt` 也缺席**（M2 修订）：它是 IEEE 正确舍入，技术上可留；但规则里
//!   要平方根的地方是**距离比较**，那必须留在定点域（`world.isqrt`，
//!   `crates/ygg-sim/src/fixed.rs`）。留一个浮点 `sqrt` 只会诱导作者把定量
//!   规则写成浮点——D-B5 的初衷是「定量只用定点」，不是「浮点够精确就能用」。
//! - **`random` 走世界内核流**（M2 修订）：不再是模块内的 splitmix64 `Cell`，
//!   而是 `Sim::rng32("math.random")`。三件事同时成立：脚本随机与内核其余
//!   随机同源；随机状态进快照、随回滚一起回滚（旧 Cell 方案下回滚不生效）；
//!   种子只从 `Sim::new(seed)` 进，`randomseed` 结构性缺席（B5：Mod 禁调）。
//!
//! **时间不在这里**（M1-E，D2）：Lua 侧没有推进时钟的入口，tick 只由宿主经
//! `crate::Sandbox::step` 推进；`ygg.ticks` / `DeterminismHandle::advance_ticks`
//! 随事件-信封层一并删除。
//!
//! 语义保真说明：floor/ceil 按 Lua 5.4 在结果可表示为 integer 时返回
//! integer、否则 float；abs 对 `mininteger` 返回包装负值（Lua 5.4 wraparound
//! 语义）；min/max 逐数比较、并列取先、返回值保留原 integer/float 类型。

use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Error, Lua, MultiValue, Value};
use ygg_sim::sim::Sim;

/// 内核种子常量（"YGDR" 的 ASCII，与 `.cargo/config.toml` 的 luai_makeseed 覆盖同源）。
///
/// **唯一消费者**：装载面（`modload.rs` 用它对 `Sim::new` 播种）。本模块不再自己
/// 持有随机状态——`math.random` 消费的是世界内核的流，不是这个常量。
/// **S12 golden-vector 锚**：修改即破坏基线。
pub const KERNEL_RNG_SEED: u64 = 0x5947_4452;

/// `math.random` 消费的内核随机流名。
///
/// 流名是**公开常量**：重置/回滚测试要靠它定位流，但脚本无法改名（改名就是
/// 造一条新流，会破坏「同一世界同一序列」的前提）。
pub const RANDOM_STREAM: &str = "math.random";

/// 取世界内核槽并做一次 `Sim::rng32`（未挂载 / 借用冲突 → 可读错误）。
fn rng32(slot: &Rc<RefCell<Option<Box<Sim>>>>, what: &str) -> Result<u32, Error> {
    let mut guard = slot.try_borrow_mut().map_err(|_| {
        Error::runtime(format!(
            "WORLD_BUSY: {what} 不能在系统体执行期间从 Lua 调用（内核槽已独占借用）"
        ))
    })?;
    let sim = guard.as_mut().ok_or_else(|| {
        Error::runtime(format!(
            "YGG-SIM-UNMOUNTED: {what} 需要已挂载的世界内核（沙箱还没 set_sim）"
        ))
    })?;
    Ok(sim.rng32(RANDOM_STREAM))
}

/// `math.random()` 的 53 位均匀整数（两个 32 位输出拼成，取高位 53 位）。
///
/// 53 是双精度尾数位宽：`[0, 2^53)` 的均匀整数除以 2^53 恰好覆盖 `[0,1)` 里
/// **全部**可表示浮点（1 以下的浮点都是 k·2^-53），于是取值计数上无偏差。
fn draw53(slot: &Rc<RefCell<Option<Box<Sim>>>>, what: &str) -> Result<u64, Error> {
    let hi = u64::from(rng32(slot, what)?);
    let lo = u64::from(rng32(slot, what)?);
    Ok(((hi << 32) | lo) >> 11)
}

/// `[0, bound)` 均匀整数（取模偏差不涉确定性，边界已由调用方护卫）。
fn draw_below(slot: &Rc<RefCell<Option<Box<Sim>>>>, what: &str, bound: u64) -> Result<u64, Error> {
    Ok(draw53(slot, what)? % bound)
}

/// `[0,1)` 均匀浮点：53 位整数除以 2^53（除 2 的幂精确，计数上均匀）。
///
/// 返回 **float**（不是 integer）——Lua 5.4 的 `math.random()` 就是 float，
/// 而 1 以下的每个浮点都是 k·2^-53，所以「取 53 位整数再除」与「直接取 53 位
/// 精度内的浮点」是同一个分布；让作者把 `math.random() * x` 写成
/// `math.random() / 2^53 * x` 只多一种写错的方式。
fn random_unit(slot: &Rc<RefCell<Option<Box<Sim>>>>, what: &str) -> Result<f64, Error> {
    const TWO_POW_53: f64 = 9_007_199_254_740_992.0;
    Ok(draw53(slot, what)? as f64 / TWO_POW_53)
}

/// 数值参数提取（整数升浮点比较；非数值即带定位的可读错误）。
fn as_float(v: &Value, name: &str, idx: usize) -> Result<f64, Error> {
    match v {
        Value::Integer(i) => Ok(*i as f64),
        Value::Number(f) => Ok(*f),
        _ => Err(Error::runtime(format!(
            "bad argument #{idx} to '{name}' (number expected)"
        ))),
    }
}

/// Lua 5.4 语义的整型归一：结果可表示为 integer 时返回 integer，否则 float
/// （含 NaN/±∞ 直通——非有限值拒绝是状态哈希面（D-继承-5）的职责，非本层）。
/// 区间为**半开** `[i64::MIN as f64, i64::MAX as f64)`：`i64::MAX as f64` 实为
/// 2^63（向上舍入），在 i64 中不可精确表示——Rust 饱和转换会把它归到
/// i64::MAX，而 Lua 5.4 对不可表示值走 fail-conversion 返 float；半开区间
/// 使 2^63 落入 float 分支，与 Lua 语义一致（audit R1 修复）。
fn to_integer_or_float(r: f64) -> Value {
    if r.is_finite() && (i64::MIN as f64..i64::MAX as f64).contains(&r) && r.fract() == 0.0 {
        Value::Integer(r as i64)
    } else {
        Value::Number(r)
    }
}

/// 安装确定性注入：从零构造 `math` 白名单表（唯一产物）。
///
/// `sim` 是**世界内核槽**（与 [`crate::Sandbox::sim_handle`] 同一个 `Rc`）：
/// `math.random` 从它取流。传槽而不是传种子/传 `Sim` 是有意的——回滚/重播会把
/// 槽里的 `Sim` 整个换掉，而闭包拿着的槽不变：于是「回滚到快照 ⇒ 随机序列
/// 也回滚」自动成立（旧的自带 `Cell<u64>` 方案下这条是假的）。
///
/// 必须在 `Sandbox` 构造之后、`set_sim` 之后调用（依赖 build 已替换掉的 globals
/// 与已挂载的内核）；不触碰 `ygg` 表——`ygg.*` 的安装面归 [`crate::host_api`]。
pub fn install(lua: &Lua, sim: Rc<RefCell<Option<Box<Sim>>>>) -> Result<(), Error> {
    let math = lua.create_table()?;

    // ---- 精确集（IEEE / 整数运算，逐条可审计）----
    math.raw_set(
        "abs",
        lua.create_function(|_, v: Value| {
            Ok(match v {
                // Lua 5.4 wraparound：abs(mininteger) == mininteger（保真声明）。
                Value::Integer(i) => Value::Integer(i.wrapping_abs()),
                Value::Number(f) => Value::Number(f.abs()),
                _ => return Err(Error::runtime("bad argument #1 to 'abs' (number expected)")),
            })
        })?,
    )?;

    math.raw_set(
        "floor",
        lua.create_function(|_, v: Value| {
            Ok(to_integer_or_float(as_float(&v, "floor", 1)?.floor()))
        })?,
    )?;
    math.raw_set(
        "ceil",
        lua.create_function(|_, v: Value| {
            Ok(to_integer_or_float(as_float(&v, "ceil", 1)?.ceil()))
        })?,
    )?;
    // Rust f64 `%` = C fmod：截断余、符号随被除数（Lua 语义）；除零得 NaN 直通。
    math.raw_set(
        "fmod",
        lua.create_function(|_, (a, b): (Value, Value)| {
            let (a, b) = (as_float(&a, "fmod", 1)?, as_float(&b, "fmod", 2)?);
            Ok::<_, Error>(Value::Number(a % b))
        })?,
    )?;
    // 两返回值均为 float（Lua 5.4 语义）。
    math.raw_set(
        "modf",
        lua.create_function(|_, v: Value| {
            let f = as_float(&v, "modf", 1)?;
            let t = f.trunc();
            Ok::<_, Error>((t, f - t))
        })?,
    )?;
    math.raw_set(
        "tointeger",
        lua.create_function(|_, v: Value| {
            Ok(match v {
                Value::Integer(i) => Some(i),
                Value::Number(f)
                    if f.is_finite()
                        && f.fract() == 0.0
                        && (i64::MIN as f64..i64::MAX as f64).contains(&f) =>
                {
                    Some(f as i64)
                }
                _ => None,
            })
        })?,
    )?;
    // 无符号比较：整数按位模式重解释（负数进入无符号域，Lua 5.4 语义）。
    math.raw_set(
        "ult",
        lua.create_function(|_, (a, b): (i64, i64)| Ok((a as u64) < (b as u64)))?,
    )?;

    // min/max：逐数比较、并列取先（含 3 == 3.0 场景）、返回值保留原类型。
    for (name, pick) in [("min", true), ("max", false)] {
        math.raw_set(
            name,
            lua.create_function(move |_, args: MultiValue| {
                let mut items: Vec<(Value, f64)> = Vec::new();
                for (idx, v) in args.into_iter().enumerate() {
                    let f = as_float(&v, name, idx + 1)?;
                    items.push((v, f));
                }
                let better = |a: &(Value, f64), b: &(Value, f64)| {
                    if pick {
                        b.1 < a.1
                    } else {
                        b.1 > a.1
                    }
                };
                items
                    .into_iter()
                    .reduce(|a, b| if better(&a, &b) { b } else { a })
                    .map(|(v, _)| v)
                    .ok_or_else(|| {
                        Error::runtime(format!("bad argument #1 to '{name}' (value expected)"))
                    })
            })?,
        )?;
    }

    // ---- 世界内核流 random（D-B5 / M2 修订）----
    math.raw_set("random", {
        let sim = Rc::clone(&sim);
        lua.create_function(move |_, (m, n): (Option<i64>, Option<i64>)| {
            match (m, n) {
                (None, None) => Ok(Value::Number(random_unit(&sim, "math.random")?)),
                (Some(m), None) => {
                    if m < 1 {
                        return Err(Error::runtime(
                            "bad argument #1 to 'random' (interval is empty)",
                        ));
                    }
                    Ok(Value::Integer(
                        1 + draw_below(&sim, "math.random", m as u64)? as i64,
                    ))
                }
                (Some(m), Some(n)) => {
                    if n < m {
                        return Err(Error::runtime(
                            "bad argument #2 to 'random' (interval is empty)",
                        ));
                    }
                    // 全区间护栏（audit R2）：宽度超 i64 可行域时拒绝为可读
                    // 错误而非算术 panic。判定在 i128 域进行：
                    // m + (width-1) ≤ i64::MAX 即整个结果域落在 i64。
                    let width = n as i128 - m as i128 + 1;
                    if width > u64::MAX as i128 || m as i128 + (width - 1) > i64::MAX as i128 {
                        return Err(Error::runtime(
                            "bad argument #2 to 'random' (interval too large for kernel rng)",
                        ));
                    }
                    let width = width as u64;
                    let pick = draw_below(&sim, "math.random", width)? as i128;
                    Ok(Value::Integer((m as i128 + pick) as i64))
                }
                // random(nil, n)：实参缺失只允许双缺——单缺第二参拒绝。
                (None, Some(_)) => Err(Error::runtime(
                    "bad argument #2 to 'random' (interval is empty)",
                )),
            }
        })?
    })?;

    // 常量面（huge = IEEE +inf；整数边界随 Lua 5.4 语义）。
    math.raw_set("huge", f64::INFINITY)?;
    math.raw_set("maxinteger", i64::MAX)?;
    math.raw_set("mininteger", i64::MIN)?;

    // math.type：M0 白名单成员之一（D-B5 列表的伴生判定函数）。
    math.raw_set(
        "type",
        lua.create_function(|_, v: Value| {
            Ok(match v {
                Value::Integer(_) => Some("integer"),
                Value::Number(_) => Some("float"),
                _ => None,
            })
        })?,
    )?;

    lua.globals().raw_set("math", math)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::Sandbox;

    /// 构造**已挂载内核**的沙箱并完成确定性注入。
    ///
    /// 旧形态下 inject 自带 `Cell` 随机状态，不挂内核也能跑；M2 起
    /// `math.random` 消费世界内核流，**未挂载即 `YGG-SIM-UNMOUNTED`**——
    /// 这是「脚本随机必须进快照」的代价，也顺带把「忘了 set_sim」提前暴露。
    fn mounted(seed: u64) -> Sandbox {
        let mut sb = Sandbox::new().expect("sandbox construction");
        sb.set_sim(Some(Box::new(Sim::new(seed))));
        install(sb.lua(), sb.sim_handle()).expect("determinism install");
        sb
    }

    #[test]
    fn math_table_is_exact_op_whitelist() {
        let sb = mounted(KERNEL_RNG_SEED);
        let lua = sb.lua();
        // 白名单成员全部可达且为函数。
        for name in [
            "abs",
            "ceil",
            "floor",
            "fmod",
            "modf",
            "tointeger",
            "ult",
            "min",
            "max",
            "random",
            "type",
        ] {
            let kind: String = lua
                .load(format!("return type(math.{name})"))
                .eval()
                .unwrap();
            assert_eq!(kind, "function", "math.{name} must be exposed");
        }
        // 常量面。
        let wraps: bool = lua
            .load("return math.maxinteger + 1 == math.mininteger")
            .eval()
            .unwrap();
        assert!(wraps, "integer boundary constants must follow Lua 5.4");
        let huge_ok: bool = lua.load("return math.huge == math.huge").eval().unwrap();
        assert!(huge_ok, "math.huge must be present");
        // 超越函数 / sqrt / randomseed 结构性缺席：
        // sqrt 是精确的，但定量规则要的平方根只在定点域（world.isqrt）——
        // 留一个浮点 sqrt 就是把作者往浮点定量上引。
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
            let absent: bool = lua
                .load(format!("return math.{name} == nil"))
                .eval()
                .unwrap();
            assert!(absent, "math.{name} must be structurally absent");
        }
        // `^` 运算符走 libm pow（glibc vs wasi-libc 位差）——`math.pow` 删了还
        // 不够，语法层的那条路也必须没有落点吗？这里只钉住「库表里没有」：
        // 运算符由 lint 面（YGG-STD，另一切片）与「定量只用定点」的约定约束。
    }

    #[test]
    fn exact_ops_behave_per_lua54() {
        let sb = mounted(KERNEL_RNG_SEED);
        let lua = sb.lua();
        // floor/ceil：可表示即 integer，否则 float。
        let (v, t): (i64, String) = lua
            .load("return math.floor(3.7), math.type(math.floor(3.7))")
            .eval()
            .unwrap();
        assert_eq!((v, t.as_str()), (3, "integer"));
        let neg: i64 = lua.load("return math.floor(-3.7)").eval().unwrap();
        assert_eq!(neg, -4);
        let ceil_neg: i64 = lua.load("return math.ceil(-3.2)").eval().unwrap();
        assert_eq!(ceil_neg, -3);
        let big: String = lua
            .load("return math.type(math.floor(2^70))")
            .eval()
            .unwrap();
        assert_eq!(big, "float", "beyond i64 range floor must stay float");
        // 2^63 边界（audit R1 回归锚）：i64::MAX as f64 舍入到 2^63，不可表示，
        // Lua 5.4 fail-conversion → float（而非饱和 Integer）。
        let boundary: String = lua
            .load("return math.type(math.floor(2^63))")
            .eval()
            .unwrap();
        assert_eq!(boundary, "float", "2^63 is not representable in i64");
        let ti_boundary: bool = lua
            .load("return math.tointeger(2^63) == nil")
            .eval()
            .unwrap();
        assert!(
            ti_boundary,
            "tointeger(2^63) must be nil (Lua 5.4 fail-conversion)"
        );
        // abs：整数/浮点/mininteger wraparound。
        let abs_min: i64 = lua.load("return math.abs(math.mininteger)").eval().unwrap();
        assert_eq!(
            abs_min,
            i64::MIN,
            "Lua 5.4 wraparound: abs(mininteger) == mininteger"
        );
        // fmod：截断余，符号随被除数。
        let (a, b): (f64, f64) = lua
            .load("return math.fmod(-7, 3), math.fmod(7, -3)")
            .eval()
            .unwrap();
        assert_eq!((a, b), (-1.0, 1.0));
        // modf：两返回值均为 float（小数部分 f64 非精确表示，epsilon 断言）。
        let (i, f): (f64, f64) = lua.load("return math.modf(3.7)").eval().unwrap();
        assert_eq!(i, 3.0);
        assert!((f - 0.7).abs() < 1e-12, "fractional part: {f}");
        // tointeger / ult。
        let ti: Option<i64> = lua.load("return math.tointeger(3.0)").eval().unwrap();
        assert_eq!(ti, Some(3));
        let ti_nil: bool = lua
            .load("return math.tointeger(3.7) == nil and math.tointeger('3') == nil")
            .eval()
            .unwrap();
        assert!(ti_nil);
        let (lt1, lt2): (bool, bool) = lua
            .load("return math.ult(1, -1), math.ult(-1, 1)")
            .eval()
            .unwrap();
        assert_eq!((lt1, lt2), (true, false), "unsigned reinterpretation");
        // min/max：逐数比较、返回值保类型。
        let (mn, mx, mt): (i64, i64, String) = lua
            .load("return math.min(3, 1, 2), math.max(3, 1, 2), math.type(math.max(1, 2))")
            .eval()
            .unwrap();
        assert_eq!((mn, mx, mt.as_str()), (1, 3, "integer"));
        let mixed: f64 = lua.load("return math.max(1, 2.5)").eval().unwrap();
        assert_eq!(mixed, 2.5);
    }

    #[test]
    fn random_comes_from_the_world_stream() {
        // ① 同一世界种子 ⇒ 同一序列（跨状态、跨进程都只取决于 Sim::new 的种子）。
        let a = mounted(KERNEL_RNG_SEED);
        let b = mounted(KERNEL_RNG_SEED);
        let seq_a: Vec<f64> = a
            .lua()
            .load("local r = {} for i = 1, 10 do r[i] = math.random() end return r")
            .eval()
            .unwrap();
        let seq_b: Vec<f64> = b
            .lua()
            .load("local r = {} for i = 1, 10 do r[i] = math.random() end return r")
            .eval()
            .unwrap();
        assert_eq!(seq_a, seq_b, "same world seed must pin the sequence");
        assert!(
            seq_a.iter().all(|&x| (0.0..1.0).contains(&x)),
            "random() must stay in [0, 1)"
        );
        // ② 不同世界种子 ⇒ 不同序列（证明种子确实从 Sim 进，不是模块常量）。
        let c = mounted(KERNEL_RNG_SEED ^ 1);
        let seq_c: Vec<f64> = c
            .lua()
            .load("local r = {} for i = 1, 10 do r[i] = math.random() end return r")
            .eval()
            .unwrap();
        assert_ne!(seq_a, seq_c, "the seed must come from the world kernel");
        // ③ 整数形态：边界与范围。
        let bounded: Vec<i64> = a
            .lua()
            .load("local r = {} for i = 1, 30 do r[i] = math.random(10) end return r")
            .eval()
            .unwrap();
        assert!(bounded.iter().all(|&x| (1..=10).contains(&x)));
        let ranged: Vec<i64> = b
            .lua()
            .load("local r = {} for i = 1, 30 do r[i] = math.random(5, 9) end return r")
            .eval()
            .unwrap();
        assert!(ranged.iter().all(|&x| (5..=9).contains(&x)));
        // ④ 空区间拒绝（可读报错）。
        for chunk in ["return math.random(0)", "return math.random(5, 3)"] {
            let msg = a
                .lua()
                .load(chunk)
                .exec()
                .expect_err("empty interval must be rejected")
                .to_string();
            assert!(
                msg.contains("interval is empty"),
                "readable rejection expected: {msg}"
            );
        }
        // ⑤ 全区间护栏（audit R2 回归锚）：width 超 i64 可行域 → 可读错误非 panic。
        let msg = a
            .lua()
            .load("return math.random(math.mininteger, math.maxinteger)")
            .exec()
            .expect_err("full-range interval must be rejected readably")
            .to_string();
        assert!(
            msg.contains("interval too large"),
            "readable rejection expected, got panic or other error: {msg}"
        );
    }

    #[test]
    fn random_state_rolls_back_with_the_world() {
        // 旧方案（模块自带 Cell）下这条是假的：快照/回滚到不了随机状态。
        // 现在 random 消费内核流，回滚世界即回滚序列。
        let mut sb = mounted(KERNEL_RNG_SEED);
        let frame = sb.snapshot().expect("snapshot");
        let chunk = "local r = {} for i = 1, 10 do r[i] = math.random() end return r";
        let first: Vec<f64> = sb.lua().load(chunk).eval().unwrap();
        sb.restore(&frame).expect("restore");
        let again: Vec<f64> = sb.lua().load(chunk).eval().unwrap();
        assert_eq!(first, again, "rollback must rewind the random stream");
    }

    #[test]
    fn random_without_a_world_is_a_readable_error() {
        // 未挂载 = 可读错误，不是 nil、不是 panic、不是静默用模块常量。
        let mut sb = Sandbox::new().expect("sandbox construction");
        sb.set_sim(None);
        install(sb.lua(), sb.sim_handle()).expect("determinism install");
        let msg = sb
            .lua()
            .load("return math.random()")
            .exec()
            .expect_err("unmounted world must reject math.random")
            .to_string();
        assert!(msg.contains("YGG-SIM-UNMOUNTED"), "readable error: {msg}");
    }

    #[test]
    fn no_lua_side_clock_survives_injection() {
        // M1-E：时间只由宿主推进（`Sandbox::step`）——注入面不得重建
        // 任何 Lua 侧可读/可写的时钟（`ygg.ticks` 随事件-信封层删除）。
        let sb = mounted(KERNEL_RNG_SEED);
        let lua = sb.lua();
        let absent: bool = lua
            .load("return ygg.ticks == nil and ygg.advance == nil and ygg.set_ticks == nil")
            .eval()
            .unwrap();
        assert!(absent, "no Lua-side clock may exist");
    }
}
