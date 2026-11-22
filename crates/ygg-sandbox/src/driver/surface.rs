use ygg_core::snapshot::{canonical_snapshot, SnapshotError};

use super::KernelFacade;
use crate::host_api::{HostToolRunner, MountFs};
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

    // ---- `ygg.fs` 的宿主机侧面（预载 / 落盘水位 / 诊断）----

    /// 预载一件 VFS 文件（宿主开局从 `RunStore::artifact_get` 回灌）。
    /// **不置 dirty / 不动 revision**：它不产生新的落盘需求。
    pub fn vfs_load(&mut self, path: &str, content: &[u8], sha256: &str) {
        self.vfs.borrow_mut().load(path, content, sha256);
    }

    /// 宿主侧写 VFS（`--artifact-put` 预载 / 宿主直接落地）。返回摘要。
    ///
    /// 与 Lua 写面**同一道门**：错误串已是可读原因。
    pub fn vfs_put(&mut self, path: &str, content: &[u8]) -> Result<String, String> {
        self.vfs
            .borrow_mut()
            .put(path, content)
            .map(|file| file.sha256)
    }

    /// 全量条目（`BTreeMap` 序）——诊断面与「强制重刷」面。
    pub fn vfs_entries(&self) -> Vec<(String, Vec<u8>, String)> {
        self.vfs
            .borrow()
            .entries()
            .map(|(path, file)| (path.to_owned(), file.bytes.clone(), file.sha256.clone()))
            .collect()
    }

    /// 取走未落盘条目（排空 dirty）。宿主 flush 成功后调用。
    pub fn vfs_take_dirty(&mut self) -> Vec<(String, Vec<u8>, String)> {
        self.vfs
            .borrow_mut()
            .take_dirty()
            .into_iter()
            .map(|(path, file)| (path, file.bytes, file.sha256))
            .collect()
    }

    /// 把一批路径重新标脏（落盘中途失败后的回填：内容寻址让重试只重写相同字节）。
    pub fn vfs_requeue_dirty(&mut self, paths: &[String]) {
        self.vfs.borrow_mut().requeue_dirty(paths);
    }

    /// VFS 写代数（每次成功 `put` +1）。
    pub fn vfs_revision(&self) -> u64 {
        self.vfs.borrow().revision()
    }

    /// VFS 当前件数。
    pub fn vfs_len(&self) -> usize {
        self.vfs.borrow().len()
    }

    /// 挂载或卸载宿主侧同步工具执行器（供 `ygg.tool.call` 同步调用）。
    pub fn set_tool_runner(&mut self, runner: Option<Box<dyn HostToolRunner>>) {
        self.host.set_tool_runner(runner);
    }

    /// 挂载或卸载宿主侧文件挂载桥（供 `ygg.fs.*` 的 `mounts/` 面）；
    /// 卸载后访问 `mounts/…` 抛 `YGG-FS-UNMOUNTED`，旧面 `artifacts/` 不受影响。
    pub fn set_mount_fs(&mut self, bridge: Option<Box<dyn MountFs>>) {
        self.host.set_mount_fs(bridge);
    }

    /// 注入或卸载模拟内核实例（供 `ygg.sim.*`，Tier 1）。
    ///
    /// 未注入时 10 个函数抛 `YGG-SIM-UNMOUNTED`（与 fs 桥同一纪律：不返回 `nil`/`false`）。
    /// CLI 激活触点（`egg.rs` 的 `set_sim` 与开关）**不在此处**——宿主面改动走用户授权账。
    pub fn set_sim(&mut self, sim: Option<Box<ygg_sim::Sim>>) {
        self.host.set_sim(sim);
    }

    /// 取模拟内核句柄（宿主批量帧走这条，不经 Lua 边界）。
    pub fn sim_handle(&self) -> std::rc::Rc<std::cell::RefCell<Option<Box<ygg_sim::Sim>>>> {
        self.host.sim_handle()
    }

    /// 取实时宿主面句柄（S31 R1；常驻宿主逐帧闩锁输入快照与帧号走这条，
    /// 不经 Lua 边界）。返回与 `ygg.rt.*` 投影**同一个** `Rc`。
    pub fn rt_handle(&self) -> std::rc::Rc<std::cell::RefCell<crate::host_api::RtFrame>> {
        self.host.rt_handle()
    }
}
