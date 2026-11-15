# Yggdrasill · mod 文案契约 · core 与全局键账
> **id** copy/core · **幕** all · **触发** 改 `core.json` 前
> **读不到代价** silent-drift
> **ver** c0b732

> **定位**：`core.json` 的 options 27 / tuning 8 / endings 1（共 36 键），以及跨域指针壳与全局键数核对。
> **上游**：[copy/README.md](README.md) · **运行时投影**：`.ygg/mods/agent-seed/data/texts/core.json`
> **维护**：改文案先改本表再同步 mod；键增删/改域触发索引、prompt-surface 与 §H 同批更新。

## core.json（options 27 / tuning 8 / endings 1 = 36 键；scenes/prompts 散文见 §H）

### options · 菜单措辞（mod 运行时渲染选择面；语义对照见 workflow-acts.md 各幕）

| 键 | 值/句式（逐字） | 变量 | 协议归属 |
|---|---|---|---|
| `options.accept` | proposal.accept — accept pending proposal {{id}} ({{req}} / {{stage}}, {{pending}} pending) | id, req, stage, pending | 叙述 |
| `options.charter_set` | charter.set — 修订章程三件套（载荷 "标题\|目标\|成功判据"，立局内可反复修订） | — | 叙述 |
| `options.config` | config — config entry (play mode / evolution preference / gates; opens a sub-page) | — | 叙述 |
| `options.config_close` | back — return to the stage menu | — | 叙述 |
| `options.config_evolution` | config.set — evolution preference {{mode}} → {{next}} (narration only, no auto-rewrite) | mode, next | 叙述 |
| `options.config_gates` | config.set — gates require_all_green {{state}} → {{next}} | state, next | 叙述 |
| `options.config_play` | config.set — play mode {{mode}} → {{next}} | mode, next | 叙述 |
| `options.config_retire` | config.set — run.retire {{state}} → {{next}} (armed: the next review.note ends the run; off: review.note settles the batch and the run continues) | state, next | 叙述 |
| `options.coordinate_submit` | coordinate.submit — enter final review once every requirement is closed | — | 叙述 |
| `options.dispatch` | requirement.dispatch — dispatch by readiness order ({{count}} candidates, jobs {{jobs}}/{{cap}}, head {{id}}) | count, jobs, cap, id | 叙述 |
| `options.gate_approve` | gate.approve — pass the {{kind}} gate on {{id}} ({{gates}} gates left) | kind, id, gates | 叙述 |
| `options.gate_reject` | gate.reject — reject the {{kind}} gate on {{id}} and send it back for rework | kind, id | 叙述 |
| `options.map` | map.show — show the map and the stage briefing (coordinator {{stage}}) | stage | 叙述 |
| `options.release_confirm` | release.confirm — 声明本批 develop→master 合并（载荷 "master@<sha>\|v年.月.日.序号"） | — | 叙述 |
| `options.release_skip` | release.skip — 声明本批无需发布（载荷 = 非空理由） | — | 叙述 |
| `options.req_add` | requirement.add — add a requirement (payload "title\|description[\|external_id[\|depends_on]]") | — | 叙述 |
| `options.req_list` | requirement.list — show the requirement graph / proposals / lanes ({{pending}} pending, {{open}} open) | pending, open | 叙述 |
| `options.req_resolve_first` | requirement.resolve keep_first — {{ids}} keep the first, the other becomes dropped | ids | 叙述 |
| `options.req_resolve_keep` | requirement.resolve keep_both — {{ids}} keep both sides and requeue | ids | 叙述 |
| `options.req_resolve_revise` | requirement.resolve revise — {{ids}} keep both but flag needs-amend | ids | 叙述 |
| `options.review_note` | review.note — record the lesson, archive the evolution registration single-source and close the run (payload "lesson\|score") | — | 叙述 |
| `options.run_begin` | run.begin — 章程三件套齐备，开工进协调幕 | — | 叙述 |
| `options.settlement_evolution` | settlement.evolution — register the evolution review and settle sessions + todo in one transaction (payload "artifact\|summary[\|c=<kept>/<screened>[\|cls=<kind>:<n>,...]][\|sessions=<moved>/<remaining>/<destination>][\|todo=<remaining>/<verified_by>]") | — | 叙述 |
| `options.settlement_sessions` | settlement.sessions — register subagent session cleanup (payload "moved\|remaining\|destination", remaining must be 0) | — | 叙述 |
| `options.settlement_todo` | settlement.todo — register main-session todo cleanup (payload "remaining\|verified_by", remaining must be 0) | — | 叙述 |
| `options.verify_fail` | verify.submit all_green=false — report the failure honestly (sent back to coordinate) | — | 叙述 |
| `options.verify_green` | verify.submit all_green=true — gates are green (record evidence, move to review) | — | 叙述 |

