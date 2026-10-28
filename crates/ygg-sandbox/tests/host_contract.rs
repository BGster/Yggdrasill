//! S9 host API 契约测试（权威 conformance 层，12 项）。
//!
//! 覆盖 D-S9-1..D-S9-4：narrate/choice/emit 输出保真、sentinel 终结、
//! 收集-回放的相对序等价、注册序、KV/实体即席写与门拒绝、seal 禁令、
//! payload 类型拒绝。
//!
//! 测试形态（每项一致）：`Sandbox::new()` → `fuel::install` →
//! `inject::install` → `host_api::install`（持 [`HostHandles`]）→ 装载期
//! Lua chunk 经 `ygg.on` 注册 handler → `seal` → `register_into` → 转换驱动。
//! 输出断言取**两个互补面**：
//!
//! - **result 面**：`run_transition`（内核转换驱动，`TransitionResult` 携带
//!   narrative / pending_choice / 事件计数——run_transition 内建 per-transition
//!   driver，不接收外部 sink，这是 core 的既定面）；
//! - **sink 面**：`registry.handle_event` 直派单事件进 [`RecordingSink`]——
//!   桥（LuaHandler → sink 回放序）的逐项观测面，与 result 面互证
//!   「收集-回放相对序等价」。
//!
//! 二面只在**无状态副作用**的测试（1–5）中叠加（同一 registry 两次驱动
//! 会重复执行 handler；6–9 有状态写，只用单面）。
//!
//! **driver store 与宿主句柄 store 的关系（有意为之，非疏漏）**：本层
//! `run_transition` 传入的 `EntityStore` 是独立的 driver 侧 store；宿主
//! 句柄里的 `Rc<RefCell<EntityStore>>` 由 `ygg.entity.*` 即席写消费并被
//! 测试直接观测。二者合一（同一 store 同时作为转换输入与 handler 可见面）
//! 属 S10 驱动接线职责——RefCell 借用纪律（转换期间 handler 不得与 driver
//! 互借）也在彼处收口；本层契约对象是 host API 桥语义本身。

use std::cell::RefCell;
use std::rc::Rc;

use ygg_core::bus::{EventSink, HandlerRegistry, NarrationSink, PendingChoice};
use ygg_core::entity::{EntityStore, Value};
use ygg_core::event::Event;
use ygg_core::kv::KvStore;
use ygg_core::transition::{run_transition, TransitionError, TransitionResult};
use ygg_sandbox::fuel::{self, HookFuelMeter};
use ygg_sandbox::host_api::{install as install_host, HostApi, HostHandles};
use ygg_sandbox::inject::{self, DeterminismHandle};
use ygg_sandbox::sandbox::Sandbox;

/// 测试燃料预算：派发税 + VM 指令计量共享，对短 handler 序列绰绰有余。
const FUEL_BUDGET: u64 = 10_000;

/// 记录型 sink：narrative / choices / emitted 三面各自按到达序收集。
#[derive(Default)]
struct RecordingSink {
    narrative: Vec<String>,
    choices: Vec<PendingChoice>,
    emitted: Vec<(String, Value)>,
}

impl EventSink for RecordingSink {
    fn emit(&mut self, event: Event) {
        self.emitted
            .push((event.name().to_owned(), event.payload().clone()));
    }
}

impl NarrationSink for RecordingSink {
    fn narrate(&mut self, line: String) {
        self.narrative.push(line);
    }
    fn choose(&mut self, choice: PendingChoice) {
        self.choices.push(choice);
    }
}

/// 完整测试台架：全程持有 Sandbox / fuel / inject / handles（Rc 存活面——
/// 在移交 install 前克隆出本侧句柄，见 [`Rig::new`]）。
struct Rig {
    sandbox: Sandbox,
    fuel: HookFuelMeter,
    _inject: DeterminismHandle,
    host: HostApi,
    handles: HostHandles,
}

impl Rig {
    fn new() -> Self {
        let sandbox = Sandbox::new().expect("sandbox construction");
        let lua = sandbox.lua();
        let fuel_meter = fuel::install(lua, FUEL_BUDGET).expect("fuel install");
        let determinism = inject::install(lua).expect("determinism install");
        let handles = HostHandles {
            kv: Rc::new(RefCell::new(KvStore::new())),
            entities: Rc::new(RefCell::new(EntityStore::new())),
        };
        // install 移交所有权；台架侧克隆 Rc 句柄保持存活与可观测。
        let local_handles = HostHandles {
            kv: Rc::clone(&handles.kv),
            entities: Rc::clone(&handles.entities),
        };
        let host = install_host(lua, handles).expect("host api install");
        Rig {
            sandbox,
            fuel: fuel_meter,
            _inject: determinism,
            host,
            handles: local_handles,
        }
    }

