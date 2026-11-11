//! 正式燃料计量（D-继承-2 / D-B1）：`set_hook` 指令计数 + 每转换共享预算。
//!
//! S7 的临时 10M hook（常驻 `Sandbox::build` 的先行证明）已随本模块移除——
//! 双计量冲突：正式燃料与本模块共享同一预算单元，临时 hook 残留会双扣。
//!
//! **两个消费面共享同一预算**（D-B1「每转换共享」的 Rust 落地语义）：
//!
//! - **内核派发税**：[`FuelMeter::charge`]——S4 `run_transition` 在每次事件
//!   派发前 `charge(1)`，直接扣减共享预算；这是细粒度面（每次派发必扣）。
//! - **VM 指令计量**：`set_hook(every_nth_instruction(n))`——Lua VM 每 n 条
//!   指令触发一次 hook，按 n 扣减同一共享预算，耗尽即以可读 RuntimeError
//!   终止 Lua 执行（mlua 文档认可模式）；这是兜底面（拦截 Mod 内死循环，
//!   免于 kernel 派发税之外的无界执行）。
//!
//! 任一面耗尽 = 转换终止：charge 面返回 [`FuelExhausted`]（core 类型，经
//! `TransitionError::Fuel` 传播）；hook 面以 [`FUEL_EXHAUSTED_MESSAGE`] 中断
//! Lua 并由调用方取回 mlua 错误。预算在**每次转换开始时重置**
//! （[`HookFuelMeter::reset`]，S10 驱动在转换边界调用）——同一转换内的
//! 互发 handler 级联共享同一预算，因此两 handler 互发无法靠 per-dispatch
//! 重置永动（D-B1 立法理由）。
//!
//! **量化声明**：VM 面以 n 条指令为粒度记账，耗尽阈值量化到 n 的倍数——
//! 短于 n 条指令的 chunk 即使预算为 0 也不会触发 hook（细粒度保护由
//! charge 派发税承担）。n 值是开销权衡（research §2：低值开销高）。**S12
//! 实测定档**（探针 `probe_fuel_granularity_overhead_documents_default`，
//! 2026-10-27）：300k 次迭代工作负载、min-of-5 口径下 hook 面开销比
//! ≈ ×2.0（n=1000）/ ≈ ×2.6（n=100）——低粒度更贵但不灾难性；M0 默认
//! 保留 n=1000（探针 println! 输出为数据记录，工件 Performance
//! Considerations / Design History 同源引用）。
//!
//! **装载期独立预算（r1 模块装载面）**：模块装载（`require`）期间 VM 计量
//! 走 [`LoadBudget`]（[`FUEL_LOAD_BUDGET`]），与单转换预算**分离**——同一份
//! 预算被装载期与转换期共享时，模块越多、装载吃掉越多，每次转换可用步数
//! 就越少，是「文件数悄悄压低玩法预算」的隐性退化。分流依据是 require 边界
//! 的活跃计数（[`LoadBudget::begin`] / [`LoadBudget::end`]）：装载期的指令
//! 计量命中 hook 时认到装载预算，否则认转换预算；两者文案也不同（装载面
//! 报装载预算耗尽，不冒充转换预算）。不传装载预算的 [`install`] 装载期预算
//! 为 0（`Sandbox` + `install` 的直用路径不装模块表，装载期面不激活）。

use std::cell::Cell;
use std::rc::Rc;

use mlua::{Error, HookTriggers, Lua, VmState};
use ygg_core::transition::{FuelExhausted, FuelMeter};

/// hook 触发粒度：每 N 条 VM 指令扣减一次预算（research §2 开销权衡；
/// S12 实测定档：n=1000 开销比 ≈×2.0、n=100 ≈×2.6——默认保留 1000，
/// 数据记录见 tests/probe_determinism.rs 探针输出）。
pub const FUEL_HOOK_GRANULARITY: u64 = 1_000;

/// VM 面耗尽报错文案（正式语义；测试锚定 "fuel exhausted" 前缀）。
pub const FUEL_EXHAUSTED_MESSAGE: &str =
    "fuel exhausted: transition budget consumed (instruction meter)";

