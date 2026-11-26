# Yggdrasill · Kernel SDK — Hard Constraints and Cold Start

> This file is **cold start only**: irreversible damage, hard refusal, and the commands you cannot guess. Subsystem
> contracts and operating detail live in `openwiki/` — route through §3 and read the body; a summary is a route, never
> the rule.

## 0. Data red line
Never read real business data (`*.xlsx` / `*.xls` / `*.csv`, or anything under a classified mount) into the
repository or context, and never take metadata readings of its directories (`ls` / `stat` / `find` / `du` / `wc` —
row counts, column names and enumerations are also classified). The user runs real-data verification and pastes the
output; synthetic data under `/tmp` is unrestricted.

## 1. Non-negotiable constraints

**1.1 The kernel is host-free.** No HTTP server, no filesystem, no clock, no subprocess, no rendering, audio or input
in `crates/**`. A host owns its engine, its loop and its I/O; the kernel owns world state and the protocol that drives
it. Test for a boundary violation: *would this code change if the host engine changed?* Then it does not belong here.

**1.2 Determinism is the product.** State carries no `f64` (fixed-point and integers only); ordered containers only
(`BTreeMap`, never `HashMap`/`HashSet` in `src/**`); time enters only through `step(ticks, dt_ms)`; randomness only from
world streams; the tick hash is computed from canonical bytes through **one** composition path — adding a second one is
a violation. Two runs of the same command log must produce identical bytes, hashes and snapshots, native and wasm.

**1.3 One world model.** The columnar ECS in `ygg-sim` is the only thing that decides what exists. Packs are Lua data
and scripts; native systems are registered by the host. A second entity store, a row-shaped shadow of the world, or a
pack reaching around `world.*` is a violation.

**1.4 Rules are sandboxed and budgeted.** Packs get no `os`, `io`, `debug`, `package`, `load*` or `coroutine`, and no
float math; every host→Lua entry runs under an instruction budget, so a runaway script is an error, not a hang; a
failed entry leaves world bytes unchanged (rollback, never half-applied state).

**1.5 Read the authoritative body before touching a subsystem.** Contracts in `openwiki/contracts/**` are the rule for
that subsystem; a receipt, a summary, a previous session's habit or this file is a route. A missing page, a moved file
or a body contradicting the index is a **stop-and-report** — never a licence to proceed from memory. Pages that still
name `egg` / `serve` / `agent-seed` are historical: the kernel SDK rewrite is tracked as its own milestone.

**1.6 Stop on violation.** When a constraint was bypassed, stop everything and list what you changed, what you ran and
which output is unverified; then wait for the user. Never back-fill the record first.

**1.7 Credentials live only in `.env` / `.env.local`**: resources, configuration and reports name keys only, never values.

**1.8 Self-evolution has exactly two exits: this file (cold-start ruling) or the code (machine enforcement)** — no side
scripts, no wrappers, no checkers shipped as patches; scripts may only perform read-only queries. Every evolution needs
all three: the action blocked, the visible violation symptom, one runnable confirmation. New capability lands in
`crates/ygg-*/` (kernel semantics) or `<pack-root>/<name>/` (pure Lua over `world.*`) in the same commit as its
contract, its tests and its perf numbers (`openwiki/contracts/packages/README.md`). The in-repo `packages/libs`
corpus retired in M2.5; a package capability now lands in the pack that consumes it, not in this repo.

**1.9 No claim without its method — absence and counts included.** "Never existed" and "16 occurrences" are unproven
until a method that could have returned the opposite produced them: run it, paste the output, name the method. An
unsupported "does not exist" reports "I did not look there".

**1.10 Every prompt-surface edit accounts for the full producer/consumer lifecycle.** One normative rule, one
authoritative carrier; other carriers are pointers or labelled projections. Answer Q1 consumer moment (cold-start /
subsystem-touch / before-action), Q2 cost of missing it (irreversible / hard-refusal / silent-drift / none), Q3 producer
(owner, triggering change, check catching a missed rewrite). Keep a line here only for Q1=cold-start with
Q2∈{irreversible,hard-refusal}; `scripts/wiki-consistency.sh` enforces this file's 160-line budget
(`openwiki/governance.md`).

