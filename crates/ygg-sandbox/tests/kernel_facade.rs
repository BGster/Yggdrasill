//! S10 KernelFacade 线格式/信封集成测试（18 测）。
//!
//! 验证面：JSON 信封形状（A1 线格式）、choice 终结与 respond（B2
//! choice-as-state）、稳定错误码映射、存读档往返与统一 store 代决证据、
//! 外部事件日志（B1 回放前提）、确定性 canonical 字节、stdlib（narrative /
//! fsm）经 facade 全链路、load 回包携带 pendingChoice（A1）、
//! choice_selected 序号域内校验（A2）、fsm when 条件边（B1）与 initial
//! 落盘可见化（B2）。
//!
//! 验证面：JSON 信封形状（A1 线格式）、choice 终结与 respond（B2
//! choice-as-state）、稳定错误码映射、存读档往返与统一 store 代决证据、
//! 外部事件日志（B1 回放前提）、确定性 canonical 字节、stdlib（narrative /
//! fsm）经 facade 全链路。
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
