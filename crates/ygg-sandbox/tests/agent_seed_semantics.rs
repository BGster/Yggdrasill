//! agent-seed semantics gate: **decidable** assertions for the 剧情双层重写 two-layer story
//! (4 true states + 13 micro-states), the requirement-level nine-act machine, the
//! run-level resident batch loop (charter → coordinate ⇄ settle → release → retired),
//! and the operation-map / tool-bridge / identity surfaces (docs/design/kernel/state-machine.md §2.10
//! behavioural scope, DESIGN.md §2 remains the thin-shell route).
//!
//! Why this layer exists: the scripted playtest walks only prove happy menus — they never
//! take a **branch** (gate.reject / amendment staleness / integration.reject / a failed
//! verify), and the turn-cut mechanics (inline `exec=`, the reactive auto-dispatch, the
//! settle-close effect, the settlement merge) are invisible to them. This file turns those
//! into assertions:
//!
//! - state assertions read **`kv.entries` of the `save` receipt** (`req.*` / `run.*` are
//!   machine state, not prose): true states are `fsm.stage ∈ {live, retired}` and
//!   `req.N.status ∈ {open, done}`; everything else (`run.phase`, `req.N.job/gate/plan_ok/
//!   integrating/closed_reason`) is a derived micro-state flag;
//! - display assertions read the **derived micro-state name** from `requirement.list`
//!   (`[plan_gate]` / `[dev_review]` / `[integrating]` / `[split]` …) — the single-source
//!   `req_phase()` projection;
//! - map assertions read the **single operation window** from `map.show` (the handler
//!   ignores every payload);
//! - fail-closed assertions read the **driver pre-check envelope** (`error.code=VALIDATION`,
//!   event name + registered count + registered set, zero side effects);
//! - every human action (proposal.accept / gate.approve / coordinate.submit / verify.submit
//!   / review.note) goes through **guide's `ygg.choice_selected`** — the ticket is only
//!   stamped there; the six new 剧情双层重写 events (`charter.set` / `run.begin` /
//!   `integration.*` / `release.*`) deliberately stay **outside** the ticket gate (R5).
//!
//! Boundaries (same stance as docs/context/acts/coordinate.md / acts/settle.md — no added exits):
//! - only the public load surface (`ModBundle` / `KernelFacade` / dispatch / save-load);
//!   no host CLI changes, no production-code changes, no side-channel scripts;
//! - **no run directory is read or written**: legacy-compat and injection fixtures use an
//!   **in-memory save** (mutate the `save` receipt, then `restore`), never `autosave.json`
//!   or `events.jsonl`.

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

mod support;

/// r9：测试侧提交者身份（宿主在真路径上盖 `|actor=`，kernel 直派没有宿主通道）。
/// 签名与推进都用**同一个**身份，才符合「执行这个动作的人读过」的语义；
/// 需要另一个代理时用 `game.as_actor(Some("..."))`。
const TEST_ACTOR: &str = "test-agent";

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
struct Game {
    bundle: ModBundle,
    kernel: KernelFacade,
    /// 被测 mod 目录：补签 helper 要读它的生成区（`data/config.json` 的 `readGate` / `readPlan`）。
    dir: PathBuf,
    /// 关掉自动补签（只给读签门禁自己的用例用：它们要「不签」的现场）。
    suppress_reads: bool,
    /// r9 提交者身份（宿主盖章的测试侧替身）。**只有四个声明路由用得上**：
    /// `proposal.submit` / `gate.approve` / `coordinate.submit` / `config.set`——宿主在真路径上
    /// 往载荷末尾盖 `|actor=<id>`，kernel 直派没有这条通道，所以测试必须自己模拟它。
    /// `None` 模拟「旧宿主 / 没带 --actor」的直接派发面（r9 的拒签与拒绝路径靠它取证）。
    stamp_actor: Option<String>,
}

impl Game {
    /// Load and start on `event.start` **without a charter payload** — the run opens in the
    /// charter micro-state (`run.phase = "charter"`, R2). Tests that need the resident
    /// coordinate phase call `charter_and_begin` / `start_requirement` next.
    fn open(dir: &Path) -> Self {
        let bundle = ModBundle::load(dir)
            .unwrap_or_else(|err| panic!("failed to load {}: {err}", dir.display()));
        let kernel = bundle.kernel().expect("kernel construction");
        let mut game = Self {
            bundle,
            kernel,
            dir: dir.to_path_buf(),
            suppress_reads: false,
            stamp_actor: Some(TEST_ACTOR.to_owned()),
        };
        game.narrate("event.start", "0");
        game
    }

    /// 本步以谁的身份提交（模拟 `--actor`；`None` = 不盖章）。
    fn as_actor(&mut self, actor: Option<&str>) {
        self.stamp_actor = actor.map(str::to_owned);
    }