/// 装载期燃料预算（模块装载面独立预算；与 [`DEFAULT_FUEL_BUDGET`] 分离，
/// 模块数量因此不影响每次转换的可用步数）。50M 指令量级：足以装载
/// [`crate::modules::MAX_MODULES`] 个常规规模模块，同时仍把装载期钉在有限
/// 时间内（1 MiB 单模块上限 × 128 的极端组合也只跑到预算为止）。
///
/// [`DEFAULT_FUEL_BUDGET`]: crate::driver::DEFAULT_FUEL_BUDGET
pub const FUEL_LOAD_BUDGET: u64 = 50_000_000;

/// 装载期耗尽文案（与转换期文案分开：两者的放大路径不同，混用会误导排障）。
pub const FUEL_LOAD_EXHAUSTED_MESSAGE: &str =
    "fuel exhausted: module load budget consumed (instruction meter)";

/// r13 预算自动扩缩：每条 KV 条目的预算增量。
///
/// 标定依据（r13 方案 §0.2，本局 unify-map-and-agent-seed 只读实测）：kv_len≈292
/// 的局面 `requirement.list` 在 100k 地板下必死，~340–411 区间在 500k 下放行
/// → 全量 list 渲染 ≈ 1.22k VM 指令/kv 条目（含 fill 模板 gsub、每行 trace 的
/// KV 桥调用）。斜率取 2000/条 ≈ 实测值的 ~1.6× 安全系数：seq 39 局面
/// （kv_len=292）预算 584k > 已证放行点 500k，seq 79 局面（kv_len=450）900k。
/// 误差留痕：配了真正的 mod 工作负载后若实测最小放行预算与本斜率偏差 >2×，
/// 先调本常量并在本注释补标定数据（测试 `autoscale_slope_calibration_documents_per_entry_cost`
/// 输出同形工作负载的指令级实测，供复核）。
pub const FUEL_AUTOSCALE_PER_ENTRY: u64 = 2_000;

/// r13 预算自动扩缩：硬顶。扩缩只升不降的另一端——顶格后死循环击杀仍有限
/// （4M 指令 ÷ [`FUEL_HOOK_GRANULARITY`] = 4k 次 hook；S12 实测 hook 开销比
/// ≈ ×2.0，`while true` 在顶格预算下仍是毫秒–十毫秒级被杀）：击杀语义
/// 保留、上限仍有限，死循环保护不因扩缩而弱化。覆盖 kv_len ≤ 2000（≈120+
/// 条累计需求，远超 req_capacity=32 的 open 操作域；只被常驻超长局的历史
/// 累积触及——超长局的根治是 mod 侧历史归档，见 r13 方案 §4）。
pub const FUEL_AUTOSCALE_CAP: u64 = 4_000_000;

/// r13 预算自动扩缩公式：**线性 + 地板 + 硬顶**（纯函数，独立表测）。
///
/// ```text
/// auto_budget(kv_len) = clamp(2000 × kv_len, 100_000, 4_000_000)
/// ```
///
/// - 地板 = [`crate::driver::DEFAULT_FUEL_BUDGET`]：空局/小局行为零变化
///   （auto(0) = 地板，既有击杀/回滚测试不感知）。
/// - 状态规模因子取 `kv.len()`（canonical KV 条目总数，`BTreeMap::len`
///   O(1)，在转换起点现读）：它是 **mod 无关的充分统计**——每需求 ≈17 键、
///   每提案 ≈8–11 键，宿主不读 `req.count` / 不扫 `prop.*.status`（域键
///   约定只归 mod，P3 宿主零语义）。§0.2 实测证明 kv_len 与撞墙/放行点强
///   相关（292 死 / ~411 活）。历史 done 需求计入 kv_len 只会**高估**预算，
///   永不误杀合法转换（方向性偏置安全）。
/// - 确定性：预算只依赖转换起点的 canonical 状态规模，回放/续局重建同
///   状态 → 同预算序列；预算不入 save、不入状态哈希，只影响「杀或不杀」，
///   与现有 env 语义同类。
pub fn auto_budget(kv_entries: usize) -> u64 {
    // 饱和乘：纯函数对全输入域无 panic（usize::MAX × 2000 溢出 → 钳到 cap）。
    let scaled = (kv_entries as u64).saturating_mul(FUEL_AUTOSCALE_PER_ENTRY);
    crate::driver::DEFAULT_FUEL_BUDGET
        .max(scaled)
        .min(FUEL_AUTOSCALE_CAP)
}

