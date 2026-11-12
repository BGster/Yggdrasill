# Yggdrasill · Agent Harness — Hard Constraints and Cold Start

> This file carries exactly two things: the constraints you may not violate, and how to start /
> continue / pause / settle a run. The mechanics (state machine / action protocol / async inbox /
> progress views / run artifacts / Mod maintenance / host contract) live in
> `.ygg/mods/agent-seed/README.md` and the `harness` chapter of the playbook; fetch field rules by
> chapter with `guide.show <section_id>`. `CLAUDE.md` is a symlink to this file.

## 1. Non-negotiable constraints

**0. Data red line**: never read real business data (`*.xlsx` / `*.xls` / `*.csv`, or anything under
a classified mount) into the repository or the context, and never take metadata readings of its
directories (`ls` / `stat` / `find` / `du` / `wc` — row counts, column names and enumerations are as
classified as the values). Verifications that need real data are run by the user, who pastes the
output back; synthetic data generated under `/tmp` is unrestricted.

0b. **serve is read-only**: the `serve` mode (r8) exposes exactly the GET-only HTTP
    observation plane (`docs/DESIGN.md` §3.7). It must never gain a write endpoint — the
    human gates (`proposal.accept` / `gate.approve` / `coordinate.submit` / `verify.submit`
    / `review.note`) have **no online path** and are still ticketed through the CLI/daemon.
    All state writes go exclusively through the single-shot CLI / interactive / daemon modes.
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
   - **Submitter identity (r3)**: subagents submit with `exec=<agent_id>` in the payload tail plus
     their identity in the environment (`YGG_ACTOR=<agent_id>`, and `YGG_ACTOR_SESSION=<session_id>`
     when the spawn side knows it); the host stamps the channel (`|actor=` / `|actor_session=` on
     declared routes) and the mod cross-checks exec vs actor — **recognised, never rejected**
     (`[verified]` / `[mismatch:actor=…]` / `[unverified]`; a main-agent proxy submission without a
     host stamp just displays `[unverified]`). Subagents also self-register a profile card
     (`config.set profile.<handle>.harness=…`, one `config.set` per key) per the brief's Register
     line; main-agent registration is a **fallback** with no data-plane difference — the
     credibility gap lives only in the `registered_by` convention and the event log. Profiles are
     self-reported declarations, not credentials; accountability stays with the executor ledger.
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
     (`grep -n '\-\-events' AGENTS.md README.md .ygg/mods/agent-seed/README.md` finds nothing outside
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
     spots are this file (written as a decidable criterion) and `.ygg/mods/agent-seed/` (gate logic
     in `src/runtime/flow.lua` and `src/handlers/**`, prompts and scripts in `data/*.json`;
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
   - **Mod text hygiene (retired path literals)**: the Mod's prose resources — `data/texts/**`
     and `data/playbook/**` — must not contain the retired path literals `ygg-agent` or
     `pi-agent-seed`; prose must name the current `agent-seed` / `egg` so executors never follow
     text into a non-existent path (the host's `with_legacy_seed_alias` compat layer is the only
     legitimate consumer of the old names, and it lives in `crates/**`, not in Mod text). A line is
     exempt only when it marks itself on the same line with the literal `[deprecated-ref]` — any
     other occurrence is a violation. ① blocks: any edit to Mod texts, briefs or playbook wording
     that reintroduces the old mod/CLI path; ② symptom: an executor runs a gate/probe against
     `…/mods/pi-agent-seed` or a `ygg-agent` binary that does not exist (fail-closed error), or the
     serve panel resolves a stale alias instead of the real mod dir; ③ verification (repo root,
     empty output = pass):
     `grep -rn 'ygg-agent\|pi-agent-seed' .ygg/mods/agent-seed/data/texts/ .ygg/mods/agent-seed/data/playbook/ | grep -v '\[deprecated-ref\]'`
     — any surviving line must add the marker (genuinely historical reference) or be rewritten to
     the current names.

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
  **at runtime** (`settings.play.mode=goal`) are checked before **every batch settlement and at
  retirement** (resident runs hit `review.note` once per batch): a missing record blocks with a
  narrative line, writes no `ending`, and does not advance. A static `goal` preset in
  `data/config.json` is the repository owner's standing configuration and is **out of scope**;
  switching back to `guide` is not checked and `resume` keeps the record. The record only proves
  "one sentence was registered", not that the user said it — never claim it as identity proof.
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
- The host decides "the next step is the retiring close" from `agent.summary`'s `nextEndsRun`: in `review`,
  the first item is `settlement.evolution` (after verify passed and nothing is registered) and
  `review.note` afterwards; with no pre-registration the default is an empty artifact + a neutral
  summary + `c=0/0`, so the scripted default path can run "register zero candidates → review.note".
  Only when the first item is `review.note` **and `run.retire` is armed** is `nextEndsRun=true`; the
  host then switches to `closing` (blocks new writes, drains one last time) before dispatching.
  Unarmed batch settles never report `nextEndsRun=true` (a dumb pump keeps cycling batches, never
  pre-closes), and the drive pump pauses safely (exit 0) when `lifecycle=="resident"` ∧
  `batchDrained` ∧ the inbox has no unconsumed line — it never machine-guns `coordinate.submit`
  on an empty batch.
- **Resident lifecycle (r9)**: a run no longer ends at `review.note` by default. The unarmed
  `review.note` is a **batch settle** (archive → `run.batch`+1 → reset verify/settlement → back to
  `coordinate`, narrative `[BATCH <n> SETTLED]`, never an `[ENDING` prefix); the run-level ending is
  reachable only through explicit retirement (`config.set run.retire=true`, value domain `true`/`""`,
  cleared with an empty value) whose next `review.note` writes `[ENDING · run retired]`. Requirement
  level `done` is untouched (the accept_gate product). `coordinate.submit` is the **batch boundary**;
  verify/settlement/evolution are per-batch (the human-gate cadence rotates per batch, strength
  unchanged); `review.evolution.*` is a rolling single slot (last batch), full history lives in the
  event log. Upgrade discipline: **non-compatibility beats compatibility debt — old runs are never
  migrated or adopted; a load failure opens a new run**; the batch boundary is the natural restart
  point (upgrade after a batch settles, then start a new run-id).

#### 9.1 Turn-cut rules (each one is decidable, and each one saves a turn)

- **Inline executor ledger**: `proposal.submit` takes an optional trailing segment
  `exec=<agent_id>[:<reason>]`. It registers the executor ledger through the same
  `src/domain/executor.lua` implementation as `config.set executor.*`, in the same transaction, before the
  proposal is stored; a failed registration refuses the payload **and creates no proposal**. The
  reserved tail segments are a fixed relative order — `…|exec=<id>[:<reason>]|actor=<actor>|actor_session=<session>`
  (each independently omittable; at most three): `actor=` / `actor_session=` are **host-injected**
  on declared routes (`inboxRoutes.<kind>.actor / .actor_session`); a caller-carried segment is
  rejected host-side (`ACTOR_FORGED`, no dispatch no persist), and a segment outside the trailing
  chain is refused mod-side with a per-segment beat (`req_prop_exec_misplaced` /
  `req_prop_actor_misplaced` / `req_prop_actor_session_misplaced`) — the host pads payloads shorter
  than the declared 7 fields and would silently move mid-payload segments into other slots. Write
  the reason with `:` inside that segment (`exec=agent2:handoff`), since the outer payload splits
  on `|`. The three-state cross-check only compares `exec=` vs the host `actor=` stamp —
  `actor_session` is a pure audit field and never participates. **Opener fact**: the run's first
  identity-carrying *successful* write claims `run.json` `opener_actor` first-come-first-served,
  immutable (rejected/failed dispatches never claim; same-actor re-claims are no-ops).
- **Stage merge (`draft -> planning`)**: `requirement.dispatch` on a `draft` row dispatches **plan**
  and stores `planning` in the same dispatch; there is no analysis turn in the default pipeline.
  `analyzing` survives only as a compatibility state (legacy saves, and an explicit `analysis`
  proposal accepted on a `planning` row), so `R_ANALYZING` and its readers stay — no migration code.
- **Declared single-successor advancement, reactive and effect-without-render**: `src/core/hooks.lua`
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
  `src/handlers/intake/actions.lua`: a handler marks the requirement it just made ready
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
- **Receipt effect (the second class)**: `hooks.useEffect("tools.receipt", …)` in `src/runtime/tools.lua`
  only latches `tool.notified.<call_id>` and narrates one line once a `tool.result` has landed. It
  touches no `req.*` / `prop.*` / gate / lane / stage state, never renders and never chooses — the
  same ① ② ③ ④ rules as `auto_dispatch` apply. Its dependencies are KV-derived scalars and the
  latch makes it idempotent, so a replay converges in two passes and a second `[TOOL SETTLED]` can
  never appear.

#### 9.2 Behavioural acceptance (mandatory when touching Mod source or the scenes map)

Changing `.ygg/mods/agent-seed/src/**` (state machine, gates, render layer, tool bridge),
`.ygg/mods/agent-seed/mod.lua` or the map wording in `data/texts/` requires, besides lint and
the all-1 playtest:
`cargo test -p ygg-sandbox --test agent_seed_semantics` (36 cases: stage merge + `analysis` compat,
accept-gate reject, accept-gate approval releasing the lane, accept-gate amend → stale, plan-gate
reject, reactive auto-dispatch in the same turn (plus `useMemo` invalidation), gate independence
(other requirements' pending proposals / running jobs do not block a gate approval — approve starts
work, per-requirement `gate_problem_for`), a gate still blocked by its own pending proposal
(approve refused, stored state unmoved), development proposal
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
pinned by `data/playbook/` (`tools.reactive-effect-scope`, `subsystems.probe-is-warning`,
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
"$YGGDRASILL_ROOT/scripts/install-egg.sh"                # build + install copy + global symlink (idempotent)
# 手工等价：cargo build --release --example egg
#           mkdir -p .ygg/bin && cp "$YGGDRASILL_ROOT/target/release/examples/egg" .ygg/bin/egg
#           mkdir -p ~/.local/bin && ln -sfn "$YGGDRASILL_ROOT/.ygg/bin/egg" ~/.local/bin/egg
```

全局命令 `egg` = `~/.local/bin/egg` →（链式软链）→ `<repo>/.ygg/bin/egg`（安装副本是
唯一事实源；重建后重跑脚本即同步；回滚 = `rm ~/.local/bin/egg`，零系统残留，D21）。

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
- **Single-source Mod symlink (`packages/mods/agent-seed` -> `../../.ygg/mods/agent-seed`)**:
  `.ygg/mods/agent-seed` is the single source of truth; `./scripts/sync-seed-mod.sh` recalculates
  its `manifest.json` and verifies the relative symlink (`./scripts/sync-seed-mod.sh --check`).
- **`docs/CONTEXT.md` + `docs/DESIGN.md` closed loop**: every `plan` and `development` subagent brief
  mandates reading `docs/CONTEXT.md` (domain invariants and vocabulary) and `docs/DESIGN.md`
  (three-layer architecture, lane lifecycle, and `harnessActions[]` contracts) before proposing or
  editing; during `review` settlement (`settlement.evolution`), both documents must be checked and
  updated whenever domain concepts or architectural contracts changed.

### Start
```bash
.ygg/bin/egg --mod .ygg/mods/agent-seed --run-id <run-name> --runs-root .ygg/runs --raw --resume --event event.start --payload '"<局级任务标题>|<一句话目标>"'
```（启动载荷即章程：boot 直接落 coordinate；无载荷也能 boot，任务显示 (untitled task)，空局收口会被可读拒绝。）

### Serve（只读观察台，r9 全局唯一实例）
```bash
scripts/install-egg.sh                                                             # 构建+安装（含全局 egg 软链）
.ygg/bin/egg --runs-root .ygg/runs serve                                          # 起服/加入全局唯一实例（默认 127.0.0.1:7800；Ctrl-C 停服）
egg serve                                                                          # 全局命令裸起：根集来自 ~/.config/ygg/serve.json（{"roots":["/abs/…"]}）
.ygg/bin/egg serve --port 7801 --poll-ms 300                                     # --port = 显式逃生门（不进全局唯一语义）；--poll-ms 加快变更检测
# 单实例语义：同端口已有自家实例 = 幂等复用退出 0（打印面板 URL）；根不被覆盖/旧版单根实例 = 退出 1。
# 面板：http://127.0.0.1:7800/（默认=最新局详情：主流程状态条+需求卡+决策时间线；#/runs=mod 分组历史局 → #/mods/<key> 实例 → #/runs/<id> 局详情；存档折叠段内 mod 面板 iframe）
# 契约与安全边界见 docs/DESIGN.md §3.7；serve 只读：gate/accept 类人类门禁不在线上，仍走 CLI 票据。
```

### Continue
```bash
A=".ygg/bin/egg --mod .ygg/mods/agent-seed --run-id <run-name> --runs-root .ygg/runs"
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
r9 resident semantics: a run **does not end at `review.note` by default** — the unarmed `review.note`
is a **batch settle** (back to `coordinate`, `run.batch`+1, never an ending). Operate in this cadence:

1. **Per batch**: keep dispatching ready requirements; when the queue is drained (every leaf done,
   0 pending proposals / jobs / conflicts), submit the batch boundary `coordinate.submit` → record
   all-green evidence with `verify.submit` → register `settlement.evolution` (screen the candidates
   against the `evolution` chapter first, per-batch `c=`/`cls=` accounting) → close the batch with
   `review.note` (under `guide` you may pre-register with
   `config.set review.evolution.artifact|summary|candidates|classes=<value>`; `review.note` only
   takes `lesson|score` and archives the registration record single-source). The registration
   payload is `artifact|summary[|c=<kept>/<screened>[|cls=<kind>:<n>,...]]` with the optional
   `sessions=` / `todo=` segments described in §1.9.
2. **Batch drained = pump pause**: after a batch settles, the drive pump pauses safely (exit 0)
   waiting for new `--requirement` feed — feed the next batch, or arm retirement.
3. **Retire (the only ending)**: `config.set run.retire=true` (sub-page item 5), walk one last
   `coordinate.submit → verify → settlement → review.note` → `[ENDING · run retired]` → the pump
   pre-closes and settles (`nextEndsRun` flips only when armed). Run `--settle` and verify `ok` in
   `settlement.json` (four switches: events non-empty / no failed step / an ending line exists / the
   inbox was consumed). Pre-running `--settle` on an open resident run is harmless but settles
   nothing (`reached=open, ok=false`, back to running).
4. **Batch boundary = restart point**: upgrades (mod evolution or host binary) land after a batch
   settles, then open a **new run-id**; old run directories are never migrated or adopted (a load
   failure opens a new run).
5. **Clear the main session's todo list** (`todo clear`, including completed and historical items)
   before ending this run; never carry a finished run's tasks into the next one.

## 3. Upper layer: none（执行壳已删除）

The pi extension (`.pi/extensions/ygg-pi-harness`) was **removed** (2026-11-12; the standing
requirement predates this session — the r1-era restore was a misread of 4acc3a4 and is
superseded by this deletion). Consequences, stated as facts:

- Coordination is driven by the **main session directly through the egg CLI** (`--event` chain);
  the daemon-resident capability is the **host-native `--daemon` / `serve`** modes (r9), not an
  orchestrator extension.
- The batch-entrypoint prohibition (§1.4) remains a **discipline rule**; its extension-side
  enforcement layer (firewall.ts) is gone with the directory. Do not reintroduce it without a
  new decision entry.
- Brief submission templates live in the Mod (`beats-harness.json` `subagent_submit_cmd`);
  worktree/lane rituals live in the playbook (`ch2-delivery.json` §integration) and the
  「开发循环仪式」 section above.
