//! 沙箱构造（FR3 / D-继承-1 / D-继承-3）：确定性 Lua 5.4 VM 的最小安全面。
//!
//! - **stdlib 裁剪**：`Lua::new_with(ALL_SAFE & !StdLib::OS)`——mlua 的
//!   `ALL_SAFE` 是内存安全而非确定性安全（research §2：其安全子集仍含 `OS`），
//!   必须显式剔除；`DEBUG`（unsafe 库）本就不在 `ALL_SAFE` 内，`new_with`
//!   对 DEBUG 直接拒绝（SafetyError），双重防线。注意：os.* 的拒绝是**结构性
//!   缺席**（new_with 裁剪层，访问即裸 nil-call），可读报错路径由 require 桩
//!   承载；S9 host API 层如需 os 类拒绝的可读报错再扩展。
//! - **内存上限**：`Lua::set_memory_limit(SANDBOX_MEMORY_LIMIT)`——超限分配
//!   触发 `Error::MemoryError`（mlua 文档语义），负例测试实证。
//! - **白名单环境**（deny-by-default）：构造全新 globals 表并 `Lua::set_globals`
//!   整体替换——原 globals 里的 io/os/package/debug/coroutine 与原始 `math` 库
//!   自此不可达。新表只暴露四类面：base 纯计算子集（[`BASE_WHITELIST`]：无
//!   dofile/loadfile/load/print/rawset/collectgarbage——文件系统、任意编译与
//!   GC 干预面全部拒之门外）；标准库计算表（[`LIBRARY_WHITELIST`]：
//!   `string` / `table` / `utf8`，`string.dump` 已摘）；受控 `require`（r1：按
//!   已登记模块表装载同 Mod 的其它文件，其它一切名字可读拒绝；实现见
//!   [`crate::modules`]）；`ygg = { engine_version }` host API 占位（完整 host
//!   API 落 S9/S10）。`math` 不在新表里由本函数放：它由 [`crate::inject`]
//!   从零构造（精确子集 + 世界流 random）。
//! - **require 面**（r1）：不在本文件直接实现——[`crate::modules::install`]
//!   装上「查已登记模块表 + 缓存 + 循环/深度判栈 + 副作用影子表」的装载器；
//!   模块表由驱动在入口 chunk 之前注入（[`Sandbox::modules`] 句柄），装载期
//!   与运行期（handler 内部）都能 require。空表时 `require` 对任何名字给可读
//!   拒绝（单文件 Mod 的既有行为不变）。
//! - **指令预算**（M2）：`set_global_hook` 每 [`BUDGET_QUANTUM`] 条字节码计一次
//!   燃料，耗尽抛内核哨兵（[`BudgetExhausted`]）；`pcall` / `xpcall` 换成内核
//!   shim（[`install_catch_shims`]）——
//!   预算是「宿主入口的燃料」，不是可捕获的异常面（旧 S34 退役的是**事件/
//!   信封式**燃料计量，本机制是 VM 级指令计数，两回事）。
//! - **确定性种子**：`luai_makeseed` 构建期常量覆盖（workspace 根
//!   `.cargo/config.toml`，D-继承-3）——本 crate 的种子锚测试实证「同进程两个
//!   Lua 状态对同序插入产出相同 `pairs` 迭代序」；跨进程一致性归 S12
//!   golden-vector。状态哈希路径依旧绝不消费 `pairs()` 序（D-继承-5 纵深）。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use mlua::{
    Error, Function, HookTriggers, Lua, LuaOptions, MultiValue, StdLib, Table, Value, VmState,
};
use ygg_sim::component::{ComponentId, EntityId};
use ygg_sim::error::SimError;
use ygg_sim::sim::Sim;

use crate::modules::{self, ModuleHandle};
use crate::world_ops::{self, WorldOp};

