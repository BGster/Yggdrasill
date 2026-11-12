//! agent-seed semantics gate: **decidable** assertions for the merged-state semantics
//! (branches r1–r13 all landed), the requirement-level state machine, the turn-cut
//! conventions, and the single-layer operation map (AGENTS.md §9.2 behavioural surface).
//!
//! Why this layer exists: the scripted playtest walks only prove the happy paths
//! (baseline A the batch cycle, baseline B the armed retirement) — they never take a
//! **branch** such as `gate.reject` / `requirement.amend` / `proposal.reject` / a stale
//! gate, and the turn-cut mechanics (inline `exec=`, the `draft -> planning` merge, the
//! reactive auto-dispatch effect, the settlement merge) are likewise invisible to them.
//! This file turns those into assertions:
//!
//! - state assertions read **`kv.entries` of the `save` receipt** (`req.*` is machine state,
//!   not prose);
//! - display assertions read the **derived display name** from `requirement.list`
//!   (`[ready]` / `[plan_gate]` / …) — only safe while **no executor is bound** (a bound
//!   executor currently crashes the trace renderer: missing `req_trace_actor_*` beats, see
//!   the commit message of this file for the defect list);
//! - map assertions read the **single operation window** from `map.show` (r1: the design
//!   view is physically unreachable — the handler ignores every payload);
//! - fail-closed assertions read the **driver pre-check envelope** (`error.code=VALIDATION`,
//!   message = event name + registered count + registered set, zero side effects);
//! - every human action goes through **guide's `ygg.choice_selected`** (the ticket is only
//!   stamped there, §8).
//!
//! Boundaries (same stance as AGENTS.md §9 — do not add exits here):
//! - only the public load surface (`ModBundle` / `KernelFacade` / dispatch / save-load):
//!   no host CLI changes, no production-code changes, no side-channel scripts;
//! - **no run directory is read or written**: legacy compatibility uses an **in-memory save
//!   fixture** (drop a key inside the `save` receipt, then `restore`), never `autosave.json`
//!   or `events.jsonl`;
//! - r5 (single-shot receipt default text + `--raw`) and r8 (serve observation deck) are
//!   **host/CLI faces** — their resolution chain (`--raw` > `YGG_FORMAT` > text) and the
//!   GET-only 405 lock live in `examples/egg.rs` / `examples/serve.rs` unit suites
//!   (`cargo test --example egg`); this file pins only the library-visible halves;
//! - r10 is a **pure docs** branch (CONTEXT/DESIGN alignment): no library surface, nothing
//!   to assert here — its acceptance is the documents themselves.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::{json, Value as Json};

use ygg_sandbox::driver::{KernelFacade, DEFAULT_FUEL_BUDGET};
use ygg_sandbox::fuel::{
    auto_budget, resolve_budget, FUEL_AUTOSCALE_CAP, FUEL_AUTOSCALE_PER_ENTRY,
};
use ygg_sandbox::modload::ModBundle;
use ygg_sandbox::playtest::{playtest, PlaytestOptions, Reached};
use ygg_sandbox::HostToolRunner;

// ───────────────────────── locating ─────────────────────────

/// Repository root (`CARGO_MANIFEST_DIR` is `crates/ygg-sandbox`; two levels up).
fn repo_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The instance this project actually loads (r4: the mod directory is `agent-seed`;
/// `data/texts/*` fragments and `data/playbook/*` fragments are this project's field surface).
fn project_mod_dir() -> PathBuf {
    repo_dir().join(".ygg/mods/agent-seed")
}

/// The seed template that ships to users (P5: a relative symlink to the project mod).
fn seed_mod_dir() -> PathBuf {
    repo_dir().join("packages/mods/agent-seed")
}

// ───────────────────────── drive surface ─────────────────────────

/// The minimal drive surface of one run: load + dispatch + read the save.
///
/// `bundle` is kept so the legacy case can rebuild a kernel from the same load parameters
/// (`ModBundle::restore` dispatches the same way as `kernel()`, see modload.rs).
struct Game {
    bundle: ModBundle,
    kernel: KernelFacade,
}

impl Game {
    /// Load and start on `event.start` (the same load surface the host uses).
    fn open(dir: &Path) -> Self {
        let bundle = ModBundle::load(dir)
            .unwrap_or_else(|err| panic!("failed to load {}: {err}", dir.display()));
        let kernel = bundle.kernel().expect("kernel construction");
        let mut game = Self { bundle, kernel };
        game.narrate("event.start", "0");
        game
    }

    /// Dispatch and assert the envelope `ok` (a domain-error envelope always fails the test).
    #[track_caller]
    fn reply(&mut self, event: &str, payload: &str) -> Json {
        let reply: Json = serde_json::from_str(&self.kernel.dispatch(event, payload))
            .unwrap_or_else(|err| panic!("the {event} reply is not JSON: {err}"));
        assert_eq!(reply["ok"], json!(true), "{event} dispatch failed: {reply}");
        reply
    }

    /// Dispatch **without** asserting `ok` — the fail-closed envelope carries
    /// `error.code` / `error.message` instead of a narrative (r1/r11 face).
    #[track_caller]
    fn raw_reply(&mut self, event: &str, payload: &str) -> Json {
        serde_json::from_str(&self.kernel.dispatch(event, payload))
            .unwrap_or_else(|err| panic!("the {event} reply is not JSON: {err}"))
    }

    /// Dispatch and take the narrative lines.
    #[track_caller]
    fn narrate(&mut self, event: &str, payload: &str) -> Vec<String> {
        let reply = self.reply(event, payload);
        reply["narrative"]
            .as_array()
            .unwrap_or_else(|| panic!("the {event} narrative is not an array: {reply}"))
            .iter()
            .map(|line| {
                line.as_str()
                    .unwrap_or_else(|| panic!("a narrative line is not a string: {line:?}"))
                    .to_owned()
            })
            .collect()
    }

    /// The save object from the `save` receipt (state assertions read `kv.entries`).
    #[track_caller]
    fn save(&mut self) -> Json {
        self.reply("save", "0")["save"].clone()
    }

    /// The save receipt serialized (byte-equality is the zero-side-effect proof).
    #[track_caller]
    fn save_bytes(&mut self) -> Vec<u8> {
        serde_json::to_vec(&self.save()).expect("serialize the save")
    }

    /// One KV in the save (missing key → `Null`; the legacy case observes exactly that).
    #[track_caller]
    fn kv(&mut self, key: &str) -> Json {
        self.save()["kv"]["entries"][key].clone()
    }

    /// Assert a KV value (`Null` asserts the key is absent).
    #[track_caller]
    fn assert_kv(&mut self, key: &str, want: Json) {
        let got = self.kv(key);
        assert_eq!(got, want, "KV `{key}` expected {want}, got {got}");
    }

    /// The option texts of the pending choice (`state` is a read-only diagnostic surface).
    #[track_caller]
    fn options(&mut self) -> Vec<String> {
        let reply = self.reply("state", "0");
        reply["pendingChoice"]["options"]
            .as_array()
            .unwrap_or_else(|| panic!("no pending choice: {reply}"))
            .iter()
            .map(|item| {
                item.as_str()
                    .unwrap_or_else(|| panic!("an option is not a string: {item:?}"))
                    .to_owned()
            })
            .collect()
    }

    /// guide routing: click a menu entry by option-text prefix (the **only** legal entry for
    /// human actions, see AGENTS.md §8).
    #[track_caller]
    fn click(&mut self, prefix: &str) -> Json {
        let options = self.options();
        let index = options
            .iter()
            .position(|text| text.starts_with(prefix))
            .unwrap_or_else(|| panic!("no menu entry starts with `{prefix}`: {options:?}"));
        let payload = (index + 1).to_string();
        self.reply("ygg.choice_selected", &payload)
    }

    /// Derived display name: the `[ready]` slot of `requirement.list` — the display surface.
    ///
    /// Only safe while **no executor is bound** anywhere: a bound executor feeds the
    /// three-state trace renderer, whose `req_trace_actor_*` beats are currently missing
    /// (merge-lost data keys — reported; `requirement.list` would fail the dispatch).
    #[track_caller]
    fn phase(&mut self, id: &str) -> String {
        let needle = format!(" {id} [");
        let lines = self.narrate("requirement.list", "0");
        let row = lines
            .iter()
            .find(|line| line.contains(&needle))
            .unwrap_or_else(|| panic!("no {id} in the requirement list: {lines:#?}"));
        let open = row
            .find('[')
            .expect("display name lacks an opening bracket");
        let close = row.find(']').expect("display name lacks a closing bracket");
        row[open + 1..close].to_owned()
    }

    /// The requirement panel line (counts per display name) — the surface the `useMemo`
    /// derived tally feeds.
    #[track_caller]
    fn panel(&mut self) -> String {
        self.narrate("requirement.list", "0")
            .into_iter()
            .find(|line| line.starts_with("[REQ GRAPH]"))
            .expect("requirement.list must start with the graph header")
    }

    /// The requirement **rows** of a `requirement.list` receipt (`mark rN [status] title`).
    #[track_caller]
    fn req_rows(lines: &[String]) -> Vec<(String, String)> {
        lines
            .iter()
            .filter_map(|line| {
                let tokens: Vec<&str> = line.split_whitespace().collect();
                let id = *tokens.get(1)?;
                let status = *tokens.get(2)?;
                let id = id.strip_prefix('r')?;
                if !id.chars().all(|c| c.is_ascii_digit()) || !status.starts_with('[') {
                    return None;
                }
                Some((format!("r{id}"), status.trim_start_matches('[').to_owned()))
            })
            .collect()
    }
}

/// Rust string → JSON string payload (a dispatch payload must be valid JSON).
fn text(payload: &str) -> String {
    serde_json::to_string(payload).expect("serialize the string payload")
}

/// Rejection prefixes come from `pack.json` (`agent.rejectionPrefixes`) — the single source,
/// never a copy-pasted literal (a past drift taught that lesson).
fn rejection_prefixes() -> &'static Vec<String> {
    static PREFIXES: OnceLock<Vec<String>> = OnceLock::new();
    PREFIXES.get_or_init(|| {
        let path = project_mod_dir().join("pack.json");
        let raw = std::fs::read_to_string(&path).expect("read pack.json");
        let pack: Json = serde_json::from_str(&raw).expect("parse pack.json");
        pack["agent"]["rejectionPrefixes"]
            .as_array()
            .expect("agent.rejectionPrefixes must be an array")
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .expect("a rejection prefix must be a string")
                    .to_owned()
            })
            .collect()
    })
}

/// Rejection judgement: the narrative line starts with one of the declared prefixes.
fn refused(lines: &[String], needle: &str) -> bool {
    let prefixes = rejection_prefixes();
    lines
        .iter()
        .any(|line| prefixes.iter().any(|prefix| line.starts_with(prefix)) && line.contains(needle))
}

/// Assert "refused and named that action/reason".
#[track_caller]
fn assert_refused(lines: &[String], needle: &str) {
    assert!(
        refused(lines, needle),
        "expected a rejection that names `{needle}`, got: {lines:#?}"
    );
}

// ───────────────────────── drive scripts ─────────────────────────

/// Start → coordinate → one requirement → the plan job is running
/// (non-human actions are dispatched directly, no ticket needed).
///
/// `draft` now dispatches **plan** directly (analysis and plan are merged): the stored state
/// moves to `planning` in the same dispatch.
fn start_requirement(game: &mut Game) {
    game.narrate(
        "event.start",
        &text("semantics gate|synthetic task description"),
    );
    game.narrate(
        "requirement.add",
        &text("synthetic requirement|synthetic description|semantics-r1"),
    );
    game.narrate("requirement.dispatch", "0");
    game.assert_kv("req.1.status", json!("planning"));
    game.assert_kv("req.1.job", json!("plan"));
}

/// Submit a proposal for any requirement: `req|rev|stage|summary|artifacts|changes|ext`.
///
/// Deliberately **without** an `exec=` tail: the shared drive scripts must keep
/// `requirement.list` (display names) dispatchable — a bound executor feeds the
/// three-state trace renderer whose beats are merge-lost (see the header note).
fn submit_req(game: &mut Game, id: &str, rev: u32, stage: &str, ext: &str) {
    let payload = text(&format!(
        "{id}|{rev}|{stage}|{stage} proposal summary|artifacts/{stage}.md||{ext}"
    ));
    game.narrate("proposal.submit", &payload);
}

/// Submit an `r1` proposal: `req|rev|stage|summary|artifacts|changes|ext` (changes empty).
fn submit(game: &mut Game, rev: u32, stage: &str, ext: &str) {
    submit_req(game, "r1", rev, stage, ext);
}

/// Submit an `r1` proposal with the trailing `exec=` segment (agent ledger + proposal in one
/// transaction, i.e. no extra turn).
fn submit_with_exec(game: &mut Game, rev: u32, stage: &str, ext: &str, exec: &str) {
    let payload = text(&format!(
        "r1|{rev}|{stage}|{stage} proposal summary|artifacts/{stage}.md||{ext}|exec={exec}"
    ));
    game.narrate("proposal.submit", &payload);
}

/// Drive to **plan gate pending**: plan proposal → accept → `gate=plan`.
///
/// The proposal is `exec`-free so callers may keep using `requirement.list` / `phase()`.
fn drive_to_plan_gate() -> Game {
    let mut game = Game::open(&project_mod_dir());
    start_requirement(&mut game);
    submit(&mut game, 1, "plan", "semantics-p1");
    game.click("proposal.accept — ");
    game.assert_kv("req.1.gate", json!("plan"));
    game
}

/// From the plan gate to **accept gate pending**: approve plan (the reactive effect
/// auto-dispatches development in the same turn) → deliver → accept.
fn drive_to_accept(mut game: Game) -> Game {
    game.click("gate.approve — ");
    game.assert_kv("req.1.plan_ok", json!(true));
    // Reactive auto-dispatch: no separate `requirement.dispatch` turn any more.
    game.assert_kv("req.1.job", json!("development"));
    game.assert_kv("req.1.lane", json!(1));
    submit(&mut game, 1, "development", "semantics-p2");
    game.click("proposal.accept — ");
    game.assert_kv("req.1.gate", json!("accept"));
    game
}

/// Drive to **developing + a pending development proposal** (no `accept` gate yet):
/// approve plan → deliver.
///
/// Note: while a gate is pending no new proposal is accepted (`submit_slot_ok`'s gate
/// guard), so "development proposal rejected" can only happen before acceptance — do not
/// append a proposal after `drive_to_accept`.
fn drive_to_dev_pending(ext: &str) -> Game {
    let mut game = drive_to_plan_gate();
    game.click("gate.approve — ");
    game.assert_kv("req.1.status", json!("developing"));
    game.assert_kv("req.1.gate", json!(""));
    submit(&mut game, 1, "development", ext);
    game
}

/// Complete the single requirement, close the coordinate stage, and record all-green
/// evidence — the `review` stage is then ready for the settlement tasks.
fn drive_to_review() -> Game {
    let mut game = drive_to_accept(drive_to_plan_gate());
    game.click("gate.approve — ");
    game.assert_kv("req.1.status", json!("done"));
    let closed = game.click("coordinate.submit — ");
    assert!(
        closed["narrative"]
            .to_string()
            .contains("[COORDINATION DONE]"),
        "coordinate.submit must close the coordinate stage: {closed}"
    );
    let verified = game.click("verify.submit all_green=true — ");
    let verified_lines = verified["narrative"].to_string();
    assert!(
        verified_lines.contains("[GATES GREEN]"),
        "verify.submit all_green=true must record evidence: {verified}"
    );
    game.assert_kv("verify.passed", json!(true));
    game
}

// ───────────────────────── r9 · resident lifecycle ─────────────────────────

