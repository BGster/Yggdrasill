# Yggdrasill · System Architecture, Behavioral Semantics & Interface Contracts (`docs/DESIGN.md`)

> **Positioning**: `docs/DESIGN.md` specifies the **Product Form, System Behavior, State Machine Semantics, and Canonical Interface Contracts** of Yggdrasill. Domain vocabulary, vision, architectural principles, and the decision ledger (`D1–D20`, `D-MA1..8`) live in [`docs/CONTEXT.md`](CONTEXT.md). To remain durable across implementation refactors, this document defines contracts at the boundary level (**JSON Schemas, CLI/JSON-RPC Wire Protocols, Lua Sandbox Host APIs, Event Payloads, and State Transition Tables**) rather than internal source function names or line numbers.

---

## 1. Product Form & Three-Layer System Boundaries

Yggdrasill separates deterministic rule execution, declarative workflow semantics, and host environment integration into three strictly bounded layers:

```text
┌────────────────────────────────────────────────────────────────────────────┐
│ Layer 2 · Presentation & Harness Adapter Shell                             │
│   - Game Clients: Web Browser (React + ygg-wasm), Godot 4 (gdext)          │
│   - Agent Adapters: Pi Extension (.pi/extensions/ygg-pi-harness),          │
│     CLI Daemon Consumers (Claude Code / Codex / Cursor hooks)              │
│   Responsibilities: UI rendering, subagent process spawning/resuming,      │
│   context window telemetry & compaction, data red-line firewall.           │
├────────────────────────────────────────────────────────────────────────────┤
│ Layer 1 · Playable Mod Package (.ygg/mods/pi-agent-seed, test-adventure)   │
│   - Executable Logic: Entry manifest (mod.lua) + pure modules (src/*.lua)  │
│   - Declarative Resources: data/scenes.json, data/config.json,             │
│     data/playbook.json, pack.json, manifest.json                           │
│   Responsibilities: Two-layer state machine, gate predicates, reactive     │
│   hooks, tool reactors, narrative receipts, and HarnessAction projection.  │
├────────────────────────────────────────────────────────────────────────────┤
│ Layer 0 · Deterministic Rule Kernel, Sandbox & Storage Port                │
│   - Rule Kernel (ygg-core) + Schema Validator (ygg-schema)                 │
│   - Deterministic Lua 5.4 Sandbox & Host Contract (ygg-sandbox)            │
│   - Single-File Transactional Store & VFS (ygg-store -> run.sqlite)        │
│   Responsibilities: Atomic (state, event) -> state' transitions,           │
│   savepoint rollback, instruction fuel enforcement, VFS & tool bridging.   │
└────────────────────────────────────────────────────────────────────────────┘
```

### 1.1 Cross-Harness Capability Negotiation Matrix

Because Layer 1 (`pi-agent-seed`) emits self-contained `HarnessAction` specifications (`summary.jobs[i].subagentAction`) and executes deterministic rituals inline via Layer 0 (`ygg.tool.call`), any coding agent host can drive Yggdrasill according to its capability tier:

| Tier | Host Environment | Subagent Execution & Resume | Context Telemetry & Compaction | Integration Mechanism |
| --- | --- | --- | --- | --- |
| **Tier 1 · Native Extension** | Pi Coding Agent (`ygg-pi-harness`) | Native `Agent({ resume })` with automatic fallback to `fullFallbackBrief` | Live token ratio (`soft >= 60%`, `hard >= 75%`), automatic `artifacts/handoff-latest.md` + `compact()` | Persistent `ygg-agent --daemon` stdio JSON-RPC client |
| **Tier 2 · CLI / Hook Runner** | Claude Code, Codex CLI, Cursor | Subprocess invocation or native task tool consuming `subagentAction.brief` and `submitCommand` | Hook-triggered `handoff_compact` at stable checkpoints | `ygg-agent --daemon` or single-shot `--resume --event` CLI |
| **Tier 3 · Prompt Delegation** | Generic LLM Chat / Custom Agent Loop | Coordinator copies `subagentAction.brief` into subagent context and relays `proposal.submit` | Manual checkpoint handoff via `vfs.write` | Single-shot CLI receipts (`narrative` + `pendingChoice`) |

