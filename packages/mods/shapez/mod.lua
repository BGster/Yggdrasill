-- packages/mods/shapez/mod.lua
-- 《shapez v1 核心循环》——S26-1 的可运行模拟 Mod（装配入口）。
--
-- 契约：docs/design/mod/shapez-game.md（核心循环 / 定点口径 / 验收映射）。
-- 形态：headless 模拟面（事件直发驱动；v2 接视口/面板）。核心循环 =
-- 矿机产形 → 传送带定点推进 → 切割/堆叠/旋转 → hub 交付 → 过级。
--
-- 本文件是**装配清单**（多文件纯度契约：入口可注册，模块只导出）：
--   §1 装载期快照    ygg.data 资源 → local 闭包
--   §2 包装配        pkg.* install（声明闭包 = pack.json packages 段）
--   §3 派生事件登记  包侧 + 游戏侧全部派生事件名（空缺 = unregistered 当场红）
--   §4 游戏事件      开局 / 放置直发面 / tick 直发面 / 摘要
--
-- 实体 id 不能存 local（读档后 Mod chunk 重装载）：一律落 KV（sz.* 前缀，
-- 本 mod 单写者命名空间）。

-- ============================================================================
-- §1 装载期快照：resources::install 先于 Mod chunk，顶层即可读 ygg.data。
-- ============================================================================
local WORLD = ygg.data("ygg:shapez/world")

local shapes = require("src.shapes")
local belts = require("src.belts")
local processors = require("src.processors")
local hub = require("src.hub")
local simtick = require("src.simtick")

-- ============================================================================
-- §2 包装配：install 只记 deps / 注注册表（不预读模板、不写 KV）。
-- ============================================================================
local entity = require("pkg.entity")
local quest = require("pkg.quest")
local procgen = require("pkg.procgen")

entity.install()
quest.install()

-- ============================================================================
-- §3 派生事件登记：包侧固定名 + 游戏侧派生名。真内核只派发已注册事件名，
--    缺一条 = 派发时 unregistered 当场失败（pkg-selftest 同纪律）。
-- ============================================================================
local DERIVED_EVENTS = {
  -- entity（entity/init.lua）
  "entity.created", "entity.attr_changed", "entity.destroyed",
  "entity.tagged", "entity.untagged",
  -- quest（quest/flow.lua TRANSITIONS）
  "quest.opened", "quest.advanced", "quest.completed",
  "quest.abandoned", "quest.jumped",
  -- 本 mod 派生面（交付/过级的机器可读投影，全标量载荷）
  "sz.delivered", "sz.levelup",
}
for i = 1, #DERIVED_EVENTS do
  ygg.on(DERIVED_EVENTS[i], function() end)
end

-- ============================================================================
-- §4 游戏事件（headless 直发面；载荷 JSON 标量串）
-- ============================================================================

--- "x,y,rot" → 三整数。
local function parse_xyr(payload)
  local x, y, rot = tostring(payload):match("^(-?%d+),(-?%d+),(-?%d+)$")
  if x == nil then
    return nil
  end
  return tonumber(x), tonumber(y), tonumber(rot)
end

--- 资源斑查找：命中圆内返回斑表（数组序首个）。
local function patch_at(x, y)
  for i = 1, #WORLD.patches do
    local p = WORLD.patches[i]
    local dx = x - p.x
    local dy = y - p.y
    if dx * dx + dy * dy <= p.radius * p.radius then
      return p
    end
  end
  return nil
end

-- ── 开局：装配 + hub 铺场（幂等；场景搭建走放置直发面）──────────────────────
ygg.on("event.start", function()
  simtick.setup(WORLD)
  local goal = hub.current_goal(WORLD)
  ygg.narrate("shapez v1 开局：tick=" .. tostring(ygg.kv.get("sz.tick") or 0)
    .. " level=" .. tostring(goal and goal.level or "?")
    .. " 目标=" .. tostring(goal and goal.shape or "已通关"))
  return { ok = true }
end)

-- ── 放置直发面 ───────────────────────────────────────────────────────────────
ygg.on("sz.place.hub", function(payload)
  local x, y, rot = parse_xyr(payload)
  if x == nil then
    ygg.narrate("【拒绝】sz.place.hub 载荷应为 \"x,y,rot\"")
    return
  end
  local id = hub.place(x, y, rot)
  ygg.narrate("hub 落位：" .. tostring(id) .. " @" .. x .. "," .. y)
end)

ygg.on("sz.place.belt", function(payload)
  local x, y, rot = parse_xyr(payload)
  if x == nil then
    ygg.narrate("【拒绝】sz.place.belt 载荷应为 \"x,y,rot\"")
    return
  end
  local id = entity.spawn("sz.belt", { x = x, y = y, rot = rot }, { belt = true })
  belts.bump_layout()
  ygg.narrate("belt 落位：" .. tostring(id) .. " @" .. x .. "," .. y .. " rot=" .. rot)
end)

ygg.on("sz.place.proc", function(payload)
  local head = tostring(payload):match("^([^|]+)|")
  local xyr = tostring(payload):match("|([^|]+)$")
  local x, y, rot
  if xyr ~= nil then
    x, y, rot = parse_xyr(xyr)
  end
  if head == nil or x == nil then
    ygg.narrate("【拒绝】sz.place.proc 载荷应为 \"cutter|sticky|x,y,rot\"")
    return
  end
  local id = processors.place(head, x, y, rot)
  ygg.narrate(head .. " 落位：" .. tostring(id) .. " @" .. x .. "," .. y)
end)

ygg.on("sz.place.miner", function(payload)
  local x, y, rot = parse_xyr(payload)
  if x == nil then
    ygg.narrate("【拒绝】sz.place.miner 载荷应为 \"x,y,rot\"")
    return
  end
  local patch = patch_at(x, y)
  if patch == nil then
    ygg.narrate("【拒绝】矿机必须落在资源斑内：" .. x .. "," .. y)
    return
  end
  local id = processors.place_miner(x, y, rot, patch)
  ygg.narrate("miner 落位：" .. tostring(id) .. " @" .. x .. "," .. y
    .. " 产形=" .. patch.subshape .. patch.color)
end)

-- ── tick 直发面："n" = 走 n 拍（缺省 1）──────────────────────────────────────
ygg.on("sz.tick", function(payload)
  local n = tonumber(payload) or 1
  for _ = 1, n do
    simtick.tick()
  end
end)

-- ── 摘要（summaryEvent：只读；daemon / 面板读它；不落 narrative）────────────
ygg.on("shapez.summary", function()
  local h = ygg
  local goal = hub.current_goal(WORLD)
  local tick = h.kv.get("sz.tick") or 0
  local hash = "?"
  local ok, value = pcall(function()
    return h.sim.status().hash
  end)
  if ok then
    hash = value
  end
  h.meta("summary", {
    tick = tick,
    level = goal and goal.level or nil,
    shape = goal and goal.shape or "",
    got = goal and goal.got or 0,
    required = goal and goal.required or 0,
    simHash = hash,
    stateHash = (simtick.snapshot and procgen.hash(simtick.snapshot())) or nil,
  })
  h.meta("sim_hash", hash)
end)

return { shapes = shapes, belts = belts, processors = processors, hub = hub, simtick = simtick }