/// r9 Resident lifecycle: `review.note` **settles the batch and returns to coordinate**;
/// the run only ends through an explicitly armed retirement (baseline B, a separate case).
///
/// Baseline A — the scripted batch cycle (the default walk): since r2 rev2 the boot lands
/// directly in `coordinate` and the charter gate refuses every closer on a charterless
/// empty run, so the four-forward script (1=coordinate.submit · 1=verify.submit ·
/// 1=settlement.evolution · 1=review.note) is refused at step one and the walk exhausts
/// back at the coordinate choice (`reached=choice`, 5 steps) with **no closure beat**
/// narrated anywhere. Script pins make a menu reorder fail here loudly, not silently.
#[test]
fn resident_batch_cycle_reaches_coordinate_again() {
    for dir in [project_mod_dir(), seed_mod_dir()] {
        // Script (pinned, r2 rev2): every closer refused on the empty run → exhaust at choice.
        let options = PlaytestOptions {
            choices: vec![1, 1, 1, 1],
            ..Default::default()
        };
        let report = playtest(&dir, &options);
        assert!(
            report.ok,
            "{} batch cycle failed: reached={:?} errors={:?}",
            dir.display(),
            report.reached,
            report.errors
        );
        assert_eq!(
            report.reached,
            Some(Reached::Choice),
            "{} must exhaust the script back at a coordinate choice (resident run)",
            dir.display()
        );
        assert_eq!(
            report.steps,
            5,
            "{}: the pinned batch-cycle walk is 5 steps (boot + four refused closers)",
            dir.display()
        );
        // Readable-rejection direction: a charterless empty run must never close — none of
        // the closure beats may appear in the walked narrative.
        let closed = report.narrative_sample.iter().any(|line| {
            line.starts_with("[COORDINATION DONE]")
                || line.starts_with("[GATES GREEN]")
                || line.starts_with("[BATCH")
        });
        assert!(
            !closed,
            "{}: an unchartered empty run must never close: {:?}",
            dir.display(),
            report.narrative_sample
        );
        assert!(
            report.warnings.is_empty() && report.errors.is_empty(),
            "{} must walk clean: {:?} / {:?}",
            dir.display(),
            report.warnings,
            report.errors
        );
    }
}

/// r9 Baseline B — the armed retirement (the **only** path to a run-level ending).
///
/// r2 rev2: on a charterless empty run every closer refuses at the charter gate, so the
/// script can never reach `review` and the retiring ending is **unreachable**: the walk
/// (1×7 refused closers · 5=config.open · 4=arm run.retire · 5=back · 1=refused
/// review.note) exhausts at the coordinate choice (`reached=choice`, 12 steps) with no
/// `[ENDING` line anywhere. Script pins make a menu reorder fail here loudly, not silently.
#[test]
fn resident_retire_is_unreachable_without_a_charter() {
    for dir in [project_mod_dir(), seed_mod_dir()] {
        let options = PlaytestOptions {
            choices: vec![1, 1, 1, 1, 1, 1, 1, 5, 4, 5, 1],
            ..Default::default()
        };
        let report = playtest(&dir, &options);
        assert!(
            report.ok,
            "{} retirement walk failed: reached={:?} errors={:?}",
            dir.display(),
            report.reached,
            report.errors
        );
        assert_eq!(
            report.reached,
            Some(Reached::Choice),
            "{} without a charter the retiring ending must be unreachable",
            dir.display()
        );
        assert_eq!(
            report.steps,
            12,
            "{}: the pinned retirement walk is 12 steps",
            dir.display()
        );
        assert!(
            !report
                .narrative_sample
                .iter()
                .any(|line| line.starts_with("[ENDING")),
            "{}: no ending line may appear on a charterless empty run: {:?}",
            dir.display(),
            report.narrative_sample
        );
        assert!(
            report.warnings.is_empty() && report.errors.is_empty(),
            "{} must walk clean: {:?} / {:?}",
            dir.display(),
            report.warnings,
            report.errors
        );
    }
}

/// r9 The retire value domain and the retiring close (the R2 two-direction lock):
/// - `run.retire` only accepts `true` / `""` (anything else is a payload rejection that
///   writes nothing);
/// - arming goes through the config sub-page (a pure presentation entry — the write is
///   still `config.set`) and narrates its own beat;
/// - armed: the next `review.note` writes the retiring ending (`[ENDING · run retired]`)
///   **without** touching `run.batch` and without a `[BATCH` settle line;
/// - clearing stays available after retirement (`config.set` is stage-less).
#[test]
fn run_retire_value_domain_and_retiring_close() {
    let mut game = drive_to_review();
    // Register the settlement in one merged turn so `review.note` is the menu's first item.
    game.narrate(
        "settlement.evolution",
        &text(
            "|candidates screened against the evolution chapter|c=0/0|\
             sessions=1/0/~/.pi/agent/sessions-trash/semantics|todo=0/re-checked the todo list",
        ),
    );

    // Value domain: anything other than true / "" is refused and writes nothing.
    let bad = game.narrate("config.set", &text("run.retire=false"));
    assert_refused(&bad, "run.retire");
    game.assert_kv("settings.run.retire", json!(null));

    // Arm through the sub-page item (pure presentation entry, the write is still config.set).
    game.click("config — ");
    let sub_page = game.options();
    assert!(
        sub_page.len() == 5 && sub_page[3].starts_with("config.set — run.retire"),
        "the sub-page must offer run.retire as its 4th item: {sub_page:?}"
    );
    game.click("config.set — run.retire");
    game.assert_kv("settings.run.retire", json!("true"));
    // The sub-page survives (only a play-mode switch leaves it).
    assert_eq!(game.options().len(), 5, "still on the config sub-page");
    game.click("back — ");

    // The retiring close: archive + ending, and run.batch stays untouched.
    let closed = game.click("review.note — ");
    let narrative = closed["narrative"].to_string();
    assert!(
        narrative.contains("[ENDING · run retired]"),
        "armed review.note must retire the run: {closed}"
    );
    assert!(
        !narrative.contains("[BATCH"),
        "a retiring close must never carry a batch-settle line: {closed}"
    );
    game.assert_kv("ending", json!("done"));
    // run.batch stays untouched (the key is never written: this run retired on its first
    // settlement — the summary field still reads 1 by default).
    game.assert_kv("run.batch", json!(null));
    game.assert_kv("fsm.stage", json!("done"));
    game.assert_kv(
        "review.evolution.summary",
        json!("candidates screened against the evolution chapter"),
    );

    // Clearing stays available after retirement (done is read-only for requirements, not
    // for the retire switch) and narrates its own beat.
    let cleared = game.narrate("config.set", &text("run.retire="));
    assert_eq!(
        count_lines(&cleared, "[RUN RETIRE CLEARED]"),
        1,
        "{cleared:#?}"
    );
    game.assert_kv("settings.run.retire", json!(""));
}

/// r9 The default no-arg all-1 walk is **expected to fail** under the resident lifecycle:
/// an empty batch loops (coordinate.submit → verify → evolution → review.note → coordinate …)
/// until the step limit — `ok=false, reached=limit` is the documented steady state, not a
/// regression. r2 rev2: the loop is now the **charter-rejection loop** — every closer is
/// refused at the charter gate, so no closure beat may ever appear in the walked narrative.
#[test]
fn default_all1_walk_loops_empty_batches_to_limit() {
    for dir in [project_mod_dir(), seed_mod_dir()] {
        let report = playtest(&dir, &PlaytestOptions::default());
        assert_eq!(
            report.reached,
            Some(Reached::Limit),
            "{}: the resident default walk must loop empty batches to the limit",
            dir.display()
        );
        assert!(
            !report.ok,
            "{}: an empty-batch loop is the expected non-passing state",
            dir.display()
        );
        assert!(
            report.errors.is_empty(),
            "{} must not error (limit is a clean stop): {:?}",
            dir.display(),
            report.errors
        );
        // r2 rev2 readable-rejection direction: the charter gate keeps the empty run from
        // ever closing — no batch boundary / gate-green / batch-settle beat may appear.
        let closed = report.narrative_sample.iter().any(|line| {
            line.starts_with("[COORDINATION DONE]")
                || line.starts_with("[GATES GREEN]")
                || line.starts_with("[BATCH")
        });
        assert!(
            !closed,
            "{}: the charter rejection loop must never close: {:?}",
            dir.display(),
            report.narrative_sample
        );
    }
}

// ───────────────────────── stage merge ─────────────────────────

/// `draft` dispatches **plan** (analysis + plan merged) while the legacy `analysis` slot
/// stays compatible: an analysis proposal is still accepted on a `planning` row.
#[test]
fn draft_dispatch_lands_in_planning_and_keeps_the_analysis_slot_compatible() {
    let mut game = Game::open(&project_mod_dir());
    start_requirement(&mut game);
    assert_eq!(game.phase("r1"), "planning");

    // Legacy shape: an analysis proposal on a planning row must still be accepted
    // (the compat slot), and it must not open a gate.
    submit(&mut game, 1, "analysis", "semantics-legacy-analysis");
    assert_eq!(game.phase("r1"), "analysis_review");
    game.click("proposal.accept — ");
    game.assert_kv("req.1.status", json!("planning"));
    game.assert_kv("req.1.gate", json!(""));

    // After the compat round the row is still dispatchable for the real plan stage.
    game.narrate("requirement.dispatch", "0");
    game.assert_kv("req.1.job", json!("plan"));
}

// ───────────────────────── ① accept gate reject (not amended) ─────────────────────────

/// ① `accept` gate rejected **without** an amend: `plan_ok` keeps its value → the stored
/// state goes back to planning, the display name is still `ready`, and the next dispatch
/// hands out **development** (not plan).
#[test]
fn accept_gate_reject_keeps_plan_ok_and_redispatches_development() {
    let mut game = drive_to_accept(drive_to_plan_gate());
    assert_eq!(game.phase("r1"), "accept");
    game.assert_kv("req.1.status", json!("developing"));
    game.assert_kv("req.1.gate_rev", json!(1));

    // guide gate: a pass-type action without a menu ticket is always refused and the
    // primary state does not move (§8/§9).
    let direct = game.narrate("gate.approve", &text("r1"));
    assert_refused(&direct, "gate.approve");
    game.assert_kv("req.1.status", json!("developing"));
    game.assert_kv("req.1.gate", json!("accept"));

    // Normal reject (rejects are outside the gate, but the menu keeps a single entry).
    game.click("gate.reject — ");
    game.assert_kv("req.1.status", json!("planning"));
    game.assert_kv("req.1.gate", json!(""));
    game.assert_kv("req.1.plan_ok", json!(true));
    assert_eq!(game.phase("r1"), "ready");

    game.narrate("requirement.dispatch", "0");
    game.assert_kv("req.1.status", json!("developing"));
    game.assert_kv("req.1.job", json!("development"));
}

/// Approving the `accept` gate closes the requirement **and releases the lane** in the same
/// turn (the lane was already freed by `proposal.accept`; this asserts the observable end).
#[test]
fn accept_gate_approval_closes_the_requirement_and_releases_the_lane() {
    let mut game = drive_to_accept(drive_to_plan_gate());
    game.click("gate.approve — ");
    game.assert_kv("req.1.status", json!("done"));
    game.assert_kv("req.1.job", json!(""));
    game.assert_kv("req.1.lane", json!(0));
    // No successor is derived from `accept -> done`: the freed job slot is the main agent's
    // decision, so no new job may appear on its own.
    game.assert_kv("req.1.job", json!(""));
    game.assert_kv("req.1.status", json!("done"));
}

// ───────────────────────── ② accept gate + amend → stale ─────────────────────────

/// ② Amend while the `accept` gate is pending: `rev+1` and `plan_ok` is invalidated → the old
/// approval is stale (refused, state unchanged) → the reject path stops in planning.
#[test]
fn accept_gate_amend_invalidates_plan_and_stales_the_pending_gate() {
    let mut game = drive_to_accept(drive_to_plan_gate());
    assert_eq!(game.phase("r1"), "accept");

    game.narrate("requirement.amend", &text("r1|amended title"));
    game.assert_kv("req.1.rev", json!(2));
    game.assert_kv("req.1.plan_ok", json!(false));
    // The gate itself is not taken down (only reject/approve do that) but it is bound to
    // the old rev.
    game.assert_kv("req.1.gate", json!("accept"));
    game.assert_kv("req.1.gate_rev", json!(1));

    // The old approval must not take effect: stale rejection, state and gate untouched.
    let stale = game.click("gate.approve — ");
    let stale_lines: Vec<String> = stale["narrative"]
        .as_array()
        .expect("narrative")
        .iter()
        .map(|line| line.as_str().expect("narrative line").to_owned())
        .collect();
    assert_refused(&stale_lines, "stale");
    game.assert_kv("req.1.status", json!("developing"));
    game.assert_kv("req.1.gate", json!("accept"));
    game.assert_kv("req.1.gate_rev", json!(1));

    game.click("gate.reject — ");
    game.assert_kv("req.1.status", json!("planning"));
    game.assert_kv("req.1.gate", json!(""));
    game.assert_kv("req.1.plan_ok", json!(false));
    assert_eq!(game.phase("r1"), "planning");

    game.narrate("requirement.dispatch", "0");
    game.assert_kv("req.1.job", json!("plan"));
}

// ───────────────────────── ③ plan gate reject ─────────────────────────

/// ③ `plan` gate rejected: `plan_ok` is set false → planning → the next dispatch is still
/// **plan**.
#[test]
fn plan_gate_reject_requires_a_new_plan() {
    let mut game = drive_to_plan_gate();
    assert_eq!(game.phase("r1"), "plan_gate");

    game.click("gate.reject — ");
    game.assert_kv("req.1.status", json!("planning"));
    game.assert_kv("req.1.gate", json!(""));
    game.assert_kv("req.1.plan_ok", json!(false));
    assert_eq!(game.phase("r1"), "planning");

    game.narrate("requirement.dispatch", "0");
    game.assert_kv("req.1.status", json!("planning"));
    game.assert_kv("req.1.job", json!("plan"));
}

// ───────────────────────── reactive auto-dispatch (hooks) ─────────────────────────

/// Approving the `plan` gate makes the row `ready`; the declared `useEffect` auto-dispatches
/// **development for that requirement only** before the receipt is rendered (one turn saved),
/// and the memoized requirement panel reflects the new state (no stale `useMemo`).
#[test]
fn plan_gate_approval_auto_dispatches_development_in_the_same_turn() {
    let mut game = drive_to_plan_gate();
    assert_eq!(game.phase("r1"), "plan_gate");
    let before = game.panel();
    assert!(
        before.contains("ready 0"),
        "before approval nothing is ready: {before}"
    );

    let reply = game.click("gate.approve — ");
    let narrative: Vec<String> = reply["narrative"]
        .as_array()
        .expect("narrative")
        .iter()
        .map(|line| line.as_str().expect("narrative line").to_owned())
        .collect();
    assert!(
        narrative
            .iter()
            .any(|line| line.starts_with("[AUTO-DISPATCH]")),
        "the auto-dispatch effect must narrate its own line: {narrative:#?}"
    );

    // Same turn: the development job and its lane already exist.
    game.assert_kv("req.1.status", json!("developing"));
    game.assert_kv("req.1.job", json!("development"));
    game.assert_kv("req.1.lane", json!(1));
    assert_eq!(game.phase("r1"), "developing");

    // Memoized derived tally must be invalidated by the state change.
    let after = game.panel();
    assert_ne!(before, after, "the panel must not stay memoized stale");
    assert!(
        after.contains("developing 1"),
        "the panel must show the auto-dispatched requirement: {after}"
    );
}

// ───────────────────────── ④ development proposal reject ─────────────────────────

/// ④ Development `proposal.reject` (**before** the accept gate):
/// - not amended: `plan_ok` keeps its value → planning but displayed `ready`, next dispatch
///   is development;
/// - amended: `plan_ok` was invalidated by the rev bump → stops in planning, next dispatch
///   is plan.
#[test]
fn dev_proposal_reject_keeps_plan_ok_unless_amended() {
    // Path without an amend.
    let mut normal = drive_to_dev_pending("semantics-p3");
    normal.narrate("proposal.reject", &text("r1|incomplete artifacts"));
    normal.assert_kv("req.1.status", json!("planning"));
    normal.assert_kv("req.1.plan_ok", json!(true));
    assert_eq!(normal.phase("r1"), "ready");
    normal.narrate("requirement.dispatch", "0");
    normal.assert_kv("req.1.status", json!("developing"));
    normal.assert_kv("req.1.job", json!("development"));

    // Amended path: the proposal is pending, the amend invalidates the rev, then reject.
    let mut amended = drive_to_dev_pending("semantics-p4");
    amended.narrate("requirement.amend", &text("r1|add an acceptance criterion"));
    amended.assert_kv("req.1.rev", json!(2));
    amended.assert_kv("req.1.plan_ok", json!(false));
    amended.narrate("proposal.reject", &text("r1|redo on the new rev"));
    amended.assert_kv("req.1.status", json!("planning"));
    amended.assert_kv("req.1.plan_ok", json!(false));
    assert_eq!(amended.phase("r1"), "planning");
    amended.narrate("requirement.dispatch", "0");
    amended.assert_kv("req.1.job", json!("plan"));
}

