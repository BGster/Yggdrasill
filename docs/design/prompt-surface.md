# Yggdrasill · 提示词面（beats）契约

> **定位**：7 个 data/texts 片段 × 404 键的职责归属与文案键分组表。
> **replaces**：DESIGN §2.12（L356–371）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../README.md) · **相关**：[identity.md](identity.md)
> **搬运**：以下正文自 DESIGN.md L356–L371 逐字迁移，不重写语义。

### 2.12 提示词面（beats）契约

叙事文案与提示词全部存放在 `data/texts/` 片段（404 个 beats 键 + core），按域分组、单一职责（P2 零硬编码；`mod.lua` 装配回 TEXTS）：

| 片段 | 键数 | 职责 |
| --- | --- | --- |
| `beats-flow.json` | 63 | 回执版式、guide/goal 提示、拒绝文案（`rejected*` / `stage_expected`）、五幕 `<节点>_common` 行文（prologue/charter/coordinate/settle/release/retired） |
| `beats-requirements.json` | 144 | 需求族文案：派发/提案/门禁/打回/依赖校验/九幕行文（含集成确认/打回与台账错位提示） |
| `beats-lanes.json` | 36 | 泳道账本、池注册（`lane.pool`）与认领/释放、分支命名拒绝、池化标签叙事（r1 分池七键） |
| `beats-map-guide.json` | 38 | 地图视图、菜单行、人机门禁提示、`map_note_*` 节点注记 |
| `beats-settlement.json` | 54 | 盘点四项、批结算、退役、发布幕文案；批结算自动维护追加五键（D-MA25）；tools 机证七键（tools.root 注册/脚本缺失/机证通过与被拒/未注册提醒/sessions-parent 与 todo-session 缺失）（D-MA26） |
| `beats-harness.json` | 69 | 配置面板、身份注册、工具桥提示、`subagent_submit_cmd` 模板；tools 注册面七键（root 注册/清除、name 与 param 注册、形状拒绝）（D-MA26） |
| `core.json` | — | `scenes`（5）/ `endings`（1）/ `prompts`（5）/ `options`（27）/ `tuning`（8） |

文案键归属随微状态走（§2.7 的拒绝文案、各幕行文键在 `beats-requirements` / `beats-settlement`）；立局/集成/发布三幕的文案键已随代码任务落位并与本表对齐（闭环纪律：代码引用键 ⊆ 本表键，动态拼接键——`<phase>_common`、`map_note_*`、`*_misplaced`——按拼接/数据链核销）。

### 正文归属列（r2「mod 剧情提示词迁出」新增）

mod 职责收缩为「状态管理 + 需求看板 + CONTEXT 索引提示」后，文案的**知识权威**在 `docs/context/` 运行流程层四件，mod `data/texts/` 是运行时投影（键面 390+core 不变，宿主零改动）：

| 知识形态 | 键范围 | 正文权威 | mod 侧形态 |
| --- | --- | --- | --- |
| 状态行文（错误码/回执/台账/面板/菜单，413 键） | 全部 beats 键（除右列 23 处）+ core options/tuning/endings | [../context/copy-contract.md](../context/copy-contract.md)（句式契约表，逐字一致） | 原文留 mod（宿主无文案缺省机制，运行时物理必须） |
| 幕台词与阶段策略（6 `*_common` + scenes 5 + prompts 5 + guide 2） | `prologue/charter/coordinate/settle/release/retired_common`、`scenes.*`、`prompts.*`、`guide_human_blocked`、`guide_hint` | [../context/workflow-acts.md](../context/workflow-acts.md) | 收缩为「状态前缀/{{变量}}头 + docs/context 指针」 |
| 派发仪式（3 键） | `subagent_brief`、`config_opened`、`subagent_fallback_note` | [../context/dispatch-ritual.md](../context/dispatch-ritual.md) | 事实头留 mod + 指针 |
| 清理政策（2 键） | `settlement_sessions_unsetted_hint`、`settlement_todo_segment` | [../context/cleanup-policy.md](../context/cleanup-policy.md) | 判据短句 + 指针 |

单向同步纪律（[../wiki-governance.md](../wiki-governance.md)）：改文案先改 context 权威文件再同步 mod 键值；指针锚点存在性由 `agent_seed_semantics` 的 `narrative_pointer_anchors_exist_in_docs_context` 用例机器闭环。

