-- `packages/libs/procgen/init.lua` —— 包根模块 `pkg.procgen`：**纯导出面**。
--
-- 契约：`docs/design/packages/procgen.md`。顶层只有 `local` / `require` / 函数定义
-- （I7）；注册与状态改动一律在 `M.install(deps)` 或调用期。
--
-- 本文件是包的**面**，实现分在四个子模块里：
--   `pkg.procgen.rng`      命名流派生 + xoshiro128** 取数面
--   `pkg.procgen.registry` defs / algo 注册表 + `run`
--   `pkg.procgen.digest`   `hash`（FNV-1a 64 over `codec.encode`）
--   `pkg.procgen.algos.*`  自带生成器（terrain / layout）
--
-- 沙箱纪律：宿主沙箱只暴露白名单 globals，`math` / `string` / `table` 三张表都
-- 不可达（`crates/ygg-sandbox/src/sandbox.rs`）。本包因此不用它们。

local rng = require("pkg.procgen.rng")

local M = { VERSION = "0.1.0" }

--- 错误码面（包内唯一一份住在 `rng.E`，这里只做对外副本）。
M.ERRORS = {}
for key, code in pairs(rng.E) do
  M.ERRORS[key] = code
end

--- `procgen.rng(seed, stream?, state?)` → 命名流句柄（I2：一条流一份状态）。
function M.rng(seed, stream, state)
  return rng.new(seed, stream, state)
end

--- `procgen.state(rng)` → 四整数快照（存档续生成的入参）。
function M.state(handle)
  return rng.state(handle)
end

--- `procgen.next32(rng)` → 一个 32 位无符号整数（原语面；KAT 的直接对象）。
function M.next32(handle)
  return rng.next32(handle)
end

return M
