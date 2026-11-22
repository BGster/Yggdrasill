//! S9 host API 契约测试（权威 conformance 层，12 项）。
//!
//! 覆盖 D-S9-1..D-S9-4：narrate/choice/emit 输出保真、sentinel 终结、
//! 收集-回放的相对序等价、注册序、KV/实体即席写与门拒绝、seal 禁令、
//! payload 类型拒绝。
//!
//! 测试形态（每项一致）：`Sandbox::new()` →
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
use ygg_sandbox::host_api::{install as install_host, HostApi, HostHandles};
use ygg_sandbox::inject::{self, DeterminismHandle};
use ygg_sandbox::sandbox::Sandbox;

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

/// 完整测试台架：全程持有 Sandbox / inject / handles（Rc 存活面——
/// 在移交 install 前克隆出本侧句柄，见 [`Rig::new`]）。
struct Rig {
    sandbox: Sandbox,
    _inject: DeterminismHandle,
    host: HostApi,
    handles: HostHandles,
}

impl Rig {
    fn new() -> Self {
        let sandbox = Sandbox::new().expect("sandbox construction");
        let lua = sandbox.lua();
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
        run_transition(
            &mut driver_store,
            registry,
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

mod adhoc;
mod entity_face;
mod entity_query;
mod gates;
mod narrative;
