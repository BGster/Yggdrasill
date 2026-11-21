-- `tests/test-a-setup.lua` —— 开局一次性架设（文件名首序：先于其余测试执行）。
--
-- 唯一派发 `event.start` 的地方：世界铺场 + 任务板首呈。后续文件（cast/world）
-- 建立在这个已铺局上；`--filter` 单跑某文件时，cast 的兜底铺场分支会接住
-- （直接 world.seed()，不走开局守卫）。

check("setup.event-start.seeds-and-presents-board", function()
  local problems = {}
  local report = ygg_dispatch("event.start", "0")
  if report == nil then
    return "event.start 无返回报告"
  end
  if (report.sim_spawned or 0) ~= 9 then
    problems[#problems + 1] = "sim 铺场数应为 9（3 NPC+2 文员+4 道具），实得 "
      .. tostring(report.sim_spawned)
  end
  if #report.npcs ~= 5 then
    problems[#problems + 1] = "NPC 注册应为 5，实得 " .. tostring(#report.npcs)
  end
  if #report.items ~= 4 then
    problems[#problems + 1] = "道具注册应为 4，实得 " .. tostring(#report.items)
  end
  local batches = ygg_spawn_batches()
  if #batches ~= 1 or batches[1] ~= 9 then
    problems[#problems + 1] = "spawn_batch 应恰一批 9 行，实得 " .. tostring(#batches) .. " 批"
  end
  -- 种子确定性：同数据同摘要（282ab700b111a01a 是 world.json 的既证值）。
  if report.seed ~= "282ab700b111a01a" then
    problems[#problems + 1] = "种子漂移：" .. tostring(report.seed)
  end
  -- 任务板首呈（D1 语义：开局 handler 只叙述不 emit）。
  local choice = ygg_last_choice()
  if choice == nil then
    problems[#problems + 1] = "event.start 后应有待决任务板"
  else
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
      problems[#problems + 1] = "任务板应有 问询/收工 两支，实得 " .. table.concat(choice.options, ",")
    end
  end
  return table.concat(problems, "; ")
end)

check("setup.emits-silence-in-choice-handlers", function()
  -- D1 语义的本 mod 落法：带任务板的 handler 不发**本 mod 投影名**的 emit
  -- （宿主会丢弃并告警）。包侧派生（entity.created 等）不归本条管。
  local events = ygg_emitted_events()
  local leaked = {}
  for i = 1, #events do
    local name = events[i].name
    if name:find("agent.", 1, true) == 1 or name:find("item.", 1, true) == 1 then
      leaked[#leaked + 1] = name .. "|" .. tostring(events[i].payload)
    end
  end
  if #leaked > 0 then
    return "开局不应留 mod 投影 emit（D1：choice 前被宿主丢弃）：" .. table.concat(leaked, "; ")
  end
  return nil
end)
