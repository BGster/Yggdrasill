//! M0 创作面验证：《灯下谜径》无头测试冒险全链路（packages/mods/test-adventure）。
//!
//! 本套件是「手写 Mod 全通关」证据：真实资源包（manifest + schema + data，
//! sha256/byteLength 寻址）+ 手写 Lua Mod，经 [`KernelFacade::with_resources`]
//! 完整装载后按玩家事件序推进到三个结局。验证面：
//!
//! - **三分支全通关**：branch_a（带灯下地窖→秘藏）/ branch_b（回客房→平安）/
//!   branch_c（无灯下地窖→摔伤折返），每分支断言叙事节拍 + kv ending 旗 +
//!   章节状态机终态；
//! - **存读档续玩**：中盘 save → `restore_with_resources` 重建 → 续玩到结局，
//!   结局旗与不间断跑法一致；
//! - **重放确定性**：同分支双 facade，逐 dispatch reply 字节一致 + 最终
//!   canonical 状态字节一致；
//! - **资源接线**：叙述行直接取自 ygg.data 资源内容（作者无法靠改 Mod 文本
//!   伪造——文案只在 data/scenes.json 中）。
//!
//! 玩家路径备忘（对应 mod.lua 头注）：
//!   选择1 @酒馆：1=接提灯 / 2=轻装出发；选择2 @岔路：1=下地窖 / 2=回客房。

use serde_json::{json, Value as Json};
use ygg_sandbox::driver::KernelFacade;

const MOD_SOURCE: &str = include_str!("../../../packages/mods/test-adventure/mod.lua");
const MANIFEST_BYTES: &[u8] = include_bytes!("../../../packages/mods/test-adventure/manifest.json");
const SCENE_SCHEMA: &str =
    include_str!("../../../packages/mods/test-adventure/schemas/scene-texts.json");
const SCENE_DATA: &[u8] = include_bytes!("../../../packages/mods/test-adventure/data/scenes.json");

const SCENE_SCHEMA_ID: &str = "https://yggdrasill.dev/schemas/v1/scene-texts.json";
const SCENE_DATA_PATH: &str = "data/scenes.json";

/// 资源包三件套（schemas / resources 形参与 `with_resources` 签名一致）。
fn schemas() -> Vec<(String, String)> {
    vec![(SCENE_SCHEMA_ID.to_owned(), SCENE_SCHEMA.to_owned())]
}

fn resources() -> Vec<(String, Vec<u8>)> {
    vec![(SCENE_DATA_PATH.to_owned(), SCENE_DATA.to_vec())]
}

/// 全量装载（失败即 panic，文案带内核错误详情）。
fn facade() -> KernelFacade {
    KernelFacade::with_resources(MOD_SOURCE, MANIFEST_BYTES, &schemas(), &resources())
        .expect("kernel with test-adventure resource package")
}

/// `restore_with_resources` 同形封装（存读档续玩路径）。
fn restore(save_bytes: &[u8]) -> KernelFacade {
    KernelFacade::restore_with_resources(
        save_bytes,
        MOD_SOURCE,
        MANIFEST_BYTES,
        &schemas(),
        &resources(),
    )
    .expect("restore with test-adventure resource package")
}

fn parse(reply: &str) -> Json {
    serde_json::from_str(reply).expect("reply must be valid JSON")
}

fn reply_ok(kernel: &mut KernelFacade, cmd: &str, payload: &str) -> Json {
    let parsed = parse(&kernel.dispatch(cmd, payload));
    assert_eq!(parsed["ok"], json!(true), "dispatch {cmd} failed: {parsed}");
    parsed
}

/// 信封 narrative 数组 → String 列表。
fn narrative_of(parsed: &Json) -> Vec<String> {
    parsed["narrative"]
        .as_array()
        .expect("narrative array")
        .iter()
        .map(|v| v.as_str().expect("narrative line").to_owned())
        .collect()
}

/// 全量存档对象（含实体/KV/pending_choice）——kv / 实体断言的取数面。
fn save_of(kernel: &mut KernelFacade) -> Json {
    reply_ok(kernel, "save", "")["save"].clone()
}

/// 玩家一步：出发（序章→酒馆，挂起选择1）。
fn start(kernel: &mut KernelFacade) -> Json {
    let parsed = reply_ok(kernel, "event.start", "0");
    assert_eq!(
        parsed["pendingChoice"]["prompt"],
        json!("掌柜把铜提灯推向你：『地窖黑得很，带上它吧？』"),
        "选择1 挂起于酒馆"
    );
    parsed
}

