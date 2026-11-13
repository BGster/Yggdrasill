# Yggdrasill · mod 文案契约（copy-contract）

> **定位**：mod 全部状态行文键（beats 六组 + core 的 options/tuning/endings，共 413 键）的**知识权威源**——每键一行「触发时机 / 句式模板 / 变量 / 协议归属」。mod `data/texts/` 是本表的运行时投影：改文案先改本表再同步 mod（单向同步纪律见 [../wiki-governance.md](../wiki-governance.md)）。
> **replaces**：r2「mod 剧情提示词迁出」前仅存在于 `.ygg/mods/agent-seed/data/texts/*.json` 的句式知识（此前无文档面）。
> **上游**：[../README.md](../README.md) · **相关**：[workflow-acts.md](workflow-acts.md)（23 处散文正文） · [../design/prompt-surface.md](../design/prompt-surface.md)（键数口径表）

## 读法与协议归属

- **句式模板**逐字复制自 mod 键值（`{{变量}}` 原样保留）；mod 运行时 `fill(TEXTS.x, {vars})` 填充后输出。
- **协议归属**：`[REJECTED:ACTION]` / `[REJECTED:PAYLOAD]` 前缀由 pack `agent.rejectionPrefixes` 声明，宿主据此把该行计为 failed step；`[ENDING` = 终局行（`agent.endingPrefix`）；`[REPLAY` = 幂等重放提示（`agent.replayPrefix`）；`叙述` = 普通叙事行，无协议语义。
- **动态拼接键**：`<node>_common`（render.lua 按 `st.."_common"` 拼接）、`map_note_*`（render.lua 注记表按动作名引用）、`req_prop_{exec,actor,actor_session}_misplaced`（wire.lua 按段名查 beat）、`executor_bad_actor{,_session}`（wire.lua 按身份段查 beat）——键名由代码拼接，不可改。
- 23 处散文键（`*_common`×6、`scenes`/`prompts`、4 长键等）的叙事正文不在本表——见 [workflow-acts.md](workflow-acts.md) / [dispatch-ritual.md](dispatch-ritual.md) / [cleanup-policy.md](cleanup-policy.md)；本表只收其收缩后的指针壳句式（见 §H）。

## A. beats-flow（63 键：回执版式 / 模式提示 / 拒绝 / 章程与幕转场）

| 键 | 句式模板（逐字） | 变量 | 协议归属 |
|---|---|---|---|
| `allowed_none` | (run closed, no actions available) | — | 叙述 |
| `bad_index` | [CHOICE OUT OF RANGE] {{stage}} has {{count}} options, got {{index}}. | stage, count, index | 叙述 |
| `bad_payload` | the payload for `{{action}}` is malformed. | action | 叙述 |
| `bad_score` | Score must be an integer 1-5, got `{{got}}`. | got | 叙述 |
| `charter_begun` | 局已开工：run.begin 只在立局微状态可用（当前「{{phase}}」） | phase | 叙述 |
| `charter_common` | 立局幕：章程三件套（标题/目标/成功判据）立好才可开工；charter.set 在立局内可反复修订，run.begin 后章程锁定。 | — | 叙述 |
| `charter_locked` | 章程在 run.begin 后锁定：当前微状态「{{phase}}」不可再改章程 | phase | 叙述 |
| `charter_missing` | 章程三件套未齐备（task.title / task.goal / task.success 任一为空）：先 charter.set 补齐再 run.begin | — | 叙述 |
| `charter_ok` | [CHARTER SET] 章程已立（立局内可反复修订）：「{{title}}」｜目标：{{goal}}｜成功判据：{{success}} | title, goal, success | 叙述 |
| `charter_payload` | 任务标题\|要达成的一句话目标\|可判定的成功标准 | — | 叙述 |
| `charter_usage` | 章程三段："标题\|目标\|成功判据"——三段都必须非空 | — | 叙述 |
| `coordinate_common` | accept proposals first (proposal.accept), then pass gates (gate.approve/reject), then dispatch (requirement.dispatch), finally coordinate.submit. | — | 叙述 |
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
| `prologue_common` | shared by both modes: acceptance and both gates still take one real action and one validation each, never skipped; requirements can be added asynchronously (--requirement / --proposal). | — | 叙述 |
| `rejected` | [REJECTED:ACTION] `{{action}}` is not available in stage {{stage}}. Allowed actions: {{allowed}}. Reason: {{detail}} | action, stage, allowed, detail | REJECTED:ACTION/PAYLOAD |
| `rejected_default` | (no reason given) | — | REJECTED:ACTION/PAYLOAD |
| `rejected_payload` | [REJECTED:PAYLOAD] the payload for `{{action}}` cannot be accepted: {{detail}} Primary state unchanged. | action, detail | REJECTED:ACTION/PAYLOAD |
| `release_common` | 发布幕：release.confirm 声明 master@<sha> 与批次 tag（v年.月.日.序号），或 release.skip 声明跳过理由；任一成立即「发布已声明」，review.note 才可批结算/退役。 | — | 叙述 |
| `retired_common` | 退役终态：业务事件全拒；map.show / requirement.list 只读查询仍可用。 | — | 叙述 |
| `run_begun` | [RUN BEGUN] 章程锁定，任务「{{title}}」开工（{{count}} 条需求在册）→ 协调幕 | title, count | 叙述 |
| `run_retire_armed` | [RUN RETIRE ARMED] the next review.note becomes the retiring close: the run settles and ends ([ENDING · run retired]). Clear it with `config.set run.retire=""`. | — | 叙述 |
| `run_retire_cleared` | [RUN RETIRE CLEARED] review.note is back to the batch settle (resident run continues; no ending). | — | 叙述 |
| `secret_redline` | Red line: credentials live only in .env / .env.local; Mod resources, config and reports expose key names only. | — | 叙述 |
| `settle_common` | 盘点幕四项：verify.submit 先全绿 → settlement.evolution 登记演进（可并入会话/待办可选段）→ settlement.sessions → settlement.todo；四项齐备自动转发布幕（不占事件）。 | — | 叙述 |
| `settle_ready` | [SETTLE READY] 盘点四项齐备（验证/演进登记/会话清理/待办清理）——自动进入发布幕 | — | 叙述 |
| `stage_expected` | 该动作只在微状态「{{expected}}」可用 | expected | 叙述 |
| `stage_prompt_missing` | (the prompt for this node is missing: check <node>_common in data/texts/beats-flow.json) | — | 叙述 |
| `unknown_choice` | [RUN-CLOSED] {{stage}} has no pending choices. | stage | 叙述 |
| `unknown_stage` | Unknown stage `{{stage}}`. | stage | 叙述 |
| `verify_fail` | [GATES FAILED] #{{failures}}: sent back to coordinate for rework. | failures | 叙述 |
| `verify_failed_record` | failed (recorded) | — | 叙述 |
| `verify_green_record` | all green (recorded) | — | 叙述 |
| `verify_ok` | [GATES GREEN] {{count}} requirements all terminal; settle act open — declare the evolution registration next (settlement.evolution, batch {{batch}}). | count, batch | 叙述 |

