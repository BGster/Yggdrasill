-- define-advance：用例段自 pkg-quest.lua 原样搬移（S-25 批十三测试件连拆）。
return function(C)
  local S, check, eq, is_true, is_nil, expect_error, run, new_world, fresh, sorted_keys, events_named, event_names, names_equal, errand_spec, pkg_sources, count_in_sources =
    C.S, C.check, C.eq, C.is_true, C.is_nil, C.expect_error, C.run, C.new_world, C.fresh, C.sorted_keys, C.events_named, C.event_names, C.names_equal, C.errand_spec, C.pkg_sources, C.count_in_sources
run("pkg.quest.define-idempotent", function()
  local world = new_world()
  local quest = fresh(world)
  local spec = errand_spec()

  is_true(quest.define("errand", spec), "首次 define 必须返回 true")
  eq(world.kv["quest.types"], "errand", "类型名表")
  local writes_after_first = world.writes_now()
  local bytes_after_first = world.kv["quest.types"]

  is_true(quest.define("errand", spec), "同 spec 二次 define 必须返回 true")
  eq(world.writes_now(), writes_after_first, "同 spec 二次 define 必须是零写")
  eq(world.kv["quest.types"], bytes_after_first, "类型名表字节不变")

  -- 异 spec 不覆盖：换一步、换一个 meta 类型，两种都必须被拒。
  expect_error("YGG-PKG-QUEST-TYPE", function()
    quest.define("errand", errand_spec({
      { id = "accept", guide = "quests.errand.accept" },
      { id = "deliver", guide = "quests.errand.deliver" },
    }))
  end, "异 steps 覆盖被拒")
  expect_error("YGG-PKG-QUEST-TYPE", function()
    quest.define("errand", {
      steps = { { id = "accept", guide = "quests.errand.accept" } },
      meta_spec = { weight = "string" },
    })
  end, "异 meta_spec 覆盖被拒")
  eq(world.kv["quest.types"], bytes_after_first, "被拒的 define 不改类型名表")
  eq(quest.type("errand").meta_spec.weight, "number", "被拒的 define 不改 meta 类型词")

  -- 名字与模板形状的拒绝路径（断错误码，不断「没报错」）。
  expect_error("YGG-PKG-QUEST-NAME", function()
    quest.define("Errand", spec)
  end, "大写开头的类型名被拒")
  expect_error("YGG-PKG-QUEST-TEMPLATE", function()
    quest.define("empty", { steps = {} })
  end, "空 steps 被拒")
  expect_error("YGG-PKG-QUEST-TEMPLATE", function()
    quest.define("dup", {
      steps = { { id = "same" }, { id = "same" } },
    })
  end, "类型内重复步 id 被拒")
  expect_error("YGG-PKG-QUEST-TEMPLATE", function()
    quest.define("bogus", {
      steps = { { id = "one" } },
      meta_spec = { weird = "table" },
    })
  end, "词表外的 meta 类型被拒")
end)

