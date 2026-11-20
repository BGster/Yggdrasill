-- `pkg.quest.flow` —— 实例的原语（open / advance / jump / abandon / assign）与读数。
--
-- # 迁移是**数据**，不是 if 链
--
-- `M.TRANSITIONS` 是一张纯导出的查表：`state × 原语 → { 迁移目标, 事件名 }`。
-- 终态没有行（`done` / `abandoned` 不在表里），于是「终态后一切原语被拒」不是
-- 十处 `if` 各自记得，而是查表一次查不到（契约 I3）。
--
-- # 事件面恰好五个名字（契约 I6）
--
-- `ygg.emit` 的载荷只收标量（`host_api.rs` 的 `value_from_lua`），所以形状必须
-- 显式定形（对齐 `entity.md` §2 的「事件载荷」小节；回写 D11）：
--
--   quest.opened      "<id>|<type>"
--   quest.advanced    "<id>|<step>"        推进后的步下标
--   quest.completed   "<id>"
--   quest.abandoned   "<id>|<reason>"
--   quest.jumped      "<id>|<from>|<to>"
--
-- `on_enter` **不自动 emit**：它只是 `quest.step()` 的读数，由 mod / dialogue 消费。
-- 自动发它会让「名字集合恰好相等」这条断言（q12）失去意义。
--
-- # meta 不参与流程（契约 I5）
--
-- `meta` 只做两件事：`open` 时按模板的 `meta_spec` 校验、随实例存放。`advance`
-- 只看 `step` 与模板——用例 q10 用「塞三个任意 meta 键后推进结果逐字段相同」钉它。

local S = require("pkg.quest.store")
local T = require("pkg.quest.types")

local F = {}

-- ── 迁移表（数据；终态缺席 = 一切原语被拒）──────────────────────────────────

F.TRANSITIONS = {
  open = {
    advance = { finish = "done", event = "quest.advanced", finish_event = "quest.completed" },
    jump = { event = "quest.jumped" },
    abandon = { to = "abandoned", event = "quest.abandoned" },
    assign = {},
  },
}

--- 取一次迁移的规则行（终态 / 未知原语 = `YGG-PKG-QUEST-TERMINAL`）。
local function rule_of(state, verb)
  local row = F.TRANSITIONS[state]
  if row == nil then
    S.fail(S.E.TERMINAL, "任务处于终态 " .. tostring(state) .. "，" .. verb .. " 被拒")
  end
  local rule = row[verb]
  if rule == nil then
    S.fail(S.E.TERMINAL, verb .. " 在状态 " .. tostring(state) .. " 下不可用")
  end
  return rule
end

-- ── 实例：读 / 写 (KV 形状见 store.lua 头注) ────────────────────────────────

--- 读一个实例 → 表（不存在 = `nil`）。`giver` / `taker` 的 0 还原成 `nil`。
---
--- meta 的**名字表在实例上**（`quest.inst.<id>.m.names`），不向模板要：读面一旦
--- 依赖模板，`get` / `advance` / `open_ids` 全部退化回 `O(步数)`——而契约 §5 把
--- 「按 id 查找 / 更新」钉在 `O(1)`。多一枚键换 O(1) 是这笔账的解法。
local function read_inst(h, id)
  local kind = S.get(h, S.inst_key(id, "type"))
  if kind == nil then
    return nil
  end
  local giver = S.get(h, S.inst_key(id, "giver"))
  local taker = S.get(h, S.inst_key(id, "taker"))
  local meta = {}
  local names = S.split(S.get(h, S.inst_key(id, "m.names")))
  for i = 1, #names do
    meta[names[i]] = S.get(h, S.inst_key(id, "m." .. names[i]))
  end
  return {
    id = id,
    type = kind,
    step = S.get(h, S.inst_key(id, "step")),
    state = S.get(h, S.inst_key(id, "state")),
    giver = (giver ~= nil and giver ~= 0) and giver or nil,
    taker = (taker ~= nil and taker ~= 0) and taker or nil,
    meta = meta,
  }
end

--- 写一个实例的标量面（meta 由调用方逐键写，见 `write_meta`）。
local function write_inst(h, inst)
  S.set(h, S.inst_key(inst.id, "type"), inst.type)
  S.set(h, S.inst_key(inst.id, "step"), inst.step)
  S.set(h, S.inst_key(inst.id, "state"), inst.state)
  S.set(h, S.inst_key(inst.id, "giver"), inst.giver or 0)
  S.set(h, S.inst_key(inst.id, "taker"), inst.taker or 0)
end

