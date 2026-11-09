# Yggdrasill · Domain Context, Vision, Decisions & Core Principles (`docs/CONTEXT.md`)

> **Positioning**: `docs/CONTEXT.md` is the canonical **Domain Knowledge, Vision, Principles, and Architectural Decision Ledger** of the Yggdrasill repository. Together with [`docs/DESIGN.md`](DESIGN.md) (which specifies Product Form, System Behavior, and Interface Contracts), these two living documents replace all legacy fragmented documentation under `docs/`. Every planning (`plan`) and implementation (`development`) task must consult `docs/CONTEXT.md` and `docs/DESIGN.md` before proposing or modifying code, and any architectural or contract change must update them in the same iteration before final `review` settlement.

---

## 1. Product Vision & Dual Identity

Yggdrasill is designed around a unified thesis: **a deterministic, data-driven rule kernel where Lua scripts and schema-validated JSON resources define playable worlds, and AI acts as both content creator and autonomous player**.

This architecture serves two complementary product identities on the exact same kernel:

### 1.1 Game & Interactive Fiction Platform ("Minecraft + AI Modder")
- **Core Value**: A pure functional state transition engine `(state, event) -> state'` capable of powering text adventures first and scaling into full tactical/narrative RPGs without rewriting the kernel (text adventure = RPG minus combat/map/growth subsystems).
- **Infinite Content Supply**: Traditional UGC platforms are bottlenecked by human modding effort. Yggdrasill pairs a small, orthogonal Mod contract (Lua 5.4 logic + JSON Schema data) with an AI generation and verification pipeline (`ygg-lint` + `playtest`) so a natural-language premise can be compiled into an immediately playable Mod.
- **Write Once, Run Everywhere**: Built in Rust with zero rendering or platform I/O dependencies, the same kernel compiles to WebAssembly (`ygg-wasm` for browsers), links natively into Godot (`gdext`), and runs natively on authoritative servers (for multiplayer lockstep replay or WeChat Mini-Game client-server architectures).

### 1.2 Agent Workflow Harness ("Workflow as a Game, Harness as the Kernel")
- **Core Value**: Software engineering multi-agent coordination is modeled as a playable Mod (`pi-agent-seed`). Instead of relying on fragile static prompt instructions that decay over long context windows, the Agent interacts with the workflow state machine step-by-step via event envelopes.
- **Situational Affordances**: Every dispatch returns a full narrative receipt describing the current state, active lanes, pending proposals, and an explicit `pendingChoice` action whitelist. Protocol constraints replace prompt-only discipline.
- **Two-Layer Coordination**: A main Coordinator Agent decomposes requirements, dispatches parallel Executor Subagents into isolated persistent Git worktree lanes, serially reviews proposals, passes two mandatory human/coordinator gates (`plan_gate` and `accept_gate`), and settles each run with structured roguelike self-evolution review.

---

## 2. Core Domain Vocabulary

