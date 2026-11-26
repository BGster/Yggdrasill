# Files

- [上下文压缩与检查点交接](compaction.md) - 显式压缩双入口、三级水位线（normal/soft_brake/hard_brake）、稳定检查点谓词、批边界天然检查点与交接产物字段。
- [事件总表](events.md) - 39+1 个事件的三种分类（剧情/操作/基础设施）、每个事件的微状态落点与 payload 契约、proposal.submit 尾段链与事件集设计依据。
- [Lua 沙箱宿主 API 契约（`ygg.*`）](lua-api.md)
- [原生模拟内核契约（engine Tier 1 · `crates/ygg-sim`）](sim-kernel.md)
- [状态机语义与微状态判定](state-machine.md)
- [状态面：存储键契约与拒绝面矩阵](state-surface.md)
- [事件 Payload 线协议契约](wire-protocol.md)