/// 引擎版本（`ygg.engine_version` 占位面 + S5 存档双版本号的引擎侧取值源）。
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 沙箱内存上限（M0 常量：64 MB）——`Lua::set_memory_limit` 的入参。
pub const SANDBOX_MEMORY_LIMIT: usize = 64 * 1024 * 1024;

/// base 库纯计算子集白名单（deny-by-default 的暴露面）。
///
/// 排除（显式决策，非遗漏）：`dofile` / `loadfile`（文件系统）、`load`
/// （任意编译——装载期后动态生成代码会破坏「handler 集是装载纯函数」，D-B4）、
/// `print`（输出必须走 host API narrate，S9）、`rawset`（绕过元语义写）、
/// `collectgarbage`（GC 干预面，与内存上限语义交叉）。
///
/// `pcall` / `xpcall` 不在这里：它们由 [`install_catch_shims`] 换成内核 shim
/// （同一个语义，但预算哨兵不可被脚本吞掉）。
const BASE_WHITELIST: &[&str] = &[
    "assert",
    "error",
    "getmetatable",
    "ipairs",
    "next",
    "pairs",
    "rawequal",
    "rawget",
    "rawlen",
    "select",
    "setmetatable",
    "tonumber",
    "tostring",
    "type",
];

/// 放行的标准库表（`StdLib::ALL_SAFE` 已经加载好，这里只决定「露不露」）。
///
/// 规则代码连字符串都拼不起来的日子到此为止：`string` / `table` / `utf8` 是
/// 纯计算面（无 IO、无时钟、无堆外可变量），露出来不会动摇确定性。
/// `math` 不在本表：它由 [`crate::inject`] **从零构造**（精确子集 + 世界 rng 流），
/// 绝不 raw_get 原始库表——那会把超越函数一并夹带进来。
const LIBRARY_WHITELIST: &[&str] = &["string", "table", "utf8"];

/// 指令预算的计量粒度（每 N 条字节码一次 hook）。
///
/// 1000 条：小到 `while true do end` 在毫秒级被截断，大到 hook 自身的开销
/// （一次 Rust 闭包调用）摊薄到可忽略。
pub const BUDGET_QUANTUM: u32 = 1000;

/// 单次宿主入口的默认指令预算。
///
/// 2e6 条：比任何合法规则的一个 tick 宽出两个数量级（10k 次迭代的循环约 1e5 条），
/// 又小到失控循环在数十毫秒内被截断。
pub const DEFAULT_INSTRUCTION_BUDGET: u64 = 2_000_000;

/// 单次入口的预算硬上限（宿主也不能把预算调到「实质无限」）。
pub const MAX_INSTRUCTION_BUDGET: u64 = 64_000_000;

/// 预算耗尽的稳定错误码（`Display` 的首段，外部可 grep、可断言）。
pub const CODE_BUDGET: &str = "E_BUDGET";

/// 指令预算耗尽——**内核拥有的哨兵**。
///
/// 为什么是一个类型而不是一个字符串：字符串是可伪造、可匹配的值，脚本只要
/// `if type(e) == "string" and e:find("E_BUDGET")` 就能把预算中止识别出来并
/// 「换个写法重试」。这里的哨兵变成 Lua 侧的**不透明 userdata**（Rust 回调
/// 抛错一律包进内核的错误对象），脚本既看不懂也伪造不出；`pcall` / `xpcall`
/// 的 shim 又按预算状态无条��重抛（见 [`install_catch_shims`]），于是「捕获后
/// 继续跑」在 VM 里没有落点。宿主侧仍拿到**类型化**的错误
/// （[`budget_exhausted`]），码在首段。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "E_BUDGET: 指令预算耗尽（本次入口上限 {limit} 条）；规则代码必须终止，\
     预算中止不是可捕获的正常控制流"
)]
pub struct BudgetExhausted {
    /// 本次入口的预算上限（诊断面：作者要知道「多少条算多」）。
    pub limit: u64,
}

impl BudgetExhausted {
    /// 稳定错误码。
    pub fn code(&self) -> &'static str {
        CODE_BUDGET
    }
}

