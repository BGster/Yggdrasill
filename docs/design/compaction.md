# Yggdrasill · 上下文压缩与检查点交接契约

> **定位**：显式压缩双入口、三级水位线与稳定检查点谓词（r9 批边界天然检查点）。
> **replaces**：DESIGN §3.6（L590–605）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../README.md) · **相关**：[action-faces.md](action-faces.md) · [cli-daemon.md](cli-daemon.md)
> **搬运**：以下正文自 DESIGN.md L590–L605 逐字迁移，不重写语义。

### 3.6 上下文压缩与检查点交接契约

> 压缩是**显式**的：它只在 `action=handoff_compact` 与 `ygg_run_lanes` 的编排层刹车两个入口发生；`dispatch` 只报告水位，不触发压缩（见 §3.4）。

长时间的多需求运行通过 Layer 2 中的确定性压缩水位线来防护上下文腐化：
- **有效窗口上限**：默认 `min(modelWindow, 200_000)` token（可经 `YGG_EFFECTIVE_CONTEXT_WINDOW` 覆盖）。
- **水位线**：
  - `ratio < 0.60` $\to$ `normal`
  - `0.60 <= ratio < 0.75` $\to$ `soft_brake`（先让当前活跃子代理波在其门禁处落地，再写交接并压缩）
  - `ratio >= 0.75` $\to$ `hard_brake`（不再派生新子代理；立即写交接并压缩）
- **稳定检查点谓词**：仅当 `summary.available == true` $\land$ `pendingTools == 0` $\land$（`gates.length > 0` $\lor$ `jobs.length == 0` $\lor$ `conflicts.length > 0`）时才允许压缩，除非经 `handoff_compact` 显式强制。r9 常驻语义下，**批边界是天然稳定检查点**（批收口后 `stage=coordinate ∧ jobs=0 ∧ gates=0 ∧ conflicts=0`，谓词常真）；常驻长局推荐在批边界做压缩交接。
- **r9 常驻批排空暂停（drive 泵）**：`summary.lifecycle == "resident"` $\land$ `summary.batchDrained` $\land$ inbox 无未消费行 → 打印等待文案后安全暂停（exit 0）：空批绝不连点 `coordinate.submit`；`lifecycle` 缺失（旧 mod）不触发。
- **交接产物（`artifacts/handoff-latest.md`）**：在压缩前持久化进 `run.sqlite` VFS 并在压缩后重新注入，包含运行 ID、阶段、token 比例、任务表（含 `executor` / `planExecutor`）、待决门禁、未解决冲突、VFS 产物清单，以及精确的下一步 `ygg_session` / `ygg_run_lanes` 命令。

---