/// 运行期预算解析（r13 纯函数化，表测友好——避免测试进程 env 竞态）：
///
/// - **显式 env 最优先、精确采用、不受 cap 削减**：`YGG_FUEL_BUDGET` 为正
///   整数时逐字节生效（含小于地板与大于硬顶的值——逃生门语义，r13 红线）；
/// - **缺省或非法 → 自动扩缩**：回落 [`auto_budget`]（≥ 地板，只升不降）。
///   这是相对旧契约的唯一行为变化：旧回落值是常量 100k，现在是随状态规模
///   的 `[100k, 4M]` 有界区间。
///
/// 解析细节与旧 [`crate::driver::fuel_budget`] 逐字节同形：`trim` 后 `u64`
/// 解析、`0` 视为非法（`filter(> 0)`）。`kv_entries = 0` 时本函数与旧
/// `fuel_budget()` 行为逐字节一致（auto(0) = 地板）。
pub fn resolve_budget(env_raw: Option<&str>, kv_entries: usize) -> u64 {
    match env_raw
        .map(|raw| raw.trim().parse::<u64>().ok())
        .flatten()
        .filter(|value| *value > 0)
    {
        Some(explicit) => explicit,
        None => auto_budget(kv_entries),
    }
}

/// 装载期预算句柄（纯 Rust，无 Lua）：`Rc<Cell>` 预算 + 活跃计数。
///
/// 活跃计数 >0 即「正处在 require 边界内」——hook 据此把指令计量记到装载
/// 预算而不是转换预算（嵌套 require 逐层计数，整棵依赖树都算装载期）。
#[derive(Clone)]
pub struct LoadBudget {
    remaining: Rc<Cell<u64>>,
    depth: Rc<Cell<u32>>,
}

impl LoadBudget {
    /// 以 `units` 为上限构造（`0` = 装载期面不激活：预算恒为 0，模块 chunk 一旦
    /// 触发指令计量（每 [`FUEL_HOOK_GRANULARITY`] 条）即判装载预算耗尽）。
    pub fn new(units: u64) -> Self {
        Self {
            remaining: Rc::new(Cell::new(units)),
            depth: Rc::new(Cell::new(0)),
        }
    }

    /// 进入一层模块装载（与 [`LoadBudget::end`] 成对）。
    pub fn begin(&self) {
        self.depth.set(self.depth.get().saturating_add(1));
    }

    /// 退出一层模块装载。
    pub fn end(&self) {
        self.depth.set(self.depth.get().saturating_sub(1));
    }

    /// 是否处于装载期（hook 的预算分流依据）。
    pub fn is_active(&self) -> bool {
        self.depth.get() > 0
    }

    /// 扣减装载预算；返回扣减后余额（饱和于 0）。
    pub fn spend(&self, units: u64) -> u64 {
        let left = self.remaining.get().saturating_sub(units);
        self.remaining.set(left);
        left
    }

    /// 当前剩余（诊断面 / 测试观测）。
    pub fn remaining(&self) -> u64 {
        self.remaining.get()
    }
}

/// 共享预算句柄：内核 charge 面与 VM hook 面经 `Rc<Cell<u64>>` 消费同一单元。
///
/// `Clone` 共享同一底层预算（克隆体扣减的是同一份燃料）；实现 core 侧
/// [`FuelMeter`]（D-继承-4 trait 反转：core 定义面、sandbox 实现面），
/// S10 驱动以 `&mut dyn FuelMeter` 接入 `run_transition`。
#[derive(Clone)]
pub struct HookFuelMeter {
    budget: Rc<Cell<u64>>,
}

