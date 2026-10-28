//! 确定性注入（D-B5 / research §3）：M0 白名单 math 表 + 内核种子化 random +
//! 显式推进时钟。
//!
//! 沙箱 globals 已被 `Sandbox::build` 整体替换（ALL_SAFE 加载的旧 math 库自此
//! 不可达），本模块**从零构造** `math` 表——绝不 raw_get 旧库（替换后不可达，
//! 也杜绝把非白名单成员夹带进来），全部函数为 Rust `create_function` 闭包：
//!
//! - **精确集**（D-B5）：`abs / floor / ceil / sqrt / fmod / modf / tointeger /
//!   ult / min / max`——IEEE 754 精确运算或整数运算，native/wasm 位一致；
//!   sqrt 为 IEEE 正确舍入运算故入白名单，超越函数（sin/exp/log/…，走 C
//!   libm 有 glibc vs wasi-libc 位差，research §3 威胁 #1）结构性缺席；
//! - **内核种子化 random**（D-B5）：splitmix64 PRNG，Rust 侧 `Cell` 状态，
//!   种子 = 内核常量 [`KERNEL_RNG_SEED`]（与 `luai_makeseed` 覆盖同源的
//!   "YGDR"）；同种子序列跨平台位相同（S12 golden-vector 固定序列锚——
//!   修改种子常量即破坏 golden 基线）；`randomseed` 不暴露（B5：Mod 禁调
//!   randomseed——结构性缺席）；
//! - **显式推进时钟**（FRD：时间由内核事件注入）：`ygg.ticks()` 返回单调
//!   u64 计数器，唯一推进路径是宿主侧 [`DeterminismHandle::advance_ticks`]
//!   （S10 驱动在转换边界调用）——零时间源依赖（不读任何 wall clock）。
//!
//! 语义保真说明：floor/ceil 按 Lua 5.4 在结果可表示为 integer 时返回
//! integer、否则 float；abs 对 `mininteger` 返回包装负值（Lua 5.4 wraparound
//! 语义）；min/max 逐数比较、并列取先、返回值保留原 integer/float 类型。

use std::cell::Cell;
use std::rc::Rc;

use mlua::{Error, Lua, MultiValue, Table, Value};

/// 内核随机种子常量（"YGDR" 的 ASCII，与 `.cargo/config.toml` 的
/// luai_makeseed 覆盖同源）。**S12 golden-vector 锚**：修改即破坏基线。
pub const KERNEL_RNG_SEED: u64 = 0x5947_4452;

/// splitmix64（零依赖确定性 PRNG；只用 wrapping 整数运算，native/wasm 位一致）。
/// `pub(crate)`：S9 host_api 的 sentinel 派生同源复用（非跨 crate 面）。
pub(crate) fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// [0,1) 均匀浮点：取 64 位输出的高 53 位除以 2^53——双精度尾数恰好 53 位，
/// 覆盖均匀且无低位偏差。
fn next_unit(rng: &Rc<Cell<u64>>) -> f64 {
    let mut s = rng.get();
    let z = splitmix64(&mut s);
    rng.set(s);
    (z >> 11) as f64 / (1u64 << 53) as f64
}