/// 从 mlua 错误里取出预算哨兵（`None` = 该错误不是预算耗尽）。
///
/// 宿主入口只需要这一个判定：`Err` 不为空就是 `E_BUDGET`，不需要解析错误文本
/// （文本是给人看的，判定不看文本）。
pub fn budget_exhausted(err: &Error) -> Option<&BudgetExhausted> {
    err.downcast_ref::<BudgetExhausted>()
}

/// 预算状态（hook 与 pcall shim 共享同一个 `Rc`）。
struct InstructionBudget {
    /// 本次入口的上限。
    limit: Cell<u64>,
    /// 还剩余的指令数（按 [`BUDGET_QUANTUM`] 递减）。
    remaining: Cell<u64>,
    /// 是否已耗尽（**粘滞**：一旦置位就一直中止，直到宿主重置预算）。
    exhausted: Cell<bool>,
}

impl InstructionBudget {
    fn new(limit: u64) -> Self {
        Self {
            limit: Cell::new(limit.min(MAX_INSTRUCTION_BUDGET)),
            remaining: Cell::new(limit.min(MAX_INSTRUCTION_BUDGET)),
            exhausted: Cell::new(false),
        }
    }

    fn reset(&self) {
        self.remaining.set(self.limit.get());
        self.exhausted.set(false);
    }

    fn set_limit(&self, limit: u64) {
        self.limit.set(limit.min(MAX_INSTRUCTION_BUDGET));
        self.reset();
    }

    fn error(&self) -> Error {
        Error::external(BudgetExhausted {
            limit: self.limit.get(),
        })
    }
}

/// 装预算 hook（`set_global_hook`，**不是** `set_hook`）。
///
/// `set_global_hook` 在主状态上装，对所有 chunk / 所有函数生效；`set_hook` 是
/// per-thread + per-function-mask，在一个全新状态上等于什么都没装。
fn install_budget_hook(lua: &Lua, budget: Rc<InstructionBudget>) -> Result<(), Error> {
    let triggers = HookTriggers::new().every_nth_instruction(BUDGET_QUANTUM);
    lua.set_global_hook(triggers, move |_, _| {
        let remaining = budget.remaining.get();
        if remaining < u64::from(BUDGET_QUANTUM) {
            // 归零 + 置位：此后每次 hook 都立即中止，所以「捕获后接着算」
            // 最多再跑一个 quantum 就又被打断——不存在可绕过的窗口。
            budget.remaining.set(0);
            budget.exhausted.set(true);
            return Err(budget.error());
        }
        budget.remaining.set(remaining - u64::from(BUDGET_QUANTUM));
        Ok(VmState::Continue)
    })
}

/// 把 `pcall` / `xpcall` 换成内核 shim：真身照旧做受保护调用，但**预算中止
/// 不允许被返回值化**。
///
/// 真身是从「旧 globals」里取出来的（`fresh` 里不放真身，脚本也就拿不到它）；
/// 判定依据是共享的预算状态而不是错误值本身——错误值的类型可以是任何东西，
/// 状态只有一个。
fn install_catch_shims(
    lua: &Lua,
    fresh: &Table,
    old: &Table,
    budget: Rc<InstructionBudget>,
) -> Result<(), Error> {
    for name in ["pcall", "xpcall"] {
        let real: Function = old.raw_get(name)?;
        let budget = Rc::clone(&budget);
        let shim = lua.create_function(move |_, args: MultiValue| {
            let returned: MultiValue = real.call(args)?;
            // 粘滞标记：只要本次入口的预算耗尽过，受保护调用的「成功」返回值
            // 就不是成功——把它重新变成错误。这条路径不需要认识错误值，
            // 所以脚本没有任何办法让它失效。
            if budget.exhausted.get() {
                return Err(budget.error());
            }
            Ok(returned)
        })?;
        fresh.raw_set(name, shim)?;
    }
    Ok(())
}