### tuning · 数值契约（容量/上限，非文案但同驻 core.json，一并归一）

| 键 | 值/句式（逐字） | 变量 | 协议归属 |
|---|---|---|---|
| `tuning.artifact_capacity` | 64 | — | 数值 |
| `tuning.default_score` | 3 | — | 数值 |
| `tuning.job_capacity` | 4 | — | 数值 |
| `tuning.max_slices` | 8 | — | 数值 |
| `tuning.min_slices` | 1 | — | 数值 |
| `tuning.req_capacity` | 32 | — | 数值 |
| `tuning.tool_capacity` | 64 | — | 数值 |
| `tuning.tool_output_max` | 4096 | — | 数值 |

### endings · 终局行（[ENDING 前缀由 pack 声明）

| 键 | 值/句式（逐字） | 变量 | 协议归属 |
|---|---|---|---|
| `endings.done` | [ENDING · run retired] 任务「{{title}}」——{{count}} 条需求（{{done}} 条终态），{{batch}} 个已结算批次；复盘评分 {{score}}/5，演进候选 {{candidates}}（保留/筛除）。一句话教训：{{lesson}} | title, count, done, batch, score, candidates, lesson | [ENDING |

## H. 23 处散文键的指针壳句式（mod 收缩后形态）

以下键的叙事/策略正文归一到 context 散文接收面；mod 侧只留「状态前缀/变量头 + 指针」。指针目标即知识权威，运行时投影不得反向成为第二正文。

| 键 | 收缩后形态（指向） |
|---|---|
| `prologue_common` | docs/context/workflow-acts.md#序幕 |
| `charter_common` | docs/context/workflow-acts.md#立局幕 |
| `coordinate_common` | docs/context/workflow-acts.md#协调幕 |
| `settle_common` | docs/context/workflow-acts.md#盘点幕 |
| `release_common` | docs/context/workflow-acts.md#发布幕 |
| `retired_common` | docs/context/workflow-acts.md#退役终态 |
| `subagent_brief` | docs/context/dispatch-ritual.md（§2–§7 策略正文） |
| `config_opened` | docs/context/dispatch-ritual.md#8 |
| `subagent_fallback_note` | docs/context/dispatch-ritual.md#9 |
| `guide_human_blocked` | docs/context/workflow-acts.md#协调幕（人机门禁） |
| `guide_hint` | docs/context/workflow-acts.md#协调幕（field rules） |
| `settlement_sessions_unsetted_hint` | docs/context/cleanup-policy.md#1 |
| `settlement_todo_segment` | docs/context/cleanup-policy.md#2 |
| `scenes.*` | docs/context/workflow-acts.md 各幕场景台词 |
| `prompts.*` | docs/context/workflow-acts.md 各幕 prompts 语义 |

---

**键数核对（Q3 已绑定）**：六个 beats 片段共 **409** 键（flow 63 + harness 74 + lanes 36 + map-guide 38 + requirements 144 + settlement 54），core 的 options/tuning/endings 共 **36** 键，合计 **445 个唯一键**。§H 的 23 处指针壳全部是这 445 键的子集（beats 13 + core scenes 5 + prompts 5），**不得再次相加**；旧文「beats 390 / A–G 426 / 加 H=436」同时漏算了 14 个 tools 键并重复计算指针壳。

**Producer / trigger / check**：owner = 修改 `data/texts/*.json` 或本契约的执行者；trigger = 任一键新增、删除、改名、跨域移动，或 scenes/prompts 指针范围变化；同 commit 必须更新本段、[../design/prompt-surface.md](../../design/prompt-surface.md)、[../README.md](../../README.md) 与受影响域文件。check = `agent_seed_semantics` 的逐域键集断言、`narrative_pointer_anchors_exist_in_docs_context` 与 `docs-consistency.sh`；任何一键缺失或计数漂移都使验收失败，不得靠手改数字放行。
