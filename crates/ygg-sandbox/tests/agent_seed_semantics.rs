//! agent-seed 语义门禁：需求级状态机（5 存储态 + `plan_ok` 派生展示名）与双层节点图的
//! **可判定**断言（AGENTS.md §一·9 的行为验收面）。
//!
//! 为什么需要这一层：`playtest` 的全 1 走查只证明 happy path 能到【结局——它对
//! `gate.reject` / `requirement.amend` / `proposal.reject` 这些**分叉**一条都不走。
//! 上一轮的三处缺陷（accept 打回丢 `plan_ok`、map 流水线缺验收跳、旁路方向反）
//! 全都在 happy path 之外，全 1 门禁对它们恒绿。本文件把这些分叉写成断言：
//!
//! - 状态断言读 **`save` 回执的 `kv.entries`**（`req.*` 是状态机事实，不是散文）；
//! - 展示断言读 **`requirement.list` 的派生展示名**（`[ready]` / `[plan_gate]` / …）；
//! - 图层断言读 **`map.show` 的图窗口**（操作视图 vs 设计视图两套契约）；
//! - 人类动作一律走 **guide 的 `ygg.choice_selected`**（票据只在这里盖章，§8）。
//!
//! 边界（与 AGENTS.md §一·7 同口径，别在这里加出口）：
//! - 只用公开装载面（`ModBundle` / `KernelFacade` / dispatch / save-load），不改 host CLI、
//!   不改生产代码、不引入旁路脚本；
//! - **不读写任何 run 目录**：legacy 兼容用**内存 save fixture**（`save` 回执里就地删键再
//!   `restore`），不碰 `autosave.json` / `events.jsonl`；
//! - legacy 用例只证明「当前 reducer 能读缺键的旧存档」，不构造历史版本引擎、不伪造事件日志。

use std::path::{Path, PathBuf};

use serde_json::{json, Value as Json};

use ygg_sandbox::driver::KernelFacade;
use ygg_sandbox::modload::ModBundle;
use ygg_sandbox::playtest::{playtest, PlaytestOptions, Reached};

// ───────────────────────── 定位 ─────────────────────────

/// 仓库根（`CARGO_MANIFEST_DIR` = `crates/ygg-sandbox`，两级上行到仓库根）。
fn repo_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// 本项目实际装载的实例（`data/scenes.json` 与 `data/playbook.json` 是本项目的现场面）。
fn project_mod_dir() -> PathBuf {
    repo_dir().join(".ygg/mods/agent-seed")
}

/// 对外分发的种子模板（骨架逐字节一致；`data/` 两侧各自维护）。
fn seed_mod_dir() -> PathBuf {
    repo_dir().join("packages/mods/agent-seed")
}

// ───────────────────────── 驱动面 ─────────────────────────

/// 一局的最小驱动面：装载 + 派发 + 读档。
///
/// `bundle` 留着手是为了 legacy 用例能按同一份装载参数重建内核（`ModBundle::restore`
/// 与 `kernel()` 同分派，见 modload.rs 文档）。
struct Game {
    bundle: ModBundle,
    kernel: KernelFacade,
}

impl Game {
    /// 装载并在 `event.start` 上开局（与宿主走同一条装载面）。
    fn open(dir: &Path) -> Self {
        let bundle =
            ModBundle::load(dir).unwrap_or_else(|err| panic!("装载 {} 失败：{err}", dir.display()));
        let kernel = bundle.kernel().expect("内核构造");
        let mut game = Self { bundle, kernel };
        game.narrate("event.start", "0");
        game
    }

    /// 派发并断言信封 `ok`（域错误信封一律算测试失败）。
    #[track_caller]
    fn reply(&mut self, event: &str, payload: &str) -> Json {
        let reply: Json = serde_json::from_str(&self.kernel.dispatch(event, payload))
            .unwrap_or_else(|err| panic!("{event} 的回执不是 JSON：{err}"));
        assert_eq!(reply["ok"], json!(true), "{event} 派发失败：{reply}");
        reply
    }

    /// 派发 + 取叙事行。
    #[track_caller]
    fn narrate(&mut self, event: &str, payload: &str) -> Vec<String> {
        let reply = self.reply(event, payload);
        reply["narrative"]
            .as_array()
            .unwrap_or_else(|| panic!("{event} 的 narrative 不是数组：{reply}"))
            .iter()
            .map(|line| {
                line.as_str()
                    .unwrap_or_else(|| panic!("叙事行不是字符串：{line:?}"))
                    .to_owned()
            })
            .collect()
    }

