//! agent-seed semantics gate: **decidable** assertions for the requirement-level state machine
//! (5 stored states + `plan_ok`-derived display names), the turn-cut conventions, and the
//! two-layer node map (the behavioural acceptance surface required by AGENTS.md §1.9).
//!
//! Why this layer exists: the all-1 `playtest` walk only proves the happy path reaches
//! `[ENDING` — it never takes a **branch** such as `gate.reject` / `requirement.amend` /
//! `proposal.reject` / a stale gate, and the turn-cut mechanics (inline `exec=`, the
//! `draft -> planning` merge, the reactive auto-dispatch effect, the settlement merge) are
//! likewise invisible to it. This file turns those into assertions:
//!
//! - state assertions read **`kv.entries` of the `save` receipt** (`req.*` is machine state,
//!   not prose);
//! - display assertions read the **derived display name** from `requirement.list`
//!   (`[ready]` / `[plan_gate]` / …);
//! - map assertions read the **map window** from `map.show` (operation view vs design view);
//! - every human action goes through **guide's `ygg.choice_selected`** (the ticket is only
//!   stamped there, §8).
//!
//! Boundaries (same stance as AGENTS.md §1.7 — do not add exits here):
//! - only the public load surface (`ModBundle` / `KernelFacade` / dispatch / save-load):
//!   no host CLI changes, no production-code changes, no side-channel scripts;
//! - **no run directory is read or written**: legacy compatibility uses an **in-memory save
//!   fixture** (drop a key inside the `save` receipt, then `restore`), never `autosave.json`
//!   or `events.jsonl`;
//! - the legacy case only proves "the current reducer can read a save with a missing key";
//!   it does not build a historical engine or forge an event log.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::{json, Value as Json};

use ygg_sandbox::driver::KernelFacade;
use ygg_sandbox::modload::ModBundle;
use ygg_sandbox::playtest::{playtest, PlaytestOptions, Reached};

// ───────────────────────── locating ─────────────────────────

