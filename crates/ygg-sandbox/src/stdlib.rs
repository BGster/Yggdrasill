//! S10 stdlib 嵌入（Roblox 内建库 / Neovim core-baked 先例）：构建期把
//! `stdlib/` 目录整体嵌入二进制，装载期按**语义序**执行。
//!
//! - **include_dir 嵌入**：[`STDLIB`] 为编译期嵌入的目录快照——运行期零
//!   文件系统 IO，嵌入内容随引擎版本原子发行（stdlib 与引擎同版本，杜绝
//!   「引擎新 / 脚本旧」的漂移面）；
//! - **显式装载序**（[`STDLIB_ORDER`]）：不用目录枚举——**顺序是语义**：
//!   `init.lua`（命名空间声明）→ `fsm.lua`（状态机）→ `narrative.lua`
//!   （叙事路由，消费 init 声明的 `ygg.narrative`）。枚举序受文件系统影响
//!   且丢失语义边界，故装载序以 Rust 常量单点收口；
//! - **版本暴露**：[`install`] 把 `ygg.stdlib_version` 注入 ygg 面，取值
//!   与 `ygg.engine_version` 同源（[`ENGINE_VERSION`]）——S5 存档
//!   `engine_version` 双版本号的引擎侧同一口径；
//! - **错误面**：嵌入文件缺失 / 非 UTF-8 属**构建期 bug**（panic——嵌入
//!   面在 `cargo build` 时已定型，运行期不可能出现）；Lua 执行错误（如
//!   stdlib 自身语法/逻辑缺陷）为可读 mlua 错误向上传播；
//! - **接线序**（S10 锁定）：`install` 必须在 host API 安装**之后**（依赖
//!   `ygg` 表与 `ygg.on` 装载期注册面）、Mod chunk **之前**（stdlib 先于
//!   Mod 完成 ygg.on 注册——注册序即派发序，stdlib 路由先于 Mod handler）。
//!
//! 装载序全景（[`crate::driver::KernelFacade::new`] 承载）：
//! `Sandbox::new` → `fuel::install` → `inject::install` →
//! `host_api::install` → `stdlib::install` → Mod chunk → `host.seal` →
//! `register_into`。

use include_dir::include_dir;
use mlua::{Lua, Table};

use crate::sandbox::ENGINE_VERSION;

/// 构建期嵌入的 stdlib 目录快照（`$CARGO_MANIFEST_DIR` 相对定位，随 crate
/// 移动而正确解析；目录内容参与编译期指纹，无运行期查找）。
static STDLIB: include_dir::Dir = include_dir!("$CARGO_MANIFEST_DIR/stdlib");

/// stdlib 装载序（语义性顺序，显式常量——不用目录枚举，理由见模块文档）。
const STDLIB_ORDER: [&str; 3] = ["init.lua", "fsm.lua", "narrative.lua"];

/// stdlib 版本（与引擎同源：`ygg.stdlib_version == ygg.engine_version`，
/// S5 存档 `engine_version` 同一口径）。
pub fn stdlib_version() -> &'static str {
    ENGINE_VERSION
}

/// 在 Lua 状态上安装 stdlib：注入 `ygg.stdlib_version` 后按
/// [`STDLIB_ORDER`] 逐文件执行（chunk name 形如 `@stdlib/fsm.lua`，
/// traceback 定位可读）。嵌入文件缺失 / 非 UTF-8 为构建期 bug（panic）；
/// Lua 侧错误以可读 mlua 错误传播。
///
/// 接线序约束：host API 安装之后、Mod chunk 之前（见模块文档）。
pub fn install(lua: &Lua) -> Result<(), mlua::Error> {
    let ygg: Table = lua.globals().raw_get("ygg")?;
    ygg.raw_set("stdlib_version", stdlib_version())?;
    for name in STDLIB_ORDER {
        let file = STDLIB
            .get_file(name)
            .unwrap_or_else(|| panic!("stdlib bug: file `{name}` missing from embedded stdlib"));
        let source = file
            .contents_utf8()
            .unwrap_or_else(|| panic!("stdlib bug: embedded file `{name}` is not valid UTF-8"));
        lua.load(source)
            .set_name(format!("@stdlib/{name}"))
            .exec()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fuel::{self, HookFuelMeter};
    use crate::host_api::{self, HostHandles};
    use crate::inject;
    use crate::sandbox::Sandbox;
    use std::cell::RefCell;
    use std::rc::Rc;
    use ygg_core::bus::HandlerRegistry;
    use ygg_core::entity::{EntityStore, Value};
    use ygg_core::event::Event;
    use ygg_core::kv::KvStore;
    use ygg_core::transition::{run_transition, TransitionResult};

    /// stdlib.rs 单测 harness：完整 S10 装载序（host → stdlib → Mod chunk →
    /// seal → register_into）——不依赖 driver，直接以 sealed 注册表 +
    /// `run_transition` 驱动（narrative 路由的转换内派生事件验证面）。
    struct Rig {
        _sandbox: Sandbox,
        _host: host_api::HostApi,
        lua: Lua,
        registry: HandlerRegistry,
        meter: HookFuelMeter,
        store: EntityStore,
    }

    fn rig(mod_source: &str) -> Rig {
        let sandbox = Sandbox::new().expect("sandbox construction");
        let lua = sandbox.lua().clone();
        let meter = fuel::install(&lua, 100_000).expect("fuel install");
        let _inject = inject::install(&lua).expect("determinism install");
        let host = host_api::install(
            &lua,
            HostHandles {
                kv: Rc::new(RefCell::new(KvStore::new())),
                entities: Rc::new(RefCell::new(EntityStore::new())),
            },
        )
        .expect("host api install");
        install(&lua).expect("stdlib install");
        lua.load(mod_source).exec().expect("mod chunk");
        host.seal(&lua).expect("seal");
        let mut registry = HandlerRegistry::new();
        host.register_into(&mut registry, "mod");
        Rig {
            _sandbox: sandbox,
            _host: host,
            lua,
            registry,
            meter,
            store: EntityStore::new(),
        }
    }

    #[test]
    fn stdlib_loads_and_exposes_version() {
        let rig = rig("-- empty mod");
        let exposed: String = rig
            .lua
            .load("return ygg.stdlib_version")
            .eval()
            .expect("stdlib_version reachable");
        assert_eq!(exposed, stdlib_version());
        assert_eq!(exposed, ENGINE_VERSION);
        // 命名空间已由 init.lua 声明（fsm / narrative 可达且为表）。
        let (fsm_kind, narrative_kind): (String, String) = rig
            .lua
            .load("return type(ygg.fsm), type(ygg.narrative)")
            .eval()
            .expect("namespace probe");
        assert_eq!(
            (fsm_kind.as_str(), narrative_kind.as_str()),
            ("table", "table")
        );
    }

    #[test]
    fn narrative_goto_routes_derived_event_within_transition() {
        let mut rig = rig(r#"
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
        "#);
        let external = Event::external("event.enter", Value::Int(0)).expect("external event");
        let result: TransitionResult =
            run_transition(&mut rig.store, &mut rig.registry, &mut rig.meter, external)
                .expect("transition succeeds");
        // 路由序：enter → goto intro（派生 1）→ intro body 叙述并 goto
        // scene（派生 2）→ scene body 叙述。FIFO 级联序 = 叙述序。
        assert_eq!(result.narrative(), ["node:intro", "node:scene"]);
        assert_eq!(result.events_derived(), 2);
        assert_eq!(result.events_processed(), 3);
        assert_eq!(result.events_dropped(), 0);
        assert!(result.pending_choice().is_none());
    }
}