---

## 2. Workflow State Machine & Behavioral Semantics

### 2.1 Macro Coordinator Stage Machine

The Coordinator governs the overall run lifecycle across 4 stages:

| Current Stage | Event | Guard / Precondition | Next Stage | Behavioral Effect |
| --- | --- | --- | --- | --- |
| `intake` | `task.intake` | Non-empty `title\|description` payload | `coordinate` | Records task metadata and opens requirement coordination. |
| `coordinate` | `coordinate.submit` | `pendingProposals == 0` $\land$ `jobs == 0` $\land$ `conflicts == 0` $\land$ all leaf requirements `done` | `review` | Resets `verify.passed = false` and clears current-round `settlement.*` keys. |
| `review` | `verify.submit` / `gate.verify_suite` | `all_green=true` (or all verification commands exit `0`) | `review` | Latches `verify.passed = true` (`verify.source = "human"` or `"tool"`). |
| `review` | `verify.submit` | `all_green=false` when `gates.require_all_green == true` | `coordinate` | Sends the run back to `coordinate` for rework and resets settlement registrations. |
| `review` | `settlement.evolution` | `verify.passed == true` $\land$ valid candidate/class counts | `review` | Registers evolution summary/artifact, settles `sessions=` and `todo=`, and triggers inline `harness.archive_sessions`. |
| `review` | `review.note` | `verify.passed == true` $\land$ `settlement.*` complete $\land$ (`play.mode != "goal"` $\lor$ `goal.authorization != ""`) | `done` | Archives evolution record into `review.evolution.*`, writes `ending = "done"`, and emits `[ENDING · run closed]`. |

### 2.2 Requirement Lifecycle & Mandatory Gate Semantics

Each requirement `rN` is stored using 5 canonical statuses (`draft`, `planning`, `developing`, `done`, plus terminal bypass statuses `split`, `merged`, `dropped`) and orthogonal state attributes (`plan_ok`, `job`, `gate`, `lane`, `rev`, `conflict`, `needs_amend`):

| Stored Status & Attributes | Derived Display Phase | Triggering Event | Target Status & Attributes | Semantic Contract |
| --- | --- | --- | --- | --- |
| `draft` | `draft` | `requirement.dispatch` | `planning` (`job="plan"`) | **Stage Merge**: Directly dispatches the `plan` job without a separate `analysis` turn. |
| `planning` (`job="plan"`) | `planning` | `proposal.submit` (`stage="plan"`) | `planning` (`job="plan"`, pending proposal `pK`) | Subagent delivers plan artifact (`artifacts/rN-plan.md`) and binds `executor.rN.plan` via trailing `exec=<agent_id>`. |
| `planning` (pending `pK`) | `plan_review` | `proposal.accept` | `planning` (`job=""`, `gate="plan"`, `gate_rev=rev`) | Releases the job slot and raises the mandatory `plan_gate`. |
| `planning` (`gate="plan"`) | `plan_gate` | `gate.approve` | `planning` (`gate=""`, `plan_ok=true`) $\to$ **Reactive Auto-Dispatch** $\to$ `developing` (`job="development"`, `lane=k`) | Approving `plan_gate` marks `plan_ok=true`; when a lane slot is free and no conflicts/gates block, the reactive hook immediately claims `lane-k` and dispatches `development` in the **same turn**. |
| `planning` (`gate="plan"`) | `plan_gate` | `gate.reject` | `planning` (`gate=""`, `plan_ok=false`, `rev=rev+1`) | Invalidates the plan, increments `rev`, and returns to the planning queue. |
| `developing` (`job="development"`, `lane=k`) | `developing` | `proposal.submit` (`stage="development"`) | `developing` (pending proposal `pM`, inline `git.lane_probe`) | Binds `executor.rN.development`, synchronously probes `lane-k` (`commits`, `clean`, `head`, `files`), and runs cross-lane conflict radar. |
| `developing` (pending `pM`) | `dev_review` | `proposal.accept` | `developing` (`job=""`, `gate="accept"`, `gate_rev=rev`) | Releases the active job slot and raises the mandatory `accept_gate` (lane remains bound until `accept_gate` passes). |
| `developing` (`gate="accept"`) | `accept` | `gate.approve` | `done` (`gate=""`, `lane=0`, inline `git.worktree_release`) | Requires non-empty `artifacts` and `gate_rev == rev`. Marks requirement `done` and parks `lane-k` on detached `HEAD` in the **same turn**. |
| `developing` (`gate="accept"`) | `accept` | `gate.reject` | `planning` (`gate=""`, `rev=rev+1`, keeps prior `plan_ok`) | If unamended (`plan_ok=true`), displays as `ready` to re-enter development; if amended (`plan_ok=false`), displays as `planning` to replan. |

