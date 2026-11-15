# Yggdrasill · Agent Harness — Hard Constraints and Cold Start

> This file carries only non-negotiable cold-start constraints, verbatim run commands and the
> consumer-moment index. Operating rituals live in `docs/context/`; subsystem contracts live in
> `docs/design/`; runtime field rules live in `.ygg/mods/agent-seed/data/playbook/`. `CLAUDE.md`
> is a symlink to this file. Read the index before treating any other surface as authoritative.

## 1. Non-negotiable constraints

**0. Data red line**: never read real business data (`*.xlsx` / `*.xls` / `*.csv`, or anything under
a classified mount) into the repository or context, and never take metadata readings of its directories
(`ls` / `stat` / `find` / `du` / `wc` — row counts, column names and enumerations are also classified).
The user runs real-data verification and pastes the output; synthetic data under `/tmp` is unrestricted.

0b. **serve is read-only — never add a run-state write endpoint.** The `/api/roots` exception and its zero-disk-write proof: `docs/design/host/serve.md` §3.7 / §3.7.6 (read it before touching serve).

0c. **Scene gate — read the authoritative body before acting in any scene.** Before executing any action
    of a scene (prologue / charter / coordinate / settle / release / retired, or a requirement stage),
    read that scene's authoritative body in `docs/context/` as routed by §3. **Authority order**: the
    `docs/context/**` body *is* the rule; the mod receipt's `[READ FIRST]` block, beats text, playbook
    summaries, a previous run's habit and this session's earlier impression are **routes and digests, never
    the rule** — substituting a digest for the body is a violation, and so is acting from memory when the
    body is one read away.
    - **Read again, every time — the decidable form.** Re-entering the same scene, a rejected-and-replanned
      requirement and a new batch each require a **fresh signature by the person doing the action**; a
      signature older than the scope entry is expired and never reusable. What that fresh signature must
      *consume* is decided by two facts, not by habit:

      | Situation | Required | Compliant when |
      | --- | --- | --- |
      | on-disk `ver` differs from the `ver` you last read | **body read** — the whole document, then sign | you can name what changed |
      | same `ver`, gap since your last **body read** ≤ `readGate.window` (default 80 dispatches), no context loss | **version check** — confirm the on-disk `ver` equals the signed one, then sign | the report carries `was=<ver> now=<ver>`; this is **not** a violation |
      | same `ver`, gap > window, or context lost (compaction / new session / handoff) | **body read**, then sign | the read is in this session's transcript |

      **A version check never extends the window: only a body read resets it.** `ver` *is* the content
      identity (`scripts/docs-ver.sh` hashes the body with the ver token blanked to `000000`, keeping 6 hex
      digits), so "the document may have changed since" is fully covered by a version check — an unchanged
      `ver` means a byte-identical body apart from that token (a 24-bit prefix: a change detector, not an
      integrity proof). What a version check cannot cover is your own recall decaying, which tracks distance
      and context loss — exactly what the window and the context-loss trigger measure. The mechanism checks
      `ver` / epoch / window only: a signature attests "signed", never "read" — report it as a declaration
      (§1.7), never as proof.
    - **Unreadable = stop**: a missing path, a moved file, or a body that contradicts the index is a
      stop-and-report — never a licence to proceed from memory.
    - **Read whole**: never claim a read from the first few lines or a single `grep` hit (same origin as §1's
      no-parse-and-truncate rule).

    Three-question criterion (§1.7): ① which action does it block? — `requirement.dispatch` /
    `proposal.submit` / `gate.approve` / `coordinate.submit` executed from a digest, a memory, or a
    last-batch version of a rule; ② symptom — a missing or expired signature, a receipt whose `[READ FIRST]`
    paths or versions no longer resolve, behaviour that follows a retired wording; ③ confirm next time —
    every scene receipt's `[READ FIRST]` block resolves to existing files, the signed version equals the
    on-disk one, and no signature predates the current scene entry.

