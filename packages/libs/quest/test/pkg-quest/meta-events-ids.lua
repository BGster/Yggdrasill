-- meta-events-ids：用例段自 pkg-quest.lua 原样搬移（S-25 批十三测试件连拆）。
return function(C)
  local S, check, eq, is_true, is_nil, expect_error, run, new_world, fresh, sorted_keys, events_named, event_names, names_equal, errand_spec, pkg_sources, count_in_sources =
    C.S, C.check, C.eq, C.is_true, C.is_nil, C.expect_error, C.run, C.new_world, C.fresh, C.sorted_keys, C.events_named, C.event_names, C.names_equal, C.errand_spec, C.pkg_sources, C.count_in_sources
run("pkg.quest.meta-not-in-flow", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("errand", errand_spec())

  -- I5：meta 里塞满三个键，推进结果与不塞时逐字段相同。
  local with_meta = quest.open("errand", {
    meta = { source = "guild", weight = 3, urgent = true, npc = 42 },
  })
  local bare = quest.open("errand")
  eq(quest.step(with_meta).id, quest.step(bare).id, "起始步相同")
  eq(quest.advance(with_meta), quest.advance(bare), "第一次推进的 state 相同")
  eq(quest.get(with_meta).step, quest.get(bare).step, "推进后的步下标相同")
  eq(quest.get(with_meta).state, quest.get(bare).state, "推进后的 state 相同")
  eq(quest.step(with_meta).id, quest.step(bare).id, "推进后读到的步 id 相同")
  eq(quest.advance(with_meta), quest.advance(bare), "第二次推进的 state 相同")

  -- meta 值本身原样可读（它只是数据，不参与判定）。
  eq(quest.get(with_meta).meta.source, "guild", "string meta")
  eq(quest.get(with_meta).meta.weight, 3, "number meta")
  eq(quest.get(with_meta).meta.urgent, true, "boolean meta")
  eq(quest.get(with_meta).meta.npc, 42, "ref meta")

  -- 事件载荷只有 id 与步下标——不含任何 meta 字段。
  local advanced = events_named(world, "quest.advanced")[1]
  eq(advanced.payload, with_meta .. "|2", "advanced 载荷只带 id 与步下标")
end)

