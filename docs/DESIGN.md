# Yggdrasill Architecture & System Design (`docs/DESIGN.md`)

> This document specifies the three-layer architecture, requirement and persistent-lane lifecycle,
> universal `harnessActions[]` and context-compaction protocol, and the living documentation contract.
> Read this file together with `docs/CONTEXT.md` before proposing a plan or implementing code changes.

---

## 1. Three-Layer Architecture

Yggdrasill separates deterministic state transitions, workflow semantics, and agent-harness orchestration into three strict layers:

### Layer 0: Deterministic Kernel, RunStore & Host Tool Runner (`crates/ygg-*`)

- **`ygg-core` & `ygg-sandbox`**: Pure deterministic state-machine kernel and sandboxed LuaJIT runtime (`ygg.on`, `ygg.kv`, `ygg.narrate`, `ygg.choice`, `ygg.meta`, `ygg.data`, `ygg.fs`, `ygg.tool.call`).
- **`ygg-store`**: Transactional persistence engine supporting the default SQLite backend (`<run-dir>/run.sqlite`) and legacy JSON backend (`autosave.json` + `events.jsonl`), plus a run-scoped Virtual File System (VFS) for `artifacts/*` (`vfs_put`, `vfs_get`, `vfs_list`).
- **`HostToolRunner` (`crates/ygg-sandbox/examples/agent.rs`)**: Contract-driven CLI and JSON-RPC `--daemon` host. Validates the Layer-1 tool allowlist (`pack.json -> tools`) and executes built-in operations (`worktree-claim`, `worktree-release`, `lane-probe`, `artifact-probe`, `verify-suite`, `archive-sessions`) synchronously during `ygg.tool.call`, folding `tool.result` back in the same turn.

### Layer 1: Reactive Workflow State Machine (`.ygg/mods/pi-agent-seed`)

- **Single-Source Mod**: `.ygg/mods/pi-agent-seed` is the single source of truth; `packages/mods/pi-agent-seed` is a relative symlink (`../../.ygg/mods/pi-agent-seed`).
- **Reactive Hooks (`src/hooks.lua`)**: `useMemo` and `useEffect` run inside `render()` via `flush_effects()` (capped at `MAX_PASSES = 4`) before emitting the turn's single `ygg.choice`.
- **Inline Engineering Rituals (`src/tools.lua` + `src/handlers/actions.lua`)**:
  - `auto_dispatch`: Automatically dispatches a requirement in the same turn after `gate.approve(plan)` or `proposal.accept(analysis)` when no gate/proposal/conflict is pending and a lane slot is free.
  - `auto_claim_worktree`, `auto_probe_lane`, `auto_release_worktree`, `auto_archive_sessions`: Triggered inline via synchronous `ygg.tool.call` so worktree claiming, lane probing, worktree parking, and session archiving settle in one turn.
- **Structured Projections (`src/render.lua`)**: Projects `meta.summary` (`jobs`, `gates`, `conflicts`, `pendingTools`, `evolution`) and `meta.harness_actions` (`subagentAction` per active job).

### Layer 2: Thin Execution Shell & Context Compaction (`.pi/extensions/ygg-pi-harness`)

- **`firewall.ts`**: Hard `tool_call` guard enforcing the Data Red Line and blocking batch event entrypoints (`--events`, shell loops calling `ygg-agent`).
- **`daemon.ts`**: Persistent line-oriented JSON-RPC client speaking to `ygg-agent --daemon`.
- **`orchestrator.ts`**: Executes `meta.summary.jobs` across isolated lane worktrees concurrently, archives deliverables into the run VFS, runs lane probes, and submits/accepts proposals to land requirements at `plan_gate` or `accept_gate` (never auto-approving gates).
- **`compaction.ts`**: Evaluates context token pressure (`soft_brake >= 0.60`, `hard_brake >= 0.75`), verifies stable checkpoints (`isStableCheckpoint`), writes `artifacts/handoff-latest.md` to VFS, triggers compaction, and auto-resumes.

---

## 2. Requirement & Lane Lifecycle

### 2.1 Requirement State & Phase Pipeline

Requirements progress through 5 stored states (`draft`, `analyzing`, `planning`, `developing`, `done`) and 14 derived display phases:

```text
[draft]
  │ requirement.dispatch (merges analysis + plan -> dispatches job="plan")
  ▼
[planning] (display: planning -> plan_review -> plan_gate)
  │ proposal.submit(plan) -> proposal.accept -> gate.approve(plan)
  ▼
[planning · plan_ok=true] (display: ready)
  │ requirement.dispatch (auto_dispatch + auto_claim_worktree -> job="development")
  ▼
[developing · lane=k] (display: developing -> dev_review -> accept)
  │ proposal.submit(development) [auto_probe_lane] -> proposal.accept -> gate.approve(accept) [auto_release_worktree]
  ▼
[done]
```

### 2.2 Persistent Lane Claim & Non-Destructive Park Semantics

1. **Worktree Root Resolution (`worktree_root_for`)**:
   - Precedence: explicit `--root` > non-empty `$YGG_WORKTREE_ROOT` > `$HOME/tmp/worktrees` > `<temp_dir>/ygg-worktrees`.
   - Lane path: `<root>/<repo_label>-lane-<k>`.