0d. **Classification axis is the recall moment, never the content type.** One line only: before creating a directory or file, ask *who opens it, and at which moment in the run* — if the answer is a content category ("decisions", "narrative", "misc", "common"), it is a residual dump and must be dissolved into the documents that govern it. Full criterion, both live proofs (`narrative/`, `archive/`) and the three-question test: `docs/wiki-governance.md`「文件边界 = 召回时点边界」.

1. **Every in-run artifact must be carried by an event first**: before writing a file, changing code or
   producing a deliverable, an event must bring it into the state machine (`requirement.dispatch` →
   proposal → gate). The verdict looks at events, not where bytes landed — `~/tmp`, `/tmp` and chat all
   count. "Small", "quick" or "the user did not ask for process" are not exemptions.

2. **The main agent does only four coordination-layer things**: dispatch work, accept proposals serially,
   pass the two mandatory gates, and declare batch release (`release.confirm` / `release.skip`). It never
   implements lane work. After filing a requirement, its first action is `requirement.dispatch`.
   Executor registration, identity, requirement granularity and decision-ID allocation are routed by
   `docs/context/req-acts/intake.md` and `docs/context/rituals/dispatch.md`; identity semantics: `docs/design/mod/identity.md`.

3. **Render receipts in full; never parse-and-truncate**: no sliced narratives, first-line-only reads,
   `startswith` filtering, or grep/python summaries. Every dispatch prints all narrative lines and hands
   raw `pendingChoice` to the user. Read rules from the playbook chapter instead of memory; the phase map
   is in `src/view/guide.lua`. Without egg/mod the self-contained base is the recall tree (`docs/context/README.md` + `acts/` + `req-acts/` + `rituals/fallback.md` + `copy/README.md`); **what degrades and what never degrades is stated in `docs/context/rituals/fallback.md`**, not here — semantics and evidence never degrade.

4. **Single-shot dispatching is not compliance**: batching `--event` flags only hides state from the
   guard; it does not replace `requirement.dispatch` → proposal → gate. Never edit `autosave.json` or
   `events.jsonl` by hand. Direct dispatch under `goal` is legal only after its authorization and still
   needs the event chain; `--drive cmd` is an input channel, never a decision maker.
   - The host exposes exactly one dispatch per process (`--event`). There is and will be no `--events`,
     timeline replay script, settlement loop or wrapper that feeds several actions. One real action per
     turn: read the full receipt, then decide. ① Block any proposed batch entrypoint in
     `crates/ygg-sandbox/examples/egg.rs` or scripts; ② a violation is an event history produced without
     receipt review or a document suggesting `--events`; ③ `grep -n '\-\-events' AGENTS.md README.md
     .ygg/mods/agent-seed/README.md` finds only prohibitions, and `cargo build --release --example egg`
     still exposes a single `--event`.

5. **Stop on violation**: when a constraint was bypassed, stop everything and honestly list what was
   dispatched, what you did yourself and which output has no event; then wait for the user. Never back-fill
   the process first — back-filling launders a violation into a compliant-looking record.

6. **Credentials live only in `.env` / `.env.local`**: resources, configuration and reports name keys only,
   never values. Runtime credential resolution: `docs/design/mod/tool-bridge.md`.

