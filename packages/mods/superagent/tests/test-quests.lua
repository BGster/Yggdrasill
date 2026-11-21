-- `tests/test-quests.lua` —— 任务系统 + 自定义任务自检（C4）。
--
-- 断言面（计划 C4 冒烟口径「headless 事件序列走完 委托→执行→交付，quest.completed
-- 在场」+ R3 终态唯一 + R4 限期收口 + E2 翻译器）：
--   1. open_all：三单全开、giver 指到实体、taker 待定；
--   2. take/work/deliver 事件序列 → quest.completed 恰一条（I3）+ 报酬在场；
--   3. 逾期单自动弃（quest.abandoned 在场 + 统计 +1）；
--   4. custom.parse：合法描述符 → define 后类型可 open；非法各分支按码拒绝。

local world = require("src.world")
local quests = require("src.quests")
local custom = require("src.custom")
local pkg_quest = require("pkg.quest")
local pkg_role = require("pkg.role_skill")

-- 文件名序在 test-world 之后：世界已铺（setup 先行）；委托在此架设。
if ygg.kv.get("sg.comms") == nil then
  quests.open_all(world.world())
end

local function emitted_names()
  local out = {}
  local events = ygg_emitted_events()
  for i = 1, #events do
    out[#out + 1] = events[i].name
  end
  return out
end

local function count(list, needle)
  local n = 0
  for i = 1, #list do
    if list[i] == needle then
      n = n + 1
    end
  end
  return n
end

check("quests.open-all.opens-three-with-giver", function()
  local problems = {}
  local keys = quests.keys()
  if #keys ~= 3 then
    problems[#problems + 1] = "应开三单，实得 " .. tostring(#keys)
  end
  for i = 1, #keys do
    local snap = quests.snapshot(keys[i])
    if snap == nil then
      problems[#problems + 1] = keys[i] .. " 无快照"
    else
      if not snap.open or snap.taker ~= 0 then
        problems[#problems + 1] = keys[i] .. " 应 open 且待定"
      end
      local inst = pkg_quest.get(snap.id)
      if inst == nil or (inst.giver or 0) == 0 then
        problems[#problems + 1] = keys[i] .. " giver 应指到委托人实体"
      end
    end
  end
  return table.concat(problems, "; ")
end)

check("quests.full-flow.take-work-deliver-completes", function()
  local problems = {}
  -- 直发事件序（headless 冒烟同形）：接单 → 干活 → 交付。
  ygg_dispatch("commission.take", "c1|clerk:clerk-mo")
  local taken = quests.snapshot("c1")
  if taken == nil or taken.taker == 0 then
    problems[#problems + 1] = "接单后 c1 应有 taker"
  end
  ygg_dispatch("commission.work", "c1")
  local worked = quests.snapshot("c1")
  if worked == nil or worked.step ~= 2 then
    problems[#problems + 1] = "干活后 c1 应在 deliver 步（step=2），实得 "
      .. tostring(worked and worked.step)
  end
  ygg_dispatch("commission.deliver", "c1|按格式归档完毕")
  local done = quests.snapshot("c1")
  if done == nil or done.state ~= "done" then
    problems[#problems + 1] = "交付后 c1 应 done，实得 " .. tostring(done and done.state)
  end
  local names = emitted_names()
  if count(names, "quest.completed") ~= 1 then
    problems[#problems + 1] = "quest.completed 应恰一条（I3），实得 " .. tostring(count(names, "quest.completed"))
  end
  if count(names, "quest.advanced") < 2 then
    problems[#problems + 1] = "quest.advanced 应至少两条（work+deliver）"
  end
  -- 报酬：clerk-mo 应持有 filing（槽位在场）。
  local clerk = world.npc_id("clerk-mo")
  if clerk == nil or pkg_role.skill_state(clerk, "filing") == nil then
    problems[#problems + 1] = "交付报酬应给 clerk-mo 授 filing"
  end
  -- 业务文件应落盘（替身 VFS：挂载路径键在场）。
  local vfs = ygg_vfs()
  local landed = false
  for path in pairs(vfs) do
    if path:find("mounts/biz/", 1, true) == 1 then
      landed = true
      break
    end
  end
  if not landed then
    problems[#problems + 1] = "干活文件应写进 mounts/biz/"
  end
  return table.concat(problems, "; ")
end)

check("quests.deliver-twice-rejected-by-package", function()
  -- I3：终态唯一在包侧钉死——对 done 单再推进必须报错（mod 不吞）。
  local done = quests.snapshot("c1")
  if done == nil then
    return "c1 快照缺失"
  end
  local ok, err = pcall(pkg_quest.advance, done.id)
  if ok then
    return "done 单再 advance 应被包面拒绝"
  end
  if tostring(err):find("YGG-PKG-QUEST", 1, true) == nil then
    return "拒绝码应来自包面（YGG-PKG-QUEST-*），实得 " .. tostring(err)
  end
  return nil
end)

check("quests.deadline.abandons-overdue-taken", function()
  local problems = {}
  ygg_dispatch("commission.take", "c2|clerk:clerk-lin")
  -- 直接把 c2 的限期拨到过去（钟面操控，不靠真等业务日）。
  ygg.kv.set("sg.comm.c2.deadline", 1)
  local dropped = quests.enforce_deadlines(9)
  local hit = false
  for i = 1, #dropped do
    if dropped[i] == "c2" then
      hit = true
    end
  end
  if not hit then
    problems[#problems + 1] = "逾期在办单 c2 应被弃"
  end
  local names = emitted_names()
  if count(names, "quest.abandoned") ~= 1 then
    problems[#problems + 1] = "quest.abandoned 应恰一条，实得 " .. tostring(count(names, "quest.abandoned"))
  end
  local _, _, abandoned = quests.stats()
  if abandoned ~= 1 then
    problems[#problems + 1] = "弃单统计应 =1，实得 " .. tostring(abandoned)
  end
  return table.concat(problems, "; ")
end)

check("quests.board-phase.follows-flow", function()
  -- c1 done、c2 abandoned → 板面应停在 c3 的 take 相。
  local phase = quests.board_phase()
  if phase ~= "take" then
    return "板面应停在 take（c3 待定），实得 " .. tostring(phase)
  end
  return nil
end)

check("quests.taker-ext.spawns-agent-entity", function()
  local problems = {}
  ygg_dispatch("commission.take", "c3|ext:agent-7")
  local snap = quests.snapshot("c3")
  if snap == nil or snap.taker == 0 then
    problems[#problems + 1] = "外线接单后 c3 应有 taker 实体"
  else
    local name = ygg.entity.get_attribute(snap.taker, "name")
    if name ~= "agent-7" then
      problems[#problems + 1] = "外线实体 name 应为 agent-7，实得 " .. tostring(name)
    end
  end
  -- 二次引用同一 agent：复用缓存实体（不重复 spawn）。
  local ref1 = quests.taker_ref("ext:agent-7")
  local ref2 = quests.taker_ref("ext:agent-7")
  if ref1 ~= ref2 then
    problems[#problems + 1] = "同一外线 agent 应复用同一实体"
  end
  return table.concat(problems, "; ")
end)

check("quests.custom.parse-and-define", function()
  local problems = {}
  local parsed = custom.parse("patrol-check|scan:扫楼|fix:整改|report:交报告")
  if parsed.name ~= "patrol-check" or #parsed.steps ~= 3 then
    problems[#problems + 1] = "合法描述符解析错："
      .. tostring(parsed.name) .. "/" .. tostring(parsed.steps and #parsed.steps)
  end
  local defined = custom.define("patrol-check|scan:扫楼|fix:整改|report:交报告")
  if defined.steps ~= 3 then
    problems[#problems + 1] = "define 应记 3 步"
  end
  -- 定义后类型可开单（运行时定义是包面能力）。
  local qid = pkg_quest.open("patrol-check")
  if qid == nil then
    problems[#problems + 1] = "自定义类型应可 quest.open"
  end
  local inst = pkg_quest.get(qid)
  if inst == nil or inst.type ~= "patrol-check" then
    problems[#problems + 1] = "开出的实例类型应为 patrol-check"
  end
  -- 同 spec 重定义幂等（包面零写，不再 emit）。
  local before = #ygg_emitted_events()
  custom.define("patrol-check|scan:扫楼|fix:整改|report:交报告")
  if #ygg_emitted_events() ~= before then
    problems[#problems + 1] = "同 spec 重定义应为零写"
  end
  local found = false
  local customs = custom.customs()
  for i = 1, #customs do
    if customs[i] == "patrol-check" then
      found = true
    end
  end
  if not found then
    problems[#problems + 1] = "customs 名表应含 patrol-check"
  end
  return table.concat(problems, "; ")
end)

check("quests.custom.rejects-bad-descriptors", function()
  local cases = {
    { "patrol|scan", "缺冒号" },
    { "X|scan:扫楼", "类型名大写" },
    { "patrol-check", "无步骤段" },
    { "patrol-check|scan:一|scan:二", "步骤 id 重复" },
    { "patrol-check|s:guide\nmore", "含换行" },
  }
  local problems = {}
  for i = 1, #cases do
    local ok, err = pcall(custom.parse, cases[i][1])
    if ok then
      problems[#problems + 1] = cases[i][2] .. " 应被拒"
    elseif tostring(err):find("YGG-MOD-CUSTOM-FORMAT", 1, true) == nil then
      problems[#problems + 1] = cases[i][2] .. " 拒绝码应为 YGG-MOD-CUSTOM-FORMAT，实得 " .. tostring(err)
    end
  end
  return table.concat(problems, "; ")
end)
