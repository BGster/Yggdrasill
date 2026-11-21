-- `tests/test-cast.lua` —— 人物系统自检（C2）。
--
-- 断言面（计划 C2 冒烟口径「NPC 对话一回合，thread 增行」+ 报酬成长 + 台账）：
--   1. talk 一回合 → cast.<key> 线程恰增一行，行文带说话人前缀；
--   2. 真包面：行真的落在 pkg.dialogue 的 KV（threads()/history() 读回）；
--   3. reward 首授 = granted，二授 = recharged（有界），effective 合成可读；
--   4. 承接登记计数 chats/deliveries 独立累加。

local cast = require("src.cast")
local world = require("src.world")
local pkg_dialogue = require("pkg.dialogue")
local pkg_role = require("pkg.role_skill")

-- 文件按名表序执行（test-cast 先于 test-world）：人物用例自铺场（seed 幂等）。
if ygg.kv.get("sg.seeded") == nil then
  world.seed()
end

check("cast.talk.appends-one-line-per-turn", function()
  local problems = {}
  local before = #cast.history("archivist")
  if before ~= 0 then
    problems[#problems + 1] = "开局线程应为空，实得 " .. tostring(before)
  end
  local after = cast.talk("archivist", "文姝", "三号档案要取回。")
  if after ~= before + 1 then
    problems[#problems + 1] = "一回合应恰增一行，实得 " .. tostring(after)
  end
  local history = cast.history("archivist")
  local line = history[#history]
  local text = (line ~= nil and line.text) or tostring(line)
  if text:find("文姝: 三号档案要取回。", 1, true) == nil then
    problems[#problems + 1] = "行文应带说话人前缀，实得 " .. text
  end
  if line == nil or line.who ~= 0 then
    problems[#problems + 1] = "系统行 who 应为 0"
  end
  return table.concat(problems, "; ")
end)

check("cast.thread.registered-in-dialogue-package", function()
  local problems = {}
  local keys = pkg_dialogue.threads()
  local found = false
  for i = 1, #keys do
    if keys[i] == "cast.archivist" then
      found = true
      break
    end
  end
  if not found then
    problems[#problems + 1] = "cast.archivist 应出现在 pkg.dialogue 线程名表"
  end
  local threads, lines = cast.threads_stat()
  if threads < 1 or lines < 1 then
    problems[#problems + 1] = "threads_stat 应见至少一线一行"
  end
  return table.concat(problems, "; ")
end)

check("cast.reward.grants-then-recharges", function()
  local problems = {}
  local clerk = world.npc_id("clerk-mo")
  if clerk == nil then
    return "在局文员应已注册"
  end
  local first = cast.reward(clerk, "filing")
  if first ~= "granted:filing" then
    problems[#problems + 1] = "首授应 granted:filing，实得 " .. tostring(first)
  end
  if pkg_role.skill_state(clerk, "filing") == nil then
    problems[#problems + 1] = "授后应持有 filing（技能槽位在场）"
  end
  local second = cast.reward(clerk, "filing")
  if second:find("recharged:filing=", 1, true) == nil then
    problems[#problems + 1] = "二授应回满格，实得 " .. tostring(second)
  end
  local state = pkg_role.skill_state(clerk, "filing")
  if state == nil or state.rank == nil then
    problems[#problems + 1] = "技能槽位应可读"
  end
  return table.concat(problems, "; ")
end)

check("cast.agent-ledger.counts-independently", function()
  local problems = {}
  local chats = cast.touch_agent("agent-x", "chats")
  local deliveries = cast.touch_agent("agent-x", "deliveries")
  if chats ~= 1 or deliveries ~= 1 then
    problems[#problems + 1] = "首记应各为 1"
  end
  cast.touch_agent("agent-x", "chats")
  local chats2, deliveries2 = cast.agent_stat("agent-x")
  if chats2 ~= 2 or deliveries2 ~= 1 then
    problems[#problems + 1] = "二记 chats 后应 2|1，实得 " .. tostring(chats2) .. "|" .. tostring(deliveries2)
  end
  local zero_chats, zero_deliveries = cast.agent_stat("agent-never-seen")
  if zero_chats ~= 0 or zero_deliveries ~= 0 then
    problems[#problems + 1] = "无记录 agent 应读 0|0"
  end
  return table.concat(problems, "; ")
end)

check("cast.role-growth.visible-in-effective", function()
  local problems = {}
  local clerk = world.npc_id("clerk-lin")
  cast.reward(clerk, "bookkeeping")
  -- bureau.clerk 的 attrs_spec diligence=2 + bookkeeping 的 order=1：合成面可读即过。
  local diligence = pkg_role.effective(clerk, "diligence")
  if diligence == nil or diligence < 2 then
    problems[#problems + 1] = "effective(diligence) 应 >= 基础 2，实得 " .. tostring(diligence)
  end
  return table.concat(problems, "; ")
end)
