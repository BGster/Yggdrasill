//! S10 KernelFacade 线格式/信封集成测试（32 测）。
//!
//! 验证面：JSON 信封形状（A1 线格式）、choice 终结与 respond（B2
//! choice-as-state）、稳定错误码映射、存读档往返与统一 store 代决证据、
//! 外部事件日志（B1 回放前提）、确定性 canonical 字节、stdlib（narrative /
//! fsm）经 facade 全链路、load 回包携带 pendingChoice（A1）、
//! choice_selected 序号域内校验（A2）、fsm when 条件边（B1）与 initial
//! 落盘可见化（B2）、**dispatch 原子性（r1 savepoint：失败步整体回滚
//! entities + kv 与两个 revision，成功步提交；域校验门早退不触发回滚；
//! 旁路面 `log` 照记失败步）**。
//!
//! 断言口径备忘：KV 不在状态哈希的覆盖范围内（`src/hash.rs` 覆盖面声明），
//! 所以「失败步没改状态」的断言必须读 `save` 回执（`save.kv.entries`）或
//! 比较 save 字节，只看 `state_hash` 是不够的。
//!
//! 保留命令名约定（驱动面）：Mod 事件不得用 `save / load / state / log /
//! drain` 五个名字；`ygg.choice_selected` 为玩家选择的 respond 事件
//! （B2：选择 = 新外部事件），不受保留名限制。

use serde_json::{json, Value as Json};
use ygg_sandbox::driver::{KernelError, KernelFacade, DEFAULT_FUEL_BUDGET};
use ygg_sandbox::sandbox::ENGINE_VERSION;

/// 玩家选择门 Mod：入口事件挂起 choice；respond 事件按序号叙述选择结果。
const GATE_MOD: &str = r#"
ygg.on("event.start", function()
  ygg.choice("继续前行？", { "go", "stay" })
end)
ygg.on("ygg.choice_selected", function(index)
  ygg.narrate("picked=" .. tostring(index))
end)
"#;

/// 状态写入 Mod：装载期生成实体（hp=10）；hit 事件扣血并计数 KV——
/// handler 即席写经 host 桥直接落 canonical store（统一 store 代决）。
const STATE_MOD: &str = r#"
local id = ygg.entity.spawn()
ygg.entity.set_attribute(id, "hp", 10)
ygg.on("event.hit", function()
  local hp = ygg.entity.get_attribute(id, "hp")
  ygg.entity.set_attribute(id, "hp", hp - 1)
  ygg.kv.set("hits", (ygg.kv.get("hits") or 0) + 1)
end)
"#;

/// 死循环 Mod：VM 指令计量面耗尽（燃料杀循环负例经 facade 信封）。
const LOOP_MOD: &str = r#"
ygg.on("event.loop", function()
  while true do end
end)
"#;

/// r13 自动扩缩击杀回归 Mod：fill 事件灌 1000 条 KV（推高自动扩缩预算到
/// 2M）；burn 事件跑固定迭代数计数循环（约几十万指令——地板 100k 杀不死、
/// 2M 扩缩预算放行）；loop 事件是纯死循环（任何有限预算下都被杀）。
const AUTOSCALE_MOD: &str = r#"
ygg.on("event.fill", function()
  for i = 1, 1000 do
    ygg.kv.set("probe." .. tostring(i), i)
  end
end)
ygg.on("event.burn", function()
  local acc = 0
  for i = 1, 150000 do
    acc = acc + i
  end
  ygg.kv.set("burned", acc)
end)
ygg.on("event.loop", function()
  while true do end
end)
"#;

/// 沙箱拒绝 Mod：handler 内 require（白名单拒绝桩，可读 "sandbox denied"）。
const DENIED_MOD: &str = r#"
ygg.on("event.denied", function()
  require("io")
end)
"#;

/// 事件日志 Mod：handler 发派生事件——日志面只记外部事件。
const LOG_MOD: &str = r#"
ygg.on("event.log", function()
  ygg.emit("evt.derived", 1)
  ygg.narrate("logged")
end)
"#;

/// 叙事流 Mod：node 声明 + goto 路由（stdlib narrative 全链路）。
const NARRATIVE_MOD: &str = r#"
ygg.narrative.node("node:intro", function()
  ygg.narrate("node:intro")
  ygg.narrative["goto"]("node:scene")
end)
ygg.narrative.node("node:scene", function()
  ygg.narrate("node:scene")
end)
ygg.on("event.enter", function()
  ygg.narrative["goto"]("node:intro")
end)
"#;

/// 状态机 Mod：门状态机（stdlib fsm 全链路；状态持久化在 KV）。
const FSM_MOD: &str = r#"
ygg.fsm.define{
  name = "door",
  initial = "closed",
  event = "event.toggle",
  transitions = {
    { from = "closed", to = "open" },
    { from = "open", to = "closed" },
  },
}
"#;

/// 条件边状态机 Mod（B1）：同 from 双出边，声明序首个 when 成立者胜出。
const FSM_WHEN_MOD: &str = r#"
ygg.fsm.define{
  name = "router",
  initial = "road",
  event = "event.go",
  transitions = {
    { from = "road", to = "cellar", when = function(p) return p == 1 end },
    { from = "road", to = "inn" },
  },
}
"#;

/// 报错定位 Mod（C3）：handler 内显式 error——验证 chunk 命名 `@mod.lua`
/// 进入错误文本（mod 侧定位面）。
const BOOM_MOD: &str = r#"
ygg.on("event.boom", function()
  error("boom")
end)
"#;

/// D1 反例 Mod：handler 先 emit 再 choice——派生事件被语义丢弃，
/// 信封应携带 warnings 警告（含事件名与丢弃条数）。
const CHOICE_DROPS_MOD: &str = r#"
ygg.on("event.go", function()
  ygg.emit("chapter.advance", "to_road")
  ygg.emit("chapter.advance", "to_cellar")
  ygg.choice("去哪儿？", { "地窖", "客房" })
end)
"#;

