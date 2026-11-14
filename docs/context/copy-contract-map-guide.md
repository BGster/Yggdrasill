# Yggdrasill · mod 文案契约 · beats-map-guide

> **定位**：`beats-map-guide.json` 38 键的句式知识权威：地图视图、菜单行、人机门禁与节点注记。
> **上游**：[copy-contract.md](copy-contract.md) · **运行时投影**：`.ygg/mods/agent-seed/data/texts/beats-map-guide.json`
> **维护**：改文案先改本表再同步 mod；键增删/改域触发索引、prompt-surface 与键数同批更新。

| 键 | 句式模板（逐字） | 变量 | 协议归属 |
|---|---|---|---|
| `guide_hint` | field rules · {{title}}: {{summary}} ({{count}} rules; full text: .ygg/mods/agent-seed/data/playbook/; act narration: docs/context/workflow-acts.md) | title, summary, count | 叙述 |
| `guide_human_blocked` | [REJECTED:ACTION] `{{action}}` is a pass-type action protected by the human-action gate (proposal.accept / gate.approve / coordinate.submit / verify.submit / review.note): in guide mode only a human may pick it from the menu (reply with the index → ygg.choice_selected); direct single-shot dispatch is refused, primary state unchanged. Hand the full pendingChoice to the user and wait. Authority: docs/context/workflow-acts.md (coordinate, human-gate). | action | REJECTED:ACTION/PAYLOAD |
| `map_branch_first` | ├─ | — | 叙述 |
| `map_branch_last` | └─ | — | 叙述 |
| `map_branch_single` | ── | — | 叙述 |
| `map_edge_no_condition` | condition not shown | — | 叙述 |
| `map_edge_ready` | selectable | — | 叙述 |
| `map_foot` | always-on actions: config · map.show · requirement.add · requirement.list · lane.pool / lane.claim / lane.release (split / merge / amend / conflict / resolve are explicit main-agent commands) | — | 叙述 |
| `map_head` | [MAP] coordinator >> [{{stage}}] << ← you are here \| {{panel}} | stage, panel | 叙述 |
| `map_jobs` | -- job / lanes: {{lanes}} \| jobs {{jobs}}/{{cap}} \| pending proposals {{pending}} -- | lanes, jobs, cap, pending | 叙述 |
| `map_local_edge` |   {{index}}. {{text}} → [{{target}}] ({{state}}; {{when}}) | index, text, target, state, when | 叙述 |
| `map_local_foot` | (operation view: current node + selectable edges; map.show takes no payload) | — | 叙述 |
| `map_local_head` | [CURRENT NODE] [{{stage}}] \| {{panel}} | stage, panel | 叙述 |
| `map_mark` | ▶  | — | 叙述 |
| `map_note_accept` | (the main agent accepts serially) | — | 叙述 |
| `map_note_back` | (sent back to coordinate for rework) | — | 叙述 |
| `map_note_batch_boundary` | (batch boundary: every leaf requirement is done, no pending proposals, no conflicts) | — | 叙述 |
| `map_note_batch_settle` | (batch settle: archive -> run.batch+1 -> back to coordinate; resident) | — | 叙述 |
| `map_note_deliver` | (self-check and delivery ≠ acceptance) | — | 叙述 |
| `map_note_dropped` | (keep_first: the abandoned side becomes terminal dropped) | — | 叙述 |
| `map_note_gate_accept` | (**mandatory gate**: approve only on artifacts / gate evidence; reject keeps plan_ok → ready, an amended one falls back to planning) | — | 叙述 |
| `map_note_gate_plan` | (**mandatory gate**: approve to develop; reject goes back to planning for a new plan) | — | 叙述 |
| `map_note_integration` | (集成幕：核验 develop@<sha> 后 confirm 落完成；reject 落待开发) | — | 叙述 |
| `map_note_job` | (async job, global cap 4) | — | 叙述 |
| `map_note_lane` | (development lane; deps ready and no conflict) | — | 叙述 |
| `map_note_merge` | (sources become terminal merged; the new requirement falls back to draft) | — | 叙述 |
| `map_note_plan_submit` | (plan 提案交付即门禁：提交后立刻升起计划门禁) | — | 叙述 |
| `map_note_profiles` | [PROFILES] {{list}} (self-reported registry; the executor ledger stays the authority) | list | 叙述 |
| `map_note_profiles_more` | +{{count}} more | count | 叙述 |
| `map_note_retire` | (run retired - the only terminal edge, reachable only when run.retire is armed) | — | 叙述 |
| `map_note_run_begin` | (章程三件套齐备才开工；run.begin 后章程锁定) | — | 叙述 |
| `map_note_settle_ready` | (盘点四项齐备自动转发布——效应驱动，不是事件边) | — | 叙述 |
| `map_note_split` | (the parent becomes terminal split; children fall back to draft) | — | 叙述 |
| `map_pipeline_bypass_head` | -- bypass (outside the main pipeline) -- | — | 叙述 |
| `map_pipeline_head` | -- requirement pipeline (each requirement runs independently; jobs may run in parallel, gates and acceptance are serial) -- | — | 叙述 |
| `map_row` | {{mark}}{{node}} {{branch}}{{action}}──▶ {{target}}{{note}} | mark, node, branch, action, target, note | 叙述 |
| `map_row_leaf` | {{mark}}{{node}} {{branch}}{{text}} | mark, node, branch, text | 叙述 |
| `map_terminal` | (end of run · no out-edges) | — | 叙述 |