7. **Self-evolution has exactly two decision/enforcement exits: `AGENTS.md` or the Mod — no side scripts,
   no CLI source changes**. Choose this file for a cold-start ruling or `.ygg/mods/agent-seed/` for machine
   enforcement (`src/runtime/flow.lua`, `src/handlers/**`, `data/*.json`; `mod.lua` is assembly only).
   Both may change together. Indexed `docs/**` may carry the chosen ruling's operating or subsystem
   elaboration in the same commit, but is never a third place to originate a constraint.
   - Never write a checker, wrapper or hook as a patch. Scripts may only perform read-only queries and may
     not carry constraints. Never patch the egg CLI source; give the user the symptom and reproduction.
     **Boundary**: a host `#[cfg(test)]` assertion that has to change because a *pack declaration* changed shape is maintenance, not capability expansion — change it in the same commit and name the declaration (`docs/design/mod/mod-package.md`「pack 声明变更的连带义务」).
   - Every evolution needs all three: ① the specific action blocked; ② the visible violation symptom;
     ③ one runnable confirmation. Fewer than three means it is not written. A verbal promise is unhandled.
   - **No claim without its method — absence and counts included.** A negative claim ("this repo never had
     X") and a quantity claim ("16 occurrences") are assertions: unproven until a method that *could have
     returned the opposite* produced them. Before either enters a requirement description, a plan or
     development artifact, a `proposal.submit` summary or an evolution artifact, run the method, paste its
     output and name the method beside the conclusion. An unsupported "does not exist" reports "I did not
     look there", not absence — the r6 ruling that `ygg-lint` "never existed" reached a requirement
     description and an evolution artifact that way, while `crates/ygg-lint` was there all along.


     Anti-patterns and two worked examples: `docs/context/rituals/dispatch.md` §4「取证标准」.
   - Candidate screening and declaration limits: `docs/context/acts/settle.md` + playbook
     `evolution`. Mod text path hygiene: `docs/design/mod/mod-package.md`.

8. **Every prompt-surface edit must account for the full producer/consumer lifecycle.** A normative rule
   has exactly one authoritative carrier. Other carriers are pointers or explicitly labelled runtime
   projections naming that authority, never competing statements. Before adding, editing or moving any
   line of `AGENTS.md`, `docs/**` or `.ygg/mods/agent-seed/data/**`, answer:
   - **Q1 — Consumer moment**: exactly one of `cold-start` · `phase-entry` (name the phase/act) ·
     `before-action` (name it) · `subsystem-touch` (name it). Importance/topic is not an answer.
   - **Q2 — Cost of missing it**: exactly one of `irreversible` (data loss, secret exposure, forced push
     over another's work, fabricated record) · `hard-refusal` · `silent-drift` · `none`.
   - **Q3 — Producer**: name owner, the action/gate/commit forcing rewrite, and the check catching a missed
     rewrite. No trigger means certain drift: add one or delete the line.
   Mechanical placement: keep a line here only for Q1=`cold-start` with Q2∈{`irreversible`,`hard-refusal`},
   or a verbatim cold-start command whose omission causes `silent-drift`. `phase-entry` → the matching act
   in `acts/` + playbook; `before-action` → `rituals/dispatch.md`, `rituals/cleanup.md` or the
   action's playbook chapter; `subsystem-touch` → `docs/design/<subsystem>.md`. A ruling goes **in place, into the documentary it governs** — it carries no identifier; a cold-start line that is merely "why we decided" belongs beside the rule it constrains, never here. Operating steps move to the specification. One-line "when touching X read Y" routes may remain here.
   ① Block a prompt-surface commit without Q1–Q3 or with competing normative copies; ② symptoms are
   duplicate rulings, Q2=`none` cold-start text, or a hard-coded fact with no rewrite trigger; ③ search a
   distinctive phrase across `AGENTS.md`, `docs/context/*.md`, `docs/design/*.md` and playbook JSON — one
   authority, all others pointers or labelled projections. Owner = touched-surface editor; trigger = every
   prompt-surface commit; check = migration matrix + `docs-consistency.sh`.

## 2. Cold start

### Build and install

```bash
YGGDRASILL_ROOT="${YGGDRASILL_ROOT:-.}"
"$YGGDRASILL_ROOT/scripts/install-egg.sh"  # build + install copy + global symlink (idempotent)
# Equivalent: cargo build --release --example egg
# mkdir -p .ygg/bin && cp target/release/examples/egg .ygg/bin/egg
# mkdir -p ~/.local/bin && ln -sfn "$YGGDRASILL_ROOT/.ygg/bin/egg" ~/.local/bin/egg
```

**On macOS the installed copy must be ad-hoc signed, or the kernel kills it on first exec** (`exit 137`,
no error text — it looks like a broken build): run `codesign --sign - --force .ygg/bin/egg` after the
install. `scripts/install-egg.sh` does this itself (skipped when `codesign` is absent); if you copy the
binary by hand, sign **the copy** — a signature on `target/release/examples/egg` does not survive `cp`.

`egg` resolves through `~/.local/bin/egg` to `<repo>/.ygg/bin/egg`; rerun the installer after rebuild,
or remove the global symlink to roll back. `.ygg/mods/agent-seed` is the single Mod source;
`./scripts/sync-seed-mod.sh` recalculates its manifest and `--check` verifies it plus the package symlink.
Storage/version compatibility and fail-closed details: `docs/design/host/cli-daemon.md`.

### Start

```bash
ROOT="$(.ygg/bin/egg new --mod .ygg/mods/agent-seed --in .ygg/runs --event event.start | tail -1 | sed 's/^run-root: //')"
A=".ygg/bin/egg open $ROOT"
$A --event charter.set --payload '"<标题>|<目标>|<成功判据>"'
$A --event run.begin
```

Boot may carry the charter payload directly; `new` ends with `run-root: <path>` (`run_root` in raw mode).
`charter.set` is editable until `run.begin`; see `docs/context/acts/charter.md`.

### Serve (read-only observation)

```bash
.ygg/bin/egg serve  # 127.0.0.1:7800; empty-root start; Ctrl-C stops
curl -X POST http://127.0.0.1:7800/api/roots -H 'Content-Type: application/json' -d '{"root": "<repo>/.ygg/runs"}'
.ygg/bin/egg serve --port 7801 --poll-ms 300  # explicit separate instance / faster polling
```

Registry lifecycle, `DELETE`, the four panel levels, SSE frames and single-instance semantics: `docs/design/host/serve.md` §3.7.1–3.7.5 — read-only, no run-state write endpoint.

### Continue

```bash
A=".ygg/bin/egg open <run-root>"
$A --drive stdin                                     # interactive: human chooses (default)
$A --event config.set --payload '"goal.authorization=<one-line authorization note>"'
$A --event config.set --payload '"play.mode=goal"'   # only after user authorization
$A --event config.set --payload '"tools.root=<checkout>/.pi/tools"'
$A --event config.set --payload '"tools.session=<main-session-id>"'
$A --event config.set --payload '"tools.sessions_root=<session-store-dir>"'
$A --drive none                                      # observe without advancing
$A --event map.show                                  # operation view + all-chapter index; payload ignored
.ygg/bin/egg open <run-root> --requirement '<title>|<description>'  # async inbox only
```

### Pause

Type `q` or end stdin. External-drive `HUMAN[:reason]`, timeout, non-zero exit, invalid output or an
out-of-range index pauses safely without advancing or falling back to option 1; `goal` is independent of
that channel. Re-run Continue to resume. Lane reuse/parking: `docs/design/host/lanes.md`.

### Settle and finish cadence

Batch cadence in one line: drain the queue → `coordinate.submit` → `verify.submit` → `settlement.evolution` → release declaration → `review.note`; unarmed `review.note` settles the batch, `config.set run.retire=true` arms the only ending. Exact payloads, evidence standards, upgrade discipline (new run-id per batch boundary, old runs never migrated or adopted) and both cleanups are stated in `docs/context/acts/settle.md`, `release.md`, `retired.md` and `rituals/cleanup.md` — not repeated here.

## 3. Prompt-surface index

Start from the consumer moment, not the file at hand. Full navigation: `docs/README.md`.

| Moment | Authoritative route |
| --- | --- |
| Entry — which file for this moment | `docs/context/README.md` (time-point → file) + `docs/context/scenes.json` |
| Cold-start vocabulary / irreversible constraints | This file + `docs/context/cold-start/vocabulary.md`, `topology.md`, `principles.md` |
| Charter / coordinate / settle / release / retired | `docs/context/acts/<act>.md`; playbook `repository`, `run_machine`, `evolution`, `integration`, `pointers` |
| Requirement stage (draft → done) | `docs/context/req-acts/intake.md` (1-3) / `delivery.md` (4-6) / `integration.md` (7-9) |
| Coordinate action and decision | Playbook `actions`, `drive`, `inbox`, `dispatch`, `lanes`; cold-start expansions `credentials`, `hard_constraints`, `output` |
| Dispatch / submit / integration ritual | `docs/context/rituals/dispatch.md`; playbook `branches`, `integration` |
| Verification / evidence / cleanup | `docs/context/acts/settle.md` + `docs/context/rituals/cleanup.md`; playbook `gates`, `live_evidence`, `evidence`, `run_artifacts` |
| No egg / no mod fallback | `docs/context/rituals/fallback.md` |
| Entry — which design file for this surface | `docs/design/README.md` (module → file → recall moment) |
| Mod / host / tool touch | `docs/design/kernel/state-machine.md`, `docs/design/mod/mod-package.md`, `docs/design/mod/tool-bridge.md`; playbook `mod_engineering`, `host_contract`, `tools` |
| serve / lanes / CLI / identity touch | `docs/design/host/serve.md`, `docs/design/host/lanes.md`, `docs/design/host/cli-daemon.md`, `docs/design/mod/identity.md` |
| Kernel semantics / storage surface touch | `docs/design/kernel/state-machine.md`, `docs/design/kernel/events.md`, `docs/design/kernel/state-surface.md`, `docs/design/kernel/wire-protocol.md` |
| Runtime copy / protocol wording | `docs/context/scenes/README.md` (seven-domain index) + `docs/design/mod/prompt-surface.md` |
| Why a ruling exists / how to add one | the 「设计依据」 section of the document the ruling governs (criterion: `docs/wiki-governance.md`; product identity: `docs/context/cold-start/vision.md`, `principles.md`) |
| Repository topology | `docs/context/cold-start/topology.md` + `docs/context/cold-start/system-boundaries.md` |
| Whole-run / requirement walk-through | `docs/context/story/run-story.md`, `docs/context/story/requirement-story.md` |

The flat `docs/context/` paths retired by r1 (`workflow-acts.md`, `dispatch-ritual.md`, `copy-contract-*.md` 等 15 条)
were **compat shims for then-frozen consumers**; r5 rewrote every mod pointer and crates test path onto the
authoritative files and deleted them, so they no longer exist and must not be reintroduced. The migration record
lives in `docs/context/README.md`; the original ruling (compat-layer two-way split — a shell for a split,
 a symlink for a 1:1 move, each with a mandatory exit condition) is stated in `docs/wiki-governance.md`
「ver 定义与旧路径兼容层两分法」, never here.

The flat `docs/design/` paths (`<topic>.md`) retired by r6, for the same reason: they became `kernel/ host/ mod/`
(entry `docs/design/README.md`) and **no shim was left** — `docs/design/*.md` may contain only `README.md`, which
`design_tree_index_is_complete_and_ids_match_paths` asserts. The three behaviour narratives (`run-story`,
`requirement-story`, `system-boundaries`) moved to `docs/context/` (`story/`, `cold-start/`) because they have no
code consumer — the design/context criterion is written once in `docs/wiki-governance.md`.

Every playbook chapter has a phase-entry route; `map.show` additionally prints the complete chapter index.
Each phase receipt names its chapter titles, summaries and rule counts. Read the fragment whose
`sections[].id` matches the route rather than quoting a summary as the full rule.