## B. beats-harness（62 键：配置面板 / 身份注册 / 工具桥 / 泳道工具）

| 键 | 句式模板（逐字） | 变量 | 协议归属 |
|---|---|---|---|
| `artifact_empty` | [ARTIFACT EMPTY] {{path}} (bytes=0) \| nothing was recorded | path | 叙述 |
| `artifact_ledger_full` | [ARTIFACT LEDGER FULL] {{path}} was not recorded (cap {{cap}}) | path, cap | 叙述 |
| `artifact_missing` | [ARTIFACT MISSING] {{path}} (exists=0) | path | 叙述 |
| `artifact_path_rejected` | [ARTIFACT PATH REFUSED] {{path}}: the ledger only accepts artifacts/<name>.md (no `..`, no whitespace, at most {{max}} bytes) \| nothing was recorded | path, max | 叙述 |
| `artifact_verified` | [ARTIFACT VERIFIED] {{path}} ({{bytes}} B, sha256 {{sha}}) | path, bytes, sha | 叙述 |
| `bad_config_key` | unknown config key `{{key}}` (available: {{allowed}}) | key, allowed | 叙述 |
| `bad_config_value` | `{{key}}` does not accept `{{value}}` (available: {{allowed}}) | key, value, allowed | 叙述 |
| `config_changed` | config updated: {{key}} = {{value}} (stage {{stage}}) | key, value, stage | 叙述 |
| `config_closed` | [BACK] returned to the {{stage}} stage menu. | stage | 叙述 |
| `config_goal_auth` | goal authorization: {{auth}} (self-declared, only checked when a runtime goal run closes) | auth | 叙述 |
| `config_goal_auth_none` | goal authorization: not registered (only runs that entered goal at runtime need it; a static default is not checked) | — | 叙述 |
| `config_keys` | play.mode / goal.authorization / executor.<rN>.<analysis\|plan\|development> / evolution.mode / evolution.max_per_run / gates.require_all_green / review.evolution.artifact / review.evolution.summary / review.evolution.candidates / review.evolution.classes | — | 叙述 |
| `config_open_usage` | config.open [] / config.close [] | — | 叙述 |
| `config_opened` | [CONFIG MENU] play mode / evolution preference / gates — picking one applies it and stays on this page; pick "back" to return to the stage menu. Note: switching play mode leaves this page (the driver changed, so the sub-page context is stale). Evolution pre-registration does not use the menu: write settings with `config.set review.evolution.artifact\|summary\|candidates\|classes=<value>`; after verify passes the settlement.evolution menu builds the payload and validates it in one place. | — | 叙述 |
| `executor_bad_actor` | an actor id must be non-empty after trim, <= 64 chars, with no whitespace and no quote/backslash (the host injects `actor=`; self-carried segments are rejected). | — | 叙述 |
| `executor_bad_actor_session` | an actor_session id must be non-empty after trim, <= 128 chars, with no whitespace and no quote/backslash (the host injects `actor_session=`; self-carried segments are rejected). | — | 叙述 |
| `executor_bad_id` | an executor id must be non-empty after trim, <= 64 chars, with no whitespace and no `\|`. | — | 叙述 |
| `executor_bad_key` | executor key shape: `executor.<rN>.<stage>`, stage ∈ {{stages}} (got `{{key}}`). | stages, key | 叙述 |
| `executor_bound` | [EXECUTOR BOUND] {{id}} / {{stage}} = {{agent}} (self-declared, not identity proof) | id, stage, agent | 叙述 |
| `executor_cleared` | [EXECUTOR CLEARED] {{id}} / {{stage}} | id, stage | 叙述 |
| `executor_hist` | executor {{stage}}: {{from}} → {{to}} ({{reason}}) | stage, from, to, reason | 叙述 |
| `executor_no_reason` | no reason | — | 叙述 |
| `executor_reason_required` | changing the executor requires a reason: `config.set executor.{{id}}.{{stage}}=<new_id>\|<reason>` (previous executor {{prev}}; cross-stage handoff needs one too). | id, stage, prev | 叙述 |
| `executor_same` | executor unchanged: {{id}} / {{stage}} = {{agent}} (idempotent, not rewritten) | id, stage, agent | 叙述 |
| `executor_unknown_req` | executor registration points at a missing requirement: `{{id}}`. | id | 叙述 |
| `goal_auth_bad` | goal authorization must be a single line (non-empty after trim, <= 200 chars, no newline). | — | 叙述 |
| `goal_auth_cleared` | [GOAL AUTH CLEARED] a runtime goal run must register it again before closing. | — | 叙述 |
| `goal_auth_pending` | [GOAL AUTH PENDING] entered goal mode without goal.authorization; register config.set goal.authorization=<note> before review.note | — | 叙述 |
| `goal_auth_registered` | [GOAL AUTH REGISTERED] {{auth}} (self-declared, not identity proof; the closing check applies only to runs that entered goal at runtime) | auth | 叙述 |
| `goal_auth_required` | this run entered goal at runtime: register the run-level authorization before closing with `config.set goal.authorization=<authorization note>` (self-declared; a static goal preset in data/config.json is out of scope). | — | 叙述 |
| `reactive_malformed` | [PROBE MALFORMED] {{call_id}} ({{tool_id}}): the output is not a `k=v;...` map - nothing was applied | call_id, tool_id | 叙述 |
| `subagent_brief` | Requirement {{id}} (rev {{rev}}, stage={{stage}}): {{title}}\nDescription: {{desc}}\nMandatory references: read docs/README.md (wiki index) first, then docs/context/* (domain invariants) and docs/design/* (architecture & contracts) before proposing or editing.\nWorktree: {{worktree}} (branch={{branch}}, base={{base}})\nArtifact: {{artifact}} — deliver through the harness archive, never as a stray file in a checkout: return the full content in your final report (or write it under /tmp) and the main session records it into the run archive with `--artifact-put artifacts/<name>.md`. All artifacts of a run live only in that run's archive (`<run-dir>/artifacts/`); do not write artifact files into any checkout's `artifacts/` — that path is a git-ignored scratch area, not the authoritative copy.\nSubmit: {{submit}}\nIntegration (development stage only): at closeout you integrate your feature branch into `develop` yourself — align `origin/develop`, `git merge --no-ff`, integrated-state gates green, push `develop`, and record the develop merge SHA (`develop@<sha>`) in the artifact. Never touch `master` or another lane's branch.\nDoc sync (when applicable): the docs wiki — docs/README.md (index) with docs/context/* (domain vocabulary, principles, decision ledger) and docs/design/* (system behaviour, interface contracts) — is the always-current authority of the whole repository — never a changelog, history lives in git and settlement artifacts. If your change alters how any subsystem works or is meant to work (kernel crates / egg CLI / Mod / serve / tooling): at the plan stage declare in your artifact which wiki files it touches; at the development stage update those files in place on your feature branch (decisions/vocabulary -> the matching decisions-*/vocabulary entry in docs/context/ for the D-MAx in this brief; behaviour/contracts -> the matching docs/design/ chapter for that subsystem). Pure fixes and rewording touch neither. | id, rev, stage, title, desc, worktree, branch, base, artifact, submit | 叙述 |
| `subagent_delta` | Resume {{id}} (rev {{rev}}) for stage `{{stage}}` in worktree `{{worktree}}`; write `{{artifact}}` and submit. | id, rev, stage, worktree, artifact | 叙述 |
| `subagent_fallback_note` | \nResume context: prior executor `{{resume_id}}`. | resume_id | 叙述 |
| `subagent_submit_cmd` | .ygg/bin/egg --mod .ygg/mods/agent-seed --runs-root .ygg/runs --resume --event proposal.submit --payload '"{{id}}\|{{rev}}\|{{stage}}\|<summary>\|{{artifact}}\|\|exec=<agent_id>"' | id, rev, stage, artifact | 叙述 |
| `tool_after_done` | the run has reached its ending: no tool call can be requested or settled any more. | — | 叙述 |
| `tool_allowed_none` | none | — | 叙述 |
| `tool_bad_args` | arguments `{{args}}` are malformed: {{problem}} | args, problem | 叙述 |
| `tool_bad_call_id` | call id `{{call_id}}` is not usable: lowercase letters/digits/hyphen/underscore, 1..{{max}} chars, and not one of count / pending / last / notified. | call_id, max | 叙述 |
| `tool_bad_code` | code `{{code}}` must be an integer (use `-1` when the tool never ran). | code | 叙述 |
| `tool_bad_status` | status `{{status}}` must be `ok` or `err`. | status | 叙述 |
| `tool_bad_tool_id` | tool id `{{tool_id}}` must look like `<namespace>.<name>`: lowercase letters/digits/underscore/hyphen in both segments, at most 64 bytes. | tool_id | 叙述 |
| `tool_call_dup` | call id `{{call_id}}` already exists ({{status}}); use a fresh id, or resend the same external_id to replay. | call_id, status | 叙述 |
| `tool_capacity` | no free tool slot ({{cap}} calls already allocated); settle the pending calls first. | cap | 叙述 |
| `tool_not_allowed` | tool `{{tool_id}}` is not in data/config.json `allowedTools` (allowed: {{allowed}}) — refused before anything was stored. | tool_id, allowed | 叙述 |
| `tool_output_truncated` | …[truncated {{dropped}} chars] | dropped | 叙述 |
| `tool_replayed` | [REPLAY:TOOL] call `{{call_id}}` with external_id `{{ext}}` was already applied — idempotent return, not executed twice. | call_id, ext | [REPLAY |
| `tool_requested` | [TOOL REQUESTED] {{call_id}} -> {{tool_id}} ({{args}}); pending {{pending}} | call_id, tool_id, args, pending | 叙述 |
| `tool_result` | [TOOL RESULT] {{call_id}} ({{tool_id}}) -> {{status}} (code {{code}}): {{output}} | call_id, tool_id, status, code, output | 叙述 |
| `tool_result_mismatch` | call `{{call_id}}` does not accept this result: expected {{expected}}, got {{got}}. | call_id, expected, got | 叙述 |
| `tool_result_usage` | tool.result <call_id>\|<tool_id>\|<ok\|err>\|<code>\|<output>[\|<external_id>] (5 or 6 segments; a vertical bar may not appear inside the output) | — | 叙述 |
| `tool_settled` | [TOOL SETTLED] {{call_id}} -> {{status}} (code {{code}}); nothing else is pending. | call_id, status, code | 叙述 |
| `tool_unknown_call` | no tool call `{{call_id}}` exists in this run. | call_id | 叙述 |
| `tool_usage` | tool.call <tool_id>[\|<k=v;...>][\|<call_id>] — the tool id must be listed in allowedTools; arguments are `k=v` pairs separated by `;` | — | 叙述 |
| `verify_suite_blocked` | [VERIFY SUITE BLOCKED] {{reason}} | reason | 叙述 |
| `verify_suite_failed` | [VERIFY SUITE FAILED] {{summary}} | summary | 叙述 |
| `verify_suite_passed` | [VERIFY SUITE PASSED] {{summary}} | summary | 叙述 |
| `vfs_ledger_only` | [VFS LEDGER ONLY] {{path}} ({{bytes}} B): this host exposes no `ygg.fs`, so nothing reached the disk - the artifact ledger records the Mod-side assertion only | path, bytes | 叙述 |
| `vfs_path_rejected` | [VFS PATH REFUSED] {{path}}: only artifacts/<name>.md is writable (no `..`, no whitespace, at most {{max}} bytes) | path, max | 叙述 |
| `vfs_usage` | usage: `vfs.write` takes `artifacts/<name>.md\|<content>` (path must be artifacts/*.md, content 1..{{max}} bytes) | max | 叙述 |
| `vfs_write_failed` | [VFS WRITE FAILED] {{path}}: {{reason}} | path, reason | 叙述 |
| `vfs_written` | [VFS WRITTEN] {{path}} ({{bytes}} B, sha256 {{sha}}) | path, bytes, sha | 叙述 |

## C. beats-lanes（36 键：泳道账本 / 池注册 / 认领释放 / probe）

| 键 | 句式模板（逐字） | 变量 | 协议归属 |
|---|---|---|---|
| `conflict_radar` | [CONFLICT RADAR] {{first}} ({{lane}}) and {{second}} ({{peer}}) both modified {{file}} | first, lane, second, peer, file | 叙述 |
| `conflict_radar_note` | overlapping file: {{file}} | file | 叙述 |
| `lane_bad_k` | a lane number accepts only 1..{{max}}, got `{{k}}`. | max, k | 叙述 |
| `lane_branch_hint` | [HINT] {{lane}} carries no branch: run lane.claim first (payload "lane\|req_id\|branch\|base\|clean\|pool", branch = feature/xxxx-semantic) so the worktree claim can bind a real branch; dispatching without one only records the ledger. | lane | 叙述 |
| `lane_branch_invalid` | [LANE BRANCH INVALID] branch "{{branch}}" violates ^feature/[a-z0-9]{4}-<semantic>$ (total <=48; semantic: lowercase alnum/dash, >=3 consecutive letters, not pure digits, not 8-16 pure hex). | branch | 叙述 |
| `lane_claim_bound` | {{id}} already holds {{lane}}; a second lane is not allowed. | id, lane | 叙述 |
| `lane_claim_closed` | {{id}} is {{status}} — terminal requirements cannot claim a lane. | id, status | 叙述 |
| `lane_claim_dirty` | {{lane}} claim refused: `clean` must be "1". The main agent runs `git status --porcelain` in the target worktree first (must be empty except `.lane`); if dirty, pick another lane or handle the changes — the mod cannot run git, it only accepts this assertion. | lane | 叙述 |
| `lane_claim_occupied` | {{lane}} is already owned by {{id}} — pick another lane or wait for its release. | lane, id | 叙述 |
| `lane_claim_usage` | lane.claim "lane\|req_id\|branch\|base\|clean\|pool" (lane 1..{{max}}; pool optional = registered named pool, default pool when omitted; clean must be "1") | max | 叙述 |
| `lane_claimed` | [LANE CLAIMED] {{lane}} ← {{id}} "{{title}}" \| branch {{branch}} \| base {{base}} \| since #{{since}} \| clean=1 (the main agent confirmed a clean tree) | lane, id, title, branch, base, since | 叙述 |
| `lane_field_none` | - | — | 叙述 |
| `lane_ledger_all_free` |   all {{total}} lanes free | total | 叙述 |
| `lane_ledger_all_free_pool` |   all {{pool}} lanes free ({{total}}) | pool, total | 叙述 |
| `lane_ledger_head` | -- lane ledger: {{used}}/{{total}} owned (branch / base / clean are recorded by lane.claim, since = claim serial, clean = the main agent's clean-tree assertion) -- | used, total | 叙述 |
| `lane_ledger_head_pool` | -- lane ledger[{{pool}}]: {{used}}/{{total}} owned (branch / base / clean are recorded by lane.claim, since = claim serial, clean = the main agent's clean-tree assertion) -- | pool, used, total | 叙述 |
| `lane_ledger_none` |   free: {{lanes}} | lanes | 叙述 |
| `lane_ledger_used` |   {{lane}} ← {{id}} ({{stage}}) \| branch {{branch}} \| base {{base}} \| since #{{since}} \| clean {{clean}} | lane, id, stage, branch, base, since, clean | 叙述 |
| `lane_pool_bad_label` | a pool label accepts only ^[a-z][a-z0-9-]{0,23}$ (reserved: seq, pools), got `{{pool}}`. | pool | 叙述 |
| `lane_pool_bad_segment` | lane.pool usage: 'label\|path' — path must be non-empty, without \| ; newline, no leading/trailing spaces. | — | 叙述 |
| `lane_pool_path_missing` | pool `{{pool}}` has no registered path — run lane.pool "{{pool}}\|<local checkout path>" first. | pool, pool | 叙述 |
| `lane_pool_registered` | [LANE POOL] {{pool}} -> {{path}} \| {{state}} ({{count}} named pools) | pool, path, state, count | 叙述 |
| `lane_pool_unknown` | pool `{{pool}}` is not registered — run lane.pool "{{pool}}\|<local checkout path>" first. | pool, pool | 叙述 |
| `lane_probe_bad_lane` | [LANE PROBE REJECTED] lane={{lane}}: out of range 1..{{max}} \| nothing was recorded | lane, max | 叙述 |
| `lane_probe_failed` | [LANE PROBE FAILED] {{lane}}: commits={{commits}} clean={{clean}} | lane, commits, clean | 叙述 |
| `lane_probe_recorded` | [LANE PROBE] {{lane}}: commits={{commits}} clean={{clean}} head={{head}} files={{files}} | lane, commits, clean, head, files | 叙述 |
| `lane_probe_truncated` | [LANE PROBE TRUNCATED] {{call_id}}: the file list was cut at the host output cap - the conflict radar is skipped for this probe | call_id | 叙述 |
| `lane_release_idle` | none (it was already free) | — | 叙述 |
| `lane_release_usage` | lane.release "lane\|pool" (1..{{max}}; pool optional = registered named pool, default pool when omitted) | max | 叙述 |
| `lane_released` | [LANE RELEASED] {{lane}} → free \| previous owner {{id}} | lane, id | 叙述 |
| `probe_rejected` | only an empty payload, "0" or "full" is accepted, got `{{got}}`; usage: {{usage}} | got, usage | 叙述 |
| `worktree_claim_refused` | [WORKTREE CLAIM REFUSED] {{lane}}: {{reason}} \| the lane ledger was not touched | lane, reason | 叙述 |
| `worktree_claimed` | [WORKTREE CLAIMED] {{lane}} <- {{req}} \| branch {{branch}} \| base {{base}} \| clean=1 | lane, req, branch, base | 叙述 |
| `worktree_release_idle` | [WORKTREE RELEASED] {{lane}} -> free (it was already idle) | lane | 叙述 |
| `worktree_release_refused` | [WORKTREE RELEASE REFUSED] {{reason}} \| the lane ledger was not touched | reason | 叙述 |
| `worktree_released` | [WORKTREE RELEASED] {{lane}} -> free | lane | 叙述 |

## D. beats-map-guide（38 键：地图视图 / 菜单行 / 人机门禁 / 节点注记）

| 键 | 句式模板（逐字） | 变量 | 协议归属 |
|---|---|---|---|
| `guide_hint` | field rules · {{title}}: {{summary}} ({{count}} rules; full text: .ygg/mods/agent-seed/data/playbook/) | title, summary, count | 叙述 |
| `guide_human_blocked` | [REJECTED:ACTION] `{{action}}` is a pass-type action protected by the human-action gate (proposal.accept / gate.approve / coordinate.submit / verify.submit / review.note): in guide mode only a human may pick it from the menu (reply with the index → ygg.choice_selected). Direct single-shot dispatch is always refused and the primary state is unchanged. Do this: hand the full pendingChoice text to the user and wait for the index; never press it on their behalf. | action | REJECTED:ACTION/PAYLOAD |
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

## E. beats-requirements（144 键：派发 / 提案 / 门禁 / 修订 / 拆分合并 / 台账面板）

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
| `req_after_done` | This run is closed: requirements need a new run (another --run-id). | — | 叙述 |
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

## F. beats-settlement（47 键：盘点四项 / 批结算 / 退役 / 发布）

| 键 | 句式模板（逐字） | 变量 | 协议归属 |
|---|---|---|---|
| `maintenance_req_added` | [MAINTENANCE REQ ADDED] {{id}} "{{title}}" — batch {{batch}} closeout appended next batch's docs-maintenance requirement (open/draft); dispatch it in coordinate — it gates the next boundary until done. | id, title, batch | 叙述 |
| `maintenance_req_desc` | 依 wiki-governance 三职责复核上批全部需求的 plan 与 develop 验收结果是否已落 docs wiki：决策与词汇 → docs/context/decisions-agent.md 台账行、vocabulary.md 词条；行为与契约 → docs/design/ 对应主题文件（按本批触碰面：state-machine/events/state-surface/run-story/requirement-story 等）；结构变化 → docs/README.md 索引与 CONTEXT.md/DESIGN.md 两薄壳映射表；纪律：每主题一文件 ≤300 行、头部定位/replaces/上下游、双向链接；纯缺陷修复与措辞调整不触碰文档。 | — | 叙述 |
| `maintenance_req_skipped_dedup` | [MAINTENANCE REQ SKIP] {{id}} already carries an open docs-maintenance requirement (maintenance marker / canonical title) — dedup hit, the batch settles without a second one. | id | 叙述 |
| `maintenance_req_skipped_full` | [MAINTENANCE REQ SKIP] requirement register is full ({{cap}} open) — docs-maintenance append deferred; the batch settles regardless. | cap | 叙述 |
| `maintenance_req_title` | 维护更新 docs wiki（上批收口） | — | 叙述 |
| `release_confirm_payload` | master@1a2b3c4d\|v2026.11.12.1 | — | 叙述 |
| `release_confirmed` | [RELEASE CONFIRMED] 第 {{batch}} 批合并已声明：master@{{sha}} tag {{tag}}——review.note 可批结算/退役 | batch, sha, tag | 叙述 |
| `release_required` | 发布未声明：先 release.confirm（master@<sha> + 批次 tag）或 release.skip（跳过理由），review.note 才能收口本批 | — | 叙述 |
| `release_skip_payload` | 本批无需发布：纯文档/测试批次，无 develop→master 合并 | — | 叙述 |
| `release_skip_usage` | release.skip 载荷 = 非空理由（本批无需发布的显式声明） | — | 叙述 |
| `release_skipped` | [RELEASE SKIPPED] 第 {{batch}} 批无需发布：{{reason}}——review.note 可批结算/退役 | batch, reason | 叙述 |
| `release_usage` | release.confirm 载荷："master@<sha>\|v年.月.日.序号"——sha 为 7-40 位十六进制，标签 v 开头四段数字点分 | — | 叙述 |
| `review_batch_ok` | [BATCH {{batch}} SETTLED] score {{score}}/5 — {{lesson}}; evolution archived to the rolling slot; batch counter -> {{next}}; back to coordinate — the resident run waits for the next batch. | batch, score, lesson, next | 叙述 |
| `review_evolution_bad_candidates` | invalid evolution candidate count: `c=<kept>/<screened>`, two non-negative integers with kept <= screened. | — | 叙述 |
| `review_evolution_bad_classes` | invalid evolution candidate classes: `cls=<kind>:<n>,...`, kind must be one of the six classes, counts non-negative and unique. | — | 叙述 |
| `review_evolution_bad_path` | invalid evolution artifact path: must be `artifacts/<file>.md`, no `..` and no whitespace. | — | 叙述 |
| `review_evolution_mismatch` | the evolution declaration is not self-consistent: the cls counts must sum to screened, and the kept classes (long_term + unused + redundant) must sum to kept. | — | 叙述 |
| `review_evolution_need_artifact` | with candidates (screened > 0) the first segment of settlement.evolution must be `artifacts/<file>.md` — every screened candidate needs a review record, not only the kept ones. | — | 叙述 |
| `review_evolution_need_classes` | with candidates (screened > 0) you must provide per-class root-cause counts: cls=<kind>:<n>,... (the six classes sum to screened, the kept classes sum to kept). | — | 叙述 |
| `review_note_legacy_evolution` | review.note no longer accepts ev= / c= / cls= segments; register with settlement.evolution first, then close with lesson\|score. | — | 叙述 |
| `review_note_usage` | review.note accepts only lesson\|score; the evolution record must be registered through settlement.evolution first — this action only archives it single-source. | — | 叙述 |
| `review_ok` | [REVIEW RECORDED] {{score}}/5 — {{lesson}} | score, lesson | 叙述 |
| `settlement_artifact_absent` | artifact {{artifact}} does not exist in the mounted VFS — registration refused (no settlement written, no archive side effects) | artifact | 叙述 |
| `settlement_artifact_empty` | artifact {{artifact}} is empty in the mounted VFS — registration refused (no settlement written, no archive side effects) | artifact | 叙述 |
| `settlement_artifact_no_vfs` | artifact {{artifact}} cannot be verified: the host VFS is not mounted — non-empty registration refused (no settlement written, no archive side effects) | artifact | 叙述 |
| `settlement_artifact_read_error` | artifact {{artifact}} probe failed — registration refused (no settlement written, no archive side effects) | artifact | 叙述 |
| `settlement_evolution_bad_segment` | settlement.evolution does not accept the segment `{{segment}}`: the optional segments are c=, cls=, sessions= and todo=; legacy ev= and unknown segments are refused. | segment | 叙述 |
| `settlement_evolution_default_summary` | candidates screened against the evolution chapter | — | 叙述 |
| `settlement_evolution_duplicate_segment` | the c= / cls= / sessions= / todo= segments must not repeat. | — | 叙述 |
| `settlement_evolution_missing` | evolution registration is missing: run settlement.evolution first; summary must be non-empty after trim (artifact may stay empty when c=0/0). | — | 叙述 |
| `settlement_evolution_need_summary` | the settlement.evolution summary must be non-empty after trim; with zero candidates you may register the neutral summary and c=0/0 with an empty artifact. | — | 叙述 |
| `settlement_evolution_ok` | [EVOLUTION REGISTERED] {{summary}} \| candidates {{candidates}} (kept/screened) \| classes {{classes}}. Sessions and todo are settled in the same transaction when their segments are present; review.note archives single-source and closes the run. | summary, candidates, classes | 叙述 |
| `settlement_evolution_usage` | settlement.evolution payload: artifact\|summary[\|c=<kept>/<screened>[\|cls=<kind>:<n>,...]][\|sessions=<moved>/<remaining>/<destination>][\|todo=<remaining>/<verified_by>]; summary is required after trim and must not contain \|; the only optional segments are c=, cls=, sessions= and todo= — ev=, duplicates and unknown segments are refused. | — | 叙述 |
| `settlement_sessions_missing` | subagent sessions are not settled (settlement.sessions, remaining must be 0) | — | 叙述 |
| `settlement_sessions_ok` | subagent sessions settled: moved {{moved}} to {{destination}} (files renamed into the trash directory — not a permanent deletion; no per-file guarantee about which sessions were spared: verify the destination yourself). | moved, destination | 叙述 |
| `settlement_sessions_payload` | 0\|0\|<your-destination>/<batch> | — | 叙述 |
| `settlement_sessions_segment` | sessions=0/0/<your-destination>/<batch> | — | 叙述 |
| `settlement_sessions_segment_bad` | the settlement.evolution sessions= segment must be `<moved>/<remaining>/<destination>`: moved a non-negative integer, remaining exactly 0, destination non-empty (slashes inside the destination are kept). | — | 叙述 |
| `settlement_sessions_unsetted_hint` | [SESSIONS CLEANUP PENDING] subagent sessions remaining={{remaining}} — review.note stays blocked. Cleanup is prompt-driven: use your own environment's native session-management capability to delete the subagent session records produced by this run; scope strictly to this run's artifacts; the main session and unrelated sessions must never be touched; moved=0 is a legal outcome. When done, declare the result via settlement.sessions (moved/remaining/destination) or the settlement.evolution sessions= segment. A missing declaration stays honestly unsettled. Success criterion: after cleanup, this run's subagent sessions must no longer appear in your own session history/resume list. Note: sub-session records may live side by side with main sessions in the same session store - identify each file by its spawn time and session id before moving anything, and never touch the main session or unrelated sessions. Do not mistake task-I/O copies under temporary directories for the records themselves: the records are the entries your own session history lists. | remaining | 叙述 |
| `settlement_sessions_usage` | settlement.sessions payload "moved\|remaining\|destination": moved a non-negative integer, remaining exactly 0, destination non-empty (e.g. <your-destination>/<batch>); all three segments are required. | — | 叙述 |
| `settlement_todo_missing` | the main-session todo list is not settled (settlement.todo, remaining must be 0) | — | 叙述 |
| `settlement_todo_ok` | main-session todo list cleared: re-checked to 0 items (basis: {{verified_by}}). | verified_by | 叙述 |
| `settlement_todo_payload` | 0\|main session todo list re-checked to 0 items (done + historical included) | — | 叙述 |
| `settlement_todo_segment` | todo=0/main session todo list re-checked to 0 items (done + historical included) | — | 叙述 |
| `settlement_todo_segment_bad` | the settlement.evolution todo= segment must be `<remaining>/<verified_by>`: remaining exactly 0, verified_by non-empty and <= 120 chars. | — | 叙述 |
| `settlement_todo_usage` | settlement.todo payload "remaining\|verified_by": exactly two segments, remaining exactly 0, verified_by non-empty and <= 120 chars. | — | 叙述 |
| `settlement_verify_first` | record all-green evidence with verify.submit first, then register the three settlement tasks (all of them in the review stage). | — | 叙述 |

## G. core.json（options 27 / tuning 8 / endings 1 = 36 键；scenes/prompts 散文见 §H）

### G.options · 菜单措辞（mod 运行时渲染选择面；语义对照见 workflow-acts.md 各幕）

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

### G.tuning · 数值契约（容量/上限，非文案但同驻 core.json，一并归一）

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

### G.endings · 终局行（[ENDING 前缀由 pack 声明）

| 键 | 值/句式（逐字） | 变量 | 协议归属 |
|---|---|---|---|
| `endings.done` | [ENDING · run retired] 任务「{{title}}」——{{count}} 条需求（{{done}} 条终态），{{batch}} 个已结算批次；复盘评分 {{score}}/5，演进候选 {{candidates}}（保留/筛除）。一句话教训：{{lesson}} | title, count, done, batch, score, candidates, lesson | [ENDING |

## H. 23 处散文键的指针壳句式（mod 收缩后形态）

以下键的叙事/策略正文已归一到 context 散文三件套；mod 侧只留「状态前缀/变量头 + 指针」。收缩后的键值以本表为准（知识权威），运行时由 P3 同步进 mod。

| 键 | 收缩后形态（指向） |
|---|---|
| `prologue_common` | docs/context/workflow-acts.md#序幕 |
| `charter_common` | docs/context/workflow-acts.md#立局幕 |
| `coordinate_common` | docs/context/workflow-acts.md#协调幕 |
| `settle_common` | docs/context/workflow-acts.md#盘点幕 |
| `release_common` | docs/context/workflow-acts.md#发布幕 |
| `retired_common` | docs/context/workflow-acts.md#退役终态 |
| `subagent_brief` | docs/context/dispatch-ritual.md（§2–§6 策略正文） |
| `config_opened` | docs/context/dispatch-ritual.md#7 |
| `subagent_fallback_note` | docs/context/dispatch-ritual.md#8 |
| `guide_human_blocked` | docs/context/workflow-acts.md#协调幕（人机门禁） |
| `guide_hint` | docs/context/workflow-acts.md#协调幕（field rules） |
| `settlement_sessions_unsetted_hint` | docs/context/cleanup-policy.md#1 |
| `settlement_todo_segment` | docs/context/cleanup-policy.md#2 |
| `scenes.*` | docs/context/workflow-acts.md 各幕场景台词 |
| `prompts.*` | docs/context/workflow-acts.md 各幕 prompts 语义 |

---

**键数核对**：A–G 共 426 键（beats 390 + core 36）+ H 23 处指针壳 = 436 键全覆盖；与 [../design/prompt-surface.md](../design/prompt-surface.md) 口径表一致。
