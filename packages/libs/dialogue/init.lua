-- `pkg.dialogue` —— 实体间对话事件总线（机制层包）。契约：`openwiki/contracts/packages/dialogue.md`。
--
-- # 这个包做什么
--
-- 「某件事发生了，谁来响应」：handler 注册表（按注册序执行、第一个认领的终止链）+
-- 记忆窗口（线程轮次有界）+ 一次对话链的嵌套深度帽。它**不**建对话节点图
-- （`ygg.narrative.*` 已提供，契约 §8 反方案二）、**不**写文案、**不**新增网络出口。
--
-- # 实现纪律（沙箱方言）
--
-- 1. **标准库表可用**（M2 起）：`string` / `table` / `utf8` 已由沙箱露出，`math` 是
--    精确子集（无 `sqrt` / 超越函数）。拼接 / 切分 / 排序仍是 `store.lua` 里的手写
--    实现——改用库表属于包迁移（M2.7），不是本包的行为变更。
-- 2. **模块顶层纯**：只有 local / 常量 / 函数定义；`ygg` 访问经 `store.host()`，
--    注册（`ygg.on`）只在 `install` 里（契约 I8）。
-- 3. **KV 只收标量**：physical 形状见 `store.lua` 头注（D10）。
--
-- # 文件切分
--
--   store.lua   键名唯一出处 / 标量纪律 / 模块局部配置
--   handlers.lua  注册面（on/off/读回）+ 函数侧 REGISTRY（D3）
--   dispatch.lua  emit 的分发面：回退链、深度帽、失败事件（D8）
--   memory.lua    线程记忆与名表（D2 的 `dlg.keys`）
--
-- 本文件只做装配、转发与两个纯导出面（`frame` / `adapters`）。

local store = require("pkg.dialogue.store")
local handlers = require("pkg.dialogue.handlers")
local dispatch = require("pkg.dialogue.dispatch")
local memory = require("pkg.dialogue.memory")

local M = { VERSION = "0.1.0" }

--- 派生事件面（恰好两个名字）。
M.EVENTS = {
  "dialogue.failed",
  "dialogue.depth_capped",
}

M.ERRORS = {}
for key, code in pairs(store.E) do
  M.ERRORS[key] = code
end

--- adapter 注册表（模块局部；包**从不**调用 `adapter.send`——装配归 mod/宿主）。
local ADAPTERS = {}

--- 桥是否已装（`ygg.on` 只允许装载期一次；重复 install 不重复注册）。
local bridged = false

--- `install(deps)`：记 host / 配置（window、max_depth）/ 装 quest 事件桥。
---
--- `deps.quest` 只读它的 `EVENTS`（五个名字），**不** `require("pkg.quest")`——
--- 依赖是形状不是编译边（契约 §3 的依赖序）。缺省则不装桥（d18）。
function M.install(deps)
  if deps ~= nil and type(deps) ~= "table" then
    store.fail(store.E.INSTALL, "install 只收 table 或 nil，得到 " .. type(deps))
  end
  store.install(deps, type(deps) == "table" and deps.config or nil)
  local quest = type(deps) == "table" and deps.quest or nil
  if quest ~= nil then
    if type(quest) ~= "table" or type(quest.EVENTS) ~= "table" then
      store.fail(store.E.INSTALL, "deps.quest 必须是有 EVENTS 表的模块表")
    end
    if not bridged then
      local h = store.host()
      for i = 1, #quest.EVENTS do
        local name = quest.EVENTS[i]
        h.on(name, function(payload)
          return M.emit(name, 0, 0, { text_key = name, quest_payload = payload })
        end)
      end
      bridged = true
    end
  end
  return M
end

--- `on(type, spec)` → `true`（同名重复注册 = `-DUP`）。
function M.on(type_name, spec)
  return handlers.on(store.host(), type_name, spec)
end

--- `off(type, name)` → `boolean`（是否真的摘掉一个）。
function M.off(type_name, name)
  return handlers.off(store.host(), type_name, name)
end

--- `emit(type, source, target, payload?)` → `{ handled, by, depth }`。
function M.emit(type_name, source, target, payload)
  return dispatch.emit(store.host(), type_name, source, target, payload)
end

--- `frame(type, source, target, payload?)` → 消息帧（纯函数）。
function M.frame(type_name, source, target, payload)
  return dispatch.frame(type_name, source, target, payload)
end

--- `say(thread_key, text)` → `true`（`who = 0` 表示系统；受 `window` 约束）。
function M.say(thread_key, text)
  memory.append(store.host(), thread_key, 0, text, store.CONFIG.window)
  return true
end

--- `history(thread_key)` → 轮次副本。
function M.history(thread_key)
  return memory.history(store.host(), thread_key)
end

--- 一个 type 的 handler 列表（注册序；自测与 mod 的只读面）。
function M.handlers(type_name)
  return handlers.list(store.host(), type_name)
end

--- 线程键名表（升序）。
function M.threads()
  return memory.threads(store.host())
end

--- `register_adapter(name, adapter)` → `true`（`adapter = { send, health }`）。
---
--- 只进模块局部表：adapter 含函数，落不进 KV（同 D3 的理由）；包也从不驱动它。
function M.register_adapter(name, adapter)
  if not store.key_name_ok(name) then
    store.fail(store.E.TYPE, "adapter 名不在合法字符集内：" .. tostring(name))
  end
  if type(adapter) ~= "table" or type(adapter.send) ~= "function" then
    store.fail(store.E.TYPE, "adapter 必须是含 send 函数的表")
  end
  if adapter.health ~= nil and type(adapter.health) ~= "function" then
    store.fail(store.E.TYPE, "adapter.health 必须是函数或 nil")
  end
  if ADAPTERS[name] ~= nil then
    store.fail(store.E.DUP, "adapter 重名注册：" .. name)
  end
  ADAPTERS[name] = adapter
  return true
end

--- `adapters()` → 名字升序副本（包不持有 adapter 的调用权）。
function M.adapters()
  local out = {}
  for name in pairs(ADAPTERS) do
    out[#out + 1] = name
  end
  store.sort(out)
  return out
end

--- `keys()` → 本包拥有的 KV 键（升序；`codec.export({ keys = dialogue.keys() })`）。
function M.keys()
  local h = store.host()
  local out = {}
  local function add(key)
    if store.get(h, key) ~= nil then
      out[#out + 1] = key
    end
  end
  add(store.K.seq)
  add(store.K.types)
  add(store.K.threads)
  local types = handlers.types(h)
  for i = 1, #types do
    local type_name = types[i]
    add(store.handler_key(type_name, "names"))
    local specs = handlers.list(h, type_name)
    for j = 1, #specs do
      local prefix = "n." .. specs[j].name .. "."
      add(store.handler_key(type_name, prefix .. "kind"))
      add(store.handler_key(type_name, prefix .. "opts"))
      for key in pairs(specs[j].opts) do
        add(store.handler_key(type_name, prefix .. "opt." .. key))
      end
    end
  end
  local threads = memory.threads(h)
  for i = 1, #threads do
    local thread = threads[i]
    local count = store.get(h, store.thread_key(thread, "turns"))
    if count ~= nil then
      add(store.thread_key(thread, "turns"))
      for t = 1, count do
        add(store.thread_key(thread, "t." .. t .. ".who"))
        add(store.thread_key(thread, "t." .. t .. ".text"))
        add(store.thread_key(thread, "t." .. t .. ".seq"))
      end
    end
  end
  store.sort(out)
  return out
end

return M
