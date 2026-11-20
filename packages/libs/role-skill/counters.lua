-- `packages/libs/role-skill/counters.lua` —— 事件计数（订阅面 + 显式清零）。
--
-- 契约：`docs/design/packages/role-skill.md` §3 I5（只增）/ I6（无时钟）/ §4。
--
-- # 冷却不是时间
--
-- 包内**零时间源**：没有 `ygg.ticks`，没有 `os`（沙箱里也没有 `os`）。`ygg.ticks()`
-- 有 API 无推进者（生产路径恒 0，见 `docs/design/kernel/state-surface.md` §2.8.1），
-- 用它做冷却等于冷却永不过期。
--
-- 冷却 = **事件计数**：订阅的事件每来一次 `counters[event] + 1`，mod 用
-- `role.count(id, "turn") >= N` 表达「冷却 N 回合」。
--
-- # 只增
--
-- handler 只做 `+1`；唯一的减法是 `role.reset_counters`，而**何时清零是 mod 的语义**
-- （「每回合重置」由 mod 在回合事件里自己调）。包不订「回合」事件、不擅自重置——
-- 那会把一个游戏的回合制变成所有游戏的回合制。
--
-- # 订阅面在安装期
--
-- `ygg.on` 只在 `M.install(deps)` 里注册一次；同一个宿主面重复 install 不重复注册
-- （否则一次事件会让计数 +2，且症状是「冷却永远差一倍」）。选宿主面一换就重新订。

local store = require("pkg.role_skill.store")
local actors = require("pkg.role_skill.actors")

local fail = store.fail
local E = store.E

local M = {}

--- 已订阅的宿主面 + 事件名（换宿主面即重订；同一宿主面不重复注册）。
local subscribed_host = nil
local subscribed = {}

--- `role.count(id, event)` → 次数（纯读；无记录 / 未计数 ⇒ 0）。
function M.count(id, event)
  local record = actors.read_actor(id)
  if record == nil then
    return 0
  end
  local value = record.counters[event]
  if type(value) ~= "number" then
    return 0
  end
  return value
end

--- `role.reset_counters(id, event?)` → `true`（显式清零；带 event 清一条，不带清全部）。
---
--- 无记录视为已清零（返回 `true` 且**不写**）：清零一个不存在的东西不该产生 KV 写。
function M.reset(id, event)
  local record = actors.read_actor(id)
  if record == nil then
    return true
  end
  if event == nil then
    if next(record.counters) == nil then
      return true
    end
    record.counters = {}
    actors.write_actor(id, record)
    return true
  end
  local value = record.counters[event]
  if value == nil or value == 0 then
    return true
  end
  record.counters[event] = 0
  actors.write_actor(id, record)
  return true
end

--- 一条订阅：`"entity"` = `payload.entity`；函数 = 自己从 payload 里取 id。
local function selector_of(events, name)
  local selector = events[name]
  if type(selector) == "string" then
    if selector ~= "entity" then
      fail(E.INSTALL, "events." .. name .. " 的字符串选择器只支持 \"entity\"，得到 " .. selector)
    end
    return function(payload)
      if type(payload) ~= "table" then
        return nil
      end
      return payload.entity
    end
  end
  if type(selector) == "function" then
    return selector
  end
  fail(E.INSTALL, "events." .. name .. " 必须是 \"entity\" 或函数 (payload) -> id|nil")
end

--- handler：解析出 id → 读记录 → `counters[k] + 1` → 写回。
---
--- id 为 `nil` 直接返回（不写不发）；实体没有记录也直接返回（不凭空建记录——记录
--- 的建立是「被授予过什么」的结果，不该由一条统计事件触发）。
local function make_handler(name, resolve)
  return function(payload)
    local id = resolve(payload)
    if id == nil then
      return
    end
    local record = actors.read_actor(id)
    if record == nil then
      return
    end
    record.counters[name] = (record.counters[name] or 0) + 1
    actors.write_actor(id, record)
  end
end

--- 装配订阅面（`install({ events = { turn = "entity" | fn, ... } })`）。
function M.subscribe(events)
  if type(events) ~= "table" then
    fail(E.INSTALL, "install 的 events 必须是 { 事件名 = \"entity\" | 函数 } 表")
  end
  local host = store.host()
  if type(host.on) ~= "function" then
    fail(E.DEPS, "宿主面缺 on(name, handler)：计数订阅面装不上")
  end
  if subscribed_host ~= host then
    subscribed_host = host
    subscribed = {}
  end
  for name, _ in pairs(events) do
    if type(name) ~= "string" or name == "" then
      fail(E.INSTALL, "events 的事件名必须是非空字符串")
    end
    if not subscribed[name] then
      host.on(name, make_handler(name, selector_of(events, name)))
      subscribed[name] = true
    end
  end
  return M
end

return M
