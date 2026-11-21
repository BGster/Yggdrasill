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

// ---- TD1：`ygg.meta` 结构化回执通道（与叙事分离、瞬态、值面把关）----

#[test]
fn meta_channel_is_separate_from_narrative() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("meta.write", function()
            ygg.narrate("人读叙事")
            ygg.meta("summary", { stage = "review", conflicts = { { id = "r1" } } })
        end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let result = rig.run_full(&mut registry, "meta.write", Value::Int(0));
    // meta 不进 calls 回放面：叙事只有那一条人读行，没有任何结构化残渣
    assert_eq!(result.narrative(), &["人读叙事".to_owned()]);
    // 结构化数据在 HostState 里（驱动 take_meta 后拼信封），与叙事完全分离
    let meta = rig.host.take_meta();
    assert_eq!(meta["summary"]["stage"], serde_json::json!("review"));
    assert_eq!(
        meta["summary"]["conflicts"][0]["id"],
        serde_json::json!("r1")
    );
    // 取走即清空；clear_meta 同样幂等清空
    assert!(rig.host.take_meta().is_empty());
    rig.load_phase(r#"ygg.meta("after", 1)"#);
    rig.host.clear_meta();
    assert!(rig.host.take_meta().is_empty());
}

#[test]
fn meta_rejects_bad_keys_and_shapes_readably() {
    let rig = Rig::new();
    let check = |chunk: &str, needle: &str| {
        let msg = rig
            .lua()
            .load(chunk)
            .exec()
            .expect_err("chunk must be rejected")
            .to_string();
        assert!(
            msg.contains(needle),
            "expected `{needle}` in the readable error, got: {msg}"
        );
    };
    check(r#"ygg.meta("", 1)"#, "must not be empty");
    check(
        r#"ygg.meta("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", 1)"#,
        "too long",
    );
    check(r#"ygg.meta(7, 1)"#, "key must be a string");
    check(r#"ygg.meta("x", 0 / 0)"#, "non-finite");
    // 稀疏 / 混合表都是可读拒绝（不静默猜形状）
    check(r#"ygg.meta("x", { [2] = 1 })"#, "dense 1..n");
    check(r#"ygg.meta("x", { 1, 2, k = 3 })"#, "mixes array indices");
    // 深度封顶（9 层起拒绝）
    check(r#"ygg.meta("x", {{{{{{{{{{1}}}}}}}}}})"#, "nesting exceeds");
}

// ── `ygg.entity.*` 批式面（P-K A1；铁律②「数组进出、单次调用承载整批」）──────────

/// 键集是**闭集**：六个单条面 + 五个批式面 + 两个控制面（`exists` / `destroy`，S9）
/// + 一个查询面（`query`，S24），多一个即红（防退化成逐实体数据 getter；控制面是单标量、
/// 不搬数据，查询面是一次跨界承载整批）。
#[test]
fn entity_face_keys_are_exactly_the_fourteen_documented_keys() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.keys", function()
            local out = ""
            for key in pairs(ygg.entity) do
                out = out .. key .. ","
            end
            ygg.narrate("KEYS:" .. out)
        end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let result = rig.run_full(&mut registry, "evt.keys", Value::Int(0));
    let line = result.narrative()[0]
        .strip_prefix("KEYS:")
        .expect("narrate 必须以 KEYS: 开头");
    let mut seen: Vec<&str> = line.split(',').filter(|s| !s.is_empty()).collect();
    seen.sort_unstable();
    let mut expected = vec![
        "add_tag",
        "attr_apply",
        "attr_fetch",
        "destroy",
        "exists",
        "get_attribute",
        "has_tag",
        "query",
        "remove_tag",
        "scan",
        "set_attribute",
        "spawn",
        "spawn_batch",
        "tag_batch",
    ];
    expected.sort_unstable();
    assert_eq!(
        seen, expected,
        "ygg.entity 的键集必须恰为这 14 个：{seen:?}"
    );
}

/// 批式面的行为：造 3 行（属性 + tag）、列式取回、同值不写、整/浮身份保留、
/// tag 过滤扫描、批量去 tag——一条用例把五个面的「值域/顺序/幂等」都钉住。
#[test]
fn entity_batch_face_round_trip_preserves_values_and_identity() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.batch", function()
            local made = ygg.entity.spawn_batch({
                { attrs = { hp = 10, title = "a" }, tags = { "enemy", "boss" } },
                { attrs = { hp = 2.5 }, tags = { "enemy" } },
                { attrs = {} },
            })
            ygg.narrate("SPAWNED:" .. made.spawned .. "|" .. made.next_id
                .. "|" .. made.ids[1] .. "," .. made.ids[3])

            local got = ygg.entity.attr_fetch({
                { id = made.ids[1], keys = { "hp", "title", "missing" } },
                { id = made.ids[2], keys = { "hp" } },
            })
            ygg.narrate("FETCH:" .. tostring(got.values[1].hp) .. "|"
                .. tostring(got.values[1].title) .. "|"
                .. tostring(got.values[1].missing) .. "|"
                .. tostring(got.values[2].hp))
            ygg.narrate("TAGS1:" .. #got.tags[1] .. ":" .. got.tags[1][1]
                .. "," .. got.tags[1][2])

            -- 同值写：applied 计 1、changed 计 0（不写、不发事件的上游依据）
            local same = ygg.entity.attr_apply({ { id = made.ids[1], attrs = { hp = 10 } } })
            -- 2.5 → 2：整/浮身份变了，值也确实变了
            local ident = ygg.entity.attr_apply({ { id = made.ids[2], attrs = { hp = 2 } } })
            ygg.narrate("APPLY:" .. same.applied .. "/" .. same.changed
                .. "|" .. ident.applied .. "/" .. ident.changed)

            local scanned = ygg.entity.scan(1, 10, "enemy")
            ygg.narrate("SCAN:" .. #scanned.ids .. ":" .. scanned.ids[1]
                .. "," .. scanned.ids[2])

            local removed = ygg.entity.tag_batch({
                { id = made.ids[1], tag = "boss", add = false },
            })
            local again = ygg.entity.tag_batch({
                { id = made.ids[1], tag = "boss", add = false },
            })
            ygg.narrate("TAGBATCH:" .. removed .. "|" .. again)
        end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let result = rig.run_full(&mut registry, "evt.batch", Value::Int(0));
    assert_eq!(
        result.narrative(),
        [
            "SPAWNED:3|4|1,3".to_string(),
            "FETCH:10|a|nil|2.5".to_string(),
            "TAGS1:2:boss,enemy".to_string(),
            "APPLY:1/0|1/1".to_string(),
            "SCAN:2:1,2".to_string(),
            "TAGBATCH:1|0".to_string(),
        ]
    );

    // 宿主侧直读：值真的落到 `EntityStore` 上，且 `2` 是 Int、`2.5` 曾是 Float。
    let entities = rig.handles.entities.borrow();
    let first = entities.get(1).expect("实体 1 必须在库里");
    assert_eq!(first.attribute("hp"), Some(&Value::Int(10)));
    assert_eq!(first.attribute("title"), Some(&Value::Str("a".to_string())));
    assert!(!first.has_tag("boss"), "tag_batch 必须真的把它摘掉");
    assert!(first.has_tag("enemy"));
    let second = entities.get(2).expect("实体 2 必须在库里");
    assert_eq!(
        second.attribute("hp"),
        Some(&Value::Int(2)),
        "2.5 → 2 必须是 Int"
    );
}

/// 三族错误码：形状（FRAME）/ 值域（VALUE）/ 未知 id（ID），且**整批零副作用**。
#[test]
fn entity_batch_face_rejects_before_mutating() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        local function first_line(err)
            local text = tostring(err)
            local cut = text:find("\n", 1, true)
            return cut and text:sub(1, cut - 1) or text
        end
        ygg.on("evt.reject", function()
            -- 非表入参由 mlua 的签名转换先拒（消息不同）；这里测「表里的元素形状错」
            local _, frame = pcall(function() return ygg.entity.spawn_batch({ "nope" }) end)
            ygg.narrate("FRAME:" .. first_line(frame))

            local made = ygg.entity.spawn_batch({ { attrs = { hp = 1 } } })
            local _, nan = pcall(function()
                return ygg.entity.attr_apply({ { id = made.ids[1], attrs = { hp = 0 / 0 } } })
            end)
            ygg.narrate("VALUE:" .. first_line(nan))

            local _, missing = pcall(function()
                return ygg.entity.attr_apply({ { id = 999, attrs = { hp = 1 } } })
            end)
            ygg.narrate("ID:" .. first_line(missing))

            -- 先校验后改：坏批之后的 hp 仍是 1
            local after = ygg.entity.attr_fetch({ { id = made.ids[1], keys = { "hp" } } })
            ygg.narrate("AFTER:" .. tostring(after.values[1].hp))
        end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let result = rig.run_full(&mut registry, "evt.reject", Value::Int(0));
    let lines = result.narrative();
    // mlua 会在 Rust error 前面加一层 `runtime error: `，所以按**包含**断言码。
    assert!(
        lines[0].contains("YGG-ENTITY-FRAME"),
        "形状错必须报 YGG-ENTITY-FRAME：{}",
        lines[0]
    );
    assert!(
        lines[1].contains("YGG-ENTITY-VALUE"),
        "NaN 必须报 YGG-ENTITY-VALUE：{}",
        lines[1]
    );
    assert!(
        lines[2].contains("YGG-ENTITY-ID"),
        "未知 id 必须报 YGG-ENTITY-ID：{}",
        lines[2]
    );
    assert_eq!(lines[3], "AFTER:1", "被拒的批不得留下半截写入");
}

// ── `ygg.entity.exists` / `ygg.entity.destroy`（S9 控制面两键）──────────────────

/// 存在性是内核真源的读面，真删是它的写面：两键合起来把「死亡」表达成
/// **记录不存在**（而不是墓碑位遮蔽），且删除幂等、缺失 id 不报错。
///
/// 钉住的四件事：① `exists` 真/假/从未存在；② `destroy` 首次 true、二次 false、
/// 未知 id false（不抛错）；③ 真删之后宿主直读 store：属性 / tag / 记录一起没了；
/// ④ I1——删后 `spawn` 拿到更大的 id（水位不动）。
#[test]
fn entity_lifecycle_exists_and_destroy_are_the_liveness_pair() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.life", function()
            local id = ygg.entity.spawn()
            ygg.entity.set_attribute(id, "hp", 7)
            ygg.entity.add_tag(id, "enemy")
            ygg.narrate("ALIVE:" .. tostring(ygg.entity.exists(id)))
            ygg.narrate("GHOST:" .. tostring(ygg.entity.exists(999)))

            local first = ygg.entity.destroy(id)
            local second = ygg.entity.destroy(id)
            local missing = ygg.entity.destroy(999)
            ygg.narrate("DESTROY:" .. tostring(first) .. ","
                .. tostring(second) .. "," .. tostring(missing))

            ygg.narrate("AFTER:" .. tostring(ygg.entity.exists(id)))
            ygg.narrate("HP:" .. tostring(ygg.entity.get_attribute(id, "hp")))
            ygg.narrate("TAG:" .. tostring(ygg.entity.has_tag(id, "enemy")))

            local fresh = ygg.entity.spawn()
            ygg.narrate("FRESH:" .. tostring(fresh) .. ">?old")
        end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let result = rig.run_full(&mut registry, "evt.life", Value::Int(0));
    assert_eq!(
        result.narrative(),
        [
            "ALIVE:true".to_string(),
            "GHOST:false".to_string(),
            "DESTROY:true,false,false".to_string(),
            "AFTER:false".to_string(),
            "HP:nil".to_string(),
            "TAG:false".to_string(),
            "FRESH:2>?old".to_string(),
        ]
    );

    // 宿主侧直读：真删就是真删——记录不在表里，属性与 tag 随记录一起回收。
    let entities = rig.handles.entities.borrow();
    assert_eq!(entities.len(), 1, "只剩删后新分配的那一条：{entities:?}");
    assert!(!entities.contains(1), "实体 1 必须整条消失");
    assert!(entities.contains(2), "删后 spawn 拿到的必须是新 id 2");
    assert_eq!(
        entities.next_entity_id(),
        3,
        "水位不动也不复用（I1）：remove 不碰 next_id"
    );
}

// ── `ygg.entity.query`（S24 通用实体查询引擎）─────────────────────────────────

/// 出表两形态 + `count` 兜底 + `where`/`tag`/`select`/`order` 四处接线。
///
/// 形态规则（契约 `packages/entity.md` §9.8）：`select` 缺省 = **裸 id 数组**；
/// 非空 = 行表（行恒带 `id`，缺属性**不设键**）；`count` 恒在，空结果可判。
#[test]
fn entity_query_ids_and_rows_shapes() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.query", function()
            ygg.entity.spawn_batch({
                { attrs = { hp = 50, name = "orc" }, tags = { "enemy" } },
                { attrs = { hp = 5.5, name = "rat" }, tags = { "enemy" } },
                { attrs = { hp = 100, name = "goblin" }, tags = { "enemy", "boss" } },
                { attrs = { name = "rock" } },
            })
            local function flat(list)
                local text = ""
                for i = 1, #list do
                    text = text .. (i > 1 and "," or "") .. tostring(list[i])
                end
                return text
            end
            local all = ygg.entity.query({})
            ygg.narrate("ALL:" .. all.count .. ":" .. flat(all.ids))
            ygg.narrate("EMPTY:" .. tostring(ygg.entity.query({ tag = "nobody" }).count))
            local enemy = ygg.entity.query({ tag = "enemy" })
            ygg.narrate("TAG:" .. enemy.count .. ":" .. flat(enemy.ids))
            local young = ygg.entity.query({ where = { hp = "lte.50" } })
            ygg.narrate("YOUNG:" .. young.count .. ":" .. flat(young.ids))
            local rows = ygg.entity.query({ where = { hp = "gte.50" }, select = { "hp", "name" } })
            local text = ""
            for i = 1, #rows.rows do
                local row = rows.rows[i]
                text = text .. (i > 1 and ";" or "") .. tostring(row.id)
                    .. "/" .. tostring(row.hp) .. "/" .. tostring(row.name)
                    .. "/" .. tostring(row.missing)
            end
            ygg.narrate("ROWS:" .. rows.count .. ":" .. text)
            -- 空结果的具名形态：`rows` 为空表、`count` 仍在（形态由 selector 决定）
            local none = ygg.entity.query({ where = { hp = "gt.1000" }, select = { "hp" } })
            ygg.narrate("NOROWS:" .. tostring(none.count) .. ":" .. tostring(#none.rows))
        end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let result = rig.run_full(&mut registry, "evt.query", Value::Int(0));
    assert_eq!(
        result.narrative(),
        [
            "ALL:4:1,2,3,4".to_string(),
            "EMPTY:0".to_string(),
            "TAG:3:1,2,3".to_string(),
            "YOUNG:2:1,2".to_string(),
            "ROWS:2:1/50/orc/nil;3/100/goblin/nil".to_string(),
            "NOROWS:0:0".to_string(),
        ]
    );
}

/// G2（v2 §6）：字符串态的五种形态一次钉死——
/// `op` / `op.operand` / `in.(…)` / `not.op.operand` / `is.xxx`。
#[test]
fn entity_query_string_form_parses_five_shapes() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.shapes", function()
            ygg.entity.spawn_batch({
                { attrs = { hp = 10 } },
                { attrs = { hp = 20 } },
                { attrs = { hp = 30, note = "x" } },
                { attrs = { name = "bare" } },
            })
            local function flat(list)
                local text = ""
                for i = 1, #list do
                    text = text .. (i > 1 and "," or "") .. tostring(list[i])
                end
                return text
            end
            local function ids(selector)
                return flat(ygg.entity.query(selector).ids)
            end
            -- ① 无操作数：操作数是空串（`eq` 一个字符串）⇒ 数值属性上恒不命中
            ygg.narrate("A:" .. ids({ where = { hp = "eq" } }))
            -- ② op.operand
            ygg.narrate("B:" .. ids({ where = { hp = "gte.20" } }))
            -- ③ in.(…)（数字按形态解析；空 `()` = 恒假）
            ygg.narrate("C:" .. ids({ where = { hp = "in.(10,30)" } }))
            ygg.narrate("C0:" .. ids({ where = { hp = "in.()" } }))
            -- ④ not.op.operand：整谓词取反，但**缺属性仍假**
            ygg.narrate("D:" .. ids({ where = { hp = "not.lt.20" } }))
            -- ⑤ is.null / is.notnull：存在性
            ygg.narrate("E:" .. ids({ where = { note = "is.null" } }))
            ygg.narrate("F:" .. ids({ where = { note = "is.notnull" } }))
        end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let result = rig.run_full(&mut registry, "evt.shapes", Value::Int(0));
    assert_eq!(
        result.narrative(),
        [
            "A:".to_string(),
            "B:2,3".to_string(),
            "C:1,3".to_string(),
            "C0:".to_string(),
            // 20 与 30 命中；**缺 hp 的 4 不命中**（`not.` 不翻「缺属性 ⇒ 假」）
            "D:2,3".to_string(),
            "E:1,2,4".to_string(),
            "F:3".to_string(),
        ]
    );
}

/// G3（v2 §6）：文法拒绝面。**先校验后扫**：形状错 `YGG-ENTITY-FRAME`、
/// 值域错 `YGG-ENTITY-VALUE`，且空库上照样拒（错误不依赖扫到什么）。
#[test]
fn entity_query_grammar_rejects() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        local function first_line(err)
            local text = tostring(err)
            local cut = text:find("\n", 1, true)
            return cut and text:sub(1, cut - 1) or text
        end
        ygg.on("evt.grammar", function()
            local function code(label, fn)
                local ok, err = pcall(fn)
                ygg.narrate(label .. ":" .. (ok and "OK" or first_line(err)))
            end
            -- 库是空的：下面的拒绝全都是「扫描前」发生的
            code("array_where", function() ygg.entity.query({ where = { { "hp", "gte", 1 } } }) end)
            code("unknown_op", function() ygg.entity.query({ where = { hp = "between.1" } }) end)
            code("unknown_key", function() ygg.entity.query({ whre = { hp = "gte.1" } }) end)
            code("like_wide", function() ygg.entity.query({ where = { name = "like.a*b" } }) end)
            code("like_bare", function() ygg.entity.query({ where = { name = "like.*" } }) end)
            code("in_null", function() ygg.entity.query({ where = { hp = "in.(1,null)" } }) end)
            code("eq_null", function() ygg.entity.query({ where = { hp = "eq.null" } }) end)
            code("is_operand", function() ygg.entity.query({ where = { hp = "is.maybe" } }) end)
            code("non_string_key", function() ygg.entity.query({ where = { [1] = "eq.1" } }) end)
            code("select_id", function() ygg.entity.query({ select = { "id" } }) end)
            code("tag_type", function() ygg.entity.query({ tag = 5 }) end)
            code("order_type", function() ygg.entity.query({ order = {} }) end)
            code("structured_extra",
                function() ygg.entity.query({ where = { hp = { op = "gte", value = 1, x = 1 } } }) end)
            code("structured_inline",
                function() ygg.entity.query({ where = { hp = { op = "gte.1", value = 1 } } }) end)
            code("structured_missing_value",
                function() ygg.entity.query({ where = { hp = { op = "gte" } } }) end)
            code("nan_value",
                function() ygg.entity.query({ where = { hp = { op = "gte", value = 0 / 0 } } }) end)
            code("in_non_scalar",
                function() ygg.entity.query({ where = { hp = { op = "in", value = { { 1 } } } } }) end)
            -- 合法但**不该**报错的两条（防把门做成过宽）
            code("ok_empty_where", function() ygg.entity.query({ where = {} }) end)
            code("ok_no_operand_eq", function() ygg.entity.query({ where = { name = "eq." } }) end)
        end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let result = rig.run_full(&mut registry, "evt.grammar", Value::Int(0));
    let lines = result.narrative();
    let frame = [
        "array_where",
        "unknown_op",
        "unknown_key",
        "like_wide",
        "like_bare",
        "in_null",
        "eq_null",
        "is_operand",
        "non_string_key",
        "select_id",
        "tag_type",
        "order_type",
        "structured_extra",
        "structured_inline",
        "structured_missing_value",
    ];
    for (index, label) in frame.iter().enumerate() {
        assert!(
            lines[index].contains("YGG-ENTITY-FRAME"),
            "{label} 必须报 YGG-ENTITY-FRAME：{}",
            lines[index]
        );
    }
    assert!(
        lines[15].contains("YGG-ENTITY-VALUE"),
        "NaN 必须报 YGG-ENTITY-VALUE：{}",
        lines[15]
    );
    assert!(
        lines[16].contains("YGG-ENTITY-VALUE"),
        "in 的非标量元素必须报 YGG-ENTITY-VALUE：{}",
        lines[16]
    );
    assert_eq!(lines[17], "ok_empty_where:OK", "空 where = 无过滤，不是错");
    assert_eq!(
        lines[18], "ok_no_operand_eq:OK",
        "`eq.` = 空串操作数，不是错"
    );
}

/// I4（v2 §6）：`order` 的值同 ⇒ id 升序兜底（asc/desc 都不反转）、
/// 缺属性排最后（与方向无关）、`id` 伪属性可排。
#[test]
fn entity_query_order_tiebreak() {
    let mut rig = Rig::new();
    rig.load_phase(
        r#"
        ygg.on("evt.order", function()
            ygg.entity.spawn_batch({
                { attrs = { hp = 7 } },
                { attrs = { hp = 7 } },
                { attrs = { hp = 3 } },
                { attrs = {} },
            })
            local function flat(list)
                local text = ""
                for i = 1, #list do
                    text = text .. (i > 1 and "," or "") .. tostring(list[i])
                end
                return text
            end
            ygg.narrate("PLAIN:" .. flat(ygg.entity.query({}).ids))
            ygg.narrate("ASC:" .. flat(ygg.entity.query({ order = "hp" }).ids))
            ygg.narrate("ASC2:" .. flat(ygg.entity.query({ order = "hp.asc" }).ids))
            ygg.narrate("DESC:" .. flat(ygg.entity.query({ order = "hp.desc" }).ids))
            ygg.narrate("IDDESC:" .. flat(ygg.entity.query({ order = "id.desc" }).ids))
        end)
    "#,
    );
    let mut registry = rig.seal_and_register();
    let result = rig.run_full(&mut registry, "evt.order", Value::Int(0));
    assert_eq!(
        result.narrative(),
        [
            "PLAIN:1,2,3,4".to_string(),
            // 3 < 7 = 7（id 兜底 1 < 2）< 缺属性
            "ASC:3,1,2,4".to_string(),
            "ASC2:3,1,2,4".to_string(),
            // desc 只反转值序：7 = 7 的 id 兜底仍升序，缺属性仍在最后
            "DESC:1,2,3,4".to_string(),
            "IDDESC:4,3,2,1".to_string(),
        ]
    );
}
