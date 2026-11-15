# Yggdrasill · mod 文案契约 · beats-harness
> **id** copy/harness · **幕** all · **触发** 改 `beats-harness.json` 前
> **读不到代价** silent-drift
> **ver** 7f6464

> **定位**：`beats-harness.json` 74 键的句式知识权威：配置面板、身份注册、工具桥、泳道工具与 tools 注册面。
> **上游**：[copy/README.md](README.md) · **运行时投影**：`.ygg/mods/agent-seed/data/texts/beats-harness.json`
> **维护**：改文案先改本表再同步 mod；键增删/改域触发索引、prompt-surface 与键数同批更新。

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
| `config_keys` | play.mode / goal.authorization / executor.<rN>.<analysis\|plan\|development> / evolution.mode / evolution.max_per_run / gates.require_all_green / review.evolution.artifact / review.evolution.summary / review.evolution.candidates / review.evolution.classes / tools.root / tools.<name> / read.<act>/<doc-id> | — | 叙述 |
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
| `read_bad_doc` | a read signature needs read.<act>/<doc-id>=<ver>: lowercase letters, digits and dashes, exactly one slash, no dots (got "{{doc}}") | doc | 叙述 |
| `read_bad_ver` | the signature version must be 4-40 lowercase hex characters (copy it from the doc header ver line); got "{{ver}}" | ver | 叙述 |
| `read_cleared` | [SIGN] {{doc}} cleared: the signature no longer counts ({{scene}}) | doc, scene | 叙述 |
| `read_signed` | [SIGNED] {{doc}}@{{ver}} — {{k}}/{{n}} signed in this act ({{scene}}) | doc, ver, k, n, scene | 叙述 |
| `read_signed_unplotted` | [SIGNED] {{doc}}@{{ver}} — {{k}} signed in this act ({{scene}}); no read plan is baked for this act, so coverage is not computed | doc, ver, k, scene | 叙述 |
| `subagent_brief` | Requirement {{id}} (rev {{rev}}, stage={{stage}}): {{title}}\nDescription: {{desc}}\nMandatory references: read docs/README.md, docs/context/workflow-acts.md, docs/context/dispatch-ritual.md, and the requirement's named docs/design/<subsystem>.md before proposing or editing. Plan: follow playbook run_machine + actions and declare touched wiki files. Development: also follow gates + live_evidence and the touched subsystem chapter.\nWorktree: {{worktree}} (branch={{branch}}, base={{base}})\nArtifact: {{artifact}} — deliver through the harness archive, never as a stray file in a checkout: return the full content in your final report (or write it under /tmp) and the main session records it into the run archive with `--artifact-put artifacts/<name>.md`. All artifacts of a run live only in that run's archive (`<run-dir>/artifacts/`); do not write artifact files into any checkout's `artifacts/` — that path is a git-ignored scratch area, not the authoritative copy.\nSubmit: {{submit}}\nIntegration (development stage only): at closeout you integrate your feature branch into `develop` yourself — align `origin/develop`, `git merge --no-ff`, integrated-state gates green, push `develop`, and record the develop merge SHA (`develop@<sha>`) in the artifact. Never touch `master` or another lane's branch.\nDoc sync (when applicable): the docs wiki — docs/README.md (index) with docs/context/* (domain vocabulary, principles, decision ledger) and docs/design/* (system behaviour, interface contracts) — is the always-current authority of the whole repository — never a changelog, history lives in git and settlement artifacts. If your change alters how any subsystem works or is meant to work (kernel crates / egg CLI / Mod / serve / tooling): at the plan stage declare in your artifact which wiki files it touches; at the development stage update those files in place on your feature branch (decisions/vocabulary -> the matching decisions-*/vocabulary entry in docs/context/ for the D-MAx in this brief; behaviour/contracts -> the matching docs/design/ chapter for that subsystem). Pure fixes and rewording touch neither. | id, rev, stage, title, desc, worktree, branch, base, artifact, submit | 叙述 |
| `subagent_delta` | Resume {{id}} (rev {{rev}}) for stage `{{stage}}` in worktree `{{worktree}}`; write `{{artifact}}` and submit. | id, rev, stage, worktree, artifact | 叙述 |
| `subagent_fallback_note` | \nResume context: prior executor `{{resume_id}}`. | resume_id | 叙述 |
| `subagent_submit_cmd` | .ygg/bin/egg open <run-root> --event proposal.submit --payload '"{{id}}\|{{rev}}\|{{stage}}\|<summary>\|{{artifact}}\|\|exec=<agent_id>"' | id, rev, stage, artifact | 叙述 |
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
| `tools_name_registered` | [TOOLS NAME REGISTERED] {{name}} -> {{path}} (override for the default <name>.sh; empty path clears it) | name, path | 叙述 |
| `tools_param_bad` | tools parameter value must be non-empty after trim (empty clears), <= 240 chars, no `..` segment, no `\|` or control characters. | — | 叙述 |
| `tools_param_registered` | [TOOLS PARAM REGISTERED] {{name}} = {{value}} (reserved parameter for the settlement cross-check argv) | name, value | 叙述 |
| `tools_relpath_bad` | tools.<name> value must be a script path relative to the root: `.sh` suffix, <= 240 chars, no `..`, no `\|` or control characters (empty clears the override). | — | 叙述 |
| `tools_root_bad` | tools.root must be a directory path: non-empty after trim (empty clears), <= 240 chars, no `..` segment, no `\|` or control characters. | — | 叙述 |
| `tools_root_cleared` | [TOOLS ROOT CLEARED] settlement checks fall back to human attestation. | — | 叙述 |
| `tools_root_registered` | [TOOLS ROOT REGISTERED] {{root}} (self-declared; settlement machine cross-checks enabled when this is set) | root | 叙述 |
| `verify_suite_blocked` | [VERIFY SUITE BLOCKED] {{reason}} | reason | 叙述 |
| `verify_suite_failed` | [VERIFY SUITE FAILED] {{summary}} | summary | 叙述 |
| `verify_suite_passed` | [VERIFY SUITE PASSED] {{summary}} | summary | 叙述 |
| `vfs_ledger_only` | [VFS LEDGER ONLY] {{path}} ({{bytes}} B): this host exposes no `ygg.fs`, so nothing reached the disk - the artifact ledger records the Mod-side assertion only | path, bytes | 叙述 |
| `vfs_path_rejected` | [VFS PATH REFUSED] {{path}}: only artifacts/<name>.md is writable (no `..`, no whitespace, at most {{max}} bytes) | path, max | 叙述 |
| `vfs_usage` | usage: `vfs.write` takes `artifacts/<name>.md\|<content>` (path must be artifacts/*.md, content 1..{{max}} bytes) | max | 叙述 |
| `vfs_write_failed` | [VFS WRITE FAILED] {{path}}: {{reason}} | path, reason | 叙述 |
| `vfs_written` | [VFS WRITTEN] {{path}} ({{bytes}} B, sha256 {{sha}}) | path, bytes, sha | 叙述 |