/// `string.dump` 摘除（在真表上摘，露出的就是摘过的同一张表）。
///
/// 它把函数序列化成字节码：既是内核实现细节的出口，也是「把代码当数据
/// 排出去」的通道。其余 `string` 成员全部有界、纯计算。
fn strip_string_dump(globals: &Table) -> Result<(), Error> {
    let string: Table = globals.raw_get("string")?;
    string.raw_set("dump", Value::Nil)?;
    Ok(())
}

/// 构造完成的确定性沙箱（一个 [`Sandbox`] 持有一个独立 Lua 状态；
/// 测试与 VM 状态绝不跨转换共享——research Precedents 4307ee5 教训）。
pub struct Sandbox {
    lua: Lua,
    /// 模块装载面句柄（驱动：注入模块表 + 装载结束收口；燃料：转交装载预算）。
    modules: ModuleHandle,
    /// 世界内核槽（宿主侧时间与全部世界状态）。
    ///
    /// 与 [`crate::host_api`] 的 `HostState` **共享同一个 `Rc`**：Lua 的 `world.*`
    /// 面在宿主 `step` 的窗口之外才取得借用；窗口内（系统体里回手调 `world.*`）
    /// 借用冲突被 `world.rs` 翻成可读的 `WORLD_BUSY`，不是 `RefCell` panic。
    sim: Rc<RefCell<Option<Box<Sim>>>>,
    /// 指令预算状态（与 hook、pcall shim 共享同一个 `Rc`）。
    budget: Rc<InstructionBudget>,
}

/// 一次宿主推进的结果（[`Sandbox::step`]）。
///
/// 给**终值与增量**：`tick` 是内核时钟（绝对序号，与本次推进了几 tick 无关），
/// `hash` 是该时刻的规范状态哈希（快照帧尾的 fnv1a64）；`changed` 是本次推进里
/// **真的变过的格** `(实体, 组件)`，按 `(eid, cid)` 升序去重。中间每 tick 的状态
/// 不返回——回放证据要的是「同一序列到同一终态」，不是逐步回执；而增量面只要
/// 「谁变了」，不要「怎么变的」。
///
/// `changed` 的来源是内核 undo 日志（[`ygg_sim::Sim::changed`]）：不另装仪器，
/// 所以不存在「某个写点忘了报」。同一格多次写（多 tick 或多次写回）只算一条；
/// 结构操作（生成/删除/加列/摘列）不产生条目——协议层的 `delta` 因此看不见结构变更。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StepReport {
    pub tick: u64,
    pub hash: u64,
    /// 本次推进写过的格 `(实体, 组件)`（升序去重）。
    pub changed: Vec<(EntityId, ComponentId)>,
}

/// 宿主推进面的唯一错误类型。
///
/// 码是**稳定字符串**（外部可 grep、可断言），与 `YGG-SIM-*` 一族同形：
/// `E_STEP_TICKS` / `E_STEP_BUDGET` 是本面自有的两个参数拒绝，其余直接沿用
/// 内核的 `YGG-SIM-*` 码（`code()` 直通）。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StepError {
    /// [`Sandbox::step`] 的 `ticks` 为 0：推进 0 tick 没有语义，且会让
    /// 「调用成功但时间没走」与「宿主忘了推进」不可分辨。
    #[error("E_STEP_TICKS: ticks 必须 ≥ 1（推进 0 tick 无意义）")]
    ZeroTicks,
    /// [`Sandbox::step`] 的 `ticks` 超过单次上限 [`STEP_TICK_BUDGET`]。
    #[error("E_STEP_BUDGET: ticks={ticks} 超过单次上限 {}", STEP_TICK_BUDGET)]
    Budget { ticks: u32 },
    /// 内核拒绝（未挂载 / 推进中出错）。
    #[error(transparent)]
    Sim(#[from] SimError),
}

impl StepError {
    /// 稳定错误码。
    pub fn code(&self) -> &'static str {
        match self {
            StepError::ZeroTicks => "E_STEP_TICKS",
            StepError::Budget { .. } => "E_STEP_BUDGET",
            StepError::Sim(err) => err.code(),
        }
    }
}