    /// `save` 回执里的存档对象（状态断言的事实面 = `kv.entries`）。
    #[track_caller]
    fn save(&mut self) -> Json {
        self.reply("save", "0")["save"].clone()
    }

    /// 存档里的一个 KV（缺键 → `Null`；legacy 用例正是要观测「键不在」）。
    #[track_caller]
    fn kv(&mut self, key: &str) -> Json {
        self.save()["kv"]["entries"][key].clone()
    }

    /// 断言 KV 取值（`Null` 即断言该键缺席）。
    #[track_caller]
    fn assert_kv(&mut self, key: &str, want: Json) {
        let got = self.kv(key);
        assert_eq!(got, want, "KV `{key}` 期望 {want}，实际 {got}");
    }

    /// 当前待决选择的选项文本（`state` 是只读诊断面，不推进状态）。
    #[track_caller]
    fn options(&mut self) -> Vec<String> {
        let reply = self.reply("state", "0");
        reply["pendingChoice"]["options"]
            .as_array()
            .unwrap_or_else(|| panic!("没有待决选择：{reply}"))
            .iter()
            .map(|item| {
                item.as_str()
                    .unwrap_or_else(|| panic!("选项不是字符串：{item:?}"))
                    .to_owned()
            })
            .collect()
    }

    /// guide 路由：按选项文本前缀点菜单（人类动作的**唯一**合法入口，见 AGENTS.md §8）。
    #[track_caller]
    fn click(&mut self, prefix: &str) -> Json {
        let options = self.options();
        let index = options
            .iter()
            .position(|text| text.starts_with(prefix))
            .unwrap_or_else(|| panic!("菜单里没有以 `{prefix}` 开头的项：{options:?}"));
        let payload = (index + 1).to_string();
        self.reply("ygg.choice_selected", &payload)
    }

    /// 派生展示名：`requirement.list` 的 `[ready]` 那一格——「展示成什么」的断言面。
    #[track_caller]
    fn phase(&mut self, id: &str) -> String {
        let needle = format!(" {id} [");
        let lines = self.narrate("requirement.list", "0");
        let row = lines
            .iter()
            .find(|line| line.contains(&needle))
            .unwrap_or_else(|| panic!("需求列表里没有 {id}：{lines:#?}"));
        let open = row.find('[').expect("展示名缺左括号");
        let close = row.find(']').expect("展示名缺右括号");
        row[open + 1..close].to_owned()
    }
}

/// Rust 字符串 → JSON 字符串载荷（`dispatch` 的 payload 必须是合法 JSON 标量）。
fn text(payload: &str) -> String {
    serde_json::to_string(payload).expect("序列化字符串载荷")
}

/// 拒绝判定：拒绝文案的前缀由 `pack.json` 的 `rejectionPrefixes` 声明。
fn refused(lines: &[String], needle: &str) -> bool {
    lines.iter().any(|line| {
        ["【动作被拒】", "【载荷被拒】", "【需求待办】"]
            .iter()
            .any(|prefix| line.starts_with(prefix))
            && line.contains(needle)
    })
}

/// 断言「被拒且点名了该动作/原因」。
#[track_caller]
fn assert_refused(lines: &[String], needle: &str) {
    assert!(
        refused(lines, needle),
        "期望一条点名 `{needle}` 的拒绝文案，实际：{lines:#?}"
    );
}

// ───────────────────────── 驱动脚本 ─────────────────────────

/// 开局 → coordinate → 一条需求 → 分析 job 起跑（非人类动作走直派，不必盖章）。
fn start_requirement(game: &mut Game) {
    game.narrate("task.intake", &text("语义门禁局|合成任务的描述"));
    game.narrate(
        "requirement.add",
        &text("合成需求|合成需求的描述|semantics-r1"),
    );
    game.narrate("requirement.dispatch", "0");
    game.assert_kv("req.1.job", json!("analysis"));
}

/// 提交一条 `r1` 的提案：`req|rev|stage|summary|artifacts|changes|ext`（changes 留空）。
fn submit(game: &mut Game, rev: u32, stage: &str, ext: &str) {
    let payload = text(&format!(
        "r1|{rev}|{stage}|{stage} 提案摘要|artifacts/{stage}.md||{ext}"
    ));
    game.narrate("proposal.submit", &payload);
}