/// [0, bound) 均匀整数（取模偏差不涉确定性，M0 接受并声明）。
fn next_below(rng: &Rc<Cell<u64>>, bound: u64) -> u64 {
    let mut s = rng.get();
    let z = splitmix64(&mut s);
    rng.set(s);
    z % bound
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

/// 确定性注入句柄：宿主侧推进/重置面（Lua 侧只读消费）。
pub struct DeterminismHandle {
    rng: Rc<Cell<u64>>,
    ticks: Rc<Cell<u64>>,
}

impl DeterminismHandle {
    /// 推进时钟一格并返回新值（唯一推进路径；S10 驱动在转换边界调用）。
    pub fn advance_ticks(&self) -> u64 {
        let next = self.ticks.get().wrapping_add(1);
        self.ticks.set(next);
        next
    }

    /// 当前 tick 计数（诊断面）。
    pub fn ticks(&self) -> u64 {
        self.ticks.get()
    }

    /// 重置随机状态（M0 恒为内核常量种子；本能力供测试与日后「种子 =
    /// 内核常量 + 转换计数」演进预留，golden-vector 基线按常量种子固定）。
    pub fn reseed_random(&self, seed: u64) {
        self.rng.set(seed);
    }
}

/// 安装确定性注入：从零构造 `math` 白名单表 + 扩展 `ygg` 表（`ygg.ticks`）。
///
/// 必须在 `Sandbox` 构造之后调用（依赖 build 已创建的 `ygg` 占位表；
/// 缺失时 raw_get 直接报错，失败即暴露接线顺序问题）。
pub fn install(lua: &Lua) -> Result<DeterminismHandle, Error> {
    let rng: Rc<Cell<u64>> = Rc::new(Cell::new(KERNEL_RNG_SEED));
    let ticks: Rc<Cell<u64>> = Rc::new(Cell::new(0));

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
    // sqrt：IEEE 754 正确舍入（与超越函数的 libm 位差不同类），入白名单（D-B5）。
    math.raw_set(
        "sqrt",
        lua.create_function(|_, v: Value| Ok(Value::Number(as_float(&v, "sqrt", 1)?.sqrt())))?,
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

    // ---- 内核种子化 random（D-B5）----
    math.raw_set("random", {
        let rng = Rc::clone(&rng);
        lua.create_function(move |_, (m, n): (Option<i64>, Option<i64>)| {
            match (m, n) {
                (None, None) => Ok(Value::Number(next_unit(&rng))),
                (Some(m), None) => {
                    if m < 1 {
                        return Err(Error::runtime(
                            "bad argument #1 to 'random' (interval is empty)",
                        ));
                    }
                    Ok(Value::Integer(1 + next_below(&rng, m as u64) as i64))
                }
                (Some(m), Some(n)) => {
                    if n < m {
                        return Err(Error::runtime(
                            "bad argument #2 to 'random' (interval is empty)",
                        ));
                    }
                    // 全区间护栏（audit R2）：宽度超 i64 可行域时拒绝为可读
                    // 错误而非算术 panic（width=2^64 时 `as u64` 截为 0 →
                    // `z % 0`；近全区间时 `m + x` 在 i64 溢出）。判定在 i128
                    // 域进行：m + (width-1) ≤ i64::MAX 即整个结果域落在 i64。
                    let width = n as i128 - m as i128 + 1;
                    if width > u64::MAX as i128 || m as i128 + (width - 1) > i64::MAX as i128 {
                        return Err(Error::runtime(
                            "bad argument #2 to 'random' (interval too large for kernel rng)",
                        ));
                    }
                    let width = width as u64;
                    let pick = next_below(&rng, width) as i128;
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

    // ---- ygg.ticks：显式推进时钟（唯一推进路径在宿主侧句柄）----
    let ygg: Table = lua.globals().raw_get("ygg")?;
    ygg.raw_set("ticks", {
        let t = Rc::clone(&ticks);
        lua.create_function(move |_, ()| Ok(t.get()))?
    })?;

    Ok(DeterminismHandle { rng, ticks })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::Sandbox;

    /// 构造沙箱并完成确定性注入（两个返回值须同时持有——句柄丢弃不影响
    /// Lua 侧只读面，但测试统一全程持有以贴近 S10 驱动形态）。
    fn sandbox_with_inject() -> (Sandbox, DeterminismHandle) {
        let sb = Sandbox::new().expect("sandbox construction");
        let handle = install(sb.lua()).expect("determinism install");
        (sb, handle)
    }

    #[test]
    fn math_table_is_exact_op_whitelist() {
        let (sb, _) = sandbox_with_inject();
        let lua = sb.lua();
        // 白名单成员全部可达且为函数。
        for name in [
            "abs",
            "floor",
            "ceil",
            "sqrt",
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
        // 超越函数与 randomseed 结构性缺席（research §3 威胁 #1 / B5）。
        for name in [
            "sin",
            "cos",
            "tan",
            "asin",
            "acos",
            "atan",
            "exp",
            "log",
            "pow",
            "randomseed",
        ] {
            let absent: bool = lua
                .load(format!("return math.{name} == nil"))
                .eval()
                .unwrap();
            assert!(absent, "math.{name} must be structurally absent");
        }
    }

    #[test]
    fn exact_ops_behave_per_lua54() {
        let (sb, _) = sandbox_with_inject();
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
        // sqrt：IEEE 正确舍入（sqrt(4) 整数入参返回 float）。
        let (r, t): (f64, String) = lua
            .load("return math.sqrt(4), math.type(math.sqrt(4))")
            .eval()
            .unwrap();
        assert_eq!((r, t.as_str()), (2.0, "float"));
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
    fn random_is_kernel_seeded_and_reproducible() {
        // 同进程两个独立 Lua 状态：同常量种子 → 同序列（跨平台一致性由
        // S12 golden-vector 固定序列锚；此处证「状态不参与种子」）。
        let (sa, _ha) = sandbox_with_inject();
        let (sb_, hb) = sandbox_with_inject();
        let seq_a: Vec<f64> = sa
            .lua()
            .load("local r = {} for i = 1, 10 do r[i] = math.random() end return r")
            .eval()
            .unwrap();
        let seq_b: Vec<f64> = sb_
            .lua()
            .load("local r = {} for i = 1, 10 do r[i] = math.random() end return r")
            .eval()
            .unwrap();
        assert_eq!(seq_a, seq_b, "constant kernel seed must pin the sequence");
        assert!(
            seq_a.iter().all(|&x| (0.0..1.0).contains(&x)),
            "random() must stay in [0, 1)"
        );
        // 整数形态：边界与范围。
        let bounded: Vec<i64> = sa
            .lua()
            .load("local r = {} for i = 1, 30 do r[i] = math.random(10) end return r")
            .eval()
            .unwrap();
        assert!(bounded.iter().all(|&x| (1..=10).contains(&x)));
        let ranged: Vec<i64> = sb_
            .lua()
            .load("local r = {} for i = 1, 30 do r[i] = math.random(5, 9) end return r")
            .eval()
            .unwrap();
        assert!(ranged.iter().all(|&x| (5..=9).contains(&x)));
        // 空区间拒绝（可读报错）。
        for chunk in ["return math.random(0)", "return math.random(5, 3)"] {
            let msg = sa
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
        // 全区间护栏（audit R2 回归锚）：width 超 i64 可行域 → 可读错误非 panic。
        let msg = sa
            .lua()
            .load("return math.random(math.mininteger, math.maxinteger)")
            .exec()
            .expect_err("full-range interval must be rejected readably")
            .to_string();
        assert!(
            msg.contains("interval too large"),
            "readable rejection expected, got panic or other error: {msg}"
        );
        // 重置种子 → 序列复现（handle 是唯一状态写入口）。
        hb.reseed_random(KERNEL_RNG_SEED);
        let replay: Vec<f64> = sb_
            .lua()
            .load("local r = {} for i = 1, 10 do r[i] = math.random() end return r")
            .eval()
            .unwrap();
        assert_eq!(replay, seq_b, "reseed must reproduce the pinned sequence");
    }

    #[test]
    fn ygg_ticks_advances_only_via_host_handle() {
        let (sb, handle) = sandbox_with_inject();
        let lua = sb.lua();
        let t0: u64 = lua.load("return ygg.ticks()").eval().unwrap();
        assert_eq!(t0, 0, "clock starts at zero");
        assert_eq!(handle.advance_ticks(), 1);
        assert_eq!(handle.advance_ticks(), 2);
        let t: u64 = lua.load("return ygg.ticks()").eval().unwrap();
        assert_eq!(t, 2, "Lua side reads, only the host handle advances");
        assert_eq!(handle.ticks(), 2);
        // Lua 侧无任何推进入口（结构性：ticks 是无参只读闭包）。
        let absent: bool = lua
            .load("return ygg.advance == nil and ygg.set_ticks == nil")
            .eval()
            .unwrap();
        assert!(absent, "no Lua-side advance path may exist");
    }
}
