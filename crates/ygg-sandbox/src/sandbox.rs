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
//!   整体替换——原 globals（含 ALL_SAFE 加载的 io/string/table/math/package 等）
//!   自此不可达。新表只暴露三类面：base 纯计算子集（[`BASE_WHITELIST`]：无
//!   dofile/loadfile/load/print/rawset/collectgarbage——文件系统、任意编译与
//!   GC 干预面全部拒之门外）；`require` 受控模块面（r1：按已登记模块表装载
//!   同 Mod 的其它文件，其它一切名字可读拒绝；调用时机不设边界——实现见
//!   [`crate::modules`]，见下）；`ygg = { engine_version }` host API 占位（完整
//!   host API 落 S9/S10）。首个 host API 面刻意极小：M0 骨架期不预支 S9 的
//!   白名单决策。
//! - **require 面**（r1）：不在本文件直接实现——[`crate::modules::install`]
//!   装上「查已登记模块表 + 缓存 + 循环/深度判栈 + 副作用影子表」的装载器；
//!   模块表由驱动在入口 chunk 之前注入（[`Sandbox::modules`] 句柄），装载期
//!   与运行期（handler 内部）都能 require。空表时 `require` 对任何名字给可读
//!   拒绝（单文件 Mod 的既有行为不变）。
//! - **无燃料计量**（S34 裁定）：燃料机制整体退役——构造本体不内建任何
//!   hook，也不再接入任何计量面（原正式燃料接点随 S34 机制移除）。
//! - **确定性种子**：`luai_makeseed` 构建期常量覆盖（workspace 根
//!   `.cargo/config.toml`，D-继承-3）——本 crate 的种子锚测试实证「同进程两个
//!   Lua 状态对同序插入产出相同 `pairs` 迭代序」；跨进程一致性归 S12
//!   golden-vector。状态哈希路径依旧绝不消费 `pairs()` 序（D-继承-5 纵深）。

use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Error, Lua, LuaOptions, StdLib, Value};
use ygg_sim::error::SimError;
use ygg_sim::sim::Sim;

use crate::modules::{self, ModuleHandle};

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
const BASE_WHITELIST: &[&str] = &[
    "assert",
    "error",
    "getmetatable",
    "ipairs",
    "next",
    "pairs",
    "pcall",
    "rawequal",
    "rawget",
    "rawlen",
    "select",
    "setmetatable",
    "tonumber",
    "tostring",
    "type",
    "xpcall",
];

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
}

/// 一次宿主推进的结果（[`Sandbox::step`]）。
///
/// 只给**终值**：`tick` 是内核时钟（绝对序号，与本次推进了几 tick 无关），
/// `hash` 是该时刻的规范状态哈希（快照帧尾的 fnv1a64）。中间每 tick 的状态
/// 不返回——回放证据要的是「同一序列到同一终态」，不是逐步回执。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StepReport {
    pub tick: u64,
    pub hash: u64,
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
/// 后者的时间长度由脚本决定（所以 `ygg.sim.step` 被整个删掉）。
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
        })
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
    /// Lua 侧**没有任何推进时钟的入口**（`ygg.sim.step` 已随 `ygg.sim.*` 面删除）：
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
        for _ in 0..ticks {
            sim.step(dt)?;
        }
        Ok(StepReport {
            tick: sim.tick(),
            hash: sim.hash(),
        })
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
    /// 权限，不是脚本的权限（旧 `ygg.sim.snapshot/restore` 面已删）。
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
        for name in [
            "os",
            "io",
            "package",
            "string",
            "table",
            "math",
            "coroutine",
            "utf8",
        ] {
            let absent: bool = lua.load(format!("return {name} == nil")).eval().unwrap();
            assert!(absent, "{name} must be absent from the sandbox globals");
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