/// D2 取证 Mod：host 回调拒绝（kv.set 非有限值）——mlua CallbackError
/// 链应携带回调点捕获的 traceback（mod.lua 调用点帧）。
const KV_NAN_MOD: &str = r#"
ygg.on("event.bad", function()
  ygg.kv.set("x", 0/0)
end)
"#;

fn parse(reply: &str) -> Json {
    serde_json::from_str(reply).expect("reply must be valid JSON")
}

fn reply_ok(reply: &str) -> Json {
    let parsed = parse(reply);
    assert_eq!(parsed["ok"], json!(true), "expected ok reply: {reply}");
    parsed
}

fn reply_error_code(reply: &str) -> String {
    let parsed = parse(reply);
    assert_eq!(parsed["ok"], json!(false), "expected error reply: {reply}");
    parsed["error"]["code"]
        .as_str()
        .expect("error code")
        .to_owned()
}

#[test]
fn envelope_ok_shape_for_entry_event() {
    let mut kernel = KernelFacade::new(GATE_MOD).expect("kernel construction");
    let reply = kernel.dispatch("event.start", "0");
    let parsed = reply_ok(&reply);
    // A1 线格式字段面：恰好八个键，无 stateHash（S12 才回填哈希面）。
    let mut keys: Vec<&str> = parsed
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "event",
            "eventsDerived",
            "eventsDropped",
            "eventsProcessed",
            "narrative",
            "ok",
            "pendingChoice",
            "revision"
        ]
    );
    assert!(parsed.get("stateHash").is_none(), "stateHash lands in S12");
    assert_eq!(parsed["event"], json!("event.start"));
    assert_eq!(parsed["narrative"], json!([]));
    assert_eq!(parsed["eventsProcessed"], json!(1));
    assert_eq!(parsed["eventsDerived"], json!(0));
    assert_eq!(parsed["eventsDropped"], json!(0));
    assert!(
        parsed["pendingChoice"].is_object(),
        "choice pending: {parsed}"
    );
    assert_eq!(parsed["pendingChoice"]["prompt"], json!("继续前行？"));
    assert_eq!(parsed["pendingChoice"]["options"], json!(["go", "stay"]));
}

#[test]
fn choice_terminates_and_respond_is_new_external_event() {
    let mut kernel = KernelFacade::new(GATE_MOD).expect("kernel construction");
    // 首事件：choice 终结转换（first-wins；FIFO 余量计 dropped）。
    let parsed = reply_ok(&kernel.dispatch("event.start", "0"));
    assert!(parsed["pendingChoice"].is_object());
    // respond = 新外部事件（B2 choice-as-state；payload 2 → picked=2）。
    let parsed = reply_ok(&kernel.dispatch("ygg.choice_selected", "2"));
    assert_eq!(parsed["narrative"], json!(["picked=2"]));
    assert!(parsed["pendingChoice"].is_null(), "respond clears pending");
}

#[test]
fn load_command_reply_carries_pending_choice() {
    // A1：load 成功回包携带 pendingChoice（同 state 形态）——读档方零额外
    // dispatch 即可恢复提示面。挂起选择随存档往返（B3）的直接证据。
    let mut source = KernelFacade::new(GATE_MOD).expect("kernel construction");
    let _ = reply_ok(&source.dispatch("event.start", "0"));
    let save_reply = reply_ok(&source.dispatch("save", ""));
    let save_str = save_reply["save"].to_string();
    let mut target = KernelFacade::new(GATE_MOD).expect("kernel construction");
    let parsed = reply_ok(&target.dispatch("load", &save_str));
    assert_eq!(parsed["restored"], json!(true));
    assert_eq!(parsed["pendingChoice"]["prompt"], json!("继续前行？"));
    assert_eq!(parsed["pendingChoice"]["options"], json!(["go", "stay"]));
    // respond 在 load 恢复态上照常工作（选择 = 新外部事件）。
    let parsed = reply_ok(&target.dispatch("ygg.choice_selected", "1"));
    assert_eq!(parsed["narrative"], json!(["picked=1"]));
    // 无待决选择的存档：load 回包 pendingChoice 为 null（与 state 同形态）。
    let mut clean = KernelFacade::new(GATE_MOD).expect("kernel construction");
    let clean_save = reply_ok(&clean.dispatch("save", ""));
    let parsed = reply_ok(&clean.dispatch("load", &clean_save["save"].to_string()));
    assert!(parsed["pendingChoice"].is_null());
}

#[test]
fn choice_selected_out_of_range_is_rejected_readably() {
    // A2：待决选择存在时序号越界 → 可读 VALIDATION（不进内核转换）。
    let mut kernel = KernelFacade::new(GATE_MOD).expect("kernel construction");
    let _ = reply_ok(&kernel.dispatch("event.start", "0"));
    let reply = kernel.dispatch("ygg.choice_selected", "9");
    let parsed: Json = parse(&reply);
    assert_eq!(parsed["ok"], json!(false), "reply: {reply}");
    assert_eq!(parsed["error"]["code"], json!("VALIDATION"));
    let message = parsed["error"]["message"].as_str().expect("message");
    assert!(message.contains("选项不存在"), "readable message: {reply}");
    assert!(message.contains("1..2"), "valid range in message: {reply}");
    // 校验失败不消费待决选择：仍挂起，随后合法序号照常工作。
    let state = reply_ok(&kernel.dispatch("state", ""));
    assert!(state["pendingChoice"].is_object(), "pending must survive");
    let parsed = reply_ok(&kernel.dispatch("ygg.choice_selected", "2"));
    assert_eq!(parsed["narrative"], json!(["picked=2"]));
}

#[test]
fn choice_selected_non_integer_is_rejected_readably() {
    // A2：非整数序号（非 JSON / 浮点 / 字符串标量）→ 同一可读 VALIDATION。
    let mut kernel = KernelFacade::new(GATE_MOD).expect("kernel construction");
    let _ = reply_ok(&kernel.dispatch("event.start", "0"));
    for bad in ["abc", "1.5", r#""2""#] {
        let reply = kernel.dispatch("ygg.choice_selected", bad);
        let parsed: Json = parse(&reply);
        assert_eq!(
            parsed["error"]["code"],
            json!("VALIDATION"),
            "payload {bad}"
        );
        let message = parsed["error"]["message"].as_str().expect("message");
        assert!(
            message.contains("选项不存在") && message.contains("1..2"),
            "non-integer {bad} must name the valid range: {reply}"
        );
    }
}