/// 单次推进的 tick 上限。
///
/// **为什么有上限**：宿主一次调用里插入了整段不可中断的时间。一万 tick 已是
/// 分钟级的真实工作（一个 tick 要跑完全部系统），更大的批量只会在一个
/// `&mut self` 里把主线程锁死——调用方既无法中途取消，也拿不到部分进度。
/// 需要更多时间就多次 `step`：每次回来都能看 hash、能存档、能被中断。
/// 这是「宿主驱动时间」与「Lua 驱动时间」的关键区别：前者的粒度可被宿主控制，
/// 后者的时间长度由脚本决定（所以 `ygg` 的 `sim.step` 被整个删掉）。
pub const STEP_TICK_BUDGET: u32 = 10_000;

impl Sandbox {
    /// 按默认约束构造沙箱：stdlib 裁剪 + 64 MB 内存上限 + 白名单环境。
    pub fn new() -> Result<Self, Error> {
        Self::build(SANDBOX_MEMORY_LIMIT)
    }

    /// 实际构造流程（`memory_limit` 参数化为测试提供小上限观测面；生产路径
    /// 恒为 [`SANDBOX_MEMORY_LIMIT`]）。
    fn build(memory_limit: usize) -> Result<Self, Error> {
        // ① stdlib 裁剪：ALL_SAFE 剔除 OS（确定性）——DEBUG 本就不在其中。
        let lua = Lua::new_with(StdLib::ALL_SAFE & !StdLib::OS, LuaOptions::default())?;

        // ② 内存上限：超限分配即 Error::MemoryError（不依赖 hook，独立机制）。
        lua.set_memory_limit(memory_limit)?;

        // ③ 白名单环境：全新 globals 表 deny-by-default 整体替换（chunk 装载发生在
        //    替换之后，chunk 的 _ENV 即新表）。
        let fresh = lua.create_table()?;
        let old = lua.globals();
        for name in BASE_WHITELIST {
            let value: Value = old.raw_get(*name)?;
            fresh.raw_set(*name, value)?;
        }
        // ③b 标准库表白名单：`string` / `table` / `utf8` 原样露出（纯计算面）。
        for name in LIBRARY_WHITELIST {
            let lib: Table = old.raw_get(*name)?;
            fresh.raw_set(*name, lib)?;
        }
        // ③c `string.dump` 摘掉（见 [`strip_string_dump`]）。
        strip_string_dump(&old)?;

        // ③d 指令预算 hook：**必须早于任何库/Mod 代码**——hook 装晚了，先跑
        //     的代码不计量（`stdlib::install` 在 modload 里，晚于本函数）。
        let budget = Rc::new(InstructionBudget::new(DEFAULT_INSTRUCTION_BUDGET));
        install_budget_hook(&lua, Rc::clone(&budget))?;

        // ③e 预算中止不可捕获：`pcall` / `xpcall` 换成内核 shim。
        install_catch_shims(&lua, &fresh, &old, Rc::clone(&budget))?;

        // ④ require 面：受控模块装载器（r1）。空模块表就位——未注入模块的
        //    沙箱对任何名字都给可读拒绝（白名单面不放行外部模块）；模块表由
        //    驱动在入口 chunk 之前注入。调用时机不设边界（封的是注册面，
        //    不是模块面）。
        let modules = modules::install(&lua, &fresh)?;

        // ⑤ host API 占位：ygg = { engine_version }（完整 host API 落 S9/S10）。
        let ygg = lua.create_table()?;
        ygg.raw_set("engine_version", ENGINE_VERSION)?;
        fresh.raw_set("ygg", ygg)?;

        lua.set_globals(fresh)?;
        Ok(Self {
            lua,
            modules,
            sim: Rc::new(RefCell::new(None)),
            budget,
        })
    }