run("pkg.quest.meta-spec-gate", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("errand", errand_spec())

  -- 未声明的键：拒绝（不是「忽略」——忽略会让存档里出现模板从不认识的字段）。
  expect_error("YGG-PKG-QUEST-META", function()
    quest.open("errand", { meta = { difficulty = 5 } })
  end, "未声明的 meta 键")
  -- 类型不符：四个词表逐一拒绝。
  expect_error("YGG-PKG-QUEST-META", function()
    quest.open("errand", { meta = { weight = "heavy" } })
  end, "number 位塞字符串")
  expect_error("YGG-PKG-QUEST-META", function()
    quest.open("errand", { meta = { urgent = 1 } })
  end, "boolean 位塞数字")
  expect_error("YGG-PKG-QUEST-META", function()
    quest.open("errand", { meta = { npc = 0 } })
  end, "ref 位塞 0")
  expect_error("YGG-PKG-QUEST-META", function()
    quest.open("errand", { meta = { npc = 2.5 } })
  end, "ref 位塞非整数")
  -- 被拒的 open 不分配 id（否则 seq 会被空转推进）。
  eq(world.kv["quest.seq"], nil, "被拒的 open 不写 seq")
  eq(#quest.open_ids(), 0, "被拒的 open 不产生实例")

  -- giver / taker 的 ref 校验走独立错误码。
  expect_error("YGG-PKG-QUEST-REF", function()
    quest.open("errand", { giver = -1 })
  end, "负 giver")
  expect_error("YGG-PKG-QUEST-REF", function()
    quest.open("errand", { taker = "7" })
  end, "字符串 taker")
end)

run("pkg.quest.events-exact", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("errand", errand_spec())

  local a = quest.open("errand")
  quest.advance(a)
  quest.jump(a, "reward")
  quest.advance(a)
  local b = quest.open("errand")
  quest.abandon(b, "nope")

  -- I6：「名字集合恰好相等」= 名字数组逐项相等（含次数与顺序），不是「包含」。
  names_equal(event_names(world), {
    "quest.opened",
    "quest.advanced",
    "quest.jumped",
    "quest.completed",
    "quest.opened",
    "quest.abandoned",
  }, "事件全序")
  eq(#events_named(world, "quest.opened"), 2, "opened 两次")
  eq(#events_named(world, "quest.advanced"), 1, "advanced 一次")
  eq(#events_named(world, "quest.jumped"), 1, "jumped 一次")
  eq(#events_named(world, "quest.completed"), 1, "completed 一次")
  eq(#events_named(world, "quest.abandoned"), 1, "abandoned 一次")

  -- 载荷全部是字符串（标量纪律），且 opened 的第二段就是类型名。
  for i = 1, #world.events do
    eq(type(world.events[i].payload), "string", "事件 " .. world.events[i].name .. " 的载荷是字符串")
  end
  eq(events_named(world, "quest.opened")[2].payload, b .. "|errand", "opened 载荷 = id|type")
end)

-- ── 静态面：源文件清单来自 manifest 的 packages 段（内容寻址的那份，不是目录扫描）──
--
-- 为什么不硬编码文件清单：包加一个模块就要改两处（实现 + 测试清单），那正是漂移。
-- 为什么不用目录扫描：沙箱外能扫，但「该在册的文件」的权威是 manifest——用名单表
-- 扫出来的是**可能漏登记**的影子集合。

--- 本包在册的源文件（相对包根），按 manifest 的 packages 段。

--- 在包的源文件里数字面量出现次数（会返回反例的形态：命中数 > 0 即可证伪）。

-- ── 用例 13-18：委托是引用、读面、无时钟、顶层纯度 ──────────────────────────

run("pkg.quest.no-entity-api", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("errand", errand_spec())

  -- I4：giver / taker 只是 ref，包不判存活——一个根本不存在的 id 照样能开。
  local id = quest.open("errand", { giver = 999999, taker = 1000000 })
  eq(quest.get(id).giver, 999999, "悬垂 giver 原样存")
  eq(quest.get(id).taker, 1000000, "悬垂 taker 原样存")
  names_equal(quest.open_ids(999999), { id }, "owner_ref 命中 giver")
  names_equal(quest.open_ids(1000000), { id }, "owner_ref 命中 taker")

  -- assign 只写两个 ref：换成另一个（同样不存在的）id 也不报错。
  is_true(quest.assign(id, 7, 8), "assign 返回 true")
  eq(quest.get(id).giver, 7, "giver 被改写")
  eq(quest.get(id).taker, 8, "taker 被改写")
  eq(world.entity_calls(), 0, "ygg.entity.* 调用次数必须为 0")
end)

run("pkg.quest.open-ids-ordered", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("errand", errand_spec())

  local ids = {}
  for i = 1, 5 do
    ids[i] = quest.open("errand")
  end
  quest.abandon(ids[2], "gone")
  quest.abandon(ids[4], "gone")

  -- 升序、只含 open（终态的不在列表里）。
  names_equal(quest.open_ids(), { ids[1], ids[3], ids[5] }, "open_ids 升序")
  eq(#quest.open_ids(), 3, "只剩三个开着的")
  -- 无参 = 不过滤（owner 缺省）；传一个不存在的 ref = 空表而不是 nil。
  names_equal(quest.open_ids(12345), {}, "不命中 owner 返回空表")
end)

run("pkg.quest.open-ids-owner", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("errand", errand_spec())

  local mine = quest.open("errand", { giver = 10 })
  local taken = quest.open("errand", { taker = 10 })
  local other = quest.open("errand", { giver = 11, taker = 12 })
  local loose = quest.open("errand")

  names_equal(quest.open_ids(10), { mine, taken }, "giver 或 taker 命中都算")
  names_equal(quest.open_ids(11), { other }, "giver 命中")
  names_equal(quest.open_ids(12), { other }, "taker 命中")
  names_equal(quest.open_ids(13), {}, "谁都不是")
  eq(#quest.open_ids(), 4, "不过滤时四个都在")
  is_true(loose > other, "id 递增")

  -- assign 换手后归属跟着走（过滤读的是 KV 里的现值，不是开单时的快照）。
  quest.assign(other, 10, nil)
  names_equal(quest.open_ids(10), { mine, taken, other }, "换手后归属更新")
  names_equal(quest.open_ids(11), {}, "旧 giver 不再命中")
end)

run("pkg.quest.step-reader", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("errand", errand_spec())
  local id = quest.open("errand")

  local first = quest.step(id)
  eq(first.index, 1, "index")
  eq(first.id, "accept", "步 id 来自模板")
  eq(first.guide, "quests.errand.accept", "guide 是文本键")
  is_nil(first.on_enter, "首步没有 on_enter")
  names_equal(sorted_keys(first), { "guide", "id", "index" }, "step 的键集（无 on_enter 时三枚）")

  quest.advance(id)
  local second = quest.step(id)
  eq(second.index, 2, "推进后的 index")
  eq(second.id, "deliver", "推进后的步 id")
  eq(second.on_enter, "quest.check.deliver", "on_enter 只是读数（不自动 emit）")
  names_equal(sorted_keys(second), { "guide", "id", "index", "on_enter" }, "step 的键集（有 on_enter 时四枚）")
  eq(#events_named(world, "quest.check.deliver"), 0, "on_enter 不得被自动发成事件")

  -- 不存在 = nil（不抛错：读面与写面的错误面不同）。
  is_nil(quest.step(999), "不存在的 id")
  is_nil(quest.get(999), "get 不存在的 id")
end)

end
