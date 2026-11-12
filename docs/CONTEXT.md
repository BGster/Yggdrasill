<!-- r2 wiki 拆分 · Commit 1 占位：本文件将在 Commit 2 改写为薄壳索引（映射表）。以下 L1–6 为原定位段 verbatim 暂存（载体自指改写列入 R4 清单）。 -->
# Yggdrasill · 领域语境、愿景、决策与核心原则（`docs/CONTEXT.md`）

> **定位**：`docs/CONTEXT.md` 是 Yggdrasill 仓库**始终最新的领域知识、愿景、原则与架构决策台账（Architectural Decision Ledger）权威文档**——记录的是**当前生效状态**，不是流水账：决策变更就原地修订对应行、概念演进就就地改写对应条目，任何变更叙事与历史都归 git 与结算产物，不进本文档。覆盖范围是**整个仓库**（内核 crates / egg CLI / Mod / serve / 工具链），按章节对应各子系统，不局限于 Mod。它与 [`docs/DESIGN.md`](DESIGN.md)（负责规定产品形态、系统行为与接口契约）共同取代了 `docs/` 下所有遗留的碎片化文档。每一次规划（`plan`）与实现（`development`）任务，都必须在提议或修改代码之前先查阅 `docs/CONTEXT.md` 与 `docs/DESIGN.md`；任何架构或契约变更都必须在同一次迭代中、在同一特性分支上同步更新这两份文档的对应章节，之后才能进入最终 `review` 结算。

---

