# Yggdrasill · 领域语境、愿景、决策与核心原则（薄壳索引）

> **定位**：`docs/context/*` 树的入口薄壳——原单文件已按 r2 拆分为 LLM Wiki，本文件保留定位职责并承载旧式 `§N` 引用的解析映射。
> **replaces**：原 CONTEXT.md 全文（L1–189 @ `3a2536f`），逐节映射见下表。
> **上游**：[README.md](README.md) · **相关**：[DESIGN.md](DESIGN.md)（系统设计树薄壳）

**定位（载体改写，语义保留）**：本树是仓库**始终最新的领域知识、愿景、原则与架构决策台账（Architectural Decision Ledger）权威**——记录**当前生效状态**，不是流水账：决策变更就地在台账文件修订对应行、概念演进就地改写对应词条，任何变更叙事与历史都归 git 与结算产物。覆盖范围是**整个仓库**（内核 crates / egg CLI / Mod / serve / 工具链）。它与 [DESIGN.md](DESIGN.md)（产品形态、系统行为与接口契约）共同以受索引约束的 wiki 章节树取代碎片化文档。每一次规划（`plan`）与实现（`development`）任务，都必须在提议或修改代码之前先查阅本树与设计树；任何架构或契约变更都必须在同一次迭代中、在同一特性分支上同步更新对应主题文件（见 [wiki-governance.md](wiki-governance.md)），之后才能进入最终 `review` 结算。

**定位扩展（r2「mod 剧情提示词迁出」）**：本树同时是**无 egg 环境自洽可读的工作流底座**——除领域知识与台账外，新增四件运行流程层文件：[workflow-acts.md](context/workflow-acts.md)（五幕进行时台词 + 无 egg 执行章）、[dispatch-ritual.md](context/dispatch-ritual.md)（派发仪式正文）、[cleanup-policy.md](context/cleanup-policy.md)（盘点清理政策）、[copy-contract.md](context/copy-contract.md)（mod 文案契约：413 键句式模板与协议前缀归属，**文案知识权威源**，mod `data/texts/` 是其运行时投影）。只读本树（含 [README.md](README.md) 导航）即可理解并执行工作流，且能预知每个动作的文案行为；装 egg 后由 mod 提供状态监控、历史记录与强制流程合规——语义与文案知识均以本树为权威源。

## 拆分映射表（旧章节 → 新文件）

| 旧章节 | 新文件 | 内容 |
| --- | --- | --- |
| 术语速览（Glossary） | [context/vocabulary.md](context/vocabulary.md) | 9 行速查表 |
| §1 产品愿景与双重身份 | [context/vision.md](context/vision.md) | 统一命题与双重身份 |
| §2 核心领域词汇 | [context/vocabulary.md](context/vocabulary.md) | 12 条权威领域定义 |
| §3 架构与运营原则 | [context/principles.md](context/principles.md) | P1–P11 + PLAY 注记 |
| §4 Monorepo 模块拓扑与依赖白名单 | [context/topology.md](context/topology.md) | 目录树 + 4 条硬性依赖不变量 |
| §5 引言（台账维护规则） | [context/decisions-engine.md](context/decisions-engine.md) 与 [context/decisions-agent.md](context/decisions-agent.md) | 刻意双份（两表同规则） |
| §5.1 核心引擎与平台决策 | [context/decisions-engine.md](context/decisions-engine.md) | D1–D21 表（19 行，无 D2/D5） |
| §5.2 Agent 治理决策 | [context/decisions-agent.md](context/decisions-agent.md) | D-MA1–24 表 |

## 旧引用解析注记

- 决策表行内保留的旧式引用（如 D-MA9「契约正文见 `docs/DESIGN.md` §3.4」）经 [DESIGN.md](DESIGN.md) 薄壳映射表解析，仍有效。
- git 历史引用（如 D-MA16 double-claim 证据 `d0b4a7c` / `6848138`）指向本文件的历史版本，不受拆分影响。
- 本表与 [README.md](README.md) 导航表同源（r2 Commit 2 生成）；树变化时两处同步更新（核对项见 wiki-governance）。