/// 驱动到 **plan 门禁待批**：分析提案验收 → 派 plan → plan 提案验收。
fn drive_to_plan_gate() -> Game {
    drive_to_plan_gate_in(&project_mod_dir())
}

fn drive_to_plan_gate_in(dir: &Path) -> Game {
    let mut game = Game::open(dir);
    start_requirement(&mut game);
    submit(&mut game, 1, "analysis", "semantics-p1");
    game.click("proposal.accept —— ");
    game.assert_kv("req.1.status", json!("planning"));
    game.narrate("requirement.dispatch", "0");
    submit(&mut game, 1, "plan", "semantics-p2");
    game.click("proposal.accept —— ");
    game.assert_kv("req.1.gate", json!("plan"));
    game
}

/// 从 plan 门禁继续到 **accept 门禁待批**：批准 plan → 派开发 → 交付提案验收。
fn drive_to_accept(mut game: Game) -> Game {
    game.click("gate.approve —— ");
    game.assert_kv("req.1.plan_ok", json!(true));
    game.narrate("requirement.dispatch", "0");
    game.assert_kv("req.1.job", json!("development"));
    submit(&mut game, 1, "development", "semantics-p3");
    game.click("proposal.accept —— ");
    game.assert_kv("req.1.gate", json!("accept"));
    game
}

/// 驱动到 **开发中 + 待验收的开发提案**（`accept` 门禁尚未落）：批准 plan → 派开发 → 交付提案。
///
/// 注意：门禁待批期间不再收提案（`submit_slot_ok` 的 gate 守卫），所以「开发提案被拒」
/// 只能发生在验收之前——不接受 `drive_to_accept` 之后再补提案的写法。
fn drive_to_dev_pending(ext: &str) -> Game {
    let mut game = drive_to_plan_gate();
    game.click("gate.approve —— ");
    game.narrate("requirement.dispatch", "0");
    game.assert_kv("req.1.status", json!("developing"));
    game.assert_kv("req.1.gate", json!(""));
    submit(&mut game, 1, "development", ext);
    game
}

// ───────────────────────── ⑦ 全 1 走查（两侧 Mod）─────────────────────────

/// ⑦ 两侧 Mod 的全 1 走查都必须到【结局，且 `warnings` / `errors` 为空。
///
/// 这是**补充**而不是替代：全 1 只覆盖 happy path，语义分叉由 ①–⑥ 覆盖。
#[test]
fn playtest_reaches_ending_on_both_mods() {
    for dir in [project_mod_dir(), seed_mod_dir()] {
        let report = playtest(&dir, &PlaytestOptions::default());
        assert!(
            report.ok,
            "{} 全 1 走查失败：reached={:?} errors={:?}",
            dir.display(),
            report.reached,
            report.errors
        );
        assert_eq!(
            report.reached,
            Some(Reached::Ending),
            "{} 应走到【结局",
            dir.display()
        );
        assert!(
            report.warnings.is_empty(),
            "{} 不应有警告：{:?}",
            dir.display(),
            report.warnings
        );
        assert!(
            report.errors.is_empty(),
            "{} 不应有错误：{:?}",
            dir.display(),
            report.errors
        );
    }
}

// ───────────────────────── ① accept 门禁正常打回 ─────────────────────────

/// ① `accept` 门禁**未 amend** 打回：`plan_ok` 沿用 true → 存储态回 planning，
/// 展示名仍是 `ready`，下一次派活派的是 **development**（不是 plan）。
#[test]
fn accept_gate_reject_keeps_plan_ok_and_redispatches_development() {
    let mut game = drive_to_accept(drive_to_plan_gate());
    assert_eq!(game.phase("r1"), "accept");
    game.assert_kv("req.1.status", json!("developing"));
    game.assert_kv("req.1.gate_rev", json!(1));

    // guide 门禁：通过类动作没有菜单票据时一律拒绝，且主状态不动（§8/§9）。
    let direct = game.narrate("gate.approve", &text("r1"));
    assert_refused(&direct, "gate.approve");
    game.assert_kv("req.1.status", json!("developing"));
    game.assert_kv("req.1.gate", json!("accept"));

    // 正常打回（打回类不在闸内，直派/点菜单等价；这里仍走菜单，保持单一入口）。
    game.click("gate.reject —— ");
    game.assert_kv("req.1.status", json!("planning"));
    game.assert_kv("req.1.gate", json!(""));
    game.assert_kv("req.1.plan_ok", json!(true));
    assert_eq!(game.phase("r1"), "ready");

    game.narrate("requirement.dispatch", "0");
    game.assert_kv("req.1.status", json!("developing"));
    game.assert_kv("req.1.job", json!("development"));
}