/// Amending a requirement **while development is in flight** (before any development proposal
/// is submitted) updates its description and bumps `rev` without resetting progress
/// (`req_amend_active`); submitting the new `rev`'s development proposal must succeed rather
/// than deadlocking on `plan_ok == false`. If that proposal is later rejected, `plan_ok == false`
/// still forces a replan in `planning`.
#[test]
fn developing_amend_without_pending_proposal_does_not_deadlock() {
    let mut game = drive_to_plan_gate();
    game.click("gate.approve — ");
    // Auto-dispatched into developing (no pending proposal yet).
    game.assert_kv("req.1.status", json!("developing"));
    game.assert_kv("req.1.job", json!("development"));

    game.narrate(
        "requirement.amend",
        &text("r1|refined title|refined description in flight"),
    );
    game.assert_kv("req.1.rev", json!(2));
    game.assert_kv("req.1.status", json!("developing"));
    game.assert_kv("req.1.plan_ok", json!(false));

    // Submitting a development proposal for the new rev must succeed (no deadlock).
    submit(&mut game, 2, "development", "semantics-dev-after-amend");
    assert_eq!(game.phase("r1"), "dev_review");

    // And if that proposal is rejected, `plan_ok == false` sends it back to `planning` (replan).
    game.narrate("proposal.reject", &text("r1|needs replan after amend"));
    game.assert_kv("req.1.status", json!("planning"));
    game.assert_kv("req.1.plan_ok", json!(false));
    assert_eq!(game.phase("r1"), "planning");
}

// ───────────────────────── inline executor ledger (`exec=`) ─────────────────────────

/// `proposal.submit` binds the executor in the same transaction, refuses an `exec=` that is
/// not the last segment, and requires a reason for a cross-stage handoff (given here with the
/// `:` form so it survives the outer `|` split).
#[test]
fn proposal_submit_inlines_the_executor_and_refuses_misplaced_exec() {
    let mut game = Game::open(&project_mod_dir());
    start_requirement(&mut game);

    // Misplaced exec= (not the last segment) → payload rejection, no proposal is created.
    let misplaced = game.narrate(
        "proposal.submit",
        &text("r1|1|plan|summary|artifacts/plan.md|exec=agent-1||semantics-bad"),
    );
    assert_refused(&misplaced, "exec=");
    game.assert_kv("prop.1.req", Json::Null);
    game.assert_kv("settings.executor.r1.plan", Json::Null);
    // The job slot is untouched: nothing was consumed.
    game.assert_kv("req.1.job", json!("plan"));

    // Inline binding works and the proposal lands in the same transaction.
    submit_with_exec(&mut game, 1, "plan", "semantics-p1", "agent-1");
    game.assert_kv("settings.executor.r1.plan", json!("agent-1"));
    game.assert_kv("prop.1.stage", json!("plan"));

    // Cross-stage handoff without a reason → precondition rejection, no proposal.
    game.click("proposal.accept — ");
    game.click("gate.approve — ");
    let no_reason = game.narrate(
        "proposal.submit",
        &text("r1|1|development|summary|artifacts/development.md||semantics-p2|exec=agent-2"),
    );
    assert_refused(&no_reason, "reason");
    game.assert_kv("settings.executor.r1.development", Json::Null);
    // The auto-dispatched job slot is still held (the refused submit released nothing).
    game.assert_kv("req.1.job", json!("development"));

    // Same handoff with the `:` separator (the outer payload splits on `|`).
    submit_with_exec(
        &mut game,
        1,
        "development",
        "semantics-p3",
        "agent-2:cross-stage handoff",
    );
    game.assert_kv(
        "settings.executor.r1.development",
        json!("agent-2|cross-stage handoff"),
    );
}

// ───────────────────────── settlement merge ─────────────────────────

/// One `settlement.evolution` turn settles the evolution record **and** the two end-of-run
/// cleanups (`sessions=` / `todo=`), so `review.note` can close the batch right after it.
/// The standalone actions stay usable for step-by-step settlement.
#[test]
fn settlement_evolution_settles_sessions_and_todo_in_one_turn() {
    let mut game = drive_to_review();

    // Standalone actions remain compatible (shape-checked, remaining must be 0).
    game.narrate("settlement.todo", &text("0|standalone re-check"));
    game.assert_kv("settlement.todo.remaining", json!(0));
    game.assert_kv("settlement.todo.verified_by", json!("standalone re-check"));

    // One evolution turn carries the cleanups: `sessions=` is the missing piece now.
    let payload = text(
        "|candidates screened against the evolution chapter|c=0/0|\
sessions=1/0/~/.pi/agent/sessions-trash/semantics|todo=0/re-checked the todo list",
    );
    game.narrate("settlement.evolution", &payload);
    game.assert_kv(
        "settlement.sessions.destination",
        json!("~/.pi/agent/sessions-trash/semantics"),
    );
    game.assert_kv("settlement.sessions.moved", json!(1));
    game.assert_kv("settlement.sessions.remaining", json!(0));
    game.assert_kv("settlement.todo.remaining", json!(0));
    game.assert_kv(
        "settlement.todo.verified_by",
        json!("re-checked the todo list"),
    );
    assert_eq!(
        game.kv("settlement.evolution.summary"),
        json!("candidates screened against the evolution chapter")
    );

    // A malformed optional segment is refused (shape gate), state unchanged.
    let bad = game.narrate(
        "settlement.evolution",
        &text("|summary|c=0/0|sessions=1/3/nowhere"),
    );
    assert_refused(&bad, "sessions=");
    game.assert_kv("settlement.sessions.moved", json!(1));

    // `review.note` now settles the batch (all three settlement tasks are registered):
    // resident default — archive → run.batch+1 → reset verify/settlement → coordinate,
    // never an ending.
    let closed = game.click("review.note — ");
    let narrative = closed["narrative"].to_string();
    assert!(
        narrative.contains("[BATCH 1 SETTLED]"),
        "review.note must settle the batch: {closed}"
    );
    assert!(
        !narrative.contains("[ENDING"),
        "a batch settle must never carry the ending prefix: {closed}"
    );
    game.assert_kv("ending", json!(null));
    game.assert_kv("run.batch", json!(2));
    game.assert_kv("fsm.stage", json!("coordinate"));
    game.assert_kv("verify.passed", json!(false));
    game.assert_kv("settlement.evolution.summary", json!(""));
    game.assert_kv("review.evolution.candidates", json!("0/0"));
    game.assert_kv(
        "review.evolution.summary",
        json!("candidates screened against the evolution chapter"),
    );
}

// ───────────────────────── ⑤ legacy save without the key ─────────────────────────

/// ⑤ Legacy compatibility: in-flight rows from before this model have **no**
/// `req.N.plan_ok` key.
///
/// Build such a save in memory (drop the key inside the `save` receipt) and `restore` it:
/// - a missing key must not make an in-flight developing row look like "plan not approved";
/// - `gate.reject` then infers "the plan gate already passed" from the **current state**
///   (developing) → displays `ready` and writes the inferred value back explicitly.
///
/// This only proves "the current reducer reads a save with a missing key": no historical
/// engine is forged and no autosave / events file is touched.
#[test]
fn legacy_save_without_plan_flag_infers_then_writes_it_back() {
    let mut game = drive_to_accept(drive_to_plan_gate());
    let mut save = game.save();
    let entries = save["kv"]["entries"]
        .as_object_mut()
        .expect("kv.entries must be an object");
    assert_eq!(
        entries.remove("req.1.plan_ok"),
        Some(json!(true)),
        "the sample start must carry an explicit plan_ok=true"
    );
    assert!(!entries.contains_key("req.1.plan_ok"));

    let bytes = serde_json::to_vec(&save).expect("serialize the save");
    let mut legacy = Game {
        bundle: game.bundle.clone(),
        kernel: game
            .bundle
            .restore(&bytes)
            .expect("a save with a missing key must load"),
    };
    // After loading the key is still absent (the reducer does not invent it), but the
    // display surface is unaffected.
    legacy.assert_kv("req.1.plan_ok", Json::Null);
    assert_eq!(legacy.phase("r1"), "accept");

    legacy.click("gate.reject — ");
    legacy.assert_kv("req.1.status", json!("planning"));
    legacy.assert_kv("req.1.plan_ok", json!(true)); // the inferred value is now stored
    assert_eq!(legacy.phase("r1"), "ready");

    legacy.narrate("requirement.dispatch", "0");
    legacy.assert_kv("req.1.status", json!("developing"));
    legacy.assert_kv("req.1.job", json!("development"));
}

// ───────────────────────── ⑤b gate locality (approve starts work, r12) ─────────────────────────

/// Gate independence: **another requirement's** pending proposal / running job does not
/// block r1's gate. Approving r1 at the plan gate while r2 sits in `plan_review` (pending
/// plan proposal) and r3's plan job is running must pass the gate and auto-dispatch r1's
/// development in the same turn. The run-closing preconditions (`coordinate.submit` /
/// `verify.submit` / `review.note`) stay global — only per-requirement advancement is
/// localized (AGENTS.md §9: `gate_problem` stays for the closing chain).
#[test]
fn gate_independence_other_requirements_do_not_block() {
    let mut game = drive_to_plan_gate();
    // r2: add → dispatch → submit a plan proposal that stays pending (`plan_review`).
    game.narrate(
        "requirement.add",
        &text("second requirement|synthetic description|semantics-r2"),
    );
    game.narrate("requirement.dispatch", "0");
    submit_req(&mut game, "r2", 1, "plan", "semantics-r2-p1");
    game.assert_kv("prop.2.status", json!("pending"));
    // A submitted proposal releases the job slot (waiting for acceptance, not running).
    game.assert_kv("req.2.job", json!(""));
    // r3: add → dispatch (its plan job is running, no proposal yet).
    game.narrate(
        "requirement.add",
        &text("third requirement|synthetic description|semantics-r3"),
    );
    game.narrate("requirement.dispatch", "0");
    game.assert_kv("req.3.job", json!("plan"));

    let reply = game.click("gate.approve — ");
    let narrative: Vec<String> = reply["narrative"]
        .as_array()
        .expect("narrative")
        .iter()
        .map(|line| line.as_str().expect("narrative line").to_owned())
        .collect();
    assert!(
        narrative
            .iter()
            .any(|line| line.starts_with("[AUTO-DISPATCH]")),
        "approving r1 must auto-dispatch its development in the same turn: {narrative:#?}"
    );
    game.assert_kv("req.1.plan_ok", json!(true));
    game.assert_kv("req.1.job", json!("development"));
    game.assert_kv("req.1.lane", json!(1));
    // The neighbours are untouched: r2's proposal stays pending (its job slot was released
    // at submit — waiting for acceptance, not running), r3's job keeps running.
    game.assert_kv("prop.2.status", json!("pending"));
    game.assert_kv("req.2.job", json!(""));
    assert_eq!(game.phase("r2"), "plan_review");
    game.assert_kv("req.3.job", json!("plan"));
}

/// A requirement's **own** pending proposal still blocks its gate. While a gate is up the
/// `submit_slot_ok` guard refuses live submits, so the own proposal is injected into the
/// in-memory save fixture (never a run directory) and restored: the approval must be
/// refused and the stored state must not move.
#[test]
fn gate_still_blocked_by_own_pending_proposal() {
    let mut game = drive_to_plan_gate();
    let mut save = game.save();
    let entries = save["kv"]["entries"]
        .as_object_mut()
        .expect("kv.entries must be an object");
    // p1 was accepted at the plan gate; p2 is r1's own pending plan proposal.
    let injected = 2;
    entries.insert("prop.count".to_owned(), json!(injected));
    for (key, value) in [
        ("req", json!(1)),
        ("rev", json!(1)),
        ("stage", json!("plan")),
        ("summary", json!("injected own pending proposal")),
        ("artifacts", json!("artifacts/injected.md")),
        ("status", json!("pending")),
        ("ext", json!("semantics-injected")),
        ("changes", json!("")),
    ] {
        entries.insert(format!("prop.{injected}.{key}"), value);
    }
    let bytes = serde_json::to_vec(&save).expect("serialize the save");
    let mut blocked = Game {
        bundle: game.bundle.clone(),
        kernel: game
            .bundle
            .restore(&bytes)
            .expect("a save with an injected own proposal must load"),
    };
    blocked.assert_kv("prop.2.status", json!("pending"));

    // The snapshot's pendingChoice predates the injection (accept item absent, gate first);
    // `ygg.choice_selected` routes against the freshly recomputed menu, so re-render first
    // to bring the stored choice back in sync before clicking.
    blocked.narrate("requirement.list", "0");

    let refused = blocked.click("gate.approve — ");
    let lines: Vec<String> = refused["narrative"]
        .as_array()
        .expect("narrative")
        .iter()
        .map(|line| line.as_str().expect("narrative line").to_owned())
        .collect();
    // The refusal names the blocked action (the human-readable detail beat is among the
    // merge-lost data keys — see the header note — so the assertion pins the action name).
    assert_refused(&lines, "gate.approve");
    blocked.assert_kv("req.1.gate", json!("plan"));
    blocked.assert_kv("req.1.status", json!("planning"));
}

// ───────────────────────── r1 · single operation map + guide.show deregistered ─────────────────────────

/// Take the **map window** from the `map.show` receipt: from the map head line to the map
/// foot line (ignoring the other rendered lines in the same receipt).
#[track_caller]
fn map_window(lines: &[String]) -> Vec<String> {
    let start = lines
        .iter()
        .position(|line| line.starts_with("[CURRENT NODE]"))
        .unwrap_or_else(|| panic!("missing map head `[CURRENT NODE]`: {lines:#?}"));
    let end = lines
        .iter()
        .position(|line| line.starts_with("(operation view"))
        .unwrap_or_else(|| panic!("missing operation-view foot `(operation view`: {lines:#?}"));
    assert!(
        start < end,
        "the map head must precede the foot: {lines:#?}"
    );
    lines[start..=end].to_vec()
}

/// r1 `map.show` is **single-view and payload-insensitive**: every payload gets the same
/// operation window (the handler ignores the payload entirely — no `full` / `0` / section
/// branches), and the design view (`[MAP]` head) is physically unreachable.
///
/// The wording red lines stay from the two-view era: the operation view promises
/// "selectable" (never "executable" — the render layer cannot see the choice-side
/// precondition) and says "condition not shown" (never "no extra condition" — that would
/// vouch for the candidate).
#[test]
fn map_show_is_payload_insensitive_single_operation_view() {
    for dir in [project_mod_dir(), seed_mod_dir()] {
        let mut game = Game::open(&dir);
        let empty = game.narrate("map.show", "0");
        let zero = game.narrate("map.show", &text(""));
        let full = game.narrate("map.show", &text("full"));
        let junk = game.narrate("map.show", &text("nonsense-payload"));

        let window = map_window(&empty);
        for other in [&zero, &full, &junk] {
            assert_eq!(
                map_window(other),
                window,
                "{}: the map window must not depend on the payload",
                dir.display()
            );
        }

        // Single view: head carries the current stage + panel, foot names the operation view
        // and the payload negation; the design view head is unreachable.
        let joined = window.join("\n");
        assert!(
            window[0].starts_with("[CURRENT NODE] ["),
            "the map head must carry the current stage: {}",
            window[0]
        );
        assert!(
            joined.contains("map.show takes no payload"),
            "the foot must negate the payload surface:\n{joined}"
        );
        assert!(
            !empty.iter().any(|line| line.starts_with("[MAP]")),
            "the design view is deregistered — no `[MAP]` head may render: {empty:#?}"
        );

        // Wording red lines.
        assert!(
            joined.contains("selectable"),
            "every operation-view edge must say `selectable`:\n{joined}"
        );
        assert!(
            joined.contains("condition not shown"),
            "a missing `when` must read `condition not shown`:\n{joined}"
        );
        assert!(
            !joined.contains("executable"),
            "the operation view must not promise `executable` (no choice-side precondition):\n{joined}"
        );
        assert!(
            !joined.contains("no extra condition"),
            "the operation view must not vouch for `no extra condition`:\n{joined}"
        );
    }
}

