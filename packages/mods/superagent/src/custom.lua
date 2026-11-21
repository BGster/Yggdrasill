-- `src/custom.lua` —— 自定义任务：信封模板描述符 → quest.define 翻译器。
--
-- # 能力出口（计划 §7 E2）
--
-- ① blocked：无翻译器 → 每个自定义任务都要改 mod 代码；
-- ② symptom：两任务两段复制粘贴 Lua；
-- ③ confirmation：本文件 + `tests/test-quests.lua` 的翻译器用例（合法/重定义/
--    非法字符集/坏段各一）。
--
-- # 描述符文法（quest.custom 信封的 text 字段，单行）
--
--   <type-name>|<step-id>:<guide>|<step-id>:<guide>|…
--
--   * type-name：`^[a-z][a-z0-9-]{2,31}$`（与包侧类型名同字符集；带 custom-
--     前缀约定但不强制，冲突由 quest.define 的同 spec 幂等/异 spec 报错兜底）；
--   * step 1..=8 个，step-id `^[a-z][a-z0-9-]{1,23}$` 且不得重复；
--   * guide 非空、≤64 字节、不含换行（KV 标量纪律）。
--
-- 翻译成功 = `quest.define(name, {steps})`（同 spec 幂等零写；异 spec 报
-- `-TYPE` → 调用方记 agent.error）+ 返回步骤数。运行时定义是包面已核实能力
-- （`types.define(store.host(),…)`）。

local pkg_quest = require("pkg.quest")

local M = { VERSION = "1.0.0" }

local E_FORMAT = "YGG-MOD-CUSTOM-FORMAT"

local function host()
  return ygg
end

--- 名字字符集：^[a-z][a-z0-9-]*$，长度 [min, max]。
local function name_ok(name, min, max)
  if type(name) ~= "string" or #name < min or #name > max then
    return false
  end
  local head = name:byte(1)
  if head < 97 or head > 122 then
    return false
  end
  for i = 2, #name do
    local b = name:byte(i)
    if not ((b >= 97 and b <= 122) or (b >= 48 and b <= 57) or b == 45) then
      return false
    end
  end
  return true
end

--- 切分一行（竖线段；沙箱方言手写 split）。
local function split_pipe(text)
  local out = {}
  for seg in (text .. "|"):gmatch("([^|]*)|") do
    out[#out + 1] = seg
  end
  return out
end

--- `parse(descriptor)` → `{name=…, steps={{id,guide},…}}`；非法即 error（码面
--- `YGG-MOD-CUSTOM-FORMAT`，细节带段号）。
function M.parse(descriptor)
  if type(descriptor) ~= "string" or #descriptor == 0 then
    error(E_FORMAT .. ": 描述符必须是非空字符串，得到 " .. type(descriptor), 0)
  end
  if descriptor:find("\n", 1, true) ~= nil then
    error(E_FORMAT .. ": 描述符必须单行（含换行）", 0)
  end
  if #descriptor > 512 then
    error(E_FORMAT .. ": 描述符超 512 字节上限", 0)
  end
  local segments = split_pipe(descriptor)
  local name = segments[1]
  if not name_ok(name, 3, 32) then
    error(E_FORMAT .. ": 类型名不合法（^[a-z][a-z0-9-]{2,31}$）：" .. tostring(name), 0)
  end
  if #segments < 2 then
    error(E_FORMAT .. ": 至少一个步骤段（<step-id>:<guide>）", 0)
  end
  if #segments > 9 then
    error(E_FORMAT .. ": 步骤最多 8 个，实得 " .. tostring(#segments - 1), 0)
  end
  local steps = {}
  local seen = {}
  for i = 2, #segments do
    local seg = segments[i]
    local colon = seg:find(":", 1, true)
    if colon == nil then
      error(E_FORMAT .. ": 第 " .. tostring(i - 1) .. " 步缺冒号（<step-id>:<guide>）：" .. seg, 0)
    end
    local id = seg:sub(1, colon - 1)
    local guide = seg:sub(colon + 1)
    if not name_ok(id, 2, 24) then
      error(E_FORMAT .. ": 第 " .. tostring(i - 1) .. " 步 id 不合法：" .. tostring(id), 0)
    end
    if seen[id] then
      error(E_FORMAT .. ": 步骤 id 重复：" .. id, 0)
    end
    seen[id] = true
    if type(guide) ~= "string" or #guide == 0 or #guide > 64 then
      error(E_FORMAT .. ": 第 " .. tostring(i - 1) .. " 步 guide 须非空且 ≤64 字节", 0)
    end
    steps[#steps + 1] = { id = id, guide = guide }
  end
  return { name = name, steps = steps }
end

--- `define(descriptor)` → `{name, steps=n}`；成功后类型可 `quest.open`。
---
--- 异 spec 重定义：包面报 `YGG-PKG-QUEST-TYPE` 原样上浮（调用方记 agent.error）。
function M.define(descriptor)
  local parsed = M.parse(descriptor)
  local spec_steps = {}
  for i = 1, #parsed.steps do
    spec_steps[i] = { id = parsed.steps[i].id, guide = parsed.steps[i].guide }
  end
  pkg_quest.define(parsed.name, { steps = spec_steps })
  -- 已定义类型账（换行名表，读数用）。
  local h = host()
  local existing = h.kv.get("sg.customs")
  local names = {}
  if type(existing) == "string" and #existing > 0 then
    for key in (existing .. "\n"):gmatch("([^\n]*)\n") do
      if #key > 0 then
        names[#names + 1] = key
      end
    end
  end
  local present = false
  for i = 1, #names do
    if names[i] == parsed.name then
      present = true
      break
    end
  end
  if not present then
    names[#names + 1] = parsed.name
  end
  for i = 1, #names do
    local min = i
    for j = i + 1, #names do
      if names[j] < names[min] then
        min = j
      end
    end
    if min ~= i then
      names[i], names[min] = names[min], names[i]
    end
  end
  local out = ""
  for i = 1, #names do
    if i > 1 then
      out = out .. "\n"
    end
    out = out .. names[i]
  end
  h.kv.set("sg.customs", out)
  return { name = parsed.name, steps = #parsed.steps }
end

--- `customs()` → 已定义自定义类型名表（升序）。
function M.customs()
  local out = {}
  local existing = host().kv.get("sg.customs")
  if type(existing) == "string" and #existing > 0 then
    for key in (existing .. "\n"):gmatch("([^\n]*)\n") do
      if #key > 0 then
        out[#out + 1] = key
      end
    end
  end
  return out
end

return M