// ───────────────────────── ② accept 门禁上 amend → stale ─────────────────────────

/// ② `accept` 门禁待批期间 amend：`rev+1` 且 `plan_ok` 作废 → 旧批准判 stale
/// （拒绝且状态不变）→ 打回停在 planning（不重派 plan 之外的事）→ 派活回 plan。
#[test]
fn accept_gate_amend_invalidates_plan_and_stales_the_pending_gate() {
    let mut game = drive_to_accept(drive_to_plan_gate());
    assert_eq!(game.phase("r1"), "accept");

    game.narrate("requirement.amend", &text("r1|改过的标题"));
    game.assert_kv("req.1.rev", json!(2));
    game.assert_kv("req.1.plan_ok", json!(false));
    // 门禁本身没被摘（打回/批准才摘），但绑定的是旧 rev。
    game.assert_kv("req.1.gate", json!("accept"));
    game.assert_kv("req.1.gate_rev", json!(1));

    // 旧批准不得生效：stale 拒绝，状态与门禁原样。
    let stale = game.click("gate.approve —— ");
    let stale_lines: Vec<String> = stale["narrative"]
        .as_array()
        .expect("narrative")
        .iter()
        .map(|line| line.as_str().expect("叙事行").to_owned())
        .collect();
    assert_refused(&stale_lines, "stale");
    game.assert_kv("req.1.status", json!("developing"));
    game.assert_kv("req.1.gate", json!("accept"));
    game.assert_kv("req.1.gate_rev", json!(1));

    game.click("gate.reject —— ");
    game.assert_kv("req.1.status", json!("planning"));
    game.assert_kv("req.1.gate", json!(""));
    game.assert_kv("req.1.plan_ok", json!(false));
    assert_eq!(game.phase("r1"), "planning");

    game.narrate("requirement.dispatch", "0");
    game.assert_kv("req.1.job", json!("plan"));
}

// ───────────────────────── ③ plan 门禁打回 ─────────────────────────

/// ③ `plan` 门禁打回：`plan_ok` 置假 → planning → 下一次派活仍是 **plan**。
#[test]
fn plan_gate_reject_requires_a_new_plan() {
    let mut game = drive_to_plan_gate();
    assert_eq!(game.phase("r1"), "plan_gate");

    game.click("gate.reject —— ");
    game.assert_kv("req.1.status", json!("planning"));
    game.assert_kv("req.1.gate", json!(""));
    game.assert_kv("req.1.plan_ok", json!(false));
    assert_eq!(game.phase("r1"), "planning");

    game.narrate("requirement.dispatch", "0");
    game.assert_kv("req.1.status", json!("planning"));
    game.assert_kv("req.1.job", json!("plan"));
}

// ───────────────────────── ④ dev 提案打回 ─────────────────────────

/// ④ 开发阶段 `proposal.reject`（**未到 accept 门禁**）：
/// - 未 amend：`plan_ok` 原值保留 → 回 planning 但展示 `ready`，再派活是 development；
/// - amend 过：`plan_ok` 已被 rev 作废 → 停在 planning，再派活是 plan。
#[test]
fn dev_proposal_reject_keeps_plan_ok_unless_amended() {
    // 未 amend 的路径。
    let mut normal = drive_to_dev_pending("semantics-p4");
    normal.narrate("proposal.reject", &text("r1|产物不全"));
    normal.assert_kv("req.1.status", json!("planning"));
    normal.assert_kv("req.1.plan_ok", json!(true));
    assert_eq!(normal.phase("r1"), "ready");
    normal.narrate("requirement.dispatch", "0");
    normal.assert_kv("req.1.status", json!("developing"));
    normal.assert_kv("req.1.job", json!("development"));

    // amend 过的路径：提案先挂着，amend 把 rev 作废（plan_ok=false），再打回。
    let mut amended = drive_to_dev_pending("semantics-p5");
    amended.narrate("requirement.amend", &text("r1|补一条验收要求"));
    amended.assert_kv("req.1.rev", json!(2));
    amended.assert_kv("req.1.plan_ok", json!(false));
    amended.narrate("proposal.reject", &text("r1|按新 rev 重做"));
    amended.assert_kv("req.1.status", json!("planning"));
    amended.assert_kv("req.1.plan_ok", json!(false));
    assert_eq!(amended.phase("r1"), "planning");
    amended.narrate("requirement.dispatch", "0");
    amended.assert_kv("req.1.job", json!("plan"));
}