| Term | Domain Definition |
| --- | --- |
| **Run** | A single isolated execution instance identified by `run_id` under `.ygg/runs/<run_id>/`, persisted in a single SQLite database (`run.sqlite`) containing the event log, state snapshots, inbox queue, and virtual filesystem artifacts. |
| **Coordinator Stage (`stage`)** | The macro lifecycle of a Run: `intake` (task injection & requirement filing) $\to$ `coordinate` (dispatching lanes, accepting proposals, passing gates) $\to$ `review` (verification suite, evolution screening, session/todo cleanup) $\to$ `done` (terminal settled state). |
| **Requirement (`r1..rN`)** | A unit of delivery tracked by the state machine. Stored as 5 canonical states (`draft`, `planning`, `developing`, `done`, plus terminal bypass states `split` / `merged` / `dropped`) and projected into fine-grained display phases (`draft`, `planning`, `plan_review`, `plan_gate`, `ready`, `developing`, `dev_review`, `accept`, `done`, `blocked`) via `plan_ok`, `job`, `gate`, and `conflict` attributes. |
| **Persistent Lane Pool (`lane-1..4`)** | A bounded pool of up to 4 reusable Git worktrees (`~/tmp/worktrees/<repo>-lane-<k>`). Claiming a lane resets its branch to `HEAD` without deleting untracked build caches (`target/`, `node_modules/`), and releasing a lane parks it on a detached `HEAD` so branches are never locked. |
| **Proposal (`p1..pM`)** | An immutable delivery submission (`proposal.submit`) produced by an Executor Subagent for a specific `(req_id, rev, stage)` tuple, referencing an in-archive VFS artifact (`artifacts/<req>-<stage>.md`) and binding the executor identity (`exec=<agent_id>`). Proposals do not mutate requirement state until the Coordinator serially runs `proposal.accept` or `proposal.reject`. |
| **Mandatory Gates (`plan_gate`, `accept_gate`)** | Two non-skippable quality checkpoints per requirement. `plan_gate` approves a technical plan (`planning` $\to$ `ready`) and reactively auto-dispatches development. `accept_gate` requires non-empty artifact evidence (`artifacts`) and matching revision (`gate_rev == req.rev`) before marking a requirement `done` and releasing its lane. |
| **Executor Ledger (`executor.<rN>.<stage>`)** | A self-declared audit trail recording which subagent ID executed each stage (`analysis`, `plan`, `development`) of a requirement. Cross-stage handoffs or same-stage replacements to a different executor ID require an explicit reason (`<new_id>\|<reason>`). Placeholder IDs (`pi-ygg-harness`, `main`, `default`, `unregistered`) are filtered out of resume decisions. |
| **In-Archive VFS (`artifacts/*`)** | A virtual filesystem table (`vfs_files`) inside `run.sqlite` exposed to the Lua sandbox via `ygg.fs.*` and to external tools via `vfs.write` / JSON-RPC `vfs_put`. Stores plans, development reports, evolution reviews, and compaction handoffs with SHA-256 integrity. |
| **Synchronous Tool Bridge (`ygg.tool.call`)** | An opt-in host capability allowing the Mod to invoke allowlisted deterministic host rituals (`git.worktree_claim`, `git.lane_probe`, `git.worktree_release`, `fs.artifact_probe`, `gate.verify_suite`, `harness.archive_sessions`) synchronously within the same dispatch turn, eliminating multi-turn async callback round-trips. |
| **HarnessAction (`subagentAction`)** | A structured machine-readable directive emitted by the Mod inside `summary.jobs[i].subagentAction` (`subagent.spawn` or `subagent.resume`), carrying the rendered `brief`, `deltaPrompt`, `fullFallbackBrief`, `worktreePath`, `artifactPath`, and `submitCommand` so any upper-layer harness adapter can execute lanes without re-implementing workflow logic. |
| **Stable Checkpoint** | A quiescent run state (`available == true` $\land$ `pendingTools == 0` $\land$ (`gates > 0` $\lor$ `jobs == 0` $\lor$ `conflicts > 0`)) where context compaction and structured handoff (`artifacts/handoff-latest.md`) can occur safely without losing mid-flight subagent output. |

---

## 3. Ten Architectural & Operational Principles

1. **P1 · Pure Functional Transition & Transactional Savepoint (`(state, event) -> state'`)**
   - Every dispatch is an all-or-nothing atomic transition. If a Lua handler errors or exhausts its instruction fuel, all mutations to the entity table and KV store during that dispatch are rolled back completely. In SQLite persistence mode, the event log row and state snapshot are committed in a single database transaction.
2. **P2 · Zero Hardcoding — Data & Contract Driven (`D11/D15`)**
   - Business rules, state transitions, narrative templates, and prompt beats belong in Mod scripts (`src/*.lua`) and JSON resources (`data/*.json`), never hardcoded in Rust host code or TypeScript extension code. All prompt templates, subagent briefs, and narrative lines live in `data/scenes.json`.
3. **P3 · Contract-Driven Generic Host**
   - The Rust host (`ygg-agent`) and kernel (`ygg-sandbox`) know nothing about specific Mod event names, stage names, or narrative prefixes. Everything is declared in `pack.json` (`agent`, `inboxRoutes`, `tools`, `code`) and `data/config.json` (`drive`, `settlement`, `allowedTools`). Missing or malformed contract fields fail closed at load time with zero fallback defaults.
4. **P4 · Less is More — One Canonical Path per Mechanism**
   - Every capability has a single canonical mechanism. Redundant validation blocks, duplicate Mod copies, or overlapping multi-turn async ceremonies are consolidated into single-source helpers and 1-turn inline rituals.
5. **P5 · Single Source of Truth**
   - ** schemas**: `packages/schema/schemas/` is the sole authority for JSON Schemas (shared by Rust and TypeScript).
   - **Mod**: `.ygg/mods/pi-agent-seed` is the single physical Mod directory; `packages/mods/pi-agent-seed` is a relative symlink (`../../.ygg/mods/pi-agent-seed`).
   - **Run State**: `<run-dir>/run.sqlite` is the single storage file for new runs.
   - **Documentation**: `docs/CONTEXT.md` and `docs/DESIGN.md` are the only two architectural/design documents under `docs/`.