#### Bypass & Exception Transitions
- **`requirement.amend`**: Increments `rev` and invalidates `plan_ok` (unless already `developing`, where progress is kept). If a gate was pending, `gate_rev != rev` renders the gate `stale` and blocks `gate.approve`. Amending a `done` requirement derives a new `draft` requirement with `origin=rN`.
- **`requirement.split` / `requirement.merge`**: Valid only when the requirement is in `draft` / `analyzing` / `planning` with no active job, gate, lane, or conflict. Parent/source requirements enter terminal `split` / `merged` states while child/target requirements start at `draft`.
- **`requirement.conflict` & Conflict Radar**: Raises a bidirectional conflict between two requirements (`req.i.conflict`, `req.i.conflict_peer`), halting dispatch and closing until resolved via `requirement.resolve` (`keep_both`, `keep_first`, or `revise`).

### 2.3 Reactive Single-Successor Advancement & Effect Convergence

Before emitting `pendingChoice`, the Mod flushes registered reactive effects (`useEffect`) in deterministic registration order (up to `MAX_PASSES = 4`; non-convergence is a hard error). Every reactive effect must obey four invariants:
1. **Unique Declared Successor**: The transition is deterministic and declared in one place.
2. **Zero External Judgment**: Requires no human approval or unmounted external I/O.
3. **Never Bypasses a Gate**: Milestone pass actions (`proposal.accept`, `gate.approve`, `coordinate.submit`, `verify.submit`, `review.note`) are never triggered by an effect.
4. **Effect Without Render**: Effects mutate KV and call `ygg.narrate`, but never call `render()` or `ygg.choice`.

### 2.4 Persistent Lane Pool & Live Progress Projection

- **Non-Destructive Claim (`worktree-claim`)**: Allocates `<repo-parent>/worktrees/<repo>-lane-<k>` ($k \in 1..4$). If the worktree already exists and its tracked working tree is clean, it runs `git checkout -B <branch> HEAD` so untracked build caches (`target/`, `node_modules/`) survive across tasks.
- **Non-Destructive Release (`worktree-release`)**: Verifies the worktree is clean and runs `git checkout --detach HEAD`, unlocking `feature/<req>` for Coordinator merge without deleting the directory.
- **Live Status Projection**: Every requirement row in `requirement.list` and `summary.jobs[]` projects three real-time status strings derived from KV:
  - `worktreeStatus`: `"n/a"` | `"pending"` | `"ready:<path>@<branch>"` | `"bound:lane-<k>@<branch>"`
  - `subagentStatus`: `"idle"` | `"pending"` | `"dispatched:<agent_id>"` | `"delivered:p<N>(<agent_id>)"`
  - `probeStatus`: `"n/a"` | `"pending"` | `"verified:commits=<N>,clean=<0|1>"`

---

## 3. Interface Contracts (The Canonical Wire & API Surface)

### 3.1 Lua Sandbox Host API Contract (`ygg.*`)

The Lua 5.4 sandbox exposes only safe standard libraries (`base`, `table`, `string`, `math`, `utf8`, `coroutine`) plus the frozen `ygg` global table:

| API Surface | Signature & Return Contract | Side-Effect Class (`YGG-MOD-EFFECT`) |
| --- | --- | --- |
| `ygg.on(event_name, handler_fn)` | Registers an event handler `function(payload)`. Sealed after `mod.lua` finishes loading; calling at runtime or inside a sub-module top level is an error. | **Banned at module top level** (Entry `mod.lua` or `M.install` only) |
| `ygg.emit(event_name, payload)` | Enqueues an internal derived event (FIFO, capped by `max_cascade_depth`). Dropped if called after `ygg.choice` in the same handler (`YGG-R1`). | **Banned at module top level** |
| `ygg.narrate(text)` | Appends a line to the current transition's transient narrative receipt buffer. | **Banned at module top level** |
| `ygg.choice(prompt, options_array)` | Writes `pending_choice` into state and **immediately terminates** the current handler via a sentinel error. Player selection arrives as a new external event `ygg.choice_selected` (1-based integer index). | **Banned at module top level** |
| `ygg.meta(key, json_value)` | Writes structured metadata (`summary`, `tool_calls`, `harness_actions`) into the current dispatch receipt's transient `meta` map. | **Banned at module top level** |
| `ygg.data(resource_id)` | Pure read returning a deep Lua table copy of a schema-validated JSON resource declared in `pack.json` `entries`. | Allowed at module top level (Pure read) |
| `ygg.kv.get(key)` / `ygg.kv.set(key, val)` | Reads or writes persistent key-value state (`nil`, `boolean`, `number`, `string`, or JSON-serializable table). Rolled back on handler failure. | `get` allowed; `set` **banned at module top level** |
| `ygg.entity.*` | `spawn()`, `exists(id)`, `destroy(id)`, `get_attribute(id, k)`, `set_attribute(id, k, v)`, `add_tag(id, t)`, `remove_tag(id, t)`, `has_tag(id, t)`. Return numeric entity IDs (`YGG-R2` requires persisting IDs in `ygg.kv`). | Queries allowed; mutations **banned at module top level** |
| `ygg.fsm.*` / `ygg.narrative.*` | Embedded Lua stdlib helpers for flat finite-state machines (`fsm.define`, `fsm.current`, `fsm.on_event`) and story nodes (`narrative.node`, `narrative["goto"]`; dot-form `narrative.goto` is a Lua 5.4 syntax error caught by `YGG-R3`). | `define` / `node` / `goto` **banned at module top level** |
| `ygg.fs.{write,read,stat,list}` | Optional host-injected VFS interface scoped strictly to `artifacts/*.md`. `write(path, content)` returns `{ bytes, sha256 }`. | Mutating calls **banned at module top level** |
| `ygg.tool.call(tool_id, args_table)` | Optional synchronous host tool bridge. When mounted, executes the allowlisted tool inline and returns `{ ok = true, data = { ... } }` or `{ ok = false, error = "..." }`. When unmounted, returns `{ ok = false, unmounted = true }` so the Mod falls back to async `meta.tool_calls`. | **Banned at module top level** |

### 3.2 Mod Package & Multi-File Purity Contract (`pack.json`, `manifest.json`, `data/*`)

A Mod package directory contains:
1. **`pack.json` (Authoring & Host Contract Declaration)**:
   - `id`, `version`, `engineVersion` (semver range), `seed`, `limits` (`maxFuelPerDispatch`, `maxCascadeDepth`, `maxMemoryBytes`).
   - `code` (optional): `{ "entry": "mod.lua", "modules": ["src/util.lua", ...] }`. When present, `entry` must not appear in `modules`, and every module chunk top level is enforced pure by `ygg_modules::effects`.
   - `entries`: Maps logical resource IDs (`config`, `scenes`, `playbook`) to `{ path, schema }`.
   - `agent`: 8 mandatory fields (`startEvent`, `summaryEvent`, `choiceEvent`, `endingPrefix`, `rejectionPrefixes`, `configEntry`, `guideEntry`, `guideEvent`).
   - `inboxRoutes`: Declares `idField`, `externalIdField`, `kindField`, and per-kind `{ event, fields[] }` mappings for asynchronous JSONL inbox ingestion.
   - `tools` (optional, **Layer 1 Capability Whitelist**): Maps `<namespace>.<name>` to `{ kind: "cli" | "http", program, args[], timeoutMs, maxOutputBytes, readOnly, resultEvent }`. Placeholders use `{{arg}}` for call arguments and `$ENV{VAR}` for host-side secret resolution.