/// 在 Lua 状态上安装正式燃料计量并返回内核侧消费面。
///
/// 必须在 `Sandbox` 构造之后调用（S7 起沙箱构造不再内建任何 hook，本函数
/// 是唯一计量安装点）；重复调用会以新预算替换旧 hook（mlua set_hook 语义），
/// 返回的新句柄与旧句柄不共享预算——M0 单状态单燃料，不提供多计量叠加。
///
/// 本形态不接装载预算（装载期面按 0 预算关闭：模块 chunk 一旦触发指令计量
/// 即判耗尽）；模块装载面走 [`install_with_load_budget`]，把沙箱
/// `ModuleHandle` 的装载预算接进来。
pub fn install(lua: &Lua, budget: u64) -> Result<HookFuelMeter, Error> {
    install_with_load_budget(lua, budget, LoadBudget::new(0))
}

/// 在 Lua 状态上安装正式燃料计量，并把模块装载预算接进同一 hook。
///
/// `load` 必须是模块装载面（`modules::ModuleHandle::load_budget`）持有的同一
/// 句柄——hook 靠它判相位，换一个全新句柄则活跃计数永远为 0，装载期会退回
/// 吃转换预算（正是本函数要消除的隐性退化）。
pub fn install_with_load_budget(
    lua: &Lua,
    budget: u64,
    load: LoadBudget,
) -> Result<HookFuelMeter, Error> {
    let meter = HookFuelMeter {
        budget: Rc::new(Cell::new(budget)),
    };
    let counter = Rc::clone(&meter.budget);
    let load_counter = load.clone();
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(FUEL_HOOK_GRANULARITY as u32),
        move |_, _| {
            // 装载期：记独立预算（转换预算一单位不动）。
            if load_counter.is_active() {
                return if load_counter.spend(FUEL_HOOK_GRANULARITY) == 0 {
                    Err(Error::runtime(FUEL_LOAD_EXHAUSTED_MESSAGE))
                } else {
                    Ok(VmState::Continue)
                };
            }
            let left = counter.get().saturating_sub(FUEL_HOOK_GRANULARITY);
            counter.set(left);
            if left == 0 {
                Err(Error::runtime(FUEL_EXHAUSTED_MESSAGE))
            } else {
                Ok(VmState::Continue)
            }
        },
    )?;
    Ok(meter)
}

impl HookFuelMeter {
    /// 转换开始时重置预算（D-B1：每转换共享——同一转换的派发税与指令计量
    /// 共享消耗，转换边界全额刷新）。S10 驱动在每次 `run_transition` 前调用。
    pub fn reset(&self, budget: u64) {
        self.budget.set(budget);
    }

    /// 当前剩余预算（诊断面 / 测试观测）。
    pub fn remaining(&self) -> u64 {
        self.budget.get()
    }
}

