-- `packages/libs/role-skill/compose.lua` —— `effective` 的合成规则（**只读，从不写回**）。
--
-- 契约：`docs/design/packages/role-skill.md` §3 I2 / §4 的 `effective` 与 `compose`。
--
-- # 为什么是合成而不是写回
--
-- 把技能加成加进属性袋最直接，也是最贵的：卸下技能要把属性减回去，而「减回去」需要
-- 知道当前值里有多少来自技能——要么存一份来源账本，要么加一次就再也减不干净。
-- 合成读法让这个问题不可能发生：属性袋只由 mod 写，本包只读。
--
-- # 加成列表的顺序是契约
--
-- `[角色加成, 技能1加成, 技能2加成, …]`，技能按**名字升序**。自定义合成规则
-- （`role.compose`）拿到的就是这个顺序——顺序变了两个游戏的同一技能加成分属两套
-- 算法，而这类差异在验收里只表现为「数值对不上」，最难归因。
--
-- # rank 不参与合成
--
-- 加成只取 `attrs_spec[attr]`（契约 D8）。要「按等级缩放」的 mod 用 `role.compose`
-- 自己算——包不猜等级曲线。
--
-- # 读次数是常数
--
-- 一次 `effective` 恰好三次 `kv.get`：记录、角色定义表、技能定义表。与实体总数无关
-- （`role.actor.<id>` 是直取，不是遍历）。性能用例按这个数钉 O(1)。

local store = require("pkg.role_skill.store")
local actors = require("pkg.role_skill.actors")

local fail = store.fail
local E = store.E

local M = {}

--- 模块级合成规则（`nil` = 缺省「相加」）。它是配置，不是游戏状态：不写 KV。
local rule = nil

--- 取记录里技能名的数组副本（升序前的输入；不就地排记录自己的键集）。
local function copy_names(skills)
  local names = {}
  for name in pairs(skills) do
    names[#names + 1] = name
  end
  return names
end

--- 插入排序（升序）。不用 `table.sort`：宿主沙箱里 `table` 表不可达，而技能数很小。
local function sort_names(names)
  for i = 2, #names do
    local value = names[i]
    local j = i - 1
    while j >= 1 and names[j] > value do
      names[j + 1] = names[j]
      j = j - 1
    end
    names[j + 1] = value
  end
  return names
end

--- 缺省规则：`base + Σ bonuses`。
local function sum_rule(base, bonuses)
  local total = base
  for i = 1, #bonuses do
    total = total + bonuses[i]
  end
  return total
end

--- `role.compose(fn)` → `true`；非函数即 `-COMPOSE`（`nil` 表示恢复缺省相加，供测试与
--- mod 热重载用）。
function M.set(fn)
  if fn ~= nil and type(fn) ~= "function" then
    fail(E.COMPOSE, "compose 需要函数 (base, bonuses) -> number，得到 " .. type(fn))
  end
  rule = fn
  return true
end

--- `role.effective(id, attr)` → `number`（基础值 + 合成加成；纯读）。
function M.effective(id, attr)
  if type(attr) ~= "string" or attr == "" then
    fail(E.ATTR, "属性名必须是非空字符串，得到 " .. type(attr))
  end

  local base = store.entity().get(id, attr)
  if base == nil then
    base = 0
  end
  if type(base) ~= "number" then
    fail(E.ATTR, "实体 " .. tostring(id) .. " 的属性 " .. attr .. " 不是数字（" .. type(base) .. "）")
  end

  local bonuses = {}
  local record = actors.read_actor(id)
  if record ~= nil then
    local role_defs = actors.all_roles()
    local role_spec = record.role ~= nil and role_defs[record.role] or nil
    if role_spec ~= nil and role_spec.attrs_spec[attr] ~= nil then
      bonuses[#bonuses + 1] = role_spec.attrs_spec[attr]
    end

    local skill_defs = actors.all_skills()
    local names = sort_names(copy_names(record.skills))
    for i = 1, #names do
      local spec = skill_defs[names[i]]
      if spec ~= nil and spec.attrs_spec[attr] ~= nil then
        bonuses[#bonuses + 1] = spec.attrs_spec[attr]
      end
    end
  end

  return (rule or sum_rule)(base, bonuses)
end

return M