/// Repository root (`CARGO_MANIFEST_DIR` is `crates/ygg-sandbox`; two levels up).
fn repo_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The instance this project actually loads (`data/scenes.json` and `data/playbook.json`
/// are this project's field surface).
fn project_mod_dir() -> PathBuf {
    repo_dir().join(".ygg/mods/agent-seed")
}

/// The seed template that ships to users (skeleton is byte-identical; `data/` differs).
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
        "task.intake",
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

/// Submit an `r1` proposal: `req|rev|stage|summary|artifacts|changes|ext` (changes empty).
fn submit(game: &mut Game, rev: u32, stage: &str, ext: &str) {
    let payload = text(&format!(
        "r1|{rev}|{stage}|{stage} proposal summary|artifacts/{stage}.md||{ext}"
    ));
    game.narrate("proposal.submit", &payload);
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
fn drive_to_plan_gate() -> Game {
    drive_to_plan_gate_in(&project_mod_dir())
}

fn drive_to_plan_gate_in(dir: &Path) -> Game {
    let mut game = Game::open(dir);
    start_requirement(&mut game);
    submit_with_exec(&mut game, 1, "plan", "semantics-p1", "agent-1");
    game.assert_kv("settings.executor.r1.plan", json!("agent-1"));
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
    submit_with_exec(&mut game, 1, "development", "semantics-p2", "agent-1");
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
    submit_with_exec(&mut game, 1, "development", ext, "agent-1");
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

// ───────────────────────── ⑦ all-1 walk (both mods) ─────────────────────────

/// ⑦ The all-1 walk of both mods must reach `[ENDING` with empty `warnings` / `errors`.
///
/// This is a **supplement**, never a replacement: all-1 only covers the happy path, while
/// the semantic branches are covered by the numbered cases below.
#[test]
fn playtest_reaches_ending_on_both_mods() {
    for dir in [project_mod_dir(), seed_mod_dir()] {
        let report = playtest(&dir, &PlaytestOptions::default());
        assert!(
            report.ok,
            "{} all-1 walk failed: reached={:?} errors={:?}",
            dir.display(),
            report.reached,
            report.errors
        );
        assert_eq!(
            report.reached,
            Some(Reached::Ending),
            "{} must reach [ENDING",
            dir.display()
        );
        assert!(
            report.warnings.is_empty(),
            "{} must not warn: {:?}",
            dir.display(),
            report.warnings
        );
        assert!(
            report.errors.is_empty(),
            "{} must not error: {:?}",
            dir.display(),
            report.errors
        );
        // The settlement merge only pays off if the whole review stage collapses: start,
        // intake, coordinate.submit, verify, evolution (with sessions= + todo=), review.note.
        assert!(
            report.steps <= 8,
            "{} needed {} steps — the review stage must settle in one evolution turn",
            dir.display(),
            report.steps
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
/// cleanups (`sessions=` / `todo=`), so `review.note` can close the run right after it. The
/// standalone actions stay usable for step-by-step settlement.
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

    // `review.note` now closes the run (all three settlement tasks are registered).
    let closed = game.click("review.note — ");
    let narrative = closed["narrative"].to_string();
    assert!(
        narrative.contains("[ENDING"),
        "review.note must close the run: {closed}"
    );
    game.assert_kv("ending", json!("done"));
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

// ───────────────────────── ⑥ two-layer map contracts ─────────────────────────

/// Take the **map window** from the `map.show` receipt: from the map head line to the map
/// foot line (ignoring the other rendered lines in the same receipt).
#[track_caller]
fn map_window(lines: &[String], head: &str, foot: &str) -> Vec<String> {
    let start = lines
        .iter()
        .position(|line| line.starts_with(head))
        .unwrap_or_else(|| panic!("missing map head `{head}`: {lines:#?}"));
    let end = lines
        .iter()
        .position(|line| line.starts_with(foot))
        .unwrap_or_else(|| panic!("missing map foot `{foot}`: {lines:#?}"));
    assert!(
        start < end,
        "the map head must precede the foot: {lines:#?}"
    );
    lines[start..=end].to_vec()
}

/// Split a map row into "left column (precondition/current node) | action area" and
/// "right column (target/result)".
#[track_caller]
fn split_row(row: &str) -> (&str, &str) {
    row.split_once("──▶ ")
        .unwrap_or_else(|| panic!("map row lacks the `──▶` separator: {row}"))
}

/// A bracketed label in the left column (`[planning]` / `[draft/ready conflict pair]`).
#[track_caller]
fn row_label(left: &str) -> &str {
    let open = left
        .find('[')
        .unwrap_or_else(|| panic!("map row left column lacks a label: {left}"));
    let close = left[open..]
        .find(']')
        .unwrap_or_else(|| panic!("map row left column label is not closed: {left}"))
        + open;
    &left[open..=close]
}

/// Operation view (`map.show`, default): it only promises "selectable" and says
/// "condition not shown" when the condition is unknown.
///
/// The counter-examples are the two wording regressions from an earlier round: writing
/// "executable" for "selectable" (the render layer cannot see the choice-side precondition)
/// and "no extra condition" for "condition not shown" (that would vouch for the candidate).
#[track_caller]
fn assert_map_local(lines: &[String]) {
    let window = map_window(lines, "[CURRENT NODE]", "(full design map");
    let joined = window.join("\n");
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

/// Design view (`map.show full`):
/// - both requirement-level hops (`planning` / `developing`) must show `proposal.accept`
///   between the proposal and the gate;
/// - the bypass left column is a **mutable precondition**, the right column is the
///   **resulting terminal state**.
#[track_caller]
fn assert_map_full(lines: &[String]) {
    let window = map_window(lines, "[MAP]", "always-on actions");

    // Two hops: the middle `proposal.accept` hop must not be lost.
    let hop = "proposal → proposal.accept → gate.approve";
    let hop_rows: Vec<&String> = window.iter().filter(|row| row.contains(hop)).collect();
    assert_eq!(
        hop_rows.len(),
        2,
        "the pipeline must show proposal.accept on both the planning and the developing hop: {window:#?}"
    );
    let planning = hop_rows
        .iter()
        .find(|row| row_label(split_row(row).0) == "[planning]")
        .unwrap_or_else(|| panic!("missing the `[planning]` hop: {window:#?}"));
    assert!(
        split_row(planning)
            .1
            .trim_start()
            .starts_with("[planning·plan_ok]"),
        "`[planning]` must lead to `[planning·plan_ok]` (approved first, then develop): {planning}"
    );
    let developing = hop_rows
        .iter()
        .find(|row| row_label(split_row(row).0) == "[developing]")
        .unwrap_or_else(|| panic!("missing the `[developing]` hop: {window:#?}"));
    assert!(
        split_row(developing).1.trim_start().starts_with("[done]"),
        "`[developing]` must lead to `[done]`: {developing}"
    );

    // Bypass direction: precondition → resulting terminal state.
    const BYPASS_ACTIONS: [&str; 3] = [
        "requirement.split",
        "requirement.merge",
        "requirement.resolve keep_first",
    ];
    const PRECONDITIONS: [&str; 4] = ["draft", "analyzing", "planning", "ready"];
    const TERMINALS: [&str; 3] = ["split", "merged", "dropped"];
    const TERMINAL_TARGETS: [&str; 3] = ["[split]", "[merged]", "[dropped]"];

    let bypass_rows: Vec<&String> = window
        .iter()
        .filter(|row| BYPASS_ACTIONS.iter().any(|action| row.contains(action)))
        .collect();
    assert_eq!(
        bypass_rows.len(),
        BYPASS_ACTIONS.len(),
        "there must be exactly three bypass rows: {window:#?}"
    );
    for row in bypass_rows {
        let (left, right) = split_row(row);
        let label = row_label(left);
        assert!(
            PRECONDITIONS.iter().any(|state| label.contains(state)),
            "the bypass left column must name a precondition (not a terminal state): {row}"
        );
        for terminal in TERMINALS {
            assert!(
                !label.contains(terminal),
                "the bypass left column must not be the terminal `{terminal}` (direction reversed): {row}"
            );
        }
        let target = right.trim_start();
        assert!(
            TERMINAL_TARGETS.iter().any(|end| target.starts_with(end)),
            "the bypass right column must be the resulting terminal state: {row}"
        );
    }
}

/// ⑥ Both map contracts (both mods: the `map.show` wording lives in the scenes resource and
/// can drift on either side).
#[test]
fn map_views_keep_operation_and_design_contracts() {
    for dir in [project_mod_dir(), seed_mod_dir()] {
        let mut game = Game::open(&dir);
        assert_map_local(&game.narrate("map.show", "0"));

        // Design view in the planning situation.
        let mut game = drive_to_plan_gate_in(&dir);
        assert_eq!(game.phase("r1"), "plan_gate");
        assert_map_full(&game.narrate("map.show", &text("full")));

        // Design view in the developing situation (both hops on the same map).
        game.click("gate.approve — ");
        assert_eq!(game.phase("r1"), "developing");
        assert_map_full(&game.narrate("map.show", &text("full")));
    }
}