    fn lua(&self) -> &mlua::Lua {
        self.sandbox.lua()
    }

    /// 装载期：执行 chunk（chunk 内经 `ygg.on` 注册 handler）。
    fn load_phase(&self, chunk: &str) {
        self.lua()
            .load(chunk)
            .exec()
            .expect("load phase chunk must succeed");
    }

    /// seal + register_into（每测试一次；后续二面共用同一 registry）。
    fn seal_and_register(&self) -> HandlerRegistry {
        self.host.seal(self.lua()).expect("seal after load phase");
        let mut registry = HandlerRegistry::new();
        self.host.register_into(&mut registry, "mod");
        registry
    }

    /// result 面：完整 `run_transition`（driver store 独立，见模块文档）。
    fn run_full(
        &mut self,
        registry: &mut HandlerRegistry,
        event: &str,
        payload: Value,
    ) -> TransitionResult {
        let mut driver_store = EntityStore::new();
        let mut fuel_meter = self.fuel.clone();
        run_transition(
            &mut driver_store,
            registry,
            &mut fuel_meter,
            Event::external(event, payload).expect("external event"),
        )
        .unwrap_or_else(|err| panic!("transition `{event}` failed: {err}"))
    }
}

/// sink 面：直派单事件进 registry（无 FIFO 排空；桥回放序的逐项观测面）。
fn dispatch_once(
    registry: &mut HandlerRegistry,
    sink: &mut RecordingSink,
    event: &str,
    payload: Value,
) {
    let external = Event::external(event, payload).expect("external event");
    registry
        .handle_event(event, &external, sink)
        .unwrap_or_else(|err| panic!("dispatch `{event}` failed: {err}"));
}

#[test]
fn narrate_reaches_output_buffer() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.talk", function(p) ygg.narrate("hello " .. tostring(p)) end)
    "#,
    );
    // sink 面：narrate 经回放进 sink
    let mut registry = rig.seal_and_register();
    let mut sink = RecordingSink::default();
    dispatch_once(
        &mut registry,
        &mut sink,
        "evt.talk",
        Value::Str("world".into()),
    );
    assert_eq!(sink.narrative, vec!["hello world".to_string()]);
    assert!(sink.choices.is_empty());
    assert!(sink.emitted.is_empty());
    // result 面：narrate 进转换输出缓冲
    let result = rig.run_full(&mut registry, "evt.talk", Value::Str("world".into()));
    assert_eq!(result.narrative(), ["hello world".to_string()]);
    assert!(result.pending_choice().is_none());
}

#[test]
fn choice_terminates_handler_and_sets_pending_choice() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.gate", function()
            ygg.narrate("before-choice")
            ygg.choice("Which path?", {"left", "right"})
            ygg.narrate("after-choice")
        end)
        -- 同事件第二 handler 也 choice：first-wins 裁定的对侧锚
        ygg.on("evt.gate", function() ygg.choice("Second", {"x"}) end)
    "#,
    );
    // sink 面：prompt/options 保真；桥回放两个 handler 的 choose 各自进 sink
    let mut registry = rig.seal_and_register();
    let mut sink = RecordingSink::default();
    dispatch_once(&mut registry, &mut sink, "evt.gate", Value::Int(0));
    let choice = sink.choices.first().expect("choice must reach the sink");
    assert_eq!(choice.prompt(), "Which path?");
    assert_eq!(choice.options(), ["left", "right"]);
    assert!(sink.narrative.contains(&"before-choice".to_string()));
    assert!(!sink.narrative.contains(&"after-choice".to_string()));
    // result 面：first-wins——驱动层裁定首个 choice 胜出（桥面两 handler 均已回放）
    let result = rig.run_full(&mut registry, "evt.gate", Value::Int(0));
    assert_eq!(
        result.pending_choice().expect("result pending").prompt(),
        "Which path?"
    );
    assert_eq!(
        result.pending_choice().expect("result pending").options(),
        ["left", "right"]
    );
    assert!(result.narrative().contains(&"before-choice".to_string()));
    assert!(!result.narrative().contains(&"after-choice".to_string()));
}

#[test]
fn code_after_choice_does_not_execute() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.stop", function()
            ygg.narrate("entered")
            ygg.choice("Pick one", {"a"})
            ygg.narrate("FORBIDDEN-AFTER-CHOICE")
        end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let mut sink = RecordingSink::default();
    dispatch_once(&mut registry, &mut sink, "evt.stop", Value::Int(0));
    // sentinel 锚：choice 之后的 narrate 不得出现在任何输出面
    assert_eq!(sink.narrative, vec!["entered".to_string()]);
    assert_eq!(sink.choices.len(), 1);
    let result = rig.run_full(&mut registry, "evt.stop", Value::Int(0));
    assert_eq!(result.narrative(), ["entered".to_string()]);
    assert!(result.pending_choice().is_some());
}