    /// 派发前按声明路由盖身份段（宿主 `inject_identity_segments` 的同形模拟）。
    /// 只盖四个真声明了 `actor` 的动作；已自带 `actor=` / `actor_session=` 尾段的载荷原样透传。
    fn stamped(&self, event: &str, payload: &str) -> String {
        let Some(actor) = self.stamp_actor.as_deref() else {
            return payload.to_owned();
        };
        if !matches!(
            event,
            "proposal.submit" | "gate.approve" | "coordinate.submit" | "config.set"
        ) {
            return payload.to_owned();
        }
        if payload.contains("|actor=") || payload.contains("|actor_session=") {
            return payload.to_owned();
        }
        let trimmed = payload.trim_end();
        if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
            format!("{}|actor={actor}\"", &trimmed[..trimmed.len() - 1])
        } else {
            payload.to_owned()
        }
    }

    /// Dispatch and assert the envelope `ok` (a domain-error envelope always fails the test).
    #[track_caller]
    fn reply(&mut self, event: &str, payload: &str) -> Json {
        let stamped = self.stamped(event, payload);
        let reply: Json = serde_json::from_str(&self.kernel.dispatch(event, &stamped))
            .unwrap_or_else(|err| panic!("the {event} reply is not JSON: {err}"));
        assert_eq!(reply["ok"], json!(true), "{event} dispatch failed: {reply}");
        reply
    }

    /// Dispatch **without** asserting `ok` — the fail-closed envelope carries
    /// `error.code` / `error.message` instead of a narrative.
    #[track_caller]
    fn raw_reply(&mut self, event: &str, payload: &str) -> Json {
        let stamped = self.stamped(event, payload);
        serde_json::from_str(&self.kernel.dispatch(event, &stamped))
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

    /// One KV in the save (missing key → `Null`; the fixtures observe exactly that).
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
    /// human actions — the ticket is stamped there and nowhere else).
    ///
    /// r4 开闸后的补签：`gate.approve` / `coordinate.submit` 是读签门禁的受控动作，
    /// 这里在派发票据前把该动作的必读集补签齐（见 `auto_sign`）。补签只动 `settings.read.*`，
    /// 不改菜单结构，所以不移动选项下标。
    #[track_caller]
    fn click(&mut self, prefix: &str) -> Json {
        if let Some(action) = action_of_option(prefix) {
            let req = self.pending_gate().map(|(index, _)| index);
            self.ensure_reads(action, None, req);
        }
        let options = self.options();
        let index = options
            .iter()
            .position(|text| text.starts_with(prefix))
            .unwrap_or_else(|| panic!("no menu entry starts with `{prefix}`: {options:?}"));
        let payload = (index + 1).to_string();
        self.reply("ygg.choice_selected", &payload)
    }

    // ── r4 读签门禁的测试侧补签（开闸后既有用例靠它保持绿）──────────────────
    //
    // 口径镜像 `src/domain/readset.lua` 的 `bearing()`：幕维度取 `readPlan.<scene>`，
    // 阶段维度取 `readGate.reqActs.<stage>`，门禁维度取 `readGate.gates.<kind>`，
    // 批边界取 `readPlan.settle`；ver 一律从 `readGate.ver` 台账读，**不硬编码**。
    // 镜像逻辑由「未签必拒 / 签后必放行 / 台账未登记 ver 不补签」几条用例反向铉住。

    /// 被测 mod 的静态配置（生成区就在里面）。
    fn mod_config(&self) -> Json {
        read_json(&self.dir.join("data/config.json"))
    }

    /// 运行幕（微状态口径，与 `requirements.run_phase()` 同义：`""` = coordinate）。
    fn scene(&mut self) -> String {
        match self.kv("run.phase") {
            Json::String(value) if !value.is_empty() => value,
            _ => "coordinate".to_owned(),
        }
    }

    /// 待批门禁：`gate_actions.pick_gate` 取第一个非空 `req.N.gate`。
    fn pending_gate(&mut self) -> Option<(u32, String)> {
        for i in 1..=64 {
            let kind = self.kv(&format!("req.{i}.gate"));
            if let Some(text) = kind.as_str() {
                if !text.is_empty() {
                    return Some((i, text.to_owned()));
                }
            }
            if self.kv(&format!("req.{i}.status")) == Json::Null {
                break;
            }
        }
        None
    }

    /// 该动作的必读集 + 每件的台账 ver（读不到 ver 的件不补签，交给判据叙事告警）。
    fn bearing(
        &mut self,
        action: &str,
        stage: Option<&str>,
        req: Option<u32>,
    ) -> Vec<(String, String)> {
        let config = self.mod_config();
        let plan = config["readPlan"].clone();
        let gate = config["readGate"].clone();
        let scene = self.scene();
        let mut docs: Vec<String> = Vec::new();
        let mut push = |group: &Json| {
            if let Some(items) = group.as_array() {
                for item in items {
                    if let Some(doc) = item.as_str() {
                        if !docs.iter().any(|existing| existing == doc) {
                            docs.push(doc.to_owned());
                        }
                    }
                }
            }
        };
        match action {
            "proposal.submit" => {
                push(&plan[scene.as_str()]);
                if let Some(stage) = stage {
                    push(&gate["reqActs"][stage]);
                }
            }
            "gate.approve" => {
                let index = match req {
                    Some(index) => Some(index),
                    None => self.pending_gate().map(|(index, _)| index),
                };
                if let Some(index) = index {
                    if let Some(kind) = self
                        .kv(&format!("req.{index}.gate"))
                        .as_str()
                        .map(str::to_owned)
                    {
                        push(&gate["gates"][kind.as_str()]);
                    }
                }
            }
            "coordinate.submit" => push(&plan["settle"]),
            _ => {}
        }
        docs.into_iter()
            .map(|doc| {
                let ver = gate["ver"][doc.as_str()].as_str().unwrap_or("").to_owned();
                (doc, ver)
            })
            .collect()
    }

    /// 本动作的作用域进入 seq：幕纪元与该需求阶段纪元取最大者（与 `gates` 的 scope_epoch_of 同口径）。
    fn scope_epoch(&mut self, req: Option<u32>) -> u64 {
        let mut epoch = self.kv("settings.epoch.act.seq").as_u64().unwrap_or(0);
        if let Some(index) = req {
            let stage = self
                .kv(&format!("settings.epoch.req.{index}.stage.seq"))
                .as_u64()
                .unwrap_or(0);
            epoch = epoch.max(stage);
        }
        epoch
    }

    /// 签名序号：值段第三段（r3 形状 `ver|scene|seq`），为 0 时回落 `settings.sign_seq.<doc>`。
    fn sign_seq_of(&mut self, doc: &str, value: &str) -> u64 {
        let third = value
            .split('|')
            .nth(2)
            .and_then(|part| part.parse::<u64>().ok());
        match third {
            Some(seq) if seq > 0 => seq,
            _ => self
                .kv(&format!("settings.sign_seq.{doc}"))
                .as_u64()
                .unwrap_or(0),
        }
    }

    /// 把该动作的必读集**签成新鲜的**：未签、跨了纪元（过期）、或超出窗口的件重签一遍。
    /// 「已签」不等于「还有效」——阶段边界会作废旧签名，所以这里按判据同一口径复核。
    fn ensure_reads(&mut self, action: &str, stage: Option<&str>, req: Option<u32>) {
        if self.suppress_reads {
            return;
        }
        let docs = self.bearing(action, stage, req);
        let epoch = self.scope_epoch(req);
        let now = self.kv("settings.clock_seq").as_u64().unwrap_or(0);
        let window = self.kv("settings.readGate.window").as_u64().unwrap_or(80);
        for (doc, ver) in docs {
            if ver.is_empty() {
                continue;
            }
            let signed = self.kv(&format!("settings.read.{doc}"));
            if let Some(value) = signed.as_str() {
                if !value.is_empty() {
                    let seq = self.sign_seq_of(&doc, value);
                    // 版本也对：文档改版（台账 ver ≠ 签名 ver）同样算过期。
                    let same_ver = value.split('|').next() == Some(ver.as_str());
                    if same_ver && seq > epoch && now.saturating_sub(seq) <= window {
                        continue;
                    }
                }
            }
            self.narrate("config.set", &text(&format!("read.{doc}={ver}")));
        }
    }

    /// Derived micro-state name: the `[…]` slot of the `requirement.list` row — the
    /// `req_phase()` display surface (nine acts + `blocked` overlay + closed reasons).
    /// `req_phase()` display surface (nine acts + `blocked` overlay + closed reasons).
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

    /// The requirement panel line (per-micro-state tally) — the `useMemo` derived surface.
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

/// `rN` → N（补签要按该需求的阶段纪元判新鲜度）。
fn req_index(id: &str) -> Option<u32> {
    id.strip_prefix('r')
        .and_then(|rest| rest.parse::<u32>().ok())
}

/// r4 读签门禁的受控动作：菜单项前缀 → 动作名（其余菜单项返回 None = 不补签）。
/// 三个动作与 `src/domain/readset.lua` 的 `bearing()` 分支一一对应。
fn action_of_option(prefix: &str) -> Option<&'static str> {
    if prefix.starts_with("gate.approve") {
        Some("gate.approve")
    } else if prefix.starts_with("coordinate.submit") {
        Some("coordinate.submit")
    } else {
        None
    }
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

/// The narrative array of an envelope (prose assertions read these lines).
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

// ───────────────────────── drive scripts ─────────────────────────

/// Charter the run and begin it: `charter.set` (the menu payload is a complete three-piece
/// charter) + `run.begin` → `run.phase = ""` (the resident coordinate micro-state, R2/R3).
fn charter_and_begin(game: &mut Game) {
    game.click("charter.set — ");
    game.assert_kv("task.title", json!("任务标题"));
    let begun = game.click("run.begin — ");
    let begun_text = begun["narrative"].to_string();
    assert!(
        begun_text.contains("[RUN BEGUN]"),
        "run.begin must narrate the open: {begun}"
    );
    game.assert_kv("run.phase", json!(""));
}

/// Coordinate → one requirement → the plan job is running. True status stays `open`
/// (剧情双层重写: status only moves at the integration confirm); the micro-state is `planning`.
fn start_requirement(game: &mut Game) {
    charter_and_begin(game);
    game.narrate(
        "requirement.add",
        &text("synthetic requirement|synthetic description|semantics-r1"),
    );
    game.narrate("requirement.dispatch", "0");
    game.assert_kv("req.1.status", json!("open"));
    game.assert_kv("req.1.job", json!("plan"));
    assert_eq!(game.phase("r1"), "planning");
}

/// Submit a proposal for any requirement: `req|rev|stage|summary|artifacts|changes|ext`.
///
/// Deliberately **without** an `exec=` tail: the shared drive scripts must keep
/// `requirement.list` (display names) dispatchable for callers that assert clean rows.
fn submit_req(game: &mut Game, id: &str, rev: u32, stage: &str, ext: &str) {
    // r4 开闸：提交前把该动作的必读集签成新鲜的（模拟「执行者读过才提交」）。
    game.ensure_reads("proposal.submit", Some(stage), req_index(id));
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
    game.ensure_reads("proposal.submit", Some(stage), Some(1));
    let payload = text(&format!(
        "r1|{rev}|{stage}|{stage} proposal summary|artifacts/{stage}.md||{ext}|exec={exec}"
    ));
    game.narrate("proposal.submit", &payload);
}

/// Submit an `r1` **development** proposal with an `exec=` tail and a non-empty changes
/// segment (the changes feed the lane-probe narration in the sync-bridge case).
fn submit_dev_with_exec(
    game: &mut Game,
    rev: u32,
    ext: &str,
    exec: &str,
    changes: &str,
) -> Vec<String> {
    game.ensure_reads("proposal.submit", Some("development"), Some(1));
    let payload = text(&format!(
        "r1|{rev}|development|development proposal summary|artifacts/development.md|{changes}|{ext}|exec={exec}"
    ));
    game.narrate("proposal.submit", &payload)
}

/// Drive to **plan gate pending**: plan proposal → **delivery is the gate** (剧情双层重写) —
/// the submit itself raises `gate="plan"` bound to the rev; there is no acceptance step.
fn drive_to_plan_gate() -> Game {
    drive_to_plan_gate_at(&project_mod_dir())
}

/// Same drive against an arbitrary mod directory (r8: the armed / bare-copy pairs need the
/// **same** pipeline with a different shipped config, so the opening move must be a parameter).
fn drive_to_plan_gate_at(mod_dir: &Path) -> Game {
    let mut game = Game::open(mod_dir);
    start_requirement(&mut game);
    submit(&mut game, 1, "plan", "semantics-p1");
    game.assert_kv("req.1.gate", json!("plan"));
    game.assert_kv("req.1.gate_rev", json!(1));
    game.assert_kv("req.1.job", json!(""));
    assert_eq!(game.phase("r1"), "plan_gate");
    game
}

/// From the plan gate to **dev review pending**: approve plan (the reactive effect
/// auto-dispatches development in the same turn) → deliver.
fn drive_to_dev_pending(ext: &str) -> Game {
    drive_to_dev_pending_at(ext, &project_mod_dir())
}

fn drive_to_dev_pending_at(ext: &str, mod_dir: &Path) -> Game {
    let mut game = drive_to_plan_gate_at(mod_dir);
    game.click("gate.approve — ");
    game.assert_kv("req.1.plan_ok", json!(true));
    game.assert_kv("req.1.job", json!("development"));
    game.assert_kv("req.1.lane", json!(1));
    submit(&mut game, 1, "development", ext);
    // Delivery moves the row to dev_review; the lane stays bound (R7: the executor still
    // needs the worktree for the feature→develop integration).
    game.assert_kv("req.1.job", json!(""));
    game.assert_kv("req.1.lane", json!(1));
    assert_eq!(game.phase("r1"), "dev_review");
    game
}

/// Dev review → **accept gate pending**: the acceptance is the only `proposal.accept` left.
fn drive_to_accept(mut game: Game) -> Game {
    game.click("proposal.accept — ");
    game.assert_kv("req.1.gate", json!("accept"));
    game.assert_kv("req.1.gate_rev", json!(1));
    assert_eq!(game.phase("r1"), "accept_gate");
    game
}

/// Accept gate → **integrating**: `gate.approve` no longer closes the requirement — it opens
/// act 8, and the lane stays bound until the integration confirm/reject (R7).
fn drive_to_integrating(mut game: Game) -> Game {
    game.click("gate.approve — ");
    game.assert_kv("req.1.integrating", json!(true));
    game.assert_kv("req.1.status", json!("open"));
    game.assert_kv("req.1.lane", json!(1));
    assert_eq!(game.phase("r1"), "integrating");
    game
}

/// Integrating → **done**: `integration.confirm` carries the `develop@<sha>` evidence,
/// writes the true state `done`, and releases the lane.
fn drive_to_done(mut game: Game) -> Game {
    let confirmed = game.narrate("integration.confirm", &text("r1|develop@abc1234"));
    assert!(
        confirmed
            .iter()
            .any(|line| line.contains("[INTEGRATION CONFIRMED]")),
        "the integration confirm must narrate its beat: {confirmed:#?}"
    );
    game.assert_kv("req.1.status", json!("done"));
    game.assert_kv("req.1.develop_sha", json!("abc1234"));
    game.assert_kv("req.1.integrating", json!(false));
    game.assert_kv("req.1.lane", json!(0));
    assert_eq!(game.phase("r1"), "done");
    game
}

/// Complete the single requirement and close the batch boundary: `coordinate.submit` moves
/// `run.phase` to `settle` (a micro-state move — `fsm.stage` stays `live`, R8).
fn drive_to_settle() -> Game {
    drive_to_settle_at(&project_mod_dir())
}

/// Same drive against an arbitrary mod directory (r8 armed / bare-copy pairs).
fn drive_to_settle_at(mod_dir: &Path) -> Game {
    let mut game = drive_to_done(drive_to_integrating(drive_to_accept(
        drive_to_dev_pending_at("semantics-p2", mod_dir),
    )));
    let closed = game.click("coordinate.submit — ");
    assert!(
        closed["narrative"]
            .to_string()
            .contains("[COORDINATION DONE]"),
        "coordinate.submit must open the settle act: {closed}"
    );
    game.assert_kv("run.phase", json!("settle"));
    game.assert_kv("fsm.stage", json!(null)); // live is the FSM initial state: not materialized in KV yet
    game.assert_kv("verify.passed", json!(false));
    game
}

/// Settle the four items (verify → merged evolution registration with the `sessions=` /
/// `todo=` segments) — the settle-close effect then flips `run.phase` to `release` (R9).
fn drive_to_release() -> Game {
    let mut game = drive_to_settle();
    let verified = game.click("verify.submit all_green=true — ");
    assert!(
        verified["narrative"].to_string().contains("[GATES GREEN]"),
        "verify.submit all_green=true must record evidence: {verified}"
    );
    game.assert_kv("verify.passed", json!(true));
    let registered = game.narrate(
        "settlement.evolution",
        &text(
            "|candidates screened against the evolution chapter|c=0/0|\
             sessions=1/0/~/.pi/agent/sessions-trash/semantics|todo=0/re-checked the todo list",
        ),
    );
    assert!(
        registered
            .iter()
            .any(|line| line.contains("[SETTLE READY]")),
        "the four-settled effect must narrate the release move: {registered:#?}"
    );
    game.assert_kv("run.phase", json!("release"));
    game
}

// ───────────────────────── charter & resident lifecycle ─────────────────────────

/// The charter act (R2/R3/R4): boot lands in `charter` with `charter.set` as the only
/// business item; `run.begin` requires the three-piece charter and is refused before it;
/// the charter is revisable inside the act; `run.begin` moves to `run.phase=""` and locks
/// the charter (a later `charter.set` is a readable refusal).
#[test]
fn charter_gate_revises_locks_and_begins_the_run() {
    let mut game = Game::open(&project_mod_dir());

    // Boot lands in the charter micro-state; the menu tells the truth (no run.begin yet).
    game.assert_kv("run.phase", json!("charter"));
    let options = game.options();
    assert!(
        options[0].starts_with("charter.set — "),
        "charter.set must be the first charter item: {options:?}"
    );
    assert!(
        !options.iter().any(|item| item.starts_with("run.begin — ")),
        "an incomplete charter must not put run.begin on the menu: {options:?}"
    );

    // run.begin precondition: the three-piece charter. Readable refusal, no phase move.
    let early = game.narrate("run.begin", "0");
    assert_refused(&early, "run.begin");
    game.assert_kv("run.phase", json!("charter"));

    // charter.set writes the trio and is revisable inside the act (last write wins).
    game.narrate(
        "charter.set",
        &text("semantics run|prove the v2 state machine|suite green"),
    );
    game.assert_kv("task.title", json!("semantics run"));
    game.assert_kv("task.goal", json!("prove the v2 state machine"));
    game.assert_kv("task.success", json!("suite green"));
    assert!(
        game.options()
            .iter()
            .any(|item| item.starts_with("run.begin — ")),
        "a complete charter must put run.begin on the menu"
    );
    game.narrate(
        "charter.set",
        &text("semantics run v2|prove the 剧情双层重写 loop|suite green"),
    );
    game.assert_kv("task.title", json!("semantics run v2"));

    // run.begin → coordinate: run.phase = "" (the resident main state, R2), [RUN BEGUN].
    game.click("run.begin — ");
    game.assert_kv("run.phase", json!(""));
    let options = game.options();
    assert!(
        options[0].starts_with("coordinate.submit"),
        "the coordinate menu must lead with the batch closer on an empty queue: {options:?}"
    );

    // R4 lock: after run.begin the charter is immutable — a readable refusal, value kept.
    let locked = game.narrate("charter.set", &text("late charter|must refuse|locked"));
    assert_refused(&locked, "charter.set");
    game.assert_kv("task.title", json!("semantics run v2"));
}

/// r9 Resident lifecycle: `review.note` **settles the batch and returns to coordinate**;
/// the run only ends through an explicitly armed retirement (a separate case).
///
/// Baseline A — the pinned script walks the charter first (`2`=run.begin once the menu
/// offers it, then `1`×3 refused batch closers on the empty queue): 1 boot + 4 choices,
/// exhausted back at a coordinate choice (`reached=choice`, 5 steps) with the `[RUN BEGUN]`
/// line present and **no closure beat** anywhere. Script pins make a menu reorder fail here
/// loudly, not silently.
#[test]
fn resident_batch_cycle_reaches_coordinate_again() {
    for dir in [project_mod_dir(), seed_mod_dir()] {
        let options = PlaytestOptions {
            choices: vec![1, 2, 1, 1],
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
            "{}: the pinned batch-cycle walk is 5 steps (boot + charter.set + run.begin + two refused closers)",
            dir.display()
        );
        // (The narrative_sample is capped at 8 lines and the boot receipt alone fills 7 —
        // later receipts are invisible here; the coordinate-phase reachability is pinned by
        // the Game-driven cases instead.)
        let closed = report.narrative_sample.iter().any(|line| {
            line.starts_with("[COORDINATION DONE]")
                || line.starts_with("[GATES GREEN]")
                || line.starts_with("[BATCH")
        });
        assert!(
            !closed,
            "{}: an empty queue must never close: {:?}",
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

/// r9 Baseline B — the armed retirement (the **only** path to a run-level ending) is
/// unreachable without a completed, release-declared batch. The pinned script arms
/// `run.retire` through the config sub-page (`3`=config · `4`=run.retire · `5`=back) and
/// then walks the charter (`1`=charter.set · `2`=run.begin) into a refused batch closer:
/// exhausted at a choice (`reached=choice`, 7 steps) with **no `[ENDING` line anywhere**.
#[test]
fn resident_retire_is_unreachable_without_a_charter() {
    for dir in [project_mod_dir(), seed_mod_dir()] {
        let options = PlaytestOptions {
            choices: vec![3, 4, 5, 1, 2, 1],
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
            "{} without a completed batch the retiring ending must be unreachable",
            dir.display()
        );
        assert_eq!(
            report.steps,
            7,
            "{}: the pinned retirement walk is 7 steps",
            dir.display()
        );
        assert!(
            !report
                .narrative_sample
                .iter()
                .any(|line| line.starts_with("[ENDING")),
            "{}: no ending line may appear on an unfinished run: {:?}",
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
/// - armed: the next `review.note` writes the retiring ending (`[ENDING · run retired]`),
///   flips `fsm.stage` to `retired` (the only true FSM transition), and **without** a
///   `[BATCH` settle line or a `run.batch` write;
/// - clearing stays available after retirement (`config.set` is stage-less).
#[test]
fn run_retire_value_domain_and_retiring_close() {
    let mut game = drive_to_release();

    // Value domain: anything other than true / "" is refused and writes nothing.
    let bad = game.narrate("config.set", &text("run.retire=false"));
    assert_refused(&bad, "run.retire");
    game.assert_kv("settings.run.retire", json!(null));

    // Declare the release (R10 precondition for any review.note) before arming.
    game.narrate("release.skip", &text("纯测试批次，无 develop→master 合并"));

    // Arm through the sub-page item (pure presentation entry, the write is still config.set).
    game.click("config — ");
    let sub_page = game.options();
    assert!(
        sub_page.len() == 5 && sub_page[3].starts_with("config.set — run.retire"),
        "the sub-page must offer run.retire as its 4th item: {sub_page:?}"
    );
    game.click("config.set — run.retire");
    game.assert_kv("settings.run.retire", json!("true"));
    assert_eq!(game.options().len(), 5, "still on the config sub-page");
    game.click("back — ");

    // The retiring close: review.note in the release phase with the release declared.
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
    game.assert_kv("ending", json!("retired"));
    game.assert_kv("fsm.stage", json!("retired"));
    // run.batch stays untouched (the key is never written on the retiring path).
    game.assert_kv("run.batch", json!(null));
    game.assert_kv(
        "review.evolution.summary",
        json!("candidates screened against the evolution chapter"),
    );

    // Clearing stays available after retirement and narrates its own beat.
    let cleared = game.narrate("config.set", &text("run.retire="));
    assert_eq!(
        count_lines(&cleared, "[RUN RETIRE CLEARED]"),
        1,
        "{cleared:#?}"
    );
    game.assert_kv("settings.run.retire", json!(""));
}

/// r9 The default no-arg all-1 walk is **expected to fail** under the resident lifecycle:
/// the charter gate keeps an empty run from ever closing, so the walk loops on refusals
/// until the step limit — `ok=false, reached=limit` is the documented steady state, not a
/// regression. No closure beat ([COORDINATION DONE] / [GATES GREEN] / [BATCH) and no
/// ending line may ever appear in the walked narrative.
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
        let closed = report.narrative_sample.iter().any(|line| {
            line.starts_with("[COORDINATION DONE]")
                || line.starts_with("[GATES GREEN]")
                || line.starts_with("[BATCH")
                || line.starts_with("[ENDING")
        });
        assert!(
            !closed,
            "{}: the refusal loop must never close: {:?}",
            dir.display(),
            report.narrative_sample
        );
    }
}

// ───────────────────── requirement pipeline: gates & deliverables ─────────────────────

/// The accept gate rejected **without** an amend (R6): `plan_ok` keeps its value → the row
/// falls back to `ready` (待开发)， and the next dispatch hands out **development** (not
/// plan). A direct `gate.approve` without a menu ticket is refused with the state unmoved.
#[test]
fn accept_gate_reject_keeps_plan_ok_and_redispatches_development() {
    let mut game = drive_to_accept(drive_to_dev_pending("semantics-p2"));
    game.assert_kv("req.1.status", json!("open"));
    game.assert_kv("req.1.gate_rev", json!(1));

    // guide gate: a pass-type action without a menu ticket is always refused and the
    // state does not move.
    let direct = game.narrate("gate.approve", &text("r1"));
    assert_refused(&direct, "gate.approve");
    game.assert_kv("req.1.gate", json!("accept"));
    game.assert_kv("req.1.status", json!("open"));

    // Normal reject (rejects are outside the ticket gate; the menu keeps a single entry).
    game.click("gate.reject — ");
    game.assert_kv("req.1.gate", json!(""));
    game.assert_kv("req.1.plan_ok", json!(true));
    game.assert_kv("req.1.lane", json!(0));
    assert_eq!(game.phase("r1"), "ready");

    game.narrate("requirement.dispatch", "0");
    game.assert_kv("req.1.job", json!("development"));
    game.assert_kv("req.1.lane", json!(1));
}

/// Amend while the accept gate is pending: `rev+1` and `plan_ok` is invalidated → the old
/// approval is stale (refused, state unchanged — `gate_rev` still binds the old rev) → the
/// reject path falls back to `draft` (the invalidated plan), and the next dispatch replans.
#[test]
fn accept_gate_amend_invalidates_plan_and_stales_the_pending_gate() {
    let mut game = drive_to_accept(drive_to_dev_pending("semantics-p2"));
    assert_eq!(game.phase("r1"), "accept_gate");

    game.narrate("requirement.amend", &text("r1|amended title"));
    game.assert_kv("req.1.rev", json!(2));
    game.assert_kv("req.1.plan_ok", json!(false));
    // The gate itself is not taken down (only reject/approve do that) but it is bound to
    // the old rev.
    game.assert_kv("req.1.gate", json!("accept"));
    game.assert_kv("req.1.gate_rev", json!(1));

    // The old approval must not take effect: stale rejection, state and gate untouched.
    let stale = game.click("gate.approve — ");
    assert_refused(&narrative_lines(&stale), "stale");
    game.assert_kv("req.1.status", json!("open"));
    game.assert_kv("req.1.gate", json!("accept"));
    game.assert_kv("req.1.gate_rev", json!(1));

    game.click("gate.reject — ");
    game.assert_kv("req.1.gate", json!(""));
    game.assert_kv("req.1.plan_ok", json!(false));
    assert_eq!(game.phase("r1"), "draft");

    game.narrate("requirement.dispatch", "0");
    game.assert_kv("req.1.job", json!("plan"));
}

/// The `plan` gate rejected: `plan_ok` goes back to false and the plan task is re-attached
/// to the executor (`job="plan"`) — a fresh plan proposal on the same row is accepted again
/// (slot = planning ∧ no gate).
#[test]
fn plan_gate_reject_requires_a_new_plan() {
    let mut game = drive_to_plan_gate();
    assert_eq!(game.phase("r1"), "plan_gate");

    game.click("gate.reject — ");
    game.assert_kv("req.1.gate", json!(""));
    game.assert_kv("req.1.plan_ok", json!(false));
    // The plan job returns to the executor without a new dispatch turn.
    game.assert_kv("req.1.job", json!("plan"));
    assert_eq!(game.phase("r1"), "planning");

    // A second plan proposal is deliverable again (the gate slot is free).
    submit(&mut game, 1, "plan", "semantics-p1-replan");
    game.assert_kv("req.1.gate", json!("plan"));
    game.assert_kv("req.1.gate_rev", json!(1));
}

/// Approving the `plan` gate makes the row `ready`; the declared `useEffect` auto-dispatches
/// **development for that requirement only** before the receipt is rendered (one turn saved),
/// and the memoized requirement panel reflects the new state (no stale `useMemo`).
#[test]
fn plan_gate_approval_auto_dispatches_development_in_the_same_turn() {
    let mut game = drive_to_plan_gate();
    assert_eq!(game.phase("r1"), "plan_gate");
    let before = game.panel();
    assert!(
        before.contains("门禁 1"),
        "before approval the row sits on the plan gate: {before}"
    );

    let reply = game.click("gate.approve — ");
    let narrative = narrative_lines(&reply);
    assert!(
        narrative
            .iter()
            .any(|line| line.starts_with("[AUTO-DISPATCH]")),
        "the auto-dispatch effect must narrate its own line: {narrative:#?}"
    );

    // Same turn: the development job and its lane already exist; the true status is open.
    game.assert_kv("req.1.status", json!("open"));
    game.assert_kv("req.1.job", json!("development"));
    game.assert_kv("req.1.lane", json!(1));
    assert_eq!(game.phase("r1"), "developing");

    // Memoized derived tally must be invalidated by the state change.
    let after = game.panel();
    assert_ne!(before, after, "the panel must not stay memoized stale");
    assert!(
        after.contains("开发中 1") && after.contains("门禁 0"),
        "the panel must show the auto-dispatched requirement: {after}"
    );
}

/// Development `proposal.reject` (**before** the accept gate, R6):
/// - not amended: `plan_ok` keeps its value → back to `ready` with the lane released, the
///   next dispatch re-claims development;
/// - amended: the rev bump invalidated `plan_ok` → the row falls back to `draft`, and the
///   next dispatch hands out **plan** (replan).
#[test]
fn dev_proposal_reject_keeps_plan_ok_unless_amended() {
    // Path without an amend.
    let mut normal = drive_to_dev_pending("semantics-p3");
    normal.narrate("proposal.reject", &text("r1|incomplete artifacts"));
    normal.assert_kv("req.1.status", json!("open"));
    normal.assert_kv("req.1.plan_ok", json!(true));
    normal.assert_kv("req.1.lane", json!(0));
    assert_eq!(normal.phase("r1"), "ready");
    normal.narrate("requirement.dispatch", "0");
    normal.assert_kv("req.1.job", json!("development"));
    normal.assert_kv("req.1.lane", json!(1));

    // Amended path: the proposal is pending, the amend invalidates the rev, then reject.
    let mut amended = drive_to_dev_pending("semantics-p4");
    amended.narrate("requirement.amend", &text("r1|add an acceptance criterion"));
    amended.assert_kv("req.1.rev", json!(2));
    amended.assert_kv("req.1.plan_ok", json!(false));
    amended.narrate("proposal.reject", &text("r1|redo on the new rev"));
    amended.assert_kv("req.1.status", json!("open"));
    amended.assert_kv("req.1.plan_ok", json!(false));
    assert_eq!(amended.phase("r1"), "draft");
    amended.narrate("requirement.dispatch", "0");
    amended.assert_kv("req.1.job", json!("plan"));
}

/// Amending a requirement **while development is in flight** (before any development
/// proposal is submitted) updates its description and bumps `rev` without resetting the
/// job (`req_amend_active`); submitting the new `rev`'s development proposal must succeed
/// rather than deadlocking on `plan_ok == false`. If that proposal is later rejected,
/// `plan_ok == false` sends the row back to `draft` (replan).
#[test]
fn developing_amend_without_pending_proposal_does_not_deadlock() {
    let mut game = drive_to_plan_gate();
    game.click("gate.approve — ");
    // Auto-dispatched into developing (no pending proposal yet).
    game.assert_kv("req.1.job", json!("development"));

    game.narrate(
        "requirement.amend",
        &text("r1|refined title|refined description in flight"),
    );
    game.assert_kv("req.1.rev", json!(2));
    game.assert_kv("req.1.job", json!("development"));
    game.assert_kv("req.1.plan_ok", json!(false));
    assert_eq!(game.phase("r1"), "developing");

    // Submitting a development proposal for the new rev must succeed (no deadlock).
    submit(&mut game, 2, "development", "semantics-dev-after-amend");
    assert_eq!(game.phase("r1"), "dev_review");

    // And if that proposal is rejected, `plan_ok == false` sends it back to `draft` (replan).
    game.narrate("proposal.reject", &text("r1|needs replan after amend"));
    game.assert_kv("req.1.plan_ok", json!(false));
    assert_eq!(game.phase("r1"), "draft");
}

// ───────────────────── inline executor ledger (`exec=`) ─────────────────────

/// `proposal.submit` binds the executor in the same transaction, refuses an `exec=` that is
/// not the last segment, and requires a reason for a cross-stage handoff (given here with
/// the `:` form so it survives the outer `|` split).
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

    // Cross-stage handoff without a reason → precondition rejection, no proposal. The plan
    // gate is up now; approving it (delivery is the gate) auto-dispatches development.
    game.click("gate.approve — ");
    // r4 开闸：门禁批准跨了阶段边界，旧签名随之作废——直派提交前按判据口径重签。
    game.ensure_reads("proposal.submit", Some("development"), Some(1));
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

// ───────────────────── settlement merge & the batch loop ─────────────────────

/// One `settlement.evolution` turn settles the evolution record **and** the two end-of-run
/// cleanups (`sessions=` / `todo=`) — the four-settled effect then moves the run into the
/// release micro-state, and `review.note` (release declared) closes the batch right after
/// it. A malformed optional segment is refused (shape gate, nothing written).
#[test]
fn settlement_evolution_settles_sessions_and_todo_in_one_turn() {
    let mut game = drive_to_settle();
    let verified = game.click("verify.submit all_green=true — ");
    assert!(
        verified["narrative"].to_string().contains("[GATES GREEN]"),
        "verify.submit all_green=true must record evidence: {verified}"
    );

    // A malformed optional segment is refused before anything is written.
    let bad = game.narrate(
        "settlement.evolution",
        &text("|summary|c=0/0|sessions=1/3/nowhere"),
    );
    assert_refused(&bad, "sessions=");
    // The refusal writes nothing: moved stays at the reset_settlement baseline (0).
    game.assert_kv("settlement.sessions.moved", json!(0));

    // The merged turn: evolution + sessions + todo in one transaction.
    let payload = text(
        "|candidates screened against the evolution chapter|c=0/0|\
sessions=1/0/~/.pi/agent/sessions-trash/semantics|todo=0/re-checked the todo list",
    );
    let registered = game.narrate("settlement.evolution", &payload);
    assert!(
        registered
            .iter()
            .any(|line| line.contains("[SETTLE READY]")),
        "the four-settled effect must fire in the same turn: {registered:#?}"
    );
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
    game.assert_kv("run.phase", json!("release"));

    // The batch close needs the release declaration first; skip with a reason, then close.
    game.narrate("release.skip", &text("本批无需发布：纯测试批次"));
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
    game.assert_kv("fsm.stage", json!(null)); // live is the FSM initial state: not materialized in KV yet
    game.assert_kv("run.phase", json!(""));
    game.assert_kv("verify.passed", json!(false));
    game.assert_kv("settlement.evolution.summary", json!(""));
    game.assert_kv("run.release.master_sha", json!(""));
    game.assert_kv("run.release.skipped_reason", json!(""));
    game.assert_kv("review.evolution.candidates", json!("0/0"));
    game.assert_kv(
        "review.evolution.summary",
        json!("candidates screened against the evolution chapter"),
    );
}

/// A failed verify returns the **whole batch** to coordinate (R9): `run.phase=""`, the
/// settlement and release keys are cleared, the failure counter increments, and the
/// coordinate surface is usable again (a new requirement can enter the queue).
#[test]
fn verify_failure_returns_the_batch_to_coordinate() {
    let mut game = drive_to_settle();
    let failed = game.click("verify.submit all_green=false — ");
    assert!(
        failed["narrative"].to_string().contains("[GATES FAILED]"),
        "the honest failure must narrate its beat: {failed}"
    );
    game.assert_kv("run.phase", json!(""));
    game.assert_kv("fsm.stage", json!(null)); // live is the FSM initial state: not materialized in KV yet
    game.assert_kv("verify.passed", json!(false));
    game.assert_kv("verify.failures", json!(1));
    game.assert_kv("settlement.evolution.summary", json!(""));
    game.assert_kv("run.release.master_sha", json!(""));

    // Coordinate again: the queue accepts new work for the next round.
    game.narrate(
        "requirement.add",
        &text("rework requirement|desc|semantics-rework"),
    );
    game.assert_kv("req.2.status", json!("open"));
}

/// The release act gates the batch settlement (R-a/R10): an undeclared release keeps
/// `review.note` off the menu; `release.skip` requires a non-empty reason; `release.confirm`
/// validates the sha/tag shape; either declaration unblocks the batch close, and the next
/// batch proves the resident self-loop (coordinate ⇄ settle → release → coordinate).
#[test]
fn release_declaration_gates_the_batch_settlement() {
    let mut game = drive_to_release();

    // Undeclared: review.note is not on the menu; the declaration items are.
    let options = game.options();
    assert!(
        options
            .iter()
            .any(|item| item.starts_with("release.confirm — "))
            && options
                .iter()
                .any(|item| item.starts_with("release.skip — "))
            && !options
                .iter()
                .any(|item| item.starts_with("review.note — ")),
        "an undeclared release must offer confirm/skip and withhold review.note: {options:?}"
    );

    // release.skip without a reason is refused (an empty payload is not a reason);
    // confirm shapes are validated.
    let empty = game.narrate("release.skip", &text(""));
    assert_refused(&empty, "release.skip");
    let bad_sha = game.narrate("release.confirm", &text("master@zzzz|v2026.11.12.1"));
    assert_refused(&bad_sha, "release.confirm");
    let bad_tag = game.narrate("release.confirm", &text("master@abc1234|not-a-tag"));
    assert_refused(&bad_tag, "release.confirm");
    // The refused declarations write nothing: the release keys have never been written yet.
    game.assert_kv("run.release.master_sha", json!(null));
    game.assert_kv("run.release.skipped_reason", json!(null));

    // Skip with a reason → declared → the batch close settles back to coordinate.
    let skipped = game.narrate("release.skip", &text("本批无需发布：纯测试批次"));
    assert!(
        skipped
            .iter()
            .any(|line| line.contains("[RELEASE SKIPPED]")),
        "{skipped:#?}"
    );
    game.assert_kv(
        "run.release.skipped_reason",
        json!("本批无需发布：纯测试批次"),
    );
    assert!(
        game.options()[0].starts_with("review.note — "),
        "a declared release must lead with the batch closer"
    );
    let closed = game.click("review.note — ");
    assert!(
        closed["narrative"]
            .to_string()
            .contains("[BATCH 1 SETTLED]"),
        "{closed}"
    );
    game.assert_kv("run.phase", json!(""));
    game.assert_kv("run.batch", json!(2));

    // Batch 2 proves the self-loop with the confirm path。文档维护义务（用户 2026-11-15 裁定：
    // 归主会话自办）必须在批边界前解除，否则 coordinate.submit 被硬阻塞。
    let cleared = game.narrate("config.set", &text("docs.maintenance.done=1"));
    assert!(
        cleared
            .iter()
            .any(|l| l.contains("[DOCS MAINTENANCE CLEARED]")),
        "{cleared:#?}"
    );
    game.narrate(
        "requirement.add",
        &text("second batch requirement|desc|semantics-b2"),
    );
    game.narrate("requirement.dispatch", "0"); // dispatches r2 (the fresh requirement)
    submit_req(&mut game, "r2", 1, "plan", "semantics-b2-p1");
    game.click("gate.approve — "); // r2 plan gate → auto-dispatch development
    submit_req(&mut game, "r2", 1, "development", "semantics-b2-p2");
    game.click("proposal.accept — "); // dev review → accept gate
    game.click("gate.approve — "); // accept gate → integrating
    game.narrate("integration.confirm", &text("r2|develop@beef124"));
    game.click("coordinate.submit — ");
    game.click("verify.submit all_green=true — ");
    game.narrate(
        "settlement.evolution",
        &text("|batch two screened|c=0/0|sessions=0/0/none|todo=0/cleared"),
    );
    game.assert_kv("run.phase", json!("release"));
    let confirmed = game.narrate("release.confirm", &text("master@1a2b3c4|v2026.11.12.2"));
    assert!(
        confirmed
            .iter()
            .any(|line| line.contains("[RELEASE CONFIRMED]")),
        "{confirmed:#?}"
    );
    game.assert_kv("run.release.master_sha", json!("1a2b3c4"));
    game.assert_kv("run.release.tag", json!("v2026.11.12.2"));
    let closed = game.click("review.note — ");
    assert!(
        closed["narrative"]
            .to_string()
            .contains("[BATCH 2 SETTLED]"),
        "{closed}"
    );
    game.assert_kv("run.batch", json!(3));
    game.assert_kv("fsm.stage", json!(null)); // live is the FSM initial state: not materialized in KV yet
}

// ───────────────────── batch-maintenance auto-append (r3 / 批结算自动维护) ─────────────────────

/// An unarmed batch settle **auto-appends the next batch's docs-maintenance requirement**
/// (批结算自动维护): the `[BATCH 1 SETTLED]` receipt is followed by `[MAINTENANCE REQ ADDED]` in
/// the same transaction — one new open/draft row carrying the `req.<i>.maintenance` marker,
/// the canonical title (no batch number, so the dedup title match stays exact) and the
/// per-file wiki guidance in its description; `run.maintenance.last` points at it.
#[test]
/// 未武装批结算在尾部登记**文档维护义务**（用户 2026-11-15 裁定：CONTEXT / DESIGN 的维护是
/// 主会话自办工作，不再派生可派发的需求——subagent 缺跨批次上下文，r11 / r12 两次实测的产出
/// 仍需主会话逐条复核）。义务落在 `docs.maintenance.due`，并把下一次 `coordinate.submit`
/// 硬阻塞住，直到主会话做完复核、用 `config.set docs.maintenance.done=<batch>` 解除。
#[test]
fn batch_settle_marks_docs_maintenance_due_and_blocks_the_next_boundary() {
    let mut game = drive_to_release();
    game.narrate("release.skip", &text("纯测试批次，无 develop→master 合并"));
    let closed = game.click("review.note — ");
    let narrative = closed["narrative"].to_string();
    assert!(
        narrative.contains("[BATCH 1 SETTLED]"),
        "review.note must settle the batch: {closed}"
    );
    assert!(
        narrative.contains("[DOCS MAINTENANCE DUE]"),
        "the batch settle must register the docs-maintenance obligation: {closed}"
    );
    assert!(
        !narrative.contains("[ENDING"),
        "a batch settle must never carry the ending prefix: {closed}"
    );
    game.assert_kv("run.batch", json!(2));
    game.assert_kv("run.phase", json!(""));
    game.assert_kv("docs.maintenance.due", json!("1"));
    // 不再派生需求行：注册表里仍是那一条（已终态）——维护不是可派发的工作。
    game.assert_kv("req.count", json!(1));

    // 义务未解除 → 批边界直接拒，并给出解除命令（走菜单点击，绕开 guide 的人类门禁）。
    let blocked = narrative_lines(&game.click("coordinate.submit — "));
    let joined = blocked.join("\n");
    assert!(
        joined.contains("docs-maintenance/due") && joined.contains("docs.maintenance.done=1"),
        "the boundary must be blocked with the clearing command: {joined}"
    );
    game.assert_kv("run.phase", json!(""));

    // 解除 → 批边界放行。
    let cleared = game.narrate("config.set", &text("docs.maintenance.done=1"));
    assert!(
        cleared
            .iter()
            .any(|l| l.contains("[DOCS MAINTENANCE CLEARED]")),
        "{cleared:#?}"
    );
    game.assert_kv("docs.maintenance.due", json!(""));
    game.click("coordinate.submit — ");
    game.assert_kv("run.phase", json!("settle"));
}

/// The dedup boundary (③「未终态」语义): a **terminal** same-title maintenance requirement
/// does **not** suppress the auto-append. A manual same-title add during the settle act is
/// legal (requirement.add admits settle), but it re-arms the global gate: all-green verify
/// is refused (the row is open), the batch falls back to coordinate, the manual row walks
/// the pipeline like any requirement, and when the batch finally closes it appends a
/// fresh round's row (r3) — a terminal same-title row does not suppress it. (The failed
/// verify never completed the close, so `run.batch` stays 1: this IS batch 1's close.)
#[test]
/// 解除只认**等值批次**：拿旧批次号清不掉新义务，未知 docs 键直接拒；空值才是显式放弃。
#[test]
fn docs_maintenance_clear_requires_the_outstanding_batch() {
    let mut game = drive_to_release();
    game.narrate("release.skip", &text("纯测试批次"));
    game.click("review.note — ");
    game.assert_kv("docs.maintenance.due", json!("1"));

    let stale = game.narrate("config.set", &text("docs.maintenance.done=0"));
    assert_refused(&stale, "config.set");
    game.assert_kv("docs.maintenance.due", json!("1"));

    let unknown = game.narrate("config.set", &text("docs.maintenance.other=1"));
    assert_refused(&unknown, "config.set");
    game.assert_kv("docs.maintenance.due", json!("1"));

    let cleared = game.narrate("config.set", &text("docs.maintenance.done=1"));
    assert!(
        cleared
            .iter()
            .any(|l| l.contains("[DOCS MAINTENANCE CLEARED]")),
        "{cleared:#?}"
    );
    game.assert_kv("docs.maintenance.due", json!(""));
}

/// The auto-appended maintenance requirement **walks the nine acts** and **gates the next
/// boundary** until done: the per-batch docs cadence is enforced by `gate_problem()`, not
/// by convention. The second batch settle appends the next round's row (r3).
#[test]

/// The **retired path appends nothing**: an armed `review.note` writes the ending and
/// leaves the register (and the pointer) untouched — no maintenance row for a closed run.
#[test]
fn retire_appends_no_maintenance_requirement() {
    let mut game = drive_to_release();
    game.narrate("release.skip", &text("纯测试批次，无 develop→master 合并"));
    game.narrate("config.set", &text("run.retire=true"));
    game.assert_kv("settings.run.retire", json!("true"));
    let closed = game.click("review.note — ");
    let narrative = closed["narrative"].to_string();
    assert!(
        narrative.contains("[ENDING · run retired]"),
        "armed review.note must retire the run: {closed}"
    );
    assert!(
        !narrative.contains("[MAINTENANCE REQ"),
        "the retired path must not append a maintenance row: {closed}"
    );
    assert!(
        !narrative.contains("[BATCH"),
        "a retiring close must never carry a batch-settle line: {closed}"
    );
    game.assert_kv("req.count", json!(1));
    game.assert_kv("run.maintenance.last", json!(null));
    game.assert_kv("ending", json!("retired"));
}

// ───────────────────── gate locality & the integration act ─────────────────────

/// Gate independence: **another requirement's** pending proposal / running job does not
/// block r1's gate. Approving r1 at the plan gate while r2 sits on its own plan gate and
/// r3's plan job is running must pass the gate and auto-dispatch r1's development in the
/// same turn. The run-closing preconditions (`coordinate.submit` / `verify.submit` /
/// `review.note`) stay global — only per-requirement advancement is localized.
#[test]
fn gate_independence_other_requirements_do_not_block() {
    let mut game = drive_to_plan_gate();
    // r2: add → dispatch → submit a plan proposal that raises its own plan gate.
    game.narrate(
        "requirement.add",
        &text("second requirement|synthetic description|semantics-r2"),
    );
    game.narrate("requirement.dispatch", "0");
    submit_req(&mut game, "r2", 1, "plan", "semantics-r2-p1");
    game.assert_kv("prop.2.status", json!("pending"));
    game.assert_kv("req.2.job", json!(""));
    game.assert_kv("req.2.gate", json!("plan"));
    // r3: add → dispatch (its plan job is running, no proposal yet).
    game.narrate(
        "requirement.add",
        &text("third requirement|synthetic description|semantics-r3"),
    );
    game.narrate("requirement.dispatch", "0");
    game.assert_kv("req.3.job", json!("plan"));

    let reply = game.click("gate.approve — ");
    let narrative = narrative_lines(&reply);
    assert!(
        narrative
            .iter()
            .any(|line| line.starts_with("[AUTO-DISPATCH]")),
        "approving r1 must auto-dispatch its development in the same turn: {narrative:#?}"
    );
    game.assert_kv("req.1.plan_ok", json!(true));
    game.assert_kv("req.1.job", json!("development"));
    game.assert_kv("req.1.lane", json!(1));
    // The neighbours are untouched: r2 keeps its own plan gate, r3's job keeps running.
    game.assert_kv("prop.2.status", json!("pending"));
    game.assert_kv("req.2.gate", json!("plan"));
    assert_eq!(game.phase("r2"), "plan_gate");
    game.assert_kv("req.3.job", json!("plan"));
}

/// The plan gate's own pending proposal does **not** block its approval (剧情双层重写: delivery is
/// the gate — the gate-raising proposal settles to accepted as part of the approval), while
/// an injected **development** pending does (one gate at a time; the KV-injected fixture is
/// the only way a proposal can coexist with a gate, since `submit_slot_ok` refuses live
/// submits while a gate is up).
#[test]
fn gate_approval_settles_its_own_proposal_and_still_blocks_foreign_pendings() {
    let mut game = drive_to_plan_gate();
    // p1 (the gate-raising plan proposal) is pending; the approval must proceed and settle it.
    game.assert_kv("prop.1.status", json!("pending"));
    game.click("gate.approve — ");
    game.assert_kv("prop.1.status", json!("accepted"));
    game.assert_kv("req.1.gate", json!(""));
    game.assert_kv("req.1.job", json!("development"));

    // Inject a pending development proposal (in-memory fixture) while the row sits on a
    // fresh plan gate: the approval is refused — a pending proposal must be served first.
    let mut plan_gate = drive_to_plan_gate();
    let mut save = plan_gate.save();
    let entries = save["kv"]["entries"]
        .as_object_mut()
        .expect("kv.entries must be an object");
    let injected = 2;
    entries.insert("prop.count".to_owned(), json!(injected));
    for (key, value) in [
        ("req", json!(1)),
        ("rev", json!(1)),
        ("stage", json!("development")),
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
        bundle: plan_gate.bundle.clone(),
        kernel: plan_gate
            .bundle
            .restore(&bytes)
            .expect("a save with an injected own proposal must load"),
        dir: plan_gate.dir.clone(),
        suppress_reads: false,
        stamp_actor: Some(TEST_ACTOR.to_owned()),
    };
    blocked.assert_kv("prop.2.status", json!("pending"));

    // The snapshot's pendingChoice predates the injection; re-render first to resync.
    blocked.narrate("requirement.list", "0");

    let refused = blocked.click("gate.approve — ");
    let lines = narrative_lines(&refused);
    assert_refused(&lines, "gate.approve");
    blocked.assert_kv("req.1.gate", json!("plan"));
    blocked.assert_kv("req.1.status", json!("open"));
    blocked.assert_kv("prop.2.status", json!("pending"));
}

/// The integration act (R5/R6/R7): only `integration.confirm` / `integration.reject` act on
/// an integrating row; the confirm validates the `develop@<sha>` evidence (7-40 hex), writes
/// the true state `done` and releases the lane; the reject falls back to `ready` (plan_ok
/// kept) with feedback and the lane released. Both are refused outside the coordinate phase
/// and outside the integrating micro-state.
#[test]
fn integration_confirm_sha_gate_and_reject_path() {
    let mut game = drive_to_integrating(drive_to_accept(drive_to_dev_pending("semantics-p2")));

    // Evidence shape: the sha must be 7-40 hex (with or without the develop@ prefix).
    for evidence in ["develop@zzzz", "develop@abc", "abc", "develop@"] {
        let bad = game.narrate("integration.confirm", &text(&format!("r1|{evidence}")));
        assert_refused(&bad, "integration.confirm");
        game.assert_kv("req.1.status", json!("open"));
        game.assert_kv("req.1.integrating", json!(true));
    }

    // The act only reaches integrating rows: r2 is still a draft.
    game.narrate(
        "requirement.add",
        &text("bystander requirement|desc|semantics-bystander"),
    );
    let bystander = game.narrate("integration.confirm", &text("r2|develop@abc1234"));
    assert_refused(&bystander, "integration.confirm");

    // Confirm: done (true state), the sha evidence is kept, the lane is released.
    let confirmed = game.narrate("integration.confirm", &text("r1|develop@abc1234"));
    assert!(
        confirmed
            .iter()
            .any(|line| line.contains("[INTEGRATION CONFIRMED]")),
        "{confirmed:#?}"
    );
    game.assert_kv("req.1.status", json!("done"));
    game.assert_kv("req.1.develop_sha", json!("abc1234"));
    game.assert_kv("req.1.lane", json!(0));
    game.assert_kv("req.1.closed_reason", json!(""));

    // The reject path: walk a second requirement to integrating, then reject it.
    game.narrate("requirement.dispatch", "0");
    submit_req(&mut game, "r2", 1, "plan", "semantics-bystander-p1");
    game.click("gate.approve — ");
    submit_req(&mut game, "r2", 1, "development", "semantics-bystander-p2");
    game.click("proposal.accept — ");
    game.click("gate.approve — ");
    game.assert_kv("req.2.lane", json!(1));
    let rejected = game.narrate("integration.reject", &text("r2|rework the diff"));
    assert!(
        rejected
            .iter()
            .any(|line| line.contains("[INTEGRATION REJECTED]")),
        "{rejected:#?}"
    );
    game.assert_kv("req.2.status", json!("open"));
    game.assert_kv("req.2.integrating", json!(false));
    game.assert_kv("req.2.plan_ok", json!(true));
    game.assert_kv("req.2.lane", json!(0));
    assert_eq!(game.phase("r2"), "ready");

    // A done row only accepts read-back: integration on it is refused.
    let done = game.narrate("integration.confirm", &text("r1|develop@abc1234"));
    assert_refused(&done, "integration.confirm");
}

/// The closed-reason bypass (R11): split/merge close the parents with `status=done` plus a
/// `closed_reason` field (the display shows the reason); a bypassed row is not amendable;
/// amending a **delivered** done row derives a successor instead of reopening it.
#[test]
fn closed_reason_bypass_split_and_done_amend_derives() {
    let mut game = Game::open(&project_mod_dir());
    charter_and_begin(&mut game);
    // A pre-development draft with no job in flight (split/merge need a mutable row).
    game.narrate(
        "requirement.add",
        &text("split me|synthetic description|semantics-split"),
    );

    // Split a pre-development draft: the parent becomes done + closed_reason=split.
    let split = game.narrate("requirement.split", &text("r1|first child|second child"));
    assert!(
        split.iter().any(|line| line.contains("[REQ SPLIT]")),
        "{split:#?}"
    );
    game.assert_kv("req.1.status", json!("done"));
    game.assert_kv("req.1.closed_reason", json!("split"));
    assert_eq!(game.phase("r1"), "split");

    // A bypassed row is not amendable (split/merged/dropped are terminal with a reason).
    let bypassed = game.narrate("requirement.amend", &text("r1|reopen attempt"));
    assert_refused(&bypassed, "requirement.amend");

    // Merge two open rows: the sources become done + closed_reason=merged, a new row appears.
    game.narrate(
        "requirement.merge",
        &text("r2,r3|merged requirement|merged desc"),
    );
    game.assert_kv("req.2.status", json!("done"));
    game.assert_kv("req.2.closed_reason", json!("merged"));
    game.assert_kv("req.3.status", json!("done"));
    game.assert_kv("req.3.closed_reason", json!("merged"));
    assert_eq!(game.phase("r4"), "draft");

    // Deliver r4 fully (dispatch by target id), then amend the done row: a successor is
    // derived (origin=r4).
    game.narrate("requirement.dispatch", &text("r4"));
    submit_req(&mut game, "r4", 1, "plan", "semantics-derive-p1");
    game.click("gate.approve — ");
    submit_req(&mut game, "r4", 1, "development", "semantics-derive-p2");
    game.click("proposal.accept — ");
    game.click("gate.approve — ");
    game.narrate("integration.confirm", &text("r4|develop@feed777"));
    game.assert_kv("req.4.status", json!("done"));
    game.assert_kv("req.4.closed_reason", json!(""));
    let derived = game.narrate("requirement.amend", &text("r4|follow-up title"));
    assert!(
        derived.iter().any(|line| line.contains("[REQ DERIVED]")),
        "{derived:#?}"
    );
    game.assert_kv("req.4.status", json!("done"));
    game.assert_kv("req.5.origin", json!("r4"));
    assert_eq!(game.phase("r5"), "draft");
}

// ───────────────────── r1 · single operation map + registry fail-closed ─────────────────────

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

        // Single view: head carries the current micro-state + panel, foot names the
        // operation view and the payload negation; the design view head is unreachable.
        let joined = window.join("\n");
        assert!(
            window[0].starts_with("[CURRENT NODE] ["),
            "the map head must carry the current micro-state: {}",
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
    assert!(
        set.contains(&"charter.set".to_owned()) && set.contains(&"run.begin".to_owned()),
        "the registered set must carry the charter-act pair: {message}"
    );
    assert!(
        set.contains(&"integration.confirm".to_owned())
            && set.contains(&"release.confirm".to_owned()),
        "the registered set must carry the new 剧情双层重写 acts: {message}"
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
    // 剧情双层重写: 38 story/operate/infrastructure events (DESIGN §2.6) + `ygg.narrative.goto`
    // (the node:render render event) = 39 registered handlers; r1 (泳道分池) adds
    // `lane.pool` (the named-pool registry action) = 40 registered handlers.
    assert_eq!(
        count, 40,
        "the merged mod registers 40 events after r1 lane.pool — got {count}: {message}"
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

// ───────────────────── r11 · unknown events fail closed ─────────────────────

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
    game.assert_kv("req.1.status", json!("open"));
    game.assert_kv("req.1.job", json!("plan"));
}

// ───────────────────── r2 · lane branch naming is enforced ─────────────────────

/// r2 The lane branch **must** be self-reported in the forced shape
/// `feature/{{4-char uuid}}-{{semantic segment}}` (≤ 48 total): `lane.claim` accepts a legal
/// name into the lane ledger, refuses every illegal shape through the readable rejection
/// channel (reject ledger moved), and an **empty** branch stays legal at the claim layer
/// (the auto-claim layer refuses it instead — the default name generator is gone).
#[test]
fn lane_branch_naming_is_enforced() {
    let mut game = Game::open(&project_mod_dir());
    start_requirement(&mut game);

    // Legal shape: claimed into the ledger byte-for-byte, with its own narration.
    let claimed = game.narrate(
        "lane.claim",
        &text("1|r1|feature/aa11-semantic-name|abc1234|1"),
    );
    assert_eq!(count_lines(&claimed, "[LANE CLAIMED]"), 1, "{claimed:#?}");

    // The refusal matrix (one entry per rule). Every refusal moves the reject ledger and
    // leaves the lane ledger untouched.
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

// ───────────────────── r3 · submitter identity (actor chapter + profiles) ─────────────────────

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
    // r9：**签名与提交同身份**——调用方声明的 actor 就是本步的执行者，补签也必须以它签，
    // 否则按人门禁会把「自己签的」看成「别人签的」（传 None = 老宿主无章，补签也不盖章）。
    let previous = game.stamp_actor.clone();
    game.as_actor(actor);
    game.ensure_reads("proposal.submit", Some(stage), req_index(req));
    let lines = game.narrate("proposal.submit", &text(&payload));
    game.stamp_actor = previous;
    lines
}

/// verified：exec 与宿主章同句柄 → 双章落库；save/重放后 KV 仍可读。
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
        dir: game.dir.clone(),
        suppress_reads: false,
        stamp_actor: Some(TEST_ACTOR.to_owned()),
    };
    rebuilt.assert_kv("prop.1.actor", json!("agent-1"));
    rebuilt.assert_kv("prop.1.actor_session", json!("s-plan-1"));
    rebuilt.assert_kv("settings.executor.r1.plan", json!("agent-1"));
}

/// mismatch：exec 与章不同句柄 → **识别不拒绝**（章照记、账本照绑、门禁照常可达）。
/// plan 提案交付即门禁：proposal.accept 对它可读拒绝（裁决走 gate.*），门禁保持挂起。
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
    // 交付即门禁：门禁已挂起；受理菜单隐瞒 plan 提案（prop_next 只表 development），
    // 直派 proposal.accept 被人机门可读拒绝——裁决只能走 gate.*。
    assert_eq!(game.phase("r1"), "plan_gate");
    let options = game.options();
    assert!(
        !options
            .iter()
            .any(|item| item.starts_with("proposal.accept — ")),
        "a plan proposal must not be on the acceptance menu: {options:?}"
    );
    let direct = game.narrate("proposal.accept", &text("p1"));
    assert_refused(&direct, "proposal.accept");
    game.assert_kv("req.1.gate", json!("plan"));
}

/// unverified + actor-only：r9 起「有 exec 无章」不再是可推进态——无身份就无法签名，
/// 也就无法自证「执行这个动作的人读过」（拒签 + 拒绝推进，零写入）；
/// 只带章不带 exec 仍合法（只记章、不绑账本）；账本绑定不受「打回」影响（账本只增改）。
#[test]
fn actor_channel_unverified_and_actor_only() {
    let mut game = Game::open(&project_mod_dir());
    start_requirement(&mut game);

    // 本段模拟「旧宿主 / 没带 --actor」：整个步骤无身份。
    game.as_actor(None);

    // unverified：exec 有、章无（旧宿主 / 直接派发面）→ r9 拒。
    let refused = submit_with_identity(&mut game, 1, "plan", "id-unv", Some("agent-1"), None, None);
    assert_refused(&refused, "read-gate/no-identity");
    game.assert_kv("prop.1.req", Json::Null);
    game.assert_kv("settings.executor.r1.plan", Json::Null);
    // 无身份时签名本身也被拒（“不知道谁签的”的签名在判定里等于不存在）。
    let unsigned = game.narrate("config.set", &text("read.acts/coordinate=aaaaaa"));
    assert!(
        unsigned.iter().any(|line| line.contains("SIGNED REFUSED")),
        "a signature without an actor must be refused: {unsigned:#?}"
    );
    game.assert_kv("settings.read.acts/coordinate", Json::Null);

    // 恢复正常身份，r1 的账本绑定不受「打回」影响（账本只增改）；plan 提案打回清门禁、job 挂回 plan。
    game.as_actor(Some(TEST_ACTOR));
    submit_with_identity(
        &mut game,
        1,
        "plan",
        "id-plan",
        Some("agent-1"),
        Some(TEST_ACTOR),
        None,
    );
    game.narrate("proposal.reject", &text("r1|replan without identity"));
    game.assert_kv("settings.executor.r1.plan", json!("agent-1"));
    game.assert_kv("req.1.gate", json!(""));
    game.assert_kv("req.1.job", json!("plan"));

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

/// 段序契约（mod 侧半）：错位 / 重复 `exec=` 逐段点名拒绝；全链 10 段载荷
/// （7 段主体 + exec + actor + actor_session）各归其位。
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
    // r4 开闸：本用例直派提交（不走 submit* helper），补签一行以穿读签门禁。
    // r9：签名与提交必须**同身份**——载荷章是 agent-1，补签也必须以 agent-1 签，
    // 否则按人门禁会把「自己签的」判成「别人签的」（other-actor 拒绝）。
    game.as_actor(Some("agent-1"));
    game.ensure_reads("proposal.submit", Some("plan"), Some(1));
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
    // 键形坏（缺 <key> 段）→ parse 层拒绝且不落 KV。
    let malformed = game.narrate("config.set", &text("profile.agent-1=x"));
    assert_refused(&malformed, "config.set");
    game.assert_kv("settings.profile.agent-1", Json::Null);
    // 真未知键 → fail-closed（config_keys 面同步拒绝）。
    let alien = game.narrate("config.set", &text("profile_index=agent-x"));
    assert_refused(&alien, "config.set");
    game.assert_kv("settings.profile_index", json!("agent-1"));
}

/// 幕前必读签名（r3 销票制）：`config.set read.<act>/<doc-id>=<ver>` 落 `settings.read.*`，
/// 值形状 `ver|签名幕|seq`，平铺索引 `settings.read_index` 同步；同键重签是幂等 upsert。
/// 覆盖面只有在 `readPlan` 已烘进 `data/config.json` 时才算——缺表即降级（不拒签）。
#[test]
fn read_signature_records_ver_scene_and_index() {
    let mut game = Game::open(&project_mod_dir());
    charter_and_begin(&mut game); // 协调幕（run.phase = "" → scene = coordinate）

    let signed = game.narrate("config.set", &text("read.acts/coordinate=9f2c1a"));
    assert_eq!(
        count_lines(&signed, "[SIGNED] acts/coordinate@9f2c1a"),
        1,
        "{signed:#?}"
    );
    game.assert_kv(
        "settings.read.acts/coordinate",
        json!("9f2c1a|coordinate|0"),
    );
    game.assert_kv("settings.read_index", json!("acts/coordinate"));

    // `settings.` 前缀写法等价；同键重签覆盖 ver，索引不重复登记。
    game.narrate("config.set", &text("settings.read.acts/coordinate=aa11bb"));
    game.assert_kv(
        "settings.read.acts/coordinate",
        json!("aa11bb|coordinate|0"),
    );
    game.assert_kv("settings.read_index", json!("acts/coordinate"));

    // 第二件：索引追加，两件互不覆盖。
    game.narrate("config.set", &text("read.coordinate/dispatch=c0ffee"));
    game.assert_kv(
        "settings.read.coordinate/dispatch",
        json!("c0ffee|coordinate|0"),
    );
    game.assert_kv(
        "settings.read_index",
        json!("acts/coordinate,coordinate/dispatch"),
    );

    // 缺 readPlan → 降级：回执报件数、不报 k/n，且**不拒签**。
    // 开闸后入库件带 readPlan，故这条降级路径在「剥掉 readPlan」的副本上验。
    let bare_dir = support::temp_dir("read-unplotted");
    support::copy_dir(&project_mod_dir(), &bare_dir);
    let mut bare_config = support::read_json(&bare_dir.join("data/config.json"));
    bare_config
        .as_object_mut()
        .expect("config object")
        .remove("readPlan");
    support::write_json(&bare_dir.join("data/config.json"), &bare_config);
    support::reseal(&bare_dir);
    let mut bare = Game::open(&bare_dir);
    charter_and_begin(&mut bare);
    let unplotted = bare.narrate("config.set", &text("read.acts/charter=123456"));
    assert_eq!(
        count_lines(&unplotted, "no read plan is baked for this act"),
        1,
        "{unplotted:#?}"
    );
    bare.assert_kv("settings.read.acts/charter", json!("123456|coordinate|0"));
}

/// 非法键形 / 非法 ver 一律 `[REJECTED:PAYLOAD]` 且不落 KV；未知键仍 fail-closed
///（白名单扩张没有把 `bad_config_key` 那条路径堵死）。
#[test]
fn read_signature_rejects_bad_doc_id_and_bad_ver() {
    let mut game = Game::open(&project_mod_dir());
    charter_and_begin(&mut game);

    let long_doc = format!("read.acts/{}=9f2c1a", "a".repeat(49));
    let long_scene = format!("read.{}/coordinate=9f2c1a", "a".repeat(33));
    for bad in [
        "read.acts=9f2c1a",            // 缺 <doc-id> 段（无斜杠）
        "read.acts/sub/dir=9f2c1a",    // 斜杠过多（只认一段）
        "read.acts/charter.md=9f2c1a", // 点号：effective() 会下钻错位（合成键形，非真实 doc-id）
        "read.Acts/Coordinate=9f2c1a", // 大写
        "read./coordinate=9f2c1a",     // 幕段空
    ] {
        let lines = game.narrate("config.set", &text(bad));
        assert_refused(&lines, "config.set");
    }
    for bad in [long_doc.as_str(), long_scene.as_str()] {
        let lines = game.narrate("config.set", &text(bad));
        assert_refused(&lines, "config.set");
    }
    assert_eq!(game.kv("settings.read_index"), Json::Null);
    assert_eq!(game.kv("settings.read.acts"), Json::Null);

    for bad in [
        "read.acts/coordinate=xyz",    // 非 hex
        "read.acts/coordinate=9F2C1A", // 大写 hex 不收
        "read.acts/coordinate=abc",    // 短于 4
    ] {
        let lines = game.narrate("config.set", &text(bad));
        assert_refused(&lines, "config.set");
    }
    assert_eq!(game.kv("settings.read.acts/coordinate"), Json::Null);

    // 索引键不在用户面（无 read. 前缀 → 落 bad_config_key），只能由 mod 自维护。
    let alien = game.narrate("config.set", &text("read_index=acts/coordinate"));
    assert_refused(&alien, "config.set");
    assert_eq!(game.kv("settings.read_index"), Json::Null);
}

/// 值 = 显式清除该件（与 run.retire / goal.authorization 同构）：值置空 + 索引摘除；
/// 重复清除幂等（不产生拒绝行，也不留垃圾键）。
#[test]
fn read_signature_clears_with_an_empty_value() {
    let mut game = Game::open(&project_mod_dir());
    charter_and_begin(&mut game);
    game.narrate("config.set", &text("read.acts/coordinate=9f2c1a"));
    game.narrate("config.set", &text("read.coordinate/dispatch=c0ffee"));
    game.assert_kv(
        "settings.read_index",
        json!("acts/coordinate,coordinate/dispatch"),
    );

    let cleared = game.narrate("config.set", &text("read.acts/coordinate="));
    assert_eq!(
        count_lines(&cleared, "[SIGN] acts/coordinate cleared"),
        1,
        "{cleared:#?}"
    );
    game.assert_kv("settings.read.acts/coordinate", json!(""));
    game.assert_kv("settings.read_index", json!("coordinate/dispatch"));

    // 幂等：再清一次既不拒绝也不改索引。
    let again = game.narrate("config.set", &text("read.acts/coordinate="));
    assert!(
        !again.iter().any(|line| line.contains("REJECTED")),
        "{again:#?}"
    );
    game.assert_kv("settings.read_index", json!("coordinate/dispatch"));
}

/// 销票：作用域边界**主动清除**全部签名，于是「键非空」本身就等于「本作用域内已签」——
/// 消费者（r4）不需要时钟、不需要比较序号。覆蓋三类边界：立局→协调（run.begin）、
/// rev 推进（requirement.amend，停在 req_bump_rev）、提案打回（proposal.reject）。
#[test]
fn read_scope_boundary_wipes_every_signature() {
    let mut game = Game::open(&project_mod_dir());

    // ① run.begin（立局幕 → 协调幕）
    game.narrate("config.set", &text("read.acts/charter=aaaaaa"));
    game.assert_kv("settings.read.acts/charter", json!("aaaaaa|charter|0"));
    charter_and_begin(&mut game);
    game.assert_kv("settings.read.acts/charter", json!(""));
    game.assert_kv("settings.read_index", json!(""));

    // 一条在册需求（协调幕；不动 phase）
    game.narrate(
        "requirement.add",
        &text("synthetic requirement|synthetic description|semantics-r1"),
    );
    game.narrate("requirement.dispatch", "0");

    // ② requirement.amend 推进 rev → 销票（amend 三条成功路径都过 req_bump_rev）
    game.narrate("config.set", &text("read.acts/coordinate=9f2c1a"));
    game.narrate("config.set", &text("read.coordinate/dispatch=c0ffee"));
    game.assert_kv(
        "settings.read_index",
        json!("acts/coordinate,coordinate/dispatch"),
    );
    let amended = game.narrate("requirement.amend", &text("r1||amended description"));
    assert!(!refused(&amended, "requirement.amend"), "{amended:#?}");
    game.assert_kv("settings.read.acts/coordinate", json!(""));
    game.assert_kv("settings.read.coordinate/dispatch", json!(""));
    game.assert_kv("settings.read_index", json!(""));

    // ③ proposal.reject 打回（plan 分支不碰 rev，靠 handler 里的显式 clear）
    game.narrate("config.set", &text("read.acts/coordinate=9f2c1a"));
    game.assert_kv("settings.read_index", json!("acts/coordinate"));
    submit(&mut game, 2, "plan", "semantics-read-plan");
    let rejected = game.narrate("proposal.reject", &text("p1|needs rework"));
    assert!(!refused(&rejected, "proposal.reject"), "{rejected:#?}");
    // 成功回执带 [PROPOSAL REJECTED] 前缀（它不是拒绝行：拒绝前缀见 pack.json）。
    assert_eq!(
        count_lines(&rejected, "[PROPOSAL REJECTED] p1 (r1)"),
        1,
        "{rejected:#?}"
    );
    game.assert_kv("settings.read.acts/coordinate", json!(""));
    game.assert_kv("settings.read_index", json!(""));
}

/// 覆盖面 k/n：`readPlan` 在场时按本幕清单计算（计划外的 doc 不进分子），
/// 不在场时降级。清单是**装资源的静态值**，改它必须重算 manifest（`reseal`）。
#[test]
fn read_coverage_counts_planned_docs() {
    let dir = support::temp_dir("read-plan");
    support::copy_dir(&project_mod_dir(), &dir);
    let mut config = support::read_json(&dir.join("data/config.json"));
    config["readPlan"] = json!({
        "coordinate": ["acts/coordinate", "coordinate/dispatch"]
    });
    support::write_json(&dir.join("data/config.json"), &config);
    support::reseal(&dir);

    let mut game = Game::open(&dir);
    charter_and_begin(&mut game);

    let first = game.narrate("config.set", &text("read.acts/coordinate=9f2c1a"));
    assert_eq!(
        count_lines(&first, "this act has 1/2 docs signed, 1 of them by you"),
        1,
        "{first:#?}"
    );
    // r9：另一个代理签下第二件 —— 覆盖面 k 上升，但「我签的」不动。两个口径必须分得开：
    // 旧文案的 `2/2` 读起来像「我已签全」，而它从来只统计「本幕覆盖了几件」。
    game.as_actor(Some("other-agent"));
    let second = game.narrate("config.set", &text("read.coordinate/dispatch=c0ffee"));
    assert_eq!(
        count_lines(&second, "this act has 2/2 docs signed, 1 of them by you"),
        1,
        "{second:#?}"
    );
    game.assert_kv(
        "settings.read_actor.coordinate/dispatch",
        json!("other-agent"),
    );
    // 计划外的 doc：签名照旧成立，但不进分子（不拒签、不越界）。
    game.as_actor(Some(TEST_ACTOR));
    let extra = game.narrate("config.set", &text("read.acts/charter=aaaaaa"));
    assert_eq!(
        count_lines(&extra, "this act has 2/2 docs signed, 1 of them by you"),
        1,
        "{extra:#?}"
    );
}

// ───────────────────── r4 · read-gate: the four dimensions ─────────────────────

/// 造一份**独立受控**的 mod 副本（`readPlan` + `readGate` 换成用例自己的小台账）：
/// 入库件的台账跟着真实文档头走（ver 会变），门禁用例要的是稳定常量。
/// 改 `data/config.json` 必须重算 manifest（内容寻址）——`reseal` 就是这一步。
fn armed_mod(tag: &str) -> PathBuf {
    let dir = support::temp_dir(tag);
    support::copy_dir(&project_mod_dir(), &dir);
    let mut config = support::read_json(&dir.join("data/config.json"));
    config["readPlan"] = json!({
        "coordinate": ["acts/coordinate"],
        "settle": ["acts/settle", "rituals/cleanup"]
    });
    config["readGate"] = json!({
        "ver": {
            "acts/coordinate": "aaaaaa",
            "req-acts/intake": "bbbbbb",
            "req-acts/delivery": "cccccc",
            "acts/settle": "dddddd",
            "rituals/cleanup": "eeeeee"
        },
        "reqActs": { "plan": ["req-acts/intake"], "development": ["req-acts/delivery"] },
        "gates": { "plan": ["req-acts/delivery"], "accept": ["req-acts/delivery"] }
    });
    support::write_json(&dir.join("data/config.json"), &config);
    support::reseal(&dir);
    dir
}

/// 一份已立局、已挂一条需求并派发（`job=plan`）的局：r4 三动作的共同起跑线。
fn armed_game(tag: &str) -> Game {
    let mut game = Game::open(&armed_mod(tag));
    charter_and_begin(&mut game);
    game.narrate(
        "requirement.add",
        &text("armed requirement|armed description|armed-r1"),
    );
    game.narrate("requirement.dispatch", "0");
    game
}

/// 签一件（值段是 r3 的 `ver|scene|seq` 形状，ver 必须与台账逐字相等）。
fn sign(game: &mut Game, doc: &str, ver: &str) {
    game.narrate("config.set", &text(&format!("read.{doc}={ver}")));
}

/// ① 覆盖：缺签直接拒，零副作用，且拒台词自带「读什么 + 跑什么命令 + 窗口多大」。
#[test]
fn read_gate_missing_signature_blocks_a_proposal_with_zero_side_effects() {
    let mut game = armed_game("gate-missing");
    let lines = game.narrate(
        "proposal.submit",
        &text("r1|1|plan|armed plan|artifacts/r1.md"),
    );
    assert_refused(&lines, "proposal.submit");
    let joined = lines.join("\n");
    assert!(
        joined.contains("read-gate/missing"),
        "the refusal must name the dimension: {lines:#?}"
    );
    // 三件事：确切的签名命令（逐件）、当前窗口值、已隔派发数。
    assert!(
        joined.contains("config.set read.acts/coordinate=aaaaaa")
            && joined.contains("config.set read.req-acts/intake=bbbbbb"),
        "each missing doc must come with its exact sign command: {lines:#?}"
    );
    assert!(
        joined.contains("window 80") && joined.contains("dispatch(es) ago"),
        "the refusal must report the window and the elapsed dispatches: {lines:#?}"
    );
    // 零副作用：门禁没升起、提案没落库、签名账本没动（派发计数器前进不算副作用）。
    game.assert_kv("req.1.gate", json!(""));
    game.assert_kv("req.1.job", json!("plan"));
    game.assert_kv("prop.1.status", Json::Null);
    game.assert_kv("settings.read_index", Json::Null);
}

/// ② 新鲜：阶段边界之后旧签名作废（跨阶段读 = 没读），拒台词报两个 seq。
#[test]
fn read_gate_voids_a_signature_taken_before_the_stage_edge() {
    let mut game = armed_game("gate-stale");
    sign(&mut game, "acts/coordinate", "aaaaaa");
    sign(&mut game, "req-acts/intake", "bbbbbb");
    game.narrate(
        "proposal.submit",
        &text("r1|1|plan|armed plan|artifacts/r1.md"),
    );
    game.assert_kv("req.1.gate", json!("plan"));
    // plan 批准把需求推进到 development 阶段：纪元前进。
    sign(&mut game, "req-acts/delivery", "cccccc");
    game.click("gate.approve — ");
    game.assert_kv("req.1.plan_ok", json!(true));

    let lines = game.narrate(
        "proposal.submit",
        &text("r1|1|development|armed dev|artifacts/r1-dev.md"),
    );
    let joined = lines.join("\n");
    assert_refused(&lines, "proposal.submit");
    assert!(
        joined.contains("read-gate/stale") && joined.contains("stage development began at"),
        "the refusal must name the stage edge that voided the signature: {lines:#?}"
    );
    assert!(
        joined.contains("was signed at dispatch") && joined.contains("Window 80"),
        "the refusal must report sign seq, entry seq and the window: {lines:#?}"
    );
}

/// ③ 遗忘：窗口是**可配**的；超窗即拒，抬高窗口或重签都能解除。
#[test]
fn read_gate_reports_the_forget_distance_past_the_window() {
    let mut game = armed_game("gate-forgotten");
    game.narrate("config.set", &text("readGate.window=0"));
    sign(&mut game, "acts/coordinate", "aaaaaa");
    sign(&mut game, "req-acts/intake", "bbbbbb");
    let lines = game.narrate(
        "proposal.submit",
        &text("r1|1|plan|armed plan|artifacts/r1.md"),
    );
    let joined = lines.join("\n");
    assert_refused(&lines, "proposal.submit");
    assert!(
        joined.contains("read-gate/forgotten") && joined.contains("window 0"),
        "the refusal must report the configured window: {lines:#?}"
    );
    assert!(
        joined.contains("dispatch(es) ago") && joined.contains("config.set readGate.window="),
        "the refusal must report the gap and the raise command: {lines:#?}"
    );
    // 解除路径：抬高窗口 → 同一动作通过。
    game.narrate("config.set", &text("readGate.window=80"));
    let ok = game.narrate(
        "proposal.submit",
        &text("r1|1|plan|armed plan|artifacts/r1.md"),
    );
    assert!(
        !refused(&ok, "proposal.submit"),
        "raising the window must release the blocker: {ok:#?}"
    );
    game.assert_kv("req.1.gate", json!("plan"));
}

/// **r11 的证据**：amend 推进 rev 时 r3 的销票制先把签名全抹掉，所以下一次推进动作报的是
/// **覆盖维** `read-gate/missing`（分层而非重复报错）。
/// 这条用例是「主体维退役」的机械依据：rev 一变签名已不存在，「签名早于新 rev」永远轮不到
/// 判定——所以主体维（`read-gate/subject`）测不到 amend，它是假保护。
#[test]
fn read_gate_amendment_loses_signatures_through_the_clear_not_a_subject_dimension() {
    let mut game = armed_game("gate-amend");
    sign(&mut game, "acts/coordinate", "aaaaaa");
    sign(&mut game, "req-acts/intake", "bbbbbb");
    // amend 推进 rev：销票制先抹掉全部签名，所以此后报覆盖（missing）——不双报、不错报。
    let amended = game.narrate("requirement.amend", &text("r1||amended description"));
    assert!(!refused(&amended, "requirement.amend"), "{amended:#?}");
    game.assert_kv("settings.read_index", json!(""));
    let after = game.narrate(
        "proposal.submit",
        &text("r1|2|plan|replanned|artifacts/r1.md"),
    );
    let joined = after.join("\n");
    assert!(
        refused(&after, "proposal.submit") && joined.contains("read-gate/missing"),
        "an amendment wipes signatures through r3's clear(), so the refusal is coverage: {after:#?}"
    );
    assert!(
        !joined.contains("read-gate/subject"),
        "r11: the amend path must report coverage only — the subject dimension is retired: {after:#?}"
    );
}

/// **r11 守护（可证伪）**：主体维已退役。复刻它当年「唯一可达」的路径——先签名，再
/// `requirement.add` 一条新需求（该需求的 rev/stage 纪元因此晚于签名）——现在必须**绝不出现**
/// `read-gate/subject`；同一场景由**新鲜维** `read-gate/stale` 接管（新需求的阶段纪元晚于签名），
/// 所以删掉主体维并没有开口子。
/// 变异探针：把主体维分支加回 `gates.read_problem`，这条立刻转红。
#[test]
fn read_gate_handles_a_signature_taken_before_a_new_requirement_without_a_subject_dimension() {
    let mut game = armed_game("gate-no-subject");
    sign(&mut game, "acts/coordinate", "aaaaaa");
    sign(&mut game, "req-acts/intake", "bbbbbb");
    game.narrate(
        "requirement.add",
        &text("second requirement|second description|armed-r2"),
    );
    game.narrate("requirement.dispatch", "0");
    let lines = game.narrate(
        "proposal.submit",
        &text("r2|1|plan|r2 plan|artifacts/r2.md"),
    );
    let joined = lines.join("\n");
    assert!(
        !joined.contains("read-gate/subject"),
        "r11: the subject dimension is retired — no refusal may carry it: {lines:#?}"
    );
    assert!(
        refused(&lines, "proposal.submit") && joined.contains("read-gate/stale"),
        "the freshness dimension must take over the old subject scenario: {lines:#?}"
    );
}

/// ⑤ 重签后放行：拒台词给的命令原样执行即解除（证明拒绝面自带自救路径，不靠外部知识）。
#[test]
fn read_gate_passes_after_reading_and_resigning() {
    let mut game = armed_game("gate-resign");
    sign(&mut game, "acts/coordinate", "aaaaaa");
    sign(&mut game, "req-acts/intake", "bbbbbb");
    let lines = game.narrate(
        "proposal.submit",
        &text("r1|1|plan|armed plan|artifacts/r1.md"),
    );
    assert!(!refused(&lines, "proposal.submit"), "{lines:#?}");
    game.assert_kv("req.1.gate", json!("plan"));
    // 序号是 KV 标量（读档可复算）：签名快照非零、派发计数单调。
    let sign_seq = game.kv("settings.sign_seq.acts/coordinate");
    assert!(
        sign_seq.as_u64().unwrap_or(0) > 0,
        "mark_sign must record a real dispatch sequence: {sign_seq}"
    );
    let clock = game.kv("settings.clock_seq");
    assert!(
        clock.as_u64().unwrap_or(0) > 0,
        "the dispatch counter must live in KV: {clock}"
    );
}

/// ⑥ 入库态双断言：① 生成区**已开闸**（readPlan / readGate 真实存在，且 ver 台账与
/// 文档头逐件相等）；② 把生成区整体剥掉的副本仍然 **inert**（不拦、不叙事）——
/// 「缺席即安全阀」这条路径不能因为开闸而失去回归保护。
#[test]
fn read_gate_ships_armed_and_a_bare_copy_stays_inert() {
    let config = support::read_json(&project_mod_dir().join("data/config.json"));
    let plan = config["readPlan"]
        .as_object()
        .expect("the shipped config must carry readPlan (the gate is armed in the repo)");
    assert!(
        !plan.is_empty() && plan.contains_key("coordinate") && plan.contains_key("settle"),
        "readPlan must cover the acts the gate guards: {plan:?}"
    );
    let ledger = config["readGate"]["ver"]
        .as_object()
        .expect("the shipped config must carry readGate.ver");
    assert!(
        ledger.len() >= 5,
        "the version ledger must cover the guarded docs: {ledger:?}"
    );
    // 台账 == 文档头（单一事实源：generator 从同一批文件头产出）。
    for (doc, ver) in ledger {
        let path = repo_dir().join("docs/context").join(format!("{doc}.md"));
        let head = std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {doc}: {err}"));
        let declared = head
            .lines()
            .find(|line| line.contains("**ver**"))
            .and_then(|line| line.split("**ver**").nth(1))
            .and_then(|tail| tail.split_whitespace().next())
            .unwrap_or_else(|| panic!("{doc}: no ver header"));
        assert_eq!(
            declared,
            ver.as_str().expect("ver string"),
            "readGate.ver.{doc} must equal the doc header (one source, generated)"
        );
    }

    // 剥掉生成区的副本：判据 inert 且静默。
    let bare_dir = support::temp_dir("read-gate-inert");
    support::copy_dir(&project_mod_dir(), &bare_dir);
    let mut bare_config = support::read_json(&bare_dir.join("data/config.json"));
    let bare_object = bare_config.as_object_mut().expect("config object");
    bare_object.remove("readGate");
    bare_object.remove("readPlan");
    support::write_json(&bare_dir.join("data/config.json"), &bare_config);
    support::reseal(&bare_dir);

    let mut game = Game::open(&bare_dir);
    charter_and_begin(&mut game);
    game.narrate(
        "requirement.add",
        &text("inert requirement|inert description|inert-r1"),
    );
    game.narrate("requirement.dispatch", "0");
    let lines = game.narrate(
        "proposal.submit",
        &text("r1|1|plan|inert plan|artifacts/r1.md"),
    );
    let joined = lines.join("\n");
    assert!(!refused(&lines, "proposal.submit"), "{lines:#?}");
    assert!(
        !joined.contains("read-gate/") && !joined.contains("[READ GATE]"),
        "an absent readset must be silent as well as inert: {lines:#?}"
    );
    game.assert_kv("req.1.gate", json!("plan"));
}

/// ⑧ 批边界（`coordinate.submit`）的端到端：未签被拒且**状态未推进**，补签后放行。
/// 必读集 = 结算幕清单（`acts/settle` + `rituals/cleanup`，由生成器写在 `readPlan.settle`）。
#[test]
fn read_gate_blocks_and_releases_the_batch_boundary() {
    let mut game = drive_to_done(drive_to_integrating(drive_to_accept(drive_to_dev_pending(
        "semantics-batch-gate",
    ))));
    // 关掉自动补签：这一条要的是「没签的现场」。
    game.suppress_reads = true;
    let refused_lines = narrative_lines(&game.click("coordinate.submit — "));
    assert_refused(&refused_lines, "coordinate.submit");
    let joined = refused_lines.join("\n");
    assert!(
        joined.contains("read-gate/missing")
            && joined.contains("acts/settle")
            && joined.contains("rituals/cleanup"),
        "the batch boundary must name the settle-act set it wants: {refused_lines:#?}"
    );
    // 状态未推进、零副作用：相位还停在协调幕，机证也没被登记。
    game.assert_kv("run.phase", json!(""));
    game.assert_kv("verify.passed", Json::Null);

    // 补签（恢复自动补签，走人类票据）→ 同一动作放行。
    game.suppress_reads = false;
    let passed = narrative_lines(&game.click("coordinate.submit — "));
    assert!(
        !refused(&passed, "coordinate.submit"),
        "after signing the batch boundary must release: {passed:#?}"
    );
    game.assert_kv("run.phase", json!("settle"));
}

/// ⑩ 批边界的身份载体缺失：`coordinate.submit` 只收空载荷 / "0" / "full"，宿主无处追加
/// `|actor=`，因此按人判定在这个动作上**结构不可满足**。直派路必须降级为覆盖检查并另立
/// 叙事告明原因——否则 goal 模式的批边界永远跳不过去。
/// 这条用例补的正是 r9 的缺口：既有批边界用例走菜单点击的**票据路**，覆盖不到直派路。
#[test]
fn batch_boundary_degrades_when_identity_has_no_carrier() {
    let mut game = drive_to_done(drive_to_integrating(drive_to_accept(drive_to_dev_pending(
        "semantics-batch-no-carrier",
    ))));
    // 直派路只在 goal 模式可达（guide 会先被人机门禁拒，走不到读签）。
    game.narrate("config.set", &text("play.mode=goal"));
    let lines = game.narrate("coordinate.submit", "0");
    let joined = lines.join("\n");
    assert!(
        joined.contains("no-carrier") && joined.contains("attribution is structurally unavailable"),
        "the batch boundary must narrate the missing identity carrier: {joined}"
    );
    assert!(
        !joined.contains("read-gate/no-identity"),
        "no-carrier must not degrade into a hard no-identity refusal: {joined}"
    );
    // 本次仍会因未签结算幕文档被**覆盖维**拦下（acts/settle / rituals/cleanup 未签）——
    // 降级只放弃「按人」，不放弃其余维度。goal 模式端到端能否收批由真实运行验证。
    assert!(
        joined.contains("read-gate/missing"),
        "coverage must still be enforced after the identity dimension degrades: {joined}"
    );
}

/// ⑪ ⑩ 的反证：降级只让「按人」这一维失效，**覆盖维仍然生效**——没签就照拒。
#[test]
fn batch_boundary_still_enforces_coverage_without_identity_carrier() {
    let mut game = drive_to_done(drive_to_integrating(drive_to_accept(drive_to_dev_pending(
        "semantics-batch-no-carrier-unsigned",
    ))));
    game.suppress_reads = true;
    game.narrate("config.set", &text("play.mode=goal"));
    let lines = game.narrate("coordinate.submit", "0");
    let joined = lines.join("\n");
    assert!(
        joined.contains("read-gate/missing") && joined.contains("acts/settle"),
        "coverage must still block the batch boundary: {joined}"
    );
    assert_ne!(
        game.kv("run.phase").as_str(),
        Some("settle"),
        "a coverage refusal must not advance the batch boundary"
    );
}

/// ⑨ 文档改版：签名本身没变，但台账 ver 前进了 → `read_gate_stale` 拒；按新 ver 重签后放行。
/// 现场构成：先签 aaaaaa → 存档 → 改 fixture 的 `readGate.ver` → 重装 mod 并 restore 存档。
/// 这正是「文档改版需重读」的机械载体（ver 台账与文档头同源）。
#[test]
fn read_gate_refuses_a_signature_against_a_revised_doc() {
    let dir = armed_mod("gate-revised");
    let mut game = Game::open(&dir);
    charter_and_begin(&mut game);
    game.narrate(
        "requirement.add",
        &text("revised requirement|revised description|revised-r1"),
    );
    game.narrate("requirement.dispatch", "0");
    sign(&mut game, "acts/coordinate", "aaaaaa");
    sign(&mut game, "req-acts/intake", "bbbbbb");
    let bytes = serde_json::to_vec(&game.save()).expect("serialize the save");

    // 文档「改版」：台账把 acts/coordinate 的 ver 推到 111111（等价于 docs 头被改）。
    let mut config = support::read_json(&dir.join("data/config.json"));
    config["readGate"]["ver"]["acts/coordinate"] = json!("111111");
    support::write_json(&dir.join("data/config.json"), &config);
    support::reseal(&dir);

    let bundle = ModBundle::load(&dir).expect("reload the revised fixture");
    let mut revised = Game {
        kernel: bundle
            .restore(&bytes)
            .expect("restore across the doc revision"),
        bundle,
        dir: dir.clone(),
        suppress_reads: true,
        stamp_actor: Some(TEST_ACTOR.to_owned()),
    };
    let lines = revised.narrate(
        "proposal.submit",
        &text("r1|1|plan|stale-ver plan|artifacts/r1.md"),
    );
    assert_refused(&lines, "proposal.submit");
    let joined = lines.join("\n");
    assert!(
        joined.contains("read-gate/stale")
            && joined.contains("the doc's ver moved from aaaaaa to 111111"),
        "a revised doc must void the signature and say which ver moved: {lines:#?}"
    );
    revised.assert_kv("req.1.gate", json!(""));

    // 按新 ver 重签 → 放行。
    revised.suppress_reads = false;
    revised.narrate("config.set", &text("read.acts/coordinate=111111"));
    let passed = revised.narrate(
        "proposal.submit",
        &text("r1|1|plan|stale-ver plan|artifacts/r1.md"),
    );
    assert!(!refused(&passed, "proposal.submit"), "{passed:#?}");
    revised.assert_kv("req.1.gate", json!("plan"));
}

/// ⑦ 不误伤：既有语义拒绝的文案优先，读签不得抢嘴（装订期严密与否不能改变拒因归属）。
#[test]
fn read_gate_does_not_mask_existing_rejection_reasons() {
    let mut game = armed_game("gate-nomask");
    sign(&mut game, "acts/coordinate", "aaaaaa");
    sign(&mut game, "req-acts/intake", "bbbbbb");
    game.narrate(
        "proposal.submit",
        &text("r1|1|plan|armed plan|artifacts/r1.md"),
    );
    // 待批期间 amend → gate_rev 与 rev 失配（既有 stale 判据）。零签名也在场，但先报既有原因。
    game.narrate("requirement.amend", &text("r1||amended while pending"));
    let lines = game.narrate("gate.approve", &text("r1"));
    let joined = lines.join("\n");
    assert!(!joined.contains("read-gate/"), "{lines:#?}");
    assert!(
        joined.contains("is bound to rev") || refused(&lines, "gate.approve"),
        "the pre-existing gate reason must win: {lines:#?}"
    );
}

// ───────────────── r9 · attribution: the signature belongs to an actor ─────────────────

/// r9 核心断言：**别人**签的不能当「你读过」——这正是本需求要修的缺陷（r6 子代理签名
/// 被主代理的 gate.approve 计入 `2/2 signed`）。同一份台账、同一份签名，换个人推进就必须拒。
#[test]
fn read_gate_refuses_a_signature_owned_by_another_actor() {
    let mut game = armed_game("gate-other-actor");
    // 另一个代理把本幕必读签齐（真路径上它就是当时的执行者）。
    game.as_actor(Some("r6-planner"));
    sign(&mut game, "acts/coordinate", "aaaaaa");
    sign(&mut game, "req-acts/intake", "bbbbbb");
    game.assert_kv("settings.read_actor.acts/coordinate", json!("r6-planner"));

    // 换成另一个代理推进：签名有效（ver 对、序号新），但不是它签的。
    game.as_actor(Some("main-agent"));
    let lines = game.narrate(
        "proposal.submit",
        &text("r1|1|plan|armed plan|artifacts/r1.md"),
    );
    let joined = lines.join("\n");
    assert_refused(&lines, "proposal.submit");
    assert!(
        joined.contains("read-gate/other-actor"),
        "another actor's signature must be reported as such (not as missing): {lines:#?}"
    );
    assert!(
        joined.contains("by r6-planner") && joined.contains("dispatched as main-agent"),
        "the refusal must name both the signer and the acting actor: {lines:#?}"
    );
    assert!(
        joined.contains("0 of them by you"),
        "the two counts must stay separate (coverage vs mine): {lines:#?}"
    );
    // 拒绝零副作用：提案不落库、门禁不挂（dispatch 时写下的空串保持不变）。
    game.assert_kv("prop.1.req", Json::Null);
    game.assert_kv("req.1.gate", json!(""));

    // 正面对照：换回签署人本人 → 同一份签名立刻生效（证明拒的是“不是你”，不是“签名无效”）。
    game.as_actor(Some("r6-planner"));
    let ok = game.narrate(
        "proposal.submit",
        &text("r1|1|plan|armed plan|artifacts/r1.md"),
    );
    assert!(!refused(&ok, "proposal.submit"), "{ok:#?}");
    game.assert_kv("req.1.gate", json!("plan"));
}

/// 造一个「计划门禁已挂起」的现场：以 `actor` 签齐本幕必读、交计划提案（交付即门禁），
/// 并把该门禁交出的下一幕规则（`req-acts/delivery`）也签上——门禁批的就是它。
fn armed_plan_gate(tag: &str, actor: &str) -> Game {
    let mut game = armed_game(tag);
    game.as_actor(Some(actor));
    sign(&mut game, "acts/coordinate", "aaaaaa");
    sign(&mut game, "req-acts/intake", "bbbbbb");
    sign(&mut game, "req-acts/delivery", "cccccc");
    let lines = game.narrate(
        "proposal.submit",
        &text("r1|1|plan|armed plan|artifacts/r1.md"),
    );
    assert!(!refused(&lines, "proposal.submit"), "{lines:#?}");
    assert_eq!(game.phase("r1"), "plan_gate");
    game
}

/// r9：无身份的直接派发**不能**推进（能带而不带 = 机器门被绕开）；guide 模式的人机门
/// 先报（更粗、更早），所以这里用 `play.mode=goal` 把那条门绕到背后去。
#[test]
fn read_gate_refuses_a_direct_approve_without_identity() {
    let mut game = armed_plan_gate("gate-no-identity", "r6-planner");
    game.narrate("config.set", &text("play.mode=goal"));
    game.as_actor(None);

    let lines = game.narrate("gate.approve", &text("r1"));
    let joined = lines.join("\n");
    assert_refused(&lines, "gate.approve");
    assert!(
        joined.contains("read-gate/no-identity"),
        "a direct advance without an actor must be refused: {lines:#?}"
    );
    assert!(
        joined.contains("YGG_ACTOR") && joined.contains("config.set read."),
        "the refusal must give the self-release path (identity flag + sign commands): {lines:#?}"
    );
    // 状态不动：门禁仍挂、计划未过。
    game.assert_kv("req.1.gate", json!("plan"));
    game.assert_kv("req.1.plan_ok", json!(false));
}

/// r9：**票据路降级**——人工菜单点击永远没有宿主身份段（`ygg.choice_selected` → emit）。
/// 此时按人判定不可得，于是降为覆盖检查，并把“不区分签署人”写进回执。
/// 这条用例同时是 `guide` 模式的防锁死线：若票据也 fail-closed，人类就再也过不了门。
#[test]
fn read_gate_ticket_degrades_to_coverage_only() {
    // 另一个代理把该签的都签了（本用例的现场就是“签字的人不是点菜单的人”）。
    let mut game = armed_plan_gate("gate-ticket", "r6-planner");
    game.as_actor(None);
    game.suppress_reads = true;

    let clicked = game.click("gate.approve — ");
    let lines = narrative_lines(&clicked);
    let joined = lines.join("\n");
    assert!(
        joined.contains("[READ GATE · ticket]"),
        "the ticket path must declare its degraded semantics: {lines:#?}"
    );
    assert!(
        joined.contains("signers are NOT distinguished"),
        "degradation must be stated, not implied: {lines:#?}"
    );
    assert!(!joined.contains("read-gate/"), "{lines:#?}");
    // 降级不是放水：覆盖率仍然被核（这里 1/1 已签），所以门禁真的过了。
    game.assert_kv("req.1.plan_ok", json!(true));
    game.assert_kv("req.1.gate", json!(""));
}

/// r9：侧键与值**同生共死**——单件清除与作用域销票都必须连带清签署人，
/// 否则判定会读到「值空了、签署人还在」或「值在、签署人丢了」的半状态。
#[test]
fn read_actor_side_key_dies_with_the_signature() {
    let mut game = Game::open(&project_mod_dir());
    charter_and_begin(&mut game);

    game.narrate("config.set", &text("read.acts/coordinate=9f2c1a"));
    game.assert_kv(
        "settings.read.acts/coordinate",
        json!("9f2c1a|coordinate|0"),
    );
    game.assert_kv("settings.read_actor.acts/coordinate", json!(TEST_ACTOR));

    // ① 单件清除：两个键一起变空。
    game.narrate("config.set", &text("read.acts/coordinate="));
    game.assert_kv("settings.read.acts/coordinate", json!(""));
    game.assert_kv("settings.read_actor.acts/coordinate", json!(""));

    // ② 换人重签 = 侧键 upsert（不是两个签名叠加）。
    game.as_actor(Some("other-agent"));
    game.narrate("config.set", &text("read.acts/coordinate=9f2c1a"));
    game.assert_kv("settings.read_actor.acts/coordinate", json!("other-agent"));

    // ③ 作用域边界（rev 推进 → req_bump_rev 的销票）：两个键一起清。
    game.narrate(
        "requirement.add",
        &text("side-key requirement|synthetic description|semantics-side"),
    );
    game.narrate("requirement.dispatch", "0");
    game.narrate(
        "requirement.amend",
        &text("r1||amended to bump the revision"),
    );
    game.assert_kv("settings.read.acts/coordinate", json!(""));
    game.assert_kv("settings.read_actor.acts/coordinate", json!(""));
    game.assert_kv("settings.read_index", json!(""));
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

// ───────────────────── r4/r6/r7 · layout, fragments, tool declarations ─────────────────────

/// A mod resource read straight from disk (the declaration surface is a file, not Lua state).
fn read_json(path: &Path) -> Json {
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|err| panic!("parse {}: {err}", path.display()))
}

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

/// r6 The mod architecture is **fragments + five layers**: 27 code modules all under
/// `src/{{core,domain,view,runtime,handlers}}/` (剧情双层重写 added the charter / integration /
/// release handlers), texts split into 7 fragments (core + 6 beats groups) whose beat keys
/// are pairwise disjoint (the `mod.lua` assembly contract), the playbook split into 9
/// chapter fragments (r5: head + ch1..ch5 + the four ch4- domain fragments; the former
/// seven-fragment `harness` chapter is abolished), no monolith survives, and tuning living in `data/texts/core.json`.
#[test]
fn mod_architecture_is_fragments_plus_five_layers() {
    let project = project_mod_dir();
    let pack = read_json(&project.join("pack.json"));

    // ① the code face: 28 modules, five layers, entry excluded, all on disk.
    let modules = pack["code"]["modules"]
        .as_array()
        .expect("code.modules must be an array");
    assert_eq!(
        modules.len(),
        30,
        "the 剧情双层重写 mod declares 30 modules: {modules:?}"
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
    assert_eq!(playbook, 7, "playbook = head + 6 chapter fragments (r5; ch2/ch5 removed on 2026-11-15)");
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
    // T3 aligned the beats to the v2 story: the merged fragments carry 378 keys.
    assert!(
        total_beats >= 378,
        "the merged fragments must carry the v2 beat set (378+): got {total_beats}"
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
}

/// The playbook is reachable in both directions: every assembled chapter id occurs in `guide.lua`'s
/// STAGE_GUIDE, and every mapped id resolves to a non-empty section. A chapter outside the mapping
/// is dead at phase-entry; a mapped-but-missing or mapped-but-empty chapter makes its hint narrate
/// nothing. The behavioural receipt probe below separately proves that all candidates are narrated.
#[test]
fn stage_guide_maps_existing_nonempty_sections() {
    let project = project_mod_dir();
    let guide_src =
        std::fs::read_to_string(project.join("src/view/guide.lua")).expect("read guide.lua");
    let start = guide_src
        .find("local STAGE_GUIDE = {")
        .expect("STAGE_GUIDE table must exist in guide.lua");
    let end = guide_src[start..]
        .find("\n}")
        .expect("STAGE_GUIDE table must close");
    let block = &guide_src[start..start + end];
    // odd indices of a `"` split are exactly the quoted fragments inside the table
    let mapped: std::collections::BTreeSet<String> = block
        .split('"')
        .enumerate()
        .filter(|(index, _)| index % 2 == 1)
        .map(|(_, part)| part.to_owned())
        .collect();
    assert_eq!(
        mapped.len(),
        10,
        "STAGE_GUIDE must route all 10 remaining chapters (moved to docs on 2026-11-15): {mapped:?}"
    );

    // assembled sections, same merge semantics as mod.lua (only ADJACENT same-id merges)
    let pack = read_json(&project.join("pack.json"));
    let mut sections: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut assembly_order: Vec<String> = Vec::new();
    for entry in pack["entries"].as_array().expect("pack entries") {
        let path = entry["path"].as_str().expect("entry path");
        let Some(rel) = path.strip_prefix("data/playbook/") else {
            continue;
        };
        if rel == "head.json" {
            continue;
        }
        let fragment = read_json(&project.join(path));
        for section in fragment["sections"].as_array().expect("sections") {
            let id = section["id"].as_str().expect("section id").to_owned();
            let rules = section["rules"].as_array().expect("rules").len();
            match sections.get_mut(&id) {
                Some(count) => {
                    assert_eq!(
                        assembly_order.last().map(String::as_str),
                        Some(id.as_str()),
                        "mod.lua merges only ADJACENT same-id sections; `{id}` reappears after another id"
                    );
                    *count += rules;
                }
                None => {
                    sections.insert(id.clone(), rules);
                    assembly_order.push(id);
                }
            }
        }
    }
    assert_eq!(
        sections.len(),
        10,
        "r5 assembles 10 chapters (chapters move to docs in batches, 2026-11-15): {sections:?}"
    );
    for id in &mapped {
        let rules = sections.get(id).unwrap_or_else(|| {
            panic!("STAGE_GUIDE maps `{id}` but no playbook section carries that id")
        });
        assert!(
            *rules >= 1,
            "STAGE_GUIDE maps `{id}` but the section is empty"
        );
    }
    let assembled: std::collections::BTreeSet<String> = sections.keys().cloned().collect();
    assert_eq!(
        mapped, assembled,
        "phase-entry routing and the assembled playbook must be the same chapter set"
    );
}

/// Phase-entry consumption is behavioural, not just a static mapping: `map.show` contributes one
/// global-index line for every chapter and the following render contributes one additional line for
/// each chapter mapped to the current phase. Therefore every chapter must occur once, and a current
/// phase chapter exactly twice. This catches both the old unmapped-six gap and the old first-candidate
/// shadowing without adding a payload-driven section selector back to `map.show`.
#[test]
fn playbook_index_and_phase_entry_receipts_cover_every_chapter() {
    let project = project_mod_dir();
    let pack = read_json(&project.join("pack.json"));
    let mut chapters: Vec<(String, String)> = Vec::new();
    for entry in pack["entries"].as_array().expect("pack entries") {
        let path = entry["path"].as_str().expect("entry path");
        if !path.starts_with("data/playbook/") || path.ends_with("head.json") {
            continue;
        }
        let fragment = read_json(&project.join(path));
        for section in fragment["sections"].as_array().expect("sections") {
            chapters.push((
                section["id"].as_str().expect("section id").to_owned(),
                section["title"].as_str().expect("section title").to_owned(),
            ));
        }
    }
    assert_eq!(
        chapters.len(),
        10,
        "chapter inventory changed: {chapters:?}"
    );

    let assert_receipt = |game: &mut Game, current: &[&str]| {
        let lines = game.narrate("map.show", "0");
        for (id, title) in &chapters {
            let needle = format!("field rules · {title}:");
            let expected = if current.contains(&id.as_str()) { 2 } else { 1 };
            assert_eq!(
                count_lines(&lines, &needle),
                expected,
                "chapter `{id}` must have one map-index line plus its current-phase line: {lines:#?}"
            );
        }
    };

    let mut charter = Game::open(&project);
    charter.narrate("event.start", "0");
    assert_receipt(
        &mut charter,
        &["repository", "credentials", "hard_constraints"],
    );
    charter_and_begin(&mut charter);
    assert_receipt(
        &mut charter,
        &[
            "run_machine",
            "actions",
            "drive",
            "branches",
        ],
    );

    let mut settle = drive_to_settle();
    assert_receipt(
        &mut settle,
        &[
            "run_artifacts",
            "mod_engineering",
            "host_contract",
        ],
    );
    let mut release = drive_to_release();
    assert_receipt(&mut release, &["branches"]);

    release.narrate("release.skip", &text("playbook receipt probe"));
    release.click("config — ");
    release.click("config.set — run.retire");
    release.click("back — ");
    release.click("review.note — ");
    release.assert_kv("fsm.stage", json!("retired"));
    // retired 相位已无规则章节映射（pointers 已归位 cold-start/principles.md），
    // guide_section 回退到装配后的第一章——因此当前相位行落在 chapters[0] 上。
    assert_receipt(&mut release, &[chapters[0].0.as_str()]);
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
/// the built-in subsystem tools are served by the renamed host binary **`egg`** (the two
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
        // `readOnly` is a declared fact, not a policy: the active subsystem needs a writer
        // tool (`git.worktree_claim`), and a writer may never detach (host contract:
        // `inlineWaitMs > 0` when `readOnly == false`).
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
            // identifier). A credential arrives through `$ENV{{VAR}}` host-side, never inline.
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

    // ② the built-in subsystem tools are served by `egg`; the two plain git readers stay git.
    let subsystem_tools = [
        "git.worktree_claim",
        "git.worktree_release",
        "git.lane_probe",
        "fs.artifact_probe",
        "gate.verify_suite",
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

// ───────────────────── E6 · Mod text hygiene (别名退役/子会话清理) ─────────────────────

/// Mod prose fragments must not carry retired path literals (`ygg-agent` / `pi-agent-seed`).
/// Scope = pack.json `entries` whose path sits under `data/texts/` or `data/playbook/`; a line
/// violates when it names a retired literal without the explicit `[deprecated-ref]` marker on
/// the same line. Machine-checkable half of docs/design/mod/mod-package.md (Mod text hygiene).
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

/// The copy contract is split by the existing text-consumer domains, not by arbitrary line counts.
/// Every domain table must cover exactly its JSON fragment's key set and template values; core is
/// read before §H because the 23 pointer-shell rows are a subset, not additional keys. Mutation:
/// delete a row, move it to another domain, alter a template, or hard-code a different total -> red.
#[test]
fn copy_contract_domain_fragments_cover_every_text_key() {
    let repo = repo_dir();
    let mod_dir = project_mod_dir();
    let mut total = 0usize;
    for (doc, fragment) in [
        ("flow.md", "beats-flow.json"),
        ("harness.md", "beats-harness.json"),
        ("lanes.md", "beats-lanes.json"),
        ("map-guide.md", "beats-map-guide.json"),
        ("requirements.md", "beats-requirements.json"),
        ("settlement.md", "beats-settlement.json"),
    ] {
        let text = std::fs::read_to_string(repo.join("docs/context").join("scenes").join(doc))
            .unwrap_or_else(|err| panic!("read docs/context/scenes/{doc}: {err}"));
        let documented = markdown_key_set(&text);
        let fragment_json = read_json(&mod_dir.join(format!("data/texts/{fragment}")));
        let actual: std::collections::BTreeSet<String> = fragment_json["beats"]
            .as_object()
            .expect("beats object")
            .keys()
            .cloned()
            .collect();
        assert_eq!(documented, actual, "{doc} must exactly cover {fragment}");
        let templates = markdown_template_map(&text);
        for (key, value) in fragment_json["beats"].as_object().expect("beats object") {
            let expected = value.as_str().expect("beat template string");
            assert_eq!(
                templates.get(key).map(String::as_str),
                Some(expected),
                "{doc} `{key}` must be the exact runtime template from {fragment}"
            );
        }
        total += documented.len();
    }
    let core_text = std::fs::read_to_string(repo.join("docs/context/scenes/core.md"))
        .expect("read core copy contract");
    let core_tables = core_text.split("## H.").next().expect("core tables");
    let documented = markdown_key_set(core_tables);
    let core = read_json(&mod_dir.join("data/texts/core.json"));
    let mut actual = std::collections::BTreeSet::new();
    let core_templates = markdown_template_map(core_tables);
    for namespace in ["options", "tuning", "endings"] {
        for (key, value) in core[namespace].as_object().expect("core namespace") {
            let full_key = format!("{namespace}.{key}");
            let expected = if let Some(text) = value.as_str() {
                text.to_owned()
            } else {
                value.to_string()
            };
            assert_eq!(
                core_templates.get(&full_key),
                Some(&expected),
                "core copy table `{full_key}` must match the runtime value"
            );
            actual.insert(full_key);
        }
    }
    assert_eq!(documented.len(), 36, "core options/tuning/endings = 36");
    assert_eq!(documented, actual, "core copy table must cover core.json");
    total += documented.len();
    assert_eq!(total, 454, "418 beats + 36 core keys; §H adds no keys");
    assert!(
        core_text.contains("454 个唯一键") && core_text.contains("不得再次相加"),
        "the global ledger must explain why the 23 pointer shells are not added again"
    );
}

fn markdown_key_set(text: &str) -> std::collections::BTreeSet<String> {
    text.lines()
        .filter_map(|line| line.strip_prefix("| `"))
        .filter_map(|tail| tail.split('`').next().map(str::to_owned))
        .collect()
}

fn markdown_template_map(text: &str) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    for line in text.lines().filter(|line| line.starts_with("| `")) {
        let mut cells = line.rsplitn(3, " | ");
        let _protocol = cells.next();
        let _variables = cells.next();
        let Some(head) = cells.next() else {
            panic!("malformed copy-contract row: {line}")
        };
        let rest = head.strip_prefix("| `").expect("row key prefix");
        let (key, template) = rest.split_once("` | ").expect("row key/template split");
        out.insert(
            key.to_owned(),
            template.replace("\\|", "|").replace("\\n", "\n"),
        );
    }
    out
}

/// 从一个文案值里取出所有 `docs/context/<相对路径>.md` 指针（r5）。
///
/// 旧实现只收 `[A-Za-z0-9._-]`，于是 `docs/context/acts/charter.md` 被截成 `acts`、不以
/// `.md` 结尾 → **静默跳过**（既不计数也不报违规）。r1 把权威文件切进子目录之后，整张锚点账
/// 从那天起就是假绿。现在允许 `/`，仍然要求 `.md` 结尾：目录指针 `acts/`、通配 `acts/*.md`
/// 与空串一律不算文件指针。
fn context_pointer_paths(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = value;
    while let Some(pos) = rest.find("docs/context/") {
        let tail = &rest[pos + "docs/context/".len()..];
        let file: String = tail
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
            .collect();
        rest = &tail[file.len().min(tail.len())..];
        if file.ends_with(".md") {
            out.push(file);
        }
    }
    out
}

/// 核一个键值的全部指针：累加 `count` 并返回违规清单（锚点缺失 / 非法路径段）。
///
/// `..` 与空段一律判违规——`docs/context/acts/../AGENTS.md` 这类写法能骗过存在性检查，
/// 却把读者带出 context 树。
fn context_pointer_violations(
    context_dir: &std::path::Path,
    ns: &str,
    key: &str,
    value: &str,
    count: &mut usize,
) -> Vec<String> {
    let mut violations = Vec::new();
    for file in context_pointer_paths(value) {
        *count += 1;
        if file.split('/').any(|seg| seg.is_empty() || seg == "..") {
            violations.push(format!(
                "{ns}.{key} -> docs/context/{file} (illegal path segment)"
            ));
            continue;
        }
        if !context_dir.join(&file).is_file() {
            violations.push(format!("{ns}.{key} -> docs/context/{file} (missing)"));
        }
    }
    violations
}

/// r2 (mod 职责再分工): 23 个剧情/策略键收缩为「状态行 + 指针」后，凡 beat/core 文本值携带
/// `docs/context/` 指针，其锚点文件必须真实存在于仓库 `docs/context/`——指针漂移（指向不存在的
/// 文件）会让「无 egg 读 context 自洽」的承诺失效。同时核销：六个 `<node>_common` 键与 core 的
/// scenes/prompts 必须携带指针（防收缩被意外回退）。Mutation：删掉某个锚点文件或把某指针改成
/// 不存在的路径 → 本用例转红。
///
/// r5：解析改走 `context_pointer_paths`（允许子目录）。反面证据见
/// `context_pointer_parser_accepts_subdirectories_and_flags_missing_anchors`。
#[test]
fn narrative_pointer_anchors_exist_in_docs_context() {
    let repo = repo_dir();
    let mod_dir = project_mod_dir();
    let context_dir = repo.join("docs/context");
    let mut pointer_count = 0usize;
    let mut violations: Vec<String> = Vec::new();
    let mut check = |ns: &str, key: &str, value: &str| {
        violations.extend(context_pointer_violations(
            &context_dir,
            ns,
            key,
            value,
            &mut pointer_count,
        ));
    };
    for f in [
        "beats-flow.json",
        "beats-harness.json",
        "beats-lanes.json",
        "beats-map-guide.json",
        "beats-requirements.json",
        "beats-settlement.json",
    ] {
        let fragment = read_json(&mod_dir.join(format!("data/texts/{f}")));
        if let Some(beats) = fragment["beats"].as_object() {
            for (key, value) in beats {
                if let Some(text) = value.as_str() {
                    check(f, key, text);
                }
            }
        }
    }
    let core = read_json(&mod_dir.join("data/texts/core.json"));
    for ns in ["scenes", "prompts"] {
        for (key, value) in core[ns].as_object().expect("core ns object") {
            if let Some(text) = value.as_str() {
                check(ns, key, text);
            }
        }
    }
    // 23 个收缩键必须携带指针（6 个 <node>_common + scenes 5 + prompts 5 + harness 3 + map-guide 2
    // + settlement 2 = 23；settlement_todo_segment 亦含指针）。
    let flow = read_json(&mod_dir.join("data/texts/beats-flow.json"));
    for key in [
        "prologue_common",
        "charter_common",
        "coordinate_common",
        "settle_common",
        "release_common",
        "retired_common",
    ] {
        let value = flow["beats"][key].as_str().expect("common beat string");
        assert!(
            value.contains("docs/context/"),
            "shrunk beat `{key}` must carry a docs/context pointer"
        );
    }
    // r5 指针账（子目录化 + 解析器修好后实测）：
    //   flow 6 个 <node>_common      = 7（settle_common 两处：正文 + 清理政策）
    //   harness.subagent_brief       = 2（coordinate + rituals/dispatch）
    //   map-guide 2 键               = 2
    //   settlement 4 键              = 4（r5 为 3；r6 的 maintenance_req_desc 增一处
    //                                  `docs/context/cold-start/system-boundaries.md`，因该份已从 design 归位）
    //   core.scenes 5 键             = 13（幕台词基行各 1 + [READ FIRST] 块内每条必读 1）
    //   core.prompts 5 键             = 5
    // 合计 33（r5 为 32；r6 的 settlement 增了 1 处，见上）。注意：块内的必读指针（如 `cold-start/vocabulary.md`）都是子目录形，旧解析器
    // 一并静默跳过——所以此数从旧实装口径的 25 直接跳到 32，正是漏检被补上的尺度。
    // `stage_prompt_missing` 指向 `docs/context/acts/`（目录指针，不是 .md，不计入）。
    assert_eq!(
        pointer_count, 33,
        "docs/context pointer ledger drifted: expected 33, got {pointer_count}"
    );
    assert!(
        violations.is_empty(),
        "narrative pointer anchors missing under docs/context/: {}",
        violations.join(", ")
    );
}

/// r5 反向断言（漏检防线）：旧解析器对子目录指针**静默跳过**——
/// `docs/context/acts/charter.md` 被截成 `acts`、不以 `.md` 结尾，于是 pointer_count 不增、
/// violations 不报，用例照样绿。这就是「漏检不是通过」。本用例把三件事钉死：
/// ① 子目录指针必须被识别并计数；② 指向不存在的子目录文件必须产生 violation；
/// ③ `..` 穿越必须判违规。Mutation：把 `context_pointer_paths` 的字符集退回
/// `[A-Za-z0-9._-]`（去掉 `/`）→ ①②两条断言立即转红。
#[test]
fn context_pointer_parser_accepts_subdirectories_and_flags_missing_anchors() {
    let context_dir = repo_dir().join("docs/context");

    // ① 子目录指针必须被识别为 1 条（旧实现：0 条）
    assert_eq!(
        context_pointer_paths("正文见 docs/context/acts/charter.md（立局幕）。"),
        vec!["acts/charter.md".to_string()]
    );
    // ② 两层子目录 + 紧邻中文标点 / `§` / `；` 不得吞掉路径
    assert_eq!(
        context_pointer_paths(
            "政策见 docs/context/rituals/cleanup.md §2；叙事 docs/context/story/run-story.md。"
        ),
        vec![
            "rituals/cleanup.md".to_string(),
            "story/run-story.md".to_string()
        ]
    );
    // ③ 目录指针 / 通配不算文件指针（真实写法：`docs/context/acts/` 与 `docs/context/acts/*.md`）
    assert!(context_pointer_paths("见 docs/context/acts/ 与 docs/context/acts/*.md").is_empty());

    // ④ 存在的子目录锚点：计数 1、零违规
    let mut n = 0;
    assert!(context_pointer_violations(
        &context_dir,
        "t",
        "ok",
        "docs/context/acts/charter.md",
        &mut n
    )
    .is_empty());
    assert_eq!(n, 1, "子目录指针必须计入 pointer_count");

    // ⑤ 不存在的子目录锚点：必须计数 1 且产生 1 条 violation（旧实现：0 条 = 漏检）
    let mut n = 0;
    let violations = context_pointer_violations(
        &context_dir,
        "t",
        "missing",
        "docs/context/acts/definitely-no-such-doc.md",
        &mut n,
    );
    assert_eq!(
        (n, violations.len()),
        (1, 1),
        "指向不存在子目录文件的指针必须转红：{violations:?}"
    );

    // ⑥ `..` 穿越必须判违规
    let mut n = 0;
    let violations = context_pointer_violations(
        &context_dir,
        "t",
        "escape",
        "docs/context/acts/../README.md",
        &mut n,
    );
    assert_eq!(violations.len(), 1, "`..` 穿越必须违规：{violations:?}");
}

// ──────────────── r6: docs/design 契约树的指针 / 索引机检 ────────────────

/// 通用「树内指针」解析器：在 `prefix`（形如 `docs/design/`）之后取字符集内的最长串。
///
/// 字符集 `[A-Za-z0-9._/-]`：遇空白、`§`、`；`、中文标点即止，
/// 所以占位 `docs/design/<module>/<topic>.md`、通配 `docs/design/*.md`、目录指针
/// `docs/design/kernel/` 都不会被当成文件指针。
///
/// r6 比 r5 的实现多做一步：把尾部的 `.` 与 `/` 剥掉再判 `.md`——
/// `…/harness-tools.md.` 这类「句末点多写一个」的写法在只认 `.md` 结尾的解析器里会被整条吞掉（漏检不是通过）。
fn pointer_paths(value: &str, prefix: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = value;
    while let Some(pos) = rest.find(prefix) {
        let tail = &rest[pos + prefix.len()..];
        let run: String = tail
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
            .collect();
        rest = &tail[run.len().min(tail.len())..];
        let file = run.trim_end_matches(['.', '/']);
        if file.ends_with(".md") {
            out.push(file.to_owned());
        }
    }
    out
}

/// 指向 `docs/design/` 的文件指针（**允许多级子目录**）。
fn design_pointer_paths(value: &str) -> Vec<String> {
    pointer_paths(value, "docs/design/")
}

/// 核一个键值的全部 design 指针：累加 `count` 并返回违规清单（锚点缺失 / 非法路径段）。
///
/// `..` 与空段一律判违规——`docs/design/kernel/../../AGENTS.md` 能骗过存在性检查，
/// 却把读者带出契约树。
fn design_pointer_violations(
    design_dir: &std::path::Path,
    ns: &str,
    key: &str,
    value: &str,
    count: &mut usize,
) -> Vec<String> {
    let mut violations = Vec::new();
    for file in design_pointer_paths(value) {
        *count += 1;
        if file.split('/').any(|seg| seg.is_empty() || seg == "..") {
            violations.push(format!(
                "{ns}.{key} -> docs/design/{file} (illegal path segment)"
            ));
            continue;
        }
        if !design_dir.join(&file).is_file() {
            violations.push(format!("{ns}.{key} -> docs/design/{file} (missing)"));
        }
    }
    violations
}

/// 递归收集目录下按后缀过滤的文件（不引 `walkdir`：`ygg-sandbox` 的 dev-dependencies 只有 `ygg-store`）。
fn collect_files_with(dir: &std::path::Path, suffix: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|e| e.path())
        .collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_files_with(&path, suffix, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some(suffix) {
            out.push(path);
        }
    }
}

/// 词法归一化：消 `./` 与 `..` 段（不碰磁盘）。
fn normalize_repo_path(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out.join("/")
}

/// r6：设计契约树的**指针可达性**。
///
/// `docs/design/` 收缩为 `kernel/ host/ mod/` 三模块后，playbook 的 `basis`/`source`、`texts` 的
/// 文案键、`src/**` 的注释里那些 `docs/design/<module>/<topic>.md` 指针必须逐个命中真实文件——
/// 它们会原样出现在回执与章摘要里，断了就是把读者送进空目录。
///
/// 同时钉住**逐来源硬账**：33 处（ch1 4 + ch4-mod-host 8 + ch4-protocol 3 + ch4-run-machine 7
/// + ch5 6 + beats-settlement 3 + src/runtime/tools.lua 2）。删掉一处指针 → ledger 少 1 → 转红。
///
/// Mutation：把任一指针改成 `docs/design/kernel/nope.md`，或用例改用不存在的子目录 → 本用例转红。
#[test]
fn design_pointer_anchors_exist_under_docs_design() {
    let repo = repo_dir();
    let mod_dir = project_mod_dir();
    let design_dir = repo.join("docs/design");

    // (来源文件, 键, 值) —— playbook / texts / core / src 四处，缺一处就是漏检
    let mut items: Vec<(String, String, String)> = Vec::new();
    let mut playbook: Vec<PathBuf> = Vec::new();
    collect_files_with(&mod_dir.join("data/playbook"), "json", &mut playbook);
    for path in &playbook {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("playbook file name")
            .to_owned();
        let fragment = read_json(path);
        let Some(sections) = fragment["sections"].as_array() else {
            continue; // `head.json` 是章首投影，没有 sections
        };
        for section in sections {
            let id = section["id"].as_str().unwrap_or("?").to_owned();
            for key in ["source", "summary"] {
                if let Some(text) = section[key].as_str() {
                    items.push((name.clone(), format!("{id}.{key}"), text.to_owned()));
                }
            }
            for rule in section["rules"].as_array().expect("rules") {
                let rid = rule["id"].as_str().unwrap_or("?").to_owned();
                for key in ["text", "basis"] {
                    if let Some(text) = rule[key].as_str() {
                        items.push((name.clone(), format!("{rid}.{key}"), text.to_owned()));
                    }
                }
            }
        }
    }
    let mut texts: Vec<PathBuf> = Vec::new();
    collect_files_with(&mod_dir.join("data/texts"), "json", &mut texts);
    for path in &texts {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("texts file name")
            .to_owned();
        let fragment = read_json(path);
        if let Some(beats) = fragment["beats"].as_object() {
            for (key, value) in beats {
                if let Some(text) = value.as_str() {
                    items.push((name.clone(), key.clone(), text.to_owned()));
                }
            }
        }
        for ns in ["scenes", "prompts"] {
            if let Some(map) = fragment[ns].as_object() {
                for (key, value) in map {
                    if let Some(text) = value.as_str() {
                        items.push((name.clone(), format!("{ns}.{key}"), text.to_owned()));
                    }
                }
            }
        }
    }
    let mut lua: Vec<PathBuf> = Vec::new();
    collect_files_with(&mod_dir.join("src"), "lua", &mut lua);
    for path in &lua {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("lua file name")
            .to_owned();
        let text = std::fs::read_to_string(path).expect("read lua source");
        items.push((name, "source".to_owned(), text));
    }

    let mut ledger: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut violations: Vec<String> = Vec::new();
    let mut total = 0usize;
    for (source, key, value) in &items {
        let mut n = 0usize;
        violations.extend(design_pointer_violations(
            &design_dir,
            source,
            key,
            value,
            &mut n,
        ));
        if n > 0 {
            total += n;
            *ledger.entry(source.clone()).or_insert(0) += n;
        }
    }

    let expected: std::collections::BTreeMap<String, usize> = [
        ("ch1-foundation.json", 4usize),
        ("ch4-mod-host.json", 8),
        ("ch4-protocol.json", 3),
        ("ch4-run-machine.json", 7),
        ("beats-settlement.json", 3),
        ("tools.lua", 2),
    ]
    .into_iter()
    .map(|(name, count)| (name.to_owned(), count))
    .collect();

    assert_eq!(
        ledger, expected,
        "docs/design 指针账漂移（逐来源）：{ledger:?}"
    );
    assert_eq!(total, 27, "docs/design 指针总数漂移：期望 27，实得 {total}");
    assert!(
        violations.is_empty(),
        "docs/design 指针锚点不存在：{}",
        violations.join(", ")
    );
}

/// r6 反向断言（漏检防线）：验证性、子目录、`..` 三件事一起钉死。
///
/// ① 子目录指针必须识别并计数（旧式只认 `[A-Za-z0-9._-]` 的实现给 0 条）；
/// ② 句末点多写一个（`…/harness-tools.md.`）也必须识别（r5 的实现会整条吞掉）；
/// ③ 目录指针 / 通配 / 占位不算文件指针；
/// ④ 存在的子目录锚点：计数 1、零违规；
/// ⑤ 不存在的子目录锚点：计数 1、一条违规（**漏检不是通过**）；
/// ⑥ `..` 穿越必须判违规。
///
/// Mutation：把 `design_pointer_paths` 的字符集退回 `[A-Za-z0-9._-]`（去掉 `/`）→ ①⑤转红；
/// 去掉 `trim_end_matches` → ②转红；去掉 `..` 判定 → ⑥转红。
#[test]
fn design_pointer_parser_accepts_subdirectories_and_flags_missing_anchors() {
    let design_dir = repo_dir().join("docs/design");

    // ① 一层子目录
    assert_eq!(
        design_pointer_paths("见 docs/design/kernel/state-machine.md（状态机）。"),
        vec!["kernel/state-machine.md".to_string()]
    );
    // ② 句末点多写一个也不吞；紧邻中文标点 / `；` 不吞路径
    assert_eq!(
        design_pointer_paths("改 harness-tools 前读 docs/design/host/harness-tools.md. 再看 docs/design/mod/tool-bridge.md；"),
        vec![
            "host/harness-tools.md".to_string(),
            "mod/tool-bridge.md".to_string()
        ]
    );
    // ③ 目录指针 / 通配 / 占位都不算文件指针
    assert!(design_pointer_paths(
        "目录 docs/design/kernel/ 与通配 docs/design/*.md 与占位 docs/design/<module>/<topic>.md"
    )
    .is_empty());

    // ④ 存在的子目录锚点：计数 1、零违规
    let mut n = 0;
    assert!(design_pointer_violations(
        &design_dir,
        "t",
        "ok",
        "docs/design/kernel/state-machine.md",
        &mut n
    )
    .is_empty());
    assert_eq!(n, 1, "子目录指针必须计入 pointer_count");

    // ⑤ 不存在的子目录锚点：必须计数 1 且产生 1 条违规
    let mut n = 0;
    let violations = design_pointer_violations(
        &design_dir,
        "t",
        "missing",
        "docs/design/kernel/definitely-no-such-doc.md",
        &mut n,
    );
    assert_eq!(
        (n, violations.len()),
        (1, 1),
        "指向不存在子目录文件的指针必须转红：{violations:?}"
    );

    // ⑥ `..` 穿越必须违规
    let mut n = 0;
    let violations = design_pointer_violations(
        &design_dir,
        "t",
        "escape",
        "docs/design/kernel/../README.md",
        &mut n,
    );
    assert_eq!(violations.len(), 1, "`..` 穿越必须违规：{violations:?}");
}

/// r6：**索引集合相等** + `id` 与路径的双射 + 结构性禁止留壳。
///
/// ① `docs/design/README.md` 里指向本树的链接集合 **双向等于** 磁盘上的实文件集合
///    （新增文件忘进索引 → 红；删文件索引还挂着 → 红）；
/// ② 每份文件头的 `id` 与其相对路径（去 `.md`）逐字相等；
/// ③ `docs/design/*.md` 只允许 `README.md`——出现任何平铺文件或软链即违规
///    （这是「不留兼容层」的结构性表达：壳没有藏身处）；
/// ④ 一级子目录恰好 `{kernel, host, mod}`，各自非空——出现第四目录（如残余桶 `narrative/`）即违规。
///
/// Mutation：往 `docs/design/` 放一个平铺文件或软链、或把某份文件搬进新目录 → 本用例转红。
#[test]
fn design_tree_index_is_complete_and_ids_match_paths() {
    let repo = repo_dir();
    let design_dir = repo.join("docs/design");

    // ④ 一级子目录恰好三模块
    let mut modules: Vec<String> = Vec::new();
    let mut flat: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&design_dir).expect("read docs/design") {
        let path = entry.expect("dir entry").path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("name")
            .to_owned();
        if path.is_dir() {
            modules.push(name);
        } else {
            flat.push(name);
        }
    }
    modules.sort();
    flat.sort();
    assert_eq!(
        modules,
        vec!["host".to_string(), "kernel".to_string(), "mod".to_string()],
        "docs/design 一级子目录必须是 kernel/host/mod 三模块（不得出现第四目录/残余桶）"
    );
    assert_eq!(
        flat,
        vec!["README.md".to_string()],
        "docs/design/*.md 只允许树入口 README.md（平铺文件或软链 = 留了兼容壳）"
    );

    // ① 实文件集合（相对 docs/design）
    let mut files: Vec<PathBuf> = Vec::new();
    collect_files_with(&design_dir, "md", &mut files);
    let mut actual: Vec<String> = files
        .iter()
        .filter_map(|path| {
            path.strip_prefix(&design_dir)
                .ok()
                .map(|rel| rel.to_string_lossy().replace('\\', "/"))
        })
        .filter(|rel| rel != "README.md")
        .collect();
    actual.sort();
    assert_eq!(actual.len(), 15, "契约面应为 15 份文件：{actual:?}");

    // ① 索引里的链接集合
    let readme = std::fs::read_to_string(design_dir.join("README.md")).expect("read design README");
    let mut indexed: Vec<String> = Vec::new();
    for chunk in readme.split("](").skip(1) {
        let target = chunk
            .split(')')
            .next()
            .unwrap_or("")
            .split('#')
            .next()
            .unwrap_or("");
        if target.is_empty() || target.starts_with("http") || target.starts_with("mailto:") {
            continue;
        }
        if !target.ends_with(".md") {
            continue;
        }
        let resolved = normalize_repo_path(&format!("docs/design/{target}"));
        if let Some(rel) = resolved.strip_prefix("docs/design/") {
            if rel != "README.md" {
                indexed.push(rel.to_owned());
            }
        }
    }
    indexed.sort();
    indexed.dedup();
    assert_eq!(
        indexed, actual,
        "docs/design/README.md 的索引与实文件集合必须双向相等"
    );

    // ② id 与路径双射
    for rel in &actual {
        let path = design_dir.join(rel);
        let text = std::fs::read_to_string(&path).expect("read design file");
        let id_line = text
            .lines()
            .find(|line| line.starts_with("> **id** "))
            .unwrap_or_else(|| panic!("{} 缺 `> **id**` 固定头", rel));
        let declared = id_line
            .trim_start_matches("> **id** ")
            .split_whitespace()
            .next()
            .expect("id token");
        let expected_id = rel.trim_end_matches(".md");
        assert_eq!(declared, expected_id, "{rel} 的 id 与路径不一致");
    }
}

/// r6：`AGENTS.md §3` 的每条 design 路由都能解析到真实文件，且路由行不许写裸文件名。
///
/// 13 条 design 指针（Entry 1 + Mod/host/tool 3 + serve/lanes/CLI/identity 4 + Kernel 4 +
/// Runtime copy 1）。此外 §3 的 `Repository topology` 行必须把 `system-boundaries` 指向
/// **`docs/context/cold-start/system-boundaries.md`**（r6 归位后它不再是 design 文件），
/// 且该指针也在本用例的机检范围内。
///
/// Mutation：把某条路由改回旧平铺路径、或写成裸文件名 `mod-package.md`、或让 topology 行
/// 仍指 `docs/design/system-boundaries.md` → 本用例转红。
#[test]
fn agents_design_routes_resolve() {
    let repo = repo_dir();
    let design_dir = repo.join("docs/design");
    let agents = std::fs::read_to_string(repo.join("AGENTS.md")).expect("read AGENTS.md");

    let start = agents
        .find("## 3. Prompt-surface index")
        .expect("AGENTS.md 必须有 `## 3. Prompt-surface index` 段");
    let end = agents[start + 5..]
        .find("\n## ")
        .map(|offset| start + 5 + offset)
        .unwrap_or(agents.len());
    let section = &agents[start..end];
    // 只看表格行：§3 的散文（退役说明）里也会提到 design 路径，但它不是路由。
    let table: String = section
        .lines()
        .filter(|line| line.trim_start().starts_with('|'))
        .collect::<Vec<_>>()
        .join("\n");

    let routes = design_pointer_paths(&table);
    assert_eq!(
        routes.len(),
        13,
        "AGENTS §3 的 design 路由应为 13 条：{routes:?}"
    );
    for route in &routes {
        let path = design_dir.join(route);
        assert!(
            path.is_file(),
            "AGENTS §3 路由解析不到文件：docs/design/{route}"
        );
        let module = route.split('/').next().unwrap_or("");
        assert!(
            route == "README.md" || matches!(module, "kernel" | "host" | "mod"),
            "AGENTS §3 路由必须落在三模块内（或指向树入口 README.md）：docs/design/{route}"
        );
    }

    // 裸文件名省略形（`mod-package.md`）不可机械解析，禁；
    // 只禁「指向 design 的裸名」——context 树自身的裸名（`topology.md`）不属本约束。
    let topics: Vec<String> = ["kernel", "host", "mod"]
        .iter()
        .filter_map(|module| std::fs::read_dir(design_dir.join(module)).ok())
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().to_str().map(|n| n.to_owned()))
        .collect();
    for (index, token) in table.split('`').enumerate() {
        if index % 2 == 1
            && token.ends_with(".md")
            && !token.contains('/')
            && topics.iter().any(|topic| topic == token)
        {
            panic!(
                "AGENTS §3 路由禁写裸文件名：`{token}`（写全路径 docs/design/<module>/{token}）"
            );
        }
    }

    // Repository topology 行：system-boundaries 必须落在 context/cold-start
    let topology = table
        .lines()
        .find(|line| line.contains("Repository topology"))
        .expect("AGENTS §3 必须有 Repository topology 行");
    assert!(
        topology.contains("docs/context/cold-start/system-boundaries.md"),
        "Repository topology 行必须指向 docs/context/cold-start/system-boundaries.md：{topology}"
    );
    assert!(
        !topology.contains("docs/design/system-boundaries.md"),
        "system-boundaries 已归位 context，路由不得再指 design：{topology}"
    );
    assert!(
        repo.join("docs/context/cold-start/system-boundaries.md")
            .is_file(),
        "Repository topology 路由指向的文件必须真实存在"
    );
}

/// r6：`scenes.json` 的 `reference` 段——**登记但不拦门禁**，且默认值被负向断言钉死。
///
/// ① 三个 id 都能解析到 `docs/context/<id>.md`，且文件头 `id` 与表中值逐字相等；
/// ② 负向：它们**不在** `scene` / `reqStage` 里（不被升格成必读）；
/// ③ 负向：`data/config.json` 的 `readPlan` 里**不含**这三个 id（不进 `[READ FIRST]`、不出签名义务）。
///
/// Mutation：把某份归位文件挪进 `scene`/`reqStage`（升格成必读）→ ②③转红。
#[test]
fn context_scene_reference_ids_resolve() {
    let repo = repo_dir();
    let mod_dir = project_mod_dir();
    let context_dir = repo.join("docs/context");
    let scenes = read_json(&context_dir.join("scenes.json"));

    let mut reference: Vec<String> = Vec::new();
    let reference_obj = scenes["reference"].as_object().expect("scenes.reference");
    for (key, value) in reference_obj {
        if key == "note" {
            continue;
        }
        for id in value.as_array().expect("reference group array") {
            reference.push(id.as_str().expect("reference id string").to_owned());
        }
    }
    assert_eq!(
        reference.len(),
        4,
        "reference 段应登记 4 个（含不进 readPlan 的规范文档）：{reference:?}"
    );

    let mut scene_ids: Vec<String> = Vec::new();
    for group in scenes["scene"].as_object().expect("scenes.scene").values() {
        for id in group.as_array().expect("scene group array") {
            scene_ids.push(id.as_str().expect("scene id").to_owned());
        }
    }
    let mut stage_ids: Vec<String> = Vec::new();
    for value in scenes["reqStage"]
        .as_object()
        .expect("scenes.reqStage")
        .values()
    {
        stage_ids.push(value.as_str().expect("reqStage id").to_owned());
    }
    let read_plan = read_json(&mod_dir.join("data/config.json"))["readPlan"].to_string();

    for id in &reference {
        let path = context_dir.join(format!("{id}.md"));
        assert!(
            path.is_file(),
            "reference id 解析不到文件：docs/context/{id}.md"
        );
        let text = std::fs::read_to_string(&path).expect("read reference doc");
        let id_line = text
            .lines()
            .find(|line| line.starts_with("> **id** "))
            .unwrap_or_else(|| panic!("docs/context/{id}.md 缺 `> **id**` 固定头"));
        let declared = id_line
            .trim_start_matches("> **id** ")
            .split_whitespace()
            .next()
            .expect("id token");
        assert_eq!(
            declared,
            id.as_str(),
            "docs/context/{id}.md 的 id 与 scenes 表不一致"
        );

        assert!(
            !scene_ids.iter().any(|scene| scene == id),
            "{id} 是理解性文档，不得进 scenes.scene（那会进 [READ FIRST] 并产生签名义务）"
        );
        assert!(
            !stage_ids.iter().any(|stage| stage == id),
            "{id} 是理解性文档，不得进 scenes.reqStage"
        );
        assert!(
            !read_plan.contains(id.as_str()),
            "{id} 不得出现在 data/config.json 的 readPlan 里（默认不拦门禁）"
        );
    }
}

/// r6：playbook / texts / core / `src/**` 里的 `docs/context/` 指针必须逐个可达。
///
/// 这是 r5 盲区的补丁：r5 的锚点用例（`narrative_pointer_anchors_exist_in_docs_context`）
/// 只扫 `data/texts/**` 与 `core.scenes/prompts`，**从不打开 `data/playbook/**` 与 `src/**`**，
/// 于是 11 处 playbook 指针 + 1 处 `src/runtime/tools.lua` 注释在 r5 之后仍指向已删除的文件
/// （**死链全绿**）。本用例把这两个入口纳入，并钉住 48 处逐来源硬账。
///
/// Mutation：把任一处改回一个已解散的台账路径 → 本用例转红。
#[test]
fn playbook_and_src_context_pointers_resolve() {
    let repo = repo_dir();
    let mod_dir = project_mod_dir();
    let context_dir = repo.join("docs/context");

    let mut items: Vec<(String, String, String)> = Vec::new();
    let mut playbook: Vec<PathBuf> = Vec::new();
    collect_files_with(&mod_dir.join("data/playbook"), "json", &mut playbook);
    for path in &playbook {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("playbook file name")
            .to_owned();
        let fragment = read_json(path);
        let Some(sections) = fragment["sections"].as_array() else {
            continue; // `head.json` 是章首投影，没有 sections
        };
        for section in sections {
            let id = section["id"].as_str().unwrap_or("?").to_owned();
            for key in ["source", "summary"] {
                if let Some(text) = section[key].as_str() {
                    items.push((name.clone(), format!("{id}.{key}"), text.to_owned()));
                }
            }
            for rule in section["rules"].as_array().expect("rules") {
                let rid = rule["id"].as_str().unwrap_or("?").to_owned();
                for key in ["text", "basis"] {
                    if let Some(text) = rule[key].as_str() {
                        items.push((name.clone(), format!("{rid}.{key}"), text.to_owned()));
                    }
                }
            }
        }
    }
    let mut texts: Vec<PathBuf> = Vec::new();
    collect_files_with(&mod_dir.join("data/texts"), "json", &mut texts);
    for path in &texts {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("texts file name")
            .to_owned();
        let fragment = read_json(path);
        if let Some(beats) = fragment["beats"].as_object() {
            for (key, value) in beats {
                if let Some(text) = value.as_str() {
                    items.push((name.clone(), key.clone(), text.to_owned()));
                }
            }
        }
        for ns in ["scenes", "prompts"] {
            if let Some(map) = fragment[ns].as_object() {
                for (key, value) in map {
                    if let Some(text) = value.as_str() {
                        items.push((name.clone(), format!("{ns}.{key}"), text.to_owned()));
                    }
                }
            }
        }
    }
    let mut lua: Vec<PathBuf> = Vec::new();
    collect_files_with(&mod_dir.join("src"), "lua", &mut lua);
    for path in &lua {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("lua file name")
            .to_owned();
        let text = std::fs::read_to_string(path).expect("read lua source");
        items.push((name, "source".to_owned(), text));
    }

    let mut ledger: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut violations: Vec<String> = Vec::new();
    let mut total = 0usize;
    for (source, key, value) in &items {
        for file in pointer_paths(value, "docs/context/") {
            total += 1;
            *ledger.entry(source.clone()).or_insert(0) += 1;
            if file.split('/').any(|seg| seg.is_empty() || seg == "..") {
                violations.push(format!("{source}.{key} -> docs/context/{file} (illegal)"));
            } else if !context_dir.join(&file).is_file() {
                violations.push(format!("{source}.{key} -> docs/context/{file} (missing)"));
            }
        }
    }

    let expected: std::collections::BTreeMap<String, usize> = [
        ("beats-flow.json", 7usize),
        ("beats-harness.json", 2),
        ("beats-map-guide.json", 2),
        ("beats-settlement.json", 4),
        ("ch1-foundation.json", 6),
        ("ch4-mod-host.json", 0),
        ("ch4-protocol.json", 2),
        ("ch4-run-machine.json", 5),
        ("core.json", 18),
        ("tools.lua", 1),
    ]
    .into_iter()
    .filter(|(_, count)| *count > 0)
    .map(|(name, count)| (name.to_owned(), count))
    .collect();

    assert_eq!(
        ledger, expected,
        "docs/context 指针账漂移（playbook + texts + src）：{ledger:?}"
    );
    assert_eq!(
        total, 47,
        "docs/context 指针总数漂移：期望 47，实得 {total}"
    );
    assert!(
        violations.is_empty(),
        "docs/context 指针锚点不存在：{}",
        violations.join(", ")
    );
}

/// r5 (子会话清理): the cleanup prompt beat must stay harness-agnostic — subagent session cleanup
/// is delegated to the main session's own environment, so `data/texts/beats-settlement.json`
/// must not carry any harness-implementation literal (`pi-subagents`, `.pi/agent`, `.claude`,
/// `.codex`, `archive_sessions`). Mutation: inject a forbidden literal line into the
/// fragment → this test goes red.
#[test]
fn beats_cleanup_prompt_carries_no_harness_literals() {
    let mod_dir = project_mod_dir();
    let fragment = mod_dir.join("data/texts/beats-settlement.json");
    let text = std::fs::read_to_string(&fragment)
        .unwrap_or_else(|err| panic!("read {}: {err}", fragment.display()));
    let forbidden = [
        "pi-subagents",
        ".pi/agent",
        ".claude",
        ".codex",
        "archive_sessions",
    ];
    let mut violations: Vec<String> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        for literal in forbidden {
            if line.contains(literal) {
                violations.push(format!("{}:{} ({literal})", fragment.display(), index + 1));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "harness-implementation literals in the cleanup prompt fragment (keep it \
         harness-agnostic): {}",
        violations.join(", ")
    );
}

// ───────────────────── tool bridge (T1–T4) ─────────────────────

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
    let mut game = drive_to_settle();
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
    let mut game = drive_to_settle();
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

// ───────────────────── A · active subsystem ─────────────────────

/// One full tool round trip: register the request, then settle it. Machine actions carry no
/// menu ticket, and the call id is explicit so every assertion can name it.
fn tool_round(game: &mut Game, call: &str, result: &str) -> Vec<String> {
    game.narrate("tool.call", &text(call));
    narrative_lines(&game.reply("tool.result", &text(result)))
}

/// A1 `git.worktree_claim` folds into the **same** lane ledger as `lane.claim` (same function,
/// same keys, `since` tick included), and `git.worktree_release` frees it by lane number.
/// The claim input branch must carry the forced naming shape — an illegal name is refused
/// before any ledger write, through the narration channel (ledger and reject ledger untouched).
#[test]
fn worktree_claim_binds_the_lane_ledger_and_release_frees_it() {
    let mut game = Game::open(&project_mod_dir());
    charter_and_begin(&mut game);
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

    // An illegal branch name in the **input** is refused through the narration channel: the
    // ledger and the reject counter stay untouched by design.
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
#[test]
fn lane_probe_records_truth_and_the_radar_blocks_overlapping_lanes() {
    let mut game = Game::open(&project_mod_dir());
    charter_and_begin(&mut game);
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

/// r1 (泳道分池) lane pools: `lane.pool "label|path"` registers named pools and the same lane
/// number k hosts **different requirements in different pools** (the multi-repo case that a
/// flat `lane.<k>` ledger could never express). Registration is an idempotent upsert with a
/// shape gate (label regex + reserved words; path without `|` `;` newline or edge spaces).
#[test]
fn lane_pool_registry_and_multi_repo_parallel_claims() {
    let mut game = Game::open(&project_mod_dir());
    charter_and_begin(&mut game);
    game.narrate("requirement.add", &text("alpha side|description|subsys-a"));
    game.narrate("requirement.add", &text("beta side|description|subsys-b"));

    // ① registration is an idempotent upsert: re-registering `alpha` updates in place.
    let first = game.narrate("lane.pool", &text("alpha|/tmp/checkout-alpha"));
    assert_eq!(count_lines(&first, "[LANE POOL]"), 1, "{first:#?}");
    game.narrate("lane.pool", &text("beta|/tmp/checkout-beta"));
    game.narrate("lane.pool", &text("alpha|/tmp/checkout-alpha"));
    game.assert_kv("lane.pools", json!("alpha,beta"));
    game.assert_kv("lane.pool.alpha.path", json!("/tmp/checkout-alpha"));

    // ② THE multi-repo case: the same k=1 in two pools holds two requirements at once.
    let a = game.narrate(
        "lane.claim",
        &text("1|r1|feature/aa11-alpha|abc1234|1|alpha"),
    );
    assert_eq!(count_lines(&a, "[LANE CLAIMED]"), 1, "{a:#?}");
    assert!(
        a.iter().any(|line| line.contains("lane-alpha-1")),
        "a named-pool claim is labelled lane-<pool>-<k>: {a:#?}"
    );
    let b = game.narrate("lane.claim", &text("1|r2|feature/aa11-beta|abc1234|1|beta"));
    assert_eq!(count_lines(&b, "[LANE CLAIMED]"), 1, "{b:#?}");
    game.assert_kv("lane.alpha.1.req", json!("r1"));
    game.assert_kv("lane.beta.1.req", json!("r2"));
    game.assert_kv("lane.1.req", Json::Null); // the default pool's k=1 stays untouched
    game.assert_kv("req.1.lane", json!(1));
    game.assert_kv("req.1.lane_pool", json!("alpha"));
    game.assert_kv("req.2.lane_pool", json!("beta"));

    // ③ registration shape gate: bad label / reserved word / dirty path segments are refused
    // (payload-face rejections move the reject ledger) and register nothing.
    let rejects_before = game.kv("reject.count");
    for payload in [
        "Alpha|/tmp/x",   // label must be lowercase
        "seq|/tmp/x",     // reserved word
        "pools|/tmp/x",   // reserved word
        "alpha| /tmp/x ", // leading/trailing spaces in path
        "alpha|/tmp/a;b", // `;` would corrupt the args wire
        "alpha|",         // empty path
    ] {
        let lines = game.narrate("lane.pool", &text(payload));
        assert_refused(&lines, "lane.pool");
    }
    assert_ne!(
        game.kv("reject.count"),
        rejects_before,
        "the registration refusals must move the reject ledger"
    );
    game.assert_kv("lane.pools", json!("alpha,beta"));

    // ④ a claim naming an unregistered pool is refused, ledger untouched.
    let rejects_before = game.kv("reject.count");
    let unknown = game.narrate(
        "lane.claim",
        &text("2|r1|feature/aa11-gamma|abc1234|1|gamma"),
    );
    assert_refused(&unknown, "lane.claim");
    assert_ne!(game.kv("reject.count"), rejects_before);
    game.assert_kv("lane.gamma.2.req", Json::Null);
}

/// r1 (泳道分池) parallelism is counted **per pool** (k ∈ 1..job_capacity inside one pool),
/// occupancy and the one-lane-per-requirement bound are (pool, k)-scoped, and a different
/// pool's same k never counts as occupied.
#[test]
fn same_pool_parallelism_and_occupancy_scoped() {
    let mut game = Game::open(&project_mod_dir());
    charter_and_begin(&mut game);
    game.narrate("lane.pool", &text("alpha|/tmp/checkout-alpha"));
    game.narrate("requirement.add", &text("first|description|subsys-a"));
    game.narrate("requirement.add", &text("second|description|subsys-b"));
    game.narrate("requirement.add", &text("third|description|subsys-c"));

    // ① two parallel lanes inside one named pool.
    game.narrate("lane.claim", &text("1|r1|feature/aa11-one|abc1234|1|alpha"));
    game.narrate("lane.claim", &text("2|r2|feature/aa11-two|abc1234|1|alpha"));
    game.assert_kv("lane.alpha.1.req", json!("r1"));
    game.assert_kv("lane.alpha.2.req", json!("r2"));

    // ② k beyond the per-pool capacity is refused (job_capacity = 4).
    let over = game.narrate(
        "lane.claim",
        &text("5|r3|feature/aa11-five|abc1234|1|alpha"),
    );
    assert_refused(&over, "lane.claim");

    // ③ occupancy is (pool, k)-scoped: r3 cannot take alpha-1 (held by r1), and r1 cannot
    // hold a second lane in the same pool — but r3 CAN take default-1 while alpha-1 is held.
    let occupied = game.narrate(
        "lane.claim",
        &text("1|r3|feature/aa11-three|abc1234|1|alpha"),
    );
    assert_refused(&occupied, "lane.claim");
    let bound = game.narrate(
        "lane.claim",
        &text("2|r1|feature/aa11-one-b|abc1234|1|alpha"),
    );
    assert_refused(&bound, "lane.claim");
    game.assert_kv("lane.alpha.1.req", json!("r1"));
    game.assert_kv("lane.alpha.2.req", json!("r2"));
    let cross = game.narrate("lane.claim", &text("1|r3|feature/aa11-def|abc1234|1"));
    assert_eq!(count_lines(&cross, "[LANE CLAIMED]"), 1, "{cross:#?}");
    game.assert_kv("lane.1.req", json!("r3"));
    game.assert_kv("req.3.lane_pool", json!(""));

    // ④ the pool-scoped lane labels flow through the panel surface (`lane_text` rides the
    // requirement.list head): the default pool keeps the byte-stable `lane-<k>` labels and
    // the named pool adds `lane-<pool>-<k>` rows.
    let list = game.narrate("requirement.list", "0");
    let joined = list.iter().cloned().collect::<Vec<_>>().join("\n");
    assert!(
        joined.contains("lane-1 `r3`(open)"),
        "the default pool keeps the byte-stable label: {joined}"
    );
    assert!(
        joined.contains("lane-alpha-1 `r1`(open)") && joined.contains("lane-alpha-2 `r2`(open)"),
        "named-pool rows use the lane-<pool>-<k> label: {joined}"
    );
}

/// r1 (泳道分池) release is (pool, k)-scoped: releasing one pool's lane never touches another
/// pool's same-k lane, the requirement-side binding (lane + lane_pool) clears together, and
/// the tool-face pooled release resolves the pool from the **request** repo argument.
#[test]
fn release_paths_are_pool_scoped() {
    let mut game = Game::open(&project_mod_dir());
    charter_and_begin(&mut game);
    game.narrate("lane.pool", &text("alpha|/tmp/checkout-alpha"));
    game.narrate("lane.pool", &text("beta|/tmp/checkout-beta"));
    game.narrate("requirement.add", &text("first|description|subsys-a"));
    game.narrate("requirement.add", &text("second|description|subsys-b"));
    game.narrate("requirement.add", &text("third|description|subsys-c"));
    game.narrate("lane.claim", &text("1|r1|feature/aa11-one|abc1234|1|alpha"));
    game.narrate("lane.claim", &text("1|r2|feature/aa11-two|abc1234|1|beta"));
    game.narrate("lane.claim", &text("1|r3|feature/aa11-three|abc1234|1"));

    // ① releasing beta-1 leaves alpha-1 and default-1 (same k!) untouched.
    let rel = game.narrate("lane.release", &text("1|beta"));
    assert_eq!(count_lines(&rel, "[LANE RELEASED]"), 1, "{rel:#?}");
    assert!(
        rel.iter().any(|line| line.contains("lane-beta-1")),
        "the release names the pool-scoped label: {rel:#?}"
    );
    game.assert_kv("lane.beta.1.req", json!(""));
    game.assert_kv("lane.alpha.1.req", json!("r1"));
    game.assert_kv("lane.1.req", json!("r3"));
    game.assert_kv("req.2.lane", json!(0));
    game.assert_kv("req.2.lane_pool", json!(""));

    // ② re-releasing the now-empty pool lane is idle, narrated, and moves nothing.
    let idle = game.narrate("lane.release", &text("1|beta"));
    assert_eq!(count_lines(&idle, "[LANE RELEASED]"), 1, "{idle:#?}");
    assert!(
        idle.iter()
            .any(|line| line.contains("none (it was already free)")),
        "{idle:#?}"
    );

    // ③ the default-pool release path is untouched by the pool segments.
    game.narrate("lane.release", &text("1"));
    game.assert_kv("lane.1.req", json!(""));
    game.assert_kv("lane.alpha.1.req", json!("r1"));

    // ④ the pooled release tool resolves the pool from the request repo and clears the
    // requirement side (lane + lane_pool) in the same fold.
    let released = tool_round(
        &mut game,
        "git.worktree_release_pool|lane=1;repo=/tmp/checkout-alpha|t5",
        "t5|git.worktree_release_pool|ok|0|lane=1;released=1;parked=1;path=/tmp/checkout-alpha-lane-1",
    );
    assert_eq!(
        count_lines(&released, "[WORKTREE RELEASED]"),
        1,
        "{released:#?}"
    );
    game.assert_kv("lane.alpha.1.req", json!(""));
    game.assert_kv("lane.alpha.1.path", json!(""));
    game.assert_kv("req.1.lane", json!(0));
    game.assert_kv("req.1.lane_pool", json!(""));

    // ⑤ a repo that resolves to no registered pool is refused; the ledger stays untouched.
    let refused = tool_round(
        &mut game,
        "git.worktree_release_pool|lane=2;repo=/tmp/ghost|t6",
        "t6|git.worktree_release_pool|ok|0|lane=2;released=1;parked=0;path=/tmp/ghost-lane-2",
    );
    assert_eq!(
        count_lines(&refused, "[WORKTREE RELEASE REFUSED]"),
        1,
        "{refused:#?}"
    );
    game.assert_kv("lane.alpha.2.req", Json::Null);
}

/// r1 (泳道分池) the conflict radar compares probes **within one pool only**: the same file
/// list in two different pools raises nothing (different repos, unrelated file trees), while
/// the same overlap inside one pool still blocks through the human-path writer.
#[test]
fn conflict_radar_compares_within_pool_only() {
    let mut game = Game::open(&project_mod_dir());
    charter_and_begin(&mut game);
    game.narrate("lane.pool", &text("alpha|/tmp/checkout-alpha"));
    game.narrate("requirement.add", &text("left half|description|subsys-a"));
    game.narrate("requirement.add", &text("right half|description|subsys-b"));
    game.narrate("requirement.add", &text("other repo|description|subsys-c"));
    game.narrate("lane.claim", &text("1|r1|||1|alpha"));
    game.narrate("lane.claim", &text("2|r2|||1|alpha"));
    game.narrate("lane.claim", &text("1|r3|||1"));

    // ① probe alpha-1: the facts land in the pool-scoped probe keys.
    tool_round(
        &mut game,
        "git.lane_probe_pool|lane=1;base=abc;branch=feature/aa11-left;repo=/tmp/checkout-alpha|t1",
        "t1|git.lane_probe_pool|ok|0|lane=1;branch=feature/aa11-left;base=abc;head=1111111;commits=2;clean=1;files=src/a.lua,src/b.lua",
    );
    game.assert_kv("lane.alpha.1.probe.commits", json!(2));
    game.assert_kv("lane.alpha.1.probe.files", json!("src/a.lua,src/b.lua"));

    // ② the SAME file list in a different pool (default lane-1) raises no conflict —
    // the flat ledger would have false-positived here. (`req.conflicts` is created
    // lazily by the conflict writer, so assert the per-requirement fields instead.)
    tool_round(
        &mut game,
        "git.lane_probe|lane=1;base=abc;branch=feature/aa11-other|t2",
        "t2|git.lane_probe|ok|0|lane=1;branch=feature/aa11-other;base=abc;head=3333333;commits=1;clean=1;files=src/a.lua",
    );
    game.assert_kv("req.1.conflict", json!(""));
    game.assert_kv("req.3.conflict", json!(""));

    // ③ the same overlap inside one pool still blocks, labelled with pool-scoped names.
    let radar = tool_round(
        &mut game,
        "git.lane_probe_pool|lane=2;base=abc;branch=feature/aa11-right;repo=/tmp/checkout-alpha|t3",
        "t3|git.lane_probe_pool|ok|0|lane=2;branch=feature/aa11-right;base=abc;head=2222222;commits=1;clean=1;files=src/b.lua,src/c.lua",
    );
    assert_eq!(count_lines(&radar, "[CONFLICT RADAR]"), 1, "{radar:#?}");
    assert!(
        radar
            .iter()
            .any(|line| line.contains("lane-alpha-1") && line.contains("lane-alpha-2")),
        "the radar names both pool-scoped lanes: {radar:#?}"
    );
    game.assert_kv("req.conflicts", json!(2));
    game.assert_kv("req.1.conflict", json!("overlapping file: src/b.lua"));
    game.assert_kv("req.3.conflict", json!(""));

    // ④ `requirement.resolve` remains the only way out.
    game.narrate("requirement.resolve", &text("r1,r2|keep_both|re-checked"));
    game.assert_kv("req.conflicts", json!(0));
    game.assert_kv("req.1.conflict", json!(""));
}

/// r1 (泳道分池) the pooled tool bridge folds into the pool ledger via the **request** repo
/// argument (the host output line does not carry `--repo`): a pooled claim binds
/// `lane.<pool>.<k>.*`, the default tool still binds the flat legacy keys, and an
/// unregistered repo is refused without touching the ledger or the reject counter.
#[test]
fn pooled_tool_bridge_folds_and_refuses() {
    let mut game = Game::open(&project_mod_dir());
    charter_and_begin(&mut game);
    game.narrate("lane.pool", &text("alpha|/tmp/checkout-alpha"));
    game.narrate("requirement.add", &text("pooled req|description|subsys-a"));
    game.narrate("requirement.add", &text("default req|description|subsys-b"));
    game.narrate(
        "lane.claim",
        &text("2|r1|feature/aa11-pool|abc1234|1|alpha"),
    );

    // ① the pooled claim folds into `lane.alpha.2.*` with the host-reported worktree path.
    let claimed = tool_round(
        &mut game,
        "git.worktree_claim_pool|lane=2;req=r1;branch=feature/aa11-pool;repo=/tmp/checkout-alpha|t1",
        "t1|git.worktree_claim_pool|ok|0|lane=2;req=r1;branch=feature/aa11-pool;base=abc1234;clean=1;reused=0;path=/tmp/checkout-alpha-lane-2",
    );
    assert_eq!(
        count_lines(&claimed, "[WORKTREE CLAIMED]"),
        1,
        "{claimed:#?}"
    );
    assert!(
        claimed
            .iter()
            .any(|line| line.contains("lane-alpha-2 <- r1")),
        "the narration names the pool-scoped lane: {claimed:#?}"
    );
    game.assert_kv("lane.alpha.2.req", json!("r1"));
    game.assert_kv("lane.alpha.2.path", json!("/tmp/checkout-alpha-lane-2"));
    // tick 1 = the pre-claim, tick 2 = this tool round's re-bind (same fold, fresh since).
    game.assert_kv("lane.alpha.2.since", json!(2));
    game.assert_kv("req.1.lane", json!(2));
    game.assert_kv("req.1.lane_pool", json!("alpha"));

    // ② the default tool still folds into the flat legacy keys — both pools coexist at k=2.
    let default_claim = tool_round(
        &mut game,
        "git.worktree_claim|lane=2;req=r2;branch=feature/aa11-def|t2",
        "t2|git.worktree_claim|ok|0|lane=2;req=r2;branch=feature/aa11-def;base=abc1234;clean=1;reused=0;path=/tmp/wt-default-2",
    );
    assert_eq!(
        count_lines(&default_claim, "[WORKTREE CLAIMED]"),
        1,
        "{default_claim:#?}"
    );
    game.assert_kv("lane.2.req", json!("r2"));
    game.assert_kv("lane.alpha.2.req", json!("r1"));

    // ③ a repo that resolves to no registered pool is refused before any ledger write, and
    // the tool-face refusal does not move the reject ledger (same face as the legacy path).
    let rejects_before = game.kv("reject.count");
    let ghost = tool_round(
        &mut game,
        "git.worktree_claim_pool|lane=3;req=r1;branch=feature/aa11-ghost;repo=/tmp/ghost|t3",
        "t3|git.worktree_claim_pool|ok|0|lane=3;req=r1;branch=feature/aa11-ghost;base=abc1234;clean=1;reused=0;path=/tmp/ghost-lane-3",
    );
    assert_eq!(
        count_lines(&ghost, "[WORKTREE CLAIM REFUSED]"),
        1,
        "{ghost:#?}"
    );
    game.assert_kv("lane.alpha.3.req", Json::Null);
    game.assert_kv("lane.3.req", Json::Null);
    assert_eq!(
        game.kv("reject.count"),
        rejects_before,
        "the tool-face refusal must not move the reject ledger"
    );
}

/// A3 `fs.artifact_probe` and `vfs.write` feed **one** bounded ledger (`record_artifact` is the
/// single writer), and the VFS path shape is refused before anything is recorded.
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

/// A4 `gate.verify_suite` writes green evidence only in the **settle** micro-state (剧情双层重写:
/// the review stage became the settle act), only when the payload says `passed=1`, and only
/// when the same gate precondition holds. A failed suite narrates honestly and never writes
/// green; the green path writes the same KV as the human `verify.submit` plus provenance.
#[test]
fn verify_suite_result_records_green_evidence_only_in_settle() {
    let mut game = drive_to_done(drive_to_integrating(drive_to_accept(drive_to_dev_pending(
        "semantics-p2",
    ))));

    // ① wrong micro-state (coordinate): refused, no green.
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

/// A5 (子会话清理): `harness.archive_sessions` is retired from the Mod tool bridge — the cleanup
/// is prompt-driven and the only settlement path is the human `settlement.sessions`
/// declaration. Re-pin the retirement facts: the tool is declared in neither `pack.json`
/// nor `allowedTools`, and a direct `settlement.sessions` registration still unblocks the
/// batch close (through the 剧情双层重写 release declaration).
#[test]
fn archive_sessions_is_retired_from_the_tool_bridge() {
    let mod_dir = project_mod_dir();
    let pack: Json = serde_json::from_str(
        &std::fs::read_to_string(mod_dir.join("pack.json")).expect("read pack.json"),
    )
    .expect("parse pack.json");
    assert!(
        pack["tools"].get("harness.archive_sessions").is_none(),
        "pack.json must no longer declare harness.archive_sessions"
    );
    let config: Json = serde_json::from_str(
        &std::fs::read_to_string(mod_dir.join("data/config.json")).expect("read config"),
    )
    .expect("parse data/config.json");
    let allowed = config["allowedTools"]
        .as_array()
        .expect("allowedTools is an array");
    assert!(
        !allowed
            .iter()
            .any(|entry| entry == "harness.archive_sessions"),
        "allowedTools must no longer release harness.archive_sessions"
    );

    // The human declaration path (the only one left) still unblocks the close.
    let mut game = drive_to_release();
    game.narrate("release.skip", &text("纯测试批次，无 develop→master 合并"));
    let closed = game.click("review.note — ");
    assert!(
        closed["narrative"]
            .to_string()
            .contains("[BATCH 1 SETTLED]"),
        "review.note must settle after the human sessions declaration: {closed}"
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
            other => Err(format!("unexpected tool_id: {other}")),
        }
    }
}

/// A7 Synchronous `ygg.tool.call` mounted on `KernelFacade` settles the 3 inline engineering
/// rituals (`git.worktree_claim`, `git.lane_probe`, `git.worktree_release`) in **1 single turn
/// each** without `tool.result` callbacks. 剧情双层重写 window (R7): the lane stays bound through
/// the acceptance and the accept gate — the auto-release happens on the **integration
/// confirm**, not on the gate approval.
#[test]
fn synchronous_tool_bridge_runs_inline_rituals() {
    let mut game = Game::open(&project_mod_dir());
    game.kernel
        .set_tool_runner(Some(Box::new(SyncRitualRunner)));

    charter_and_begin(&mut game);
    game.narrate(
        "requirement.add",
        &text("sync tool bridge|implement synchronous ygg.tool.call|sync-r1"),
    );
    game.narrate("requirement.dispatch", "0");

    // Pre-claim the lane (the branch is self-reported; the auto-claim reuses it).
    // Timing: the plan gate is raised by the delivery itself, so the pre-claim lands right
    // after the submit, before the gate approval that auto-dispatches development.
    submit_with_exec(&mut game, 1, "plan", "sync-p1", "agent-plan");
    game.narrate(
        "lane.claim",
        &text("1|r1|feature/d4e5-sync-bridge|2e24c45|1"),
    );
    game.assert_kv("lane.1.branch", json!("feature/d4e5-sync-bridge"));

    // 1) Approving plan_gate triggers auto_dispatch -> auto_claim_worktree in the SAME turn,
    //    consuming the pre-claimed branch.
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

    // 2) Submitting the development proposal triggers auto_probe_lane in the SAME turn.
    let dev_lines = submit_dev_with_exec(
        &mut game,
        1,
        "sync-p2",
        "agent-plan",
        "src/runtime/tools.lua",
    );
    assert_eq!(
        count_lines(&dev_lines, "[TOOL SETTLED]"),
        1,
        "{dev_lines:#?}"
    );
    assert_eq!(count_lines(&dev_lines, "[LANE PROBE]"), 1, "{dev_lines:#?}");
    game.assert_kv("lane.1.probe.commits", json!(2));
    game.assert_kv("lane.1.probe.clean", json!("1"));
    game.assert_kv("lane.1.probe.head", json!("deadbeef1234"));
    game.assert_kv("tool.settled_count", json!(2));

    game.click("proposal.accept — ");

    // 2) Approving the accept gate opens the integration act — the lane STAYS bound (R7):
    //    no release ritual fires on the gate approval any more.
    let approved_accept = game.click("gate.approve — ");
    let approved_accept_lines = narrative_lines(&approved_accept);
    assert_eq!(
        count_lines(&approved_accept_lines, "[WORKTREE RELEASED]"),
        0,
        "the accept-gate approval must keep the lane bound (integration act owns the tree): {approved_accept_lines:#?}"
    );
    game.assert_kv("lane.1.req", json!("r1"));
    game.assert_kv("req.1.lane", json!(1));

    // 3) The integration confirm releases the worktree in the SAME turn.
    let confirmed = game.narrate("integration.confirm", &text("r1|develop@deadbeef"));
    let confirmed_lines = confirmed;
    assert_eq!(
        count_lines(&confirmed_lines, "[WORKTREE RELEASED]"),
        1,
        "{confirmed_lines:#?}"
    );
    game.assert_kv("lane.1.path", json!(""));
    game.assert_kv("lane.1.req", json!(""));
    game.assert_kv("req.1.lane", json!(0));
    game.assert_kv("tool.settled_count", json!(3));

    // 4) r1 rev1 fail-closed contract: with run.id/run.parent unset, registering
    //    settlement.evolution WITHOUT a `sessions=` segment must NOT auto-archive —
    //    fabricating parent="default" could rename the main session into the trash.
    //    The evolution turn narrates a readable [SESSIONS CLEANUP PENDING] hint instead.
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
        count_lines(&evo_lines, "[SESSIONS CLEANUP PENDING]"),
        1,
        "{evo_lines:#?}"
    );
    game.assert_kv("settlement.sessions.remaining", json!(-1));
    game.assert_kv("tool.settled_count", json!(3));

    // …and an explicit human settlement (the safe path) unblocks the batch close.
    game.narrate(
        "settlement.sessions",
        &text("2|0|/tmp/sessions-trash/sem-explicit"),
    );
    game.narrate("release.skip", &text("纯测试批次，无 develop→master 合并"));
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
    game.assert_kv("fsm.stage", json!(null)); // live is the FSM initial state: not materialized in KV yet
    game.assert_kv("run.phase", json!(""));
}

// ───────────────────── r13 · fuel autoscale + requirement list pagination ─────────────────────

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
    charter_and_begin(&mut game);

    // r1 goes the full nine acts to `done` (the only closed row).
    game.narrate(
        "requirement.add",
        &text("synthetic requirement|synthetic description|page-r1"),
    );
    game.narrate("requirement.dispatch", "0");
    submit(&mut game, 1, "plan", "page-p1");
    game.click("gate.approve — ");
    submit(&mut game, 1, "development", "page-p2");
    game.click("proposal.accept — ");
    game.click("gate.approve — ");
    game.narrate("integration.confirm", &text("r1|develop@abc1234"));
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
/// beside the resident trio — the pump-pause signal `batchDrained` reads
/// coordinate ∧ gate_problem()==nil ∧ not armed (剧情双层重写: the run-level facts, no stage name).
#[test]
fn summary_projection_carries_play_and_resident_trio() {
    let mut game = Game::open(&project_mod_dir());

    // Fresh run: the boot lands in the charter act, the closer preconditions are unmet.
    let summary = game.reply("agent.summary", "0")["meta"]["summary"].clone();
    assert_eq!(
        summary["play"],
        json!("guide"),
        "play.mode default is guide and must be projected: {summary}"
    );
    assert_eq!(summary["lifecycle"], json!("resident"));
    assert_eq!(summary["batch"], json!(1));
    assert_eq!(summary["batchDrained"], json!(false));

    // Chartered and begun, but the open (draft) requirement keeps `gate_problem()` non-nil,
    // so the pump is still not drained.
    charter_and_begin(&mut game);
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

    // A completed, settled batch drains the pump: coordinate ∧ nothing pending ∧ not armed.
    game.narrate("requirement.dispatch", "0");
    submit(&mut game, 1, "plan", "watchdog-p1");
    game.click("gate.approve — ");
    submit(&mut game, 1, "development", "watchdog-p2");
    game.click("proposal.accept — ");
    game.click("gate.approve — ");
    game.narrate("integration.confirm", &text("r1|develop@abc1234"));
    game.click("coordinate.submit — ");
    game.click("verify.submit all_green=true — ");
    game.narrate(
        "settlement.evolution",
        &text("|screened|c=0/0|sessions=0/0/none|todo=0/cleared"),
    );
    game.narrate("release.skip", &text("纯测试批次"));
    game.click("review.note — ");
    let summary = game.reply("agent.summary", "0")["meta"]["summary"].clone();
    assert_eq!(summary["batch"], json!(2));
    // 文档维护义务不再派生需求行（用户 2026-11-15 裁定：CONTEXT / DESIGN 维护归主会话自办）：
    // 注册表没有新增行，pump 在批结算后即可 drain——拦截转移到了**下一次批边界**
    // （coordinate.submit 被 docs.maintenance.due 硬阻塞），而不是靠一条占位的需求行。
    game.assert_kv("req.count", json!(1));
    game.assert_kv("docs.maintenance.due", json!("1"));
    assert_eq!(
        summary["batchDrained"],
        json!(true),
        "a settled batch with no open requirements drains the pump: {summary}"
    );
    // 主会话做完复核后解除义务；此后新需求正常入册、可派发。
    let cleared = game.narrate("config.set", &text("docs.maintenance.done=1"));
    assert!(
        cleared
            .iter()
            .any(|l| l.contains("[DOCS MAINTENANCE CLEARED]")),
        "{cleared:#?}"
    );
    game.assert_kv("docs.maintenance.due", json!(""));
    game.narrate("requirement.add", &text("watchdog b2|desc|watchdog-2"));
    game.assert_kv("req.count", json!(2));
}

// ───────────────────────── harness tools 机证 cross-check（r3 / harness 机证）─────────────────────────

/// 注入 harness 工具 runner：按 tool_id 返回 controlled 输出（k=v 对的 BTreeMap 形式，
/// 与 InProcessToolRunner.parse_kv_output 的产物同形）。用于机证 cross-check 的矛盾/一致两路。
struct HarnessRunner {
    todo_remaining: i32,
    sessions_matching: i32,
}

impl HostToolRunner for HarnessRunner {
    fn call(
        &mut self,
        tool_id: &str,
        _args: &std::collections::BTreeMap<String, String>,
    ) -> Result<std::collections::BTreeMap<String, String>, String> {
        let mut out = std::collections::BTreeMap::new();
        match tool_id {
            "harness.todo-check" => {
                out.insert("found".to_owned(), "1".to_owned());
                out.insert("total".to_owned(), "3".to_owned());
                out.insert("remaining".to_owned(), self.todo_remaining.to_string());
                out.insert("active".to_owned(), "1".to_owned());
                out.insert(
                    "ok".to_owned(),
                    if self.todo_remaining == 0 { "1" } else { "0" }.to_owned(),
                );
                Ok(out)
            }
            "harness.sessions-count" => {
                out.insert("total".to_owned(), "4".to_owned());
                out.insert("matching".to_owned(), self.sessions_matching.to_string());
                out.insert("trash".to_owned(), "1".to_owned());
                out.insert(
                    "ok".to_owned(),
                    if self.sessions_matching == 0 {
                        "1"
                    } else {
                        "0"
                    }
                    .to_owned(),
                );
                Ok(out)
            }
            other => Err(format!("unexpected tool_id: {other}")),
        }
    }
}

/// 未注册 tools.root：coordinate.submit 进入结算时给一次性 [TOOLS UNREGISTERED] 提醒；
/// settlement.todo 走人证旧路径 + [TOOLS ROOT MISSING] 提示，写 source=human。
#[test]
fn settlement_todo_unregistered_falls_back_to_human_attestation() {
    let mut game = drive_to_settle();
    // 未注册 tools.root（drive_to_settle 内的 coordinate.submit 已发一次性提醒）。
    game.assert_kv("settings.tools.root", json!(null));
    // 独立 settlement.todo 动作：人证路径（无工具挂载也无注册，回退）。
    let lines = game.narrate("settlement.todo", &text("0|re-checked the todo list"));
    assert!(
        lines.iter().any(|l| l.contains("[TOOLS ROOT MISSING]")),
        "unregistered settlement.todo must hint the missing tools.root: {lines:#?}"
    );
    game.assert_kv("settlement.todo.remaining", json!(0));
    game.assert_kv("settlement.todo.source", json!("human"));
}

/// 注册 tools.root + 挂载 harness runner：机证 remaining>0 与人证 remaining=0 矛盾 →
/// settlement.todo 被拒（拒绝前缀 + 零状态写入，remaining 保持 reset_settlement 基线）。
#[test]
fn settlement_todo_machine_contradiction_is_refused_with_zero_writes() {
    let mut game = drive_to_settle();
    game.kernel.set_tool_runner(Some(Box::new(HarnessRunner {
        todo_remaining: 2,
        sessions_matching: 0,
    })));
    // 注册 tools.root + 主会话 session id（mod 沙箱读不到进程环境，参数只能来自 KV）。
    game.narrate("config.set", &text("tools.root=/tmp/ygg-tools"));
    game.narrate("config.set", &text("tools.session=semantics-parent-uuid"));
    game.assert_kv("settings.tools.root", json!("/tmp/ygg-tools"));
    let lines = game.narrate("settlement.todo", &text("0|re-checked the todo list"));
    assert_refused(&lines, "machine remaining=2");
    // 拒绝 = 零状态写入：settlement.todo.remaining 未被本动作登记（仍 reset 基线）。
    game.assert_kv("settlement.todo.source", json!(null));
    game.assert_kv("settlement.todo.verified_by", json!(""));
}

/// 注册 + 机证一致（remaining=0 / matching=0）：settlement.todo 与 settlement.sessions
/// 均通过并写 source=machine:* 溯源键。
#[test]
fn settlement_machine_agreement_passes_and_records_provenance() {
    let mut game = drive_to_settle();
    game.kernel.set_tool_runner(Some(Box::new(HarnessRunner {
        todo_remaining: 0,
        sessions_matching: 0,
    })));
    game.narrate("config.set", &text("tools.root=/tmp/ygg-tools"));
    game.narrate("config.set", &text("tools.session=semantics-parent-uuid"));
    game.narrate("config.set", &text("tools.sessions_root=/tmp/ygg-sessions"));
    let todo_lines = game.narrate("settlement.todo", &text("0|re-checked the todo list"));
    assert!(
        todo_lines.iter().any(|l| l.contains("[TOOLS CHECK OK]")),
        "machine agreement must narrate the check pass: {todo_lines:#?}"
    );
    game.assert_kv("settlement.todo.remaining", json!(0));
    game.assert_kv(
        "settlement.todo.source",
        json!("machine:harness.todo-check"),
    );
    let sess_lines = game.narrate(
        "settlement.sessions",
        &text("1|0|~/.pi/agent/sessions-trash/semantics"),
    );
    assert!(
        sess_lines.iter().any(|l| l.contains("[TOOLS CHECK OK]")),
        "sessions machine agreement must narrate the pass: {sess_lines:#?}"
    );
    game.assert_kv("settlement.sessions.remaining", json!(0));
    game.assert_kv(
        "settlement.sessions.source",
        json!("machine:harness.sessions-count"),
    );
}

// ───────────────────── r8 · E3: the armed surface ─────────────────────
//
// 命题：证据清单过去只度量「代码存在」。r4 首版交付代码完整、10 条用例、三套门禁 473/158/77
// 全绿，而 `data/config.json` 里 `readGate` / `readPlan` 根本不存在——线上零拦截。缺的不是用例，
// 是一条**通用规则**：凡「缺席即失效 / 关掉即失效」的开关，必须在**入库态**生效，并被一条
// `ships_armed_*` 用例钉住「生效态 + 退让路径」两半。
//
// 三种失效形态在本仓同时存在，这就是本规则要覆盖的全集：
//   ① 可选段缺席（`readPlan` / `readGate` 在 schema 里是可选 → 整段不见即 inert，r4 的实况）；
//   ② 必填键缺席（`gates.require_all_green` 是 required → 装载期就拦，schema 能保这一格）；
//   ③ **必填键值不对**（`false` 是 schema 合法值，但 `false` 一样让保护静默失效）——
//      schema 只管形状，值域只能靠 armed 断言钉。③ 是②的盲区，也是本规则多咬的那一口。
//
// 三条判定式：
//   R1 分区完备（机检）—— 配置面每个键都必须被分类：生成区 / 手写区 × armed / fixed；
//      生成区的归属**解析自生成器的自报键表**，不靠手抄常量，两边分叉立刻红。
//   R2 armed 有据（机检）—— Armed 键入库态生效（非空对象 / 布尔 true），且同文件里有
//      `fn <slug>_ships_armed` 用例。
//   R3 退让可回（**不**机检）—— `ships_armed` 用例内部必须同时断言「剥掉该键的副本仍走
//      退让路径」。刻意不造「检查用例内容是否完整」的元检查：那是无穷倒退，也违反
//      「不写自造检查器当纪律载体」的红线；R3 靠样板复制 + 评审，此处如实记账。

/// 配置面的**来源**：生成区（`scripts/gen-scene-readfirst.py --readset` 产出；手改会被内容寻址
/// 的 manifest 与文档头比对拦下）还是手写区（人工维护，无生成器覆盖）。
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Origin {
    Generated,
    HandWritten,
}

/// Armed 键「生效」的判定形态。新开关选一种形态登记即可——**不绑定具体键名**。
#[derive(Debug, Clone, Copy)]
enum ArmedProbe {
    /// 非空对象即生效（必读集 / ver 台账这类表）。
    NonEmptyObject,
    /// 布尔 `true` 即生效（`effective(path) == true` 式的开关）。
    IsTrue,
}

/// 配置面上的一格：键路径 + 来源 + 是否为「开关型」。
struct ConfigKey {
    /// 点分路径；顶层键只写一段。
    path: &'static str,
    origin: Origin,
    /// `Some((slug, probe))` = 关掉即静默削弱一项保护；`slug` 决定 `ships_armed` 用例名前缀。
    armed: Option<(&'static str, ArmedProbe)>,
}

/// 配置面分区表（r8 / R1）。**新增 `data/config.json` 顶层键或 `gates` 子键必须在此登记**，
/// 否则 `config_surface_every_key_is_classified` 直接红——这正是 R1 的咬合力所在：
/// 加开关的人必须先回答「这是固定项还是 armed」。
const CONFIG_SURFACE: &[ConfigKey] = &[
    // ── 生成区：生成器拥有；两份都是 armed（缺席 ⇒ 必读集为空 ⇒ 读签门禁整体 inert）。
    ConfigKey {
        path: "readPlan",
        origin: Origin::Generated,
        armed: Some(("read_gate", ArmedProbe::NonEmptyObject)),
    },
    ConfigKey {
        path: "readGate",
        origin: Origin::Generated,
        armed: Some(("read_gate", ArmedProbe::NonEmptyObject)),
    },
    // ── 手写区：缺席走 lib 缺省，且不削弱任何保护。
    ConfigKey {
        path: "play",
        origin: Origin::HandWritten,
        armed: None,
    },
    ConfigKey {
        path: "drive",
        origin: Origin::HandWritten,
        armed: None,
    },
    ConfigKey {
        path: "evolution",
        origin: Origin::HandWritten,
        armed: None,
    },
    ConfigKey {
        path: "autosave",
        origin: Origin::HandWritten,
        armed: None,
    },
    ConfigKey {
        path: "hot_reload",
        origin: Origin::HandWritten,
        armed: None,
    },
    ConfigKey {
        path: "harness",
        origin: Origin::HandWritten,
        armed: None,
    },
    ConfigKey {
        path: "settlement",
        origin: Origin::HandWritten,
        armed: None,
    },
    // `allowedTools` 缺席 = 一个工具都不放行（fail-closed）：缺席**收紧**而非削弱，故非 armed。
    ConfigKey {
        path: "allowedTools",
        origin: Origin::HandWritten,
        armed: None,
    },
    // ── `gates` 容器与其开关。
    ConfigKey {
        path: "gates",
        origin: Origin::HandWritten,
        armed: None,
    },
    // 见文件尾 `gates_strict_ships_armed_*` 用例：schema 只保存在，值域靠此断言钉。
    ConfigKey {
        path: "gates.require_all_green",
        origin: Origin::HandWritten,
        armed: Some(("gates_strict", ArmedProbe::IsTrue)),
    },
];

const NULL_CONFIG: Json = Json::Null;

/// 按点分路径取值（缺键 → `Null`，不 panic：R2 要把「缺席」本身报成断言失败而不是崩溃）。
fn config_value(config: &Json, path: &str) -> Json {
    let mut node = config;
    for segment in path.split('.') {
        node = node.get(segment).unwrap_or(&NULL_CONFIG);
    }
    node.clone()
}

/// 某一层被声明过的键（`depth` = 段数，只取该层的末段）。
fn declared_keys_at(depth: usize) -> Vec<String> {
    CONFIG_SURFACE
        .iter()
        .filter(|entry| entry.path.split('.').count() == depth)
        .map(|entry| {
            entry
                .path
                .rsplit('.')
                .next()
                .expect("non-empty path")
                .to_owned()
        })
        .collect()
}

/// 覆盖检查：实际键集合与声明集合**双向**相等——漏登记（实际多）与残留（表里多）都红。
#[track_caller]
fn assert_cover(label: &str, actual: &[String], declared: &[String]) {
    let mut unclassified: Vec<&String> = actual
        .iter()
        .filter(|key| !declared.iter().any(|known| known == *key))
        .collect();
    let mut stale: Vec<&String> = declared
        .iter()
        .filter(|known| !actual.iter().any(|key| key == *known))
        .collect();
    unclassified.sort();
    stale.sort();
    assert!(
        unclassified.is_empty(),
        "{label}: unclassified config key(s) {unclassified:?} — register them in CONFIG_SURFACE \
         (a new switch must answer `fixed` or `armed` before it ships)"
    );
    assert!(
        stale.is_empty(),
        "{label}: CONFIG_SURFACE declares key(s) {stale:?} that the shipped config no longer has"
    );
}

/// 生成器自报它拥有哪些键（`scripts/gen-scene-readfirst.py` 的 `READSET_KEYS`）。
/// 解析而不是抄一份常量：两处一旦分叉，R1 立刻红（生成区归属是事实，不是声明）。
fn generator_owned_keys() -> Vec<String> {
    let path = repo_dir().join("scripts/gen-scene-readfirst.py");
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    let line = source
        .lines()
        .find(|line| line.trim_start().starts_with("READSET_KEYS"))
        .expect("gen-scene-readfirst.py must declare READSET_KEYS (the generated-region oracle)");
    let mut keys = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find('"') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('"') else { break };
        keys.push(after[..end].to_owned());
        rest = &after[end + 1..];
    }
    assert!(
        !keys.is_empty(),
        "READSET_KEYS parsed to nothing — the oracle line changed shape: {line}"
    );
    keys
}

/// **R1 · 分区完备**：配置面每个键都被分类，且生成区归属 == 生成器自报的键表。
#[test]
fn config_surface_every_key_is_classified() {
    let config = support::read_json(&project_mod_dir().join("data/config.json"));
    let object = config
        .as_object()
        .expect("data/config.json top level is an object");

    let actual_top: Vec<String> = object.keys().cloned().collect();
    assert_cover("top-level", &actual_top, &declared_keys_at(1));

    let actual_gates: Vec<String> = object
        .get("gates")
        .and_then(Json::as_object)
        .map(|table| table.keys().cloned().collect())
        .unwrap_or_default();
    assert_cover("gates", &actual_gates, &declared_keys_at(2));

    // 生成区不是「我们在表里这么写」，而是「生成器这么报」——两处必须逐项相等。
    let mut declared_generated: Vec<String> = CONFIG_SURFACE
        .iter()
        .filter(|entry| entry.origin == Origin::Generated)
        .map(|entry| entry.path.to_owned())
        .collect();
    declared_generated.sort();
    let mut actual_generated = generator_owned_keys();
    actual_generated.sort();
    assert_eq!(
        actual_generated, declared_generated,
        "the generated region must equal READSET_KEYS in scripts/gen-scene-readfirst.py: a key the \
         generator owns but the table calls hand-written (or the reverse) means the partition is a \
         declaration about nothing"
    );

    // 分区必须真的分出了 armed：一张全是 fixed 的表会让 R2 空转。
    let armed = CONFIG_SURFACE
        .iter()
        .filter(|entry| entry.armed.is_some())
        .count();
    assert!(
        armed >= 2,
        "the surface must classify at least two armed switches, got {armed} — an empty armed set \
         turns R2 into a vacuous green"
    );
}

/// **R2 · armed 有据**：每个 Armed 键在入库态生效，且同文件里有一条 `ships_armed` 用例。
///
/// 这一条直接咬住 r4 的失效态：`readGate` / `readPlan` 当时**整键缺席**，而三套门禁照样全绿。
#[test]
fn armed_features_ship_armed_with_a_semantics_case() {
    let config = support::read_json(&project_mod_dir().join("data/config.json"));
    let source_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/agent_seed_semantics.rs");
    let source = std::fs::read_to_string(&source_path)
        .unwrap_or_else(|err| panic!("read {}: {err}", source_path.display()));

    let armed: Vec<&ConfigKey> = CONFIG_SURFACE
        .iter()
        .filter(|entry| entry.armed.is_some())
        .collect();
    assert!(!armed.is_empty(), "an empty armed table proves nothing");

    for entry in armed {
        let (slug, probe) = entry.armed.expect("armed entry");
        let value = config_value(&config, entry.path);

        // ①入库态生效：缺键 / 空表 / false 都不是「已 armed」，而是 r4 那种「代码在、开关没开」。
        match probe {
            ArmedProbe::NonEmptyObject => {
                let table = value.as_object().unwrap_or_else(|| {
                    panic!(
                        "{} must ship as an object — the shipped config carries no such key, which \
                         is exactly the r4 failure mode (code complete, gate inert): {value}",
                        entry.path
                    )
                });
                assert!(
                    !table.is_empty(),
                    "{} must ship **armed**: an empty table is the inert state",
                    entry.path
                );
            }
            ArmedProbe::IsTrue => {
                assert_eq!(
                    value,
                    json!(true),
                    "{} must ship as true — absent or false silently disables the guard it fronts",
                    entry.path
                );
            }
        }

        // ② 同文件里有 ships_armed 用例：生效态 + 退让路径（R3）由那条用例承担。
        //    针要求行首 `fn `（Rust 顶层项 = 列 0）：注释里提到用例名不能冒充用例本体。
        let needle = format!("\nfn {slug}_ships_armed");
        assert!(
            source.contains(&needle),
            "the armed switch {:?} (slug {slug}) has no `fn {slug}_ships_armed*` case: an armed \
             config key without an assertion on BOTH halves (armed behaviour + the disabled \
             fallback) is the silence this rule exists to remove",
            entry.path
        );
    }
}

/// **R3 的第二实例**（第一个是 `read_gate_ships_armed_and_a_bare_copy_stays_inert`）：
/// `gates.require_all_green` 是「关掉即失效」的开关——`src/core/config.lua` 的
/// `strict_gates()` 判的是 `== true`。
///
/// 后果不是旁枝：红盘（`verify.submit all_green=false`）会被记成 `verify.passed=true`——
/// 能骗过所有后续门禁的那种错误。全仓此前零用例引用该键。
#[test]
fn gates_strict_ships_armed_and_a_bare_copy_bypasses_a_red_verify() {
    // ① 入库态：红盘如实记失败，整批退回协调。
    let mut armed = drive_to_settle();
    let failed = armed.click("verify.submit all_green=false — ");
    assert!(
        failed["narrative"].to_string().contains("[GATES FAILED]"),
        "an armed require_all_green must send a red verify back to coordinate: {failed}"
    );
    armed.assert_kv("verify.passed", json!(false));
    armed.assert_kv("run.phase", json!(""));
    armed.assert_kv("verify.failures", json!(1));

    // ② 值改成 false（**schema 合法**：required 只管存在，不管值）：同一条路径必须走退让分支。
    //    R3 的回归保护：退让路径必须仍然可选（关闸是设计内的安全阀），但它的存在必须被写在
    //    断言里——「不拦人」与「红盘记成绿盘」是同一行代码的两面，不能靠记忆区分。
    let bare_dir = support::temp_dir("gates-strict-bare");
    support::copy_dir(&project_mod_dir(), &bare_dir);
    let mut bare_config = support::read_json(&bare_dir.join("data/config.json"));
    bare_config["gates"]["require_all_green"] = json!(false);
    support::write_json(&bare_dir.join("data/config.json"), &bare_config);
    support::reseal(&bare_dir);

    let mut bare = drive_to_settle_at(&bare_dir);
    let bypassed = bare.click("verify.submit all_green=false — ");
    assert!(
        bypassed["narrative"]
            .to_string()
            .contains("[GATES BYPASSED]"),
        "a disabled require_all_green must narrate the bypass, not fail silently: {bypassed}"
    );
    bare.assert_kv("verify.passed", json!(true));
    bare.assert_kv("run.phase", json!(""));
    bare.assert_kv("verify.failures", json!(1));
}