**1.11 Zero debt.** Every discovered defect or worth-building capability has exactly two legal endings: implement it
now, or turn it into a scheduled item with acceptance criteria. "Deferred / future work / 待优化 / 遗留" without a
schedule ID is an unfinished deliverable.

## 2. Cold start

```bash
cargo build --workspace
cargo test --workspace          # the executable surface of this repo
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

Wasm target (needs a WASI SDK; `/opt/wasi-sdk` is the config default, this machine keeps it under `~/.local`):

```bash
W="$HOME/.local/wasi-sdk-34.0-arm64-macos"
CC_wasm32_unknown_unknown="$W/bin/clang" CXX_wasm32_unknown_unknown="$W/bin/clang++" \
AR_wasm32_unknown_unknown="$W/bin/llvm-ar" \
CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS="-Ctarget-feature=+reference-types \
 -Lnative=$W/share/wasi-sysroot/lib/wasm32-wasip1/eh \
 -Lnative=$W/share/wasi-sysroot/lib/wasm32-wasip1 \
 -lstatic=c++abi -lstatic=unwind -lstatic=wasi-emulated-process-clocks -lstatic=c" \
cargo build --target wasm32-unknown-unknown -p ygg-wasm
```

## 3. Prompt-surface index

| Moment | Authoritative route |
| --- | --- |
| Entry — whole tree / time point → page | `openwiki/overview.md`, `openwiki/quickstart.md` |
| Contract tree / what a subsystem must not change | `openwiki/architecture/README.md` → `openwiki/contracts/**` |
| World model, hash and replay | `openwiki/contracts/kernel/sim-kernel.md` |
| Events, state surface, compaction | `openwiki/contracts/kernel/events.md`, `state-surface.md`, `compaction.md` |
| Lua facade and protocol | `openwiki/contracts/kernel/lua-api.md`, `openwiki/contracts/kernel/wire-protocol.md` |
| Capability carrier (kernel crate vs Lua package) | `openwiki/contracts/packages/README.md` |
| Why a ruling exists / how to add one | the 「设计依据」 section of the page the ruling governs (`openwiki/governance.md`) |

`openwiki/ver.json` is the single ver ledger (`scripts/wiki-ver.sh`); a page carries no `ver` line. Closure, link and
line-budget checks: `scripts/wiki-consistency.sh`.

<!-- OPENWIKI:START -->

## OpenWiki

This repository has a generated `openwiki/` evidence index. It is optional just-in-time context, not required startup reading.

- Do not enumerate, preload, or search wikis at task start. Use retrieval when the user asks for it, when unfamiliar architecture or dependency behavior materially affects the task, or when source inspection leaves an important uncertainty. Stop once the question is grounded.
- When those conditions apply and OpenWiki retrieval tools are available, use `openwiki_search` for just-in-time context and `openwiki_read` for the relevant complete sections. If search returns `workspace_required`, ask which listed workspace to use and retry with its ID.
- Use `openwiki_list_workspaces` or `openwiki_list_wikis` when workspace membership itself needs to be discovered.
- If the retrieval tools are unavailable, read `openwiki/quickstart.md` and follow its links to the relevant pages.
- Treat source code and tests as authoritative. A brief's unknowns and review items are verification gaps, not automatic requirements.
- Prefer the narrowest quiet validation that proves the changed behavior. Preserve complete failure output.

Use the repository's documented maintenance process to refresh generated pages after source changes. Check its configuration and completed runs before relying on automatic updates. Do not hand-edit generated OpenWiki pages unless explicitly asked; prefer updating source code/docs and let OpenWiki regenerate.
<!-- OPENWIKI:END -->