#[test]
fn emit_drains_fifo_to_downstream_lua_handler() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.start", function()
            ygg.emit("evt.a", 1)
            ygg.emit("evt.b", 2)
            ygg.narrate("start-done")
        end)
        ygg.on("evt.a", function() ygg.narrate("a-ran") end)
        ygg.on("evt.b", function() ygg.narrate("b-ran") end)
    "#,
    );
    // sink 面：emit 回放进 sink 按收集序、payload 保真；emit→narrate 交错序
    // 在 handler 内部保真（emit 先于 narrate 收集，回放同序）。
    let mut registry = rig.seal_and_register();
    let mut sink = RecordingSink::default();
    dispatch_once(&mut registry, &mut sink, "evt.start", Value::Int(0));
    assert_eq!(
        sink.emitted,
        vec![
            ("evt.a".to_string(), Value::Int(1)),
            ("evt.b".to_string(), Value::Int(2))
        ]
    );
    assert_eq!(sink.narrative, vec!["start-done".to_string()]);
    // result 面：FIFO 排空序 start → a → b（downstream narrate 在后）
    let result = rig.run_full(&mut registry, "evt.start", Value::Int(0));
    assert_eq!(
        result.narrative(),
        [
            "start-done".to_string(),
            "a-ran".to_string(),
            "b-ran".to_string()
        ]
    );
    // 计数：外部 1 + 派生 2 = 3；choice 缺席无丢弃
    assert_eq!(result.events_processed(), 3);
    assert_eq!(result.events_derived(), 2);
    assert_eq!(result.events_dropped(), 0);
}

#[test]
fn registration_order_is_handler_call_order() {
    let rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.ord", function() ygg.narrate("first") end)
        ygg.on("evt.ord", function() ygg.narrate("second") end)
        ygg.on("evt.ord", function() ygg.narrate("third") end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    // D-S9-4：注册序 = ygg.on 调用序（稳定键 seq 单调）
    let seqs: Vec<u64> = registry.keys("evt.ord").iter().map(|k| k.seq).collect();
    assert_eq!(seqs, vec![0, 1, 2]);
    // 派发序 = 注册序（sink 面运行时序）
    let mut sink = RecordingSink::default();
    dispatch_once(&mut registry, &mut sink, "evt.ord", Value::Int(0));
    assert_eq!(
        sink.narrative,
        vec![
            "first".to_string(),
            "second".to_string(),
            "third".to_string()
        ]
    );
}

#[test]
fn kv_set_get_round_trip() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.kv", function()
            ygg.kv.set("gold", 30)
            ygg.kv.set("name", "feni")
            ygg.narrate("gold=" .. tostring(ygg.kv.get("gold")))
            ygg.narrate("name=" .. tostring(ygg.kv.get("name")))
            local miss = ygg.kv.get("missing")
            ygg.narrate(miss == nil and "MISS_NIL" or "PRESENT")
        end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let result = rig.run_full(&mut registry, "evt.kv", Value::Int(0));
    assert_eq!(
        result.narrative(),
        [
            "gold=30".to_string(),
            "name=feni".to_string(),
            "MISS_NIL".to_string()
        ]
    );
    // 即席写直达宿主句柄 store（非回放）
    assert_eq!(rig.handles.kv.borrow().get("gold"), Some(&Value::Int(30)));
    assert_eq!(
        rig.handles.kv.borrow().get("name"),
        Some(&Value::Str("feni".into()))
    );
}

#[test]
fn kv_nan_rejected_with_readable_error() {
    let rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.nan", function() ygg.kv.set("ratio", 0.0 / 0.0) end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let mut driver_store = EntityStore::new();
    let mut fuel_meter = rig.fuel.clone();
    let err = run_transition(
        &mut driver_store,
        &mut registry,
        &mut fuel_meter,
        Event::external("evt.nan", Value::Int(0)).unwrap(),
    )
    .expect_err("NaN kv set must fail the transition");
    let msg = err.to_string();
    assert!(
        matches!(err, TransitionError::Handler(_)),
        "handler error expected: {msg}"
    );
    // 可读性：消息含键名 + 非有限语义
    assert!(msg.contains("ratio"), "error must name the key: {msg}");
    assert!(
        msg.contains("non-finite"),
        "error must state non-finite: {msg}"
    );
    // 拒绝路径不落表
    assert!(rig.handles.kv.borrow().get("ratio").is_none());
}

