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

local pkg_quest = require("pkg.quest")
local simframe = require("src.simframe")

local M = { VERSION = "1.0.0" }

local DAY_KEY = "sg.day"
local STEP_CODE = "YGG-SIM-UNMOUNTED"

local function host()
  return ygg
end

--- 名册规模（world.json 的 npcs+clerks+items；装载期数据直读，不经 world 模块
--- ——clock 不依赖业务模块，防 require 环）。
local function roster_count()
  local world = host().data("ygg:superagent/world")
  return #world.npcs + #world.clerks + #world.items
end

--- 在办委托数（quest 状态直读；前缀 sg.comms 名表归 quests.lua 单写者，此处只读）。
local function open_count()
  local h = host()
  local raw = h.kv.get("sg.comms")
  if type(raw) ~= "string" or #raw == 0 then
    return 0
  end
  local n = 0
  for key in (raw .. "\n"):gmatch("([^\n]*)\n") do
    if #key > 0 then
      local id = h.kv.get("sg.comm." .. key)
      local inst = id ~= nil and pkg_quest.get(id) or nil
      if inst ~= nil and inst.state == "open" then
        n = n + 1
      end
    end
  end
  return n
end

--- 进程内首拍：把 sim 世界重建到「名册 + 在办」的规模。
---
--- sim 是进程内状态（egg 每进程以常量种子重建，世界只在构造它的进程里存在），
--- 跨进程的规模真源是 KV；每个进程的第一拍据此重建，之后 settle 的 despawn
--- 才有实体可注销——sim.hash 因此携带游戏进度（tick + 世界规模），重放断言
--- 不是常数。
local function ensure_world(h)
  -- sim 面没有 tick 键（10 键：step/hash/snapshot/restore/批式三面/status/trace/
  -- replay）——tick 经 status() 读（两面同形，selftest mock 与真内核一致）。
  local status = h.sim.status()
  if type(status) == "table" and (status.tick or 0) ~= 0 then
    return nil
  end
  local rows = roster_count() + open_count()
  local report = h.sim.spawn_batch(simframe.spawn_entities(rows))
  h.kv.set("sg.sim_top", tostring(report.next_id or rows))
  return rows
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
  local ok, err = pcall(function()
    ensure_world(h)
    h.sim.step(1)
  end)
  local day = (h.kv.get(DAY_KEY) or 0) + 1
  h.kv.set(DAY_KEY, day)
  if not ok then
    h.narrate("[SIM-UNMOUNTED] 业务钟降级：本步未进模拟内核（宿主未挂 sim 或世界重建失败），KV 计日照走 day="
      .. tostring(day) .. "；宿主面细节：" .. tostring(err))
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