run("pkg.quest.template-not-inherited", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("errand", errand_spec())
  local id = quest.open("errand")
  eq(quest.get(id).step, 1, "新实例从第 1 步起")

  -- I1：实例只存 step 下标，不复制步骤内容——KV 里没有 `guide` / `on_enter` 的副本。
  local inst_keys = {}
  for key in pairs(world.kv) do
    if key:find("quest.inst." .. id .. ".", 1, true) == 1 then
      inst_keys[#inst_keys + 1] = key
    end
  end
  table.sort(inst_keys)
  names_equal(inst_keys, {
    "quest.inst." .. id .. ".giver",
    "quest.inst." .. id .. ".m.names",
    "quest.inst." .. id .. ".state",
    "quest.inst." .. id .. ".step",
    "quest.inst." .. id .. ".taker",
    "quest.inst." .. id .. ".type",
  }, "实例键集")

  local before = {}
  for i = 1, #inst_keys do
    before[inst_keys[i]] = world.kv[inst_keys[i]]
  end

  -- 模拟「有人改了数据里的模板」：加一步、改第 1 步的指引键。实例键必须一字不动。
  world.kv["quest.t.errand.steps"] = 4
  world.kv["quest.t.errand.s.4.id"] = "epilogue"
  world.kv["quest.t.errand.s.4.guide"] = "quests.errand.epilogue"
  world.kv["quest.t.errand.s.4.on_enter"] = ""
  world.kv["quest.t.errand.s.1.guide"] = "quests.errand.accept.v2"

  for i = 1, #inst_keys do
    eq(world.kv[inst_keys[i]], before[inst_keys[i]], "实例键未随模板改动：" .. inst_keys[i])
  end
  -- 而读数按新模板解释——「同一份存档在新模板下的进度」由此可判。
  eq(quest.step(id).guide, "quests.errand.accept.v2", "step().guide 来自模板")
  eq(quest.get(id).step, 1, "实例自己的步下标没变")
end)

run("pkg.quest.id-monotonic", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("errand", errand_spec())

  local first = quest.open("errand")
  local second = quest.open("errand")
  local third = quest.open("errand")
  eq(first, 1, "首个 id")
  eq(second, 2, "第二个 id")
  eq(third, 3, "第三个 id")
  is_true(second > first and third > second, "id 严格递增")
  eq(world.kv["quest.seq"], 3, "seq = 已分配的最大 id")

  -- 放弃不回收 id（I1：只增不减、不复用）。
  quest.abandon(second, "dropped")
  eq(quest.get(second).state, "abandoned", "放弃后的状态")
  eq(quest.open("errand"), 4, "放弃后新开的 id 继续递增")
  eq(world.kv["quest.seq"], 4, "seq 随之推进")
  eq(quest.get(1).state, "open", "放弃别的任务不影响它")
end)

run("pkg.quest.advance-monotonic", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("errand", errand_spec())
  local id = quest.open("errand")

  -- I2：每次恰好 +1，返回值是推进后的 state。
  eq(quest.advance(id), "open", "非末步 advance 返回 open")
  eq(quest.get(id).step, 2, "advance 后步下标")
  eq(quest.advance(id), "open", "第二步 advance 返回 open")
  eq(quest.get(id).step, 3, "第二步后的步下标")

  -- 「无传步号入口」：advance 只收 id，多给的实参不改变推进量。
  local fourth = quest.open("errand")
  quest.advance(fourth, 99)
  eq(quest.get(fourth).step, 2, "advance 不接受步号实参（只 +1）")

  -- 步下标是 1 基，且 step() 的 index 与实例一致。
  eq(quest.step(fourth).index, 2, "step().index")
  eq(quest.step(fourth).id, "deliver", "step().id 来自模板")
end)

run("pkg.quest.advance-to-done", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("short", {
    steps = { { id = "one" }, { id = "two" } },
  })
  local id = quest.open("short")

  eq(quest.advance(id), "open", "第一步 advance")
  eq(quest.advance(id), "done", "末步 advance 转 done")
  eq(quest.get(id).state, "done", "实例状态落盘")

  -- 事件名恰好这三个（顺序也钉）。
  names_equal(event_names(world), {
    "quest.opened",
    "quest.advanced",
    "quest.completed",
  }, "事件序")
  eq(#events_named(world, "quest.completed"), 1, "completed 恰好一次")
  eq(events_named(world, "quest.completed")[1].payload, tostring(id), "completed 载荷 = id")
end)

-- ── 用例 6-12：终态、跳步与事件面 ────────────────────────────────────────────

run("pkg.quest.terminal-once", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("short", { steps = { { id = "one" }, { id = "two" } } })
  local id = quest.open("short")
  quest.advance(id)
  eq(quest.advance(id), "done", "到 done")
  local events_before = #world.events

  -- I3：终态后一切原语被拒，且**零事件**（重复设置终态会让奖励双发）。
  expect_error("YGG-PKG-QUEST-TERMINAL", function()
    quest.advance(id)
  end, "done 后 advance")
  expect_error("YGG-PKG-QUEST-TERMINAL", function()
    quest.jump(id, "one")
  end, "done 后 jump")
  expect_error("YGG-PKG-QUEST-TERMINAL", function()
    quest.abandon(id, "late")
  end, "done 后 abandon")
  expect_error("YGG-PKG-QUEST-TERMINAL", function()
    quest.assign(id, 7, 8)
  end, "done 后 assign")
  eq(#world.events, events_before, "被拒的调用不得产生事件")
  eq(quest.get(id).state, "done", "状态不得被改写")
  eq(quest.get(id).step, 2, "步下标不得被改写")

  -- abandoned 同样是终态（两态各自独立判）。
  local other = quest.open("short")
  quest.abandon(other, "cancelled")
  expect_error("YGG-PKG-QUEST-TERMINAL", function()
    quest.advance(other)
  end, "abandoned 后 advance")
end)

run("pkg.quest.abandon", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("errand", errand_spec())
  local id = quest.open("errand")

  is_true(quest.abandon(id, "giver-left"), "abandon 返回 true")
  eq(quest.get(id).state, "abandoned", "状态落盘")
  eq(#events_named(world, "quest.abandoned"), 1, "quest.abandoned 恰好一次")
  eq(events_named(world, "quest.abandoned")[1].payload, id .. "|giver-left", "载荷 = id|reason")

  -- reason 缺省是空串（载荷形状仍定形，不出现 nil）。
  local second = quest.open("errand")
  quest.abandon(second)
  eq(events_named(world, "quest.abandoned")[2].payload, second .. "|", "缺省 reason 落空串")

  -- 放弃的任务不再出现在 open_ids。
  eq(#quest.open_ids(), 0, "全部任务已放弃")
end)

run("pkg.quest.jump-audit", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("errand", errand_spec())
  local id = quest.open("errand")

  is_true(quest.jump(id, "reward"), "jump 到第三步")
  eq(quest.get(id).step, 3, "步下标被显式改写")
  eq(quest.get(id).state, "open", "跳步不进终态")
  eq(#events_named(world, "quest.jumped"), 1, "quest.jumped 恰好一次")
  eq(events_named(world, "quest.jumped")[1].payload, id .. "|1|3", "载荷 = id|from|to")

  -- 跳步留痕后仍可继续推进（末步 advance → done）。
  eq(quest.advance(id), "done", "跳到末步后 advance 转 done")
end)

run("pkg.quest.jump-unknown-step", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("errand", errand_spec())
  local id = quest.open("errand")
  local events_before = #world.events

  expect_error("YGG-PKG-QUEST-STEP", function()
    quest.jump(id, "no-such-step")
  end, "未知步 id")
  eq(quest.get(id).step, 1, "被拒的 jump 不改步下标")
  eq(#world.events, events_before, "被拒的 jump 不发事件")
  eq(#events_named(world, "quest.jumped"), 0, "没有 jumped 事件")
end)

end
