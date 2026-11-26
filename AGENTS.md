# Yggdrasill · Agent Harness — Hard Constraints and Cold Start

> This file is **cold start only**: what must be in the first context (irreversible damage, hard refusal) plus the
> commands you cannot guess. Process, acts and contracts live in `openwiki/` — route through §3 and read the body; a
> summary or "last batch did this" is a route, never the rule.

## 0. Data red line
Never read real business data (`*.xlsx` / `*.xls` / `*.csv`, or anything under a classified mount) into the
repository or context, and never take metadata readings of its directories (`ls` / `stat` / `find` / `du` / `wc` —
row counts, column names and enumerations are also classified). The user runs real-data verification and pastes the
output; synthetic data under `/tmp` is unrestricted.
## 1. Non-negotiable constraints

**1.1 serve is read-only — never add a run-state write endpoint.** The `/api/roots` exception and its proof live in
`openwiki/contracts/host/serve.md`; read it before touching serve.

**1.2 Scene gate — read the authoritative body before acting in a scene** (prologue / charter / coordinate / settle /
release / retired, or a requirement stage). Re-entering a scene, a rejected-and-replanned requirement and a new batch
each need a **fresh signature**; one older than the scope entry is expired. On-disk `ver` differs from the one you last
read → **body read, then sign**; same `ver` inside `readGate.window` → version check (`was=/now=`) suffices, and a
version check never extends the window. **Unreadable = stop** — a missing page or a body contradicting the index has
no licence to proceed from memory (`openwiki/operating/rituals/read-gate.md`).

**1.3 Every in-run artifact must be carried by an event first** — before a file write, a code change or a deliverable,
an event must bring it into the state machine (`requirement.dispatch` → proposal → gate); the verdict looks at events,
not where bytes landed (`~/tmp` and chat count too).

**1.4 The main agent does only four coordination-layer things**: dispatch work, accept proposals serially, pass the
two mandatory gates, declare batch release; it never implements lane work, and after filing a requirement its first
action is `requirement.dispatch`. **1.5 Render receipts in full, never parse-and-truncate** — no sliced narratives,
first-line reads, `startswith` filters or grep summaries: print every narrative line and hand raw `pendingChoice` to
the user.
**1.6 One dispatch per process, one real action per turn.** The host exposes exactly one `--event`; batching flags,
timeline replay scripts, settlement loops and wrapper entrypoints are forbidden, and `autosave.json` / `events.jsonl`
are never edited by hand. A violation is an event history produced without receipt review.

**1.7 Stop on violation**: stop everything, list what you dispatched, what you did yourself and which output has no
event, then wait for the user. Never back-fill the process first.

**1.8 Credentials live only in `.env` / `.env.local`**: resources, configuration and reports name keys only, never values.

**1.9 Self-evolution has exactly two exits — `AGENTS.md` (cold-start ruling) or the Mod (machine enforcement,
`.ygg/mods/agent-seed/`) — no side scripts, no CLI source changes.** Scripts may only perform read-only queries. Every
evolution needs all three: the action blocked, the violation symptom, one runnable confirmation. New capability lands in
`crates/ygg-*/` (kernel semantics) or `packages/libs/<name>/` (pure Lua over `ygg.*`) with its contract, `selftest`
cases and perf numbers (`openwiki/contracts/packages/README.md`).

**1.10 No claim without its method — absence and counts included.** A negative claim ("never existed") and a quantity
claim ("16 occurrences") are unproven until a method that could have returned the opposite produced them; run it, paste
the output, name the method. An unsupported "does not exist" reports "I did not look there"
(`openwiki/operating/rituals/dispatch.md`).

**1.11 Every prompt-surface edit accounts for the full producer/consumer lifecycle.** One normative rule, one
authoritative carrier; other carriers are pointers or labelled projections. Answer Q1 consumer moment (cold-start /
phase-entry / before-action / subsystem-touch), Q2 cost of missing it (irreversible / hard-refusal / silent-drift /
none), Q3 producer (owner, triggering action, check catching a missed rewrite). Keep a line here only for Q1=cold-start
with Q2∈{irreversible,hard-refusal} (`openwiki/governance.md`).

**1.12 Zero debt.** Every discovered defect, optimisation or worth-building capability has exactly two legal endings —
implement it now, or convert it on the spot into a scheduled requirement with acceptance criteria; "deferred / future
work / 待优化 / 遗留" without a schedule ID is an unfinished deliverable.

## 2. Cold start

### Build and install

```bash
YGGDRASILL_ROOT="${YGGDRASILL_ROOT:-.}"
"$YGGDRASILL_ROOT/scripts/install-egg.sh"  # build + install copy + global symlink (idempotent)
```

On macOS the installed copy must be ad-hoc signed or the kernel kills it on first exec (`exit 137`, no error text):
`codesign --sign - --force .ygg/bin/egg`. Sign **the copy** — a signature on `target/release/examples/egg` does not
survive `cp`. `.ygg/mods/agent-seed` is the single Mod source; `./scripts/sync-seed-mod.sh` recalculates its manifest
(`--check` verifies it plus the package symlink).

