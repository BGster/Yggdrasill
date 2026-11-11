# Yggdrasill · Agent Harness — Hard Constraints and Cold Start

> This file carries exactly two things: the constraints you may not violate, and how to start /
> continue / pause / settle a run. The mechanics (state machine / action protocol / async inbox /
> progress views / run artifacts / Mod maintenance / host contract) live in
> `.ygg/mods/pi-agent-seed/README.md` and the `harness` chapter of the playbook; fetch field rules by
> chapter with `guide.show <section_id>`. `CLAUDE.md` is a symlink to this file.

## 1. Non-negotiable constraints

**0. Data red line**: never read real business data (`*.xlsx` / `*.xls` / `*.csv`, or anything under
a classified mount) into the repository or the context, and never take metadata readings of its
directories (`ls` / `stat` / `find` / `du` / `wc` — row counts, column names and enumerations are as
classified as the values). Verifications that need real data are run by the user, who pastes the
output back; synthetic data generated under `/tmp` is unrestricted.

1. **Every in-run artifact must be carried by an event first**: before writing a file, changing code
   or producing a deliverable with a command, an event must bring it into the state machine
   (`requirement.dispatch` → proposal → gate). **The verdict looks at events, not at where the bytes
   landed** — `~/tmp`, `/tmp` and the chat transcript all count as in-run output. "It is small", "it
   was quick", "the user did not ask for the process" are not exemptions.