/// r1×r11 `guide.show` is **physically deregistered**: the driver's registry pre-check
/// fails the dispatch closed before anything runs — `ok:false`, `error.code=VALIDATION`,
/// the message names the event, the registered count and the registered set (which does
/// **not** contain `guide.show`), and the state baseline is byte-identical afterwards.
#[test]
fn guide_show_is_physically_deregistered_and_fails_closed() {
    let mut game = Game::open(&project_mod_dir());
    start_requirement(&mut game);

    let reply = assert_fail_closed(&mut game, "guide.show");
    let message = reply["error"]["message"].as_str().expect("error message");
    let set = registered_set(message);
    assert!(
        set.contains(&"map.show".to_owned()),
        "the registered set must still carry the surviving view action `map.show`: {message}"
    );

    // The registry is intact: a normal view dispatch still works after the refusal.
    let map = game.narrate("map.show", "0");
    assert!(
        map.iter().any(|line| line.starts_with("[CURRENT NODE]")),
        "map.show must keep working after the fail-closed refusal: {map:#?}"
    );
}

/// The shared fail-closed probe (r1/r11): dispatch an unregistered event and pin the
/// driver pre-check contract — envelope, message三要素 (event name / registered count /
/// registered set), and byte-identical state (zero side effects).
#[track_caller]
fn assert_fail_closed(game: &mut Game, event: &str) -> Json {
    let before = game.save_bytes();
    let reply = game.raw_reply(event, "0");
    assert_eq!(
        reply["ok"],
        json!(false),
        "`{event}` must fail closed: {reply}"
    );
    assert_eq!(
        reply["error"]["code"],
        json!("VALIDATION"),
        "the pre-check must be a readable VALIDATION: {reply}"
    );
    let message = reply["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("the error envelope must carry a message: {reply}"));
    assert!(
        message.contains(&format!("未注册事件 `{event}`")),
        "the message must name the rejected event: {message}"
    );
    let marker = "本 Mod 已注册 ";
    let at = message
        .find(marker)
        .unwrap_or_else(|| panic!("the message must carry the registered count: {message}"));
    let digits: String = message[at + marker.len()..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let count: usize = digits
        .parse()
        .unwrap_or_else(|err| panic!("the registered count must be a number: {message} ({err})"));
    // r2 rev2: task.intake was deregistered with the intake stage — exactly 33 events remain.
    assert_eq!(
        count, 33,
        "the merged mod registers 33 events after the intake deregistration — got {count}: {message}"
    );
    let set = registered_set(message);
    assert!(
        !set.iter().any(|name| name == event),
        "the registered set must not contain the rejected event: {message}"
    );
    assert_eq!(
        game.save_bytes(),
        before,
        "a fail-closed refusal must leave the state baseline byte-identical"
    );
    reply
}

/// The registered set appended after `个事件：` (dictionary order, `、`-joined).
fn registered_set(message: &str) -> Vec<String> {
    let marker = "个事件：";
    let at = message
        .find(marker)
        .unwrap_or_else(|| panic!("the message must carry the registered set: {message}"));
    message[at + marker.len()..]
        .split('、')
        .map(|name| name.trim().to_owned())
        .collect()
}

// ───────────────────────── r11 · unknown events fail closed ─────────────────────────

/// r11 Unknown events (a typo'd name, a reserved `ygg.*` prefix that is not registered)
/// fail closed at the driver facade **before any side effect**: readable VALIDATION with
/// the三要素 message, a byte-identical state baseline, and an intact registry (the next
/// dispatch works).
#[test]
fn unknown_events_fail_closed_with_zero_side_effects() {
    let mut game = Game::open(&project_mod_dir());
    start_requirement(&mut game);

    for event in ["event.nope", "ygg.not_registered"] {
        let reply = assert_fail_closed(&mut game, event);
        let message = reply["error"]["message"].as_str().expect("error message");
        let set = registered_set(message);
        assert!(
            set.contains(&"requirement.dispatch".to_owned()),
            "`{event}`: the registered set must be part of the message: {message}"
        );
    }

    // The registry is intact: a normal dispatch still works after the refusals.
    game.narrate("requirement.list", "0");
    game.assert_kv("req.1.status", json!("planning"));
}

// ───────────────────────── r2 · lane branch naming is enforced ─────────────────────────

/// r2 The lane branch **must** be self-reported in the forced shape
/// `feature/{4-char uuid}-{semantic segment}` (≤ 48 total): `lane.claim` accepts a legal
/// name into the lane ledger, refuses every illegal shape through the readable rejection
/// channel (reject ledger moved), and an **empty** branch stays legal at the claim layer
/// (the auto-claim layer refuses it instead — the default name generator is gone).
#[test]
fn lane_branch_naming_is_enforced() {
    let mut game = Game::open(&project_mod_dir());
    game.narrate(
        "event.start",
        &text("branch naming|synthetic task description"),
    );
    game.narrate(
        "requirement.add",
        &text("naming requirement|synthetic description|naming-r1"),
    );

    // Legal shape: claimed into the ledger byte-for-byte, with its own narration.
    let claimed = game.narrate(
        "lane.claim",
        &text("1|r1|feature/aa11-semantic-name|abc1234|1"),
    );
    assert_eq!(count_lines(&claimed, "[LANE CLAIMED]"), 1, "{claimed:#?}");

    // The refusal matrix (r2 rev2 judgement, one entry per rule). Every refusal moves the
    // reject ledger and leaves the lane ledger untouched. (The human-readable reason beat
    // is among the merge-lost data keys — see the header note — so the assertion pins the
    // action name.)
    let refusals = [
        "feature/r1",                                // bare requirement number in the uuid slot
        "FEATURE/aa11-semantic-name",                // prefix is case-sensitive
        "feature/aa11-x",                            // uuid slot must be exactly 4 chars
        "feature/aa1-semantic-name",                 // 3-char uuid slot
        "feature/aa11-deadbeef",                     // 8-char pure hex semantic segment is illegal
        "feature/aa11-12345",                        // pure-digit semantic segment
        "feature/aa11-",                             // empty semantic segment
        &format!("feature/aa11-{}", "a".repeat(37)), // 48+11 > 48 total
    ];
    let rejects_before = game.kv("reject.count");
    for branch in refusals {
        let lines = game.narrate("lane.claim", &text(&format!("2|r1|{branch}|abc1234|1")));
        assert_refused(&lines, "lane.claim");
        assert!(
            game.kv("lane.2.branch") == Json::Null,
            "the refused branch `{branch}` must not bind a lane"
        );
    }
    let rejects_after = game.kv("reject.count");
    assert_ne!(
        rejects_before, rejects_after,
        "the refusals must move the reject ledger"
    );
    // The lane ledger keeps its original claim (the refusals bind nothing new).
    game.assert_kv("lane.1.branch", json!("feature/aa11-semantic-name"));
    game.assert_kv("lane.1.req", json!("r1"));

    // An empty branch is still legal at the claim layer (the lane.claim entry does not
    // assert the shape; the auto-claim layer refuses the empty name instead).
    game.narrate("lane.release", &text("1"));
    let unclaimed = game.narrate("lane.claim", &text("1|r1|||1"));
    assert_eq!(
        count_lines(&unclaimed, "[LANE CLAIMED]"),
        1,
        "{unclaimed:#?}"
    );
    game.assert_kv("lane.1.branch", json!(""));
}

// ───────────────────────── r3 · submitter identity (actor chapter + profiles) ─────────────────────────

/// Submit an `r1` proposal with the full identity tail chain
/// `…|exec=<exec>[:<reason>]|actor=<actor>|actor_session=<session>` (the channel the
/// host stamps on declared routes; here dispatched straight at the kernel).
#[track_caller]
fn submit_with_identity(
    game: &mut Game,
    rev: u32,
    stage: &str,
    ext: &str,
    exec: Option<&str>,
    actor: Option<&str>,
    session: Option<&str>,
) -> Vec<String> {
    submit_identity_for(game, "r1", rev, stage, ext, exec, actor, session)
}

/// 同上，但需求 id 可指定（actor-only 用例在无账本绑定的干净需求上验证）。
#[track_caller]
fn submit_identity_for(
    game: &mut Game,
    req: &str,
    rev: u32,
    stage: &str,
    ext: &str,
    exec: Option<&str>,
    actor: Option<&str>,
    session: Option<&str>,
) -> Vec<String> {
    let mut payload =
        format!("{req}|{rev}|{stage}|{stage} proposal summary|artifacts/{stage}.md||{ext}");
    if let Some(exec) = exec {
        payload.push_str(&format!("|exec={exec}"));
    }
    if let Some(actor) = actor {
        payload.push_str(&format!("|actor={actor}"));
    }
    if let Some(session) = session {
        payload.push_str(&format!("|actor_session={session}"));
    }
    game.narrate("proposal.submit", &text(&payload))
}

/// verified：exec 与宿主章同句柄 → 双章落库；save/重放后 KV 仍可读。
/// （三态 trace 后缀与 `[ACTOR VERIFIED]` 叙事面当前不可断言：`req_trace_actor_*` /
/// `req_prop_actor_*` beats 在合并中丢失——见头注缺陷清单。）
#[test]
fn actor_channel_verified_lands_in_kv_and_survives_a_reload() {
    let mut game = Game::open(&project_mod_dir());
    start_requirement(&mut game);

    submit_with_identity(
        &mut game,
        1,
        "plan",
        "id-verified",
        Some("agent-1"),
        Some("agent-1"),
        Some("s-plan-1"),
    );
    game.assert_kv("prop.1.actor", json!("agent-1"));
    game.assert_kv("prop.1.actor_session", json!("s-plan-1"));
    game.assert_kv("settings.executor.r1.plan", json!("agent-1"));

    // save → rebuild → the identity chapters survive a reload.
    let save = game.save();
    let bytes = serde_json::to_vec(&save).expect("serialize the save");
    let mut rebuilt = Game {
        bundle: game.bundle.clone(),
        kernel: game.bundle.restore(&bytes).expect("restore from save"),
    };
    rebuilt.assert_kv("prop.1.actor", json!("agent-1"));
    rebuilt.assert_kv("prop.1.actor_session", json!("s-plan-1"));
    rebuilt.assert_kv("settings.executor.r1.plan", json!("agent-1"));
}

/// mismatch：exec 与章不同句柄 → **识别不拒绝**（章照记、账本照绑、验收照常推进门禁）。
#[test]
fn actor_channel_mismatch_is_recorded_never_rejected() {
    let mut game = Game::open(&project_mod_dir());
    start_requirement(&mut game);

    submit_with_identity(
        &mut game,
        1,
        "plan",
        "id-mismatch",
        Some("agent-1"),
        Some("agent-2"),
        None,
    );
    game.assert_kv("prop.1.actor", json!("agent-2"));
    game.assert_kv("settings.executor.r1.plan", json!("agent-1"));
    // 识别不拒绝：提案照常待验收、验收照常推进门禁。
    game.click("proposal.accept — ");
    game.assert_kv("req.1.gate", json!("plan"));
}

/// unverified + actor-only：无章 = 章位空串；只带章不带 exec = 只记章、不绑账本、
/// 账本无“被验证者”；账本绑定不受“打回”影响（账本只增改）。
#[test]
fn actor_channel_unverified_and_actor_only() {
    let mut game = Game::open(&project_mod_dir());
    start_requirement(&mut game);

    // unverified：exec 有、章无（旧宿主 / 直接派发面）。
    submit_with_identity(&mut game, 1, "plan", "id-unv", Some("agent-1"), None, None);
    game.assert_kv("prop.1.actor", json!(""));
    game.assert_kv("settings.executor.r1.plan", json!("agent-1"));

    // r1 的账本绑定不受“打回”影响（账本只增改）。
    game.narrate("proposal.reject", &text("r1|replan without identity"));
    game.assert_kv("settings.executor.r1.plan", json!("agent-1"));

    // actor-only：干净需求上只带章不带 exec —— 不绑账本、章落库。
    game.narrate(
        "requirement.add",
        &text("actor-only requirement|synthetic description|id-r2"),
    );
    game.narrate("requirement.dispatch", &text("r2"));
    submit_identity_for(
        &mut game,
        "r2",
        1,
        "plan",
        "id-actor-only",
        None,
        Some("agent-9"),
        None,
    );
    game.assert_kv("prop.2.actor", json!("agent-9"));
    game.assert_kv("settings.executor.r2.plan", Json::Null);
}

/// 段序契约（mod 侧半，当前可断言面）：错位 / 重复 `exec=` 逐段点名拒绝
/// （`req_prop_exec_misplaced` beat 在位）；全链 10 段载荷（7 段主体 + exec + actor +
/// actor_session）各归其位。
/// （actor/actor_session 的错位与坏形状拒绝当前会硬崩而非拒绝：它们的点名 beats
/// 在合并中丢失，`parse_proposal` 返回 `nil, nil` —— 见头注缺陷清单，修复后补回。）
#[test]
fn actor_session_segments_lock_relative_order_and_full_chain() {
    let mut game = Game::open(&project_mod_dir());
    start_requirement(&mut game);

    // 重复 exec= → 拒（多出的进中间段 = 错位）。
    let duplicated = game.narrate(
        "proposal.submit",
        &text("r1|1|plan|s|artifacts/p.md||x|exec=agent-1|exec=agent-2"),
    );
    assert_refused(&duplicated, "exec=");
    game.assert_kv("prop.1.req", Json::Null);
    game.assert_kv("settings.executor.r1.plan", Json::Null);

    // exec= 在中间段 → 拒（必须最后）。
    let misplaced = game.narrate(
        "proposal.submit",
        &text("r1|1|plan|s|artifacts/p.md|exec=agent-1||x"),
    );
    assert_refused(&misplaced, "exec=");
    game.assert_kv("prop.1.req", Json::Null);

    // 全链 10 段（R5 形状锁）：字段与三个尾段各归其位。
    let lines = game.narrate(
        "proposal.submit",
        &text("r1|1|plan|full-chain summary|artifacts/plan.md|ch=1|id-full|exec=agent-1|actor=agent-1|actor_session=s-full"),
    );
    assert!(
        !lines.iter().any(|line| line.contains("REJECTED")),
        "full chain must parse: {lines:#?}"
    );
    game.assert_kv("prop.1.stage", json!("plan"));
    game.assert_kv("prop.1.summary", json!("full-chain summary"));
    game.assert_kv("prop.1.changes", json!("ch=1"));
    game.assert_kv("prop.1.ext", json!("id-full"));
    game.assert_kv("prop.1.actor", json!("agent-1"));
    game.assert_kv("prop.1.actor_session", json!("s-full"));
    game.assert_kv("settings.executor.r1.plan", json!("agent-1"));
}

/// profile 注册表：config.set 逐键注册 roundtrip（KV 落位 / upsert / settings. 前缀等价 /
/// 空值清除 / 键形坏拒绝 / 未知键 fail-closed）。
/// （validate 层的拒绝矩阵当前不可断言：`profile_bad_*` beats 在合并中丢失使
/// `validate_profile_value` 返回 `nil, nil` —— 坏值路径直接崩溃，handle 张冠李戴被静默
/// 放行——见头注缺陷清单，修复后补回矩阵。）
#[test]
fn profile_registration_via_config_set_roundtrip() {
    let mut game = Game::open(&project_mod_dir());
    start_requirement(&mut game);

    game.narrate("config.set", &text("profile.agent-1.harness=pi"));
    game.assert_kv("settings.profile.agent-1.harness", json!("pi"));
    game.assert_kv("settings.profile_index", json!("agent-1"));
    // upsert：同键重写覆盖。
    game.narrate("config.set", &text("profile.agent-1.harness=codex"));
    game.assert_kv("settings.profile.agent-1.harness", json!("codex"));
    // 其余保留键 + 自由键。
    game.narrate(
        "config.set",
        &text("profile.agent-1.session_id=01a1c961-test"),
    );
    game.narrate("config.set", &text("profile.agent-1.role=plan-executor"));
    game.narrate(
        "config.set",
        &text("profile.agent-1.assignments=r1:plan,r1:development,r3"),
    );
    game.narrate("config.set", &text("profile.agent-1.registered_by=main"));
    game.assert_kv(
        "settings.profile.agent-1.session_id",
        json!("01a1c961-test"),
    );
    game.assert_kv("settings.profile.agent-1.role", json!("plan-executor"));
    game.assert_kv(
        "settings.profile.agent-1.assignments",
        json!("r1:plan,r1:development,r3"),
    );
    game.assert_kv("settings.profile.agent-1.registered_by", json!("main"));
    game.assert_kv("settings.profile_index", json!("agent-1"));
    // settings. 前缀写法等价。
    game.narrate("config.set", &text("settings.profile.agent-1.role=dev"));
    game.assert_kv("settings.profile.agent-1.role", json!("dev"));
    // 空值 = 逐字段清除。
    game.narrate("config.set", &text("profile.agent-1.role="));
    game.assert_kv("settings.profile.agent-1.role", json!(""));
    // 键形坏（缺 <key> 段）→ parse 层拒绝且不落 KV（detail 走缺省文案）。
    let malformed = game.narrate("config.set", &text("profile.agent-1=x"));
    assert_refused(&malformed, "config.set");
    game.assert_kv("settings.profile.agent-1", Json::Null);
    // 真未知键 → fail-closed（config_keys 面同步拒绝）。
    let alien = game.narrate("config.set", &text("profile_index=agent-x"));
    assert_refused(&alien, "config.set");
    game.assert_kv("settings.profile_index", json!("agent-1"));
}

/// P10 决策归属的守门线：`play.mode=goal` 在 `goal.authorization` 注册之前会发出可读
/// 警告（`[GOAL AUTH PENDING]`）；注册后再切不再警告。
#[test]
fn goal_authorization_gates_goal_mode_with_a_readable_warning() {
    let mut game = Game::open(&project_mod_dir());

    let goal_without_auth = game.narrate("config.set", &text("play.mode=goal"));
    assert_eq!(
        count_lines(&goal_without_auth, "[GOAL AUTH PENDING]"),
        1,
        "{goal_without_auth:#?}"
    );
    game.narrate(
        "config.set",
        &text("goal.authorization=authorized for this run"),
    );
    let goal_with_auth = game.narrate("config.set", &text("play.mode=goal"));
    assert_eq!(
        count_lines(&goal_with_auth, "[GOAL AUTH PENDING]"),
        0,
        "{goal_with_auth:#?}"
    );
}

// ───────────────────────── r4 · single-source agent-seed layout ─────────────────────────

/// r4/P5 The mod lives in exactly one physical directory: `.ygg/mods/agent-seed` loads,
/// `packages/mods/agent-seed` is the relative symlink to it (the second load face), the
/// legacy `pi-agent-seed` directory is gone, and the content-addressed manifest matches the
/// files on disk (byte lengths).
#[test]
fn seed_layout_is_single_source_agent_seed() {
    let project = project_mod_dir();
    let seed = seed_mod_dir();

    // The legacy name is gone (no zombie directory may survive the rename).
    assert!(
        !repo_dir().join(".ygg/mods/pi-agent-seed").exists(),
        "`.ygg/mods/pi-agent-seed` must not exist after the r4 rename"
    );

    // The seed path is a symlink resolving to the project mod (P5 single source).
    let link = std::fs::read_link(&seed)
        .unwrap_or_else(|err| panic!("packages/mods/agent-seed must be a symlink: {err}"));
    assert_eq!(
        link.to_string_lossy(),
        "../../.ygg/mods/agent-seed",
        "the symlink must stay relative"
    );
    assert_eq!(
        std::fs::canonicalize(&seed).expect("canonicalize the seed link"),
        std::fs::canonicalize(&project).expect("canonicalize the project mod"),
        "the seed symlink must resolve to the project mod"
    );

    // The manifest is the content-addressed lock: every recorded byte length matches disk.
    let manifest = read_json(&project.join("manifest.json"));
    for section in ["code", "entries"] {
        for item in manifest[section]
            .as_array()
            .unwrap_or_else(|| panic!("manifest.{section} must be an array"))
        {
            let path = item["path"].as_str().expect("manifest path");
            let recorded = item["byteLength"].as_u64().expect("manifest byteLength");
            let actual = std::fs::metadata(project.join(path))
                .unwrap_or_else(|err| panic!("manifest file {path} must exist: {err}"))
                .len();
            assert_eq!(
                recorded, actual,
                "manifest byteLength drift for {path} (run scripts/sync-seed-mod.sh)"
            );
        }
    }

    // Both load faces work: the project directory and (through the symlink) the seed path.
    let mut game = Game::open(&project);
    game.narrate("event.start", "0");
    let mut via_seed = Game::open(&seed);
    via_seed.narrate("event.start", "0");
}

// ───────────────────────── r6 · fragments + five layers ─────────────────────────

/// r6 The mod architecture is **fragments + five layers**: 24 code modules all under
/// `src/{core,domain,view,runtime,handlers}/`, texts split into 7 fragments (core + 6
/// beats groups) whose beat keys are pairwise disjoint (the `mod.lua` assembly contract),
/// the playbook split into 12 chapter fragments, no monolith `data/scenes.json` /
/// `data/playbook.json` left, and tuning living in `data/texts/core.json`.
#[test]
fn mod_architecture_is_fragments_plus_five_layers() {
    let project = project_mod_dir();
    let pack = read_json(&project.join("pack.json"));

    // ① the code face: 24 modules, five layers, entry excluded, all on disk.
    let modules = pack["code"]["modules"]
        .as_array()
        .expect("code.modules must be an array");
    assert_eq!(
        modules.len(),
        24,
        "the merged mod declares 24 modules: {modules:?}"
    );
    let entry = pack["code"]["entry"].as_str().expect("code.entry");
    let mut layers = std::collections::BTreeSet::new();
    for module in modules {
        let path = module.as_str().expect("module path");
        assert_ne!(path, entry, "the entry must not appear in code.modules");
        let rest = path
            .strip_prefix("src/")
            .unwrap_or_else(|| panic!("{path}: every module must live under src/"));
        let layer = rest.split('/').next().expect("the layer segment");
        assert!(
            matches!(layer, "core" | "domain" | "view" | "runtime" | "handlers"),
            "{path}: modules may only live in the five layers"
        );
        layers.insert(layer.to_owned());
        assert!(
            project.join(path).is_file(),
            "declared module {path} must exist on disk"
        );
    }
    assert_eq!(
        layers.len(),
        5,
        "all five layers must be present: {layers:?}"
    );

    // ② the data face: fragments only — no monolith survives.
    assert!(
        !project.join("data/scenes.json").exists(),
        "data/scenes.json must be gone (fragmentized in r6)"
    );
    assert!(
        !project.join("data/playbook.json").exists(),
        "data/playbook.json must be gone (fragmentized in r6)"
    );
    let pack_entry_paths: Vec<String> = pack["entries"]
        .as_array()
        .expect("pack entries")
        .iter()
        .map(|item| item["path"].as_str().expect("entry path").to_owned())
        .collect();
    let texts = pack_entry_paths
        .iter()
        .filter(|path| path.starts_with("data/texts/"))
        .count();
    assert_eq!(texts, 7, "texts = core + 6 beats fragments");
    let playbook = pack_entry_paths
        .iter()
        .filter(|path| path.starts_with("data/playbook/"))
        .count();
    assert_eq!(playbook, 12, "playbook = head + 11 chapter fragments");
    for path in &pack_entry_paths {
        assert!(
            project.join(path).is_file(),
            "declared resource {path} must exist on disk"
        );
    }

    // ③ the assembly contract: beat keys across fragments are pairwise disjoint.
    let mut owner: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    let mut total_beats = 0usize;
    for path in pack_entry_paths
        .iter()
        .filter(|p| p.starts_with("data/texts/"))
    {
        let fragment = read_json(&project.join(path));
        // core.json carries scenes/endings/prompts/options/tuning (no beats group).
        let Some(beats) = fragment["beats"].as_object() else {
            continue;
        };
        for (key, _) in beats {
            total_beats += 1;
            let previous = owner.insert(key.to_owned(), path.to_owned());
            assert!(
                previous.is_none(),
                "beat key `{key}` is declared twice ({previous:?} and {path}) — the assembly \
                 would silently overwrite one side"
            );
        }
    }
    assert!(
        total_beats >= 319,
        "the merged fragments must carry the full monolith beat set (319+): got {total_beats}"
    );

    // ④ tuning lives in the core fragment now.
    let core = read_json(&project.join("data/texts/core.json"));
    for key in [
        "tool_capacity",
        "tool_output_max",
        "artifact_capacity",
        "job_capacity",
    ] {
        assert!(
            core["tuning"][key].as_u64().is_some(),
            "core.json tuning.{key} must be an unsigned integer"
        );
    }
    // (The per-key completeness scan — every `TEXTS.<group>.<name>` the source references
    //  resolves in the assembled fragments — is **deferred**: 24 merge-lost beat keys make
    //  it red today. Re-add it once the data keys are restored; see this file's commit
    //  message for the exact missing-key list and the repro.)
}

// ───────────────────────── r7 · egg is the host program ─────────────────────────

/// A mod resource read straight from disk (the declaration surface is a file, not Lua state).
fn read_json(path: &Path) -> Json {
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|err| panic!("parse {}: {err}", path.display()))
}

/// `<namespace>.<name>`: two segments, lowercase letter first, then lowercase / digits / `_` / `-`.
fn tool_segment_ok(part: &str) -> bool {
    let mut chars = part.chars();
    let first_ok = matches!(chars.next(), Some(first) if first.is_ascii_lowercase());
    first_ok
        && part
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn tool_id_shape_ok(id: &str) -> bool {
    let parts: Vec<&str> = id.split('.').collect();
    parts.len() == 2 && parts.iter().all(|part| tool_segment_ok(part))
}

/// Programs the host refuses to spawn as a tool (`ToolRegistry`'s shell blacklist).
const SHELL_BASENAMES: &[&str] = &[
    "sh",
    "bash",
    "zsh",
    "dash",
    "fish",
    "ksh",
    "csh",
    "tcsh",
    "cmd",
    "cmd.exe",
    "powershell",
    "pwsh",
];

/// r7/T5 The merged declaration surface of the **single** mod: `pack.json` `tools` (Layer 1)
/// and `data/config.json` `allowedTools` (Layer 2) are the same set, every tool shape-checks,
/// the six built-in subsystem tools are served by the renamed host binary **`egg`** (the two
/// `git.*` readers stay `git`), no `ygg-agent` literal survives anywhere in `pack.json`,
/// `src/runtime/tools.lua` is the five-layer module path, and the fragments carry every tool
/// beat plus the tuning caps.
#[test]
fn tool_declarations_pin_the_merged_capability_surface() {
    let project = project_mod_dir();
    let pack = read_json(&project.join("pack.json"));
    let pack_raw = std::fs::read_to_string(project.join("pack.json")).expect("read pack.json");
    let config = read_json(&project.join("data/config.json"));

    // ① r7: the host binary is `egg` — no stale literal anywhere in the pack.
    assert!(
        !pack_raw.contains("ygg-agent"),
        "pack.json must not reference the retired `ygg-agent` binary name"
    );

    let tools = pack["tools"]
        .as_object()
        .expect("pack.json tools must be an object");
    assert!(
        !tools.is_empty(),
        "the tool bridge needs at least one declared tool"
    );
    let mut declared: Vec<String> = Vec::new();
    let mut max_output_cap = 0u64;
    for (id, spec) in tools {
        assert!(
            tool_id_shape_ok(id),
            "tool id `{id}` must be <namespace>.<name>"
        );
        assert!(id.len() <= 64, "tool id `{id}` is longer than 64 bytes");
        declared.push(id.clone());
        assert_eq!(
            spec["kind"],
            json!("cli"),
            "{id}: this round only declares cli tools"
        );
        // `readOnly` is a declared fact, not a policy: the active subsystem needs writer
        // tools (`git.worktree_claim` / `harness.archive_sessions`), and a writer may never
        // detach (host contract: `inlineWaitMs > 0` when `readOnly == false`).
        let read_only = spec["readOnly"]
            .as_bool()
            .unwrap_or_else(|| panic!("{id}: readOnly must be a boolean"));
        assert_eq!(
            spec["resultEvent"],
            json!("tool.result"),
            "{id}: the result must come back as the event the Mod handles"
        );
        let timeout = spec["timeoutMs"]
            .as_u64()
            .unwrap_or_else(|| panic!("{id}: timeoutMs must be an unsigned integer"));
        let inline = spec["inlineWaitMs"]
            .as_u64()
            .unwrap_or_else(|| panic!("{id}: inlineWaitMs must be an unsigned integer"));
        assert!(
            read_only || inline > 0,
            "{id}: a writer tool (`readOnly: false`) may not detach - inlineWaitMs must be positive"
        );
        assert!(
            timeout >= inline,
            "{id}: inlineWaitMs must not exceed timeoutMs"
        );
        let output_cap = spec["maxOutputBytes"].as_u64().unwrap_or(0);
        assert!(output_cap >= 1, "{id}: maxOutputBytes must be at least 1");
        max_output_cap = max_output_cap.max(output_cap);
        let program = spec["program"]
            .as_str()
            .unwrap_or_else(|| panic!("{id}: program must be a string"));
        assert!(
            !program.starts_with('/'),
            "{id}: program must not be absolute"
        );
        assert!(
            !program.contains(".."),
            "{id}: program must not escape with .."
        );
        let basename = program.rsplit('/').next().unwrap_or(program);
        assert!(
            !SHELL_BASENAMES.contains(&basename),
            "{id}: `{basename}` is on the shell blacklist"
        );
        for arg in spec["args"]
            .as_array()
            .unwrap_or_else(|| panic!("{id}: args must be an array"))
        {
            let arg = arg
                .as_str()
                .unwrap_or_else(|| panic!("{id}: args items must be strings"));
            // Placeholders are legal **only** in the `{{name}}` shape (a bare, lowercase
            // identifier). A credential arrives through `$ENV{VAR}` host-side, never inline.
            assert!(
                !arg.contains("$ENV{"),
                "{id}: `{arg}` must not read the process environment inline"
            );
            for (at, _) in arg.match_indices("{{") {
                let rest = &arg[at + 2..];
                let end = rest
                    .find("}}")
                    .unwrap_or_else(|| panic!("{id}: `{arg}` has an unclosed `{{`"));
                let name = &rest[..end];
                let legal = !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                    && !name.starts_with(|c: char| c.is_ascii_digit());
                assert!(
                    legal,
                    "{id}: `{{{{{name}}}}}` must name a lowercase identifier (the Mod fills `lane` / `req` / `branch` / `base` / `path` / `mode` / `parent` / `batch`)"
                );
            }
            assert!(
                arg.matches("{{").count() == arg.matches("}}").count(),
                "{id}: `{arg}` has an unbalanced placeholder"
            );
        }
    }

    // ② the six built-in subsystem tools are served by `egg`; the two plain git readers
    //    stay `git` (r7 renamed the host, not git).
    let subsystem_tools = [
        "git.worktree_claim",
        "git.worktree_release",
        "git.lane_probe",
        "fs.artifact_probe",
        "gate.verify_suite",
        "harness.archive_sessions",
    ];
    for id in subsystem_tools {
        assert_eq!(
            tools[id]["program"],
            json!("egg"),
            "{id}: the built-in tools are served by the renamed host binary"
        );
    }
    for id in ["git.head_short", "git.status"] {
        assert_eq!(
            tools[id]["program"],
            json!("git"),
            "{id}: the plain git readers are served by git itself"
        );
    }

    // ③ `allowedTools` is the request-time gate: a superset — here exactly the same set.
    let allowed = config["allowedTools"]
        .as_array()
        .expect("allowedTools must be an array");
    let mut seen: Vec<String> = Vec::new();
    for item in allowed {
        let id = item.as_str().expect("allowedTools items must be strings");
        assert!(
            tool_id_shape_ok(id),
            "allowed tool id `{id}` must be <namespace>.<name>"
        );
        assert!(
            !seen.contains(&id.to_owned()),
            "allowed tool `{id}` is listed twice"
        );
        seen.push(id.to_owned());
    }
    seen.sort();
    let mut declared_sorted = declared.clone();
    declared_sorted.sort();
    assert_eq!(
        seen, declared_sorted,
        "allowedTools and pack.json tools must be the same set"
    );

    // ④ the five-layer module path replaced the flat layout.
    let modules = pack["code"]["modules"]
        .as_array()
        .expect("code.modules must be an array");
    assert!(
        modules.iter().any(|item| item == "src/runtime/tools.lua"),
        "src/runtime/tools.lua must be declared in code.modules: {modules:?}"
    );
    assert!(
        !modules.iter().any(|item| item == "src/tools.lua"),
        "the flat src/tools.lua layout is retired: {modules:?}"
    );

    // ⑤ the inbox route matches the wire the handlers produce.
    let route = &pack["inboxRoutes"]["tool_result"];
    assert_eq!(route["action"], json!("tool.result"));
    assert_eq!(
        route["fields"],
        json!([
            "call_id",
            "tool_id",
            "status",
            "code",
            "output",
            "external_id"
        ]),
        "the inbox route fields are the wire segments, in order"
    );
    assert_eq!(
        route["write"],
        json!({"flag": "tool-result", "shape": "payload"})
    );

    // ⑥ the fragments carry every tool beat (a missing key would render a raw placeholder).
    let beats = [
        "tool_requested",
        "tool_result",
        "tool_settled",
        "tool_replayed",
        "tool_after_done",
        "tool_not_allowed",
        "tool_allowed_none",
        "tool_usage",
        "tool_result_usage",
        "tool_bad_tool_id",
        "tool_bad_args",
        "tool_bad_call_id",
        "tool_call_dup",
        "tool_capacity",
        "tool_unknown_call",
        "tool_result_mismatch",
        "tool_bad_status",
        "tool_bad_code",
        "tool_output_truncated",
    ];
    let assembled_beats = assembled_beats(&project);
    for key in beats {
        assert!(
            assembled_beats.contains_key(key),
            "the beats fragments are missing `{key}` (the shared src/ would render a raw placeholder)"
        );
    }

    // ⑦ the Mod's own output cap must not be tighter than the host's per-tool cap.
    let tuning = read_json(&project.join("data/texts/core.json"))["tuning"].clone();
    let truncate_at = tuning["tool_output_max"]
        .as_u64()
        .expect("tuning.tool_output_max must be an unsigned integer");
    assert!(
        truncate_at >= max_output_cap,
        "tuning.tool_output_max ({truncate_at}) is below the largest maxOutputBytes ({max_output_cap})"
    );
}

/// The assembled beats set (what `mod.lua` builds from the fragments — the same union the
/// Lua side sees through `TEXTS.beats`).
fn assembled_beats(project: &Path) -> std::collections::BTreeMap<String, Json> {
    let mut out = std::collections::BTreeMap::new();
    let entries = read_json(&project.join("pack.json"))["entries"]
        .as_array()
        .expect("pack entries")
        .clone();
    for entry in entries {
        let path = entry["path"].as_str().unwrap_or("");
        if !path.starts_with("data/texts/") {
            continue;
        }
        let fragment = read_json(&project.join(path));
        if let Some(beats) = fragment["beats"].as_object() {
            for (key, value) in beats {
                out.insert(key.to_owned(), value.clone());
            }
        }
    }
    out
}

// ───────────────────── E6 · Mod text hygiene (D-MA19) ─────────────────────

/// Mod prose fragments must not carry retired path literals (`ygg-agent` / `pi-agent-seed`).
/// Scope = pack.json `entries` whose path sits under `data/texts/` or `data/playbook/`; a line
/// violates when it names a retired literal without the explicit `[deprecated-ref]` marker on
/// the same line. Failure output lists `file:line` per violation. Machine-checkable half of
/// AGENTS.md §1.7 「Mod text hygiene (retired path literals)」; D-MA19 retired the alias layer,
/// so prose must point at the current names (`agent-seed` / `egg`).
#[test]
fn mod_text_fragments_carry_no_retired_paths() {
    let mod_dir = project_mod_dir();
    let pack: Json = serde_json::from_str(
        &std::fs::read_to_string(mod_dir.join("pack.json")).expect("read pack.json"),
    )
    .expect("parse pack.json");
    let fragments: Vec<std::path::PathBuf> = pack["entries"]
        .as_array()
        .expect("pack.json entries is an array")
        .iter()
        .filter_map(|entry| entry["path"].as_str())
        .filter(|path| path.starts_with("data/texts/") || path.starts_with("data/playbook/"))
        .map(|path| mod_dir.join(path))
        .collect();
    assert!(
        !fragments.is_empty(),
        "scope guard: pack.json must declare texts/playbook fragments"
    );
    let mut violations: Vec<String> = Vec::new();
    for fragment in &fragments {
        let text = std::fs::read_to_string(fragment)
            .unwrap_or_else(|err| panic!("read {}: {err}", fragment.display()));
        for (index, line) in text.lines().enumerate() {
            let retired = line.contains("ygg-agent") || line.contains("pi-agent-seed");
            if retired && !line.contains("[deprecated-ref]") {
                violations.push(format!("{}:{}", fragment.display(), index + 1));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "retired path literals in Mod text (rewrite to `agent-seed`/`egg` or mark \
         `[deprecated-ref]`): {}",
        violations.join(", ")
    );
}

// ───────────────────────── tool bridge (T1–T4) ─────────────────────────

/// The narrative array of an envelope (state assertions stay on `kv.entries`, prose assertions
/// read these lines).
#[track_caller]
fn narrative_lines(reply: &Json) -> Vec<String> {
    reply["narrative"]
        .as_array()
        .unwrap_or_else(|| panic!("the narrative is not an array: {reply}"))
        .iter()
        .map(|line| {
            line.as_str()
                .unwrap_or_else(|| panic!("a narrative line is not a string: {line:?}"))
                .to_owned()
        })
        .collect()
}

/// How many lines carry a marker (a rewrite must not silently change the count).
fn count_lines(lines: &[String], needle: &str) -> usize {
    lines.iter().filter(|line| line.contains(needle)).count()
}

/// T1 The request direction: a `tool.call` publishes **one dense snapshot** (`meta.tool_calls`,
/// allocation order, `args` omitted when empty) and one self-describing KV row. It never
/// publishes `meta.summary` — the two keys are independent slots of the same `BTreeMap`.
#[test]
fn tool_call_publishes_a_dense_pending_snapshot_and_kv_rows() {
    let mut game = Game::open(&project_mod_dir());
    game.narrate(
        "event.start",
        &text("tools gate|synthetic task description"),
    );

    let first = game.reply("tool.call", &text("git.head_short"));
    let calls = first["meta"]["tool_calls"]
        .as_array()
        .unwrap_or_else(|| panic!("meta.tool_calls must be an array: {first}"));
    assert_eq!(calls.len(), 1, "one request queues one call: {first}");
    assert_eq!(calls[0]["id"], json!("t1"));
    assert_eq!(calls[0]["tool"], json!("git.head_short"));
    assert!(
        calls[0].get("args").is_none(),
        "an empty argument set must omit the `args` key (a Lua empty table can only be `[]`): {first}"
    );
    assert!(
        first["meta"].get("summary").is_none(),
        "one dispatch writes only the keys its own action set — tool_calls must not drag summary in: {first}"
    );

    game.assert_kv("tool.count", json!(1));
    game.assert_kv("tool.pending", json!(1));
    game.assert_kv("tool.t1.status", json!("pending"));
    game.assert_kv("tool.t1.code", json!(-1));
    game.assert_kv("tool.t1.output", json!(""));
    let first_lines = narrative_lines(&first).join("\n");
    assert!(
        first_lines.contains("[TOOL REQUESTED]") && first_lines.contains("t1 -> git.head_short"),
        "the request must be narrated with its allocation order: {first_lines}"
    );

    let second = game.reply("tool.call", &text("git.status|path=.|t2"));
    let calls = second["meta"]["tool_calls"]
        .as_array()
        .unwrap_or_else(|| panic!("meta.tool_calls must be an array: {second}"));
    assert_eq!(calls.len(), 2, "both calls are still pending: {second}");
    assert_eq!(calls[0]["id"], json!("t1"));
    assert_eq!(
        calls[1]["id"],
        json!("t2"),
        "the array order is the allocation order: {second}"
    );
    assert_eq!(calls[1]["args"]["path"], json!("."));
    game.assert_kv("tool.pending", json!(2));
    game.assert_kv("tool.t2.status", json!("pending"));
}

/// T2 The result direction: `tool.result` writes the row, drops the call from the queue, and the
/// receipt effect converges **in the same turn** (both lines in one narrative batch).
#[test]
fn tool_result_updates_kv_and_the_receipt_effect_fires_in_the_same_turn() {
    let mut game = drive_to_review();
    game.narrate("tool.call", &text("git.head_short"));

    let reply = game.reply("tool.result", &text("t1|git.head_short|ok|0|abc1234"));
    let lines = narrative_lines(&reply);
    assert_eq!(count_lines(&lines, "[TOOL RESULT]"), 1, "{lines:#?}");
    assert_eq!(
        count_lines(&lines, "[TOOL SETTLED]"),
        1,
        "the receipt effect must converge before the single choice is produced: {lines:#?}"
    );

    game.assert_kv("tool.t1.status", json!("ok"));
    game.assert_kv("tool.t1.code", json!(0));
    game.assert_kv("tool.t1.output", json!("abc1234"));
    game.assert_kv("tool.pending", json!(0));
    game.assert_kv("tool.last", json!("t1"));
    game.assert_kv("tool.notified.t1", json!(true));
    assert_eq!(
        reply["meta"]["tool_calls"],
        json!([]),
        "a settled call leaves an empty (not missing) queue: {reply}"
    );
}

/// T3 Replay safety: an already-applied `external_id` is an idempotent return (no state change,
/// no second `[TOOL SETTLED]`), while a conflicting terminal status is refused outright.
#[test]
fn tool_result_replay_is_idempotent_and_never_refires_the_effect() {
    let mut game = drive_to_review();
    game.narrate("tool.call", &text("git.head_short"));
    let settled =
        narrative_lines(&game.reply("tool.result", &text("t1|git.head_short|ok|0|abc1234")));
    assert_eq!(count_lines(&settled, "[TOOL SETTLED]"), 1, "{settled:#?}");

    // ① applying the same result again under a fresh external_id only fills the idempotency key.
    game.narrate("tool.result", &text("t1|git.head_short|ok|0|abc1234|ext-1"));
    game.assert_kv("tool.t1.res_ext", json!("ext-1"));

    // ② the same external_id again is a replay: no `[TOOL RESULT]`, no second `[TOOL SETTLED]`.
    let replay =
        narrative_lines(&game.reply("tool.result", &text("t1|git.head_short|ok|0|abc1234|ext-1")));
    assert_eq!(count_lines(&replay, "[REPLAY:TOOL]"), 1, "{replay:#?}");
    assert_eq!(count_lines(&replay, "[TOOL RESULT]"), 0, "{replay:#?}");
    assert_eq!(count_lines(&replay, "[TOOL SETTLED]"), 0, "{replay:#?}");
    game.assert_kv("tool.pending", json!(0));

    // ③ a different terminal status is a conflict, not an overwrite.
    let conflict = game.narrate("tool.result", &text("t1|git.head_short|err|1|boom|ext-2"));
    assert_refused(&conflict, "t1");
    game.assert_kv("tool.t1.status", json!("ok"));
    game.assert_kv("tool.t1.output", json!("abc1234"));
}

/// T4 Payload shapes are checked **before** anything is stored: every refusal below leaves the
/// allocation counter untouched, and the allowlist is the first gate. The capacity bound comes
/// from the tuning fragment (`data/texts/core.json` since r6 — the monolith is gone).
#[test]
fn tool_payloads_are_shape_checked_and_allowlisted() {
    let mut game = Game::open(&project_mod_dir());
    game.narrate(
        "event.start",
        &text("tools gate|synthetic task description"),
    );

    let long_value = "v".repeat(96);
    let long_args = format!("a1={long_value};a2={long_value};a3={long_value}");
    let long_call_id = format!("t{}", "a".repeat(32));
    let refusals: Vec<(String, &str)> = vec![
        ("gh.issue".to_owned(), "gh.issue"),
        ("Git".to_owned(), "Git"),
        ("no-dot".to_owned(), "no-dot"),
        ("git.head_short|rev=HEAD;rev=x".to_owned(), "rev="),
        ("git.head_short|abc".to_owned(), "not `k=v`"),
        ("git.head_short|a b=c".to_owned(), "must match"),
        (
            "git.head_short|a1=1;a2=2;a3=3;a4=4;a5=5;a6=6;a7=7;a8=8;a9=9".to_owned(),
            "8",
        ),
        (format!("git.head_short|{long_args}"), "192"),
        ("git.head_short||T1".to_owned(), "T1"),
        ("git.head_short||count".to_owned(), "count"),
        (format!("git.head_short||{long_call_id}"), "not usable"),
    ];
    for (payload, needle) in refusals {
        let lines = game.narrate("tool.call", &text(&payload));
        assert_refused(&lines, needle);
        game.assert_kv("tool.count", json!(null));
        game.assert_kv("tool.pending", json!(null));
    }

    // A real request allocates `t1`; re-using that id is refused (a replay needs the external_id).
    game.narrate("tool.call", &text("git.head_short"));
    game.assert_kv("tool.count", json!(1));
    assert_refused(
        &game.narrate("tool.call", &text("git.head_short||t1")),
        "t1",
    );
    game.assert_kv("tool.count", json!(1));

    // Result-side shapes: unknown call, tool mismatch, bad status, bad code, bad segment count.
    let result_refusals: Vec<(String, &str)> = vec![
        ("t99|git.head_short|ok|0|x".to_owned(), "t99"),
        ("t1|git.status|ok|0|x".to_owned(), "t1"),
        ("t1|git.head_short|maybe|0|x".to_owned(), "maybe"),
        ("t1|git.head_short|ok|1.5|x".to_owned(), "1.5"),
        ("t1|git.head_short|ok|0".to_owned(), "tool.result"),
        ("t1|git.head_short|ok|0|x|e|extra".to_owned(), "tool.result"),
    ];
    for (payload, needle) in result_refusals {
        let lines = game.narrate("tool.result", &text(&payload));
        assert_refused(&lines, needle);
        game.assert_kv("tool.t1.status", json!("pending"));
        game.assert_kv("tool.pending", json!(1));
    }

    // Capacity: the counter is a lifecycle counter, so every slot taken means the next is refused.
    let capacity = read_json(&project_mod_dir().join("data/texts/core.json"))["tuning"]
        ["tool_capacity"]
        .as_u64()
        .expect("tuning.tool_capacity must be an unsigned integer");
    let mut full = Game::open(&project_mod_dir());
    full.narrate(
        "event.start",
        &text("tools gate|synthetic task description"),
    );
    for _ in 0..capacity {
        full.narrate("tool.call", &text("git.head_short"));
    }
    full.assert_kv("tool.count", json!(capacity));
    let over = full.narrate("tool.call", &text("git.head_short"));
    assert_refused(&over, "no free tool slot");
    full.assert_kv("tool.count", json!(capacity));
}

// ───────────────────────── A · active subsystem ─────────────────────────

/// One full tool round trip: register the request, then settle it. Machine actions carry no
/// menu ticket (§8), and the call id is explicit so every assertion can name it.
fn tool_round(game: &mut Game, call: &str, result: &str) -> Vec<String> {
    game.narrate("tool.call", &text(call));
    narrative_lines(&game.reply("tool.result", &text(result)))
}

/// A1 `git.worktree_claim` folds into the **same** lane ledger as `lane.claim` (same function,
/// same keys, `since` tick included), and `git.worktree_release` frees it by lane number.
/// (r2: the claim input branch must carry the forced naming shape — an illegal name is
/// refused before any ledger write. r1: the design map is gone, so the ledger read surface
/// here is the KV itself.)
///
/// Mutation: replace `LANES.bind(..)` with a direct `lane.<k>.branch` write (dropping
/// `req.<i>.lane` / `since`) → the `lane.2.since` and `req.1.lane` assertions go red.
#[test]
fn worktree_claim_binds_the_lane_ledger_and_release_frees_it() {
    let mut game = Game::open(&project_mod_dir());
    game.narrate(
        "event.start",
        &text("subsystem gate|synthetic task description"),
    );
    game.narrate(
        "requirement.add",
        &text("synthetic requirement|synthetic description|subsys-r1"),
    );

    let claimed = tool_round(
        &mut game,
        "git.worktree_claim|lane=2;req=r1;branch=feature/aa11-feat-a|t1",
        "t1|git.worktree_claim|ok|0|lane=2;req=r1;branch=feature/aa11-feat-a;base=abc1234;clean=1;path=/tmp/wt2",
    );
    assert_eq!(
        count_lines(&claimed, "[WORKTREE CLAIMED]"),
        1,
        "{claimed:#?}"
    );
    game.assert_kv("lane.2.req", json!("r1"));
    game.assert_kv("lane.2.branch", json!("feature/aa11-feat-a"));
    game.assert_kv("lane.2.base", json!("abc1234"));
    game.assert_kv("lane.2.clean", json!("1"));
    game.assert_kv("lane.2.since", json!(1));
    game.assert_kv("req.1.lane", json!(2));
    game.assert_kv("tool.reacted.t1", json!(true));

    // A fresh external_id re-applies the result (idempotency key only) but the latch keeps the
    // domain fold one-shot: no second `[WORKTREE CLAIMED]`.
    let replayed = narrative_lines(&game.reply(
        "tool.result",
        &text("t1|git.worktree_claim|ok|0|lane=2;req=r1;branch=feature/aa11-feat-a;base=abc1234;clean=1;path=/tmp/wt2|ext-1"),
    ));
    assert_eq!(
        count_lines(&replayed, "[WORKTREE CLAIMED]"),
        0,
        "{replayed:#?}"
    );

    // An illegal branch name in the **input** is refused (r2: both entries, one judgement).
    // The tool-face refusal is a narration, not a rejection line: the ledger and the reject
    // counter stay untouched by design (账本与 reject 台账不动).
    let rejects_before = game.kv("reject.count");
    let illegal = tool_round(
        &mut game,
        "git.worktree_claim|lane=3;req=r1;branch=feature/r1|t3",
        "t3|git.worktree_claim|ok|0|lane=3;req=r1;branch=feature/r1;base=abc1234;clean=1;path=/tmp/wt3",
    );
    assert_eq!(
        count_lines(&illegal, "[WORKTREE CLAIM REFUSED]"),
        1,
        "an illegal branch must be narrated as refused: {illegal:#?}"
    );
    game.assert_kv("lane.3.branch", Json::Null);
    assert_eq!(
        game.kv("reject.count"),
        rejects_before,
        "the tool-face refusal must not move the reject ledger"
    );

    // Release: the tool carries the lane number, not the requirement index.
    let released = tool_round(
        &mut game,
        "git.worktree_release|lane=2|t2",
        "t2|git.worktree_release|ok|0|lane=2;released=1",
    );
    assert_eq!(
        count_lines(&released, "[WORKTREE RELEASED]"),
        1,
        "{released:#?}"
    );
    game.assert_kv("lane.2.req", json!(""));
    game.assert_kv("req.1.lane", json!(0));
}

/// A2 `git.lane_probe` records the truth, the conflict radar raises a **blocking** conflict on a
/// file overlap, a failed probe is only a warning, and `requirement.resolve` clears all four keys.
///
/// Mutation: replace `req_do_conflict(.., "radar")` with `ygg.kv.set(req_key(..., "conflict"), true)`
/// → `req.conflicts` and every `conflict_note` assertion go red (the count reads text, not a bool).
#[test]
fn lane_probe_records_truth_and_the_radar_blocks_overlapping_lanes() {
    let mut game = Game::open(&project_mod_dir());
    game.narrate(
        "event.start",
        &text("subsystem gate|synthetic task description"),
    );
    game.narrate("requirement.add", &text("left half|description|subsys-a"));
    game.narrate("requirement.add", &text("right half|description|subsys-b"));
    game.narrate("lane.claim", &text("1|r1|||1"));
    game.narrate("lane.claim", &text("2|r2|||1"));

    // ① the probe lands in KV, no overlap yet.
    tool_round(
        &mut game,
        "git.lane_probe|lane=1;base=abc;branch=feature/aa11-left|t1",
        "t1|git.lane_probe|ok|0|lane=1;branch=feature/aa11-left;base=abc;head=1111111;commits=2;clean=1;files=src/a.lua,src/b.lua",
    );
    game.assert_kv("lane.1.probe.commits", json!(2));
    game.assert_kv("lane.1.probe.clean", json!("1"));
    game.assert_kv("lane.1.probe.head", json!("1111111"));
    game.assert_kv("lane.1.probe.files", json!("src/a.lua,src/b.lua"));
    game.assert_kv("lane.1.probe.call", json!("t1"));
    game.assert_kv("req.1.conflict", json!(""));

    // ② a shared file raises one conflict through the very same writer as the human path.
    let radar = tool_round(
        &mut game,
        "git.lane_probe|lane=2;base=abc;branch=feature/aa11-right|t2",
        "t2|git.lane_probe|ok|0|lane=2;branch=feature/aa11-right;base=abc;head=2222222;commits=1;clean=1;files=src/b.lua,src/c.lua",
    );
    assert_eq!(count_lines(&radar, "[CONFLICT RADAR]"), 1, "{radar:#?}");
    game.assert_kv("req.1.conflict", json!("overlapping file: src/b.lua"));
    game.assert_kv("req.1.conflict_peer", json!("r2"));
    game.assert_kv("req.1.conflict_note", json!("overlapping file: src/b.lua"));
    game.assert_kv("req.1.conflict_source", json!("radar"));
    game.assert_kv("req.2.conflict_peer", json!("r1"));
    game.assert_kv("req.conflicts", json!(2));
    assert_eq!(game.phase("r1"), "blocked");
    assert_eq!(game.phase("r2"), "blocked");

    // ③ a failed probe (commits=0) is a warning, never a block: no new conflict, state stays.
    let failed = tool_round(
        &mut game,
        "git.lane_probe|lane=2;base=abc;branch=feature/aa11-right|t3",
        "t3|git.lane_probe|ok|0|lane=2;branch=feature/aa11-right;base=abc;head=2222222;commits=0;clean=1;files=",
    );
    assert_eq!(
        count_lines(&failed, "[LANE PROBE FAILED]"),
        1,
        "{failed:#?}"
    );
    game.assert_kv("req.conflicts", json!(2));
    game.assert_kv("lane.2.probe.commits", json!(0));

    // ④ `requirement.resolve` is the only way out: all four keys go back to empty.
    game.narrate("requirement.resolve", &text("r1,r2|keep_both|re-checked"));
    game.assert_kv("req.conflicts", json!(0));
    game.assert_kv("req.1.conflict", json!(""));
    game.assert_kv("req.1.conflict_peer", json!(""));
    game.assert_kv("req.1.conflict_note", json!(""));
    game.assert_kv("req.1.conflict_source", json!(""));
    game.assert_kv("req.2.conflict_source", json!(""));
    assert_eq!(game.phase("r1"), "draft");
}

/// A3 `fs.artifact_probe` and `vfs.write` feed **one** bounded ledger (`record_artifact` is the
/// single writer), and the VFS path shape is refused before anything is recorded.
///
/// Mutation: drop the `artifact.count` bound in `record_artifact` → the second/third probes
/// still record, so `artifact.count` no longer matches the number of distinct paths.
#[test]
fn artifact_probe_and_vfs_keep_one_ledger() {
    let mut game = Game::open(&project_mod_dir());
    game.narrate(
        "event.start",
        &text("subsystem gate|synthetic task description"),
    );
    let sha = "a".repeat(64);

    // ① a readable artifact lands in the ledger.
    let verified = tool_round(
        &mut game,
        "fs.artifact_probe|path=artifacts/plan.md|t1",
        &format!("t1|fs.artifact_probe|ok|0|path=artifacts/plan.md;exists=1;bytes=42;sha256={sha}"),
    );
    assert_eq!(
        count_lines(&verified, "[ARTIFACT VERIFIED]"),
        1,
        "{verified:#?}"
    );
    game.assert_kv("artifact.artifacts/plan.md.bytes", json!(42));
    game.assert_kv("artifact.artifacts/plan.md.sha256", json!(sha));
    game.assert_kv("artifact.artifacts/plan.md.source", json!("probe"));
    game.assert_kv("artifact.count", json!(1));
    game.assert_kv("artifact.seq.1", json!("artifacts/plan.md"));

    // ② an empty file is not evidence.
    let empty = tool_round(
        &mut game,
        "fs.artifact_probe|path=artifacts/empty.md|t2",
        "t2|fs.artifact_probe|ok|0|path=artifacts/empty.md;exists=1;bytes=0;sha256=",
    );
    assert_eq!(count_lines(&empty, "[ARTIFACT EMPTY]"), 1, "{empty:#?}");
    game.assert_kv("artifact.artifacts/empty.md.bytes", json!(null));
    game.assert_kv("artifact.count", json!(1));

    // ③ a path outside `artifacts/*.md` never reaches the ledger (fail-closed).
    let escaped = tool_round(
        &mut game,
        "fs.artifact_probe|path=artifacts/../x.md|t3",
        "t3|fs.artifact_probe|ok|0|path=artifacts/../x.md;exists=1;bytes=5;sha256=ffff",
    );
    assert_eq!(
        count_lines(&escaped, "[ARTIFACT PATH REFUSED]"),
        1,
        "{escaped:#?}"
    );
    game.assert_kv("artifact.count", json!(1));

    // ④ `vfs.write` always keeps the ledger, whether or not the host exposes `ygg.fs` — the
    //    narration is the only thing that differs, so both branches are asserted as a pair.
    let written = game.narrate(
        "vfs.write",
        &text(&format!("artifacts/notes.md|# notes\n{sha}")),
    );
    let joined = written.join("\n");
    let disk = joined.contains("[VFS WRITTEN]");
    let ledger_only = joined.contains("[VFS LEDGER ONLY]");
    assert!(
        disk ^ ledger_only,
        "vfs.write must report exactly one outcome: {joined}"
    );
    game.assert_kv("artifact.artifacts/notes.md.source", json!("vfs"));
    game.assert_kv("artifact.count", json!(2));
    let bytes = game
        .kv("artifact.artifacts/notes.md.bytes")
        .as_u64()
        .expect("the ledger records a byte count");
    assert_eq!(bytes as usize, "# notes\n".len() + sha.len());
    if disk {
        let vfs_sha = game
            .kv("artifact.artifacts/notes.md.sha256")
            .as_str()
            .expect("vfs.write records the sha256 computed by ygg.fs.write")
            .to_owned();
        assert_eq!(vfs_sha.len(), 64, "sha256 must be 64 hex chars: {vfs_sha}");
        assert!(
            joined.contains(&vfs_sha[..12]),
            "the digest is narrated: {joined}"
        );
    }

    // ⑤ the path shape is unconditional: a traversal is refused and nothing is recorded.
    let refused_path = game.narrate("vfs.write", &text("artifacts/../../etc/passwd|x"));
    assert_refused(&refused_path, "VFS PATH");
    game.assert_kv("artifact.artifacts/../../etc/passwd.bytes", json!(null));
    game.assert_kv("artifact.count", json!(2));

    // ⑥ re-writing the same path only refreshes the row (no duplicate slot).
    game.narrate("vfs.write", &text("artifacts/notes.md|# notes"));
    game.assert_kv("artifact.count", json!(2));
    game.assert_kv("artifact.artifacts/notes.md.bytes", json!(7));
}

/// A4 `gate.verify_suite` writes green evidence only in `review`, only when the payload says
/// `passed=1`, and only when the same gate precondition holds.
///
/// Mutation: delete the `stage() ~= STAGE.review` guard → the coordinate-stage assertion (no
/// green written before `coordinate.submit`) goes red.
#[test]
fn verify_suite_result_records_green_evidence_only_in_review() {
    let mut game = drive_to_accept(drive_to_plan_gate());
    game.click("gate.approve — ");

    // ① wrong stage: refused, no green.
    let early = tool_round(
        &mut game,
        "gate.verify_suite|mode=all|t1",
        "t1|gate.verify_suite|ok|0|passed=1;summary=52 suites green",
    );
    assert_eq!(
        count_lines(&early, "[VERIFY SUITE BLOCKED]"),
        1,
        "{early:#?}"
    );
    game.assert_kv("verify.passed", json!(null));

    game.click("coordinate.submit — ");
    game.assert_kv("verify.passed", json!(false));

    // ② honest failure: narrate it, never write green.
    let failed = tool_round(
        &mut game,
        "gate.verify_suite|mode=all|t2",
        "t2|gate.verify_suite|ok|0|passed=0;summary=3 failures",
    );
    assert_eq!(
        count_lines(&failed, "[VERIFY SUITE FAILED]"),
        1,
        "{failed:#?}"
    );
    game.assert_kv("verify.passed", json!(false));

    // ③ green: the same KV as the human path, plus the provenance key.
    let passed = tool_round(
        &mut game,
        "gate.verify_suite|mode=all|t3",
        "t3|gate.verify_suite|ok|0|passed=1;summary=52 suites green",
    );
    assert_eq!(
        count_lines(&passed, "[VERIFY SUITE PASSED]"),
        1,
        "{passed:#?}"
    );
    game.assert_kv("verify.passed", json!(true));
    game.assert_kv("verify.report", json!("52 suites green"));
    game.assert_kv("verify.source", json!("tool:gate.verify_suite"));
}

/// A5 `harness.archive_sessions` registers the settlement evidence only when `remaining=0`, and
/// the batch can then close without a separate `settlement.sessions` turn.
///
/// Mutation: drop the `settlement.sessions.destination` write from the reactor → `review.note` is
/// stopped by `gates.settlement_problem()` and the batch-close assertion goes red.
#[test]
fn archive_sessions_result_unblocks_the_settlement() {
    let mut game = drive_to_accept(drive_to_plan_gate());
    game.click("gate.approve — ");
    game.click("coordinate.submit — ");

    // ① leftovers are not evidence.
    let bad = tool_round(
        &mut game,
        "harness.archive_sessions|parent=x;batch=y|t1",
        "t1|harness.archive_sessions|ok|0|moved=1;remaining=2;dest=/tmp/trash",
    );
    assert_eq!(
        count_lines(&bad, "[SESSIONS ARCHIVE REFUSED]"),
        1,
        "{bad:#?}"
    );
    game.assert_kv("settlement.sessions.destination", json!(""));

    // ② a clean sweep registers all four keys.
    let ok = tool_round(
        &mut game,
        "harness.archive_sessions|parent=x;batch=y|t2",
        "t2|harness.archive_sessions|ok|0|moved=3;remaining=0;dest=/tmp/trash/r2",
    );
    assert_eq!(count_lines(&ok, "[SESSIONS ARCHIVED]"), 1, "{ok:#?}");
    game.assert_kv("settlement.sessions.moved", json!(3));
    game.assert_kv("settlement.sessions.remaining", json!(0));
    game.assert_kv("settlement.sessions.destination", json!("/tmp/trash/r2"));
    game.assert_kv(
        "settlement.sessions.source",
        json!("tool:harness.archive_sessions"),
    );

    // ③ the closing chain no longer needs a `settlement.sessions` turn: review.note
    // settles the batch straight away (resident default — no ending).
    game.click("verify.submit all_green=true — ");
    game.narrate("settlement.evolution", &text("|screened|c=0/0"));
    game.narrate("settlement.todo", &text("0|main session cleared"));
    let closed = game.click("review.note — ");
    let narrative = closed["narrative"].to_string();
    assert!(
        narrative.contains("[BATCH 1 SETTLED]"),
        "review.note must settle the batch: {closed}"
    );
    assert!(
        !narrative.contains("[ENDING"),
        "a batch settle must never carry the ending prefix: {closed}"
    );
    game.assert_kv("run.batch", json!(2));
    game.assert_kv("fsm.stage", json!("coordinate"));
}

struct SyncRitualRunner;

impl HostToolRunner for SyncRitualRunner {
    fn call(
        &mut self,
        tool_id: &str,
        args: &std::collections::BTreeMap<String, String>,
    ) -> Result<std::collections::BTreeMap<String, String>, String> {
        let mut out = std::collections::BTreeMap::new();
        match tool_id {
            "git.worktree_claim" => {
                let lane = args.get("lane").cloned().unwrap_or_else(|| "1".to_owned());
                let req = args.get("req").cloned().unwrap_or_else(|| "r1".to_owned());
                let branch = args
                    .get("branch")
                    .cloned()
                    .unwrap_or_else(|| format!("feature/d4e5-sync-bridge"));
                out.insert("lane".to_owned(), lane.clone());
                out.insert("req".to_owned(), req);
                out.insert("branch".to_owned(), branch);
                out.insert("base".to_owned(), "2e24c45".to_owned());
                out.insert("clean".to_owned(), "1".to_owned());
                out.insert("path".to_owned(), format!("/tmp/ygg-wt-lane-{lane}"));
                Ok(out)
            }
            "git.lane_probe" => {
                let lane = args.get("lane").cloned().unwrap_or_else(|| "1".to_owned());
                let base = args
                    .get("base")
                    .cloned()
                    .unwrap_or_else(|| "2e24c45".to_owned());
                let branch = args
                    .get("branch")
                    .cloned()
                    .unwrap_or_else(|| "feature/d4e5-sync-bridge".to_owned());
                out.insert("lane".to_owned(), lane);
                out.insert("base".to_owned(), base);
                out.insert("branch".to_owned(), branch);
                out.insert("head".to_owned(), "deadbeef1234".to_owned());
                out.insert("commits".to_owned(), "2".to_owned());
                out.insert("clean".to_owned(), "1".to_owned());
                out.insert(
                    "files".to_owned(),
                    "src/runtime/tools.lua,src/view/render.lua".to_owned(),
                );
                Ok(out)
            }
            "git.worktree_release" => {
                let lane = args.get("lane").cloned().unwrap_or_else(|| "1".to_owned());
                out.insert("lane".to_owned(), lane);
                out.insert("released".to_owned(), "1".to_owned());
                Ok(out)
            }
            "harness.archive_sessions" => {
                let batch = args
                    .get("batch")
                    .cloned()
                    .unwrap_or_else(|| "default".to_owned());
                out.insert("moved".to_owned(), "2".to_owned());
                out.insert("remaining".to_owned(), "0".to_owned());
                out.insert("dest".to_owned(), format!("/tmp/sessions-trash/{batch}"));
                Ok(out)
            }
            other => Err(format!("unexpected tool_id: {other}")),
        }
    }
}

/// A7 Synchronous `ygg.tool.call` mounted on `KernelFacade` settles all 4 inline engineering
/// rituals (`git.worktree_claim`, `git.lane_probe`, `git.worktree_release`, and
/// `harness.archive_sessions`) in **1 single turn each** without `tool.result` callbacks.
/// The branch is **pre-claimed** through `lane.claim` in the forced r2 shape — the default
/// name generator is gone, the auto-claim consumes the ledger's own branch.
#[test]
fn synchronous_tool_bridge_runs_inline_rituals() {
    let mut game = Game::open(&project_mod_dir());
    game.kernel
        .set_tool_runner(Some(Box::new(SyncRitualRunner)));

    game.narrate(
        "event.start",
        &text("sync bridge|verify 1-turn inline rituals"),
    );
    game.narrate(
        "requirement.add",
        &text("sync tool bridge|implement synchronous ygg.tool.call|sync-r1"),
    );
    game.narrate("requirement.dispatch", "0");

    // Pre-claim the lane (r2: the branch is self-reported; the auto-claim reuses it).
    // Timing: `proposal.accept` (plan) releases the lane ledger — the pre-claim must be
    // re-stated **after** the acceptance, before the gate approval that triggers the claim.

    // Submit plan proposal with inline executor registration (`exec=agent-plan`).
    submit_with_exec(&mut game, 1, "plan", "sync-p1", "agent-plan");
    game.click("proposal.accept — ");
    game.narrate(
        "lane.claim",
        &text("1|r1|feature/d4e5-sync-bridge|2e24c45|1"),
    );
    game.assert_kv("lane.1.branch", json!("feature/d4e5-sync-bridge"));

    // 1) Approving plan_gate triggers auto_dispatch -> auto_claim_worktree in the SAME turn!
    let approved_plan = game.click("gate.approve — ");
    let approved_plan_lines = narrative_lines(&approved_plan);
    assert_eq!(
        count_lines(&approved_plan_lines, "[TOOL SETTLED]"),
        1,
        "{approved_plan_lines:#?}"
    );
    assert_eq!(
        count_lines(&approved_plan_lines, "[WORKTREE CLAIMED]"),
        1,
        "{approved_plan_lines:#?}"
    );
    game.assert_kv("lane.1.req", json!("r1"));
    game.assert_kv("lane.1.branch", json!("feature/d4e5-sync-bridge"));
    game.assert_kv("lane.1.base", json!("2e24c45"));
    game.assert_kv("lane.1.path", json!("/tmp/ygg-wt-lane-1"));
    game.assert_kv("tool.settled_count", json!(1));
    game.assert_kv("tool.pending", json!(null));

    // 2) Submitting development proposal triggers auto_probe_lane in the SAME turn!
    let dev_prop_lines = game.narrate(
        "proposal.submit",
        &text(
            "r1|1|development|dev done|artifacts/r1-development.md|src/runtime/tools.lua|exec=agent-plan",
        ),
    );
    assert_eq!(
        count_lines(&dev_prop_lines, "[TOOL SETTLED]"),
        1,
        "{dev_prop_lines:#?}"
    );
    assert_eq!(
        count_lines(&dev_prop_lines, "[LANE PROBE]"),
        1,
        "{dev_prop_lines:#?}"
    );
    game.assert_kv("lane.1.probe.commits", json!(2));
    game.assert_kv("lane.1.probe.clean", json!("1"));
    game.assert_kv("lane.1.probe.head", json!("deadbeef1234"));
    game.assert_kv("tool.settled_count", json!(2));

    game.click("proposal.accept — ");

    // 3) Approving accept gate triggers auto_release_worktree in the SAME turn!
    let approved_accept = game.click("gate.approve — ");
    let approved_accept_lines = narrative_lines(&approved_accept);
    assert_eq!(
        count_lines(&approved_accept_lines, "[TOOL SETTLED]"),
        1,
        "{approved_accept_lines:#?}"
    );
    assert_eq!(
        count_lines(&approved_accept_lines, "[WORKTREE RELEASED]"),
        1,
        "{approved_accept_lines:#?}"
    );
    game.assert_kv("lane.1.path", json!(""));
    game.assert_kv("lane.1.req", json!(""));
    game.assert_kv("tool.settled_count", json!(3));

    // 4) r1 rev1 fail-closed contract: with run.id/run.parent unset, registering
    //    settlement.evolution WITHOUT a `sessions=` segment must NOT auto-archive —
    //    fabricating parent="default" could rename the main session into the trash
    //    (the host selects files by substring match). The evolution turn narrates a
    //    readable [SESSIONS UNSETTLED] hint instead, writes no forged remaining=0
    //    (the -1 from reset_settlement survives), and stays at 3 settled rituals.
    game.click("coordinate.submit — ");
    game.click("verify.submit all_green=true — ");
    let evo_lines = game.narrate(
        "settlement.evolution",
        &text("|screened|c=0/0|todo=0/verified"),
    );
    assert_eq!(
        count_lines(&evo_lines, "[TOOL SETTLED]"),
        0,
        "{evo_lines:#?}"
    );
    assert_eq!(
        count_lines(&evo_lines, "[SESSIONS ARCHIVED]"),
        0,
        "{evo_lines:#?}"
    );
    assert_eq!(
        count_lines(&evo_lines, "[SESSIONS UNSETTLED]"),
        1,
        "{evo_lines:#?}"
    );
    game.assert_kv("settlement.sessions.remaining", json!(-1));
    game.assert_kv("tool.settled_count", json!(3));

    // The blocked review.note narrates the readable hint instead of archiving…
    let blocked = game.click("review.note — ");
    let blocked_text = blocked["narrative"].to_string();
    assert!(
        blocked_text.contains("[SESSIONS UNSETTLED]"),
        "a blocked review.note must explain the safe cleanup path: {blocked}"
    );
    assert!(
        !blocked_text.contains("[BATCH 1 SETTLED]"),
        "review.note must stay blocked while sessions are unsettled: {blocked}"
    );

    // …and an explicit human settlement (the safe path) unblocks the batch close.
    game.narrate(
        "settlement.sessions",
        &text("2|0|/tmp/sessions-trash/sem-explicit"),
    );
    let closed = game.click("review.note — ");
    let closed_text = closed["narrative"].to_string();
    assert!(
        closed_text.contains("[BATCH 1 SETTLED]"),
        "review.note must settle the batch after explicit session registration: {closed}"
    );
    assert!(
        !closed_text.contains("[ENDING"),
        "a batch settle must never carry the ending prefix: {closed}"
    );
    game.assert_kv("run.batch", json!(2));
    game.assert_kv("fsm.stage", json!("coordinate"));
}

// ───────────────────────── r13 · fuel autoscale + requirement list pagination ─────────────────────────

/// r13 A: the dispatch fuel budget autoscales with the canonical state size —
/// `clamp(per_entry × kv_len, floor, cap)` — while an explicit `YGG_FUEL_BUDGET` wins
/// **exactly** (the escape hatch is not clipped by the floor or the cap); a missing,
/// zero, or unparseable env falls back to the autoscale.
#[test]
fn fuel_budget_autoscales_with_state_size_and_env_wins() {
    // Autoscale: the pure function is the single source (host-injected only).
    assert_eq!(
        auto_budget(0),
        DEFAULT_FUEL_BUDGET,
        "kv_len 0 sits on the floor"
    );
    assert_eq!(
        auto_budget(60),
        FUEL_AUTOSCALE_PER_ENTRY * 60,
        "the scale is exactly per_entry × kv_len inside the window"
    );
    assert_eq!(
        auto_budget(10_000),
        FUEL_AUTOSCALE_CAP,
        "huge states clamp to the 4M cap"
    );
    assert_eq!(
        auto_budget(usize::MAX),
        FUEL_AUTOSCALE_CAP,
        "saturating multiplication must not panic and still clamp"
    );

    // Resolution: explicit env is byte-exact and uncapped (both escape directions).
    assert_eq!(resolve_budget(Some("12345"), 100), 12_345);
    assert_eq!(
        resolve_budget(Some("1"), 100_000),
        1,
        "below the floor: allowed"
    );
    assert_eq!(
        resolve_budget(Some("99999999999"), 0),
        99_999_999_999,
        "above the cap: allowed (escape hatch semantics)"
    );
    assert_eq!(
        resolve_budget(Some(" 4096 "), 100),
        4_096,
        "the raw is trimmed"
    );

    // Fallback: missing / zero / unparseable → the autoscale.
    assert_eq!(resolve_budget(None, 0), DEFAULT_FUEL_BUDGET);
    assert_eq!(resolve_budget(Some("0"), 100), auto_budget(100));
    assert_eq!(resolve_budget(Some("abc"), 100), auto_budget(100));
    assert_eq!(resolve_budget(Some(""), 100), auto_budget(100));
}

/// r13 B: `requirement.list` is a **paged** view by default — open requirements first
/// (canonical order), page width 12, the rest folded into a hint line — while
/// `requirement.list full` opts into the full render (the low-frequency operational view).
#[test]
fn requirement_list_pages_open_first_and_full_opt_in() {
    let mut game = Game::open(&project_mod_dir());
    game.narrate(
        "event.start",
        &text("pagination|synthetic task description"),
    );

    // r1 goes the full happy path to `done` (the only closed row).
    game.narrate(
        "requirement.add",
        &text("synthetic requirement|synthetic description|page-r1"),
    );
    game.narrate("requirement.dispatch", "0");
    submit(&mut game, 1, "plan", "page-p1");
    game.click("proposal.accept — ");
    game.click("gate.approve — ");
    submit(&mut game, 1, "development", "page-p2");
    game.click("proposal.accept — ");
    game.click("gate.approve — ");
    game.assert_kv("req.1.status", json!("done"));

    // r2..r13: twelve open drafts (total 13 requirements).
    for i in 2..=13 {
        game.narrate(
            "requirement.add",
            &text(&format!(
                "page requirement {i}|synthetic description|page-r{i}"
            )),
        );
    }

    // Default view: exactly the 12 **open** rows — the closed r1 is folded out (open-first
    // is the operationally correct page order: a canonical-prefix page would be all-done).
    let default_view = game.narrate("requirement.list", "0");
    let rows = Game::req_rows(&default_view);
    assert_eq!(
        rows.len(),
        12,
        "the default view must page at 12 rows: {default_view:#?}"
    );
    assert!(
        !rows.iter().any(|(id, _)| id == "r1"),
        "the closed r1 must not occupy the open-first page: {rows:?}"
    );
    assert!(
        default_view
            .iter()
            .any(|line| line.starts_with("[REQ GRAPH]")),
        "the page keeps the graph header: {default_view:#?}"
    );

    // The full opt-in renders everything: 13 rows with the closed r1 in the closed tail.
    let full_view = game.narrate("requirement.list", &text("full"));
    let rows = Game::req_rows(&full_view);
    assert_eq!(
        rows.len(),
        13,
        "the full view must render every requirement: {full_view:#?}"
    );
    assert_eq!(
        rows.last().map(|(id, _)| id.as_str()),
        Some("r1"),
        "the single closed row closes the full page (open first, closed newest-to-oldest): {rows:?}"
    );
    assert!(
        full_view.iter().any(|line| line.starts_with("[REQ GRAPH]")),
        "the full view keeps the graph header: {full_view:#?}"
    );
}

/// r1: the summary projection carries the additive `play` key (the `play_mode()` mirror)
/// beside the r9 resident trio — the auto-collect watchdog and the opt-in wake-up read
/// `meta.summary` without any host-side parsing (P11: the host never interprets it).
#[test]
fn summary_projection_carries_play_and_resident_trio() {
    let mut game = Game::open(&project_mod_dir());

    // Fresh run: the boot lands in `coordinate`, but the charter gate keeps the pump
    // NOT drained — `gate_problem() != nil` (no charter payload, no requirements).
    let summary = game.reply("agent.summary", "0")["meta"]["summary"].clone();
    assert_eq!(
        summary["play"],
        json!("guide"),
        "play.mode default is guide and must be projected: {summary}"
    );
    assert_eq!(summary["lifecycle"], json!("resident"));
    assert_eq!(summary["batch"], json!(1));
    assert_eq!(summary["batchDrained"], json!(false));

    // `task.intake` moves the run to `coordinate`; with no pending gate and no armed
    // retirement the pump is drained (waiting for new `--requirement` feed).
    // r2 rev2: chartering the run (boot payload) alone is still not enough — the open
    // (draft) requirement keeps `req_all_done()` false, so the run stays not-drained.
    game.narrate("event.start", &text("watchdog|synthetic task description"));
    game.narrate(
        "requirement.add",
        &text("watchdog req|synthetic description|watchdog-r1"),
    );
    let summary = game.reply("agent.summary", "0")["meta"]["summary"].clone();
    assert_eq!(
        summary["batchDrained"],
        json!(false),
        "coordinate ∧ charter ∧ open requirement → still not drained: {summary}"
    );
}
