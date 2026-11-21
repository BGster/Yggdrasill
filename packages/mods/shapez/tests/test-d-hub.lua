-- `tests/test-d-hub.lua` —— 交付判定与过级（hub_goals v1 面）。
--
-- 覆盖：非目标形状收下不计数；目标命中计数；满额过级（KV 换级 + 新 quest
-- 实例 + 旧实例 done 恰一次 I3）；空项不收；关卡表打完仍收不计数；
-- snapshot 形状。

local hub = require("src.hub")
local quest = require("pkg.quest")

local WORLD = ygg.data("ygg:shapez/world")

--- 交付 n 个给定形状。
local function deliver_n(key, n)
  local last
  for _ = 1, n do
    last = hub.deliver(WORLD, key)
  end
  return last
end

check("hub.setup-and-goal", function()
  reset_all()
  ygg_dispatch("event.start")
  local goal = hub.current_goal(WORLD)
  assert_eq(goal.level, 1, "1 级")
  assert_eq(goal.shape, "CuCuCuCu", "源 regular.js level1 目标")
  assert_eq(goal.required, 10, "需 10 件")
  assert_eq(goal.got, 0, "零计数")
  assert_true(goal.quest_id ~= nil, "关卡 quest 实例在场")
end)

check("hub.non-goal-accepted-not-counted", function()
  reset_all()
  ygg_dispatch("event.start")
  local r = hub.deliver(WORLD, "RuRuRuRu")
  assert_true(r.accepted, "收下")
  assert_true(not r.counted, "不计数")
  assert_eq(hub.current_goal(WORLD).got, 0, "计数不动")
  local r2 = hub.deliver(WORLD, "")
  assert_true(not r2.accepted, "空项不收")
end)

check("hub.levelup-2-and-quest-i3", function()
  reset_all()
  ygg_dispatch("event.start")
  local goal1 = hub.current_goal(WORLD)
  local last = deliver_n("CuCuCuCu", 10)
  assert_true(last.levelup, "第 10 件触发过级")
  local goal2 = hub.current_goal(WORLD)
  assert_eq(goal2.level, 2, "换 2 级")
  assert_eq(goal2.shape, "----CuCu", "源 regular.js level2 目标")
  assert_eq(goal2.required, 20, "需 20 件")
  assert_eq(goal2.got, 0, "计数归零")
  assert_true(goal2.quest_id ~= goal1.quest_id, "新 quest 实例")
  -- I3：旧实例 done 恰一次
  local old = quest.get(goal1.quest_id)
  assert_eq(old.state, "done", "旧实例终态 done")
  -- quest.completed 恰一次（包面 emit + sz.levelup 各一次）
  local completed, levelup = 0, 0
  for _, e in ipairs(ygg_emitted_events()) do
    if e.name == "quest.completed" then
      completed = completed + 1
    end
    if e.name == "sz.levelup" then
      levelup = levelup + 1
    end
  end
  assert_eq(completed, 1, "quest.completed 恰一次")
  assert_eq(levelup, 1, "sz.levelup 恰一次")
  -- sz.delivered 恰 10 次（每件目标命中一条）
  local delivered = 0
  for _, e in ipairs(ygg_emitted_events()) do
    if e.name == "sz.delivered" then
      delivered = delivered + 1
    end
  end
  assert_eq(delivered, 10, "sz.delivered 十次")
end)

check("hub.exhausted-levels-still-accept", function()
  reset_all()
  ygg_dispatch("event.start")
  -- 1→2→3：两级全交付后到 3 级（RuRuRuRu×30）
  deliver_n("CuCuCuCu", 10)
  deliver_n("----CuCu", 20)
  local goal3 = hub.current_goal(WORLD)
  assert_eq(goal3.level, 3, "3 级")
  assert_eq(goal3.shape, "RuRuRuRu", "3 级目标")
  -- 一路交付到底（3/4/5 级全清）
  deliver_n("RuRuRuRu", 30)
  deliver_n("RuRu----", 30)
  local last = deliver_n("Cu----Cu", 75)
  assert_true(last.levelup, "末级过级")
  local goal_after = hub.current_goal(WORLD)
  assert_true(goal_after == nil, "关卡表打完 → 无目标")
  local r = hub.deliver(WORLD, "CuCuCuCu")
  assert_true(r.accepted, "打完后仍收")
  assert_true(not r.counted, "打完后不计数")
end)

check("hub.snapshot-shape", function()
  reset_all()
  ygg_dispatch("event.start")
  hub.place(4, 2, 0)
  local snap = hub.snapshot()
  assert_eq(snap.level, 1, "level 在案")
  assert_eq(snap.got, 0, "got 在案")
  assert_true(snap.hub ~= nil, "hub 实体 id 在案")
  assert_true(snap.quest ~= nil, "quest 实例 id 在案")
end)
