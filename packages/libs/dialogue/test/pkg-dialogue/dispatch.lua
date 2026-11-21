-- dispatch：用例段自 pkg-dialogue.lua 原样搬移（S-25 批十三测试件连拆）。
return function(C)
  local S, check, eq, is_true, is_nil, expect_error, run, MODULES, new_world, fresh, sorted_keys, events_named, event_names, names_equal, builtin, agent, pkg_sources, count_in_sources =
    C.S, C.check, C.eq, C.is_true, C.is_nil, C.expect_error, C.run, C.MODULES, C.new_world, C.fresh, C.sorted_keys, C.events_named, C.event_names, C.names_equal, C.builtin, C.agent, C.pkg_sources, C.count_in_sources
run("pkg.dialogue.on-off-shape", function()
  local world = new_world()
  local dialogue = fresh(world)
  dialogue.on("greet", builtin("npc", { memory = "entity", window = 3 }, function()
    return true
  end))

  -- KV 面只有 spec：名表 + kind + opts 名表 + opts 值。**没有函数**（mock 的 set 会拒）。
  local specs = dialogue.handlers("greet")
  eq(#specs, 1, "handler 数量")
  eq(specs[1].kind, "builtin", "kind 落 KV")
  eq(specs[1].name, "npc", "name 落 KV")
  names_equal(sorted_keys(specs[1].opts), { "memory", "window" }, "opts 键集")
  eq(specs[1].opts.memory, "entity", "opts.memory 落 KV")
  eq(specs[1].opts.window, 3, "opts.window 落 KV")
  for key in pairs(world.kv) do
    local kind = type(world.kv[key])
    is_true(kind == "number" or kind == "string" or kind == "boolean", "KV 值非标量：" .. key)
  end

  -- off 后条目从名表消失（KV 里的墓碑键留在原处——宿主没有删除面）。
  is_true(dialogue.off("greet", "npc"), "off 命中")
  eq(#dialogue.handlers("greet"), 0, "off 后 handler 列表为空")
  eq(dialogue.off("greet", "npc"), false, "二次 off 返回 false")
  eq(world.kv["dlg.types"], "", "最后一个 handler 摘掉后 type 名表为空串（KV 无删除面）")

  -- 名与 kind 的拒绝路径。
  expect_error("YGG-PKG-DIALOGUE-TYPE", function()
    dialogue.on("Greet", builtin("npc", {}, function() end))
  end, "大写 type")
  expect_error("YGG-PKG-DIALOGUE-TYPE", function()
    dialogue.on("greet", { kind = "remote", name = "npc" })
  end, "词表外的 kind")
  expect_error("YGG-PKG-DIALOGUE-TYPE", function()
    dialogue.on("greet", { kind = "builtin", name = "bad name" })
  end, "名字带空格")
  expect_error("YGG-PKG-DIALOGUE-WINDOW", function()
    dialogue.on("greet", builtin("w0", { window = 0 }, function() end))
  end, "window=0")
end)

run("pkg.dialogue.order-is-registration", function()
  local world = new_world()
  local dialogue = fresh(world)
  local first_calls, second_calls = 0, 0
  dialogue.on("greet", builtin("first", {}, function()
    first_calls = first_calls + 1
    return true
  end))
  dialogue.on("greet", builtin("second", {}, function()
    second_calls = second_calls + 1
    return true
  end))

  -- I7：注册序即执行序，第一个认领的终止链。
  local receipt = dialogue.emit("greet", 1, 2, { text_key = "beats.greet" })
  eq(receipt.by, "first", "第一个注册的认领")
  eq(first_calls, 1, "first 调用次数")
  eq(second_calls, 0, "second 一次都没被调")

  -- 摘掉 first 后 order 前移到 second（名表序，不是数字下标）。
  dialogue.off("greet", "first")
  local second_receipt = dialogue.emit("greet", 1, 2)
  eq(second_receipt.by, "second", "摘掉 first 后由 second 认领")
  eq(second_calls, 1, "second 调用次数")

  -- 同名重复注册 = -DUP（不是「后覆盖前」：那会让注册序随装载序漂）。
  expect_error("YGG-PKG-DIALOGUE-DUP", function()
    dialogue.on("greet", builtin("second", {}, function() end))
  end, "同名重复注册")
end)

run("pkg.dialogue.emit-receipt", function()
  local world = new_world()
  local dialogue = fresh(world)
  dialogue.on("greet", builtin("npc", { memory = "none" }, function()
    return true
  end))

  local receipt = dialogue.emit("greet", 7, 8, { text_key = "beats.greet" })
  names_equal(sorted_keys(receipt), { "by", "depth", "handled" }, "回执键集恰好三个")
  eq(receipt.handled, true, "handled")
  eq(receipt.by, "npc", "by")
  eq(receipt.depth, 1, "顶层链的深度 = 1")

  -- 帧形状（d14 的同一断言面，这里先钉「不泄漏进回执」）。
  local frame = dialogue.frame("greet", 7, 8)
  names_equal(sorted_keys(frame), { "payload", "source", "target", "type", "v" }, "帧键集")
  eq(frame.v, 1, "帧版本")
  names_equal(sorted_keys(frame.payload), {}, "缺省 payload = 空表")
end)

run("pkg.dialogue.fallback-chain", function()
  local world = new_world()
  local dialogue = fresh(world)
  world.tool_replies["npc.agent"] = { ok = false, unmounted = false, code = 1, error = "timeout after 900ms" }
  local builtin_calls = 0
  dialogue.on("greet", agent("npc-agent", { tool = "npc.agent", credential_key = "NPC_TOKEN" }))
  dialogue.on("greet", builtin("npc-fallback", {}, function()
    builtin_calls = builtin_calls + 1
    return true
  end))

  local receipt = dialogue.emit("greet", 1, 2, { text_key = "beats.greet" })
  eq(receipt.by, "npc-fallback", "agent 失败后回退到 builtin")
  eq(receipt.handled, true, "回退后仍算已处理")
  -- 跳数 = 2（agent 一次 + builtin 一次），不是「调用成功」那么弱。
  eq(world.tool_call_count(), 1, "agent 跳数")
  eq(builtin_calls, 1, "builtin 跳数")
  eq(#events_named(world, "dialogue.failed"), 0, "回退成功不得发 failed")
end)

-- ── 用例 5-10：失败面、深度帽、记忆窗口 ─────────────────────────────────────

run("pkg.dialogue.failed-event", function()
  local world = new_world()
  local dialogue = fresh(world)
  world.tool_replies["npc.agent"] = { ok = false, unmounted = false, code = 1, error = "boom-42" }
  dialogue.on("greet", agent("npc-agent", { tool = "npc.agent" }))

  local receipt = dialogue.emit("greet", 3, 4, { text_key = "beats.greet" })
  eq(receipt.handled, false, "无人认领")
  is_nil(receipt.by, "by 为空")
  eq(#events_named(world, "dialogue.failed"), 1, "dialogue.failed 恰好一次")
  local payload = events_named(world, "dialogue.failed")[1].payload
  eq(type(payload), "string", "载荷是字符串（标量纪律）")
  eq(payload, "greet|boom-42", "载荷 = <type>|<原错误>")
  -- I2：失败不静默吞——错误文本原样挂在载荷里。
  is_true(payload:find("boom-42", 1, true) ~= nil, "载荷带原错误")
end)

run("pkg.dialogue.agent-unmounted", function()
  local world = new_world()
  local dialogue = fresh(world)
  world.tool_replies["npc.agent"] = { ok = false, unmounted = true, code = -1, error = "no host tool runner mounted" }
  local builtin_calls = 0
  dialogue.on("greet", agent("npc-agent", { tool = "npc.agent" }))
  dialogue.on("greet", builtin("npc-fallback", {}, function()
    builtin_calls = builtin_calls + 1
    return true
  end))

  local receipt = dialogue.emit("greet", 1, 2)
  eq(receipt.by, "npc-fallback", "unmounted 也走回退链（不静默吞）")
  eq(builtin_calls, 1, "回退命中一次")

  -- 无 builtin 可退：failed 的 reason 是 unmounted（可断言，不是「没响应」）。
  dialogue.off("greet", "npc-fallback")
  dialogue.emit("greet", 1, 2)
  eq(events_named(world, "dialogue.failed")[1].payload, "greet|unmounted", "reason = unmounted")
end)

run("pkg.dialogue.depth-capped", function()
  local world = new_world()
  local dialogue = fresh(world, { host = world.host, config = { max_depth = 3 } })
  local calls = 0
  dialogue.on("loop", builtin("recur", { memory = "none" }, function(frame)
    calls = calls + 1
    -- 同步嵌套：handler 内再 emit 才是深度 +1（ygg.emit 入队不算）。
    dialogue.emit("loop", frame.source, frame.target)
    return true
  end))

  local receipt = dialogue.emit("loop", 1, 2)
  eq(calls, 3, "总调用数 == max_depth")
  eq(#events_named(world, "dialogue.depth_capped"), 1, "depth_capped 恰好一次")
  eq(events_named(world, "dialogue.depth_capped")[1].payload, "loop|4", "载荷 = <type>|<超限深度>")
  eq(receipt.handled, true, "最外层仍算已处理")
  eq(receipt.depth, 1, "最外层深度 = 1")

  -- 链结束后深度复位：下一次 emit 仍从 1 起算（异常/截断都不得永久压帽）。
  local after = dialogue.emit("loop", 1, 2)
  eq(after.depth, 1, "截断后深度复位")
  eq(#events_named(world, "dialogue.depth_capped"), 2, "第二次链同样在 max_depth+1 截断")
end)

run("pkg.dialogue.thread-window", function()
  local world = new_world()
  local dialogue = fresh(world)
  dialogue.on("greet", builtin("npc", {}, function()
    return true
  end))

  for i = 1, 20 do
    dialogue.emit("greet", 5, 6, { text_key = "beats.n" .. i })
  end
  local turns = dialogue.history("5:6")
  eq(#turns, 12, "窗口缺省 12")
  eq(turns[1].text, "beats.n9", "保留最新 12 轮（丢最旧）")
  eq(turns[12].text, "beats.n20", "最后一轮是第 20 次")
  for i = 2, #turns do
    is_true(turns[i].seq > turns[i - 1].seq, "n 严格递增")
  end
  eq(turns[12].seq, 20, "轮次号 = 全局 seq（不因截断回退）")
  eq(world.kv["dlg.seq"], 20, "dlg.seq 是唯一写口")
  eq(world.kv["dlg.thread.5:6.turns"], 12, "轮数落 KV")

  -- `opts.window` 覆盖缺省：3 轮窗口时只留 3 条。
  local other = fresh(world)
  other.on("greet", builtin("tight", { window = 3 }, function()
    return true
  end))
  for i = 1, 5 do
    other.emit("greet", 1, 2, { text_key = "x" .. i })
  end
  eq(#other.history("1:2"), 3, "opts.window 覆盖缺省")
end)

end
