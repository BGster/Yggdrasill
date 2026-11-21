-- `src/clock.lua` —— 业务钟：sim tick 换算 + 降级诚实账（模块顶层纯导出）。
--
-- # 设计
--
-- 一个业务日 = `ygg.sim.step(1)` 一次 + KV 计日 +1。sim 是**进程内**状态
-- （egg 每次进程以常量种子重建），所以「第几天」的真源是 KV `sg.day`；
-- sim.step 的职责是让 Tier 1 状态哈希随游戏进度移动——「同信封序列重放
-- sim.hash 逐字节相等」（契约 §五 P4）因此断的是真状态，不是常数。
--
-- # 降级诚实账（工具桥既有先例：「[VFS LEDGER ONLY]」）
--
-- 裸 `KernelFacade`（selftest / playtest）不注 sim：`ygg.sim.step` 抛
-- `YGG-SIM-UNMOUNTED`。此时业务钟**不许静默假装走了**：KV 计日照走（游戏
-- 语义还在），但必须叙述一行 `[SIM-UNMOUNTED]` 明示本步没进模拟内核——
-- 两种宿主形态下 KV 与叙述必须一致，读数的人不会把降级当满血。
--
-- # 纪律
--
-- 模块顶层纯；`ygg` 只在函数体内经 `host()` 单点取。所有键前缀 `sg.`
-- （本 mod 命名空间，单写者）。

local M = { VERSION = "1.0.0" }

local DAY_KEY = "sg.day"
local STEP_CODE = "YGG-SIM-UNMOUNTED"

local function host()
  return ygg
end

--- 当前业务日（0 = 开局钟未走）。
function M.day()
  return host().kv.get(DAY_KEY) or 0
end

--- `advance(step_ok_text)` → `day, degraded`：走一个业务日。
---
--- `sim.step` 成功 → KV 计日 +1，返回 `(day, false)`；
--- 未挂 sim → KV 计日仍 +1（游戏语义不停摆），叙述降级行，返回 `(day, true)`。
--- dt 固定 1（定点闸门收整数）；步长调参走 world.json 的 tuning（此处只走一步）。
function M.advance()
  local h = host()
  local ok = pcall(function()
    h.sim.step(1)
  end)
  local day = (h.kv.get(DAY_KEY) or 0) + 1
  h.kv.set(DAY_KEY, day)
  if not ok then
    h.narrate("[SIM-UNMOUNTED] 业务钟降级：本步未进模拟内核（宿主未挂 sim），KV 计日照走 day=" .. tostring(day))
  end
  return day, not ok
end

--- `sim_hash()` → 16 位十六进制状态哈希（裸 KernelFacade 下返回 "unmounted"）。
---
--- 断言面用：回执 meta 里带出去，重放对账逐字节比。
function M.sim_hash()
  local h = host()
  local ok, hash = pcall(function()
    return h.sim.hash()
  end)
  if ok then
    return hash
  end
  return "unmounted"
end

--- `sim_status_line()` → `tick=<n>;hash=<hex>` 的一行读数（降级态明示）。
function M.sim_status_line()
  local h = host()
  local ok, status = pcall(function()
    return h.sim.status()
  end)
  if ok and type(status) == "table" then
    return "tick=" .. tostring(status.tick) .. ";hash=" .. tostring(status.hash)
  end
  return "tick=unmounted;hash=unmounted"
end

--- 是否降级形态（selftest / 裸 KernelFacade 探测；开局铺场用）。
function M.sim_mounted()
  local ok = pcall(function()
    host().sim.status()
  end)
  return ok
end

return M