// ───────────────────────── ⑤ legacy 缺键存档 ─────────────────────────

/// ⑤ legacy 兼容：本模型之前的在途行**没有** `req.N.plan_ok` 这个键。
///
/// 用内存 save fixture 造出这种存档（`save` 回执里删键），再走 `restore` 读档：
/// - 缺键时展示与判定都不得把在途的 developing 行当成「计划未批准」；
/// - `gate.reject` 打回时按**当前状态**（developing）推断为已过 plan 门禁 → 展示 `ready`，
///   并把推断结果**显式写回**（此后不再依赖推断）。
///
/// 只证「当前 reducer 兼容缺键」：不伪造历史引擎、不读写任何 autosave / events 文件。
#[test]
fn legacy_save_without_plan_flag_infers_then_writes_it_back() {
    let mut game = drive_to_accept(drive_to_plan_gate());
    let mut save = game.save();
    let entries = save["kv"]["entries"]
        .as_object_mut()
        .expect("kv.entries 必须是对象");
    assert_eq!(
        entries.remove("req.1.plan_ok"),
        Some(json!(true)),
        "样例起点应当带显式 plan_ok=true"
    );
    assert!(!entries.contains_key("req.1.plan_ok"));

    let bytes = serde_json::to_vec(&save).expect("存档序列化");
    let mut legacy = Game {
        bundle: game.bundle.clone(),
        kernel: game.bundle.restore(&bytes).expect("缺键存档必须可装载"),
    };
    // 读档后键仍缺席（reducer 不凭空补写），但展示不受影响。
    legacy.assert_kv("req.1.plan_ok", Json::Null);
    assert_eq!(legacy.phase("r1"), "accept");

    legacy.click("gate.reject —— ");
    legacy.assert_kv("req.1.status", json!("planning"));
    legacy.assert_kv("req.1.plan_ok", json!(true)); // 推断结果已显式落库
    assert_eq!(legacy.phase("r1"), "ready");

    legacy.narrate("requirement.dispatch", "0");
    legacy.assert_kv("req.1.status", json!("developing"));
    legacy.assert_kv("req.1.job", json!("development"));
}

// ───────────────────────── ⑥ 双层节点图两套契约 ─────────────────────────

/// 取 `map.show` 回执里的**图窗口**：从图头行到图脚行（不含同一条回执里的其他渲染行）。
#[track_caller]
fn map_window(lines: &[String], head: &str, foot: &str) -> Vec<String> {
    let start = lines
        .iter()
        .position(|line| line.starts_with(head))
        .unwrap_or_else(|| panic!("缺图头 `{head}`：{lines:#?}"));
    let end = lines
        .iter()
        .position(|line| line.starts_with(foot))
        .unwrap_or_else(|| panic!("缺图脚 `{foot}`：{lines:#?}"));
    assert!(start < end, "图头必须早于图脚：{lines:#?}");
    lines[start..=end].to_vec()
}

/// 把一行图拆成「左列（前件/当前节点）｜动作区」与「右列（目标/结果）」。
#[track_caller]
fn split_row(row: &str) -> (&str, &str) {
    row.split_once("──▶ ")
        .unwrap_or_else(|| panic!("图行缺 `──▶` 分隔：{row}"))
}

/// 左列里的方括号标签（`[planning]` / `[draft/analyzing/planning]`）。
#[track_caller]
fn row_label(left: &str) -> &str {
    let open = left
        .find('[')
        .unwrap_or_else(|| panic!("图行左列缺标签：{left}"));
    let close = left[open..]
        .find(']')
        .unwrap_or_else(|| panic!("图行左列标签未闭合：{left}"))
        + open;
    &left[open..=close]
}