    /// 重置指令预算（**每次宿主入口之前调一次**）。
    ///
    /// 预算的语义是「一次入口的总燃料」而不是「每 chunk 各一份」：派生调用
    /// （handler 里再 `require`、再进别的系统体）共享同一个计数器，否则
    /// 「多包几层就不受限」就是一条可绕路。
    pub fn reset_budget(&self) {
        self.budget.reset();
    }

    /// 设定本次入口的指令预算（超 [`MAX_INSTRUCTION_BUDGET`] 按上限夹断并重置）。
    pub fn set_instruction_budget(&self, instructions: u64) {
        self.budget.set_limit(instructions);
    }

    /// 本次入口的指令预算上限。
    pub fn instruction_budget(&self) -> u64 {
        self.budget.limit.get()
    }

    /// 预算是否已耗尽（**状态判定，不解析错误文本**：宿主入口只需看这一个值
    /// 配 [`budget_exhausted`]）。
    pub fn budget_exhausted(&self) -> bool {
        self.budget.exhausted.get()
    }

    /// 内部 Lua 句柄（inject/host 面经此接线；
    /// 状态哈希等消费面在 S10+ 才出现，本 slice 不再扩 API）。
    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    /// 模块装载面句柄（驱动：`set_modules` 注入模块表 / `load_budget` 接
    /// 装载期燃料）。
    pub fn modules(&self) -> &ModuleHandle {
        &self.modules
    }

    /// 内核槽（与 `world.*` 共享；宿主批量帧、存档与哈希都经此取实例）。
    pub fn sim_handle(&self) -> Rc<RefCell<Option<Box<Sim>>>> {
        Rc::clone(&self.sim)
    }

    /// 注入或卸载世界内核实例。
    ///
    /// 未注入时 `world.*` 与 [`Sandbox::step`] 全部报 `YGG-SIM-UNMOUNTED`——
    /// 不是 `nil`、不是 panic：未挂载与「世界是空的」必须可分辨。
    pub fn set_sim(&mut self, sim: Option<Box<Sim>>) {
        *self.sim.borrow_mut() = sim;
    }

    /// **时间只从这里走**：推进 `ticks` 个 tick，每 tick 的步长是 `dt` 秒。
    ///
    /// `dt` 是**整数秒**（`u32`），在这里换成内核的 Q32.32 定点值（`dt << 32`）；
    /// 系统体里拿到的 `dt` 仍是 Q32.32 裸值，即 `dt = 1` 时体里看到 `4294967296`。
    /// 整数秒而不是毫秒：tick 长度在确定性世界里的语义是「世界时间前进多少」，
    /// 而毫秒以上的精度只能靠定点小数表达——那会把单位偷偷变成半个浮点面。
    ///
    /// Lua 侧**没有任何推进时钟的入口**（`ygg` 的 `sim.step` 已随 `ygg` 的 `sim.*` 面删除）：
    /// 脚本能决定「一个 tick 里算什么」，不能决定「走多少 tick」。这就是宿主与
    /// 脚本的分界：时间的所有权与可中断性。
    ///
    /// 推进期间内核槽被独占借用，系统体里回手调 `world.*` 得到 `WORLD_BUSY`
    /// 可读错误（而非借用 panic）。
    pub fn step(&mut self, ticks: u32, dt: u32) -> Result<StepReport, StepError> {
        if ticks == 0 {
            return Err(StepError::ZeroTicks);
        }
        if ticks > STEP_TICK_BUDGET {
            return Err(StepError::Budget { ticks });
        }
        let dt = ygg_sim::fixed::from_int(i64::from(dt));

        let slot = Rc::clone(&self.sim);
        let mut guard = slot.borrow_mut();
        let sim = guard
            .as_mut()
            .ok_or_else(|| SimError::Unmounted(String::from("Sandbox::step")))?;
        // 借用在整个循环里不放：这是 `WORLD_BUSY` 的物理来源。
        let mut changed: Vec<(EntityId, ComponentId)> = Vec::new();
        for _ in 0..ticks {
            sim.step(dt)?;
            // 每 tick 一拍：内核的 `changed` 只描述刚刚完成的那次事务。
            changed.extend_from_slice(sim.changed());
        }
        changed.sort_unstable();
        changed.dedup();
        Ok(StepReport {
            tick: sim.tick(),
            hash: sim.hash(),
            changed,
        })
    }