6. **P6 · Module Top-Level Purity (`YGG-MOD-EFFECT`)**
   - In a multi-file Mod, `mod.lua` is the sole assembly entry point. Supporting modules (`src/*.lua`) must only export pure functions and constants at top level; calling side-effectful host APIs (`ygg.on`, `ygg.emit`, `ygg.narrate`, `ygg.choice`, `ygg.kv.set`, `ygg.entity.*`, `ygg.fsm.define`) at module top level is rejected both statically (`ygg-lint`) and at load time (`ygg-sandbox`). This guarantees handler registration order (and therefore dispatch order) is explicitly controlled by `mod.lua`, never by `require` order.
7. **P7 · Reactive Convergent Effects Before Choice**
   - Once `ygg.choice` is called, the kernel terminates the current transition and drops any remaining queued internal events. Therefore, all reactive side effects (`hooks.useEffect`, such as `auto_dispatch`, `tools.receipt`, and `tools.reactive`) must flush and converge (`MAX_PASSES = 4`) at the top of `render()` *before* `ygg.choice` is emitted.
8. **P8 · Opt-In Capability Degradation (Wasm Purity vs Local Host Power)**
   - The Lua sandbox defaults to strict isolation (no I/O, no clock, deterministic PRNG, instruction fuel cap) so Mods run identically in browser Wasm. Local host capabilities (`ygg.fs` VFS and `ygg.tool.call` synchronous tool runner) are injected via optional host traits; when unmounted, the Mod degrades gracefully without breaking state semantics.
9. **P9 · Persistent Lane Workspaces Preserving Build Caches**
   - Parallel subagent lanes reuse persistent worktree directories (`<repo>-lane-1..4`) across requirements and runs. Claiming a lane resets tracked files to `HEAD` while preserving untracked build directories (`target/`, `node_modules/`), avoiding cold-compile penalties while maintaining branch isolation.
10. **P10 · Data Red Line & Explicit Per-Run Goal Authorization**
    - **Data Red Line**: Reading, listing, stat-ing, or copying real business data (`*.xlsx`, `*.xls`, `*.csv`, or classified mounts such as `/Volumes/WenshuSpace/**`) is strictly prohibited at both the prompt level and the extension `tool_call` firewall.
    - **Decision Ownership**: `guide` mode is the default (milestone actions require a human menu index via `ygg.choice_selected`). Switching to `goal` mode at runtime transfers dispatch authority to the main agent, requires a per-run `config.set goal.authorization=<note>` record before `review.note` can close the run, and warns immediately (`[GOAL AUTH PENDING]`) if entered before authorization is registered.

---

## 4. Monorepo Module Topology & Dependency Whitelist

```text
Yggdrasill/
├── crates/
│   ├── ygg-core/        # Pure deterministic kernel: Entity table, Event bus, Transition driver (Zero internal deps; serde/serde_json/thiserror only)
│   ├── ygg-modules/     # Pure multi-file Lua module path resolution & side-effect ban list (Zero deps leaf crate)
│   ├── ygg-schema/      # Manifest SHA-256 content addressing & JSON Schema validation (Depends only on ygg-core)
│   ├── ygg-store/       # Run persistence port: SQLite default (run.sqlite + VFS) & legacy JSON backend (Dev-dep of ygg-sandbox examples only)
│   ├── ygg-sandbox/     # Deterministic Lua 5.4 VM (mlua), Host API whitelist, ygg stdlib, ModBundle loader, ygg-agent host CLI
│   ├── ygg-lint/        # Static AST linter (full_moon): YGG-R1/R2/R3, YGG-MOD-EFFECT, YGG-PARSE (Depends only on ygg-modules, zero kernel/VM deps)
│   └── ygg-wasm/        # Browser WebAssembly bindings (wasm-bindgen thin shell over ygg-core + ygg-sandbox)
├── packages/
│   ├── schema/          # Single source of truth for JSON Schemas (manifest, pack, story, agent-config)
│   ├── mods/
│   │   ├── pi-agent-seed -> ../../.ygg/mods/pi-agent-seed   # Symlink to canonical workflow Mod
│   │   └── test-adventure/                                  # Reference playable text adventure Mod
│   ├── client-web/      # React + TypeScript browser player
│   └── pipeline/        # TypeScript AI Mod generation & verification loop
├── clients/
│   └── godot/           # Godot 4 native gdext client (M2.5)
├── .ygg/
│   └── mods/pi-agent-seed/   # Canonical multi-file Agent Harness Mod (mod.lua, src/*.lua, data/*.json, schemas/)
├── .pi/
│   └── extensions/ygg-pi-harness/   # Pi Coding Agent Tier-1 extension adapter (firewall, daemon RPC client, orchestrator, compaction)
└── docs/
    ├── CONTEXT.md       # Domain knowledge, vision, principles & architectural decisions ledger (this file)
    └── DESIGN.md        # System architecture, state machine semantics & interface contracts
```

