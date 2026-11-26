//! S9 host API（M1-E 后只剩「世界 + KV + codec」三块面）。
//!
//! - **安装面**：[`install`] 在装载期把 Mod 可见的能力装到 Lua 全局面上——
//!   `ygg.kv.*`（持久 KV）、`ygg.codec.*`（原生加速面）、`world.*`（列存世界
//!   facade，装 `ygg.kv` 之后：`world.kv` 直接复用那张表）。事件面（`on / emit /
//!   narrate / choice`）随事件-信封层整体删除（M1-E，D2）：规则层按 M2 计划在
//!   `world.*` 上重建，因此本模块不再有收集缓冲、回放桥、相位与 sentinel。
//! - **状态归属**：全部共享状态经 `Rc<RefCell<HostState>>` 单线程持有——
//!   无 mlua Scope、无线程局部存储、无静态可变全局。
//! - **世界内核槽**：`Rc<RefCell<Option<Box<Sim>>>>` 由调用方交给 [`install`]
//!   （`Sandbox` 与句柄共享同一个槽），不在本模块自建：时间归宿主
//!   （[`crate::Sandbox::step`]），槽必须挂在宿主侧的 `Sandbox` 上。
//! - **诊断面**：`world.log` 进环形缓冲，只由宿主经
//!   [`HostApi::take_world_log`] drain（日志不进任何「状态 = 输出」的对账面）。
//! - **payload 双向 marshal**：Integer↔`Int`、Number↔`Float`、String↔`Str`、
//!   Boolean↔`Bool`；Lua→core 遇 NaN/±∞ 由 core 门拒绝（可读 mlua 错误）、
//!   `-0.0` 由 core 规范化；`nil` 仅在 get 类 miss 上作为 Lua `nil` 返回；
//!   table / function / 其他类型 → 可读 `unsupported payload type: {lua_type}`。

use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Error, Lua, Table};
use ygg_core::kv::KvStore;

// ---- 模块化拆分 shim（既有路径逐字节不变）----
mod kv;
mod marshal;
mod world;

use kv::*;
use marshal::*;
use world::install_world;
pub use world::*;

/// 宿主侧状态句柄（宿主与 host API 共享同一份 KV 的唯一凭据）。
pub struct HostHandles {
    pub kv: Rc<RefCell<KvStore>>,
}

impl HostHandles {
    /// 构造句柄（宿主持有同一 `Rc` 即可读回脚本写入的 KV）。
    pub fn new(kv: Rc<RefCell<KvStore>>) -> Self {
        Self { kv }
    }
}

/// 共享状态：宿主句柄 + 世界内核槽 + `world` 面的宿主侧状态。
pub struct HostState {
    handles: HostHandles,
    /// 宿主侧模拟内核实例（未注入时 `None`，`world.*` 全部抛 `YGG-SIM-UNMOUNTED`）。
    sim: Rc<RefCell<Option<Box<ygg_sim::Sim>>>>,
    /// `world` 面的宿主侧状态（kind 表 + 日志环；M1-C）。
    world: WorldState,
}

/// 宿主 API 的 Rust 侧控制柄（诊断面：world 日志 drain、KV / 内核句柄）。
pub struct HostApi {
    state: Rc<RefCell<HostState>>,
}

pub fn install(
    lua: &Lua,
    handles: HostHandles,
    sim: Rc<RefCell<Option<Box<ygg_sim::Sim>>>>,
) -> Result<HostApi, Error> {
    let state = Rc::new(RefCell::new(HostState {
        handles,
        sim,
        world: WorldState::default(),
    }));

    let ygg: Table = lua.globals().raw_get("ygg")?;

    install_kv(lua, &state, &ygg)?;

    // `ygg.codec.*`（Tier 2 pkg.codec 的原生加速面；契约 lua-api.md §3.1.4）。
    // 无状态、无实例，所以没有 `set_*` 注入面，也不需要 CLI 激活。
    crate::codec_face::install(lua)?;
    // `world.*`（M1-C）：须在 `ygg.kv` 之后——`world.kv` 直接复用那张表。
    install_world(lua, &state)?;

    Ok(HostApi { state })
}

impl HostApi {
    /// 取走 `world.log` 累积的日志行（M1-C；环形缓冲，满则丢最旧）。
    ///
    /// 只由宿主 drain：日志不进叙述流、不进事件流（混进去会让「输出 = 状态」
    /// 的对账多出一类噪声）。
    pub fn take_world_log(&self) -> Vec<String> {
        let mut state = self.state.borrow_mut();
        state.world.take_logs()
    }

    /// 取持久 KV 句柄（宿主面：读回脚本写入的 KV；KV 不是世界帧的一部分，
    /// 所以这一面不经过快照字节）。
    pub fn kv(&self) -> Rc<RefCell<KvStore>> {
        Rc::clone(&self.state.borrow().handles.kv)
    }

    /// 注入或卸载模拟内核实例（`world.*` 的宿主面）。
    ///
    /// 未注入时 `world.*` 全部抛 `YGG-SIM-UNMOUNTED`——**不是** `nil` / `false`：
    /// `nil` 与「世界是空的」不可分辨，静默降级会让 Mod 把「宿主没这个能力」读成
    /// 「世界为空」并据此写下错误状态。
    ///
    /// 具体类型而不是 trait object：只有一份实现（`ygg_sim::Sim`），`dyn` 只会给
    /// 每次调用加一层 vtable（AGENTS §1.7「能不加就不加」）。
    pub fn set_sim(&self, sim: Option<Box<ygg_sim::Sim>>) {
        *self.state.borrow().sim.borrow_mut() = sim;
    }

    /// 取模拟内核句柄（宿主层共享；宿主批量帧走这条，不经 Lua 边界）。
    pub fn sim_handle(&self) -> Rc<RefCell<Option<Box<ygg_sim::Sim>>>> {
        Rc::clone(&self.state.borrow().sim)
    }
}