    /// 宿主批量写面：一整批 [`WorldOp`] **要么全成、要么全回**。
    ///
    /// 返回实际生效的写次数（值写入按格计，结构操作按受影响实体计）。
    /// 校验与落笔全在 [`world_ops`]（内核语义的唯一实现）——宿主面只把协议结构
    /// 翻译成 `WorldOp`，不在这里也不在宿主里复刻一份校验。
    ///
    /// 推进中（`step` 持有内核槽）调用得到 `WORLD_BUSY` 可读错误，与 `world.*` 同源。
    pub fn apply(&mut self, ops: &[WorldOp]) -> Result<usize, StepError> {
        let slot = Rc::clone(&self.sim);
        let mut guard = slot.borrow_mut();
        let sim = guard
            .as_mut()
            .ok_or_else(|| SimError::Unmounted(String::from("Sandbox::apply")))?;
        world_ops::apply_ops(sim, ops).map(|report| report.writes)
    }

    /// 当前世界时钟（内核 tick；未挂载 = `YGG-SIM-UNMOUNTED`）。
    pub fn hash(&self) -> Result<u64, StepError> {
        let slot = Rc::clone(&self.sim);
        let guard = slot.borrow();
        let sim = guard
            .as_ref()
            .ok_or_else(|| SimError::Unmounted(String::from("Sandbox::hash")))?;
        Ok(sim.hash())
    }

    pub fn tick(&self) -> Result<u64, StepError> {
        let slot = Rc::clone(&self.sim);
        let guard = slot.borrow();
        let sim = guard
            .as_ref()
            .ok_or_else(|| SimError::Unmounted(String::from("Sandbox::tick")))?;
        Ok(sim.tick())
    }

    /// 当前世界状态的规范字节（快照帧；同一状态 ⇒ 同一字节）。
    ///
    /// 宿主能力，Lua 不可达：能存档就等于能回到任意时刻，那是时间机器的
    /// 权限，不是脚本的权限（旧 `ygg` 的 `sim.snapshot/restore` 面已删）。
    pub fn snapshot(&self) -> Result<Vec<u8>, StepError> {
        let slot = Rc::clone(&self.sim);
        let guard = slot.borrow();
        let sim = guard
            .as_ref()
            .ok_or_else(|| SimError::Unmounted(String::from("Sandbox::snapshot")))?;
        Ok(sim.snapshot())
    }

