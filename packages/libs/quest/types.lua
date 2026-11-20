-- `pkg.quest.types` —— 流程模板的编译 / 落盘 / 比较（契约 §2 模板形状 + I1）。
--
-- # 模板是数据，不是代码
--
-- 有哪些任务类型、每类几步、每步叫什么——全从 `define` 的入参来。本文件只做四件事：
-- 校验形状、规范成「名字升序的内部形」、落成扁平标量键、读回时重建对外形。
--
-- # 幂等是**零写**
--
-- 同 spec 二次 `define` 返回 `true` 且一个字节都不写（用例 q1 用写计数为证）；
-- 异 spec 不覆盖而是 `YGG-PKG-QUEST-TYPE`——覆盖等于改所有存档对同一实例的解释，
-- 正是契约 I1 要挡的事。
--
-- # 落盘形状（键名出处 = `store.K`）
--
--   quest.types                 string  类型名表（升序、换行连接）
--   quest.t.<n>.steps           number
--   quest.t.<n>.s.<i>.{id,guide,on_enter}
--   quest.t.<n>.m.names         string  meta 键名表（升序）
--   quest.t.<n>.m.<key>         string  number|string|boolean|ref
--
-- `on_enter` 的 `nil` 落成空串：KV 没有「删除」，`nil` 存不进去，空串是唯一
-- 可往返的「无」表示（读回时再还原成 `nil`，见 `read_one`）。

local S = require("pkg.quest.store")

local T = {}

--- meta 值域词表：与 `entity.md` §2 的四个类型词同一份词表（不另造第五种）。
local META_TYPES = { number = true, string = true, boolean = true, ref = true }

local function template_error(detail)
  S.fail(S.E.TEMPLATE, detail)
end

-- ── 校验与规范化 ────────────────────────────────────────────────────────────

