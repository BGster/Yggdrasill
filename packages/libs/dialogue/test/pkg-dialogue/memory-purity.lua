-- memory-purity：用例段自 pkg-dialogue.lua 原样搬移（S-25 批十三测试件连拆）。
return function(C)
  local S, check, eq, is_true, is_nil, expect_error, run, MODULES, new_world, fresh, sorted_keys, events_named, event_names, names_equal, builtin, agent, pkg_sources, count_in_sources =
    C.S, C.check, C.eq, C.is_true, C.is_nil, C.expect_error, C.run, C.MODULES, C.new_world, C.fresh, C.sorted_keys, C.events_named, C.event_names, C.names_equal, C.builtin, C.agent, C.pkg_sources, C.count_in_sources
run("pkg.dialogue.memory-none", function()
  local world = new_world()
  local dialogue = fresh(world)
  dialogue.on("shout", builtin("npc", { memory = "none" }, function()
    return true
  end))

  local receipt = dialogue.emit("shout", 9, 10, { text_key = "beats.shout" })
  eq(receipt.handled, true, "handler 仍被调")
  eq(#dialogue.history("9:10"), 0, "没有线程历史")
  for key in pairs(world.kv) do
    is_true(key:find("^dlg%.thread%.") == nil, "memory=none 不得写线程键：" .. key)
  end
  is_nil(world.kv["dlg.keys"], "没有线程名表")
  is_nil(world.kv["dlg.seq"], "没有轮次推进")
end)

run("pkg.dialogue.memory-entity", function()
  local world = new_world()
  local dialogue = fresh(world)
  dialogue.on("greet", builtin("npc", { memory = "entity" }, function()
    return true
  end))

  dialogue.emit("greet", 5, 6, { text_key = "beats.one" })
  dialogue.emit("greet", 5, 7, { text_key = "beats.two" })

  -- entity 模式：键 = "<source>"，两次对同一 source 的对话进同一条线程。
  local turns = dialogue.history("5")
  eq(#turns, 2, "entity 模式按 source 聚合")
  eq(turns[1].text, "beats.one", "第一轮")
  eq(turns[2].text, "beats.two", "第二轮")
  eq(world.kv["dlg.keys"], "5", "线程名表里只有 source 键")
  eq(#dialogue.history("5:6"), 0, "pair 键不存在")
  names_equal(dialogue.threads(), { "5" }, "threads() 名表")
end)

-- ── 静态面：源文件清单来自 manifest 的 packages 段（内容寻址的那份）─────────────



-- ── 用例 11-16：无时钟、通道封闭、凭据只留键名、帧、adapter、文本键 ──────────

run("pkg.dialogue.no-clock", function()
  local world = new_world()
  local dialogue = fresh(world)
  dialogue.on("greet", builtin("npc", { memory = "none" }, function()
    return true
  end))

  -- I5：静态面——包源文件里一次 `ticks(` 都没有。
  local hits, where = count_in_sources("dialogue", "ticks(")
  eq(hits, 0, "包源码里的 ticks( 命中数： " .. table.concat(where, ","))

  -- 行为面——把 mock 时钟推到 12345，回执与之一致，且一次都没读它。
  world.tick = 12345
  local receipt = dialogue.emit("greet", 1, 2)
  eq(receipt.handled, true, "结果与时钟无关")
  eq(receipt.depth, 1, "深度与时钟无关")
  eq(world.ticks_calls(), 0, "ygg.ticks 调用次数必须为 0")
end)

run("pkg.dialogue.no-second-egress", function()
  local world = new_world()
  local dialogue = fresh(world)

  -- I1：静态面——四类字面量命中数全为 0（每一条都能返回反例）。
  local needles = {
    "http",
    "socket",
    "os.execute",
    "io.",
  }
  for i = 1, #needles do
    local hits, where = count_in_sources("dialogue", needles[i])
    eq(hits, 0, "静态命中 " .. needles[i] .. "： " .. table.concat(where, ","))
  end

  -- 运行时面——agent 走 `ygg.tool.call`，其它宿主面调用计数全 0。
  world.tool_replies["npc.agent"] = { ok = true, unmounted = false, code = 0, data = {} }
  dialogue.on("greet", agent("npc-agent", { tool = "npc.agent" }))
  local receipt = dialogue.emit("greet", 1, 2, { text_key = "beats.greet" })
  eq(receipt.handled, true, "agent 认领")
  eq(world.tool_call_count(), 1, "ygg.tool.call 是唯一外部调用")
  eq(world.entity_calls(), 0, "ygg.entity 调用次数")
  eq(world.data_calls(), 0, "ygg.data 调用次数")
  eq(world.narrative_calls(), 0, "ygg.narrative 调用次数（节点图归宿主，包不碰）")
end)

run("pkg.dialogue.credential-key-only", function()
  local world = new_world()
  local dialogue = fresh(world)
  local SECRET = "sk-live-DO-NOT-LEAK"
  world.tool_replies["npc.agent"] = { ok = true, unmounted = false, code = 0, data = {} }
  dialogue.on("greet", agent("npc-agent", { tool = "npc.agent", credential_key = "NPC_SHOPKEEPER_TOKEN" }))

  dialogue.emit("greet", 1, 2, { text_key = "beats.greet" })
  local call = world.tool_calls[1]
  eq(call.args.credential_key, "NPC_SHOPKEEPER_TOKEN", "工具实参里只有键名")
  -- I6：凭据值不出现在任何 KV 值里（哪怕调用方把它塞进 payload）。
  for key in pairs(world.kv) do
    local value = world.kv[key]
    is_true(tostring(value):find(SECRET, 1, true) == nil, "KV 里出现了凭据值：" .. key)
  end
  for i = 1, #world.events do
    is_true(tostring(world.events[i].payload):find(SECRET, 1, true) == nil, "事件里出现了凭据值")
  end

  -- 键名形态是硬约束（.env 的键名形态）；非法形态在注册期就拒。
  expect_error("YGG-PKG-DIALOGUE-TYPE", function()
    dialogue.on("greet", agent("bad-key", { tool = "npc.agent", credential_key = "npc-token" }))
  end, "小写 credential_key")
  expect_error("YGG-PKG-DIALOGUE-TYPE", function()
    dialogue.on("greet", agent("bad-key2", { tool = "npc.agent", credential_key = SECRET }))
  end, "把凭据值当键名")
end)

end
