<!-- r2 wiki 拆分 · Commit 1 占位：本文件将在 Commit 2 改写为薄壳索引（映射表）。以下为原 L1–9 定位段与 L733–739 §4 verbatim 暂存（§4 将迁入 docs/wiki-governance.md 并改写载体指称，列入 R4 清单）。 -->
# Yggdrasill · 系统架构、行为语义与接口契约（`docs/DESIGN.md`）

> **定位**：`docs/DESIGN.md` 规定 Yggdrasill **始终最新的产品形态、系统行为、状态机语义与规范接口契约**——它是当前设计方案的权威快照，不是流水账：变更就地在对应章节改写，历史归 git 与结算产物，不进本文档。覆盖范围是**整个仓库**，按子系统分章节：内核（§3.1 Lua 宿主 API）、Mod（§3.2 包契约）、宿主 CLI/Daemon（§3.5 线协议）、serve 观察面（§3.7）等各有其章，修改哪个子系统就更新哪个章节。领域词汇、愿景、架构原则与决策台账（`D1–D20` 与 `D-MA` 系列台账，上界随台账顺延不写死）见 [`docs/CONTEXT.md`](CONTEXT.md)。为使本文档在实现重构下保持持久，其契约定义在边界层（**JSON Schema、CLI/JSON-RPC 线协议、Lua 沙箱宿主 API、事件 Payload、状态迁移表**），而非内部源码函数名或行号。
>
> **术语速览**：领域术语的完整定义见 [CONTEXT §2 核心领域词汇](CONTEXT.md#2-核心领域词汇)，本文档不再重复维护。
>
> **动作面分层**：Layer 2 的工具参数面契约（`ygg_session.action` 与 `event` / `op` 的分层、三类落点与硬不变量）见 [§3.4](#34-三层动作面契约event--op--action)。

---

<!-- —— 以下为原 §4 活文档闭环（L733–739）verbatim 暂存 —— -->

## 4. 活文档闭环

`docs/CONTEXT.md` 与 `docs/DESIGN.md` 在每次运行中都作为闭环契约强制执行：

1. **执行前注入**：Mod 的 `subagent_brief` 模板向每个 `plan` 与 `development` 子代理 brief 注入强制指令：*"在提议或编辑之前，先阅读 `docs/CONTEXT.md`（领域不变量）与 `docs/DESIGN.md`（架构与契约）。"* 以及 Doc sync 职责（见下）。
2. **执行中同步（Doc sync）**：两份文档是**整个仓库始终最新的设计原则与设计方案权威**（内核 / CLI / Mod / serve / 工具链分章节），不是流水账。子代理职责分阶段：`plan` 阶段在其 plan artifact 中声明本次变更触碰哪份文档的哪些章节；`development` 阶段在同一特性分支上就地更新对应章节（决策与领域词汇 → `docs/CONTEXT.md` 决策表行/词汇条目；行为与契约 → `docs/DESIGN.md` 对应子系统章节），不追加任何变更叙事。纯缺陷修复与措辞调整不触碰文档。
3. **结算前核验**：主会话在 `accept_gate` 核验文档更新已随特性分支集成（决策类需求对应 `D-MAx` 行、契约类需求对应章节），并在每批的 `review` 盘点结算（常驻局的批收口或退役）前复查两份文档仍与当前代码一致——`settlement.evolution` 前是最后期限。
