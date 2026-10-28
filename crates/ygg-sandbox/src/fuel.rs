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
pub fn install(lua: &Lua, budget: u64) -> Result<HookFuelMeter, Error> {
    let meter = HookFuelMeter {
        budget: Rc::new(Cell::new(budget)),
    };
    let counter = Rc::clone(&meter.budget);
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(FUEL_HOOK_GRANULARITY as u32),
        move |_, _| {
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
}