2. **`data/config.json` (Runtime Defaults & Layer 2 Release Whitelist)**:
   - Validated against `packages/schema/schemas/agent-config.json`.
   - Declares `play.mode` (`"guide"` | `"goal"`), `evolution` (`mode`, `allow_paths`, `max_per_run`), `gates.require_all_green`, `drive` (`maxSteps`, `stagnationLimit`, `decideTimeoutMs`), `settlement` (`requireNonEmptyEvents`, `requireZeroFailedSteps`, `requireEnding`, `requireInboxConsumed`), `allowedTools` (**Layer 2 Release Whitelist**, must be a superset of `pack.json` `tools` keys), and optional `harness` adapter metadata.
3. **`manifest.json` (Content-Addressed Integrity Lock)**:
   - Validated against `packages/schema/schemas/manifest.json`. Records `{ path, sha256, byteLength }` for every code file and data resource. Maintained via `./scripts/sync-seed-mod.sh`.

### 3.3 Event Payload Wire Contract

All events dispatched to `pi-agent-seed` use pipe-delimited (`|`) UTF-8 string payloads (with optional trailing `@ext=<id>` idempotency keys where supported):

| Event Name | Wire Payload Format | Contract & Validation Rules |
| --- | --- | --- |
| `event.start` | `"0"` or `""` | Initializes defaults and renders the `intake` stage receipt. |
| `task.intake` | `"<title>\|<description>"` | Moves `intake -> coordinate`. |
| `requirement.add` | `"<title>\|<description>[\|<ext_id>[\|<dep1,dep2>]]"` | Adds `rN` in `draft`. Validates DAG acyclicity and idempotency on `ext_id`. |
| `requirement.dispatch` | `""` \| `"0"` \| `"rN"` | Dispatches ready requirements up to `job_capacity` (4). `draft` rows enter `planning`; `ready` (`planning` + `plan_ok`) rows claim a lane and enter `developing`. |
| `proposal.submit` | `"<req_id>\|<rev>\|<stage>\|<summary>\|<artifacts>\|[<changes>]\|[<ext_id>][\|exec=<agent_id>[:<reason>]]"` | Submits a proposal for `analysis`, `plan`, or `development`. `exec=` must be the final segment and binds the executor ledger in the same transaction. |
| `proposal.accept` | `""` \| `"0"` \| `"pN"` \| `"rN"` | Accepts the head (or specified) pending proposal. Requires menu ticket in `guide` mode. |
| `proposal.reject` | `"<pN_or_rN>[\|<reason>]"` | Rejects the pending proposal and returns the requirement to its stage queue. |
| `gate.approve` | `""` \| `"0"` \| `"rN[\|<note>]"` | Passes `plan_gate` ($\to$ `ready` + reactive auto-dispatch) or `accept_gate` ($\to$ `done` + inline lane release). Requires `gate_rev == req.rev` and non-empty `artifacts` for `accept_gate`. |
| `gate.reject` | `"<rN>[\|<feedback>]"` | Rejects `plan_gate` (`plan_ok=false`) or `accept_gate` (preserves `plan_ok`), increments `rev`, and returns to `planning`. |
| `requirement.amend` | `"<req_id>\|<title>\|<description>\|<deps>"` | Empty segment preserves current value; `deps="-"` clears dependencies. Increments `rev`. |
| `requirement.split` | `"<parent_id>\|<child_title_1>\|<child_title_2>[\|...]"` | Splits a pre-development requirement into $\ge 2$ `draft` children. |
| `requirement.merge` | `"<rA,rB>\|<merged_title>\|<merged_desc>"` | Merges $\ge 2$ pre-development requirements into a new `draft` requirement with unioned dependencies. |
| `requirement.conflict` | `"<rA,rB>\|<note>"` | Marks both requirements `blocked` until resolved. |
| `requirement.resolve` | `"<rA,rB>\|<keep_both\|keep_first\|revise>[\|<note>]"` | Clears conflict and applies the chosen resolution policy. |
| `coordinate.submit` | `""` \| `"0"` | Advances `coordinate -> review` once all closing preconditions hold. |
| `verify.submit` | `"all_green=<true\|false>[;report=<text>]"` | Records verification outcome in `review`. |
| `settlement.evolution` | `"<artifact>\|<summary>[\|c=<kept>/<screened>[\|cls=<kind>:<n>,...]][\|sessions=<moved>/0/<dest>][\|todo=0/<verified_by>]"` | Registers evolution screening and settles session/todo cleanups in one transaction. |
| `review.note` | `"<lesson>\|<score_1_to_5>"` | Archives `settlement.evolution.*` into `review.evolution.*` and closes the run (`[ENDING`). |
| `config.set` | `"<key>=<value>"` | Updates runtime `settings.<key>` (`play.mode`, `goal.authorization`, `executor.<rN>.<stage>`, `evolution.mode`, `gates.require_all_green`, `review.evolution.*`). |
| `vfs.write` | `"artifacts/<name>.md\|<content>"` | Writes content to `run.sqlite` (`vfs_files`) via `ygg.fs.write` and records SHA-256 + byte length in `artifact.*`. |

