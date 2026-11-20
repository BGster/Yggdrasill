-- `packages/libs/role-skill/skills.lua` —— 技能槽位（`rank` / `charges`）与充能语义。
--
-- 契约：`docs/design/packages/role-skill.md` §4 / I4（引用建立即事件）/ I7。
--
-- # 槽位是引用，不是效果
--
-- `rank` 与 `charges` 都存在槽位里，但 `rank` **不参与属性合成**（契约 D8）：加成只
-- 取 `attrs_spec[attr]`。要「按等级缩放」的 mod 用 `role.compose` 自定义规则——包不
-- 猜等级曲线（那在不同游戏里差得太远：线性 / 指数 / 分段）。
--
-- # charges 是 PP 语义
--
-- 声明了 `charges` 的技能才有充能：建立槽位时充到满格、`charge` 每次 −1 且**零不减
-- 负**、`recharge` 补到声明上限。未声明 `charges` 的技能不参与充能语义——两个函数都
-- 返回 `0` 且不写，这样「没有 PP 的技能」与「PP 用光的技能」在返回值上不同（前者
-- 永远 0，后者从 0 起步）。
--
-- # 冷却不是时间
--
-- 冷却由 `counters`（事件计数）表达，不由时钟：宿主的时间面（`ticks()`）有 API
-- 无推进者（恒 0），拿它做冷却等于冷却永不过期（契约 I6）。

local store = require("pkg.role_skill.store")
local actors = require("pkg.role_skill.actors")

local fail = store.fail
local E = store.E

local M = {}

--- 技能声明表里的一项（未定义 → `nil`）。
local function skill_spec(name)
  local defs = store.read(store.KEY.SKILLS, nil)
  if defs == nil then
    return nil
  end
  return defs[name]
end

--- 引用面校验（I7）：只查 tag 与角色引用，等级 / 前置链归 mod。
local function requires_ok(id, record, spec)
  local requires = spec.requires
  if requires == nil then
    return true
  end
  local tags = requires.tags or {}
  if #tags > 0 then
    local entity = store.entity()
    for i = 1, #tags do
      if not entity.has(id, tags[i]) then
        return false
      end
    end
  end
  if requires.role ~= nil then
    if record == nil or record.role ~= requires.role then
      return false
    end
  end
  return true
end

local function check_rank(rank, label)
  if type(rank) ~= "number" or rank ~= (rank // 1) then
    fail(E.DEF, label .. " 必须是整数，得到 " .. tostring(rank))
  end
  return rank | 0
end

--- `role.grant_skill(id, name, rank?)` → `boolean`。
---
--- 已有槽位时**只更新 `rank`**（缺省保持原值）且不发事件：`role.skill_gained` 表示
--- 「引用建立」，升级不是建立（I4 的事件名固定，不随技能名或等级变化）。
--- 新建槽位时 `rank` 缺省 1，`charges` 按声明充满。
function M.grant_skill(id, name, rank)
  local spec = skill_spec(name)
  if spec == nil then
    fail(E.UNKNOWN, "技能 " .. tostring(name) .. " 未定义")
  end
  if rank ~= nil then
    rank = check_rank(rank, "grant_skill 的 rank")
  end

  local record = actors.read_actor(id)
  if not requires_ok(id, record, spec) then
    return false
  end
  if record == nil then
    record = { counters = {}, skills = {} }
  end

  local slot = record.skills[name]
  if slot ~= nil then
    if rank ~= nil and slot.rank ~= rank then
      slot.rank = rank
      actors.write_actor(id, record)
    end
    return true
  end

  slot = { rank = rank or 1 }
  if spec.charges ~= nil then
    slot.charges = spec.charges
  end
  record.skills[name] = slot
  actors.write_actor(id, record)
  store.emit("role.skill_gained", { id = id, rank = slot.rank, skill = name })
  return true
end

--- `role.revoke_skill(id, name)` → `boolean`（原本是否有槽位；记录保留）。
function M.revoke_skill(id, name)
  local record = actors.read_actor(id)
  if record == nil or record.skills[name] == nil then
    return false
  end
  record.skills[name] = nil
  actors.write_actor(id, record)
  store.emit("role.skill_lost", { id = id, skill = name })
  return true
end

--- `role.charge(id, name)` → 剩余格数。
---
--- 无槽位、或该技能没声明 `charges` ⇒ `0` 且不写（不参与充能语义）。零不再减负。
function M.charge(id, name)
  local record = actors.read_actor(id)
  if record == nil then
    return 0
  end
  local slot = record.skills[name]
  if slot == nil or slot.charges == nil then
    return 0
  end
  if slot.charges > 0 then
    slot.charges = slot.charges - 1
    actors.write_actor(id, record)
  end
  return slot.charges
end

--- `role.recharge(id, name, n?)` → 恢复后的格数。
---
--- 上限取**声明**的 `charges`（不是槽位当前值）：声明是唯一真源，槽位只是当次状态。
--- `n` 缺省 = 直接充满；给了 `n` 就加 `n` 再夹到上限。
function M.recharge(id, name, n)
  local record = actors.read_actor(id)
  if record == nil then
    return 0
  end
  local slot = record.skills[name]
  if slot == nil or slot.charges == nil then
    return 0
  end
  local spec = skill_spec(name)
  if spec == nil or spec.charges == nil then
    return 0
  end
  local capacity = spec.charges
  local step = capacity
  if n ~= nil then
    step = check_rank(n, "recharge 的 n")
    if step < 0 then
      fail(E.DEF, "recharge 的 n 必须是非负整数，得到 " .. step)
    end
  end
  local value = slot.charges + step
  if value > capacity then
    value = capacity
  end
  if value ~= slot.charges then
    slot.charges = value
    actors.write_actor(id, record)
  end
  return value
end

return M
