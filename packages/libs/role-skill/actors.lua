-- `packages/libs/role-skill/actors.lua` —— 角色 / 技能定义表 + 实体记录（授予、撤销、查询）。
--
-- 契约：`openwiki/contracts/packages/role-skill.md` §2 / §4 / I1（薄层）/ I3（授予幂等）/
-- I7（只看引用面）。
--
-- # 记录里只有引用与计数
--
-- `role.actor.<id>` 的载荷恰好三个键：`role`（角色名）、`skills`（技能名 → `{ rank,
-- charges }`）、`counters`（事件名 → 次数）。属性值一个字节都不存——属性袋的真源是
-- [entity](../../openwiki/contracts/packages/entity.md)，本包只在读时**合成**（I2）。
--
-- # 同 spec 判定 = 规范化后编码字节相同
--
-- 幂等（I3）要一个可判定的「同一份 spec」。规范化把缺省补成显式空表、把字段顺序
-- 交给 codec 的字节序，于是比较退化成两个字符串相等——不需要第二套深比较规则。
--
-- # 索引只增不减
--
-- KV 没有删除面（`store.lua` 文件头），所以 `role.index` 是**只增**的升序稠密数组：
-- 首次为某个 id 建立记录时插入，之后永不摘除。`revoke` 只清引用面。

local store = require("pkg.role_skill.store")

local fail = store.fail
local E = store.E

local M = {}

-- ── 规范化（一切外部 spec 都经这里变成内部形）────────────────────────────────

local function string_array(value, label)
  local out = {}
  if value == nil then
    return out
  end
  if type(value) ~= "table" then
    fail(E.DEF, label .. " 必须是字符串数组，得到 " .. type(value))
  end
  for i = 1, #value do
    if type(value[i]) ~= "string" then
      fail(E.DEF, label .. "[" .. i .. "] 必须是字符串，得到 " .. type(value[i]))
    end
    out[i] = value[i]
  end
  return out
end

local function attrs_spec(value, label)
  local out = {}
  if value == nil then
    return out
  end
  if type(value) ~= "table" then
    fail(E.DEF, label .. " 必须是 { 属性 = number } 表，得到 " .. type(value))
  end
  for name, bonus in pairs(value) do
    if type(name) ~= "string" or name == "" then
      fail(E.DEF, label .. " 的属性名必须是非空字符串")
    end
    if type(bonus) ~= "number" then
      fail(E.DEF, label .. "." .. name .. " 的加成必须是 number，得到 " .. type(bonus))
    end
    out[name] = bonus
  end
  return out
end

local function normalize_role(name, spec)
  return {
    attrs_spec = attrs_spec(spec.attrs_spec, "角色 " .. name .. " 的 attrs_spec"),
    grants_tags = string_array(spec.grants_tags, "角色 " .. name .. " 的 grants_tags"),
    requires_tags = string_array(spec.requires_tags, "角色 " .. name .. " 的 requires_tags"),
  }
end