### Start

```bash
ROOT="$(.ygg/bin/egg new --mod .ygg/mods/agent-seed --in .ygg/runs --event event.start | tail -1 | sed 's/^run-root: //')"
A=".ygg/bin/egg open $ROOT"
$A --event charter.set --payload '"<title>|<goal>|<success>"'
$A --event run.begin
```

`charter.set` is editable until `run.begin`; project build/check commands are in `openwiki/overview.md` §2 and the
entry ritual in `openwiki/operating/acts/charter.md`.

### Serve (read-only observation)

```bash
.ygg/bin/egg serve  # 127.0.0.1:7800; empty-root start; Ctrl-C stops
curl -X POST http://127.0.0.1:7800/api/roots -H 'Content-Type: application/json' -d '{"root": "<repo>/.ygg/runs"}'
```

### Continue

```bash
A=".ygg/bin/egg open <run-root>"
$A --drive stdin                                     # interactive: human chooses (default)
$A --event config.set --payload '"goal.authorization=<one-line authorization note>"'
$A --event config.set --payload '"play.mode=goal"'   # only after user authorization
$A --event config.set --payload '"tools.root=<checkout>/.pi/tools"'
$A --event config.set --payload '"tools.session=<main-session-id>"'
$A --drive none                                      # observe without advancing
$A --event map.show                                  # operation view + all-chapter index
.ygg/bin/egg open <run-root> --requirement '<title>|<description>'  # async inbox only
```

### Pause

Type `q` or end stdin. `HUMAN[:reason]`, timeout, non-zero exit, invalid output or an out-of-range index pauses safely
without advancing; re-run Continue to resume (`openwiki/contracts/host/lanes.md`).

### Settle and finish

Drain the queue → `coordinate.submit` → `verify.submit` → `settlement.evolution` → release declaration → `review.note`
(settles the batch); `config.set run.retire=true` arms the only ending. Payloads, evidence standards and upgrade
discipline (new run-id per batch boundary; old runs never migrated): `openwiki/operating/acts/settle.md`,
`release.md`, `retired.md`, `openwiki/operating/rituals/cleanup.md`.

## 3. Prompt-surface index

| Moment | Authoritative route |
| --- | --- |
| Entry — which page for this moment | `openwiki/overview.md` (whole tree) + `openwiki/quickstart.md` (time point → page) |
| Contract tree entry / touch surface | `openwiki/architecture/README.md` → `openwiki/contracts/**` |
| Cold-start vocabulary / topology / principles | `openwiki/operating/cold-start/vocabulary.md`, `topology.md`, `principles.md` |
| Charter / coordinate / settle / release / retired | `openwiki/operating/acts/charter.md`, `coordinate.md`, `settle.md`, `release.md`, `retired.md` |
| Requirement stage (draft → done) | `openwiki/operating/req-acts/intake.md`, `delivery.md`, `integration.md` |
| Dispatch / submit / integration ritual | `openwiki/operating/rituals/dispatch.md` |
| Verification / evidence / cleanup / read gate | `openwiki/operating/acts/settle.md`, `openwiki/operating/rituals/cleanup.md`, `openwiki/operating/rituals/read-gate.md` |
| No egg / no mod fallback | `openwiki/operating/rituals/fallback.md` |
| Mod / host / tool touch | `openwiki/contracts/mod/mod-package.md`, `openwiki/contracts/mod/tool-bridge.md` |
| serve / lanes / CLI / identity | `openwiki/contracts/host/serve.md`, `openwiki/contracts/host/lanes.md`, `openwiki/contracts/host/cli-daemon.md`, `openwiki/contracts/mod/identity.md` |
| Kernel semantics / storage surface | `openwiki/contracts/kernel/state-machine.md`, `openwiki/contracts/kernel/events.md`, `openwiki/contracts/kernel/state-surface.md`, `openwiki/contracts/kernel/wire-protocol.md` |
| Game-side mods / runtime faces | `openwiki/contracts/mod/action-faces.md` |
| Capability carrier (kernel vs package) | `openwiki/contracts/packages/README.md` |
| Why a ruling exists / how to add one | the 「设计依据」 section of the page the ruling governs (`openwiki/governance.md`) |
| Repository topology / walk-through | `openwiki/operating/cold-start/topology.md`, `openwiki/operating/story/run-story.md` |

`openwiki/ver.json` is the single ver ledger (`scripts/wiki-ver.sh`); a page carries no `ver` line. Closure and
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

Use the repository's documented maintenance process to refresh generated pages after source changes. Check its configuration and completed runs before relying on automatic updates. Do not hand-edit generated OpenWiki pages unless explicitly asked; prefer updating source code/docs and letting OpenWiki regenerate.

<!-- OPENWIKI:END -->
