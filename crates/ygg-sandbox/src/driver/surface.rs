use super::KernelFacade;
use crate::resources::ResourceTable;
use crate::sandbox::{StepError, StepReport};

impl KernelFacade {
    /// canonical 状态字节（世界帧规范字节：同一状态 ⇒ 同一字节；
    /// 哈希与存档共用这一个序列化面）。
    pub fn canonical_state_bytes(&self) -> Result<Vec<u8>, StepError> {
        self.sandbox.snapshot()
    }

    /// 装载的 Mod 源码（诊断只读面）。
    pub fn mod_source(&self) -> &str {
        &self.mod_source
    }

    /// 只读资源表（S11 诊断面；资源不可变，无写面外溢）。
    pub fn resources(&self) -> &ResourceTable {
        &self.resources
    }

    /// 注入或卸载世界内核实例（宿主面：载入特定存档、或换入测试种子）。
    ///
    /// 默认已在装载时挂载 `Sim::new(KERNEL_RNG_SEED)`；未挂载时 `world.*`
    /// 与 [`KernelFacade::step`] 报 `YGG-SIM-UNMOUNTED`。
    pub fn set_sim(&mut self, sim: Option<Box<ygg_sim::Sim>>) {
        self.sandbox.set_sim(sim);
    }

    /// 取世界内核句柄（宿主批量帧走这条，不经 Lua 边界）。
    pub fn sim_handle(&self) -> std::rc::Rc<std::cell::RefCell<Option<Box<ygg_sim::Sim>>>> {
        self.sandbox.sim_handle()
    }

    /// **宿主推进时间**：推进 `ticks` 个 tick，每 tick `dt` 整数秒
    /// （见 [`crate::Sandbox::step`] 的单次上限与错误码）。
    ///
    /// Lua 侧没有任何推进时钟的入口——脚本决定一个 tick 里算什么，
    /// 宿主决定走多少 tick；这就是可中断性与时间所有权的分界。
    pub fn step(&mut self, ticks: u32, dt: u32) -> Result<StepReport, StepError> {
        self.sandbox.step(ticks, dt)
    }

    /// 当前世界时钟（内核 tick）。
    pub fn tick(&self) -> Result<u64, StepError> {
        self.sandbox.tick()
    }
}
