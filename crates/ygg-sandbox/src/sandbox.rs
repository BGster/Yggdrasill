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
//!   GC 干预面全部拒之门外）；`require` 拒绝桩（可读报错——Verification
//!   Notes「非裸 nil-call」要求；真正的 require 语义随 S9/S10 host API/装载面
//!   回来）；`ygg = { engine_version }` host API 占位（完整 host API 落
//!   S9/S10）。首个 host API 面刻意极小：M0 骨架期不预支 S9 的白名单决策。
//! - **燃料计量**（D-继承-2 / D-B1）：正式燃料落 [`crate::fuel`] 模块
//!   （`set_hook` 指令计数 + 每转换共享预算），经 `fuel::install(&Lua, budget)`
//!   在沙箱构造后接入——构造本体不内建任何 hook（S8 amendment：S7 的临时
//!   10M hook 是先行证明，正式燃料落地即移除，双计量防线归一）。
//! - **确定性种子**：`luai_makeseed` 构建期常量覆盖（workspace 根
//!   `.cargo/config.toml`，D-继承-3）——本 crate 的种子锚测试实证「同进程两个
//!   Lua 状态对同序插入产出相同 `pairs` 迭代序」；跨进程一致性归 S12
//!   golden-vector。状态哈希路径依旧绝不消费 `pairs()` 序（D-继承-5 纵深）。

use mlua::{Error, Lua, LuaOptions, StdLib, Value};

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
}

impl Sandbox {
    /// 按默认约束构造沙箱：stdlib 裁剪 + 64 MB 内存上限 + 白名单环境
    /// （燃料计量经 `fuel::install` 在构造后接入）。
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

        // ④ require 拒绝桩：可读报错（拒绝面在本 crate 单点收口，后续
        //    require 语义若回来也只改此处）。
        let require_stub = lua.create_function(|_, module: String| -> Result<(), Error> {
            Err(Error::runtime(format!(
                "sandbox denied: module '{module}' is not available \
                 (the ygg sandbox whitelists the host API; module loading is not permitted)"
            )))
        })?;
        fresh.raw_set("require", require_stub)?;

        // ⑤ host API 占位：ygg = { engine_version }（完整 host API 落 S9/S10）。
        let ygg = lua.create_table()?;
        ygg.raw_set("engine_version", ENGINE_VERSION)?;
        fresh.raw_set("ygg", ygg)?;

        lua.set_globals(fresh)?;
        Ok(Self { lua })
    }

    /// 内部 Lua 句柄（后续 slice 的 fuel/inject/host 面经此接线；
    /// 状态哈希等消费面在 S10+ 才出现，本 slice 不再扩 API）。
    pub fn lua(&self) -> &Lua {
        &self.lua
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