impl FuelMeter for HookFuelMeter {
    /// 内核派发税：从共享预算扣减 `units`，不足即拒绝（不预支）。
    fn charge(&mut self, units: u64) -> Result<(), FuelExhausted> {
        let left = self.budget.get();
        if units > left {
            return Err(FuelExhausted {
                requested: units,
                remaining: left,
            });
        }
        self.budget.set(left - units);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::DEFAULT_FUEL_BUDGET;
    use crate::sandbox::Sandbox;

    /// 构造沙箱并安装指定预算的正式燃料。
    fn sandbox_with_fuel(budget: u64) -> (Sandbox, HookFuelMeter) {
        let sb = Sandbox::new().expect("sandbox construction");
        let meter = install(sb.lua(), budget).expect("fuel install");
        (sb, meter)
    }

    #[test]
    fn infinite_loop_is_killed_by_formal_fuel() {
        let (sb, meter) = sandbox_with_fuel(10_000);
        let msg = sb
            .lua()
            .load("while true do end")
            .exec()
            .expect_err("infinite loop must be killed")
            .to_string();
        assert!(
            msg.contains("fuel exhausted"),
            "formal fuel message expected: {msg}"
        );
        // S7 临时 hook 已移除：文案不得再出现 provisional 口径（双计量防线）。
        assert!(
            !msg.contains("provisional"),
            "provisional hook must be gone from Sandbox::build: {msg}"
        );
        assert_eq!(meter.remaining(), 0, "shared budget must be drained");
    }

    #[test]
    fn charge_is_the_fine_grained_dispatch_tax() {
        let (sb, mut meter) = sandbox_with_fuel(1_000);
        meter.charge(400).expect("charge within budget");
        assert_eq!(meter.remaining(), 600);
        meter.charge(600).expect("charge exact remainder");
        assert_eq!(meter.remaining(), 0);
        // 超支拒绝：不预支，requested/remaining 原样上报（core 信封语义）。
        let err = meter.charge(1).expect_err("overdraft must be rejected");
        assert_eq!(err.requested, 1);
        assert_eq!(err.remaining, 0);
        // charge 面拒绝不影响 VM 短 chunk 执行（量化声明：低于粒度不触发 hook）。
        let v: i64 = sb.lua().load("return 21 * 2").eval().unwrap();
        assert_eq!(v, 42);
    }

    #[test]
    fn both_faces_consume_one_shared_budget() {
        let (sb, meter) = sandbox_with_fuel(10_000);
        // 内核面先扣 8000：VM 面只剩 2000（两次粒度触发即耗尽）。
        let mut kernel_face = meter.clone();
        kernel_face.charge(8_000).expect("kernel dispatch tax");
        let msg = sb
            .lua()
            .load("while true do end")
            .exec()
            .expect_err("vm face must drain the remainder")
            .to_string();
        assert!(msg.contains("fuel exhausted"), "shared budget: {msg}");
        assert_eq!(
            kernel_face.remaining(),
            0,
            "clone shares the same underlying budget"
        );
    }

    #[test]
    fn reset_re_arms_the_shared_budget() {
        let (sb, meter) = sandbox_with_fuel(1_000);
        let first = sb
            .lua()
            .load("while true do end")
            .exec()
            .expect_err("first budget must exhaust")
            .to_string();
        assert!(first.contains("fuel exhausted"));
        meter.reset(1_000);
        assert_eq!(meter.remaining(), 1_000);
        let second = sb
            .lua()
            .load("while true do end")
            .exec()
            .expect_err("reset must re-arm, not disable, the meter")
            .to_string();
        assert!(
            second.contains("fuel exhausted"),
            "same shared Cell must still guard after reset: {second}"
        );
    }

    #[test]
    fn vm_face_is_quantized_to_hook_granularity() {
        // 量化声明锚：预算耗尽后，短于粒度的 chunk 照常执行（hook 不触发）；
        // 细粒度保护语义由 charge 派发税承担——两面对照即文档。
        let (sb, mut meter) = sandbox_with_fuel(1_000);
        sb.lua()
            .load("local s = 0 for i = 1, 100 do s = s + i end return s")
            .exec()
            .expect("short loop below granularity runs fine");
        meter.charge(1_000).expect("drain budget via kernel face");
        let v: i64 = sb.lua().load("return 1 + 1").eval().unwrap();
        assert_eq!(v, 2, "quantized vm face: sub-granularity chunk unaffected");
    }

    // ── r13 预算自动扩缩：公式表测（纯函数，无 env / 无进程状态）──

    #[test]
    fn auto_budget_table_floor_linear_cap() {
        // 地板：空局与小局都 = 契约默认（小局行为零变化）。
        assert_eq!(auto_budget(0), 100_000);
        assert_eq!(auto_budget(1), 100_000);
        assert_eq!(auto_budget(50), 100_000); // 50×2000 = 100k，恰在地板上
        // 线性段：斜率 = FUEL_AUTOSCALE_PER_ENTRY。
        assert_eq!(auto_budget(51), 102_000);
        assert_eq!(auto_budget(292), 584_000); // r13 方案 §0.2 seq 39 标定点
        assert_eq!(auto_budget(411), 822_000); // seq 68 标定点
        assert_eq!(auto_budget(450), 900_000); // seq 79 标定点
        // 硬顶：恰在顶点与远超顶点都钳在 cap（顶格后击杀仍有限）。
        assert_eq!(auto_budget(2_000), FUEL_AUTOSCALE_CAP);
        assert_eq!(auto_budget(10_000), FUEL_AUTOSCALE_CAP);
        assert_eq!(auto_budget(usize::MAX), FUEL_AUTOSCALE_CAP);
        // 单调不减：任意升序对，预算不降（历史累积只高估、永不误杀）。
        let mut prev = 0;
        for entries in [0, 10, 100, 500, 1000, 1999, 2000, 5000, 100_000] {
            let budget = auto_budget(entries);
            assert!(budget >= prev, "auto_budget must be monotonic");
            assert!(budget >= 100_000 && budget <= FUEL_AUTOSCALE_CAP);
            prev = budget;
        }
    }

    #[test]
    fn resolve_budget_table_env_first_fallback_autoscale() {
        // 显式 env 最优先：任意值逐字节采用，含小于地板与大于硬顶
        // （不受 cap 削减——逃生门语义，r13 红线）。
        assert_eq!(resolve_budget(Some("123"), 0), 123);
        assert_eq!(resolve_budget(Some("123"), 5000), 123);
        assert_eq!(resolve_budget(Some("99999999999"), 0), 99_999_999_999);
        assert_eq!(resolve_budget(Some("50"), 10_000), 50);
        // trim 语义与旧 fuel_budget() 同形：空白包裹的合法值照常采用。
        assert_eq!(resolve_budget(Some("  500000  "), 0), 500_000);
        // 非法 → 自动扩缩：非数字 / 负号 / 0 / 空串 / 缺省。
        assert_eq!(resolve_budget(Some("abc"), 0), 100_000);
        assert_eq!(resolve_budget(Some("abc"), 1000), 2_000_000);
        assert_eq!(resolve_budget(Some("-5"), 1000), 2_000_000);
        assert_eq!(resolve_budget(Some("0"), 1000), 2_000_000);
        assert_eq!(resolve_budget(Some(""), 1000), 2_000_000);
        assert_eq!(resolve_budget(Some("   "), 1000), 2_000_000);
        assert_eq!(resolve_budget(None, 1000), 2_000_000);
        // 缺省/非法回落 = auto_budget 同值（同一公式，不是第二套计算）。
        for entries in [0usize, 50, 292, 411, 2000] {
            assert_eq!(
                resolve_budget(None, entries),
                auto_budget(entries),
                "fallback must be auto_budget({entries})"
            );
        }
        // kv_entries = 0 时与旧契约行为逐字节一致（auto(0) = 地板 = 常量）。
        assert_eq!(resolve_budget(None, 0), DEFAULT_FUEL_BUDGET);
    }

    #[test]
    fn autoscale_slope_calibration_documents_per_entry_cost() {
        // r13 标定探针（数据记录型，println 输出供复核）：装顶格预算，跑
        // 「1000 次 KV 桥调用同形」的工作负载，读 remaining() 得精确指令消耗
        // ——桥 Rust 侧不耗 VM 指令，stub 与真桥在指令计量下同价。断言双界：
        // ① 填充工作负载必须在地地板预算内完成（否则事件一开局就自死）；
        // ② 1000 条的指令成本 ×2 后必须仍低于 kv_len=1000 的扩缩预算 2M
        //   （斜率 2000/条 对该负载形态的覆盖性）。
        let (sb, meter) = sandbox_with_fuel(FUEL_AUTOSCALE_CAP);
        sb.lua()
            .load(
                r#"
                local set = function() end
                for i = 1, 1000 do set("probe." .. tostring(i), i) end
                "#,
            )
            .exec()
            .expect("bridge-shaped workload must run");
        let used = FUEL_AUTOSCALE_CAP - meter.remaining();
        println!(
            "r13 autoscale calibration: 1000 bridge-shaped calls consumed ~{used} instructions \
             ({} per entry); slope = {FUEL_AUTOSCALE_PER_ENTRY}, floor = {DEFAULT_FUEL_BUDGET:?}",
            used / 1000
        );
        assert!(
            used < DEFAULT_FUEL_BUDGET,
            "fill workload must fit the floor budget (100k): {used}"
        );
        assert!(
            used * 2 < 2_000_000,
            "1000-entry cost {used} must be covered by the scaled budget 2M (slope 2000/entry)"
        );
    }
}
