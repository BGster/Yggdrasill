# Yggdrasill Domain Context & Operational Invariants

> This document defines the permanent domain model, shared vocabulary, and non-negotiable operational
> invariants of Yggdrasill. Both human coordinators and autonomous subagents must read `docs/CONTEXT.md`
> alongside `docs/DESIGN.md` before proposing plans or editing source code, and must keep both documents
> synchronized before settling a run.

---

## 1. Dual Product Perspective

Yggdrasill is engineered simultaneously from two complementary product perspectives that share one deterministic kernel:

1. **Deterministic Narrative Game Engine (`ygg-core` + `ygg-sandbox` + `ygg-wasm`)**
   - Every session is an event-sourced narrative run (`event.start` -> choices / events -> `[ENDING ...]`).
   - All domain transitions execute inside a deterministic, IO-free LuaJIT sandbox (`ygg-sandbox`) backed by a strict key-value state store (`ygg.kv.*`), structured metadata projections (`ygg.meta`), and narrative emission (`ygg.narrate`, `ygg.choice`).
   - Given identical Mod resources and an identical sequence of input events, the engine produces byte-for-byte identical state transitions, narrative receipts, and save snapshots across native CLI, daemon, and WebAssembly targets.

2. **Universal Agent Harness Workflow Engine (`.ygg/mods/pi-agent-seed` + `ygg-agent` host)**
   - Multi-agent software engineering is modeled as a deterministic coordination game played by a coordinator (human or `goal`-mode main agent) and isolated lane executors (subagents).
   - Requirements, lane worktrees, proposals, mandatory human/coordinator gates, verification suites, and end-of-run self-evolution are first-class state-machine transitions rather than ad-hoc shell scripts.
   - Any external agent harness (Pi, Claude Code, Codex, or CLI drivers) acts as a thin execution shell over the state machine's structured projections (`meta.summary`, `meta.harness_actions`, `meta.tool_calls`).

---

## 2. Core Domain Vocabulary

| Term | Definition & Invariant |
| --- | --- |
| **`Run`** | A single bounded execution session identified by `run_id` and persisted under `.ygg/runs/<run_id>/` (SQLite `run.sqlite` by default, or legacy JSON `autosave.json` + `events.jsonl`). |
| **`Stage`** | Top-level run phase (`intake` -> `coordinate` -> `review` -> `done`). Stored in `fsm.stage`. |
| **`Requirement` (`rN`)** | A unit of engineering delivery tracked in `req.<N>.*`. Stored status is one of 5 core states (`draft`, `analyzing`, `planning`, `developing`, `done`) plus 3 terminal bypass markers (`split`, `merged`, `dropped`), projected into 14 display phases (`PHASE.*`). |
| **`Persistent Lane Pool`** | Bounded set of `1..=job_capacity` isolated git worktrees rooted by default at `$HOME/tmp/worktrees/<repo>-lane-<k>` (overridable via `--root` or `$YGG_WORKTREE_ROOT`). Lanes are claimed in-place (`reused=1`) and parked on release (`parked=1`, detached `HEAD`) without deleting gitignored build caches (`target/`, `node_modules/`). |
| **`Proposal` (`pM`)** | A formal delivery record (`prop.<M>.*`) submitted via `proposal.submit` for a specific `(req, rev, stage)` carrying a summary, VFS artifact path, touched files, and inline executor registration (`exec=<agent_id>`). |
| **`Gate` (`plan` / `accept`)** | Mandatory coordinator checkpoint after a `plan` or `development` proposal is accepted. `gate.approve` requires `gate_rev == req.rev` and (for `accept`) non-empty artifacts; `gate.reject` increments revision or returns the requirement for rework. |
| **`Executor Ledger`** | Self-declared binding `settings.executor.<rN>.<stage>=<agent_id>\|<reason>` recording which subagent executed a stage. Used to determine whether to spawn a fresh subagent or resume an existing session (`subagent.spawn` vs `subagent.resume`). Placeholder actor IDs (`pi-ygg-harness`, `unregistered`, `main`) are never treated as resumable subagent sessions. |
| **`VFS Artifact`** | Run-scoped deliverable stored under `artifacts/<name>` in the run store (`vfs_put` / `vfs_get` / `vfs_list`) and indexed in the bounded `artifact.*` KV ledger with byte length and SHA-256 digest. |
| **`Synchronous HostToolRunner`** | Host-side execution bridge mounted on `KernelFacade`. When a Lua handler or reactive effect invokes `ygg.tool.call(tool_id, args)`, the host executes the allowlisted built-in tool inline and settles `tool.result` + reactive domain state within the same turn. |
| **`HarnessAction`** | Structured descriptor emitted in `meta.harness_actions` and `meta.summary.jobs[i].subagentAction` (`subagent.spawn` or `subagent.resume`) carrying the complete brief, delta prompt, worktree path, artifact path, and submit command so any harness adapter can drive subagents without duplicating Mod logic. |

---

## 3. Non-Negotiable Operational Principles

1. **Data Red Line (Priority 0)**
   - Never read, list, stat, search, or copy real business data files (`*.xlsx`, `*.xls`, `*.csv`) or classified mounts (`/Volumes/WenshuSpace/**`). Synthetic test data under `/tmp` is unrestricted.

2. **Single-Step Immersive Dispatch & Prohibition of Batch Entrypoints**
   - Every in-run artifact and code change must be preceded by a state-machine event (`requirement.dispatch` -> `proposal.submit` -> `proposal.accept` -> `gate.approve`).
   - Batch event flags (`--events`) and shell loops that feed blind event sequences are permanently prohibited. Every dispatch prints the full narrative receipt and inspects the current state before the next decision.

3. **Two Mandatory Gates (`plan_gate` and `accept_gate`)**
   - No requirement may enter `developing` without passing `plan_gate`, and no requirement may enter `done` without passing `accept_gate`. Upper-layer automation (`ygg_run_lanes`) lands work at the gates and never auto-approves them.

4. **Persistent Lane Worktree Reuse Preserving Build Caches**
   - Claiming a lane (`worktree-claim`) reuses an existing `<repo>-lane-<k>` worktree in-place (`git checkout --force -B <branch> <head>`, `git reset --hard <head>`, `git clean -fd` **without `-x`**), and releasing a lane (`worktree-release`) parks it on a detached `HEAD` with `git clean -fd` (**without `-x`**).
   - This frees the feature branch for merging/deletion in the main checkout while keeping `target/` and `node_modules/` warm across requirements and runs.

5. **Single-Source Mod (`packages/mods/pi-agent-seed` -> `../../.ygg/mods/pi-agent-seed`)**
   - `.ygg/mods/pi-agent-seed` is the single source of truth for the seed Mod. `packages/mods/pi-agent-seed` is a relative symlink (`../../.ygg/mods/pi-agent-seed`), eliminating dual-directory drift while preserving package path compatibility.
   - `./scripts/sync-seed-mod.sh` recalculates `.ygg/mods/pi-agent-seed/manifest.json` and verifies the relative symlink (`--check` in CI).
