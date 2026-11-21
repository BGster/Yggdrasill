-- packages/mods/superagent/mod.lua
-- 《超级中介局》——P4「superagent game」的可运行多 agent 游戏 Mod（装配入口）。
--
-- 契约：docs/design/mod/superagent-game.md（循环 / 信封 / 路由 / 验收映射）。
-- 形态：egg new --mod packages/mods/superagent 开局可玩；核心循环 =
-- 委托（NPC quest.open）→ 执行（游戏内动作 + 挂载点业务文件）→ 交付
-- （回执信封 → quest.advance → 报酬成长）→ 下一单。
--
-- 本文件是**装配清单**（多文件纯度契约：入口可注册，模块只导出）：
--   §1 装载期快照    ygg.data 资源 → local 闭包
--   §2 包装配        pkg.* install（声明闭包 = pack.json packages 段）
--   §3 派生事件登记  包侧 + 游戏侧全部派生事件名（空缺 = unregistered 当场红）
--   §4 游戏事件      开局 / 选择路由 / 问询 / 收工 / 摘要
--   §5 inbox 路由    六类信封 kind（C5 接 agents.lua；C1 先立诚实占位）
--
-- 实体 id 不能存 local（读档后 Mod chunk 重装载）：一律落 KV（sg.* 前缀，
-- 本 mod 单写者命名空间）。

-- ============================================================================
-- §1 装载期快照：resources::install 先于 Mod chunk，顶层即可读 ygg.data。
-- ============================================================================
local WORLD = ygg.data("ygg:superagent/world")
local ENVELOPE = ygg.data("ygg:superagent/envelope")

local world = require("src.world")
local clock = require("src.clock")

-- ============================================================================
-- §2 包装配：install 只记 deps / 注注册表（不预读模板、不写 KV）。
-- ============================================================================
local entity = require("pkg.entity")
local quest = require("pkg.quest")
local dialogue = require("pkg.dialogue")
local role_skill = require("pkg.role_skill")
local procgen = require("pkg.procgen")

entity.install()
quest.install()
dialogue.install({ quest = quest })
role_skill.install()
procgen.install()

-- ============================================================================
-- §3 派生事件登记：包侧固定名 + 游戏侧派生名。真内核只派发已注册事件名，
--    缺一条 = 派发时 unregistered 当场失败（pkg-selftest 同纪律）。
-- ============================================================================
local DERIVED_EVENTS = {
  -- entity（entity/init.lua）
  "entity.created", "entity.attr_changed", "entity.destroyed",
  "entity.tagged", "entity.untagged",
  -- quest（quest/flow.lua TRANSITIONS + 显式跳步 / 放弃）
  "quest.opened", "quest.advanced", "quest.completed",
  "quest.abandoned", "quest.jumped",
  -- dialogue（dialogue/dispatch.lua）
  "dialogue.failed", "dialogue.depth_capped",
  -- role-skill（role-skill/actors.lua + skills.lua）
  "role.granted", "role.revoked", "role.skill_gained", "role.skill_lost",
  -- 本 mod 派生面（观察台 / 对话行的机器可读投影）
  "agent.dialogue", "item.transferred", "agent.recorded",
}
for i = 1, #DERIVED_EVENTS do
  ygg.on(DERIVED_EVENTS[i], function() end)
end

-- ============================================================================
-- §4 游戏事件
-- ============================================================================

-- ── 文案插值（沙箱方言：gsub 就地替换 {key} 占位）───────────────────────────
local function fill(template, vars)
  local out = template
  for key, value in pairs(vars) do
    out = out:gsub("%{" .. key .. "%}", tostring(value))
  end
  return out
end