#[test]
fn fuel_exhaustion_maps_to_stable_code() {
    let mut kernel = KernelFacade::new(LOOP_MOD).expect("kernel construction");
    let reply = kernel.dispatch("event.loop", "0");
    assert_eq!(reply_error_code(&reply), "FUEL_EXHAUSTED");
    // 预算确为驱动常量（转换边界重置额）：错误后再次 dispatch 正常工作。
    let parsed = reply_ok(&kernel.dispatch("event.start", "0"));
    assert_eq!(parsed["event"], json!("event.start"));
    assert_eq!(DEFAULT_FUEL_BUDGET, 100_000);
}

#[test]
fn autoscaled_budget_admits_the_scaled_workload_but_still_kills_loops() {
    // r13 击杀回归（大状态）：三段式证明「预算变大 ≠ 保护变弱」——
    // ① 大状态放行：kv_len=1000 → 自动预算 2M，几十万指令的合法工作负载
    //    在地板下必死、在扩缩预算下完整提交（burned 标记入档）；
    // ② 死循环仍被杀：同一大状态上派发纯死循环 → FUEL_EXHAUSTED
    //    （顶格区间 [100k, 4M] 内击杀语义保留）；
    // ③ canonical 回滚：击杀步写过的 KV 整体还原（probe.* 保留、burned
    //    消失），savepoint 语义与旧契约一致。
    let mut kernel = KernelFacade::new(AUTOSCALE_MOD).expect("kernel construction");
    // ① fill 自身（10k 指令量级，远低于地板 100k）提交，推高 kv_len 到 1000+。
    reply_ok(&kernel.dispatch("event.fill", "0"));
    assert_eq!(saved_kv(&mut kernel)["probe.1000"], json!(1000));
    let probe_count = saved_kv(&mut kernel)
        .as_object()
        .map(|entries| entries.keys().filter(|k| k.starts_with("probe.")).count())
        .unwrap_or(0);
    assert_eq!(probe_count, 1000, "fill must leave exactly 1000 probe keys");
    // ①' 同一负载在地地板预算下必死（对照组在 floor_budget... 用例覆盖）；
    //     这里只证扩缩后放行：burned 标记入档。
    reply_ok(&kernel.dispatch("event.burn", "0"));
    assert_eq!(
        saved_kv(&mut kernel)["burned"],
        json!(150_000i64 * 150_001 / 2),
        "scaled budget must admit the bounded workload"
    );
    // ② 死循环在大状态上仍被杀（预算 2M 有限，hook 面照杀）。
    let reply = kernel.dispatch("event.loop", "0");
    assert_eq!(reply_error_code(&reply), "FUEL_EXHAUSTED");
    // ③ 击杀步的 KV 写（无）与状态整体还原：probe.* 完整保留、后续 dispatch 正常。
    let probe_count_after = saved_kv(&mut kernel)
        .as_object()
        .map(|entries| entries.keys().filter(|k| k.starts_with("probe.")).count())
        .unwrap_or(0);
    assert_eq!(
        probe_count_after, 1000,
        "kill must not disturb canonical state"
    );
    let parsed = reply_ok(&kernel.dispatch("event.start", "0"));
    assert_eq!(parsed["event"], json!("event.start"));
}

#[test]
fn floor_budget_still_kills_the_workload_that_autoscale_admits() {
    // r13 击杀回归（空局对照）：同一 burn 负载在 kv_len≈0（地板 100k）下
    // 必须 FUEL_EXHAUSTED + 整体回滚——证明 ① 放行确实来自扩缩（不是负载
    // 本来就在地板内）；② 空局/小局行为与旧契约零变化。
    let mut kernel = KernelFacade::new(AUTOSCALE_MOD).expect("kernel construction");
    let reply = kernel.dispatch("event.burn", "0");
    assert_eq!(reply_error_code(&reply), "FUEL_EXHAUSTED");
    assert_eq!(
        saved_kv(&mut kernel),
        json!({}),
        "floor-budget kill must roll back the attempted writes"
    );
    let state = reply_ok(&kernel.dispatch("state", ""));
    assert_eq!(state["kvRevision"], json!(0), "kv revision rolls back");
    // 击杀后驱动照常工作（转换边界重置额仍 = 地板）。
    reply_ok(&kernel.dispatch("event.fill", "0"));
    assert_eq!(saved_kv(&mut kernel)["probe.1000"], json!(1000));
}

