# Yggdrasill · mod 文案契约 · beats-requirements

> **定位**：`beats-requirements.json` 144 键的句式知识权威：派发、提案、门禁、修订、拆分合并与台账面板。
> **上游**：[copy-contract.md](copy-contract.md) · **运行时投影**：`.ygg/mods/agent-seed/data/texts/beats-requirements.json`
> **维护**：改文案先改本表再同步 mod；键增删/改域触发索引、prompt-surface 与键数同批更新。

| 键 | 句式模板（逐字） | 变量 | 协议归属 |
|---|---|---|---|
| `integration_bad_sha` | {{id}} 集成证据非法（期望 develop@<sha>，sha 为 7-40 位十六进制；实得 {{got}}） | id, got | 叙述 |
| `integration_confirmed` | [INTEGRATION CONFIRMED] {{id}} develop@{{sha}} 已核验 → 完成（已终态 {{done}} 条） | id, sha, done | 叙述 |
| `integration_hist` | {{id}} integrated @ {{sha}} | id, sha | 叙述 |
| `integration_not_integrating` | {{id}} 不在集成幕（当前 {{status}}）：integration.* 只对集成幕的需求生效 | id, status | 叙述 |
| `integration_reject_hist` | {{id}} integration rejected | id | 叙述 |
| `integration_rejected` | [INTEGRATION REJECTED] {{id}} 集成打回 → 待开发；理由：{{reason}} | id, reason | 叙述 |
| `req_accept_hist` | v{{id}} accepted ({{stage}}) | id, stage | 叙述 |
| `req_accept_ok` | [PROPOSAL ACCEPTED] {{id}} → {{req}} / {{stage}}, state → {{status}}{{change}} \| {{pending}} still pending | id, req, stage, status, change, pending | 叙述 |
| `req_accept_plan` | {{id}}（{{req}} 的 plan 提案）的裁决走 gate.approve / gate.reject（交付即门禁）；proposal.accept 只受理 development 提案 | id, req | 叙述 |
| `req_accept_stale` | proposal {{id}} is bound to rev {{got}} while requirement {{req}} is already rev {{want}} — judged stale; rebase and resubmit (an old proposal must not override a newer decision). | id, got, req, want | 叙述 |
| `req_added` | [REQ ADDED] {{id}} "{{title}}" {{deps}} — {{counts}} | id, title, deps, counts | 叙述 |
| `req_after_done` | This run is closed: requirements need a new run (`egg new`). | — | 叙述 |
| `req_amend_active` | [REQ AMENDED] {{id}} is developing: title/description updated → rev {{rev}} (progress kept) | id, rev | 叙述 |
| `req_amend_closed` | {{id}} is {{status}} — amend its successor requirement instead. | id, status | 叙述 |
| `req_amend_deps_active` | {{id}} is developing — the title/description may change, the deps may not. | id | 叙述 |
| `req_amend_derive_hist` | v{{rev}} amended after done → derived {{id}} | rev, id | 叙述 |
| `req_amend_derived` | [REQ DERIVED] {{id}} is done — the amend derives a new requirement {{new}} "{{title}}" (origin={{id}}, rev {{rev}}) | id, new, title, id, rev | 叙述 |
| `req_amend_hist` | v{{rev}} amended (not started) | rev | 叙述 |
| `req_amend_hist_active` | v{{rev}} work description amended (developing, progress kept) | rev | 叙述 |
| `req_amend_ok` | [REQ AMENDED] {{id}} → rev {{rev}} "{{title}}" {{deps}} | id, rev, title, deps | 叙述 |
| `req_amend_ok_short` | (side amend: {{id}}) | id | 叙述 |
| `req_amend_usage` | requirement.amend "req_id\|new title\|new description\|new deps" (empty segment = unchanged; deps `-` = clear) | — | 叙述 |
| `req_bad_dep` | dependency `{{id}}` does not exist. | id | 叙述 |
| `req_conflict_busy` | {{id}} is developing — wait for its delivery (or abort) before declaring a conflict. | id | 叙述 |
| `req_conflict_default` | (no conflict note given) | — | 叙述 |
| `req_conflict_dup` | the two sides of a conflict must be different requirements. | — | 叙述 |
| `req_conflict_exists` | {{id}} already has an unresolved conflict (peer {{peer}}) — run requirement.resolve first. | id, peer | 叙述 |
| `req_conflict_hist` | conflict declared | — | 叙述 |
| `req_conflict_ok` | [REQ CONFLICT · human gate] {{first}} ⇄ {{second}}: {{note}} \| both become blocked; dispatch and settlement skip them; resolution paths: {{modes}} (goal stops and waits for your choice) | first, second, note, modes | 叙述 |
| `req_conflict_ok_short` | (side conflict: {{first}} ⇄ {{second}}) | first, second | 叙述 |
| `req_conflict_status` | {{id}} is {{status}} — terminal requirements cannot declare a conflict. | id, status | 叙述 |
| `req_conflict_usage` | requirement.conflict "req_id_1,req_id_2\|conflict note" | — | 叙述 |
| `req_cycle_dep` | adding `{{dep}}` as a dependency of `{{id}}` would create a dependency cycle. | dep, id | 叙述 |
| `req_deps_have` | (deps {{ids}}) | ids | 叙述 |
| `req_deps_none` | (no deps) | — | 叙述 |
| `req_dispatch_auto` | [AUTO-DISPATCH] {{count}} (declared single successor, reactive): {{items}} \| jobs {{jobs}}/{{cap}} | count, items, jobs, cap | 叙述 |
| `req_dispatch_item` | {{id}} → {{stage}} {{lane}} | id, stage, lane | 叙述 |
| `req_dispatch_noop_blocked` | [DISPATCH] 0 candidates: jobs {{jobs}}/{{cap}} is full, or development candidates are not ready / lanes are taken ({{lanes}}). Reasons: {{reasons}} | jobs, cap, lanes, reasons | 叙述 |
| `req_dispatch_noop_none` | [DISPATCH] 0 candidates (requirements are in a job, in a gate, awaiting acceptance, or done) \| jobs {{jobs}}/{{cap}} | jobs, cap | 叙述 |
| `req_dispatch_ok` | [DISPATCH] {{count}}: {{items}} \| jobs {{jobs}}/{{cap}} | count, items, jobs, cap | 叙述 |
| `req_dispatch_usage` | requirement.dispatch [""\|0\|rN] — empty/0 dispatches by readiness order, rN dispatches only that requirement | — | 叙述 |
| `req_dup_dep` | duplicate dependency `{{id}}`. | id | 叙述 |
| `req_full` | requirement count reached the cap {{cap}} — close some before adding more. | cap | 叙述 |
| `req_gate_block` | [CANNOT-CLOSE] {{action}} refused: {{detail}} | action, detail | 叙述 |
| `req_gate_conflicts` | {{conflicts}} unresolved conflicts — have the user decide the resolution path first. | conflicts | 叙述 |
| `req_gate_hist` | gate:{{kind}} approve | kind | 叙述 |
| `req_gate_jobs` | {{jobs}} jobs are still running (cap {{cap}}) — collect their proposals first. | jobs, cap | 叙述 |
| `req_gate_no_artifacts` | (no artifacts) | — | 叙述 |
| `req_gate_no_evidence` | the accept_gate for {{id}} has no artifact evidence (artifacts is empty) — a summary-only claim is not accepted; have the executor submit a diff / test output first. | id | 叙述 |
| `req_gate_no_note` | (approved against the checklist) | — | 叙述 |
| `req_gate_none` | no gate is awaiting approval (plan_gate / accept). | — | 叙述 |
| `req_gate_ok` | [GATE PASSED · {{kind}}] {{id}} → {{status}} \| evidence: {{artifacts}} \| note: {{note}} ({{gates}} gates left) | kind, id, status, artifacts, note, gates | 叙述 |
| `req_gate_open` | {{open}} requirements are not done yet — walk them through both gates first. | open | 叙述 |
| `req_gate_own_conflict` | the gate for {{id}} is blocked by its own unresolved conflict / needs-amend marker — resolve it first (requirement.resolve or re-dispatch). | id | 叙述 |
| `req_gate_own_job` | the gate for {{id}} is blocked by its own running job ({{job}}) — a job in flight must be delivered before the gate can be approved. | id, job | 叙述 |
| `req_gate_own_props` | the gate for {{id}} is blocked by its own {{count}} pending proposal(s) — accept or reject them first (one gate at a time). | id, count | 叙述 |
| `req_gate_props` | {{pending}} proposals are still pending — accept them serially first. | pending | 叙述 |
| `req_gate_reject_default` | checklist not passed (placeholder reason) | — | 叙述 |
| `req_gate_reject_hist` | gate:{{kind}} reject | kind | 叙述 |
| `req_gate_rejected` | [GATE REJECTED · {{kind}}] {{id}} → {{status}} \| feedback: {{feedback}} ({{gates}} gates left) | kind, id, status, feedback, gates | 叙述 |
| `req_gate_stale` | the gate for {{id}} is bound to rev {{got}} while the current rev is {{want}} — stale, approval is not allowed (redo the proposal on the new rev first). | id, got, want | 叙述 |
| `req_gate_verify` | with require_all_green on you must run verify.submit all_green=true first. | — | 叙述 |
| `req_lane_free` | {{lane}} free | lane | 叙述 |
| `req_lane_panel` | [job slots] {{lanes}} (global cap {{cap}}, {{jobs}} running) — analyze / plan / develop all run asynchronously; the requirement graph and gate checks are serial | lanes, cap, jobs | 叙述 |
| `req_lane_sep` |  ·  | — | 叙述 |
| `req_lane_used` | {{lane}} `{{id}}`({{stage}}) | lane, id, stage | 叙述 |
| `req_list_empty` | (no requirements yet: requirement.add adds one) | — | 叙述 |
| `req_list_head` | [REQ GRAPH] {{panel}} \| stage {{stage}} \| lanes {{lanes}} | panel, stage, lanes | 叙述 |
| `req_list_legend` | 图例：·立项 ◇计划中 ⊟计划门禁 ○待开发 ▶开发中 ◀交付受理 ⊞验收门禁 ◉集成 ✓完成 ⑂拆分 ⇄合并 ⊘废弃 ⚠受阻 | — | 叙述 |
| `req_list_more` | … and {{omitted}} more (requirement.list full shows all {{total}}). | omitted, total | 叙述 |
| `req_list_props` | {{count}} pending proposals (the main agent accepts serially, canonical order): | count | 叙述 |
| `req_list_props_more` | … and {{omitted}} more pending proposals (requirement.list full shows all {{total}}). | omitted, total | 叙述 |
| `req_list_row` |   {{mark}} {{id}} [{{status}}] {{title}}{{trace}} | mark, id, status, title, trace | 叙述 |
| `req_list_usage` | requirement.list [""\|0\|full] | — | 叙述 |
| `req_merge_desc` | merged from {{ids}} (deps are unioned) | ids | 叙述 |
| `req_merge_dup` | duplicate source requirement {{id}}. | id | 叙述 |
| `req_merge_hist` | merged into {{id}} | id | 叙述 |
| `req_merge_ok` | [REQ MERGED] {{ids}} → new requirement {{id}} "{{title}}" (deps unioned: {{deps}}; sources become terminal merged) | ids, id, title, deps | 叙述 |
| `req_merge_status` | {{id}} is currently {{status}} — a merge requires draft / analyzing / planning, no pending proposal, no gate and no lane. | id, status | 叙述 |
| `req_merge_title` | merge {{ids}} | ids | 叙述 |
| `req_merge_usage` | requirement.merge "source_id_1,source_id_2\|merged title\|merged description" | — | 叙述 |
| `req_panel_counts` | 需求 {{total}}：立项 {{draft}} / 计划中 {{planning}} / 门禁 {{gate}} / 待开发 {{ready}} / 开发中 {{developing}} / 复核 {{review}} / 集成 {{integrating}} / 完成 {{done}} | total, draft, planning, gate, ready, developing, review, integrating, done | 叙述 |
| `req_panel_jobs` | jobs {{jobs}}/{{cap}} \| pending {{pending}} \| conflicts {{conflicts}} | jobs, cap, pending, conflicts | 叙述 |
| `req_prop_actor_mismatch` | [ACTOR MISMATCH] {{id}}: exec={{exec}} != host actor {{actor}} — recorded, not rejected (the human gate decides). | id, exec, actor | 叙述 |
| `req_prop_actor_misplaced` | `actor=` must sit after `exec=` and before `actor_session=` in the trailing chain (fixed relative order); the host injects this segment — carry identity via `--actor` / `YGG_ACTOR` / the daemon request field instead. | — | 叙述 |
| `req_prop_actor_session_misplaced` | `actor_session=` must be the last segment (after `actor=`); the host injects this segment — carry identity via `--actor-session` / `YGG_ACTOR_SESSION` / the daemon request field instead. | — | 叙述 |
| `req_prop_actor_unverified` | [ACTOR UNVERIFIED] {{id}}: exec={{exec}} arrived without a host actor stamp (old host / direct dispatch). | id, exec | 叙述 |
| `req_prop_actor_verified` | [ACTOR VERIFIED] {{id}}: exec={{exec}} matches the host actor stamp (same handle namespace). | id, exec | 叙述 |
| `req_prop_busy` | {{id}} already has a pending proposal ({{prop}}) — accept / reject it before resubmitting (no overwrite) | id, prop | 叙述 |
| `req_prop_exec_misplaced` | `exec=` must be the last segment; write `req\|rev\|stage\|summary\|artifacts\|\|<ext>\|exec=<agent_id>[:<reason>]` (the empty segment cannot be omitted). | — | 叙述 |
| `req_prop_no_summary` | (no summary given) | — | 叙述 |
| `req_prop_none` | no pending proposal (proposal.submit first, or pick another req/proposal id). | — | 叙述 |
| `req_prop_ok` | [PROPOSAL RECEIVED] {{id}}: {{req}} / {{stage}} rev {{rev}} — {{summary}} ({{pending}} pending) | id, req, stage, rev, summary, pending | 叙述 |
| `req_prop_plan_gate` | [PLAN GATE UP] {{id}} rev {{rev}} 的 plan 提案已交付即门禁——gate.approve / gate.reject 裁决 | id, rev | 叙述 |
| `req_prop_replayed` | [REPLAY:PROPOSAL] external_id `{{ext}}` already exists ({{id}}) — idempotent return. | ext, id | [REPLAY |
| `req_prop_row` |   · {{id}} → {{req}} rev {{rev}} / {{stage}}: {{summary}} | id, req, rev, stage, summary | 叙述 |
| `req_prop_stage_mismatch` | {{id}} is currently {{status}}; it cannot accept a {{stage}} proposal (stage and state do not match). | id, status, stage | 叙述 |
| `req_prop_usage` | proposal.submit "req_id\|rev\|stage(plan\|development)\|summary\|artifacts[\|changes][\|external_id][\|exec=<agent_id>[:<reason>]]" | — | 叙述 |
| `req_reject_default` | (no reason given) | — | 叙述 |
| `req_reject_hist` | v{{id}} proposal rejected, redo | id | 叙述 |
| `req_reject_ok` | [PROPOSAL REJECTED] {{id}} ({{req}}) → back to {{status}}; reason: {{reason}} | id, req, status, reason | 叙述 |
| `req_replayed` | [REPLAY:REQ] external_id `{{ext}}` already exists ({{id}}, {{status}}) — idempotent return. | ext, id, status | [REPLAY |
| `req_replayed_op` | [REPLAY:ASYNC] external_id `{{ext}}` was already applied — idempotent return, not executed twice. | ext | [REPLAY |
| `req_resolve_drop_busy` | keep_first would abandon {{id}}, but it is {{status}} — only draft / ready can be abandoned. | id, status | 叙述 |
| `req_resolve_drop_hist` | conflict resolved: abandoned ({{note}}) | note | 叙述 |
| `req_resolve_keep_first` | {{id}} becomes terminal dropped (traceable) | id | 叙述 |
| `req_resolve_keep_hist` | conflict resolved: both kept ({{note}}) | note | 叙述 |
| `req_resolve_kept` | both sides kept and requeued | — | 叙述 |
| `req_resolve_modes` | keep_both \| keep_first \| revise | — | 叙述 |
| `req_resolve_ok` | [CONFLICT RESOLVED] {{first}} / {{second}} → {{mode}}: {{effect}} | first, second, mode, effect | 叙述 |
| `req_resolve_revise_busy` | revise requires {{id}} to be draft / ready, currently {{status}}. | id, status | 叙述 |
| `req_resolve_revise_hist` | conflict resolved: amended and requeued ({{note}}) | note | 叙述 |
| `req_resolve_revised` | both sides kept but flagged needs-amend (amend before dispatch) | — | 叙述 |
| `req_resolve_usage` | requirement.resolve "req_id_1,req_id_2\|{{modes}}[\|note]" | modes | 叙述 |
| `req_self_dep` | requirement `{{id}}` cannot depend on itself. | id | 叙述 |
| `req_split_blocked` | {{id}} has an unresolved conflict — run requirement.resolve first. | id | 叙述 |
| `req_split_desc` | split from {{id}} (inherits its deps) | id | 叙述 |
| `req_split_hist` | split into {{ids}} | ids | 叙述 |
| `req_split_ok` | [REQ SPLIT] {{id}} → {{ids}} (inherited deps {{deps}}; the parent becomes terminal split for traceability) | id, ids, deps | 叙述 |
| `req_split_status` | {{id}} is currently {{status}} — a split requires draft / analyzing / planning, no pending proposal, no gate and no lane. | id, status | 叙述 |
| `req_split_usage` | requirement.split "parent_id\|child title A\|child title B[\|…]" (at least two children) | — | 叙述 |
| `req_trace_actor_mismatch` |  [mismatch:actor={{actor}}] | actor | 叙述 |
| `req_trace_actor_unverified` |  [unverified] | — | 叙述 |
| `req_trace_actor_verified` |  [verified] | — | 叙述 |
| `req_trace_conflict` | (conflict: {{note}} \| peer {{peer}}) | note, peer | 叙述 |
| `req_trace_deps` | (deps {{ids}}) | ids | 叙述 |
| `req_trace_exec` | (exec {{list}}) | list | 叙述 |
| `req_trace_exec_missing` | (exec {{stage}}=unregistered) | stage | 叙述 |
| `req_trace_job` | (job {{job}}) | job | 叙述 |
| `req_trace_lane` | ({{lane}}) | lane | 叙述 |
| `req_trace_merged` | (merged into {{id}}) | id | 叙述 |
| `req_trace_needs_amend` | (needs amend: run requirement.amend first) | — | 叙述 |
| `req_trace_origin` | (derived from {{id}}) | id | 叙述 |
| `req_trace_profile_tag` | ({{harness}}) | harness | 叙述 |
| `req_trace_rev` | (rev {{rev}}) | rev | 叙述 |
| `req_trace_split` | (split into {{ids}}) | ids | 叙述 |
| `req_trace_stale` | (stale: the proposal rev is outdated, rebase and resubmit) | — | 叙述 |
| `req_unknown` | no requirement `{{id}}` (run requirement.list first). | id | 叙述 |
| `req_wait_rejected` | [BLOCKED:TODO] `{{action}}` is not available yet: {{detail}} Primary state unchanged. | action, detail | 叙述 |
| `req_why_amend` | {{id}} needs amend (run requirement.amend first) | id | 叙述 |
| `req_why_conflict` | {{id}} has an unresolved conflict awaiting a human decision | id | 叙述 |
| `req_why_deps` | {{id}} waits for deps {{ids}} | id, ids | 叙述 |
| `req_why_none` | (none) | — | 叙述 |
| `req_why_sep` | ,  | — | 叙述 |
