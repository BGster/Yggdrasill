-- `packages/libs/textframe/init.lua` —— 包根模块 `pkg.textframe`：**纯导出面**。
--
-- 契约：`openwiki/contracts/packages/textframe.md`。顶层只有 `local` / `require` / 函数与
-- 常量定义（I7）；本包没有装配期状态，`install` 只为夹具的同构调用面而存在。
--
-- # 分工
--
--   `pkg.textframe.grid`  全部实现：建帧 / 单格写 / 分层叠加 / 行装配 / 块拼接 + 错误码
--   `pkg.textframe`（本文件）  面：五原语 + `install` / `perf_keys` / `ERRORS` / `VERSION`
--
-- # 沙箱纪律
--
-- 宿主沙箱只暴露白名单 globals，`math` / `string` / `table` 三张表都不可达
-- （`crates/ygg-sandbox/src/sandbox.rs` 的 `BASE_WHITELIST`）。本包因此不用它们，
-- 也不迭代 `pairs`：顺序只来自数值下标（I2）。

local grid = require("pkg.textframe.grid")

local M = { VERSION = "0.1.0" }

--- 错误码面：四键逐键**显式**副本（不用 `pairs` 拷贝——I2 只许数值 `for`）。
---
--- 码字面量单点在 `grid.E`；这里只搬键，不重写字符串——两处各写一遍就是分叉起点。
M.ERRORS = {
  PARAM = grid.E.PARAM,
  RANGE = grid.E.RANGE,
  CELL = grid.E.CELL,
  SHAPE = grid.E.SHAPE,
}

--- 装配：无依赖、无状态，幂等（夹具 `PACKAGES[i].install()` 无条件调用）。
---
--- 参数被忽略是**契约**不是疏忽：本包不接宿主面（不写 KV、不发事件、不注册 handler），
--- 也没有需要注入的依赖（§3 I8）。
function M.install()
  return M
end

--- 自省面：§5 会打印的 `perf.textframe.*` 键名（不含前缀）。自测按它逐个打印，
--- 冒烟按它逐个核对——「七键齐备」因此是机检，不靠人抄。
local PERF_KEYS = {
  "load_ms",
  "put_ns",
  "blit_us",
  "compose_us",
  "throughput_fps",
  "frame_kib",
  "leak_gc_pct",
  "leak_100_equal",
  "vs_naive",
}

function M.perf_keys()
  local out = {}
  for i = 1, #PERF_KEYS do
    out[i] = PERF_KEYS[i]
  end
  return out
end

M.frame = grid.frame
M.put = grid.put
M.blit = grid.blit
M.to_lines = grid.to_lines
M.stack = grid.stack

return M