/// 完整玩家路径：出发 → 两次选择（choice1/choice2 ∈ {1,2}）→ 结局。
/// 返回最后一次 dispatch 的信封（结局叙述行所在）。
fn playthrough(kernel: &mut KernelFacade, choice1: i64, choice2: i64) -> Vec<Json> {
    let mut replies = Vec::new();
    replies.push(start(kernel));
    replies.push(reply_ok(
        kernel,
        "ygg.choice_selected",
        &choice1.to_string(),
    ));
    let parsed = reply_ok(kernel, "ygg.choice_selected", &choice2.to_string());
    assert!(
        parsed["pendingChoice"].is_null(),
        "两个选择均已回应，转换收口无待决项: {parsed}"
    );
    replies.push(parsed);
    replies
}

fn ending_of(save: &Json) -> String {
    save["kv"]["entries"]["ending"]
        .as_str()
        .expect("ending kv flag")
        .to_owned()
}

/// 结局叙述行（【结局X】前缀标识）在当次转换叙事里出现。
fn assert_ending_line(parsed: &Json, tag: &str) {
    let lines = narrative_of(parsed);
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with(&format!("【结局{tag}】"))),
        "结局{tag} 标识行必须在叙事输出中: {lines:?}"
    );
}

// ----------------------------------------------------------------------------
// 三分支全通关
// ----------------------------------------------------------------------------

#[test]
fn full_playthrough_branch_a_lantern_secret() {
    // 带灯（1）→ 下地窖（1）：提灯标签开门 → 结局A。
    let mut kernel = facade();
    let replies = playthrough(&mut kernel, 1, 1);
    let parsed = &replies[2];
    assert_ending_line(parsed, "A");
    // 叙事节拍：提灯节拍 + 灯下地窖场景（地窖亮面文本只存在于资源包）。
    let lines: Vec<String> = replies[1..=2].iter().flat_map(narrative_of).collect();
    assert!(
        lines.iter().any(|l| l.contains("你接过提灯")),
        "带灯节拍缺失: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("石阶尽头立着一扇刻满星图的暗门")),
        "灯下地窖场景（资源文本）缺失: {lines:?}"
    );
    // 状态面：结局旗 + 章节状态机落 cellar。
    let save = save_of(&mut kernel);
    assert_eq!(ending_of(&save), "A");
    assert_eq!(
        save["kv"]["entries"]["fsm.chapter"],
        json!("cellar"),
        "主线机落 cellar"
    );
    // inn_route 机在本分支未收到事件：fsm 仅在实际转移时写 KV（initial 不落盘），
    // 故键缺席 = 未触发（null 即证据）。
    assert!(
        save["kv"]["entries"]["fsm.inn_route"].is_null(),
        "客房支线机未触发时键不得落盘"
    );
    assert_eq!(save["kv"]["entries"]["took_lantern"], json!(true));
}

#[test]
fn full_playthrough_branch_b_inn_peaceful() {
    // 带灯（1）→ 回客房（2）：章节落 inn → 结局B。
    let mut kernel = facade();
    let replies = playthrough(&mut kernel, 1, 2);
    assert_ending_line(&replies[2], "B");
    let lines: Vec<String> = replies[1..=2].iter().flat_map(narrative_of).collect();
    assert!(
        lines.iter().any(|l| l.contains("你推开客房的门")),
        "客房场景（资源文本）缺失: {lines:?}"
    );
    let save = save_of(&mut kernel);
    assert_eq!(ending_of(&save), "B");
    assert_eq!(
        save["kv"]["entries"]["fsm.inn_route"],
        json!("inn"),
        "客房支线机落 inn"
    );
    assert_eq!(
        save["kv"]["entries"]["fsm.chapter"],
        json!("road"),
        "主线机停在 road（cellar 出边未触发）"
    );
}

#[test]
fn full_playthrough_branch_c_dark_cellar_fall() {
    // 轻装（2）→ 下地窖（1）：无 item:lantern 标签 → 摔伤 → 结局C。
    let mut kernel = facade();
    let replies = playthrough(&mut kernel, 2, 1);
    assert_ending_line(&replies[2], "C");
    let lines: Vec<String> = replies[1..=2].iter().flat_map(narrative_of).collect();
    assert!(
        lines.iter().any(|l| l.contains("你谢绝了提灯")),
        "无灯节拍缺失: {lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.contains("你摸黑走下石阶")),
        "黑暗地窖场景（资源文本）缺失: {lines:?}"
    );
    let save = save_of(&mut kernel);
    assert_eq!(ending_of(&save), "C");
    assert_eq!(
        save["kv"]["entries"]["fsm.chapter"],
        json!("cellar"),
        "主线机落 cellar（无灯线）"
    );
    assert_eq!(save["kv"]["entries"]["took_lantern"], json!(false));
    // 道具判定面证据：提灯实体无 item:lantern 标签。
    let lantern_id = save["kv"]["entries"]["lantern.id"]
        .as_i64()
        .expect("lantern id");
    let lantern = &save["entities"]["entities"][lantern_id.to_string()];
    let tags: Vec<&str> = lantern["tags"]
        .as_array()
        .expect("tags")
        .iter()
        .map(|t| t.as_str().expect("tag"))
        .collect();
    assert!(
        !tags.contains(&"item:lantern"),
        "无灯路径不得持有 item:lantern: {tags:?}"
    );
    // 数值调参面：结局C 消费资源 dark_fall_damage=3 扣 hp（10 → 7）。
    let hero_id = save["kv"]["entries"]["hero.id"].as_i64().expect("hero id");
    let hero = &save["entities"]["entities"][hero_id.to_string()];
    assert_eq!(
        hero["attributes"]["hp"],
        json!(7),
        "黑暗跌落扣血须取自资源数值"
    );
    assert!(lines.iter().any(|l| l.contains("（主角 hp 降至 7）")));
}