### Hard Dependency Invariants
- `ygg-core` may only depend on `serde`, `serde_json`, and `thiserror`. No VM, no schema validator, no I/O, and no platform APIs may ever enter `ygg-core`.
- `ygg-modules` is a zero-dependency leaf crate shared by `ygg-sandbox` and `ygg-lint`.
- `ygg-lint` must never depend on `ygg-core`, `ygg-schema`, or `ygg-sandbox` (keeping static AST linting fast and free of `mlua` compilation).
- `ygg-store` (and `rusqlite`) is linked only by the native host CLI (`crates/ygg-sandbox/examples/agent.rs` via `[dev-dependencies]`), ensuring `ygg-wasm` and `ygg-lint` dependency trees stay 100% free of SQLite and filesystem dependencies.

---

## 5. Key Architectural Decisions Ledger (`D1–D20` & `M-Agent` Evolution)

> Any new project-wide architectural decision or revision to an existing decision must be recorded in this table in the same commit that implements it.

### 5.1 Core Engine & Platform Decisions (`D1–D20`)

| ID | Decision Area | Ruling & Rationale | Status |
| --- | --- | --- | --- |
| **D1** | Initial Runtime Target | **Web-first**: pure browser playability via `ygg-wasm`, strict separation between presentation and logic; WeChat Mini-Game and Godot as subsequent targets. | Confirmed |
| **D3** | Kernel Specification | **Full RPG-grade rule kernel from Day 1** (entities, attributes, tags, event bus, FSM, conditions in Lua) so text adventures evolve into RPGs by adding content/subsystems rather than rewriting the kernel. | Confirmed |
| **D4** | First Milestone Strategy | **Engine + handwritten reference Mod (`test-adventure`) first**, followed by the AI generation pipeline (`M1`) once kernel semantics are proven. | Confirmed |
| **D6** | Game Engine Integration | **Godot native linking (`gdext`)** scheduled for M2.5 (links Rust library natively without Wasm overhead); presentation/logic separation preserved. | Scheduled (Spike GO) |
| **D7** | Kernel Language | **Single Rust implementation**: compiles to browser Wasm (`ygg-wasm`), links natively into Godot (`gdext`), and runs natively on authoritative servers (`ygg-agent` / Axum). Guarantees deterministic replay without GC jitter. | Confirmed |
| **D8** | Web Presentation Stack | **React + TypeScript** (`packages/client-web`) for text-centric narrative UI and mature AI tooling ecosystem. | Confirmed |
| **D9** | Platform Services (M2+) | **Rust + Axum** sharing `ygg-schema` validation logic directly with the kernel. | Planned |
| **D10** | Content Pipeline Stack | **TypeScript** (`packages/pipeline`) for I/O-bound LLM orchestration, structured output parsing, and retry loops. | Confirmed |
| **D11** | Mod Contract | **Lua script as the sole executable interface**: narrative flow, effects, and conditions are written in Lua; static content lives in JSON resource files loaded via `ygg.data(...)`. Replaces earlier declarative JSON condition/dialogue-tree drafts. | Confirmed |
| **D12** | Crate Boundaries | **`crates/ygg-sandbox`** encapsulates `mlua`, the host API whitelist, and the embedded `ygg` Lua stdlib; `crates/ygg-lint` provides standalone AST static analysis. | Confirmed |
| **D13** | Sandbox Security Boundary | **Zero I/O, zero network, zero clock, instruction fuel cap, memory cap (`16 MiB`), API whitelist (`safe_stdlib`), and deterministic seeded PRNG**. Content review reduces to JSON Schema validation + `ygg-lint` + `playtest`. | Confirmed |
| **D14** | Persistent State Authority | **Persistent state lives exclusively in Rust (`ygg.kv` / `ygg.entity`)**. Lua local/global variables are ephemeral within a single dispatch; `ygg-lint` (`YGG-R2`) flags unpersisted entity handles. | Confirmed |
| **D15** | External Data & Schema Single Source | **Resource JSON files + single-source JSON Schemas in `packages/schema/schemas/`**, validated identically by Rust (`jsonschema`) and TypeScript (`ajv`). | Confirmed |
| **D16** | Lua Dialect | **Standard Lua 5.4** (via vendored `mlua`) with instruction-count hook fuel, hardened stdlib (`os`/`io`/`debug`/`package`/`dofile`/`loadfile` removed, `require` replaced by pure sandbox loader), and deterministic `luai_makeseed`. | Confirmed |
| **D17** | Concurrency & Sharding Model | **Single-instance synchronous FIFO deterministic kernel + host-side sharding**: each `KernelFacade` is strictly single-threaded FIFO; horizontal scale uses isolated `KernelFacade` shards communicating only via canonical tick-boundary message channels. Verified at 10k entities sub-millisecond dispatch (`probe_determinism`). | Confirmed / Reserved |
| **D18** | LLM Provider Abstraction | **Dual-protocol adapter (`openai-chat` default + `anthropic-messages` fallback)** configured strictly via git-ignored `.env` keys (`YGG_LLM_*`). | Confirmed |
| **D19** | Playtest Verification Contract | **Machine-readable 8-key JSON report** from `playtest` (greedy option-1 walk, custom `--choices`, `--max-steps`), classifying outcomes into `ending` / `choice` / `limit` / `deadend` via Mod-declared `agent.endingPrefix`. | Confirmed |
| **D20** | Resource Template Single Source | **`scene-texts.json` and `agent-config.json` schemas live canonically in `packages/schema/schemas/`**; LLMs populate data instances and never invent schema definitions. | Confirmed |

