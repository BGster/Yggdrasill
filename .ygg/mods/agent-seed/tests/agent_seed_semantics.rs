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

use serde_json::{json, Value as Json};

use ygg_sandbox::driver::KernelFacade;
use ygg_sandbox::modload::ModBundle;

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

}

/// Rust string → JSON string payload (a dispatch payload must be valid JSON).
fn text(payload: &str) -> String {
    serde_json::to_string(payload).expect("serialize the string payload")
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

// ───────────────────────── charter & resident lifecycle ─────────────────────────

// ───────────────────── requirement pipeline: gates & deliverables ─────────────────────

// ───────────────────── inline executor ledger (`exec=`) ─────────────────────

// ───────────────────── settlement merge & the batch loop ─────────────────────

// ───────────────────── batch-maintenance auto-append (r3 / 批结算自动维护) ─────────────────────

// ───────────────────── gate locality & the integration act ─────────────────────

// ───────────────────── r1 · single operation map + registry fail-closed ─────────────────────

// ───────────────────── r11 · unknown events fail closed ─────────────────────

// ───────────────────── r2 · lane branch naming is enforced ─────────────────────

// ───────────────────── r3 · submitter identity (actor chapter + profiles) ─────────────────────

// ───────────────────── r4 · read-gate: the four dimensions ─────────────────────

// ───────────────── r9 · attribution: the signature belongs to an actor ─────────────────

// ───────────────────── r4/r6/r7 · layout, fragments, tool declarations ─────────────────────

/// A mod resource read straight from disk (the declaration surface is a file, not Lua state).
fn read_json(path: &Path) -> Json {
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|err| panic!("parse {}: {err}", path.display()))
}

// ───────────────────── E6 · Mod text hygiene (别名退役/子会话清理) ─────────────────────

// ──────────────── r6: docs/design 契约树的指针 / 索引机检 ────────────────

// ───────────────────── tool bridge (T1–T4) ─────────────────────

// ───────────────────── A · active subsystem ─────────────────────

// ───────────────────── r13 · fuel autoscale + requirement list pagination ─────────────────────

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
    // 本文件住在 Mod 内（迁移期由 crates/agent-seed-tests 的纯 manifest 登记为 target），
    // 因此不能假设它在 <manifest>/tests/ 下。
    let source_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.ygg/mods/agent-seed/tests/agent_seed_semantics.rs");
    let mut source = std::fs::read_to_string(&source_path)
        .unwrap_or_else(|err| panic!("read {}: {err}", source_path.display()));
    // 迁移期：断言可能已经搬到 Lua 侧（`tests/*.lua`）。把 Lua 用例一并纳入扫描，
    // 否则「armed 键必须有断言」这条规则会在迁移中途变成假绿——Rust 侧删了、Lua 侧加了，
    // 元用例却只读 Rust 文件，于是要么误报缺失、要么（若放宽）彻底失效。
    if let Some(tests_dir) = source_path.parent() {
        if let Ok(entries) = std::fs::read_dir(tests_dir) {
            let mut lua: Vec<PathBuf> = entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("lua"))
                .collect();
            lua.sort();
            for path in lua {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    source.push('\n');
                    source.push_str(&text);
                }
            }
        }
    }

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
        // Rust 顶层项（行首 `fn `）或 Lua 侧的显式锚点 `-- [armed-case] <slug>_ships_armed`。
        let rust_needle = format!("\nfn {slug}_ships_armed");
        let lua_needle = format!("\n-- [armed-case] {slug}_ships_armed");
        assert!(
            source.contains(&rust_needle) || source.contains(&lua_needle),
            "the armed switch {:?} (slug {slug}) has no `fn {slug}_ships_armed*` case or \
             `-- [armed-case] {slug}_ships_armed` anchor: an armed \
             config key without an assertion on BOTH halves (armed behaviour + the disabled \
             fallback) is the silence this rule exists to remove",
            entry.path
        );
    }
}
