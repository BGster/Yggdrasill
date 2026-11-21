-- bridge-export：用例段自 pkg-dialogue.lua 原样搬移（S-25 批十三测试件连拆）。
return function(C)
  local S, check, eq, is_true, is_nil, expect_error, run, MODULES, new_world, fresh, sorted_keys, events_named, event_names, names_equal, builtin, agent, pkg_sources, count_in_sources =
    C.S, C.check, C.eq, C.is_true, C.is_nil, C.expect_error, C.run, C.MODULES, C.new_world, C.fresh, C.sorted_keys, C.events_named, C.event_names, C.names_equal, C.builtin, C.agent, C.pkg_sources, C.count_in_sources
run("pkg.dialogue.frame-shape", function()
  local world = new_world()
  local dialogue = fresh(world)

  local frame = dialogue.frame("greet", 1, 2, { text_key = "beats.greet" })
  names_equal(sorted_keys(frame), { "payload", "source", "target", "type", "v" }, "帧键集")
  eq(frame.v, 1, "版本")
  eq(frame.type, "greet", "type")
  eq(frame.source, 1, "source 原样（ref，不转字符串）")
  eq(frame.target, 2, "target 原样")
  eq(frame.payload.text_key, "beats.greet", "payload 原样")

  -- 纯函数：不写 KV、不推 seq、不发事件。
  eq(world.writes_now(), 0, "frame 不写 KV")
  eq(#world.events, 0, "frame 不发事件")
  is_nil(world.kv["dlg.seq"], "frame 不推 dlg.seq")

  -- handler 侧拿到的帧与 frame() 同形（同一构造函数）。
  local seen = nil
  dialogue.on("greet", builtin("npc", { memory = "none" }, function(f)
    seen = f
    return true
  end))
  dialogue.emit("greet", 1, 2)
  names_equal(sorted_keys(seen), { "payload", "source", "target", "type", "v" }, "handler 收到的帧键集")
end)

run("pkg.dialogue.adapters-registry", function()
  local world = new_world()
  local dialogue = fresh(world)
  local send_calls = 0
  local function adapter()
    return {
      send = function()
        send_calls = send_calls + 1
        return nil
      end,
      health = function()
        return true
      end,
    }
  end

  is_true(dialogue.register_adapter("term", adapter()), "注册 term")
  is_true(dialogue.register_adapter("echo", adapter()), "注册 echo")
  names_equal(dialogue.adapters(), { "echo", "term" }, "adapters() 升序副本")

  -- 副本：调用方改返回值不影响注册表。
  local copy = dialogue.adapters()
  copy[1] = "tampered"
  names_equal(dialogue.adapters(), { "echo", "term" }, "adapters() 是副本")

  expect_error("YGG-PKG-DIALOGUE-DUP", function()
    dialogue.register_adapter("term", adapter())
  end, "重名注册")
  expect_error("YGG-PKG-DIALOGUE-TYPE", function()
    dialogue.register_adapter("bad", { health = function() end })
  end, "缺 send")
  expect_error("YGG-PKG-DIALOGUE-TYPE", function()
    dialogue.register_adapter("bad2", "nope")
  end, "adapter 不是表")

  -- 包不驱动 adapter：一次 emit 之后 send 调用计数仍是 0（装配归 mod/宿主）。
  dialogue.on("greet", builtin("npc", { memory = "none" }, function()
    return true
  end))
  dialogue.emit("greet", 1, 2)
  eq(send_calls, 0, "adapter.send 调用次数必须为 0")
end)

run("pkg.dialogue.text-key-passthrough", function()
  local world = new_world()
  local dialogue = fresh(world)
  local key = "beats.dlg.greet.问候"
  dialogue.on("greet", builtin("npc", {}, function()
    return true
  end))

  dialogue.emit("greet", 1, 2, { text_key = key })
  local turns = dialogue.history("1:2")
  eq(#turns, 1, "记了一轮")
  eq(turns[1].text, key, "文本键原样进 turns（字节相同）")
  eq(#turns[1].text, #key, "文本键长度不变")
  eq(world.kv["dlg.thread.1:2.t.1.text"], key, "KV 里的 text 也是同一个键")

  -- 静态面：包内零 `ygg.data(`（文案的唯一副本在 mod 的 data/texts）。
  local hits, where = count_in_sources("dialogue", "ygg.data(")
  eq(hits, 0, "包源码里的 ygg.data( 命中数： " .. table.concat(where, ","))
  eq(world.data_calls(), 0, "运行时也没调 ygg.data")
end)

-- ── 用例 17-22：quest 桥、纯导出、重载、KV 形状、导出点名面 ────────────────────

run("pkg.dialogue.quest-bridge", function()
  local world = new_world()
  package.loaded["pkg.quest"] = nil
  local quest = require("pkg.quest")
  package.loaded["pkg.quest"] = nil

  local dialogue = fresh(world, { host = world.host, quest = quest })

  -- 桥只读 `quest.EVENTS`（五个名字），一个 `ygg.on` 都不多不少。
  eq(#world.registrations, 5, "ygg.on 注册数")
  local registered = {}
  for i = 1, #world.registrations do
    registered[i] = world.registrations[i].name
  end
  table.sort(registered)
  names_equal(registered, {
    "quest.abandoned",
    "quest.advanced",
    "quest.completed",
    "quest.jumped",
    "quest.opened",
  }, "注册的事件名（= quest.EVENTS）")

  -- 触发桥：host 派发 `quest.advanced` → 对应对白 handler 被调，链深从 1 起算。
  local seen = nil
  dialogue.on("quest.advanced", builtin("npc", { memory = "none" }, function(frame)
    seen = frame
    return true
  end))
  local receipt = nil
  for i = 1, #world.registrations do
    if world.registrations[i].name == "quest.advanced" then
      receipt = world.registrations[i].fn("1|2")
      break
    end
  end
  is_true(receipt ~= nil, "桥返回回执")
  eq(receipt.depth, 1, "跨事件不共享深度：桥发起的是新链")
  eq(receipt.by, "npc", "对白 handler 认领")
  eq(seen.type, "quest.advanced", "帧 type = 事件名")
  eq(seen.source, 0, "source = 0（系统）")
  eq(seen.target, 0, "target = 0（系统）")
  eq(seen.payload.text_key, "quest.advanced", "缺省文本键 = 事件名（仍是键，不造文案）")
  eq(seen.payload.quest_payload, "1|2", "quest 载荷原样透传")
end)

run("pkg.dialogue.bridge-absent", function()
  local world = new_world()
  local dialogue = fresh(world)
  eq(#world.registrations, 0, "不传 deps.quest 时不装桥（不猜名字）")

  -- 形状不合法的 deps.quest 是硬错，不是「静默不装」。
  expect_error("YGG-PKG-DIALOGUE-INSTALL", function()
    dialogue.install({ host = world.host, quest = { EVENTS = "quest.opened" } })
  end, "EVENTS 不是表")
  eq(#world.registrations, 0, "被拒的 install 不注册")
end)

run("pkg.dialogue.pure-export", function()
  local world = new_world()

  -- I8：require + install 不写任何 KV（连空名表都不写）。
  fresh(world)
  eq(world.writes_now(), 0, "require + install 的 KV 写次数")
  eq(#sorted_keys(world.kv), 0, "require + install 后的 KV 键集")

  -- 只有 `on` 才写 handlers（install 不预写）。
  local dialogue = fresh(world)
  eq(#sorted_keys(world.kv), 0, "二次 install 仍是零写")
  dialogue.on("greet", builtin("npc", { memory = "none" }, function()
    return true
  end))
  is_true(world.writes_now() > 0, "on 才写 KV")
end)

run("pkg.dialogue.reload-unbound", function()
  local world = new_world()
  local dialogue = fresh(world)
  local calls = 0
  dialogue.on("greet", builtin("npc", { memory = "none" }, function()
    calls = calls + 1
    return true
  end))
  eq(dialogue.emit("greet", 1, 2).handled, true, "重载前正常")

  -- 模拟进程重载：清模块缓存重新 require（函数丢，KV 不动）。
  for i = 1, #MODULES do
    package.loaded[MODULES[i]] = nil
  end
  local reloaded = require("pkg.dialogue")
  reloaded.install({ host = world.host })

  local receipt = reloaded.emit("greet", 1, 2)
  eq(receipt.handled, false, "重载后有 spec 无函数 = 未绑定")
  eq(calls, 1, "旧函数不再被调（REGISTRY 已丢）")
  eq(#events_named(world, "dialogue.failed"), 1, "dialogue.failed 恰好一次")
  eq(events_named(world, "dialogue.failed")[1].payload, "greet|unbound", "reason = unbound")
end)

run("pkg.dialogue.kv-shape", function()
  local world = new_world()
  local dialogue = fresh(world)
  world.tool_replies["npc.agent"] = { ok = true, unmounted = false, code = 0, data = {} }
  dialogue.on("greet", agent("npc-agent", { tool = "npc.agent", credential_key = "NPC_TOKEN" }))
  dialogue.on("greet", builtin("npc", { memory = "pair", window = 4 }, function()
    return true
  end))
  dialogue.emit("greet", 3, 4, { text_key = "beats.greet" })
  dialogue.say("9:10", "beats.push")

  local allowed = {
    "^dlg%.seq$",
    "^dlg%.types$",
    "^dlg%.keys$",
    "^dlg%.h%.[a-z][a-z0-9_-]*%.names$",
    "^dlg%.h%.[a-z][a-z0-9_-]*%.n%.[A-Za-z_][A-Za-z0-9_.:-]*%.kind$",
    "^dlg%.h%.[a-z][a-z0-9_-]*%.n%.[A-Za-z_][A-Za-z0-9_.:-]*%.opts$",
    "^dlg%.h%.[a-z][a-z0-9_-]*%.n%.[A-Za-z_][A-Za-z0-9_.:-]*%.opt%.[A-Za-z_][A-Za-z0-9_.:-]*$",
    "^dlg%.thread%.[^%%s]+%.turns$",
    "^dlg%.thread%.[^%%s]+%.t%.%d+%.who$",
    "^dlg%.thread%.[^%%s]+%.t%.%d+%.text$",
    "^dlg%.thread%.[^%%s]+%.t%.%d+%.seq$",
  }
  for key in pairs(world.kv) do
    local ok = false
    for i = 1, #allowed do
      if key:find(allowed[i]) then
        ok = true
      end
    end
    is_true(ok, "越界键：" .. key)
    eq(key:sub(1, 4), "dlg.", "键前缀：" .. key)
    local kind = type(world.kv[key])
    is_true(kind == "number" or kind == "string" or kind == "boolean", "非标量值：" .. key)
  end

  -- 名表口径：handler 名表保注册序（不排序），线程名表升序。
  eq(world.kv["dlg.h.greet.names"], "npc-agent\nnpc", "handler 名表 = 注册序")
  eq(world.kv["dlg.keys"], "3:4\n9:10", "线程名表升序")
  eq(world.kv["dlg.types"], "greet", "type 名表")
end)

run("pkg.dialogue.keys-export", function()
  local world = new_world()
  local dialogue = fresh(world)
  dialogue.on("greet", builtin("npc", { memory = "pair" }, function()
    return true
  end))
  dialogue.emit("greet", 1, 2, { text_key = "beats.greet" })
  dialogue.say("5:6", "beats.push")

  local actual = {}
  for key in pairs(world.kv) do
    actual[#actual + 1] = key
  end
  table.sort(actual)
  names_equal(dialogue.keys(), actual, "keys() 与实际键集")
  names_equal(dialogue.keys(), dialogue.keys(), "keys() 稳定")

  -- codec 点名面（D5：`ns.keys` 是 r1 落地的等价枚举面，本包只消费）。
  package.loaded["pkg.codec"] = nil
  package.loaded["pkg.entity"] = nil
  local codec = require("pkg.codec")
  codec.install({ host = world.host })
  local bundle = codec.export({ entities = false, keys = dialogue.keys(), mod = "pkg-selftest" })
  local exported = {}
  for key in pairs(bundle.kv) do
    exported[#exported + 1] = key
  end
  table.sort(exported)
  names_equal(exported, actual, "bundle.kv 的键集 = keys()")
  package.loaded["pkg.codec"] = nil
  package.loaded["pkg.entity"] = nil
end)

end