local function normalize_skill(name, spec)
  local requires = spec.requires
  local normalized_requires = { tags = {} }
  if requires ~= nil then
    if type(requires) ~= "table" then
      fail(E.DEF, "技能 " .. name .. " 的 requires 必须是表，得到 " .. type(requires))
    end
    normalized_requires.tags = string_array(requires.tags, "技能 " .. name .. " 的 requires.tags")
    if requires.role ~= nil then
      if type(requires.role) ~= "string" or requires.role == "" then
        fail(E.DEF, "技能 " .. name .. " 的 requires.role 必须是非空字符串")
      end
      normalized_requires.role = requires.role
    end
  end

  local charges = spec.charges
  if charges ~= nil then
    if type(charges) ~= "number" or charges ~= (charges // 1) or charges < 0 then
      fail(E.DEF, "技能 " .. name .. " 的 charges 必须是非负整数")
    end
    charges = charges | 0
  end

  return {
    attrs_spec = attrs_spec(spec.attrs_spec, "技能 " .. name .. " 的 attrs_spec"),
    charges = charges,
    requires = normalized_requires,
  }
end

--- 两份规范化 spec 是否同一份（比较编码字节，见文件头）。
local function same_spec(left, right)
  local codec = store.codec()
  return codec.encode(left) == codec.encode(right)
end

-- ── 定义表（写：`role.defs` / `role.skills`）───────────────────────────────

--- 全部角色定义（一份读；`effective` 靠它把读次数钉成常数）。
function M.all_roles()
  return store.read(store.KEY.DEFS, nil) or {}
end

--- 全部技能定义（同上）。
function M.all_skills()
  return store.read(store.KEY.SKILLS, nil) or {}
end

--- `role.define(name, spec)` → `true`；同 spec 幂等，异 spec 报 `-DEF`。
function M.define(name, spec)
  if type(name) ~= "string" or name == "" then
    fail(E.DEF, "角色名必须是非空字符串，得到 " .. type(name))
  end
  if type(spec) ~= "table" then
    fail(E.DEF, "角色 " .. name .. " 的 spec 必须是表，得到 " .. type(spec))
  end
  local normalized = normalize_role(name, spec)
  local role_defs = store.read(store.KEY.DEFS, nil)
  if role_defs == nil then
    role_defs = {}
  end
  local existing = role_defs[name]
  if existing ~= nil then
    if same_spec(existing, normalized) then
      return true
    end
    fail(E.DEF, "角色 " .. name .. " 已用另一份 spec 定义过")
  end
  role_defs[name] = normalized
  store.write(store.KEY.DEFS, role_defs)
  return true
end

--- `role.skill_define(name, spec)` → `true`；同样的幂等口径。
function M.skill_define(name, spec)
  if type(name) ~= "string" or name == "" then
    fail(E.DEF, "技能名必须是非空字符串，得到 " .. type(name))
  end
  if type(spec) ~= "table" then
    fail(E.DEF, "技能 " .. name .. " 的 spec 必须是表，得到 " .. type(spec))
  end
  local normalized = normalize_skill(name, spec)
  local skill_defs = store.read(store.KEY.SKILLS, nil)
  if skill_defs == nil then
    skill_defs = {}
  end
  local existing = skill_defs[name]
  if existing ~= nil then
    if same_spec(existing, normalized) then
      return true
    end
    fail(E.DEF, "技能 " .. name .. " 已用另一份 spec 定义过")
  end
  skill_defs[name] = normalized
  store.write(store.KEY.SKILLS, skill_defs)
  return true
end

-- ── 实体记录（读 / 写 + 索引）─────────────────────────────────────────────

--- 读一份记录（不存在 → `nil`）；`skills` / `counters` 缺失时补成空表。
function M.read_actor(id)
  local record = store.read(store.actor_key(id), nil)
  if type(record) ~= "table" then
    return nil
  end
  if type(record.skills) ~= "table" then
    record.skills = {}
  end
  if type(record.counters) ~= "table" then
    record.counters = {}
  end
  return record
end

--- 升序插入（返回是否真的插入）：尾部追加是 O(1) 快路径，中间走二分 + 后移。
local function index_add(index, id)
  local count = #index
  if count == 0 or id > index[count] then
    index[count + 1] = id
    return true
  end
  local low, high = 1, count
  while low <= high do
    local middle = (low + high) // 2
    local value = index[middle]
    if value == id then
      return false
    end
    if value < id then
      low = middle + 1
    else
      high = middle - 1
    end
  end
  for i = count + 1, low + 1, -1 do
    index[i] = index[i - 1]
  end
  index[low] = id
  return true
end

--- 写一份记录；首次为某 id 建立记录时把它插入 `role.index`（只增不减）。
function M.write_actor(id, record)
  local index = store.read(store.KEY.INDEX, nil)
  if index == nil then
    index = {}
  end
  if index_add(index, id) then
    store.write(store.KEY.INDEX, index)
  end
  store.write(store.actor_key(id), record)
end

-- ── 授予 / 撤销 / 查询 ─────────────────────────────────────────────────────

--- 引用面校验（I7）：只看 tag，不看等级、不看前置链。
local function tags_ok(id, tags)
  if #tags == 0 then
    return true
  end
  local entity = store.entity()
  for i = 1, #tags do
    if not entity.has(id, tags[i]) then
      return false
    end
  end
  return true
end

--- `role.grant(id, role_name)` → `boolean`（生效后是否持有；幂等，事件只发一次）。
---
--- 返回的是**生效后的状态**而不是「本次是否改变」：连调两次都是 `true`（I3 说的是
--- 状态，不是变更）。这与 `entity.set` 的「返回是否变化」刻意不同。
function M.grant(id, role_name)
  local role_defs = store.read(store.KEY.DEFS, nil)
  local spec = role_defs ~= nil and role_defs[role_name] or nil
  if spec == nil then
    fail(E.UNKNOWN, "角色 " .. tostring(role_name) .. " 未定义")
  end
  if not tags_ok(id, spec.requires_tags) then
    return false
  end
  local record = M.read_actor(id)
  if record ~= nil and record.role == role_name then
    return true
  end
  if record == nil then
    record = { counters = {}, skills = {} }
  end
  record.role = role_name
  M.write_actor(id, record)
  store.emit("role.granted", tostring(id) .. "|" .. role_name)
  return true
end

--- `role.revoke(id, role_name)` → `boolean`（原本是否持有；未持有 `false` 不报错）。
---
--- 只清 `role` 字段：**技能集不动**（技能是独立引用，撤销角色不该连带没收技能——
--- 「哪些技能跟着角色走」是 mod 的规则，包不猜）。
function M.revoke(id, role_name)
  local record = M.read_actor(id)
  if record == nil or record.role ~= role_name then
    return false
  end
  record.role = nil
  M.write_actor(id, record)
  store.emit("role.revoked", tostring(id) .. "|" .. role_name)
  return true
end

--- `role.has(id, role_name)` → `boolean`（纯读）。
function M.has(id, role_name)
  local record = M.read_actor(id)
  return record ~= nil and record.role == role_name
end

-- ── 技能集读面（纯读：`skills_of` / `skill_state`）──────────────────────

--- `role.skills_of(id)` → **升序名字数组**（无记录 / 无槽位 ⇒ `{}`）。
---
--- 排序必须自己做：宿主沙箱只暴露白名单 globals，`math` / `string` / `table` 三张
--- 表都不可达（`crates/ygg-sandbox/src/sandbox.rs` 的 `BASE_WHITELIST`），没有
--- `table.sort`。槽位数是 6–29，插入排序的 O(k²) 可以不计。
---
--- `rank = 0` 的槽位也返回：这是**结构真相**（槽位存在与否是写面的事），包不替
--- 调用方筛「等级够不够」——那是 mod 的规则（I7）。
function M.skills_of(id)
  local record = M.read_actor(id)
  if record == nil then
    return {}
  end
  local names = {}
  for name in pairs(record.skills) do
    names[#names + 1] = name
  end
  for i = 2, #names do
    local value = names[i]
    local j = i - 1
    while j > 0 and names[j] > value do
      names[j + 1] = names[j]
      j = j - 1
    end
    names[j + 1] = value
  end
  return names
end

--- `role.skill_state(id, name)` → 槽位**副本** `{ rank, charges?, capacity? }` 或 `nil`。
---
--- 返回副本而不是内部引用：调用方改了返回值不得影响后续读取（否则「读面」会变成
--- 一个不会写盘的旁路写面）。字段只映槽位，**不并入**声明的 `charges`（声明面是
--- mod 自己的数据，包内的 defs 面只有写、没有读）。
function M.skill_state(id, name)
  if type(name) ~= "string" or name == "" then
    fail(E.DEF, "skill_state 的技能名必须是非空字符串，得到 " .. tostring(name))
  end
  local record = M.read_actor(id)
  if record == nil then
    return nil
  end
  local slot = record.skills[name]
  if slot == nil then
    return nil
  end
  local out = { rank = slot.rank }
  if slot.charges ~= nil then
    out.charges = slot.charges
  end
  if slot.capacity ~= nil then
    out.capacity = slot.capacity
  end
  return out
end

return M