#[test]
fn entity_attribute_set_get_round_trip() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.ent", function()
            local id = ygg.entity.spawn()
            ygg.narrate("id=" .. tostring(id))
            ygg.entity.set_attribute(id, "hp", 42)
            ygg.narrate("hp=" .. tostring(ygg.entity.get_attribute(id, "hp")))
            local miss = ygg.entity.get_attribute(id, "nope")
            ygg.narrate(miss == nil and "ATTR_NIL" or "PRESENT")
        end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let result = rig.run_full(&mut registry, "evt.ent", Value::Int(0));
    assert_eq!(
        result.narrative(),
        [
            "id=1".to_string(),
            "hp=42".to_string(),
            "ATTR_NIL".to_string()
        ]
    );
    // 即席写直达宿主句柄 store
    let entities = rig.handles.entities.borrow();
    let entity = entities.get(1).expect("entity 1 must exist");
    assert_eq!(entity.attribute("hp"), Some(&Value::Int(42)));
}

#[test]
fn entity_tags_ops_round_trip() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.tag", function()
            local id = ygg.entity.spawn()
            local added = ygg.entity.add_tag(id, "hero")
            local dup = ygg.entity.add_tag(id, "hero")
            local has = ygg.entity.has_tag(id, "hero")
            local removed = ygg.entity.remove_tag(id, "hero")
            local has_after = ygg.entity.has_tag(id, "hero")
            ygg.narrate(tostring(added) .. "," .. tostring(dup) .. ","
                .. tostring(has) .. "," .. tostring(removed) .. ","
                .. tostring(has_after))
        end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let result = rig.run_full(&mut registry, "evt.tag", Value::Int(0));
    assert_eq!(
        result.narrative(),
        ["true,false,true,true,false".to_string()]
    );
    assert!(!rig
        .handles
        .entities
        .borrow()
        .get(1)
        .expect("entity 1")
        .has_tag("hero"));
}

#[test]
fn ygg_on_after_seal_errors_readably() {
    let rig = Rig::new();
    rig.host.seal(rig.lua()).expect("first seal succeeds");
    let err = rig
        .lua()
        .load(r#"ygg.on("evt.late", function() end)"#)
        .exec()
        .expect_err("sealed ygg.on must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("sealed"),
        "readable seal error expected: {msg}"
    );
    assert!(
        msg.contains("forbidden") || msg.contains("load phase"),
        "error must state the registration ban: {msg}"
    );
    // 重复 seal 同样可读拒绝
    let err2 = rig.host.seal(rig.lua()).expect_err("double seal must fail");
    assert!(
        err2.to_string().contains("sealed"),
        "double seal message: {err2}"
    );
}

#[test]
fn unsupported_payload_type_rejected_readably() {
    let rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.bad", function() ygg.emit("evt.out", {1, 2}) end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let mut driver_store = EntityStore::new();
    let mut fuel_meter = rig.fuel.clone();
    let err = run_transition(
        &mut driver_store,
        &mut registry,
        &mut fuel_meter,
        Event::external("evt.bad", Value::Int(0)).unwrap(),
    )
    .expect_err("table payload must be rejected");
    let msg = err.to_string();
    assert!(
        matches!(err, TransitionError::Handler(_)),
        "handler error expected: {msg}"
    );
    assert!(
        msg.contains("unsupported payload type"),
        "readable rejection: {msg}"
    );
    assert!(msg.contains("table"), "error must name the lua type: {msg}");
}

#[test]
fn forged_sentinel_error_is_not_swallowed() {
    // S9 审计代决回归锚：sentinel 为 install 时派生的不可预测 token；
    // Mod 以静态字符串伪造首行只会成为普通 HandlerError 浮出，不会被吞。
    let rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.forge", function()
            error("\u{1}__ygg_choice_sentinel__\u{1}")
        end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let mut driver_store = EntityStore::new();
    let mut fuel_meter = rig.fuel.clone();
    let err = run_transition(
        &mut driver_store,
        &mut registry,
        &mut fuel_meter,
        Event::external("evt.forge", Value::Int(0)).unwrap(),
    )
    .expect_err("forged sentinel must surface as a handler error");
    assert!(
        matches!(err, TransitionError::Handler(_)),
        "handler error expected, got: {err}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("ygg_choice_sentinel"),
        "forged message must be visible (not swallowed): {msg}"
    );
    assert!(
        !msg.contains("choice") || msg.contains("failed"),
        "must not be treated as normal choice termination: {msg}"
    );
}