2. **In-Place Claim (`claim_fields_with`)**:
   - If `<branch>` is currently checked out in another lane worktree (e.g. after a gate rejection re-assigned the requirement to a different lane), the host first detaches `HEAD` in that other worktree (`git checkout --detach HEAD`).
   - If `<lane_dir>` and `<lane_dir>/.git` already exist (`reused=1`), the host reuses the directory in-place:
     - `git -C <lane_dir> checkout --force -B <branch> <head>`
     - `git -C <lane_dir> reset --hard <head>`
     - `git -C <lane_dir> clean -fd` (**without `-x`**, preserving gitignored caches such as `target/` and `node_modules/`)
     - Emits `lane=<k>;req=<rN>;branch=<branch>;base=<head>;clean=1;reused=1;path=<lane_dir>`.
   - If `<lane_dir>` does not exist (`reused=0`), runs `git worktree prune` and `git worktree add -B <branch> <lane_dir> <head>`, emitting `reused=0`.
3. **Non-Destructive Release / Parking (`release_fields_with`)**:
   - When `<lane_dir>` exists (`parked=1`), the host parks the worktree instead of deleting it:
     - `git -C <lane_dir> checkout --force --detach HEAD` (unlocks `<branch>` so the main checkout can merge and `git branch -d <branch>`)
     - `git -C <lane_dir> reset --hard HEAD` and `git -C <lane_dir> clean -fd` (**without `-x`**)
     - Emits `lane=<k>;released=1;parked=1;path=<lane_dir>`.
   - When `<lane_dir>` is absent (`parked=0`), runs `git worktree prune` and emits `lane=<k>;released=1;parked=0;path=<lane_dir>`.

### 2.3 Live Progress Rendering (`worktreeStatus`, `subagentStatus`, `probeStatus`)

`src/render.lua` computes three live status indicators for every requirement and exports them in both `requirement.list` (`(wt=<worktreeStatus>)(subagent=<subagentStatus>)` on active rows) and `meta.summary.jobs[i]`:

- **`worktreeStatus`**:
  - `ready:<worktree_path>@<branch>` when `lane > 0`, `lane.<k>.req == id`, and `lane.<k>.path` is populated during development/review/accept.
  - `bound:lane-<k>@<branch>` when bound to `lane.<k>` without a recorded worktree path.
  - `pending` when `phase == "ready"`.
  - `n/a` otherwise.
- **`subagentStatus`**:
  - `delivered:p<M>(<exec_id>)` (or `delivered:p<M>`) when a pending proposal exists for the requirement.
  - `dispatched:<exec_id>` when `job ~= ""` and `exec_id` is registered.
  - `pending` when `job ~= ""` and no executor is registered yet.
  - `idle` otherwise.
- **`probeStatus`**:
  - `verified:commits=<N>,clean=<0|1>` when `lane > 0` and `lane.<k>.probe.head` is recorded.
  - `pending` when `job == "development"` and no probe has settled yet.
  - `n/a` otherwise.

---

## 3. Universal `harnessActions[]` & Context Compaction Protocol

### 3.1 `subagentAction` & `is_resumable_agent_id` Guard

For every active job in `summary_jobs()`, the Mod emits a `subagentAction` descriptor (also collected at top level in `meta.harness_actions`):

- **`is_resumable_agent_id(id)`**: Returns `false` for `""`, `"unregistered"`, `"pi-ygg-harness"`, `"main"`, or `"default"`. Only genuine subagent session handles/UUIDs return `true`.
- When `resume_id` passes `is_resumable_agent_id`, `subagentAction.action` is `"subagent.resume"` with `resumeAgentId = resume_id` and a concise `deltaPrompt`.
- When the recorded executor is empty or a placeholder such as `"pi-ygg-harness"`, `resume_id` resolves to `""` and `subagentAction.action` stays `"subagent.spawn"`, preventing false resume attempts against a non-existent subagent session.
- `subagentAction.brief` always embeds the mandatory reference directive:
  `"Mandatory references: read docs/CONTEXT.md (domain invariants) and docs/DESIGN.md (architecture & contracts) before proposing or editing."`

### 3.2 Context Pressure Watermarks & Stable Checkpoints

- **Watermarks**: `soft_brake` at `ratio >= 0.60` (finish current wave, then hand off and compact); `hard_brake` at `ratio >= 0.75` (spawn no new wave; write handoff and compact immediately).
- **Stable Checkpoint Predicate (`isStableCheckpoint`)**: `summary.available && pendingTools === 0 && (gates.length > 0 || jobs.length === 0 || conflicts.length > 0)`.
- **Handoff Artifact**: Written to VFS at `artifacts/handoff-latest.md` before compaction, preserving run stage, token ratio, active jobs (`executor` / `planExecutor`), pending gates, conflicts, VFS artifacts, and the exact next command.

---

## 4. Living Documentation Contract

`docs/CONTEXT.md` and `docs/DESIGN.md` form a closed-loop engineering contract across every Yggdrasill run:

1. **Mandatory Input (`plan` & `development`)**:
   - Every subagent brief generated by `src/render.lua` instructs the executor to read `docs/CONTEXT.md` (domain invariants) and `docs/DESIGN.md` (architecture & contracts) before proposing a plan or editing code.
2. **Mandatory Settlement Sync (`review` -> `settlement.evolution`)**:
   - Before registering `settlement.evolution` and closing a run with `review.note`, the coordinator and executors must verify that any newly introduced domain concepts, state-machine rules, tool contracts, or architectural changes are reflected in `docs/CONTEXT.md` and `docs/DESIGN.md`.
   - Enforced by the Mod playbook (`harness.living-docs-closed-loop` and `evolution.living-docs-sync`).
