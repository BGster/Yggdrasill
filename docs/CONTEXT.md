# Yggdrasill · 领域语境、愿景、决策与核心原则（薄壳索引）

> **定位**：`docs/context/*` 树的入口薄壳——原单文件已按 r2 拆分为 LLM Wiki，本文件保留定位职责并承载旧式 `§N` 引用的解析映射。
> **replaces**：原 CONTEXT.md 全文（L1–189 @ `3a2536f`），逐节映射见下表。
> **上游**：[README.md](README.md) · **相关**：[DESIGN.md](DESIGN.md)（系统设计树薄壳）

**定位（载体改写，语义保留）**：本树是仓库**始终最新的领域知识、愿景、原则与架构决策台账（Architectural Decision Ledger）权威**——记录**当前生效状态**，不是流水账：决策变更就地在台账文件修订对应行、概念演进就地改写对应词条，任何变更叙事与历史都归 git 与结算产物。覆盖范围是**整个仓库**（内核 crates / egg CLI / Mod / serve / 工具链）。它与 [DESIGN.md](DESIGN.md)（产品形态、系统行为与接口契约）共同以受索引约束的 wiki 章节树取代碎片化文档。每一次规划（`plan`）与实现（`development`）任务，都必须在提议或修改代码之前先查阅本树与设计树；任何架构或契约变更都必须在同一次迭代中、在同一特性分支上同步更新对应主题文件（见 [wiki-governance.md](wiki-governance.md)），之后才能进入最终 `review` 结算。

**定位扩展（r2「mod 剧情提示词迁出」；r1 重切为召回树）**：本树同时是**无 egg 环境自洽可读的工作流底座**——除领域知识与台账外，运行流程层包括 [context/README.md](context/README.md)（召回入口：时点 → 文件）、[context/acts/](context/acts/)（主流程幕进行时）、[context/req-acts/](context/req-acts/)（需求级九幕进行时）、[context/rituals/dispatch.md](context/rituals/dispatch.md)（派发仪式）、[context/rituals/cleanup.md](context/rituals/cleanup.md)（清理政策）、[context/rituals/fallback.md](context/rituals/fallback.md)（无 egg 执行章）与 [context/scenes/README.md](context/scenes/README.md)（七域文案契约索引：409 beats + core 36 = 445 唯一键，mod `data/texts/` 是运行时投影）。只读本树（含 [README.md](README.md) 导航）即可理解并执行工作流，且能预知每个动作的文案行为；装 egg 后由 mod 提供状态监控、历史记录与强制流程合规——语义与文案知识均以本树为权威源。

## 拆分映射表（旧章节 → 新文件）

| 旧章节 | 新文件 | 内容 |
| --- | --- | --- |
| 术语速览（Glossary） | [context/cold-start/vocabulary.md](context/cold-start/vocabulary.md) | 9 行速查表 |
| §1 产品愿景与双重身份 | [context/cold-start/vision.md](context/cold-start/vision.md) | 统一命题与双重身份 |
| §2 核心领域词汇 | [context/cold-start/vocabulary.md](context/cold-start/vocabulary.md) | 12 条权威领域定义 |
| §3 架构与运营原则 | [context/cold-start/principles.md](context/cold-start/principles.md) | P1–P11 + PLAY 注记 |
| §4 Monorepo 模块拓扑与依赖白名单 | [context/cold-start/topology.md](context/cold-start/topology.md) | 目录树 + 5 条硬性依赖不变量 |
| §5 引言（台账维护规则） | [wiki-governance.md](wiki-governance.md) | 三职责（注入 / 同步 / 核验）——即原“台账维护规则”的现行形态 |
| §5.1 核心引擎与平台决策 | [design/README.md](design/README.md)（模块 → 文件） | 裁决已解散：逐条落在它治理的契约文件的「设计依据」段（内核 / host / mod） |
| §5.2 Agent 治理决策 | [wiki-governance.md](wiki-governance.md) + [design/mod/identity.md](design/mod/identity.md) | 治理法则落 wiki-governance；身份与权限落 identity.md |

## 旧引用解析注记

- 旧式 `§N` 引用经两个薄壳的映射表解析，仍有效。
- 决策不再有编号，也没有台账：要查“为什么这么定”就看它治理的那份文件的「设计依据」段（规范 + 依据 + 三问判据）。反例：当年两泳道从同一 base 各自取下一个号，造成双重认领（证据 `d0b4a7c` / `6848138`）——这正是“不取号”的理由。
- 本表与 [README.md](README.md) 导航表同源（r2 Commit 2 生成）；树变化时两处同步更新（核对项见 wiki-governance）。
