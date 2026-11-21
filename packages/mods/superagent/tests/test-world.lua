-- `tests/test-world.lua` —— 开局铺场自检（C1 落锁用例）。
--
-- 断言面：seed 幂等、批帧真解码（rows = roster 数）、实体注册齐、种子确定性。
-- 本文件跑在 harness.lua 的替身上（pkg.* 是真包，sim.spawn_batch 是**真解码**
-- mock）——帧布局错一行这里就红。模块经 require 直取（替身上没有 ygg.world 假面）。

local world = require("src.world")
local clock = require("src.clock")
local simframe = require("src.simframe")

check("world.registry.seeded-roster-readable", function()
  -- 开局派发与铺场断言归 test-a-setup（文件名序先行）；这里验已铺态的不变量。
  local problems = {}
  if ygg.kv.get("sg.seeded") == nil then
    return "世界应已铺（test-a-setup 先行）"
  end
  if #world.npc_keys() ~= 5 then
    problems[#problems + 1] = "NPC 名表应为 5，实得 " .. tostring(#world.npc_keys())
  end
  if #world.item_keys() ~= 4 then
    problems[#problems + 1] = "道具名表应为 4，实得 " .. tostring(#world.item_keys())
  end
  if ygg.kv.get("sg.seed") ~= "282ab700b111a01a" then
    problems[#problems + 1] = "种子漂移：" .. tostring(ygg.kv.get("sg.seed"))
  end
  local archivist = world.npc_id("archivist")
  if archivist == nil then
    problems[#problems + 1] = "archivist 应已注册"
  else
    local name = ygg.entity.get_attribute(archivist, "name")
    if name ~= "文姝" then
      problems[#problems + 1] = "实体属性读回应命中（name），实得 " .. tostring(name)
    end
  end
  return table.concat(problems, "; ")
end)

check("world.seed.idempotent-on-second-call", function()
  local problems = {}
  local before = #ygg_narrated_lines()
  local report = world.seed()
  if report.replay ~= true then
    problems[#problems + 1] = "二次 seed 应走幂等回放分支"
  end
  if #ygg_spawn_batches() ~= 1 then
    problems[#problems + 1] = "幂等分支不得再发批帧"
  end
  if #ygg_narrated_lines() ~= before then
    problems[#problems + 1] = "幂等分支不得叙述新行"
  end
  return table.concat(problems, "; ")
end)

check("world.seed.double-entry-rejected-by-guard", function()
  -- 开局守卫：sg.seeded 已在场 → assert_not_seeded 必须拒绝重入。
  local ok, err = pcall(world.assert_not_seeded)
  if ok then
    return "已铺局上 assert_not_seeded 应报错"
  end
  if tostring(err):find("YGG-MOD-WORLD-SEEDED", 1, true) == nil then
    return "拒绝码应为 YGG-MOD-WORLD-SEEDED，实得 " .. tostring(err)
  end
  return nil
end)

check("world.clock.advances-day-and-sim", function()
  local problems = {}
  local day_before = clock.day()
  local ticks_before = ygg.sim.tick()
  local day, degraded = clock.advance()
  if degraded then
    problems[#problems + 1] = "替身挂了 sim，advance 不应降级"
  end
  if day ~= day_before + 1 then
    problems[#problems + 1] = "业务日应 +1"
  end
  if ygg.sim.tick() ~= ticks_before + 1 then
    problems[#problems + 1] = "sim tick 应 +1"
  end
  if clock.sim_hash():find("mockhash", 1, true) == nil then
    problems[#problems + 1] = "sim_hash 读数异常：" .. tostring(clock.sim_hash())
  end
  return table.concat(problems, "; ")
end)

check("world.board.presented-on-entry", function()
  local choice = ygg_last_choice()
  if choice == nil then
    return "event.start 后应有待决任务板"
  end
  local found_ask, found_close = false, false
  for i = 1, #choice.options do
    if choice.options[i] == "向委托人问询" then
      found_ask = true
    end
    if choice.options[i] == "下班收工" then
      found_close = true
    end
  end
  if not (found_ask and found_close) then
    return "任务板应有 问询/收工 两支，实得 " .. table.concat(choice.options, ",")
  end
  return nil
end)

check("world.simframe.fnv-matches-published-vectors", function()
  local problems = {}
  if simframe.digest("") ~= 0xCBF29CE484222325 then
    problems[#problems + 1] = "FNV 空串向量不符"
  end
  if simframe.digest("a") ~= 0xAF63DC4C8601EC8C then
    problems[#problems + 1] = "FNV 'a' 向量不符"
  end
  if simframe.digest("foobar") ~= 0x85944171F73967E8 then
    problems[#problems + 1] = "FNV 'foobar' 向量不符"
  end
  local frame = simframe.despawn_one(7)
  local n = frame:byte(1) | (frame:byte(2) << 8) | (frame:byte(3) << 16) | (frame:byte(4) << 24)
  local id = frame:byte(5)
  if n ~= 1 or id ~= 7 or #frame ~= 4 + 8 + 8 then
    problems[#problems + 1] = "despawn 帧形错（n/id/长度）"
  end
  return table.concat(problems, "; ")
end)

check("world.join.roundtrips-name-table", function()
  local problems = {}
  local names = world.npc_keys()
  if #names == 0 then
    problems[#problems + 1] = "npc_keys 不应为空"
  end
  for i = 2, #names do
    if names[i - 1] > names[i] then
      problems[#problems + 1] = "名表应升序"
      break
    end
  end
  local joined = world.join(names, "\n")
  local back = world.name_table(joined)
  if #back ~= #names then
    problems[#problems + 1] = "名表往返行数不等"
  end
  return table.concat(problems, "; ")
end)
