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
local CONFIG = ygg.data("ygg:superagent/config")

local world = require("src.world")
local clock = require("src.clock")
local cast = require("src.cast")
local quests = require("src.quests")
local custom = require("src.custom")
local items = require("src.items")

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
  local phase = quests.board_phase()
  if phase == "take" then
    options[#options + 1] = "commission.take"
  elseif phase == "work" then
    options[#options + 1] = "commission.work"
  elseif phase == "deliver" then
    options[#options + 1] = "commission.deliver"
  end
  local npcs = world.npc_keys()
  for i = 1, #npcs do
    if (world.patience(npcs[i]) or 0) > 0 then
      options[#options + 1] = "bureau.ask"
      break
    end
  end
  options[#options + 1] = "bureau.close"
  ygg.kv.set("sg.board", world.join(options, "|"))
  return options
end

local function present_board()
  local options = board_options()
  local prompt = fill(WORLD.copy.boardPrompt, { day = clock.day(), open = quests.open_count() })
  local labels = {}
  for i = 1, #options do
    if options[i] == "bureau.ask" then
      labels[i] = "向委托人问询"
    elseif options[i] == "bureau.close" then
      labels[i] = "下班收工"
    elseif options[i] == "commission.take" then
      labels[i] = "接单（最先到的委托）"
    elseif options[i] == "commission.work" then
      labels[i] = "现场干活（写业务文件）"
    elseif options[i] == "commission.deliver" then
      labels[i] = "交付结单"
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
  local opened = quests.open_all(WORLD)
  ygg.narrate("委托开单：" .. tostring(opened.opened) .. " 单在板（限期见单）。")
  present_board()
  return report  -- 真内核忽略 handler 返回值；selftest 拿它做铺场断言
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
    -- C2 起对话落 pkg.dialogue 线程（线程键 cast.<key>；D1：带任务板的 handler
    -- 不 emit，行投影经线程 KV 与 summary meta 读出）。
    cast.talk(target, tostring(name), tostring(briefing))
  end
  if left == 0 then
    ygg.narrate(fill(WORLD.copy.askDown, { name = name, title = title }))
  end
  clock.advance()
  present_board()
end

local function split_payload(payload)
  local first, rest = "", ""
  if type(payload) == "string" and #payload > 0 then
    local bar = payload:find("|", 1, true)
    if bar == nil then
      first = payload
    else
      first = payload:sub(1, bar - 1)
      rest = payload:sub(bar + 1)
    end
  end
  return first, rest
end

local function commission_world()
  local world_res = WORLD
  return world_res
end

ACTIONS["commission.take"] = function(payload)
  local key, spec = split_payload(payload)
  quests.enforce_deadlines()
  local snap = quests.take(key, spec)
  if snap == nil then
    ygg.narrate("【拒绝】没有可接的委托（板已空或全部在办）。")
    present_board()
    return
  end
  local title, giver = "", 0
  local commissions = commission_world().commissions
  for i = 1, #commissions do
    if commissions[i].key == snap.key then
      title = commissions[i].title
      giver = world.npc_id(commissions[i].giver)
    end
  end
  local giver_name = ygg.entity.get_attribute(giver, "name") or "?"
  ygg.narrate(fill(WORLD.copy.takeOk, {
    title = title,
    giver = giver_name,
    taker = (ygg.entity.get_attribute(snap.taker, "name") or tostring(snap.taker)),
    deadline = tostring(snap.deadline),
  }))
  clock.advance()
  present_board()
end

ACTIONS["commission.work"] = function(payload)
  local key = split_payload(payload)
  quests.enforce_deadlines()
  local snap = key ~= "" and quests.snapshot(key) or quests.workable()
  if snap == nil or snap.taker == 0 then
    ygg.narrate("【拒绝】没有在办可推进的委托（先接单）。")
    present_board()
    return
  end
  local degraded = clock.sim_mounted() == false
  local path = quests.work_file_path(snap.key, CONFIG.game.reportPrefix, degraded)
  local content = "bureau-report|" .. snap.key .. "|day=" .. tostring(clock.day())
    .. "|taker=" .. tostring(ygg.entity.get_attribute(snap.taker, "name") or snap.taker)
  if degraded then
    ygg.narrate("[FS-DEGRADED] 挂载桥未装配，业务文件写进局内制品面（不声称已写挂载点）")
  end
  quests.record_work(snap.key, path, content)
  quests.advance_step(snap.key)
  ygg.narrate("干活完成：" .. path .. "（" .. tostring(#content) .. " 字节，回读一致）。")
  clock.advance()
  present_board()
end

ACTIONS["commission.deliver"] = function(payload)
  local key, note = split_payload(payload)
  quests.enforce_deadlines()
  local result = quests.deliver(key ~= "" and key or nil)
  if result == nil then
    ygg.narrate("【拒绝】没有可交付的委托（先干活）。")
    present_board()
    return
  end
  local title = ""
  local commissions = commission_world().commissions
  for i = 1, #commissions do
    if commissions[i].key == result.key then
      title = commissions[i].title
    end
  end
  ygg.narrate(fill(WORLD.copy.deliverOk, {
    title = title,
    day = clock.day(),
    taker = (result.reward ~= "" and "承接方" or "承接方"),
    skill = result.skill,
  }) .. (note ~= "" and (" 备注：" .. note) or ""))
  ygg.narrate("报酬落账：" .. result.reward .. "。")
  clock.advance()
  present_board()
end

ACTIONS["bureau.close"] = function()
  quests.enforce_deadlines()
  local day = clock.advance()
  local taken, delivered, abandoned = quests.stats()
  ygg.narrate(fill(WORLD.copy.closeEnding, {
    day = day,
    taken = taken,
    delivered = delivered,
    abandoned = abandoned,
  }))
  ygg.kv.set("sg.ended", true)
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

-- ── 事件直发面（headless 冒烟用；与选择路由同一动作体）───────────────────────
ygg.on("bureau.ask", function()
  ACTIONS["bureau.ask"]()
end)

ygg.on("bureau.close", function()
  ACTIONS["bureau.close"]()
end)

ygg.on("commission.take", function(payload)
  ACTIONS["commission.take"](payload)
end)

ygg.on("commission.work", function(payload)
  ACTIONS["commission.work"](payload)
end)

ygg.on("commission.deliver", function(payload)
  ACTIONS["commission.deliver"](payload)
end)

-- 道具交接（headless 直发面；无 choice → emit 存活，走事件投影）。
ygg.on("bureau.handover", function(payload)
  local bar = (type(payload) == "string") and payload:find("|", 1, true) or nil
  if bar == nil then
    ygg.narrate("【拒绝】交接载荷应为 \"<道具key>|<持有者key>\"（JSON 标量）。")
    return
  end
  local item_key = payload:sub(1, bar - 1)
  local to_key = payload:sub(bar + 1)
  local ok, result = pcall(items.transfer, item_key, to_key)
  if not ok then
    ygg.narrate("【拒绝】交接被拒：" .. tostring(result))
    return
  end
  ygg.narrate("交接完成：" .. item_key .. " → " .. to_key .. "。")
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
    quests = {
      open = quests.open_count(),
      done = #quests.completed(),
      abandoned = select(3, quests.stats()),
    },
  })
  -- 对话线计（观察面：线程数/总行数；C2 人物系统投影）。
  local threads, lines = cast.threads_stat()
  ygg.meta("dlg", { threads = threads, lines = lines })
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

--- 信封线载荷 → 段数组（wire 段序 = fields；身份段 |actor= / |actor_session=
--- 由宿主注在最后，按前缀识别后剔除）。
local function wire_segments(payload)
  local out = {}
  for seg in (tostring(payload) .. "|"):gmatch("([^|]*)|") do
    out[#out + 1] = seg
  end
  -- 身份段剥离（宿主是唯一盖章方；段值形如 actor=<id>）。
  while #out > 0 do
    local last = out[#out]
    if last:find("actor=", 1, true) == 1 then
      out[#out] = nil
    else
      break
    end
  end
  return out
end

-- 信封 kind → wire 段名序（fields 声明序 = data/envelope.json kinds[].wire）。
local KIND_FIELDS = {}
for i = 1, #ENVELOPE.kinds do
  KIND_FIELDS[ENVELOPE.kinds[i].kind] = ENVELOPE.kinds[i].wire
end

--- 信封线载荷 → 字段表（按 kind 的 wire 序对位；缺段 = 空串）。
local function wire_fields(kind, payload)
  local segs = wire_segments(payload)
  local fields = KIND_FIELDS[kind] or {}
  local out = {}
  for i = 1, #fields do
    out[fields[i]] = segs[i] or ""
  end
  out._segments = segs
  return out
end

-- C4 接线：quest.custom（信封模板 → quest.define）。无 choice → emit 存活，
-- 机器可读投影在这里落（D1 语义）。
ygg.on("quest.custom", function(payload)
  local env = wire_fields("quest.custom", payload)
  local ok, result = pcall(custom.define, env.text)
  if not ok then
    ygg.narrate("【拒绝】自定义任务模板被拒：" .. tostring(result))
    ygg.emit("agent.error", (env.agent ~= "" and env.agent or "?") .. "|custom-rejected")
    return
  end
  ygg.narrate("自定义任务已定义：" .. result.name .. "（" .. tostring(result.steps) .. " 步，来源信封 " .. env.externalId .. "）。")
  ygg.emit("agent.recorded", "custom|" .. result.name .. "|" .. tostring(result.steps))
end)

for kind in pairs(INBOX_KINDS) do
  if kind ~= "quest.custom" then
    ygg.on(kind, function(payload)
      ygg.narrate("[INBOX-UNBOUND] " .. kind .. " 信封已入队但适配器未接线（C5 落地 agents.lua）：" .. tostring(payload))
      ygg.emit("agent.recorded", kind .. "|unbound")
    end)
  end
end

-- 出向工具回执（tools 段 resultEvent；C5 起 agents.lua 落账 → 交付）。
ygg.on("harness.replied", function(payload)
  ygg.narrate("[INBOX-UNBOUND] harness.replied 工具回执未接线（C5 落地）：" .. tostring(payload))
end)
