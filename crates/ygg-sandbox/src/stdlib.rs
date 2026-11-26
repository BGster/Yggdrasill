//! S10 stdlib 版本面（M1-E 后只剩版本号）。
//!
//! stdlib 的 Lua 内容原本是 `init.lua`（命名空间声明）→ `fsm.lua`（状态机）→
//! `narrative.lua`（叙事路由）三个文件，三者**全部**建立在 `ygg` 的 `on` / `ygg.emit`
//! 之上（注册 handler、派生事件）。事件-信封层删除后（M1-E，D2），这些文件连同
//! `include_dir` 嵌入面一并退役：没有 handler 注册表，状态机与叙事路由没有执行
//! 面。它们按 M2 计划在 `world.*` 上重建（`world.fsm` 保留能力、`world.narrative`
//! 不再重建）。
//!
//! 保留的是**版本契约**：`ygg.stdlib_version` 与 `ygg.engine_version` 同源
//! （[`ENGINE_VERSION`]），存档 / 诊断报告里的「引擎侧版本」只有这一个来源。

use mlua::{Error, Lua, Table};

use crate::sandbox::ENGINE_VERSION;

/// stdlib 版本（与引擎同源：`ygg.stdlib_version == ygg.engine_version`）。
pub fn stdlib_version() -> &'static str {
    ENGINE_VERSION
}

/// 在 Lua 状态上安装 stdlib 面：注入 `ygg.stdlib_version`。
///
/// 接线序约束：`ygg` 表就位之后（[`crate::sandbox::Sandbox::new`] 建表）、Mod
/// chunk 之前——Mod 顶层即可读版本号。
pub fn install(lua: &Lua) -> Result<(), Error> {
    let ygg: Table = lua.globals().raw_get("ygg")?;
    ygg.raw_set("stdlib_version", stdlib_version())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::Sandbox;

    #[test]
    fn stdlib_exposes_version_matching_engine() {
        let sb = Sandbox::new().expect("sandbox construction");
        install(sb.lua()).expect("stdlib install");
        let version: String = sb
            .lua()
            .load("return ygg.stdlib_version")
            .eval()
            .expect("version probe");
        assert_eq!(version, stdlib_version());
        assert_eq!(version, ENGINE_VERSION);
    }
}
