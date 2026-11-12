# Yggdrasill · 活文档闭环

> **定位**：docs wiki 在每次运行中作为闭环契约强制执行的三职责权威定义。
> **replaces**：DESIGN §4 活文档闭环（原 L733–739；r2 载体指称由两单文件改述为 wiki 树，三职责语义逐条保留）。
> **上游**：[README.md](README.md) · **相关**：[context/decisions-agent.md](context/decisions-agent.md)（D-MA8 活文档整合）

`docs/` 的 LLM Wiki（顶层导航 `docs/README.md`；领域与决策在 `docs/context/*`，系统行为与接口契约在 `docs/design/*`；原两单文件 `docs/CONTEXT.md` / `docs/DESIGN.md` 为薄壳索引入口）在每次运行中都作为闭环契约强制执行：

1. **执行前注入**：Mod 的 `subagent_brief` 模板向每个 `plan` 与 `development` 子代理 brief 注入强制指令：*"在提议或编辑之前，先阅读 docs wiki（`docs/README.md` 索引；`docs/context/*` 领域不变量；`docs/design/*` 架构与契约）。"* 以及 Doc sync 职责（见下）。
2. **执行中同步（Doc sync）**：wiki 是**整个仓库始终最新的设计原则与设计方案权威**（内核 / CLI / Mod / serve / 工具链按主题分文件），不是流水账。子代理职责分阶段：`plan` 阶段在其 plan artifact 中声明本次变更触碰哪些 wiki 主题文件；`development` 阶段在同一特性分支上就地更新对应文件（决策与领域词汇 → `docs/context/decisions-*.md` 台账行 / `docs/context/vocabulary.md` 词条；行为与契约 → `docs/design/` 对应子系统主题文件），不追加任何变更叙事。纯缺陷修复与措辞调整不触碰文档。
3. **结算前核验**：主会话在 `accept_gate` 核验文档更新已随特性分支集成（决策类需求对应 `D-MAx` 台账行、契约类需求对应主题文件），并在每批的 `review` 盘点结算（常驻局的批收口或退役）前复查 wiki 仍与当前代码一致——`settlement.evolution` 前是最后期限。机器侧闭环（D-MA25，r3）：每批 `review.note` 批结算自动追加一条 draft 态「维护更新 docs wiki」需求进入下一批派发队列（防重复按 `req.<i>.maintenance` 标记 + 规范标题），未完成前阻塞下一批批边界——每批一轮的需求化维护由状态机强制承载，人工核验期限不变。

**结构与维护纪律**：每主题一文件（≤300 行）、文件头部带定位 / `replaces` / 上下游索引头、双向链接；顶层导航只有 `README.md` 一处；两薄壳的映射表使旧式 `§N` 引用持续可解析（方案 A 裁决，r2）。