--- 一个步骤 → 规范形 `{ id, guide, on_enter }`（`on_enter` 缺省 = `nil`）。
local function compile_step(raw, index)
  if type(raw) ~= "table" then
    template_error("steps[" .. index .. "] 必须是表，得到 " .. type(raw))
  end
  local id = raw.id
  if type(id) ~= "string" or #id == 0 then
    template_error("steps[" .. index .. "].id 必须是非空字符串")
  end
  local guide = raw.guide
  if guide == nil then
    guide = ""
  elseif type(guide) ~= "string" then
    template_error("steps[" .. index .. "].guide 必须是字符串（文本键）")
  end
  local on_enter = raw.on_enter
  if on_enter ~= nil and (type(on_enter) ~= "string" or #on_enter == 0) then
    template_error("steps[" .. index .. "].on_enter 必须是非空字符串或 nil")
  end
  return { id = id, guide = guide, on_enter = on_enter }
end

--- meta_spec → 名字升序的两份（名表 + 类型表），非词表内的类型词直接拒。
local function compile_meta(raw)
  local names = {}
  local types = {}
  if raw == nil then
    return names, types
  end
  if type(raw) ~= "table" then
    template_error("meta_spec 必须是表，得到 " .. type(raw))
  end
  for key, kind in pairs(raw) do
    if not S.key_name_ok(key) then
      template_error("meta_spec 的键 " .. tostring(key) .. " 不在合法字符集内")
    end
    if not META_TYPES[kind] then
      template_error("meta_spec[" .. key .. "] 的类型词必须是 number|string|boolean|ref")
    end
    names[#names + 1] = key
    types[key] = kind
  end
  S.sort(names)
  return names, types
end

--- 对外形 → 规范形。校验失败一律 `YGG-PKG-QUEST-TEMPLATE`（码面单一）。
function T.compile(name, raw)
  if not S.type_name_ok(name) then
    S.fail(S.E.NAME, "类型名必须匹配 ^[a-z][a-z0-9_-]*$，得到 " .. tostring(name))
  end
  if type(raw) ~= "table" then
    template_error("模板必须是表，得到 " .. type(raw))
  end
  local steps = raw.steps
  if type(steps) ~= "table" or #steps == 0 then
    template_error("steps 必须是非空数组")
  end
  local compiled = {}
  local seen = {}
  for i = 1, #steps do
    local step = compile_step(steps[i], i)
    if seen[step.id] then
      template_error("steps 里的 id 重复：" .. step.id)
    end
    seen[step.id] = i
    compiled[i] = step
  end
  local meta_names, meta_types = compile_meta(raw.meta_spec)
  return { name = name, steps = compiled, meta_names = meta_names, meta_types = meta_types }
end

--- 规范形 → 对外形（每次重建：调用方改坏自己的副本不影响注册表）。
local function external(spec)
  local steps = {}
  for i = 1, #spec.steps do
    local step = spec.steps[i]
    steps[i] = { id = step.id, guide = step.guide, on_enter = step.on_enter }
  end
  local meta = {}
  for i = 1, #spec.meta_names do
    local key = spec.meta_names[i]
    meta[key] = spec.meta_types[key]
  end
  return { steps = steps, meta_spec = meta }
end

-- ── 落盘 / 读回 ────────────────────────────────────────────────────────────

--- 写一份类型（调用方已确认「不存在或同 spec」）。
function T.write(h, spec)
  local names = S.split(S.get(h, S.K.types))
  local known = false
  for i = 1, #names do
    if names[i] == spec.name then
      known = true
    end
  end
  if not known then
    names[#names + 1] = spec.name
    S.sort(names)
    S.set(h, S.K.types, S.join(names))
  end
  S.set(h, S.type_key(spec.name, "steps"), #spec.steps)
  for i = 1, #spec.steps do
    local step = spec.steps[i]
    local prefix = "s." .. i .. "."
    S.set(h, S.type_key(spec.name, prefix .. "id"), step.id)
    S.set(h, S.type_key(spec.name, prefix .. "guide"), step.guide)
    S.set(h, S.type_key(spec.name, prefix .. "on_enter"), step.on_enter or "")
  end
  S.set(h, S.type_key(spec.name, "m.names"), S.join(spec.meta_names))
  for i = 1, #spec.meta_names do
    local key = spec.meta_names[i]
    S.set(h, S.type_key(spec.name, "m." .. key), spec.meta_types[key])
  end
  return true
end

--- 类型是否已注册（判据是 `steps` 键存在，不是名表——名表是索引不是真源）。
function T.exists(h, name)
  return S.get(h, S.type_key(name, "steps")) ~= nil
end

--- 读一份类型 → 规范形（不存在 = `nil`）。
function T.read(h, name)
  local count = S.get(h, S.type_key(name, "steps"))
  if count == nil then
    return nil
  end
  local steps = {}
  for i = 1, count do
    local prefix = "s." .. i .. "."
    local on_enter = S.get(h, S.type_key(name, prefix .. "on_enter"))
    if on_enter == "" then
      on_enter = nil
    end
    steps[i] = {
      id = S.get(h, S.type_key(name, prefix .. "id")),
      guide = S.get(h, S.type_key(name, prefix .. "guide")),
      on_enter = on_enter,
    }
  end
  local meta_names = S.split(S.get(h, S.type_key(name, "m.names")))
  local meta_types = {}
  for i = 1, #meta_names do
    local key = meta_names[i]
    meta_types[key] = S.get(h, S.type_key(name, "m." .. key))
  end
  return { name = name, steps = steps, meta_names = meta_names, meta_types = meta_types }
end

--- 读一份类型 → 对外形（不存在 = `nil`）。
function T.get(h, name)
  local spec = T.read(h, name)
  if spec == nil then
    return nil
  end
  return external(spec)
end

--- 两份规范形是否同 spec（逐字段，不是逐字节：名表已排序，顺序不参与比较）。
function T.same(a, b)
  if #a.steps ~= #b.steps or #a.meta_names ~= #b.meta_names then
    return false
  end
  for i = 1, #a.steps do
    local left, right = a.steps[i], b.steps[i]
    if left.id ~= right.id or left.guide ~= right.guide or left.on_enter ~= right.on_enter then
      return false
    end
  end
  for i = 1, #a.meta_names do
    local key = a.meta_names[i]
    if a.meta_types[key] ~= b.meta_types[key] then
      return false
    end
  end
  return true
end

--- `define` 的语义核心（幂等零写 / 异 spec 拒）。返回 `true`。
function T.define(h, name, raw)
  local spec = T.compile(name, raw)
  if T.exists(h, name) then
    if T.same(spec, T.read(h, name)) then
      return true
    end
    S.fail(S.E.TYPE, "类型 " .. name .. " 已注册为另一份 spec（不覆盖）")
  end
  return T.write(h, spec)
end

return T
