-- fixture 包根模块（`pkg.fixture`）：**纯导出面**。
--
-- 这是 packages 装载面的活体夹具（不是能力包）。它由 `openwiki/contracts/kernel/packaging.md`
-- §5 登记，显式豁免 P0「≥2 个不同游戏用点」硬闸——那道闸针对能力包；夹具的作用
-- 是钉住装载契约，不是提供能力。
--
-- 纯导出纪律：模块顶层只能 `local` / 函数定义 / `require` 其它纯模块 / 纯读。
-- 任何注册进内核的调用（`ygg.on` / `fsm.define` / `ygg.kv.set` …）会被模块顶层的
-- 影子表拒绝（`crate::modules::ShadowBanned`）——包模块与 mod 模块走同一个装载器，
-- 边界自动生效。副作用只允许出现在**被入口调用的函数里**。

local words = require("pkg.fixture.words") -- 多文件 require 的证明：子模块真的能到

local M = { VERSION = "0.1.0" }

--- 问候一个名字（`greet(" ygg ")` → `"hello, ygg"`）。
function M.greet(name)
  return "hello, " .. words.trim(tostring(name))
end

--- 大写版问候（`loud("ygg")` → `"HELLO, YGG!"`）。
function M.loud(name)
  return words.shout(M.greet(name))
end

return M