-- ── 任务板（C1 形态：问询 / 收工；C4 起 quests.lua 接管任务板读数）──────────
local function board_options()
  local options = {}
  local any_patience = false
  local npcs = world.npc_keys()
  for i = 1, #npcs do
    if (world.patience(npcs[i]) or 0) > 0 then
      any_patience = true
      break
    end
  end
  if any_patience then
    options[#options + 1] = "bureau.ask"
  end
  options[#options + 1] = "bureau.close"
  ygg.kv.set("sg.board", world.join(options, "|"))
  return options
end

local function present_board()
  local options = board_options()
  local open_count = 0
  local prompt = fill(WORLD.copy.boardPrompt, { day = clock.day(), open = open_count })
  local labels = {}
  for i = 1, #options do
    if options[i] == "bureau.ask" then
      labels[i] = "向委托人问询"
    elseif options[i] == "bureau.close" then
      labels[i] = "下班收工"
    else
      labels[i] = options[i]
    end
  end
  ygg.narrate(prompt)
  ygg.choice(prompt, labels)
end

-- ── 开局（entryEvent；重复开局由 world.seed 幂等 + 断言双保险）──────────────
ygg.on("event.start", function()
  world.assert_not_seeded()
  local report = world.seed()
  local prologue = WORLD.copy.prologue
  for i = 1, #prologue do
    ygg.narrate(prologue[i])
  end
  ygg.narrate("铺场：seed=" .. tostring(report.seed)
    .. " sim=" .. tostring(report.sim_spawned)
    .. " NPC=" .. tostring(#report.npcs)
    .. " 道具=" .. tostring(#report.items)
    .. "；" .. clock.sim_status_line())
  -- 宿主 D1 语义：ygg.choice 之前的 emit 被丢弃——带任务板的 handler 只叙述，
  -- 不 emit（机器可读投影归无 choice 的入向 handler，见设计文档 §3）。
  clock.advance()
  present_board()
end)

-- ── 选择路由（choiceEvent = 宿主保留路由；payload = 1 起序号）────────────────
local ACTIONS = {}

ACTIONS["bureau.ask"] = function()
  local npcs = world.npc_keys()
  local target = nil
  for i = 1, #npcs do
    if (world.patience(npcs[i]) or 0) > 0 then
      target = npcs[i]
      break
    end
  end
  if target == nil then
    ygg.narrate(WORLD.copy.noCommission)
    present_board()
    return
  end
  -- C2 起对话走 pkg.dialogue 线程；C1 先按铺场耐心出 briefing。
  local id = world.npc_id(target)
  local name = ygg.entity.get_attribute(id, "name")
  local title = ygg.entity.get_attribute(id, "title")
  local briefing = ygg.entity.get_attribute(id, "briefing")
  local left = world.spend_patience(target, WORLD.tuning.askPatienceCost)
  if briefing ~= nil and #briefing > 0 then
    ygg.narrate(fill(WORLD.copy.askLine, { name = name, title = title, briefing = briefing }))
    -- D1：带任务板的 handler 不 emit（对话行投影在 C2 走 pkg.dialogue 线程 KV；
    -- 事件页投影归无 choice 的入向 handler）。
  end
  if left == 0 then
    ygg.narrate(fill(WORLD.copy.askDown, { name = name, title = title }))
  end
  clock.advance()
  present_board()
end

ACTIONS["bureau.close"] = function()
  local day = clock.advance()
  local stats = { taken = 0, delivered = 0, abandoned = 0 }
  ygg.narrate(fill(WORLD.copy.closeEnding, {
    day = day,
    taken = stats.taken,
    delivered = stats.delivered,
    abandoned = stats.abandoned,
  }))
  ygg.kv.set("sg.ended", true)
  ygg.kv.set("sg.stats", "0|0|0")
end

ygg.on("ygg.choice_selected", function(payload)
  local index = tonumber(payload)
  local options = {}
  local board = ygg.kv.get("sg.board") or ""
  for id in (board .. "|"):gmatch("([^|]*)|") do
    if #id > 0 then
      options[#options + 1] = id
    end
  end
  local action = options[index or -1]
  if action == nil or ACTIONS[action] == nil then
    ygg.narrate("【拒绝】无效选择：" .. tostring(payload) .. "（板上是 " .. board .. "）")
    present_board()
    return
  end
  ACTIONS[action]()
end)

-- ── 问询 / 收工的事件直发面（headless 冒烟用；与选择路由同一动作体）──────────
ygg.on("bureau.ask", function()
  ACTIONS["bureau.ask"]()
end)

ygg.on("bureau.close", function()
  ACTIONS["bureau.close"]()
end)

-- ── 摘要（summaryEvent：只读，daemon / 面板读它；不落 narrative）────────────
ygg.on("agent.summary", function()
  ygg.meta("summary", {
    stage = ygg.kv.get("sg.ended") == true and "closed" or "board",
    ending = ygg.kv.get("sg.ended") == true,
    verifyPassed = true,
    nextEndsRun = ygg.kv.get("sg.ended") == true,
    lifecycle = "running",
    batch = 1,
    batchDrained = true,
    pendingProposals = 0,
    requirementCount = 0,
    day = clock.day(),
    simHash = clock.sim_hash(),
  })
  ygg.meta("sim_hash", clock.sim_hash())
end)

-- ============================================================================
-- §5 inbox 路由：六类信封 kind（信封 schema 见 data/envelope.json；
--    C5 起 agents.lua 承接校验 / 幂等 / 路由，此处先立诚实占位——
--    未接线的 kind 到场必须给人读交代，不静默吞行）。
-- ============================================================================
local INBOX_KINDS = {}
for i = 1, #ENVELOPE.kinds do
  INBOX_KINDS[ENVELOPE.kinds[i].kind] = ENVELOPE.kinds[i].wire
end

for kind in pairs(INBOX_KINDS) do
  ygg.on(kind, function(payload)
    ygg.narrate("[INBOX-UNBOUND] " .. kind .. " 信封已入队但适配器未接线（C5 落地 agents.lua）：" .. tostring(payload))
    ygg.emit("agent.recorded", kind .. "|unbound")
  end)
end

-- 出向工具回执（tools 段 resultEvent；C5 起 agents.lua 落账 → 交付）。
ygg.on("harness.replied", function(payload)
  ygg.narrate("[INBOX-UNBOUND] harness.replied 工具回执未接线（C5 落地）：" .. tostring(payload))
end)