// ----------------------------------------------------------------------------
// 存读档续玩：中盘 save → restore_with_resources → 续到同一结局
// ----------------------------------------------------------------------------

#[test]
fn save_midway_and_continue_reaches_same_ending() {
    // ① 不间断基准：带灯 + 回客房 = 结局B。
    let mut baseline = facade();
    let _ = playthrough(&mut baseline, 1, 2);
    let baseline_save = save_of(&mut baseline);
    assert_eq!(ending_of(&baseline_save), "B");

    // ② 中盘存档：选择1 回应后、选择2 之前（此刻挂起的是岔路选择）。
    let mut kernel = facade();
    let _ = start(&mut kernel);
    let _ = reply_ok(&mut kernel, "ygg.choice_selected", "1");
    let parsed = reply_ok(&mut kernel, "save", "");
    assert_eq!(
        parsed["save"]["pending_choice"]["prompt"],
        json!("后廊岔路——你的脚步停在哪里？"),
        "存档须携带待决选择（choice-as-state）"
    );
    let save_bytes = serde_json::to_vec(&parsed["save"]).unwrap();

    // ③ 读档重建 + 续玩到结局：结局旗与不间断跑法一致。
    let mut restored = restore(&save_bytes);
    let parsed = reply_ok(&mut restored, "ygg.choice_selected", "2");
    assert_ending_line(&parsed, "B");
    let restored_save = save_of(&mut restored);
    assert_eq!(ending_of(&restored_save), ending_of(&baseline_save));
    for key in ["fsm.chapter", "fsm.inn_route", "took_lantern", "stage"] {
        assert_eq!(
            restored_save["kv"]["entries"][key], baseline_save["kv"]["entries"][key],
            "kv {key}（含状态机持久化态）读档后续接一致"
        );
    }
}

// ----------------------------------------------------------------------------
// 重放确定性：同分支双 facade，reply 字节 + 最终状态字节全同
// ----------------------------------------------------------------------------

#[test]
fn replay_determinism_identical_replies_and_final_state() {
    let script: [(&str, &str); 3] = [
        ("event.start", "0"),
        ("ygg.choice_selected", "2"), // 轻装出发
        ("ygg.choice_selected", "1"), // 下地窖 → 结局C
    ];
    let mut a = facade();
    let mut b = facade();
    for (cmd, payload) in script {
        let (ra, rb) = (a.dispatch(cmd, payload), b.dispatch(cmd, payload));
        assert_eq!(ra, rb, "同分支双 facade 的 reply 必须逐字节一致 ({cmd})");
    }
    assert_eq!(
        a.canonical_state_bytes().unwrap(),
        b.canonical_state_bytes().unwrap(),
        "同事件序必须到达逐字节相同的 canonical 状态"
    );
    // 结局旗一致（C），存档 kv 面全同。
    assert_eq!(ending_of(&save_of(&mut a)), "C");
    assert_eq!(
        save_of(&mut a)["kv"],
        save_of(&mut b)["kv"],
        "最终 KV 面必须一致"
    );
}

// ----------------------------------------------------------------------------
// 资源接线：资源内容经 ygg.data 直达叙述行
// ----------------------------------------------------------------------------

#[test]
fn resources_wired_narrative_sourced_from_data() {
    // 序章文本只在 data/scenes.json 中（mod.lua 无任何场景文案字面量）——
    // 该行出现在叙事输出即证明 manifest/schema/寻址/校验/ygg.data 全链接通。
    let mut kernel = facade();
    let parsed = start(&mut kernel);
    let lines = narrative_of(&parsed);
    assert!(
        lines.iter().any(|l| l.contains("「灯下谜径」客栈到了")),
        "序章行必须逐字来自资源 scenes.prologue: {lines:?}"
    );
    // 选项文案同样来自资源：待决选择的 options 与资源 options 数组一致。
    assert_eq!(
        parsed["pendingChoice"]["options"],
        json!(["接过提灯", "轻装出发"]),
        "choice options 必须取自资源 options.tavern_*"
    );
    // 驱动侧资源表确已装载（非空基线）。
    assert!(!kernel.resources().is_empty(), "资源表必须非空");
}