2. **The main agent does only three things in the coordination layer**: dispatch work, accept
   proposals serially, pass the two mandatory gates. It never implements a lane's work itself. After
   a requirement is filed, the first action is `requirement.dispatch`, not doing the work directly.
   - **Register the executor after dispatching** (self-declared ledger, not identity proof): once you
     have the subagent id, run `config.set executor.<rN>.<stage>=<id>` (stage ∈ analysis / plan /
     development). Replacing the executor for the same (req, stage), or any **cross-stage handoff**
     to a different id than the previous stage, requires a reason (`=<id>|<reason>`). `amend` does
     not change the binding; `split` / `merge` do not inherit it. `proposal.submit` does not
     hard-check the ledger — an unregistered executor only shows as "unregistered" in
     `requirement.list`.
   - **Requirement granularity: merge before you create.** Adjacent work with the same goal, the same
     subsystem, or that is a small change sharing one plan and one acceptance evidence set belongs in
     the same requirement. Create a new one only for: an **independent delivery + acceptance**, a
     **cross-repository or cross-lane** change, or **risk isolation**. A new adjacent requirement must
     state in its description why it cannot be merged (a missing reason means "mergeable").
     Three-question criterion (see 7 below): ① which action does it block? (`requirement.add` /
     `--requirement` filing); ② what does a violation look like? (the executor points out that the
     "new" requirement is just part of the previous one while the description has no
     cannot-merge reason); ③ how do you confirm it next time? (walk `requirement.list` and check that
     every new requirement's description carries that reason).
3. **Render receipts in full; never parse-and-truncate**: no `narrative[0][:N]`, no "first line
   only", no line filtering with `startswith`, no stuffing a receipt into `grep` / `python` to read
   just a summary. Every dispatch prints **all narrative lines** into the conversation and hands the
   raw `pendingChoice` to the user. If you need a rule, fetch the whole chapter with
   `guide.show <section_id>` instead of trusting memory.
4. **Single-shot dispatching is not compliance**: batching `--event` flags only hides state from the
   script guard; it does not replace the `requirement.dispatch` → proposal → gate chain. Never edit
   `autosave.json` / `events.jsonl` by hand. **Dispatching directly under `goal` mode is a legal
   path** (see 8), but its output still needs an event chain; `--drive cmd` may never be used to
   disguise a decision (it is a host input channel, not a decision maker).
   - **Permanent prohibition of batch event entrypoints**: the host exposes exactly one dispatch per
     process (`--event`). There is and will be no `--events`, no batch script that replays a
     timeline, no loop that settles a run, and no wrapper that "just" feeds several actions. Batch
     entrypoints turn the state machine into a transcript of a decision that was never made; the
     discipline is **immersive play first** — one real action per turn, read the full receipt, then
     decide.
     Three-question criterion (see 7): ① which action does it block? (any argument that a batch
     entrypoint should be added to `crates/ygg-sandbox/examples/egg.rs` or wrapped by a script);
     ② what does a violation look like? (a run whose events were produced without a receipt review,
     or a document that suggests `--events` / a batch loop); ③ how do you confirm it next time?
     (`grep -n '\-\-events' AGENTS.md README.md .ygg/mods/pi-agent-seed/README.md` finds nothing outside
     a prohibition sentence, and `cargo build --release --example egg` still has a single
     `--event`).
5. **Stop on violation**: when a self-check or a report tells you a constraint was bypassed, stop
   every step, list honestly "what was dispatched / what I did myself / which output has no event",
   and wait for the user. **Do not first back-fill the process and then report** — back-filling
   launders a violation into a compliant-looking record.
6. **Credentials live only in `.env` / `.env.local`**: Mod resources, configuration and reports
   mention key names only, never values.
7. **Self-evolution has exactly two exits: `AGENTS.md` or the Mod — no side scripts, no CLI source
   changes**:
   - When a discipline was not followed and you want to "evolve yourself", the only legal landing
     spots are this file (written as a decidable criterion) and `.ygg/mods/pi-agent-seed/` (gate logic
     in `src/flow.lua` and `src/handlers/*.lua`, prompts and scripts in `data/*.json`;
     `mod.lua` is assembly only and carries no business logic). Both may change together, but
     **no third carrier may be added**.
   - **Never write a checker script / wrapper / hook as a patch**: no "check-*.py", no
     "pre-*-hook" that "helps you keep the discipline" — that only outsources discipline to a
     self-written checker that you will bypass in the same way. Scripts may only perform **read-only
     queries** (`requirement.list`, `map.show`, read-only statistics over `events.jsonl`) and **may
     not become the carrier of a constraint**.
   - **Never patch the egg CLI source** (`crates/ygg-sandbox/examples/egg.rs` and the host
     contract). The CLI is a non-negotiable host, not an implementation surface of this repository.
     If a capability gap genuinely must be solved in the host, describe the symptom and the
     reproduction and hand it to the user; never patch it yourself.
   - **Three-question criterion, all three required**: ① which **specific action** does it block?
     ② what is the **visible symptom** of a violation (refusal text / unchanged state / rejected
     payload)? ③ how do you **confirm it works** next time (one runnable command or one probe)?
     Fewer than three answers means it is not written yet; do not claim the evolution is done.
   - **A verbal promise is not an evolution**: "I will be careful" / "next time I will read it all"
     is not a conclusion. Anything that did not land in the two exits above counts as unhandled.
   - **Screen before writing**: before suggesting an item for the backlog, run it through the Mod's
     `evolution` chapter — long-term priority (would it appear again with another task or another
     repository?); an existing unused rule must have its root cause named (call chain /
     discoverability / contract mismatch / deliberately unused / genuinely no scenario / redundant,
     pick exactly one); review every item independently (the artifact must contain `file:line` plus
     a runnable command with its real output plus the semantic basis plus the root-cause class and
     conclusion); scenario before technique (actual scenario → which process step is unsatisfied →
     code and reproduction → root cause → long-term benefit → change surface and verification).
     Single-task constraints, temporary tools and probe conventions, and anything already solved in
     that same task do not enter the backlog.
   - **Declaring is not proving**: `settlement.evolution` only records a declaration (the Mod
     verifies that `summary` is non-empty, the path shape, and that the counts and classes are
     self-consistent). The sandbox cannot read the artifact and does not know the subagent session,
     so it may never be used to claim that the suggestion quality or the review independence was
     proven.

### 8. `goal` / `guide`: who holds the decision

- **Only the Mod interprets `play.mode`**: `settings.play.mode` (runtime override) wins over the
  static default in `data/config.json`. The host CLI neither reads nor interprets it.
- **`guide`** (default): milestone actions (`proposal.accept` / `gate.approve` / `coordinate.submit` /
  `verify.submit` / `review.note`) accept only a menu ticket — they must arrive through
  `ygg.choice_selected` (replying with the index); a direct single-shot dispatch is always
  **refused**. Rejects (`*.reject`) are outside that gate: they produce no passing effect and are the
  only correction channel.
- **`goal`**: the main agent reads the full receipt and **dispatches by itself** (tickets do not
  participate in the verdict). It is not "automatically pick option 1", and it never goes through an
  external script; `pendingChoice` is still printed in full and the evidence standard is unchanged.
- **Switching mode hands over the decision**: after the user authorizes it for that occasion, the
  main agent may run `--event config.set --payload '"play.mode=goal"'` itself. The Mod announces
  `handover_goal` (delegated scope + unchanged evidence standard + must be accounted for in the
  closing review) or `handover_guide` (decision back to the human). The menu only offers the edge to
  the other mode.
- **`goal` authorization is a per-run record**: `config.set goal.authorization=<one-line note>`
  (trimmed, ≤ 200, no newline; an empty value clears it explicitly). Only runs that entered `goal`
  **at runtime** (`settings.play.mode=goal`) are checked before `review.note` finishes: a missing
  record blocks with a narrative line, writes no `ending`, and does not advance. A static `goal`
  preset in `data/config.json` is the repository owner's standing configuration and is **out of
  scope**; switching back to `guide` is not checked and `resume` keeps the record. The record only
  proves "one sentence was registered", not that the user said it — never claim it as identity proof.
- `--drive stdin|cmd:<path>|none` is the **host** input channel (human choice / external command /
  observe only) and is orthogonal to `play.mode`: `goal` does not depend on it, and deleting an
  external decision command does not break `goal`.

### 9. Acceptance and settlement boundaries (enforced by the state machine, not by discipline)

- **Requirement level**: `proposal.accept` only advances by stage — `analysis` → `planning`,
  `plan` → **`plan_gate`**, `development` → **`accept`** (the executor delivers, and the lane is
  released at the same time). `gate.approve` passes `plan_gate` → `ready` and `accept` → `done`;
  `gate.reject` always stores `planning` and the display name is derived from `plan_ok`.
- **Two mandatory gates**: `gate.approve` at `plan_gate` → `ready`, at `accept` → `done`;
  `gate.reject` stores `planning` in both cases and the display name comes from `plan_ok` — a `plan`
  reject sets it false (→ a new plan is required), an `accept` reject keeps the previous value (a
  not-amended requirement is `ready` and re-enters the development queue; an amended one already had
  `plan_ok` invalidated → it stops in `planning` and must be replanned). Before approving, two hard
  checks apply: `gate_rev == req.rev` (amended while pending → stale, an old approval must not take
  effect) and, for `accept`, non-empty `artifacts` (a summary-only claim is not accepted).
- **Preconditions for closing** (`coordinate.submit` / `verify.submit=true` / `review.note` share
  them): zero pending proposals ∧ zero running jobs ∧ zero unresolved conflicts ∧ every requirement
  in a terminal state (`done`, or split / merged / dropped whose successors are all `done`).
- `coordinate.submit` only works in the `coordinate` stage; on success it moves to `review`, sets
  `verify.passed=false`, and clears this round's
  `settlement.evolution.{artifact,summary,candidates,classes}` (settings pre-registrations survive).
  `verify.submit` records all-green in `review` (`all_green=false` sends the run back to `coordinate`
  when `gates.require_all_green=true`, and also clears the settlement keys).
  `settlement.evolution` is registered only after verify is green; its payload is
  `artifact|summary[|c=<kept>/<screened>[|cls=<kind>:<n>,...]][|sessions=<moved>/<remaining>/<destination>][|todo=<remaining>/<verified_by>]`.
  `summary` is required after trimming; `c=0/0` allows an empty artifact; with `screened > 0` the
  artifact and `cls` are required and the counts must be self-consistent. The `sessions=` / `todo=`
  segments settle the two end-of-run cleanups in the same transaction (one turn instead of three);
  the standalone `settlement.sessions` / `settlement.todo` actions remain available for
  step-by-step settlement. `config.set review.evolution.artifact|summary|candidates|classes=<value>`
  is the guide pre-registration surface (shape checked per key; cross-field completeness is checked
  only at registration). `review.note` accepts only `lesson|score` (the legacy ev/c/cls segments are
  refused); it first runs `gates.settlement_problem()`, then archives this round's settlement keys
  single-source into `review.evolution.*`, writes `review.lesson` / `review.score` / `ending` and
  enters `done` (the **[ENDING** line). **Runtime `goal` runs** additionally require a non-empty
  `settings.goal.authorization` (missing → narrative block; a static default is not checked, see §8).
- The host decides "the next step closes the run" from `agent.summary`'s `nextEndsRun`: in `review`,
  the first item is `settlement.evolution` (after verify passed and nothing is registered) and
  `review.note` afterwards; with no pre-registration the default is an empty artifact + a neutral
  summary + `c=0/0`, so the all-1 default path can run "register zero candidates → review.note".
  Only when the first item is `review.note` is `nextEndsRun=true`; the host then switches to
  `closing` (blocks new writes, drains one last time) before dispatching.

#### 9.1 Turn-cut rules (each one is decidable, and each one saves a turn)

- **Inline executor ledger**: `proposal.submit` takes an optional trailing segment
  `exec=<agent_id>[:<reason>]`. It registers the executor ledger through the same
  `src/executor.lua` implementation as `config.set executor.*`, in the same transaction, before the
  proposal is stored; a failed registration refuses the payload **and creates no proposal**. `exec=`
  is only recognised as the **last** segment — anywhere else is refused
  (`req_prop_exec_misplaced`), because the host pads payloads shorter than the declared 7 fields and
  would silently move it into the `artifacts` slot. Write the reason with `:` inside that segment
  (`exec=agent2:handoff`), since the outer payload splits on `|`.
- **Stage merge (`draft -> planning`)**: `requirement.dispatch` on a `draft` row dispatches **plan**
  and stores `planning` in the same dispatch; there is no analysis turn in the default pipeline.
  `analyzing` survives only as a compatibility state (legacy saves, and an explicit `analysis`
  proposal accepted on a `planning` row), so `R_ANALYZING` and its readers stay — no migration code.
- **Declared single-successor advancement, reactive and effect-without-render**: `src/hooks.lua`
  provides `useMemo(key, compute, deps)` / `useEffect(name, deps_fn, effect_fn)` /
  `flush_effects()`. `render()` calls `hooks.flush_effects()` at its top under a reentrancy guard,
  **before** it emits the single `ygg.choice`; every `useEffect` therefore converges first. An effect
  may advance state and narrate, and **must never call `render()` or `ygg.choice`** — once a choice
  exists the kernel drops the rest of the queue (`transition.rs`), which is exactly why the old
  "emit at the end of a handler" pattern was wrong. An automatic successor is allowed only when all
  four hold: ① the `(state, event)` successor is unique and declared in one place; ② it needs no
  external evidence (no file read, no human judgement, no session); ③ it is not a pass-type action
  protected by the human-action gate (gates and acceptance are **never** automatic); ④ the effect
  neither renders nor emits. Two effects exist today. The first is `auto_dispatch` in
  `src/handlers/actions.lua`: a handler marks the requirement it just made ready
  (`hooks.mark_auto_dispatch(index)` after `proposal.accept(analysis)` or `gate.approve(plan)`), and
  the effect dispatches **that requirement only** when the turn has no pending proposal, no pending
  gate, no conflict and a free job slot. `requirement.add`'s `draft` row is never auto-dispatched.
  `gate.reject` / `proposal.reject` derive nothing, and `gate.approve(accept) -> done` derives nothing
  (the freed slot is the main agent's priority decision). Do not add a `derive.auto` switch: it would
  become a second bypass.
- `useMemo` dependencies must be scalars derived from KV state (status, job, pending, readiness,
  counts) — never an implicit Lua-memory diff — so a save/load or a process restart recomputes from
  the same facts and stays deterministic. `flush_effects()` runs at most `MAX_PASSES = 4` passes and
  **errors out** instead of looping forever; a non-converging effect is a defect, not a warning.
- **Receipt effect (the second class)**: `hooks.useEffect("tools.receipt", …)` in `src/tools.lua`
  only latches `tool.notified.<call_id>` and narrates one line once a `tool.result` has landed. It
  touches no `req.*` / `prop.*` / gate / lane / stage state, never renders and never chooses — the
  same ① ② ③ ④ rules as `auto_dispatch` apply. Its dependencies are KV-derived scalars and the
  latch makes it idempotent, so a replay converges in two passes and a second `[TOOL SETTLED]` can
  never appear.

#### 9.2 Behavioural acceptance (mandatory when touching Mod source or the scenes map)

Changing `.ygg/mods/pi-agent-seed/src/**` (state machine, gates, render layer, tool bridge),
`.ygg/mods/pi-agent-seed/mod.lua` or the map wording in `data/scenes.json` requires, besides lint and
the all-1 playtest:
`cargo test -p ygg-sandbox --test agent_seed_semantics` (24 cases: stage merge + `analysis` compat,
accept-gate reject, accept-gate approval releasing the lane, accept-gate amend → stale, plan-gate
reject, reactive auto-dispatch in the same turn (plus `useMemo` invalidation), development proposal
reject (not amended / amended), inline `exec=` (misplaced / no reason / `:` form), settlement merge
with `sessions=` + `todo=` and a refused malformed segment, legacy save without `plan_ok`, the
operation/design map contracts on both packs, the tool bridge (request snapshot + KV rows, result
plus same-turn receipt, replay idempotence, payload/allowlist shape checks, declaration parity
across both packs), and the active subsystem (`git.worktree_claim` binding the lane ledger +
`git.worktree_release` freeing it, `git.lane_probe` recording the truth + the conflict radar
blocking through `req.<i>.conflict` until `requirement.resolve`, `fs.artifact_probe` + `vfs.write`
sharing one bounded ledger, `gate.verify_suite` green evidence only in `review`, `harness.archive_sessions`
unblocking the settlement, declaration completeness). The all-1 playtest only covers the happy path (it
never takes `gate.reject` / `requirement.amend` / `proposal.reject`, and it cannot see the turn-cut
mechanics at all); temporary probes under `/tmp` may supplement it but never replace it, and any
scenario not covered must be declared case by case. Three-question criterion (see 7): ① which action
does it block? ("change the Mod source or the map wording, run lint plus all-1, and call it
accepted"); ② what does a violation look like? (branch semantics and map wording silently regress
while every gate stays green; the defect only shows in a real run); ③ how do you confirm it next
time? (that command, 24/24, plus a mutation probe — put the old semantics back and the test must go
red — to prove the assertions are not vacuous).

The tool bridge is a **contract**, not an internal detail: the `tool.call` / `tool.result` payload
shapes and the `meta.tool_calls` encoding (a dense array of `{id, tool, args?}`, with `args`
**omitted** when empty because a Lua empty table can only serialize to `[]`) are consumed by the
host, so changing them requires updating `pack.json` `inboxRoutes.tool_result.fields` and the host
parser in the same batch. A one-sided change is a silent integration defect, not a refactor.

#### 9.2b The tool bridge keeps two whitelists (and the Mod only sees one)

A tool request crosses two independent gates, and neither may be dropped as "redundant":

| layer | lives in | when it is enforced | what it decides |
| --- | --- | --- | --- |
| Layer 1 | `pack.json` → `tools` | at load, by the host | the **shape and the capability**: which program may ever be spawned, with which fixed arguments, timeout and output cap |
| Layer 2 | `data/config.json` → `allowedTools` | at request time, by the Mod | the **release list for this run**: which declared tool the Mod may ask for |

- `allowedTools` must be a **superset** of the `tools` keys (the host refuses to load otherwise), and
  a missing section allows **nothing** (fail-closed: a copied Mod cannot suddenly run `git`).
- The Mod **cannot** read `pack.json`: `ygg.data` only resolves the resources declared in `pack.json`
  `entries`, so the Mod enforces Layer 2 alone. Cross-layer consistency is therefore pinned by the
  `agent_seed_semantics` declaration-parity case (both packs, deep-equal `tools`, set-equal
  `allowedTools`, every `tool_*` beat present), never by hoping the two files agree.
- Honest statement: Layer 2 is a **drift gate**, not a security boundary. The Mod runs in the same
  sandbox as its own state machine; an actor who can edit `data/config.json` can also edit
  `pack.json`. The real boundary is the host's Layer 1 validation (program allowlist, no shell, no
  absolute path, no `..`) plus the sandbox having no IO at all.
- Credentials are never part of either layer: `$ENV{VAR}` placeholders resolve host-side from the
  process environment, `<mod_dir>/.env.local` and `<mod_dir>/.env`, and the resolved value never
  enters a payload, a receipt or a notice (error text names the variable only).

#### 9.3 Three-suite gate (mandatory when touching the kernel or the host CLI)

Changing `crates/**` (state machine, driver, host persistence surface) requires all three, and
missing one is a fake green:
`cargo test --workspace` ∧ `cargo test --example egg -p ygg-sandbox` ∧
`cargo test -p ygg-sandbox --test agent_seed_semantics`.
The second one cannot be skipped: Cargo's `--workspace` **does not compile the example target's unit
tests**, so the assertions inside `crates/ygg-sandbox/examples/egg.rs` (persistence atomicity,
reset semantics, hot reload, settlement policy) **never execute** when you only run the workspace,
and the gate still prints `failed=0`.
Three-question criterion (see 7): ① which action does it block? ("run only `cargo test --workspace`
after touching the kernel or the host and call the gate green"); ② what does a violation look like?
(the pass counts of `--workspace` and of `--example egg` do not match — the former always lacks the
example batch); ③ how do you confirm it next time? (compare the two counts:
`cargo test --workspace | grep -c '^test result'` and
`cargo test --example egg -p ygg-sandbox | grep -c '^test result'`).

#### 9.2c The active subsystem folds tool results, it never fakes a decision

A settled `tool.result` (`status ok`, `code 0`) is folded into domain state by a **second** effect,
`hooks.useEffect("tools.reactive")`, registered right after `tools.receipt`. The two share one
`flush_effects` (so the fold happens in the same turn, before the single choice is produced) but not
one scope: `receipt_effect` says "the result arrived" and may only latch `tool.notified.<call_id>` +
narrate one line, while the reactors own the domain keys. Three boundaries are non-negotiable and
pinned by `data/playbook.json` (`tools.reactive-effect-scope`, `subsystems.probe-is-warning`,
`subsystems.machine-evidence-parity`) plus the `agent_seed_semantics` subsystem cases:

1. **A reactor writes KV and narrates — nothing else.** No `render()`, no `ygg.choice`, no stage
   change, no proposal / gate state, and never `flow.refuse`: a rejection line carries a
   `rejectionPrefixes` marker and the host counts it as a **failed step**, so a refused probe would
   look like a Mod refusal instead of the fact it is. `vfs.write` is a real event handler (so
   `reject_payload` is legal there) precisely because it is not a reactor.
2. **A failed probe is a warning, never a gate.** `[LANE PROBE FAILED]` (`commits=0` or `clean=0`)
   only narrates and leaves `lane.<k>.probe.*` queryable; wiring it into `gates.gate_problem()`
   would let one dirty tree deadlock `coordinate.submit` / `review.note` forever (there is no
   "undo the probe" action). The conflict radar is the exception: it rides `req.<i>.conflict` via
   `props.req_do_conflict` (the same function as the human `requirement.conflict`), so it enters
   `req_conflict_total` and blocks dispatch/closing with no extra wiring, and only
   `requirement.resolve` clears it.
3. **Machine evidence is same-level, not stronger.** `gate.verify_suite` / `harness.archive_sessions`
   write the very KV that `verify.submit` / `settlement.sessions` write, saving one turn; the
   provenance keys `verify.source` / `settlement.sessions.source` must survive, and no report may
   claim the machine path outranks a self-declaration. Both only record `passed=1` / `remaining=0`
   and only in `review`, on the same precondition as the human path (`gate_problem() == nil`).

Set-of-files comparisons must respect the host cap: `parse_result` returns `truncated` and writes
`tool.<call_id>.truncated`, so the radar skips a truncated probe (a cut list can only cause a false
negative, and a false positive would fabricate a conflict). `vfs.write` never claims a disk write it
could not perform: without `ygg.fs` it records the same ledger row and narrates `[VFS LEDGER ONLY]`,
so the KV and the narration agree on both host shapes.

## 2. Cold start

### Build and install
The CLI has a single source: `crates/ygg-sandbox/examples/egg.rs` (the host itself,
contract-driven and fail-closed):

```bash
YGGDRASILL_ROOT="${YGGDRASILL_ROOT:-.}"                  # repository root (or any checkout)
(cd "$YGGDRASILL_ROOT" && cargo build --release --example egg)
mkdir -p .ygg/bin && rm -f .ygg/bin/egg && cp "$YGGDRASILL_ROOT/target/release/examples/egg" .ygg/bin/egg
```

The default build enables `store-sqlite` (`crates/ygg-sandbox/Cargo.toml` sets
`default = ["store-sqlite"]`), so new runs default to the SQLite backend (`<run-dir>/run.sqlite`).
Existing legacy JSON runs (directories carrying `autosave.json` / `events.jsonl`) are automatically
detected and stay on the JSON backend without migration; `--store json` remains available as an
explicit fallback for new runs, and `run.json`'s `"store"` field is authoritative once written
(`--store` is an opening assertion only; a mismatch fails closed with zero writes).

When the host and the Mod version do not match, loading fails immediately (the error names the
missing field and the shape it expected) and never degrades silently; `.ygg/bin/*` and `.ygg/runs/*`
are git-ignored, `.ygg/mods/` is committed.

### Persistent lane worktree pool, single-source Mod symlink, and living documentation closed loop

- **Persistent lane worktree pool (`$HOME/tmp/worktrees/<repo>-lane-<k>`)**: `worktree-claim`
  defaults to `$HOME/tmp/worktrees` (overridable via `--root` or `$YGG_WORKTREE_ROOT`). When
  `<lane_dir>` already exists (`reused=1`), it is reused in-place (`git checkout --force -B <branch>
  <head>`, `git reset --hard <head>`, `git clean -fd` **without `-x`**); `worktree-release` parks the
  worktree (`parked=1`, `git checkout --force --detach HEAD` + `git clean -fd` **without `-x`**) so
  `<branch>` is unlocked for main-checkout merge/delete while gitignored caches (`target/`,
  `node_modules/`) stay warm across requirements and runs.
- **Single-source Mod symlink (`packages/mods/pi-agent-seed` -> `../../.ygg/mods/pi-agent-seed`)**:
  `.ygg/mods/pi-agent-seed` is the single source of truth; `./scripts/sync-seed-mod.sh` recalculates
  its `manifest.json` and verifies the relative symlink (`./scripts/sync-seed-mod.sh --check`).
- **`docs/CONTEXT.md` + `docs/DESIGN.md` closed loop**: every `plan` and `development` subagent brief
  mandates reading `docs/CONTEXT.md` (domain invariants and vocabulary) and `docs/DESIGN.md`
  (three-layer architecture, lane lifecycle, and `harnessActions[]` contracts) before proposing or
  editing; during `review` settlement (`settlement.evolution`), both documents must be checked and
  updated whenever domain concepts or architectural contracts changed.

### Start
```bash
.ygg/bin/egg --mod .ygg/mods/pi-agent-seed --run-id <run-name> --runs-root .ygg/runs --event event.start
```

### Continue
```bash
A=".ygg/bin/egg --mod .ygg/mods/pi-agent-seed --run-id <run-name> --runs-root .ygg/runs"
$A --resume --drive stdin                                     # interactive: the human chooses (default)
$A --resume --event config.set --payload '"goal.authorization=the user authorized goal mode for this run"'   # register the run-level authorization first
$A --resume --event config.set --payload '"play.mode=goal"'   # take the decision yourself: read the full receipt, then dispatch (switch back any time)
$A --resume --drive none                                      # look at the situation, do not advance
$A --resume --event guide.show --payload '"harness"'          # fetch a rule chapter by id (harness / credentials / gates …)
$A --resume --event map.show                                  # current node + out-edges; add --payload '"full"' for the whole design map
.ygg/bin/egg --run-id <run-name> --runs-root .ygg/runs --requirement '<title>|<description>'   # append a requirement/proposal asynchronously (inbox only, safe while a run is live)
```

### Pause
- In interactive mode type `q`, or end stdin.
- External driving (`--drive cmd:<path>`) returning `HUMAN[:reason]`, a timeout, a non-zero exit, an
  invalid output or an out-of-range index all pause safely: the state does not advance and the chooser
  never falls back to option 1. **`goal` does not depend on that channel.**
- Re-running the "continue" command resumes; nothing is lost.

### Settle and finish
Finish with `.ygg/bin/egg --run-id <run-name> --runs-root .ygg/runs --settle` in this fixed
order (no step may be skipped):

1. Make sure every requirement is in a terminal state (done / split / merged / dropped) and the inbox
   has no unconsumed line, and record the all-green evidence through `verify.submit` (the closing
   preconditions are in §1.9).
2. `coordinate.submit` → `review`; once verify is green, register `settlement.evolution` (screen the
   candidates against the `evolution` chapter first), then close with `review.note`. The registration
   payload is `artifact|summary[|c=<kept>/<screened>[|cls=<kind>:<n>,...]]` with the optional
   `sessions=` / `todo=` segments described in §1.9; under `guide` you may pre-register with
   `config.set review.evolution.artifact|summary|candidates|classes=<value>`, and `review.note` only
   takes `lesson|score` and archives the registration record single-source.
3. Run `--settle` and verify `ok` in `settlement.json` (four switches: events non-empty / no failed
   step / an ending line exists / the inbox was consumed).
4. **Clear the main session's todo list** (`todo clear`, including completed and historical items)
   before ending this run; never carry a finished run's tasks into the next one.

## 3. Upper layer: the pi extension `.pi/extensions/ygg-pi-harness`

The CLI above is the host contract; typing its `--event` sequences by hand is not the intended
interface. `.pi/extensions/ygg-pi-harness` is the **upper level**: it never replaces the state
machine, it only speaks to it through `daemon.dispatch` → `ygg.on(...)` handlers.

| File | Job |
| --- | --- |
| `firewall.ts` | `tool_call` gate: the data red line (`*.xlsx` / `*.xls` / `*.csv`, `/Volumes/WenshuSpace/**`) and the batch-entrypoint prohibition (`--events`, shell `for/while … do … egg`, `xargs egg`). Pure [`checkToolCallFirewall`](.pi/extensions/ygg-pi-harness/firewall.ts), directly unit-tested. |
| `daemon.ts` | Persistent `egg --daemon` client: line JSON-RPC, id matching, serialized request queue, 30s timeout, respawn after an unexpected exit, and a **single-shot CLI fallback** when the host predates `--daemon` (the summary then reports `available=false` instead of guessing jobs/gates). |
| `orchestrator.ts` | Lane lifecycle for active jobs: [`buildSubagentBrief`](.pi/extensions/ygg-pi-harness/orchestrator.ts) builds a self-contained brief; [`orchestrateActiveJobs`](.pi/extensions/ygg-pi-harness/orchestrator.ts) claims the worktree, runs subagents concurrently, archives into the run VFS, probes the lane, submits and accepts the proposal — landing the requirement at `plan_gate` / `accept_gate`. It also owns the **subagent create + resume** lifecycle (real `agent_id` written into `exec=`, `resume` on a gate reject / `plan → development`, one self-heal on a stream error and one on a dirty lane probe) and the **context brake** (hard = spawn nothing and hand off first, soft = land the wave, then hand off). |
| `compaction.ts` | Context pressure monitor + stable-node predicate + structured handoff. [`evaluateContextPressure`](.pi/extensions/ygg-pi-harness/compaction.ts) (soft 0.60 / hard 0.75; unknown token counts never brake), [`isStableCheckpoint`](.pi/extensions/ygg-pi-harness/compaction.ts) (`available` ∧ no pending tools ∧ (gate ∨ no jobs ∨ conflict)), [`buildHandoffMarkdown`](.pi/extensions/ygg-pi-harness/compaction.ts) → `artifacts/handoff-latest.md` through [`writeHandoffToVfs`](.pi/extensions/ygg-pi-harness/compaction.ts), plus the resume-message and compact-instruction builders. No npm deps and erasable TypeScript only, so `node --experimental-strip-types` loads it directly. |
| `index.ts` | Wiring: session lifecycle hooks, the `tool_call` firewall, `/ygg status` and `/ygg stop`, the tools `ygg_session` (`kickoff` / `dispatch` / gate / `vfs_*` / `handoff_compact` / `settle_close`) and `ygg_run_lanes`, the compaction hooks (`session_before_compact` / `session_compact` / `session_compact_failed`), and the footer status line. |

What the extension deliberately does **not** do:

- **It never approves a gate.** `ygg_run_lanes` stops at `plan_gate` / `accept_gate`; approving is the
  one thing only the coordinator does. An auto-approved gate is the same as no gate.
- **It never advances past a conflict** (`summary.conflicts` non-empty) and never retries a refused
  receipt as if it had passed: rejections are surfaced verbatim.
- **It never truncates a receipt.** Every `narrative` line reaches the conversation, and
  `pendingChoice` is handed over raw.
- **It never compacts mid-flight.** `maybeHandoffAndCompact` requires a stable node
  (`isStableCheckpoint`) unless the caller forces it (`handoff_compact`, or a brake the orchestrator
  already took); compacting while a wave is in the air would fold that wave into prose instead of
  landing it on a gate.
- **`dispatch` never compacts.** Its contract is "one dispatch = one turn = one complete receipt", and
  `ctx.compact()` aborts the turn, so the receipt would not reach the model. The `dispatch` branch
  runs `maybeHandoffAndCompact` in `reportOnly` mode (watermark and stable-node facts only) and points
  at `handoff_compact`. Compaction has exactly two explicit entries: `action=handoff_compact` (forced)
  and the `ygg_run_lanes` orchestration brake. See `docs/DESIGN.md` §3.4.
- **It never resumes a guessed id.** `extractAgentId` returns `undefined` rather than synthesising
  one, and `runSubagent` only retries when it knows who to resume — a fabricated id would silently
  become a fresh agent with no memory of the rejected proposal.

### 3.1 Context pressure, checkpoint handoff and auto-resume

| Piece | Rule |
| --- | --- |
| Watermarks | `ratio >= 0.60` → `soft_brake`, `>= 0.75` → `hard_brake` (both inclusive, both override via `evaluateContextPressure` args). Unknown usage (`null` / `NaN` / non-positive) → `normal`: “I don't know” is not grounds for a destructive action. |
| Stable node | `available` ∧ `pendingTools === 0` ∧ (a pending gate ∨ no running job ∨ an unresolved conflict). Running jobs with no gate and no conflict is *no* stable node — that is exactly the half-way point where compaction loses subagent output. |
| Handoff artifact | `artifacts/handoff-latest.md`, rewritten at every checkpoint: run id, stage, `used / window (xx.x%)`, timestamp, jobs table (including the `executor` / `planExecutor` ledger columns), pending gates, unresolved conflicts, VFS artifacts, and the exact `ygg_session` / `ygg_run_lanes` call to run next. |
| Self-triggered compaction | `ctx.compact({ customInstructions })` with the state block marked “keep verbatim”; `onComplete` sends a `ygg-handoff-resume` custom message with `triggerTurn: true`. The tool call that triggered it is usually aborted (compact aborts the turn), so that message has to carry the whole position. |
| External compaction (`/compact`, threshold, overflow) | `session_before_compact` writes the artifact first (survives even if the summary is bad), and `session_compact` re-injects the same state block with `triggerTurn: false` — an external `/compact` must not spontaneously start a new turn. |
| Subagent lifecycle | `resume = job.executor` (gate reject → same stage, `rev+1`); `resume = job.planExecutor` only when `resumePlanAgent` is on and the stage is `development`; retry once on `ok === false` (stream drop) and once when the lane probe reports `commits=0` / `clean=0` (the subagent forgot to commit; the nudge prompt is `COMMIT_NUDGE_TEXT` and the probe is re-run afterwards). |
| Ledger honesty | `proposal.submit`'s `exec=` segment carries the returned `agent_id` (falling back to `opts.actorId`), so the next re-dispatch has somebody to resume. An unreadable probe verdict never triggers a retry. |

Verification (no external dependencies, so it runs in any checkout):

```bash
node --experimental-strip-types --test .pi/extensions/ygg-pi-harness/harness.test.ts
```

The suite covers both sides of every firewall rule (blocked **and** allowed), brief self-containment
(workspace boundary / requirement text kept line by line / verification gates / return format), the
whole orchestration chain (claim before dispatch, concurrency, `vfs_put`, probe, proposal payload
positions, landing at the right gate), and the daemon line protocol (id matching, serialization,
fallback, and `.env` exposing only `YGG_SUBAGENT_MODEL` / `YGG_SUBAGENT_THINKING`).

It also covers the checkpoint path: the watermarks (normal / soft at exactly 60% / hard at exactly
75% / overflow / unknown tokens / non-positive window / inverted custom watermarks), the stable-node
predicate (gate, idle boundary and conflict → true; pending tools, mid-flight jobs and an unavailable
summary → false), the handoff document (usage line, `executor` / `planExecutor` columns, `|`
escaping, the empty-summary warning, the exact next-step commands) and its single `vfsPut` at
`artifacts/handoff-latest.md`, the resume message and compact instructions, `extractAgentId`
(`details` first, `Agent ID:` text fallback, never invented), `resolveResumeId` (gate reject /
`plan → development` / fresh spawn), the three self-heal paths (stream error → one resume, dirty
probe → nudge commit → re-probe → re-archive, unknown verdict → no retry), the real `agent_id` in the
`exec=` segment, and both brake levels at the orchestration layer (hard spawns nothing and claims no
worktree; soft lands the wave and only then writes the handoff). Four mutation probes confirm the
assertions bite: forcing `resolveResumeId` to `""`, `laneProbeNeedsCommit` to `false`, dropping the
hard brake, or hard-coding `actorId` in `exec=` each turns exactly the matching cases red.