    /// 从快照帧字节恢复世界状态（先校验后改：坏帧不产生半改的态）。
    pub fn restore(&mut self, bytes: &[u8]) -> Result<(), StepError> {
        let slot = Rc::clone(&self.sim);
        let mut guard = slot.borrow_mut();
        let sim = guard
            .as_mut()
            .ok_or_else(|| SimError::Unmounted(String::from("Sandbox::restore")))?;
        sim.restore(bytes)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 沙箱内执行一段 Lua 并返回其错误文本（负例断言辅助）。
    fn exec_err(lua: &Lua, chunk: &str) -> String {
        lua.load(chunk)
            .exec()
            .expect_err("chunk expected to fail")
            .to_string()
    }

    #[test]
    fn whitelisted_environment_exposes_only_the_allow_list() {
        let sb = Sandbox::new().expect("sandbox construction");
        let lua = sb.lua();
        // host API 占位面可达，且取值正确。
        let version: String = lua.load("return ygg.engine_version").eval().unwrap();
        assert_eq!(version, ENGINE_VERSION);
        // base 纯计算子集可达。
        let kind: String = lua.load("return type(pairs)").eval().unwrap();
        assert_eq!(kind, "function");
        // deny-by-default：OS（stdlib 裁剪层）与其余被替换 globals 不可达。
        for name in ["os", "io", "package", "math", "coroutine"] {
            let absent: bool = lua.load(format!("return {name} == nil")).eval().unwrap();
            assert!(absent, "{name} must be absent from the sandbox globals");
        }
        // 纯计算表白名单：`string` / `table` / `utf8` 露头（不再“连字符串都拼
        // 不起来”），且摘掉了 `string.dump`。
        for name in ["string", "table", "utf8"] {
            let kind: String = lua.load(format!("return type({name})")).eval().unwrap();
            assert_eq!(kind, "table", "{name} must be exposed");
        }
        let dump: bool = lua.load("return string.dump == nil").eval().unwrap();
        assert!(dump, "string.dump must be stripped");
        // 预算中止不可捕获：`pcall` / `xpcall` 仍是函数（内核 shim），
        // 但不是旧 globals 里的真身——真身不出现在环境里。
        for name in ["pcall", "xpcall"] {
            let kind: String = lua.load(format!("return type({name})")).eval().unwrap();
            assert_eq!(kind, "function", "{name} shim must be installed");
        }
        // 白名单排除面：文件系统 / 任意编译 / print / rawset / GC 干预。
        for name in [
            "dofile",
            "loadfile",
            "load",
            "print",
            "rawset",
            "collectgarbage",
        ] {
            let absent: bool = lua.load(format!("return {name} == nil")).eval().unwrap();
            assert!(absent, "{name} must not be exposed (deny-by-default)");
        }
    }

    #[test]
    fn require_is_denied_with_readable_error() {
        let sb = Sandbox::new().expect("sandbox construction");
        let msg = exec_err(sb.lua(), r#"require("io")"#);
        assert!(
            msg.contains("sandbox denied"),
            "readable denial expected: {msg}"
        );
        assert!(
            msg.contains("io"),
            "error must name the rejected module: {msg}"
        );
    }

    #[test]
    fn runaway_allocation_hits_memory_limit() {
        // 1 MB 上限的观测沙箱：逐迭代分配小表，先于指令预算耗尽撞上内存上限。
        let sb = Sandbox::build(1024 * 1024).expect("sandbox construction");
        let err = sb
            .lua()
            .load("local t = {} for i = 1, 1000000 do t[i] = { i } end")
            .exec()
            .unwrap_err();
        assert!(
            matches!(err, Error::MemoryError(_)),
            "expected a memory error, got: {err}"
        );
    }

    #[test]
    fn string_hash_seed_is_constant_across_states_in_process() {
        // luai_makeseed 常量覆盖锚（D-继承-3）：同进程两个**同时存活**的 Lua 状态
        // 对同序插入产出相同 pairs 迭代序。默认种子（time + 地址混合）下，两个
        // 共存状态的堆地址必然不同 → 种子不同 → 迭代序不同（本测试在未覆盖
        // 构建下实证失败，见验证证据）；常量覆盖下种子相同 → 迭代序相同。
        // 注意：两个 Sandbox 必须同时存活——顺序创建并即时释放会让后者复用
        // 同一堆地址，种子巧合相等，测试退化为恒真。跨进程一致性归 S12。
        let chunk = r#"
            local t = {}
            for i = 1, 40 do t["k" .. (i % 10) .. "_" .. (i * 37 % 100)] = i end
            local order = ""
            for k in pairs(t) do order = order .. k .. ";" end
            return order
        "#;
        let sa = Sandbox::new().expect("sandbox a");
        let sb_ = Sandbox::new().expect("sandbox b");
        let a: String = sa.lua().load(chunk).eval().unwrap();
        let b: String = sb_.lua().load(chunk).eval().unwrap();
        assert_eq!(
            a, b,
            "identical insertions must iterate identically across states"
        );
        assert_eq!(
            a.matches(';').count(),
            40,
            "all 40 keys must be visited exactly once"
        );
    }
}