/// 操作视图（`map.show`，默认）：只承诺「可选择」，条件缺失时明说「条件未展示」。
///
/// 反例是上一轮的两处措辞退化：把「可以选择」写成「可执行」（渲染层看不见 choice 侧
/// 的真实前置）、把「条件未展示」写成「无额外条件」（替候选构造担保）。
#[track_caller]
fn assert_map_local(lines: &[String]) {
    let window = map_window(lines, "【当前节点】", "（完整设计图");
    let joined = window.join("\n");
    assert!(
        joined.contains("可选择"),
        "操作视图每条出边都必须标「可选择」：\n{joined}"
    );
    assert!(
        joined.contains("条件未展示"),
        "缺 `when` 时必须说「条件未展示」：\n{joined}"
    );
    assert!(
        !joined.contains("可执行"),
        "操作视图不得承诺「可执行」（渲染层没有 choice 侧前置）：\n{joined}"
    );
    assert!(
        !joined.contains("无额外条件"),
        "操作视图不得替候选构造担保「无额外条件」：\n{joined}"
    );
}

/// 设计视图（`map.show full`）：
/// - 需求级流水线的 `planning` / `developing` 两跳都必须显式带 `proposal.accept`
///   （旧版缺这一跳，读起来像「提案直接进门禁」）；
/// - 旁路左列是**可变前件**、右列才是**结果终态**（旧版左右是反的）。
#[track_caller]
fn assert_map_full(lines: &[String]) {
    let window = map_window(lines, "【双层节点图】", "常驻动作");

    // 两跳：中间那跳 proposal.accept 不得丢。
    let hop = "proposal → proposal.accept → gate.approve";
    let hop_rows: Vec<&String> = window.iter().filter(|row| row.contains(hop)).collect();
    assert_eq!(
        hop_rows.len(),
        2,
        "流水线必须有 planning / developing 两跳带 proposal.accept：{window:#?}"
    );
    let planning = hop_rows
        .iter()
        .find(|row| row_label(split_row(row).0) == "[planning]")
        .unwrap_or_else(|| panic!("缺 `[planning]` 那一跳：{window:#?}"));
    assert!(
        split_row(planning)
            .1
            .trim_start()
            .starts_with("[planning·plan_ok]"),
        "`[planning]` 的下一跳必须是 `[planning·plan_ok]`（批了才能 develop）：{planning}"
    );
    let developing = hop_rows
        .iter()
        .find(|row| row_label(split_row(row).0) == "[developing]")
        .unwrap_or_else(|| panic!("缺 `[developing]` 那一跳：{window:#?}"));
    assert!(
        split_row(developing).1.trim_start().starts_with("[done]"),
        "`[developing]` 的下一跳必须是 `[done]`：{developing}"
    );

    // 旁路方向：前件 → 结果终态（不是「终态还能再拆一次」）。
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
        "旁路必须恰好三条：{window:#?}"
    );
    for row in bypass_rows {
        let (left, right) = split_row(row);
        let label = row_label(left);
        assert!(
            PRECONDITIONS.iter().any(|state| label.contains(state)),
            "旁路左列必须点名前件（不是状态终态）：{row}"
        );
        for terminal in TERMINALS {
            assert!(
                !label.contains(terminal),
                "旁路左列不得是终态 `{terminal}`（方向反了）：{row}"
            );
        }
        let target = right.trim_start();
        assert!(
            TERMINAL_TARGETS.iter().any(|end| target.starts_with(end)),
            "旁路右列必须是结果终态：{row}"
        );
    }
}

/// ⑥ 两套图层契约（两侧 Mod 各跑一遍：`map.show` 的文案属 scenes 资源，会独立漂移）。
#[test]
fn map_views_keep_operation_and_design_contracts() {
    for dir in [project_mod_dir(), seed_mod_dir()] {
        let mut game = Game::open(&dir);
        assert_map_local(&game.narrate("map.show", "0"));

        // planning 处境的设计视图。
        let mut game = drive_to_plan_gate_in(&dir);
        assert_eq!(game.phase("r1"), "plan_gate");
        assert_map_full(&game.narrate("map.show", &text("full")));

        // developing 处境的设计视图（同一局继续走，两跳都在同一张图上）。
        game.click("gate.approve —— ");
        game.narrate("requirement.dispatch", "0");
        assert_eq!(game.phase("r1"), "developing");
        assert_map_full(&game.narrate("map.show", &text("full")));
    }
}
