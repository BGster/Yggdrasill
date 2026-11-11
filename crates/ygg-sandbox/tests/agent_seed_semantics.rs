//! pi-agent-seed semantics gate: **decidable** assertions for the requirement-level state machine
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
use ygg_sandbox::HostToolRunner;

// ───────────────────────── locating ─────────────────────────

/// Repository root (`CARGO_MANIFEST_DIR` is `crates/ygg-sandbox`; two levels up).
fn repo_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The instance this project actually loads (`data/texts/` and `data/playbook/` fragments
/// are this project's field surface).
fn project_mod_dir() -> PathBuf {
    repo_dir().join(".ygg/mods/pi-agent-seed")
}

/// The seed template that ships to users (skeleton is byte-identical; `data/` differs).
fn seed_mod_dir() -> PathBuf {
    repo_dir().join("packages/mods/pi-agent-seed")
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

// ───────────────────────── tool bridge ─────────────────────────

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

/// A mod resource read straight from disk (the declaration surface is a file, not Lua state).
fn read_json(path: &Path) -> Json {
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|err| panic!("parse {}: {err}", path.display()))
}

/// The aggregated scene-texts surface: the `data/texts/*.json` fragments merged with the same
/// rule as the mod's `mod.lua` assembly (core groups as-is; the six `beats` fragments
/// concatenated in filename order — the beats key sets are disjoint, so merge order cannot
/// shadow). The result is shape-identical to the former single `data/scenes.json`.
fn read_texts(dir: &Path) -> Json {
    let texts_dir = dir.join("data/texts");
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&texts_dir)
        .unwrap_or_else(|err| panic!("read {}: {err}", texts_dir.display()))
        .map(|entry| entry.expect("read_dir entry").path())
        .collect();
    files.sort();
    let mut texts = Json::Object(serde_json::Map::new());
    for path in files {
        let fragment = read_json(&path);
        for (group, value) in fragment.as_object().expect("fragment must be an object") {
            if group == "beats" {
                for (key, item) in value.as_object().expect("beats must be an object") {
                    texts["beats"][key] = item.clone();
                }
            } else {
                texts[group] = value.clone();
            }
        }
    }
    texts
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

/// T1 The request direction: a `tool.call` publishes **one dense snapshot** (`meta.tool_calls`,
/// allocation order, `args` omitted when empty) and one self-describing KV row. It never
/// publishes `meta.summary` — the two keys are independent slots of the same `BTreeMap`.
#[test]
fn tool_call_publishes_a_dense_pending_snapshot_and_kv_rows() {
    let mut game = Game::open(&project_mod_dir());
    game.narrate(
        "task.intake",
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
/// allocation counter untouched, and the allowlist is the first gate.
#[test]
fn tool_payloads_are_shape_checked_and_allowlisted() {
    let mut game = Game::open(&project_mod_dir());
    game.narrate(
        "task.intake",
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
    let capacity = read_texts(&project_mod_dir())["tuning"]
        ["tool_capacity"]
        .as_u64()
        .expect("tuning.tool_capacity must be an unsigned integer");
    let mut full = Game::open(&project_mod_dir());
    full.narrate(
        "task.intake",
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

/// T5 The declaration surface on both sides: `pack.json` `tools` + `data/config.json`
/// `allowedTools` + `data/texts/` beats. Pure disk reads (no kernel needed) — the two mods
/// ship the same mechanism, so any drift here is two different capability sets.
#[test]
fn tool_declarations_stay_consistent_across_both_mods() {
    let project = project_mod_dir();
    let seed = seed_mod_dir();
    let project_pack = read_json(&project.join("pack.json"));
    let seed_pack = read_json(&seed.join("pack.json"));

    // ① the mechanism surface is identical on both sides (`tools` is not an instance value).
    assert_eq!(
        project_pack["tools"], seed_pack["tools"],
        "both mods must declare the same tools"
    );
    let tools = project_pack["tools"]
        .as_object()
        .expect("pack.json tools must be an object");
    assert!(
        !tools.is_empty(),
        "the tool bridge needs at least one declared tool"
    );
    let mut declared: Vec<String> = Vec::new();
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
        // `readOnly` is a declared fact, not a policy: the active subsystem needs two writer
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
        assert!(
            spec["maxOutputBytes"].as_u64().unwrap_or(0) >= 1,
            "{id}: maxOutputBytes must be at least 1"
        );
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

    // ②③ `allowedTools` is the request-time gate: a superset of `tools`, shape-checked, no dups.
    for dir in [&project, &seed] {
        let config = read_json(&dir.join("data/config.json"));
        let allowed = config["allowedTools"]
            .as_array()
            .unwrap_or_else(|| panic!("{}: allowedTools must be an array", dir.display()));
        let mut seen: Vec<String> = Vec::new();
        for item in allowed {
            let id = item
                .as_str()
                .unwrap_or_else(|| panic!("{}: allowedTools items must be strings", dir.display()));
            assert!(
                tool_id_shape_ok(id),
                "{}: allowed tool id `{id}` must be <namespace>.<name>",
                dir.display()
            );
            assert!(
                !seen.contains(&id.to_owned()),
                "{}: allowed tool `{id}` is listed twice",
                dir.display()
            );
            seen.push(id.to_owned());
        }
        for id in &declared {
            assert!(
                seen.contains(id),
                "{}: declared tool `{id}` is missing from allowedTools (the host refuses at load)",
                dir.display()
            );
        }
        seen.sort();
        let mut declared_sorted = declared.clone();
        declared_sorted.sort();
        assert_eq!(
            seen,
            declared_sorted,
            "{}: allowedTools and pack.json tools must be the same set",
            dir.display()
        );
    }

    // ④ both scene resources carry every tool beat (the skeleton is shared, `data/` is not).
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
    let mut output_cap = 0;
    for dir in [&project, &seed] {
        let scenes = read_texts(dir);
        for key in beats {
            assert!(
                scenes["beats"].get(key).and_then(|value| value.as_str()).is_some(),
                "{}: data/texts/ beats is missing `{key}` (the shared src/ would render a raw placeholder)",
                dir.display()
            );
        }
        for spec in tools.values() {
            output_cap = output_cap.max(spec["maxOutputBytes"].as_u64().unwrap_or(0));
        }
        // ⑤ the Mod's own output cap must not be tighter than the host's per-tool cap.
        let truncate_at = scenes["tuning"]["tool_output_max"]
            .as_u64()
            .unwrap_or_else(|| {
                panic!(
                    "{}: tuning.tool_output_max must be an unsigned integer",
                    dir.display()
                )
            });
        assert!(
            truncate_at >= output_cap,
            "{}: tuning.tool_output_max ({truncate_at}) is below the largest maxOutputBytes ({output_cap})",
            dir.display()
        );
    }

    // ⑥⑦ the module is declared on both sides, in the same position, and the inbox route matches
    // the wire the handlers produce.
    assert_eq!(
        project_pack["code"]["modules"], seed_pack["code"]["modules"],
        "code.modules is the skeleton surface and must stay identical"
    );
    let modules = project_pack["code"]["modules"]
        .as_array()
        .expect("code.modules must be an array");
    assert!(
        modules.iter().any(|item| item == "src/runtime/tools.lua"),
        "src/runtime/tools.lua must be declared in code.modules: {modules:?}"
    );
    for pack in [&project_pack, &seed_pack] {
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
    }
}

// ───────────────────────── A · active subsystem (r2) ─────────────────────────

/// One full tool round trip: register the request, then settle it. Machine actions carry no
/// menu ticket (§8), and the call id is explicit so every assertion can name it.
fn tool_round(game: &mut Game, call: &str, result: &str) -> Vec<String> {
    game.narrate("tool.call", &text(call));
    narrative_lines(&game.reply("tool.result", &text(result)))
}

/// Every `TEXTS.<group>.<name>` reference in the Mod source (static keys only: the dynamic
/// `TEXTS.beats[expr]` form is skipped). One side suffices as input — the skeleton is
/// byte-identical on both, and the caller re-checks the keys against **both** scene files.
fn text_references(dir: &Path) -> Vec<(String, String)> {
    fn walk(path: &Path, out: &mut Vec<PathBuf>) {
        if path.is_file() {
            out.push(path.to_owned());
            return;
        }
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            walk(&entry.path(), out);
        }
    }

    let mut files = Vec::new();
    walk(&dir.join("mod.lua"), &mut files);
    walk(&dir.join("src"), &mut files);

    let mut refs = Vec::new();
    for file in files {
        let Ok(source) = std::fs::read_to_string(&file) else {
            continue;
        };
        let mut rest = source.as_str();
        while let Some(at) = rest.find("TEXTS.") {
            rest = &rest[at + "TEXTS.".len()..];
            let group_end = rest
                .find(|c: char| !(c.is_ascii_lowercase() || c == '_'))
                .unwrap_or(rest.len());
            let group = &rest[..group_end];
            if !matches!(group, "beats" | "prompts" | "options" | "endings") {
                continue;
            }
            let Some(tail) = rest[group_end..].strip_prefix('.') else {
                continue;
            };
            let name_end = tail
                .find(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'))
                .unwrap_or(tail.len());
            let name = &tail[..name_end];
            if !name.is_empty() {
                refs.push((group.to_owned(), name.to_owned()));
            }
        }
    }
    refs
}

/// A1 `git.worktree_claim` folds into the **same** lane ledger as `lane.claim` (same function,
/// same keys, `since` tick included), and `git.worktree_release` frees it by lane number.
///
/// Mutation: replace `LANES.bind(..)` with a direct `lane.<k>.branch` write (dropping
/// `req.<i>.lane` / `since`) → the `lane.2.since` and `req.1.lane` assertions go red.
#[test]
fn worktree_claim_binds_the_lane_ledger_and_release_frees_it() {
    let mut game = Game::open(&project_mod_dir());
    game.narrate(
        "task.intake",
        &text("subsystem gate|synthetic task description"),
    );
    game.narrate(
        "requirement.add",
        &text("synthetic requirement|synthetic description|subsys-r1"),
    );

    let claimed = tool_round(
        &mut game,
        "git.worktree_claim|lane=2;req=r1;branch=feat-a|t1",
        "t1|git.worktree_claim|ok|0|lane=2;req=r1;branch=feat-a;base=abc1234;clean=1;path=/tmp/wt2",
    );
    assert_eq!(
        count_lines(&claimed, "[WORKTREE CLAIMED]"),
        1,
        "{claimed:#?}"
    );
    game.assert_kv("lane.2.req", json!("r1"));
    game.assert_kv("lane.2.branch", json!("feat-a"));
    game.assert_kv("lane.2.base", json!("abc1234"));
    game.assert_kv("lane.2.clean", json!("1"));
    game.assert_kv("lane.2.since", json!(1));
    game.assert_kv("req.1.lane", json!(2));
    game.assert_kv("tool.reacted.t1", json!(true));
    // The ledger is a read surface too: the design map prints the ledger row.
    assert!(
        game.narrate("map.show", &text("full"))
            .join("\n")
            .contains("lane-2 ← r1"),
        "map.show full must show the claimed lane"
    );

    // A fresh external_id re-applies the result (idempotency key only) but the latch keeps the
    // domain fold one-shot: no second `[WORKTREE CLAIMED]`.
    let replayed = narrative_lines(&game.reply(
        "tool.result",
        &text("t1|git.worktree_claim|ok|0|lane=2;req=r1;branch=feat-a;base=abc1234;clean=1;path=/tmp/wt2|ext-1"),
    ));
    assert_eq!(
        count_lines(&replayed, "[WORKTREE CLAIMED]"),
        0,
        "{replayed:#?}"
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
        "task.intake",
        &text("subsystem gate|synthetic task description"),
    );
    game.narrate("requirement.add", &text("left half|description|subsys-a"));
    game.narrate("requirement.add", &text("right half|description|subsys-b"));
    game.narrate("lane.claim", &text("1|r1|||1"));
    game.narrate("lane.claim", &text("2|r2|||1"));

    // ① the probe lands in KV, no overlap yet.
    tool_round(
        &mut game,
        "git.lane_probe|lane=1;base=abc;branch=feat-a|t1",
        "t1|git.lane_probe|ok|0|lane=1;branch=feat-a;base=abc;head=1111111;commits=2;clean=1;files=src/a.lua,src/b.lua",
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
        "git.lane_probe|lane=2;base=abc;branch=feat-b|t2",
        "t2|git.lane_probe|ok|0|lane=2;branch=feat-b;base=abc;head=2222222;commits=1;clean=1;files=src/b.lua,src/c.lua",
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

    // The radar rides the very counter `gates.gate_problem()` reads, so it is a hard block with
    // no extra wiring; the summary is the read surface the host seals.
    let summary = game.reply("agent.summary", "0")["meta"]["summary"].clone();
    let conflicts = summary["conflicts"].as_array().expect("summary.conflicts");
    assert_eq!(conflicts.len(), 2, "{summary}");
    assert_eq!(conflicts[0]["peer"], json!("r2"));
    assert_eq!(conflicts[0]["note"], json!("overlapping file: src/b.lua"));

    // ③ a failed probe (commits=0) is a warning, never a block: no new conflict, state stays.
    let failed = tool_round(
        &mut game,
        "git.lane_probe|lane=2;base=abc;branch=feat-b|t3",
        "t3|git.lane_probe|ok|0|lane=2;branch=feat-b;base=abc;head=2222222;commits=0;clean=1;files=",
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

/// A2b `agent.summary` carries two **structured** detail lists next to the scalar counts:
/// `jobs` (every requirement occupying an analysis / plan / development slot) and `gates`
/// (every requirement sitting on a plan / accept gate). A scheduler reads them instead of
/// scraping narrative, so the keys must be exactly the documented set — and they must derive
/// from the same KV facts the scalar fields do (including the lane ledger's own
/// branch / base bytes).
///
/// Mutation: route `jobs` / `gates` through `summary_int` (a count) → the `as_array()`
/// unwraps below go red; drop the `SUMMARY_JOB_STAGES` filter in favour of `req_job(i) ~= ""`
/// and the `done` / gate-released rows keep stale job entries, so `jobs.is_empty()` reds.
#[test]
fn summary_exposes_structured_jobs_and_gates() {
    let mut game = Game::open(&project_mod_dir());
    game.narrate(
        "task.intake",
        &text("subsystem gate|synthetic task description"),
    );
    game.narrate(
        "requirement.add",
        &text("synthetic requirement|synthetic description|semantics-r1"),
    );
    // Pre-claim a lane **with** branch/base: `jobs[].branch` / `jobs[].base` must be the lane
    // ledger's own bytes, not something reconstructed from the requirement row.
    game.narrate("lane.claim", &text("1|r1|feat-jobs|abc1234|1"));
    game.narrate("requirement.dispatch", "0");
    game.assert_kv("req.1.job", json!("plan"));

    let summary = |game: &mut Game| {
        let summary = game.reply("agent.summary", "0")["meta"]["summary"].clone();
        assert!(
            summary.is_object(),
            "agent.summary must publish meta.summary: {summary}"
        );
        summary
    };

    // ① a plan job in flight → one `jobs` row, no gate yet.
    let fields = summary(&mut game);
    let jobs = fields["jobs"]
        .as_array()
        .unwrap_or_else(|| panic!("summary.jobs must be an array: {fields}"));
    assert_eq!(jobs.len(), 1, "{fields}");
    let job = &jobs[0];
    assert_eq!(job["id"], json!("r1"), "{fields}");
    assert_eq!(job["rev"], json!(1), "{fields}");
    assert_eq!(job["stage"], json!("plan"), "{fields}");
    assert_eq!(job["status"], json!("planning"), "{fields}");
    assert_eq!(job["phase"], json!("planning"), "{fields}");
    assert_eq!(job["lane"], json!(1), "{fields}");
    assert_eq!(job["branch"], json!("feat-jobs"), "{fields}");
    assert_eq!(job["base"], json!("abc1234"), "{fields}");
    // The executor ledger is mirrored onto every job row: unbound = the empty string
    // (never `null`), so a scheduler can read it without a null check.
    assert_eq!(job["executor"], json!(""), "{fields}");
    assert_eq!(job["planExecutor"], json!(""), "{fields}");
    assert_eq!(job["title"], json!("synthetic requirement"), "{fields}");
    assert_eq!(
        job["description"],
        json!("synthetic description"),
        "{fields}"
    );
    let gates = fields["gates"]
        .as_array()
        .unwrap_or_else(|| panic!("summary.gates must be an array: {fields}"));
    assert!(gates.is_empty(), "no gate is pending yet: {fields}");

    // ② accept the plan proposal → the gate row appears and the job slot is released.
    submit_with_exec(&mut game, 1, "plan", "semantics-j1", "agent-1");
    game.click("proposal.accept — ");
    game.assert_kv("req.1.gate", json!("plan"));
    let fields = summary(&mut game);
    let gates = fields["gates"]
        .as_array()
        .unwrap_or_else(|| panic!("summary.gates must be an array: {fields}"));
    assert_eq!(gates.len(), 1, "{fields}");
    assert_eq!(gates[0]["id"], json!("r1"), "{fields}");
    assert_eq!(gates[0]["gate"], json!("plan"), "{fields}");
    assert_eq!(gates[0]["rev"], json!(1), "{fields}");
    assert_eq!(
        gates[0]["title"],
        json!("synthetic requirement"),
        "{fields}"
    );
    let jobs = fields["jobs"]
        .as_array()
        .unwrap_or_else(|| panic!("summary.jobs must be an array: {fields}"));
    assert!(
        jobs.is_empty(),
        "the job slot is released while a gate is pending: {fields}"
    );

    // ③ approve the plan → development is auto-dispatched on the **pre-claimed** lane, so the
    //    branch/base recorded at claim time survive into the new job row.
    game.click("gate.approve — ");
    game.assert_kv("req.1.job", json!("development"));
    let fields = summary(&mut game);
    let jobs = fields["jobs"]
        .as_array()
        .unwrap_or_else(|| panic!("summary.jobs must be an array: {fields}"));
    assert_eq!(jobs.len(), 1, "{fields}");
    assert_eq!(jobs[0]["stage"], json!("development"), "{fields}");
    assert_eq!(jobs[0]["status"], json!("developing"), "{fields}");
    assert_eq!(jobs[0]["phase"], json!("developing"), "{fields}");
    assert_eq!(jobs[0]["lane"], json!(1), "{fields}");
    // ② bound the **plan** executor (`agent-1`): the development row has no executor of its
    // own yet, but `planExecutor` still carries the plan-stage id.
    assert_eq!(
        jobs[0]["executor"],
        json!(""),
        "development is not bound yet: {fields}"
    );
    assert_eq!(
        jobs[0]["planExecutor"],
        json!("agent-1"),
        "the plan-stage executor rides along the development row: {fields}"
    );
    assert_eq!(
        jobs[0]["branch"],
        json!(""),
        "`proposal.accept` (plan) releases the lane ledger, so the auto-dispatched job \
         rebinds a bare lane — the pre-claim branch is gone: {fields}"
    );
    assert_eq!(
        jobs[0]["base"],
        json!(""),
        "the lane ledger was released with the plan job slot: {fields}"
    );
    let gates = fields["gates"]
        .as_array()
        .unwrap_or_else(|| panic!("summary.gates must be an array: {fields}"));
    assert!(gates.is_empty(), "the gate is resolved: {fields}");
}

/// Summary jobs mirror the **bound executor ledger**: `executor` (current stage) and
/// `planExecutor` (the plan stage) are the clean agent id — the `|<reason>` suffix that
/// `config.set` / `exec=` store next to the id is stripped before it reaches the wire.
///
/// Mutation: drop the `wire.executor_id_of` strip in `summary_jobs` → `planExecutor`
/// becomes `agent-2|handoff` and the last assertion goes red.
#[test]
fn summary_jobs_strip_the_executor_reason_suffix() {
    let mut game = Game::open(&project_mod_dir());
    game.narrate(
        "task.intake",
        &text("subsystem gate|synthetic task description"),
    );
    game.narrate(
        "requirement.add",
        &text("synthetic requirement|synthetic description|semantics-r1"),
    );
    game.narrate("requirement.dispatch", "0");
    let jobs = |game: &mut Game| {
        let fields = game.reply("agent.summary", "0")["meta"]["summary"].clone();
        fields["jobs"]
            .as_array()
            .unwrap_or_else(|| panic!("summary.jobs must be an array: {fields}"))
            .clone()
    };

    // ① nothing is bound yet → two empty strings (never `null`).
    let rows = jobs(&mut game);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["executor"], json!(""), "{rows:?}");
    assert_eq!(rows[0]["planExecutor"], json!(""), "{rows:?}");

    // ② an inline `exec=agent-2:handoff` binds the plan stage **with a reason**.
    submit_with_exec(&mut game, 1, "plan", "semantics-exec", "agent-2:handoff");
    game.assert_kv("settings.executor.r1.plan", json!("agent-2|handoff"));
    game.click("proposal.accept — ");
    game.click("gate.approve — ");
    game.assert_kv("req.1.job", json!("development"));

    // ③ the development row reports the clean ids: its own stage is unbound, the plan stage
    //    keeps `agent-2` **without** the `|handoff` suffix.
    let rows = jobs(&mut game);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["stage"], json!("development"), "{rows:?}");
    assert_eq!(rows[0]["executor"], json!(""), "{rows:?}");
    assert_eq!(
        rows[0]["planExecutor"],
        json!("agent-2"),
        "the reason suffix must not leak into the wire: {rows:?}"
    );
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
        "task.intake",
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

    // ④ the host settlement surface reads the same key.
    let summary = game.reply("agent.summary", "0")["meta"]["summary"].clone();
    assert_eq!(
        summary["verifyPassed"],
        json!(true),
        "the summary must see the machine evidence: {summary}"
    );
}

/// A5 `harness.archive_sessions` registers the settlement evidence only when `remaining=0`, and
/// the run can then close without a separate `settlement.sessions` turn.
///
/// Mutation: drop the `settlement.sessions.destination` write from the reactor → `review.note` is
/// stopped by `gates.settlement_problem()` and the `[ENDING` assertion goes red.
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

    // ③ the closing chain no longer needs a `settlement.sessions` turn.
    game.click("verify.submit all_green=true — ");
    game.narrate("settlement.evolution", &text("|screened|c=0/0"));
    game.narrate("settlement.todo", &text("0|main session cleared"));
    let closed = game.click("review.note — ");
    assert!(
        closed["narrative"].to_string().contains("[ENDING"),
        "review.note must close the run: {closed}"
    );
}

/// A6 The declaration surface of the active subsystem on **both** sides: the six built-in tools,
/// their `allowedTools` entries, a usable `artifact_capacity`, and — the structural half — every
/// `TEXTS.<group>.<name>` the source references exists in both scene files.
///
/// Mutation: delete one beat from one side's `data/texts/` beats fragments (or rename it in the source) →
/// this test names the missing key instead of letting the run render a raw `{{placeholder}}`.
#[test]
fn subsystem_declarations_and_beat_references_stay_complete() {
    let project = project_mod_dir();
    let seed = seed_mod_dir();
    let subsystem_tools = [
        "git.worktree_claim",
        "git.worktree_release",
        "git.lane_probe",
        "fs.artifact_probe",
        "gate.verify_suite",
        "harness.archive_sessions",
    ];

    for dir in [&project, &seed] {
        let pack = read_json(&dir.join("pack.json"));
        let config = read_json(&dir.join("data/config.json"));
        let scenes = read_texts(dir);
        let tools = pack["tools"]
            .as_object()
            .unwrap_or_else(|| panic!("{}: pack.json tools must be an object", dir.display()));
        let allowed = config["allowedTools"]
            .as_array()
            .unwrap_or_else(|| panic!("{}: allowedTools must be an array", dir.display()));
        let allowed: Vec<&str> = allowed.iter().filter_map(|item| item.as_str()).collect();
        for id in subsystem_tools {
            assert!(
                tools.contains_key(id),
                "{}: pack.json tools is missing `{id}`",
                dir.display()
            );
            assert!(
                allowed.contains(&id),
                "{}: allowedTools is missing `{id}` (the host refuses the call at request time)",
                dir.display()
            );
            assert_eq!(
                tools[id]["program"],
                json!("ygg-agent"),
                "{id}: the built-in tools are served by the host binary itself"
            );
        }
        for (group, name) in text_references(&project) {
            assert!(
                scenes[group.as_str()][name.as_str()].as_str().is_some(),
                "{}: data/texts/ `{group}.{name}` is referenced by the source but missing \
                 (the run would narrate a raw `{{{{placeholder}}}}`)",
                dir.display()
            );
        }
        let cap = scenes["tuning"]["artifact_capacity"].as_u64().unwrap_or(0);
        assert!(
            cap >= 1,
            "{}: tuning.artifact_capacity must be at least 1 (the ledger is bounded)",
            dir.display()
        );
    }

    // The reference scan itself must be productive (a broken scanner proves nothing).
    let refs = text_references(&project);
    assert!(
        refs.len() > 200,
        "the scene-text scanner found only {} references",
        refs.len()
    );
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
                    .unwrap_or_else(|| format!("feature/{req}"));
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
                    .unwrap_or_else(|| "feature/r1".to_owned());
                out.insert("lane".to_owned(), lane);
                out.insert("base".to_owned(), base);
                out.insert("branch".to_owned(), branch);
                out.insert("head".to_owned(), "deadbeef1234".to_owned());
                out.insert("commits".to_owned(), "2".to_owned());
                out.insert("clean".to_owned(), "1".to_owned());
                out.insert(
                    "files".to_owned(),
                    "src/tools.lua,src/render.lua".to_owned(),
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
/// `harness.archive_sessions`) in **1 single turn each** without `tool.result` callbacks, and
/// `summary.jobs[0].subagentAction` exposes `action`, `brief`, `worktreePath`, `artifactPath`,
/// and `submitCommand`.
#[test]
fn synchronous_tool_bridge_runs_inline_rituals_and_projects_subagent_actions() {
    let mut game = Game::open(&project_mod_dir());
    game.kernel
        .set_tool_runner(Some(Box::new(SyncRitualRunner)));

    game.narrate(
        "task.intake",
        &text("sync bridge|verify 1-turn inline rituals and subagentAction"),
    );
    game.narrate(
        "requirement.add",
        &text("sync tool bridge|implement synchronous ygg.tool.call|sync-r1"),
    );
    game.narrate("requirement.dispatch", "0");

    // 0) During plan stage, summary.jobs[0].subagentAction is `subagent.spawn`.
    let plan_summary = game.reply("agent.summary", "0")["meta"]["summary"].clone();
    let plan_jobs = plan_summary["jobs"].as_array().expect("summary.jobs array");
    assert_eq!(plan_jobs.len(), 1);
    assert_eq!(plan_jobs[0]["worktreeStatus"], json!("n/a"));
    assert_eq!(plan_jobs[0]["subagentStatus"], json!("pending"));
    assert_eq!(plan_jobs[0]["probeStatus"], json!("n/a"));
    let plan_action = &plan_jobs[0]["subagentAction"];
    assert_eq!(plan_action["action"], json!("subagent.spawn"));
    assert_eq!(plan_action["reqId"], json!("r1"));
    assert_eq!(plan_action["stage"], json!("plan"));
    assert_eq!(plan_action["artifactPath"], json!("artifacts/r1-plan.md"));
    let plan_brief = plan_action["brief"].as_str().unwrap_or("");
    assert!(plan_brief.contains("Requirement r1"));
    assert!(
        plan_brief.contains("docs/CONTEXT.md") && plan_brief.contains("docs/DESIGN.md"),
        "brief must include mandatory living docs references: {plan_brief}"
    );
    assert!(plan_action["submitCommand"]
        .as_str()
        .unwrap_or("")
        .contains("proposal.submit"));

    // Submit plan proposal with inline executor registration (`exec=agent-plan`).
    game.narrate(
        "proposal.submit",
        &text("r1|1|plan|plan ready|artifacts/r1-plan.md||exec=agent-plan"),
    );
    let list_with_prop = game.narrate("requirement.list", "0").join("\n");
    assert!(
        list_with_prop.contains("(wt=n/a)(subagent=delivered:p1(agent-plan))"),
        "requirement.list must show delivered proposal status: {list_with_prop}"
    );
    game.click("proposal.accept — ");

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
    game.assert_kv("lane.1.branch", json!("feature/r1"));
    game.assert_kv("lane.1.base", json!("2e24c45"));
    game.assert_kv("lane.1.path", json!("/tmp/ygg-wt-lane-1"));
    game.assert_kv("tool.settled_count", json!(1));
    game.assert_kv("tool.pending", json!(null));

    // Check summary.jobs[0].subagentAction in development stage:
    // Because planExecutor is `agent-plan`, action is `subagent.resume` and worktreePath is populated!
    let dev_summary = game.reply("agent.summary", "0")["meta"]["summary"].clone();
    let dev_jobs = dev_summary["jobs"].as_array().expect("summary.jobs array");
    assert_eq!(dev_jobs.len(), 1);
    assert_eq!(dev_jobs[0]["worktreePath"], json!("/tmp/ygg-wt-lane-1"));
    assert_eq!(
        dev_jobs[0]["worktreeStatus"],
        json!("ready:/tmp/ygg-wt-lane-1@feature/r1")
    );
    assert_eq!(dev_jobs[0]["subagentStatus"], json!("pending"));
    assert_eq!(dev_jobs[0]["probeStatus"], json!("pending"));
    let dev_list = game.narrate("requirement.list", "0").join("\n");
    assert!(
        dev_list.contains("(wt=ready:/tmp/ygg-wt-lane-1@feature/r1)(subagent=pending)"),
        "requirement.list must show live worktreeStatus and subagentStatus: {dev_list}"
    );
    let dev_action = &dev_jobs[0]["subagentAction"];
    assert_eq!(dev_action["action"], json!("subagent.resume"));
    assert_eq!(dev_action["resumeAgentId"], json!("agent-plan"));
    assert_eq!(dev_action["worktreePath"], json!("/tmp/ygg-wt-lane-1"));
    assert_eq!(
        dev_action["artifactPath"],
        json!("artifacts/r1-development.md")
    );
    let dev_brief = dev_action["brief"].as_str().unwrap_or("");
    assert!(dev_brief.contains("/tmp/ygg-wt-lane-1"));
    assert!(
        dev_brief.contains("docs/CONTEXT.md") && dev_brief.contains("docs/DESIGN.md"),
        "development brief must include mandatory living docs references: {dev_brief}"
    );
    assert!(dev_action["submitCommand"]
        .as_str()
        .unwrap_or("")
        .contains("r1|1|development|<summary>|artifacts/r1-development.md||exec=<agent_id>"));

    // 2) Submitting development proposal triggers auto_probe_lane in the SAME turn!
    let dev_prop_lines = game.narrate(
        "proposal.submit",
        &text(
            "r1|1|development|dev done|artifacts/r1-development.md|src/tools.lua|exec=agent-plan",
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

    // 4) Entering review and registering settlement.evolution triggers auto_archive_sessions in 1 turn!
    game.click("coordinate.submit — ");
    game.click("verify.submit all_green=true — ");
    let evo_lines = game.narrate(
        "settlement.evolution",
        &text("|screened|c=0/0|todo=0/verified"),
    );
    assert_eq!(
        count_lines(&evo_lines, "[TOOL SETTLED]"),
        1,
        "{evo_lines:#?}"
    );
    assert_eq!(
        count_lines(&evo_lines, "[SESSIONS ARCHIVED]"),
        1,
        "{evo_lines:#?}"
    );
    game.assert_kv("settlement.sessions.remaining", json!(0));
    game.assert_kv("settlement.sessions.moved", json!(2));
    game.assert_kv(
        "settlement.sessions.destination",
        json!("/tmp/sessions-trash/default"),
    );
    game.assert_kv("tool.settled_count", json!(4));

    let closed = game.click("review.note — ");
    assert!(
        closed["narrative"].to_string().contains("[ENDING"),
        "review.note must close the run after inline session archiving: {closed}"
    );
}

/// A8 Placeholder actor IDs (`"pi-ygg-harness"`, `"unregistered"`, `"main"`) recorded in the
/// executor ledger must NOT trigger a false `subagent.resume`; `subagentAction.action` stays
/// `"subagent.spawn"` with `resumeAgentId = ""`, while `probeStatus` reflects `verified:commits=...,clean=...`
/// once a lane probe has settled.
#[test]
fn placeholder_executor_ids_do_not_trigger_false_subagent_resume() {
    let mut game = Game::open(&project_mod_dir());
    game.kernel
        .set_tool_runner(Some(Box::new(SyncRitualRunner)));

    game.narrate(
        "task.intake",
        &text("guard test|verify is_resumable_agent_id and probeStatus"),
    );
    game.narrate("requirement.add", &text("guard req|verify resume guard|r1"));
    game.narrate("requirement.dispatch", "0");

    // Submit plan proposal using the fallback actor ID `pi-ygg-harness`.
    game.narrate(
        "proposal.submit",
        &text("r1|1|plan|plan by fallback actor|artifacts/r1-plan.md||exec=pi-ygg-harness"),
    );
    game.click("proposal.accept — ");
    game.click("gate.approve — ");

    // Even though `planExecutor` is `"pi-ygg-harness"`, `is_resumable_agent_id` rejects it,
    // so development stage requests `subagent.spawn` with `resumeAgentId == ""`.
    let dev_summary = game.reply("agent.summary", "0")["meta"]["summary"].clone();
    let dev_jobs = dev_summary["jobs"].as_array().expect("summary.jobs array");
    assert_eq!(dev_jobs.len(), 1);
    assert_eq!(dev_jobs[0]["planExecutor"], json!("pi-ygg-harness"));
    assert_eq!(
        dev_jobs[0]["subagentAction"]["action"],
        json!("subagent.spawn")
    );
    assert_eq!(dev_jobs[0]["subagentAction"]["resumeAgentId"], json!(""));

    // Register a real executor for development and populate probe results to verify `dispatched:<id>` + `verified:...`.
    game.narrate(
        "config.set",
        &text("executor.r1.development=agent-dev-uuid-1|assigned"),
    );
    tool_round(
        &mut game,
        "git.lane_probe|lane=1;base=2e24c45;branch=feature/r1|tp1",
        "tp1|git.lane_probe|ok|0|lane=1;branch=feature/r1;base=2e24c45;head=deadbeef1234;commits=2;clean=1;files=src/render.lua",
    );
    let probed_summary = game.reply("agent.summary", "0")["meta"]["summary"].clone();
    let probed_jobs = probed_summary["jobs"]
        .as_array()
        .expect("summary.jobs array");
    assert_eq!(
        probed_jobs[0]["subagentStatus"],
        json!("dispatched:agent-dev-uuid-1")
    );
    assert_eq!(
        probed_jobs[0]["probeStatus"],
        json!("verified:commits=2,clean=1")
    );
    assert_eq!(
        probed_jobs[0]["subagentAction"]["action"],
        json!("subagent.resume")
    );
    assert_eq!(
        probed_jobs[0]["subagentAction"]["resumeAgentId"],
        json!("agent-dev-uuid-1")
    );

    // Switching to `play.mode=goal` before `goal.authorization` is registered emits `[GOAL AUTH PENDING]`.
    let goal_without_auth = game.narrate("config.set", &text("play.mode=goal"));
    assert_eq!(
        count_lines(&goal_without_auth, "[GOAL AUTH PENDING]"),
        1,
        "{goal_without_auth:#?}"
    );
    game.narrate(
        "config.set",
        &text("goal.authorization=authorized for test"),
    );
    let goal_with_auth = game.narrate("config.set", &text("play.mode=goal"));
    assert_eq!(
        count_lines(&goal_with_auth, "[GOAL AUTH PENDING]"),
        0,
        "{goal_with_auth:#?}"
    );
}