### 5.2 Agent Harness (`M-Agent`) Architectural Decisions

| ID | Decision Area | Ruling & Rationale | Status |
| --- | --- | --- | --- |
| **D-MA1** | Workflow-as-Mod Architecture | Coordinate software engineering tasks through `.ygg/mods/pi-agent-seed` (`intake -> coordinate -> review -> done` + per-requirement `draft -> planning -> plan_gate -> ready -> developing -> accept -> done`). All artifacts require a preceding state-machine event. | Confirmed |
| **D-MA2** | Multi-File Mod Purity (`YGG-MOD-EFFECT`) | `pack.json` `code.{entry, modules[]}` enumerates all Lua modules explicitly. Only `mod.lua` may register handlers or mutate state at top level; `src/*.lua` modules export pure functions (`M.install(deps)`) and carry real `@src/<file>.lua:line` diagnostics. | Confirmed |
| **D-MA3** | Single-File SQLite Run Store & VFS | New runs default to `<run-dir>/run.sqlite` (`ygg-store`), atomically committing `events`, `snapshots`, `inbox`, `kv_projection`, and `vfs_files` (`artifacts/*`). Existing JSON runs (`autosave.json` + `events.jsonl`) remain auto-detected without implicit migration. | Confirmed |
| **D-MA4** | Reactive Hooks & Turn-Cut Compression | `src/hooks.lua` (`useMemo`, `useEffect`, `flush_effects`) runs before `render()` emits `ygg.choice`. Enables 3 turn-cut rules: ① `draft -> planning` stage merge, ② inline `exec=<agent_id>` registration on `proposal.submit`, and ③ reactive `auto_dispatch` after `gate.approve(plan)`. | Confirmed |
| **D-MA5** | Synchronous Tool Bridge (`ygg.tool.call`) | When `HostToolRunner` is mounted by `ygg-agent`, allowlisted tools (`git.worktree_claim`, `git.lane_probe`, `git.worktree_release`, `fs.artifact_probe`, `gate.verify_suite`, `harness.archive_sessions`) execute inline in 1 turn and fold results into KV immediately, while preserving async `tool.call`/`tool.result` fallback for unmounted hosts. | Confirmed |
| **D-MA6** | Persistent Lane Worktree Pool | `worktree-claim` manages a persistent pool `<repo-parent>/worktrees/<repo>-lane-1..4`, resetting branches via `git checkout -B <branch> HEAD` (preserving untracked build caches like `target/`) and parking released lanes on `--detach HEAD` instead of deleting directories. | Confirmed |
| **D-MA7** | Universal Harness Adapter (`subagentAction`) | The Mod computes `summary.jobs[i].subagentAction` (`subagent.spawn` / `subagent.resume`) with rendered briefs (`data/scenes.json` templates) and live `worktreeStatus` / `subagentStatus` / `probeStatus`, allowing `.pi/extensions/ygg-pi-harness` (or Claude Code / Codex / Cursor adapters) to act as a thin execution shell. | Confirmed |
| **D-MA8** | Living Documentation Consolidation | Consolidate all architectural, domain, and contract documentation into `docs/CONTEXT.md` (Domain & Decisions) and `docs/DESIGN.md` (System Behavior & Interface Contracts), enforced by mandatory read references in subagent briefs and sync checks at final review. | Confirmed |
