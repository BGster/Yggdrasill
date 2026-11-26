-- `src/hub.lua` —— 交付判定与过级（hub + hub_goals 的 v1 等价面）。
--
-- # 契约定位（openwiki/contracts/mod/shapez-game.md §5）
--
-- 源 hub_goals.js（576 行）的关卡面：当前级目标形状 + 交付计数 + 过级。
-- 关卡实例 = `pkg.quest` 实例（类型 `shapez.level`，meta：level/shape/required），
-- 过级 = `quest.advance` 至 done + 开下一级实例；计数真源 = KV（`sz.got`），
-- quest 实例承载「本级进行中」的状态投影（I3：恰一次 completed）。
--
-- 计数口径（源 handleShapeDelivered 同构）：交付任意形状都被 hub 接收
-- （返回 accepted），但只有**命中当前目标**（shortkey 相等，空项不算）才计数。
-- 目标命中判定 = 规范形串相等（源 getHash 比较）。
--
-- 沙箱方言：模块顶层纯；ygg 经 host() 单点取。

local entity = require("pkg.entity")
local quest = require("pkg.quest")
local belts = require("src.belts")

local M = { VERSION = "1.0.0" }

local LEVEL_TYPE = "shapez-level"
local HUB_ID_KEY = "sz.hub_id"
local LEVEL_KEY = "sz.level"
local GOT_KEY = "sz.got"
local QUEST_KEY = "sz.level_quest"

local function host()
  return ygg
end

local function fail(code, detail)
  error("YGG-MOD-HUB-" .. code .. ": " .. detail, 0)
end

--- `setup(world)`：定义关卡任务类型 + 开 1 级实例 + 铺 hub 实体。
--- world = ygg.data("ygg:shapez/world")。重复调用幂等（已 setup 直接返回）。
function M.setup(world)
  local h = host()
  if h.kv.get(LEVEL_KEY) ~= nil then
    return nil
  end
  quest.define(LEVEL_TYPE, {
    steps = { { id = "deliver", guide = "交付当前级目标形状" } },
    meta_spec = { level = "number", shape = "string", required = "number" },
  })
  h.kv.set(LEVEL_KEY, 1)
  h.kv.set(GOT_KEY, 0)
  local qid = quest.open(LEVEL_TYPE, { meta = {
    level = 1,
    shape = world.levels[1].shape,
    required = world.levels[1].required,
  } })
  h.kv.set(QUEST_KEY, qid)
  return qid
end

--- `place(x, y, rot)` → hub 实体 id（一局一台；重复放置拒绝）。
function M.place(x, y, rot)
  local h = host()
  if h.kv.get(HUB_ID_KEY) ~= nil then
    fail("ONE", "hub 已在场（一局一台）")
  end
  local id = entity.spawn("sz.hub", { x = x, y = y, rot = rot }, { hub = true })
  h.kv.set(HUB_ID_KEY, id)
  belts.bump_layout()
  return id
end

--- `hub_id()` → id | nil。
function M.hub_id()
  return host().kv.get(HUB_ID_KEY)
end

local function level_of(world, n)
  for i = 1, #world.levels do
    if world.levels[i].level == n then
      return world.levels[i]
    end
  end
  return nil
end

--- `current_goal(world)` → {level, shape, required, got, quest_id} | nil（关卡表外）。
function M.current_goal(world)
  local h = host()
  local n = h.kv.get(LEVEL_KEY)
  local def = level_of(world, n)
  if def == nil then
    return nil
  end
  return {
    level = n,
    shape = def.shape,
    required = def.required,
    got = h.kv.get(GOT_KEY) or 0,
    quest_id = h.kv.get(QUEST_KEY),
  }
end

--- `deliver(world, item_key)` → {accepted, counted, levelup}。
--- accepted = hub 收下（非空项一律收）；counted = 命中当前目标；levelup = 是否过级。
function M.deliver(world, item_key)
  local h = host()
  if type(item_key) ~= "string" or item_key == "" then
    return { accepted = false, counted = false, levelup = false }
  end
  local goal = M.current_goal(world)
  if goal == nil then
    -- 关卡表打完：一律接收，不再计数
    return { accepted = true, counted = false, levelup = false }
  end
  if item_key ~= goal.shape then
    return { accepted = true, counted = false, levelup = false }
  end
  local got = (h.kv.get(GOT_KEY) or 0) + 1
  h.kv.set(GOT_KEY, got)
  h.emit("sz.delivered", goal.shape .. "|" .. got)
  local levelup = false
  if got >= goal.required then
    levelup = true
    quest.advance(goal.quest_id) -- → completed（本级恰一次，I3）
    h.kv.set(LEVEL_KEY, goal.level + 1) -- 末级也照增（源：过末级进 freeplay）
    h.kv.set(GOT_KEY, 0)
    local next_def = level_of(world, goal.level + 1)
    if next_def ~= nil then
      local qid = quest.open(LEVEL_TYPE, { meta = {
        level = next_def.level,
        shape = next_def.shape,
        required = next_def.required,
      } })
      h.kv.set(QUEST_KEY, qid)
    end
    h.emit("sz.levelup", goal.shape .. "|" .. goal.level)
  end
  return { accepted = true, counted = true, levelup = levelup }
end

--- `snapshot()` → 关卡面状态行（重放哈希输入；纯读）。
function M.snapshot()
  local h = host()
  return {
    level = h.kv.get(LEVEL_KEY),
    got = h.kv.get(GOT_KEY) or 0,
    quest = h.kv.get(QUEST_KEY),
    hub = h.kv.get(HUB_ID_KEY),
  }
end

return M