--- 逐键写 meta（只写调用方真的给了的键；`nil` 值等于缺键，不落盘）。
local function write_meta(h, id, meta)
  local names = {}
  for key, value in pairs(meta) do
    S.set(h, S.inst_key(id, "m." .. key), value)
    names[#names + 1] = key
  end
  S.sort(names)
  S.set(h, S.inst_key(id, "m.names"), S.join(names))
end

--- 读实例并要求存在（`YGG-PKG-QUEST-ID`）。
local function must_inst(h, id)
  if type(id) ~= "number" or id % 1 ~= 0 or id < 1 then
    S.fail(S.E.ID, "id 必须是正整数，得到 " .. tostring(id))
  end
  local inst = read_inst(h, id)
  if inst == nil then
    S.fail(S.E.ID, "任务 " .. tostring(id) .. " 不存在")
  end
  return inst
end

-- ── 原语 ───────────────────────────────────────────────────────────────────

--- `open(type_name, opts)`：分配 id、落实例、发 `quest.opened`。返回新 id。
function F.open(h, type_name, opts)
  local spec = T.read(h, type_name)
  if spec == nil then
    S.fail(S.E.TYPE, "类型 " .. tostring(type_name) .. " 未注册")
  end
  if opts ~= nil and type(opts) ~= "table" then
    S.fail(S.E.META, "opts 只收 table 或 nil，得到 " .. type(opts))
  end
  local meta = (opts ~= nil and opts.meta) or {}
  if type(meta) ~= "table" then
    S.fail(S.E.META, "opts.meta 只收 table")
  end
  for key, value in pairs(meta) do
    local kind = spec.meta_types[key]
    if kind == nil then
      S.fail(S.E.META, "meta 键 " .. tostring(key) .. " 未在模板的 meta_spec 里声明")
    end
    local actual = type(value)
    local ok = (kind == actual)
    if kind == "ref" then
      ok = S.ref_ok(value)
    elseif kind == "number" then
      ok = actual == "number" and S.scalar(value, S.E.META, "meta[" .. key .. "]") ~= nil
    end
    if not ok then
      S.fail(S.E.META, "meta[" .. key .. "] 必须是 " .. kind .. "，得到 " .. tostring(value))
    end
  end
  local giver, taker = (opts ~= nil) and opts.giver or nil, (opts ~= nil) and opts.taker or nil
  if not S.ref_ok(giver) then
    S.fail(S.E.REF, "opts.giver 必须是正整数 id 或 nil，得到 " .. tostring(giver))
  end
  if not S.ref_ok(taker) then
    S.fail(S.E.REF, "opts.taker 必须是正整数 id 或 nil，得到 " .. tostring(taker))
  end
  local id = S.seq(h) + 1
  local inst = { id = id, type = type_name, step = 1, state = "open", giver = giver, taker = taker }
  write_inst(h, inst)
  write_meta(h, id, meta)
  S.set(h, S.index_key(id), true)
  S.set(h, S.K.seq, id)
  h.emit("quest.opened", tostring(id) .. "|" .. type_name)
  return id
end

--- `get(id)`：实例的浅副本（不存在 = `nil`）。
function F.get(h, id)
  if type(id) ~= "number" or id % 1 ~= 0 or id < 1 then
    return nil
  end
  local inst = read_inst(h, id)
  if inst == nil then
    return nil
  end
  local meta = {}
  for key, value in pairs(inst.meta) do
    meta[key] = value
  end
  return {
    id = inst.id,
    type = inst.type,
    step = inst.step,
    state = inst.state,
    giver = inst.giver,
    taker = inst.taker,
    meta = meta,
  }
end

--- `assign(id, giver, taker)`：只写两个 ref，不判权限、不发事件（契约 §1 / §4）。
function F.assign(h, id, giver, taker)
  local inst = must_inst(h, id)
  rule_of(inst.state, "assign")
  if not S.ref_ok(giver) then
    S.fail(S.E.REF, "giver 必须是正整数 id 或 nil，得到 " .. tostring(giver))
  end
  if not S.ref_ok(taker) then
    S.fail(S.E.REF, "taker 必须是正整数 id 或 nil，得到 " .. tostring(taker))
  end
  S.set(h, S.inst_key(id, "giver"), giver or 0)
  S.set(h, S.inst_key(id, "taker"), taker or 0)
  return true
end

--- `advance(id)`：步下标恰好 +1；末步转 `done`。返回推进后的 `state`。
---
--- 只读**一个**标量（`quest.t.<n>.steps`）而不读整份模板：契约 §5 把 `advance`
--- 钉在 `O(1)`，读整张步骤表会把它变成 `O(步数)`（模板越大越慢，正是「按 id 更新」
--- 那条指标要挡的东西）。
function F.advance(h, id)
  local inst = must_inst(h, id)
  local rule = rule_of(inst.state, "advance")
  local count = S.get(h, S.type_key(inst.type, "steps"))
  if count == nil then
    S.fail(S.E.TYPE, "任务 " .. tostring(id) .. " 的类型 " .. tostring(inst.type) .. " 未注册")
  end
  if inst.step < count then
    inst.step = inst.step + 1
    S.set(h, S.inst_key(id, "step"), inst.step)
    h.emit(rule.event, tostring(id) .. "|" .. tostring(inst.step))
    return inst.state
  end
  inst.state = rule.finish
  S.set(h, S.inst_key(id, "state"), inst.state)
  h.emit(rule.finish_event, tostring(id))
  return inst.state
end

--- `jump(id, step_id)`：显式跳步，留 `quest.jumped` 事件（契约 I2 的唯一例外）。
function F.jump(h, id, step_id)
  local inst = must_inst(h, id)
  local rule = rule_of(inst.state, "jump")
  local spec = T.read(h, inst.type)
  if spec == nil then
    S.fail(S.E.TYPE, "任务 " .. tostring(id) .. " 的类型 " .. tostring(inst.type) .. " 未注册")
  end
  local target = nil
  for i = 1, #spec.steps do
    if spec.steps[i].id == step_id then
      target = i
    end
  end
  if target == nil then
    S.fail(S.E.STEP, "步 id " .. tostring(step_id) .. " 不在类型 " .. spec.name .. " 的模板里")
  end
  local from = inst.step
  S.set(h, S.inst_key(id, "step"), target)
  h.emit(rule.event, tostring(id) .. "|" .. tostring(from) .. "|" .. tostring(target))
  return true
end

--- `abandon(id, reason)`：进 `abandoned` 终态，留 `quest.abandoned`。
function F.abandon(h, id, reason)
  local inst = must_inst(h, id)
  local rule = rule_of(inst.state, "abandon")
  if reason == nil then
    reason = ""
  end
  if type(reason) ~= "string" then
    S.fail(S.E.META, "reason 必须是字符串，得到 " .. type(reason))
  end
  S.set(h, S.inst_key(id, "state"), rule.to)
  h.emit(rule.event, tostring(id) .. "|" .. reason)
  return true
end

--- `step(id)`：当前步的读数值（实例不存在 = `nil`）。
function F.step(h, id)
  local inst = F.get(h, id)
  if inst == nil then
    return nil
  end
  local spec = T.read(h, inst.type)
  if spec == nil then
    return nil
  end
  local step = spec.steps[inst.step]
  if step == nil then
    return nil
  end
  return { index = inst.step, id = step.id, guide = step.guide, on_enter = step.on_enter }
end

--- `open_ids(owner_ref?)`：还开着的任务 id（升序；给 ref 则按其 giver/taker 过滤）。
function F.open_ids(h, owner)
  if owner ~= nil and not S.ref_ok(owner) then
    S.fail(S.E.REF, "owner_ref 必须是正整数 id 或 nil，得到 " .. tostring(owner))
  end
  local out = {}
  local ids = S.ids(h)
  for i = 1, #ids do
    local inst = read_inst(h, ids[i])
    if inst ~= nil and inst.state == "open" then
      if owner == nil or inst.giver == owner or inst.taker == owner then
        out[#out + 1] = ids[i]
      end
    end
  end
  return out
end

--- `keys()`：本包实际拥有的 KV 键（升序）——codec 导出的点名列表（契约 §4）。
function F.keys(h)
  local out = {}
  local function add(key)
    if S.get(h, key) ~= nil then
      out[#out + 1] = key
    end
  end
  add(S.K.seq)
  add(S.K.types)
  local names = S.split(S.get(h, S.K.types))
  for i = 1, #names do
    local name = names[i]
    local count = S.get(h, S.type_key(name, "steps"))
    if count ~= nil then
      add(S.type_key(name, "steps"))
      for s = 1, count do
        add(S.type_key(name, "s." .. s .. ".id"))
        add(S.type_key(name, "s." .. s .. ".guide"))
        add(S.type_key(name, "s." .. s .. ".on_enter"))
      end
      add(S.type_key(name, "m.names"))
      local meta = S.split(S.get(h, S.type_key(name, "m.names")))
      for m = 1, #meta do
        add(S.type_key(name, "m." .. meta[m]))
      end
    end
  end
  local ids = S.ids(h)
  for i = 1, #ids do
    local id = ids[i]
    add(S.index_key(id))
    add(S.inst_key(id, "type"))
    add(S.inst_key(id, "step"))
    add(S.inst_key(id, "state"))
    add(S.inst_key(id, "giver"))
    add(S.inst_key(id, "taker"))
    add(S.inst_key(id, "m.names"))
    local names = S.split(S.get(h, S.inst_key(id, "m.names")))
    for m = 1, #names do
      add(S.inst_key(id, "m." .. names[m]))
    end
  end
  S.sort(out)
  return out
end

return F
