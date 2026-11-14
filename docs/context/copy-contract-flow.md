# Yggdrasill · mod 文案契约 · beats-flow

> **定位**：`beats-flow.json` 63 键的句式知识权威：回执版式、模式提示、拒绝、章程与幕转场。
> **上游**：[copy-contract.md](copy-contract.md)（总索引与协议读法） · **运行时投影**：`.ygg/mods/agent-seed/data/texts/beats-flow.json`
> **维护**：改文案先改本表再同步 mod；键增删/改域触发 docs/README、prompt-surface 与本表键数同批更新。

| 键 | 句式模板（逐字） | 变量 | 协议归属 |
|---|---|---|---|
| `allowed_none` | (run closed, no actions available) | — | 叙述 |
| `bad_index` | [CHOICE OUT OF RANGE] {{stage}} has {{count}} options, got {{index}}. | stage, count, index | 叙述 |
| `bad_payload` | the payload for `{{action}}` is malformed. | action | 叙述 |
| `bad_score` | Score must be an integer 1-5, got `{{got}}`. | got | 叙述 |
| `charter_begun` | 局已开工：run.begin 只在立局微状态可用（当前「{{phase}}」） | phase | 叙述 |
| `charter_common` | 立局幕：三件套齐备后 run.begin 开工（章程随之锁定）。正文见 docs/context/workflow-acts.md（立局幕节）。 | — | 叙述 |
| `charter_locked` | 章程在 run.begin 后锁定：当前微状态「{{phase}}」不可再改章程 | phase | 叙述 |
| `charter_missing` | 章程三件套未齐备（task.title / task.goal / task.success 任一为空）：先 charter.set 补齐再 run.begin | — | 叙述 |
| `charter_ok` | [CHARTER SET] 章程已立（立局内可反复修订）：「{{title}}」｜目标：{{goal}}｜成功判据：{{success}} | title, goal, success | 叙述 |
| `charter_payload` | 任务标题\|要达成的一句话目标\|可判定的成功标准 | — | 叙述 |
| `charter_usage` | 章程三段："标题\|目标\|成功判据"——三段都必须非空 | — | 叙述 |
| `coordinate_common` | 协调幕：受理提案 → 过门禁 → 派发 → coordinate.submit。正文与人机门禁说明见 docs/context/workflow-acts.md（协调幕节）。 | — | 叙述 |
| `coordinate_ok` | [COORDINATION DONE] {{count}} 条需求全部终态——批边界，进入盘点幕（run.batch={{batch}}） | count, batch | 叙述 |
| `coordinate_usage` | coordinate.submit [] | — | 叙述 |
| `default_gate_report` | gate failed (placeholder report) | — | 叙述 |
| `default_lesson` | split small first, parallelize next, accept on evidence | — | 叙述 |
| `default_req_desc` | (no requirement description) | — | 叙述 |
| `default_req_payload` | add a requirement\|write the title and description here, pipe-separated (third segment = external idempotency id, fourth segment = deps r1,r2) | — | 叙述 |
| `default_req_title` | (untitled requirement) | — | 叙述 |
| `default_task_goal` | （未设目标） | — | 叙述 |
| `default_task_success` | （未设成功判据） | — | 叙述 |
| `default_task_title` | (untitled task) | — | 叙述 |
| `evolution_artifact_none` | (empty) | — | 叙述 |
| `evolution_auto` | evolution auto: preference marker only (cap {{cap}}, paths {{paths}}) — no auto-rewrite, no revision requests registered; suggestions still go through the separate settlement.evolution registration. | cap, paths | 叙述 |
| `evolution_classes_none` | (zero candidates, no classes) | — | 叙述 |
| `evolution_none` | 0/0 | — | 叙述 |
| `evolution_off` | evolution off: only the review is recorded; no evolution suggestions. | — | 叙述 |
| `evolution_suggest` | evolution suggest: produce suggestions (see the evolution chapter); annotated paths {{paths}} — never auto-rewritten. | paths | 叙述 |
| `gate_bypass` | [GATES BYPASSED] failure #{{failures}}, but require_all_green is off → final review allowed. | failures | 叙述 |
| `gate_charter_first` | 章程未立或需求为零：先用 event.start 载荷或 charter.set 立好章程三件套（title\|goal\|success），requirement.add 至少一条需求，才能收口 | — | 叙述 |
| `handover_goal` | [DECISION HANDOVER] play.mode = goal: the main agent reads the full receipt and then dispatches by itself (never the default first option, never a script); acceptance and both gates still take one real action and one validation each, evidence standards unchanged. The closing review must account for the delegated scope. | — | 叙述 |
| `handover_guide` | [DECISION HANDOVER] play.mode = guide: every following step is confirmed by a human (reply with the index); milestone actions still need the menu ticket. | — | 叙述 |
| `paths_none` | (none) | — | 叙述 |
| `phase_expected` | 该动作需要微状态「{{expected}}」（当前「{{current}}」） | expected, current | 叙述 |
| `play_done_goal` | [RUN · AUTONOMOUS] every requirement is closed; this run was dispatched by the main agent itself, settlement is produced by --settle. | — | 叙述 |
| `play_hint_goal` | goal: the main agent reads full receipts and dispatches by itself (gates are never skipped) | — | 叙述 |
| `play_hint_guide` | guide: every step is confirmed by you | — | 叙述 |
| `play_receipt` | [PLAY MODE] play.mode = {{play}} — {{hint}} | play, hint | 叙述 |
| `profile_bad_assignments` | assignments must be a comma-separated list of `r<n>[:<stage>]` tokens (<= 16 tokens, <= 512 chars total), e.g. `r1:plan,r1:development,r3`. | — | 叙述 |
| `profile_bad_handle` | a profile key must be `profile.<handle>.<key>` with a handle-shaped handle (non-empty after trim, <= 64 chars, no whitespace, no `\|`). | — | 叙述 |
| `profile_bad_value` | invalid profile value for this key (harness <= 32, session_id <= 128: no whitespace / no `\|` / no quotes; role <= 64 single line; free keys `^[a-z0-9_]{1,32}$` with single-line values <= 256). | — | 叙述 |
| `profile_cleared` | [PROFILE CLEARED] {{handle}}.{{field}} (self-reported registry card only). | handle, field | 叙述 |
| `profile_registered` | [PROFILE REGISTERED] {{handle}}.{{field}} (self-reported, not identity proof; the executor ledger stays the authority for accountability). | handle, field | 叙述 |
| `prologue_common` | 序幕：受理与两道门禁各需一次真实动作与核验，永不跳过；需求可异步追加。叙事与策略正文见 docs/context/workflow-acts.md（序幕节）。 | — | 叙述 |
| `rejected` | [REJECTED:ACTION] `{{action}}` is not available in stage {{stage}}. Allowed actions: {{allowed}}. Reason: {{detail}} | action, stage, allowed, detail | REJECTED:ACTION/PAYLOAD |
| `rejected_default` | (no reason given) | — | REJECTED:ACTION/PAYLOAD |
| `rejected_payload` | [REJECTED:PAYLOAD] the payload for `{{action}}` cannot be accepted: {{detail}} Primary state unchanged. | action, detail | REJECTED:ACTION/PAYLOAD |
| `release_common` | 发布幕：release.confirm（master@<sha>\|批次 tag）或 release.skip（理由），已声明即可 review.note 批结算/退役。正文见 docs/context/workflow-acts.md（发布幕节）。 | — | 叙述 |
| `retired_common` | 退役终态：业务事件全拒；map.show / requirement.list 只读查询仍可用。正文见 docs/context/workflow-acts.md（退役终态节）。 | — | 叙述 |
| `run_begun` | [RUN BEGUN] 章程锁定，任务「{{title}}」开工（{{count}} 条需求在册）→ 协调幕 | title, count | 叙述 |
| `run_retire_armed` | [RUN RETIRE ARMED] the next review.note becomes the retiring close: the run settles and ends ([ENDING · run retired]). Clear it with `config.set run.retire=""`. | — | 叙述 |
| `run_retire_cleared` | [RUN RETIRE CLEARED] review.note is back to the batch settle (resident run continues; no ending). | — | 叙述 |
| `secret_redline` | Red line: credentials live only in .env / .env.local; Mod resources, config and reports expose key names only. | — | 叙述 |
| `settle_common` | 盘点幕四项：验证 → 演进登记 → 会话清理 → 待办清理，齐备自动转发布幕。正文见 docs/context/workflow-acts.md（盘点幕节）；清理政策见 docs/context/cleanup-policy.md。 | — | 叙述 |
| `settle_ready` | [SETTLE READY] 盘点四项齐备（验证/演进登记/会话清理/待办清理）——自动进入发布幕 | — | 叙述 |
| `stage_expected` | 该动作只在微状态「{{expected}}」可用 | expected | 叙述 |
| `stage_prompt_missing` | (the prompt for this node is missing: check <node>_common in data/texts/beats-flow.json and docs/context/workflow-acts.md) | — | 叙述 |
| `unknown_choice` | [RUN-CLOSED] {{stage}} has no pending choices. | stage | 叙述 |
| `unknown_stage` | Unknown stage `{{stage}}`. | stage | 叙述 |
| `verify_fail` | [GATES FAILED] #{{failures}}: sent back to coordinate for rework. | failures | 叙述 |
| `verify_failed_record` | failed (recorded) | — | 叙述 |
| `verify_green_record` | all green (recorded) | — | 叙述 |
| `verify_ok` | [GATES GREEN] {{count}} requirements all terminal; settle act open — declare the evolution registration next (settlement.evolution, batch {{batch}}). | count, batch | 叙述 |