#[test]
fn payload_validation_maps_to_stable_code() {
    let mut kernel = KernelFacade::new(GATE_MOD).expect("kernel construction");
    // 嵌套对象 → 标量门拒绝。
    let reply = kernel.dispatch("event.start", r#"{"deep":{"n":1}}"#);
    assert_eq!(reply_error_code(&reply), "VALIDATION");
    // 非法 JSON → 同码。
    let reply = kernel.dispatch("event.start", "{not json");
    assert_eq!(reply_error_code(&reply), "VALIDATION");
    // 非保留名 + 标量 payload 正常（boolean / string 亦合法标量）。
    assert_eq!(
        parse(&kernel.dispatch("event.start", "true"))["ok"],
        json!(true)
    );
}

#[test]
fn sandbox_denial_maps_to_stable_code() {
    let mut kernel = KernelFacade::new(DENIED_MOD).expect("kernel construction");
    let reply = kernel.dispatch("event.denied", "0");
    assert_eq!(reply_error_code(&reply), "SANDBOX_DENIED");
}

#[test]
fn handler_state_reaches_driver_save_and_restore_continues() {
    // 统一 store 代决证据：handler 的实体/KV 写落 canonical store，
    // save 快照携带之；restore 后 handler 在恢复态上续接。
    let mut kernel = KernelFacade::new(STATE_MOD).expect("kernel construction");
    let parsed = reply_ok(&kernel.dispatch("event.hit", "0"));
    assert!(
        parsed["revision"].as_u64().unwrap_or(0) >= 1,
        "handler write bumps revision"
    );
    let save_reply = reply_ok(&kernel.dispatch("save", ""));
    let mut restored = KernelFacade::restore(
        serde_json::to_vec(&save_reply["save"]).unwrap().as_slice(),
        STATE_MOD,
    )
    .expect("restore");
    // 恢复态续接：hp=9 起点，再 hit 一次 → hp=8、hits=2（读档前累计不丢）。
    let _ = restored.dispatch("event.hit", "0");
    let restored_save = reply_ok(&restored.dispatch("save", ""));
    assert_eq!(restored_save["save"]["kv"]["entries"]["hits"], json!(2));
    // 实体表序列化为 id 键控对象（BTreeMap<EntityId, Entity>）。
    let entity = &restored_save["save"]["entities"]["entities"]["1"];
    assert_eq!(entity["attributes"]["hp"], json!(8));
}

#[test]
fn pending_choice_persisted_and_informational_after_restore() {
    let mut kernel = KernelFacade::new(GATE_MOD).expect("kernel construction");
    let _ = reply_ok(&kernel.dispatch("event.start", "0"));
    let save_reply = reply_ok(&kernel.dispatch("save", ""));
    assert!(
        save_reply["save"]["pending_choice"].is_object(),
        "pending choice is kernel state (B3 snapshot)"
    );
    let mut restored = KernelFacade::restore(
        serde_json::to_vec(&save_reply["save"]).unwrap().as_slice(),
        GATE_MOD,
    )
    .expect("restore");
    // 挂起选择原样重现（informational：状态数据，读档后玩家仍面对待决项）。
    let state = reply_ok(&restored.dispatch("state", ""));
    assert_eq!(state["pendingChoice"]["prompt"], json!("继续前行？"));
    assert_eq!(state["pendingChoice"]["options"], json!(["go", "stay"]));
    // respond 在恢复态上照常工作（选择 = 新外部事件，不依赖未读档前的栈）。
    let parsed = reply_ok(&restored.dispatch("ygg.choice_selected", "1"));
    assert_eq!(parsed["narrative"], json!(["picked=1"]));
}

#[test]
fn load_command_restores_state_in_place() {
    let mut source = KernelFacade::new(STATE_MOD).expect("kernel construction");
    let _ = reply_ok(&source.dispatch("event.hit", "0"));
    let save_reply = reply_ok(&source.dispatch("save", ""));
    let save_str = save_reply["save"].to_string();
    // 目标内核（同 Mod 重装载）经 dispatch("load") 原地恢复——不走 restore 构造器。
    let mut target = KernelFacade::new(STATE_MOD).expect("kernel construction");
    let parsed = reply_ok(&target.dispatch("load", &save_str));
    assert_eq!(parsed["restored"], json!(true));
    // 原地恢复后 handler 续接：hits 计数从 1 起再 +1（新内核装载期未跑过 hit）。
    let _ = reply_ok(&target.dispatch("event.hit", "0"));
    let final_save = reply_ok(&target.dispatch("save", ""));
    assert_eq!(final_save["save"]["kv"]["entries"]["hits"], json!(2));
    assert_eq!(final_save["save"]["engine_version"], json!(ENGINE_VERSION));
    // 坏存档 → VALIDATION 信封（域错误不 throw）。
    let reply = target.dispatch("load", r#"{"format_version":1,"engine_version":"0.0.0-x"}"#);
    assert_eq!(reply_error_code(&reply), "VALIDATION");
}

#[test]
fn event_log_records_external_events_only() {
    let mut kernel = KernelFacade::new(LOG_MOD).expect("kernel construction");
    let _ = reply_ok(&kernel.dispatch("event.log", "7"));
    let _ = reply_ok(&kernel.dispatch("event.log", "9"));
    let log_reply = reply_ok(&kernel.dispatch("log", ""));
    let log = log_reply["log"].as_array().expect("log array");
    assert_eq!(
        log,
        &vec![
            json!({"name": "event.log", "payload": 7}),
            json!({"name": "event.log", "payload": 9}),
        ],
        "derived evt.derived must not enter the external log (B1)"
    );
}

#[test]
fn determinism_same_sequence_same_canonical_bytes() {
    let mut a = KernelFacade::new(STATE_MOD).expect("kernel construction");
    let mut b = KernelFacade::new(STATE_MOD).expect("kernel construction");
    for event in ["event.hit", "event.hit", "event.hit"] {
        assert_eq!(
            reply_ok(&a.dispatch(event, "0")),
            reply_ok(&b.dispatch(event, "0"))
        );
    }
    assert_eq!(
        a.canonical_state_bytes().unwrap(),
        b.canonical_state_bytes().unwrap(),
        "same sequence must reach byte-identical canonical state (FR9 native)"
    );
}

#[test]
fn stdlib_narrative_flow_through_facade() {
    let mut kernel = KernelFacade::new(NARRATIVE_MOD).expect("kernel construction");
    let parsed = reply_ok(&kernel.dispatch("event.enter", "0"));
    assert_eq!(parsed["narrative"], json!(["node:intro", "node:scene"]));
    assert_eq!(parsed["eventsDerived"], json!(2));
    assert_eq!(parsed["eventsProcessed"], json!(3));
    // 未知节点 = 可读错误（经 VALIDATION 码浮出，不静默）。
    let mut kernel2 = KernelFacade::new(
        r#"ygg.on("event.enter", function() ygg.narrative["goto"]("node:ghost") end)"#,
    )
    .expect("kernel construction");
    let reply = kernel2.dispatch("event.enter", "0");
    assert_eq!(reply_error_code(&reply), "VALIDATION");
}

#[test]
fn stdlib_fsm_drives_state_through_facade() {
    let mut kernel = KernelFacade::new(FSM_MOD).expect("kernel construction");
    // 第一次 toggle：状态未初始化 → initial "closed" 匹配 → "open"。
    let _ = reply_ok(&kernel.dispatch("event.toggle", "0"));
    let save_reply = reply_ok(&kernel.dispatch("save", ""));
    assert_eq!(
        save_reply["save"]["kv"]["entries"]["fsm.door"],
        json!("open")
    );
    // 第二次 toggle："open" → "closed"。
    let _ = reply_ok(&kernel.dispatch("event.toggle", "0"));
    let save_reply = reply_ok(&kernel.dispatch("save", ""));
    assert_eq!(
        save_reply["save"]["kv"]["entries"]["fsm.door"],
        json!("closed")
    );
    // 状态随 KV 存档往返：restore 后第三次 toggle 续接（closed → open）。
    let mut restored = KernelFacade::restore(
        serde_json::to_vec(&save_reply["save"]).unwrap().as_slice(),
        FSM_MOD,
    )
    .expect("restore");
    let _ = reply_ok(&restored.dispatch("event.toggle", "0"));
    let final_save = reply_ok(&restored.dispatch("save", ""));
    assert_eq!(
        final_save["save"]["kv"]["entries"]["fsm.door"],
        json!("open")
    );
}

#[test]
fn stdlib_fsm_persists_initial_on_first_trigger_even_without_match() {
    // B2 键缺席可见化：首次触发且键缺席 → 先落 initial 再参与匹配；
    // 即使无匹配边（静默 no-op），initial 已落盘——handler 内 kv.get
    // 与存档 kv.entries 均可见（原语义：键缺失难排查的创作痛点）。
    const FSM_SEALED_MOD: &str = r#"
ygg.fsm.define{
  name = "gate",
  initial = "sealed",
  event = "event.knock",
  transitions = {
    { from = "open", to = "ajar" },
  },
}
ygg.on("event.probe", function()
  ygg.narrate("gate=" .. tostring(ygg.kv.get("fsm.gate")))
end)
"#;
    let mut kernel = KernelFacade::new(FSM_SEALED_MOD).expect("kernel construction");
    // define 后首次触发 knock：initial "sealed" 落盘；from="open" 无匹配 →
    // 不转移（静默 no-op 面不变，但键已在）。
    let _ = reply_ok(&kernel.dispatch("event.knock", "0"));
    let parsed = reply_ok(&kernel.dispatch("event.probe", "0"));
    assert_eq!(
        parsed["narrative"],
        json!(["gate=sealed"]),
        "initial must be visible via kv.get after first trigger"
    );
    let save_reply = reply_ok(&kernel.dispatch("save", ""));
    assert_eq!(
        save_reply["save"]["kv"]["entries"]["fsm.gate"],
        json!("sealed"),
        "initial persisted in kv.entries even when no edge matched"
    );
    // 幂等：再次触发同事件（键已存在）不重复写、不改值。
    let _ = reply_ok(&kernel.dispatch("event.knock", "0"));
    let save_reply = reply_ok(&kernel.dispatch("save", ""));
    assert_eq!(
        save_reply["save"]["kv"]["entries"]["fsm.gate"],
        json!("sealed")
    );
}

#[test]
fn stdlib_fsm_when_edges_dispatch_in_declaration_order() {
    // B1 条件边：同 from（road）双出边，声明序首个 when 成立者胜出——
    // payload 1 命中第一条（road → cellar）；payload 2 时 when 不成立，
    // 第二条无 when（恒真）胜出（road → inn）。无匹配仍静默 no-op。
    let mut kernel = KernelFacade::new(FSM_WHEN_MOD).expect("kernel construction");
    let _ = reply_ok(&kernel.dispatch("event.go", "1"));
    let save_reply = reply_ok(&kernel.dispatch("save", ""));
    assert_eq!(
        save_reply["save"]["kv"]["entries"]["fsm.router"],
        json!("cellar"),
        "payload 1 must take the first (when-true) edge"
    );
    let mut kernel = KernelFacade::new(FSM_WHEN_MOD).expect("kernel construction");
    let _ = reply_ok(&kernel.dispatch("event.go", "2"));
    let save_reply = reply_ok(&kernel.dispatch("save", ""));
    assert_eq!(
        save_reply["save"]["kv"]["entries"]["fsm.router"],
        json!("inn"),
        "payload 2 must fall through to the unconditional edge"
    );
}

#[test]
fn stdlib_fsm_when_must_be_function_at_define_time() {
    // B1 定义期校验：when 声明时必须为 function（或缺省）——否则装载期
    // 可读报错（ModLoad，含字段定位）。
    const BAD_WHEN_MOD: &str = r#"
ygg.fsm.define{
  name = "broken",
  initial = "a",
  event = "event.x",
  transitions = {
    { from = "a", to = "b", when = "yes" },
  },
}
"#;
    let outcome = KernelFacade::new(BAD_WHEN_MOD);
    assert!(
        matches!(
            outcome,
            Err(KernelError::ModLoad(ref msg))
                if msg.contains("transitions[1].when") && msg.contains("function")
        ),
        "non-function when must fail at define time with a readable error"
    );
}

#[test]
fn handler_error_message_carries_mod_chunk_name() {
    // C3：Mod chunk 命名 `@mod.lua`（与 stdlib `@stdlib/<name>` 同法）——
    // handler 内报错的错误文本必须携带 mod 侧 chunk 位置，排障定位不再
    // 只见 Lua 桥行号。注：Lua 错误格式化会剥掉文件型 chunk 名前缀的 `@`
    //（`@` 是「名字为文件」的约定记号，非名字本体），故文本形态为
    // `mod.lua:3: boom`。仍映射 VALIDATION（handler 错误既有口径不变）。
    let mut kernel = KernelFacade::new(BOOM_MOD).expect("kernel construction");
    let reply = kernel.dispatch("event.boom", "0");
    assert_eq!(reply_error_code(&reply), "VALIDATION");
    let parsed: Json = parse(&reply);
    let message = parsed["error"]["message"].as_str().expect("message");
    assert!(
        message.contains("mod.lua"),
        "error text must name the mod chunk: {reply}"
    );
    assert!(
        message.contains("boom"),
        "error text carries the message: {reply}"
    );
}

#[test]
fn choice_preceded_emits_surface_runtime_warning() {
    // D1：emit-then-choice 的丢弃不再静默——信封加性 warnings 键，一条
    // 警告合并本次 dispatch 的全部丢弃条数（2 条 emit → 一条警告计数 2）。
    let mut kernel = KernelFacade::new(CHOICE_DROPS_MOD).expect("kernel construction");
    let reply = kernel.dispatch("event.go", "0");
    let parsed = reply_ok(&reply);
    assert_eq!(
        parsed["warnings"],
        json!(["事件 `event.go` 的 handler 在 ygg.choice 之前产生了 2 条派生事件（ygg.emit），已按语义丢弃（choice 须为 handler 最后一个输出调用）"]),
        "exact deterministic warning text: {reply}"
    );
    assert_eq!(parsed["eventsDropped"], json!(2));
    assert_eq!(parsed["eventsDerived"], json!(2));
    // 丢弃面只在 emit：无 narrate 时叙述缓冲为空（语义面与改动前一致）。
    assert_eq!(parsed["narrative"], json!([]));
    assert!(parsed["pendingChoice"].is_object());
    // 警告不跨 dispatch 泄漏：respond 后的信封无 warnings 键。
    let parsed = reply_ok(&kernel.dispatch("ygg.choice_selected", "1"));
    assert!(
        parsed.get("warnings").is_none(),
        "warning must not leak to the next dispatch: {parsed}"
    );
}

#[test]
fn dispatch_without_pre_choice_emits_has_no_warnings_key() {
    // D1：无该模式的 dispatch 信封形状不变——warnings 键不存在（加性键
    // 只在非空时写入；GATE_MOD 的 choice 前无 emit）。
    let mut kernel = KernelFacade::new(GATE_MOD).expect("kernel construction");
    let parsed = reply_ok(&kernel.dispatch("event.start", "0"));
    assert!(
        parsed.get("warnings").is_none(),
        "absent pattern must not add warnings key: {parsed}"
    );
    // narrate-then-choice 也不产生警告：narrate 不在丢弃面（信封 narrative 可见）。
    const NARRATE_THEN_CHOICE_MOD: &str = r#"
ygg.on("event.scene", function()
  ygg.narrate("灯下的小酒馆")
  ygg.choice("点单吗？", { "是", "否" })
end)
"#;
    let mut kernel = KernelFacade::new(NARRATE_THEN_CHOICE_MOD).expect("kernel construction");
    let parsed = reply_ok(&kernel.dispatch("event.scene", "0"));
    assert!(
        parsed.get("warnings").is_none(),
        "narrate survives: {parsed}"
    );
    assert_eq!(parsed["narrative"], json!(["灯下的小酒馆"]));
}

#[test]
fn host_callback_rejection_carries_mod_call_site() {
    // D2：host 回调拒绝（kv.set 非有限值）的错误文本已含 mod 侧调用点——
    // mlua 在 Rust 回调返回错误时捕获当时 Lua 栈（CallbackError.traceback），
    // chunk 命名 @mod.lua（C3）使帧可读：`mod.lua:3`（kv.set 所在行）。
    // 无需穿透修复：测试固证既有行为。
    let mut kernel = KernelFacade::new(KV_NAN_MOD).expect("kernel construction");
    let reply = kernel.dispatch("event.bad", "0");
    assert_eq!(reply_error_code(&reply), "VALIDATION");
    let parsed: Json = parse(&reply);
    let message = parsed["error"]["message"].as_str().expect("message");
    assert!(
        message.contains("non-finite"),
        "bridge rejection text: {reply}"
    );
    assert!(
        message.contains("stack traceback") && message.contains("mod.lua:3"),
        "host-callback error must carry the mod call-site line: {reply}"
    );
    // entity.set_attribute 同法（拒绝点在第 4 行 → mod.lua:4）。
    const ENT_NAN_MOD: &str = r#"
ygg.on("event.bad", function()
  local id = ygg.entity.spawn()
  ygg.entity.set_attribute(id, "hp", 0/0)
end)
"#;
    let mut kernel = KernelFacade::new(ENT_NAN_MOD).expect("kernel construction");
    let reply = kernel.dispatch("event.bad", "0");
    let parsed: Json = parse(&reply);
    let message = parsed["error"]["message"].as_str().expect("message");
    assert!(
        message.contains("mod.lua:4"),
        "entity host-callback error must carry the mod call-site line: {reply}"
    );
}

// ---- dispatch 级 savepoint：一次 dispatch 要么全生效、要么全回滚 ----
//
// 缺陷（改造前实测）：`cmd_event` 的 snapshot-out 副本只是喂给
// `run_transition` 的诱饵，handler 的实体/KV 写经 host 桥 `Rc` 直落
// canonical store——handler 先写再 `error(...)` 时，写与 revision 都留在
// canonical，下一个成功步会把这份脏状态写进 autosave。

/// 半写 Mod：handler 先写实体（spawn + set_attribute）与 KV，再显式报错。
const DIRTY_THEN_ERROR_MOD: &str = r#"
ygg.on("event.bad", function()
  local id = ygg.entity.spawn()
  ygg.entity.set_attribute(id, "hp", 1)
  ygg.kv.set("dirty", "yes")
  error("boom after writes")
end)
ygg.on("event.good", function()
  ygg.kv.set("kept", "yes")
end)
"#;

/// 存档里的实体表（`save.entities.entities` = id 键控对象）。
fn saved_entities(kernel: &mut KernelFacade) -> Json {
    let envelope = reply_ok(&kernel.dispatch("save", ""));
    envelope["save"]["entities"]["entities"].clone()
}

/// 存档里的 KV 条目（`save.kv.entries`）。
fn saved_kv(kernel: &mut KernelFacade) -> Json {
    let envelope = reply_ok(&kernel.dispatch("save", ""));
    envelope["save"]["kv"]["entries"].clone()
}

#[test]
fn failed_dispatch_rolls_back_entity_and_kv_writes() {
    // 红绿锚（本次改造的回归点）：handler 写实体 + KV 后报错，canonical
    // store 必须回到 dispatch 之前的基线——实体一个都不能留、两个 revision
    // 计数器都不许涨。
    let mut kernel = KernelFacade::new(DIRTY_THEN_ERROR_MOD).expect("kernel construction");
    let reply = kernel.dispatch("event.bad", "0");
    assert_eq!(reply_error_code(&reply), "VALIDATION");
    assert_eq!(
        saved_entities(&mut kernel),
        json!({}),
        "failed dispatch must not leave entities behind"
    );
    assert_eq!(
        saved_kv(&mut kernel),
        json!({}),
        "failed dispatch must not leave kv writes behind"
    );
    let state = reply_ok(&kernel.dispatch("state", ""));
    assert_eq!(
        state["revision"],
        json!(0),
        "entity revision must roll back"
    );
    assert_eq!(state["kvRevision"], json!(0), "kv revision must roll back");
}

#[test]
fn failed_dispatch_rolls_back_kv_only_writes() {
    // 矩阵 #1：纯 KV 写 + 报错。实体面为零，只验 kv 基线独立成立（与实体
    // 基线互不牵连：不是「因为实体克隆救了场」）。
    const KV_ONLY_MOD: &str = r#"
ygg.on("event.bad", function()
  ygg.kv.set("a", 1)
  ygg.kv.set("b", 2)
  error("boom")
end)
"#;
    let mut kernel = KernelFacade::new(KV_ONLY_MOD).expect("kernel construction");
    assert_eq!(
        reply_error_code(&kernel.dispatch("event.bad", "0")),
        "VALIDATION"
    );
    assert_eq!(saved_kv(&mut kernel), json!({}), "kv writes must roll back");
    assert_eq!(
        reply_ok(&kernel.dispatch("state", ""))["kvRevision"],
        json!(0)
    );
}

#[test]
fn fuel_exhaustion_rolls_back_writes() {
    // 矩阵 #3：燃料耗尽（VM 指令计量面）与 handler 报错同等对待——写过的
    // KV 与实体全部回滚。
    const BURN_MOD: &str = r#"
ygg.on("event.burn", function()
  ygg.kv.set("burned", 1)
  local id = ygg.entity.spawn()
  ygg.entity.set_attribute(id, "hp", 1)
  while true do end
end)
"#;
    let mut kernel = KernelFacade::new(BURN_MOD).expect("kernel construction");
    assert_eq!(
        reply_error_code(&kernel.dispatch("event.burn", "0")),
        "FUEL_EXHAUSTED"
    );
    assert_eq!(
        saved_entities(&mut kernel),
        json!({}),
        "entities rolled back"
    );
    assert_eq!(saved_kv(&mut kernel), json!({}), "kv rolled back");
    let state = reply_ok(&kernel.dispatch("state", ""));
    assert_eq!(state["revision"], json!(0));
    assert_eq!(state["kvRevision"], json!(0));
}

#[test]
fn derived_drain_failure_rolls_back_the_whole_dispatch() {
    // 矩阵 #4：派生事件排空中途失败——失败点之前的写（外部事件的 handler）
    // 同样回滚：savepoint 覆盖整次 dispatch，不是单个 handler。
    const DRAIN_MOD: &str = r#"
ygg.on("event.ext", function()
  ygg.kv.set("ext_wrote", 1)
  ygg.emit("evt.d", 0)
end)
ygg.on("evt.d", function()
  ygg.kv.set("derived_wrote", 2)
  error("derived handler boom")
end)
"#;
    let mut kernel = KernelFacade::new(DRAIN_MOD).expect("kernel construction");
    let reply = kernel.dispatch("event.ext", "0");
    assert_eq!(reply_error_code(&reply), "VALIDATION");
    let parsed = parse(&reply);
    assert!(
        parsed.get("eventsProcessed").is_none(),
        "error envelope carries no counters: {reply}"
    );
    assert_eq!(
        saved_kv(&mut kernel),
        json!({}),
        "external handler write must roll back with the failed dispatch"
    );
    assert_eq!(
        reply_ok(&kernel.dispatch("state", ""))["kvRevision"],
        json!(0)
    );
}

#[test]
fn committed_dispatch_survives_a_later_failed_step() {
    // 矩阵 #5（对照组）：成功步 = 提交。失败步不得吞掉它之前的成功步写入
    // ——「一律回滚」的错实现在这里会红。
    let mut kernel = KernelFacade::new(DIRTY_THEN_ERROR_MOD).expect("kernel construction");
    reply_ok(&kernel.dispatch("event.good", "0"));
    assert_eq!(saved_kv(&mut kernel), json!({"kept": "yes"}));
    assert_eq!(
        reply_error_code(&kernel.dispatch("event.bad", "0")),
        "VALIDATION"
    );
    assert_eq!(
        saved_kv(&mut kernel),
        json!({"kept": "yes"}),
        "rollback must target the failed step only, not the committed baseline"
    );
    let state = reply_ok(&kernel.dispatch("state", ""));
    assert_eq!(state["revision"], json!(0));
    assert_eq!(
        state["kvRevision"],
        json!(1),
        "kv revision of committed step"
    );
    // 失败步之后的成功步照常续接（基线仍是 {kept:yes}）。
    reply_ok(&kernel.dispatch("event.good", "0"));
    assert_eq!(saved_kv(&mut kernel), json!({"kept": "yes"}));
}

#[test]
fn rollback_baseline_follows_a_load_overwrite() {
    // 矩阵 #6：load 覆写 canonical 之后再 dispatch，基线必须是 load 之后
    // 的状态——既不是空（未刷新基线），也不是 {x:1,y:2}（半写）。
    const LOAD_MOD: &str = r#"
ygg.on("event.seed", function() ygg.kv.set("x", 1) end)
ygg.on("event.bad", function()
  ygg.kv.set("y", 2)
  error("boom")
end)
ygg.on("event.after", function() ygg.kv.set("z", 3) end)
"#;
    let mut source = KernelFacade::new(LOAD_MOD).expect("kernel construction");
    reply_ok(&source.dispatch("event.seed", "0"));
    let save_str = reply_ok(&source.dispatch("save", ""))["save"].to_string();
    let mut target = KernelFacade::new(LOAD_MOD).expect("kernel construction");
    reply_ok(&target.dispatch("load", &save_str));
    assert_eq!(
        reply_error_code(&target.dispatch("event.bad", "0")),
        "VALIDATION"
    );
    assert_eq!(
        saved_kv(&mut target),
        json!({"x": 1}),
        "baseline must be the loaded state"
    );
    reply_ok(&target.dispatch("event.after", "0"));
    assert_eq!(saved_kv(&mut target), json!({"x": 1, "z": 3}));
}

#[test]
fn failed_dispatch_still_lands_in_external_log() {
    // 矩阵 #7：旁路面口径——失败步**照记**日志（尝试日志 = 回放前提），
    // 条目形态与成功步同形。
    let mut kernel = KernelFacade::new(DIRTY_THEN_ERROR_MOD).expect("kernel construction");
    reply_ok(&kernel.dispatch("event.good", "7"));
    let _ = kernel.dispatch("event.bad", "9");
    let log = reply_ok(&kernel.dispatch("log", ""))["log"].clone();
    assert_eq!(
        log,
        json!([
            {"name": "event.good", "payload": 7},
            {"name": "event.bad", "payload": 9},
        ]),
        "failed step is still an attempted external event"
    );
}

#[test]
fn choice_and_error_in_one_dispatch_roll_back_together() {
    // 矩阵 #8：同 dispatch 内 choice（handler1）与 error（handler2）——
    // pending_choice 本就不写（错误分支早于赋值），两个 kv 键都回滚。
    const BOTH_MOD: &str = r#"
ygg.on("event.both", function()
  ygg.kv.set("first_wrote", 1)
  ygg.choice("继续？", { "go" })
end)
ygg.on("event.both", function()
  ygg.kv.set("second_wrote", 2)
  error("second handler boom")
end)
"#;
    let mut kernel = KernelFacade::new(BOTH_MOD).expect("kernel construction");
    assert_eq!(
        reply_error_code(&kernel.dispatch("event.both", "0")),
        "VALIDATION"
    );
    let state = reply_ok(&kernel.dispatch("state", ""));
    assert!(
        state["pendingChoice"].is_null(),
        "choice from failed step drops"
    );
    assert_eq!(saved_kv(&mut kernel), json!({}));
}

#[test]
fn failed_dispatch_leaves_state_hash_and_save_bytes_unchanged() {
    // 矩阵 #9：失败步对持久面零影响——状态哈希（实体面）与 save 字节
    // （含 kv 面）都不变，连续两次失败后仍不变。
    use ygg_sandbox::hash::state_hash;

    let mut kernel = KernelFacade::new(STATE_MOD).expect("kernel construction");
    reply_ok(&kernel.dispatch("event.hit", "0"));
    let hash_before = state_hash(&kernel).expect("hash");
    let bytes_before = reply_ok(&kernel.dispatch("save", ""))["save"].to_string();
    // 失败步：写实体属性 + 写 kv 后报错（同一 Mod 的命中面被包一层错误）。
    const FAILING_HIT_MOD: &str = r#"
local id = ygg.entity.spawn()
ygg.entity.set_attribute(id, "hp", 10)
ygg.on("event.hit", function()
  ygg.entity.set_attribute(id, "hp", 99)
  ygg.kv.set("hits", 99)
  error("boom")
end)
"#;
    let mut failing = KernelFacade::new(FAILING_HIT_MOD).expect("kernel construction");
    let failing_hash = state_hash(&failing).expect("hash");
    let failing_bytes = reply_ok(&failing.dispatch("save", ""))["save"].to_string();
    for _ in 0..2 {
        assert_eq!(
            reply_error_code(&failing.dispatch("event.hit", "0")),
            "VALIDATION"
        );
    }
    assert_eq!(
        state_hash(&failing).expect("hash"),
        failing_hash,
        "entity hash must be unchanged by failed dispatches"
    );
    assert_eq!(
        reply_ok(&failing.dispatch("save", ""))["save"].to_string(),
        failing_bytes,
        "save bytes (entities + kv) must be unchanged by failed dispatches"
    );
    // 参考内核未被扰动（对照：hash/save 抓取本身不改变状态）。
    assert_eq!(state_hash(&kernel).expect("hash"), hash_before);
    assert_eq!(
        reply_ok(&kernel.dispatch("save", ""))["save"].to_string(),
        bytes_before
    );
}

#[test]
fn domain_gate_rejection_does_not_touch_committed_state() {
    // 矩阵 #11（防误伤）：`ygg.choice_selected` 序号域校验门在 savepoint
    // **之前**早退——它既不该回滚，也不该被当成失败步而抹掉已提交状态。
    const GATE_KV_MOD: &str = r#"
ygg.on("event.start", function()
  ygg.kv.set("stage", "gate")
  ygg.choice("继续？", { "go", "stay" })
end)
ygg.on("ygg.choice_selected", function(index)
  ygg.kv.set("picked", index)
end)
"#;
    let mut kernel = KernelFacade::new(GATE_KV_MOD).expect("kernel construction");
    reply_ok(&kernel.dispatch("event.start", "0"));
    assert_eq!(
        reply_error_code(&kernel.dispatch("ygg.choice_selected", "9")),
        "VALIDATION"
    );
    let state = reply_ok(&kernel.dispatch("state", ""));
    assert!(
        state["pendingChoice"].is_object(),
        "pending must survive the gate"
    );
    assert_eq!(
        state["kvRevision"],
        json!(1),
        "committed step keeps its revision"
    );
    assert_eq!(saved_kv(&mut kernel), json!({"stage": "gate"}));
    // 合法序号照常落盘（域门不是「一律拒绝」）。
    reply_ok(&kernel.dispatch("ygg.choice_selected", "2"));
    assert_eq!(saved_kv(&mut kernel), json!({"picked": 2, "stage": "gate"}));
}
