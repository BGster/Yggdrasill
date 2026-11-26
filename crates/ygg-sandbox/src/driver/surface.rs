use ygg_core::snapshot::{canonical_snapshot, SnapshotError};

use super::KernelFacade;
use crate::resources::ResourceTable;

impl KernelFacade {
    /// canonical 状态字节（实体表规范快照，SHA 之前的确定性序列化面；
    /// store 写入门保证非有限值不可能入场，错误仅在驱动 bug 时出现）。
    pub fn canonical_state_bytes(&self) -> Result<Vec<u8>, SnapshotError> {
        canonical_snapshot(&self.entities.borrow())
    }

    /// 装载的 Mod 源码（诊断只读面）。
    pub fn mod_source(&self) -> &str {
        &self.mod_source
    }

    /// 只读资源表（S11 诊断面；资源不可变，无写面外溢）。
    pub fn resources(&self) -> &ResourceTable {
        &self.resources
    }

    /// 注入或卸载模拟内核实例（供 `ygg.sim.*`，Tier 1）。
    ///
    /// 未注入时 10 个函数抛 `YGG-SIM-UNMOUNTED`（不返回 `nil`/`false`：
    /// `nil` 与「世界是空的」不可分辨）。
    pub fn set_sim(&mut self, sim: Option<Box<ygg_sim::Sim>>) {
        self.host.set_sim(sim);
    }

    /// 取模拟内核句柄（宿主批量帧走这条，不经 Lua 边界）。
    pub fn sim_handle(&self) -> std::rc::Rc<std::cell::RefCell<Option<Box<ygg_sim::Sim>>>> {
        self.host.sim_handle()
    }
}