### 3.4 Host CLI & Daemon JSON-RPC Wire Contract

#### Single-Shot CLI & Builtin Tool Flags (`ygg-agent`)
- **Run Execution**: `.ygg/bin/ygg-agent --mod <mod_dir> --run-id <id> --runs-root .ygg/runs [--resume] --event <event> [--payload <json>]`
- **Settlement**: `.ygg/bin/ygg-agent --mod <mod_dir> --run-id <id> --runs-root .ygg/runs --settle`
- **Builtin Host Tools (`--builtin-tool <op>`)**:
  - `worktree-claim --lane <1..4> --req <rN> --branch <branch> [--repo <path>]` $\to$ `lane=<k>;req=<rN>;branch=<br>;base=<sha>;clean=1;path=<wt_path>`
  - `lane-probe --lane <1..4> --base <sha> --branch <branch> [--repo <path>]` $\to$ `lane=<k>;commits=<N>;clean=<0|1>;head=<sha>;files=<csv>`
  - `worktree-release --lane <1..4> [--repo <path>]` $\to$ `lane=<k>;released=1;idle=<0|1>`
  - `artifact-probe --path artifacts/<name>.md [--run-dir <dir>]` $\to$ `path=<p>;exists=<0|1>;bytes=<N>;sha256=<hex>`
  - `verify-suite [--mode fast|full] [--repo <path>]` $\to$ `passed=1;summary=<M>/<M>` (Honors `YGG_VERIFY_COMMANDS="cmd1 args;cmd2 args"` environment override before falling back to the default Rust cargo gates).
  - `archive-sessions --parent <id> --batch <run_id>` $\to$ `moved=<N>;remaining=0;dest=<path>`

#### Daemon Mode (`--daemon`) Line-Delimited JSON-RPC Protocol
Each request is a single UTF-8 JSON line on `stdin`; each response is a single UTF-8 JSON line on `stdout`:
- **Request**: `{ "id": <number|string>, "op": "<ping|summary|dispatch|vfs_put|vfs_get|vfs_list|settle|shutdown>", ...params }`
- **Response**: `{ "id": <number|string>, "ok": true, "result": <object> }` or `{ "id": <number|string>, "ok": false, "error": "<message>" }`

#### Structured Summary & `HarnessAction` Schema (`result.summary` & `result.receipt`)
```json
{
  "stage": "coordinate",
  "ending": false,
  "verifyPassed": false,
  "nextEndsRun": false,
  "pendingProposals": 0,
  "requirementCount": 1,
  "pendingTools": 0,
  "conflicts": [],
  "gates": [],
  "jobs": [
    {
      "id": "r1",
      "rev": 1,
      "stage": "development",
      "status": "developing",
      "phase": "developing",
      "lane": 1,
      "branch": "feature/r1",
      "base": "7a40830",
      "worktreePath": "/Users/.../tmp/worktrees/Yggdrasill-lane-1",
      "worktreeStatus": "ready:/Users/.../tmp/worktrees/Yggdrasill-lane-1@feature/r1",
      "subagentStatus": "pending",
      "probeStatus": "pending",
      "executor": "",
      "planExecutor": "agent-plan-1",
      "title": "...",
      "description": "...",
      "subagentAction": {
        "action": "subagent.resume",
        "reqId": "r1",
        "rev": 1,
        "stage": "development",
        "lane": 1,
        "resumeAgentId": "agent-plan-1",
        "worktreePath": "/Users/.../tmp/worktrees/Yggdrasill-lane-1",
        "artifactPath": "artifacts/r1-development.md",
        "brief": "Requirement r1 (rev 1, stage=development): ...\nMandatory references: read docs/CONTEXT.md (domain invariants) and docs/DESIGN.md (architecture & contracts) before proposing or editing.\n...",
        "deltaPrompt": "Resume r1 (rev 1) for stage `development` in worktree `...`; write `artifacts/r1-development.md` and submit.",
        "fullFallbackBrief": "Requirement r1 ...\nResume context: prior executor `agent-plan-1`.",
        "submitCommand": ".ygg/bin/ygg-agent --mod .ygg/mods/pi-agent-seed --runs-root .ygg/runs --resume --event proposal.submit --payload '\"r1|1|development|<summary>|artifacts/r1-development.md||exec=<agent_id>\"'"
      }
    }
  ]
}
```

### 3.5 Context Compaction & Checkpoint Handoff Contract

Long-running multi-requirement runs are protected against context rot by deterministic compaction watermarks in Layer 2:
- **Effective Window Cap**: Defaults to `min(modelWindow, 200_000)` tokens (overridable via `YGG_EFFECTIVE_CONTEXT_WINDOW`).
- **Watermarks**:
  - `ratio < 0.60` $\to$ `normal`
  - `0.60 <= ratio < 0.75` $\to$ `soft_brake` (finish landing the active wave of subagents at their gates, then write handoff and compact)
  - `ratio >= 0.75` $\to$ `hard_brake` (spawn no new subagents; immediately write handoff and compact)
- **Stable Checkpoint Predicate**: Compaction is only permitted when `summary.available == true` $\land$ `pendingTools == 0` $\land$ (`gates.length > 0` $\lor$ `jobs.length == 0` $\lor$ `conflicts.length > 0`), unless explicitly forced via `handoff_compact`.
- **Handoff Artifact (`artifacts/handoff-latest.md`)**: Persisted into `run.sqlite` VFS before compaction and re-injected post-compaction, containing the run ID, stage, token ratio, jobs table (with `executor` / `planExecutor`), pending gates, unresolved conflicts, VFS artifact list, and the exact next `ygg_session` / `ygg_run_lanes` command.

---

## 4. Living Documentation Loop

`docs/CONTEXT.md` and `docs/DESIGN.md` are enforced as a closed-loop contract across every run:
1. **Pre-Execution Injection**: The Mod's `subagent_brief` template (`data/scenes.json`) and the extension's brief builder (`buildSubagentBrief`) inject a mandatory directive into every `plan` and `development` subagent brief: *"read `docs/CONTEXT.md` (domain invariants) and `docs/DESIGN.md` (architecture & contracts) before proposing or editing."*
2. **Pre-Settlement Synchronization**: Whenever a requirement modifies domain concepts, architectural decisions (`D1+`), module boundaries, state machine transitions, or wire/API contracts, the executor must update `docs/CONTEXT.md` and/or `docs/DESIGN.md` in the same feature branch before `accept_gate` and final `review` settlement.
