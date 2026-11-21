-- purity-export：用例段自 pkg-quest.lua 原样搬移（S-25 批十三测试件连拆）。
return function(C)
  local S, check, eq, is_true, is_nil, expect_error, run, new_world, fresh, sorted_keys, events_named, event_names, names_equal, errand_spec, pkg_sources, count_in_sources =
    C.S, C.check, C.eq, C.is_true, C.is_nil, C.expect_error, C.run, C.new_world, C.fresh, C.sorted_keys, C.events_named, C.event_names, C.names_equal, C.errand_spec, C.pkg_sources, C.count_in_sources
run("pkg.quest.no-clock", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("errand", errand_spec())

  -- I7：静态面——包源文件里一次 `ticks(` 都没有（命中数会返回反例）。
  local hits, where = count_in_sources("quest", "ticks(")
  eq(hits, 0, "包源码里的 ticks( 命中数： " .. table.concat(where, ","))

  -- 行为面——把 mock 时钟推到 12345，结果与 0 时逐字段相同，且一次都没读它。
  world.tick = 12345
  local id = quest.open("errand")
  eq(quest.advance(id), "open", "推进结果与时钟无关")
  eq(quest.get(id).step, 2, "步下标与时钟无关")
  eq(world.ticks_calls(), 0, "ygg.ticks 调用次数必须为 0")
end)

run("pkg.quest.pure-export", function()
  local world = new_world()

  -- I8：require + install 不写任何 KV（一个字节都不写，连名表都不建）。
  local quest = fresh(world)
  eq(world.writes_now(), 0, "require + install 的 KV 写次数")
  eq(#sorted_keys(world.kv), 0, "require + install 后的 KV 键集")

  -- 首次真正的写发生在 define（不是 install）。
  quest.define("errand", errand_spec())
  is_true(world.writes_now() > 0, "define 才写 KV")
  eq(world.kv["quest.seq"], nil, "define 不碰 seq")
end)

-- ── 用例 19-20：KV 形状与导出点名面 ──────────────────────────────────────────

run("pkg.quest.kv-shape", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("errand", errand_spec())
  local id = quest.open("errand", { meta = { weight = 2, npc = 5 } })
  quest.advance(id)

  -- 键集**逐项**落在 §2 的物理落点表里：多一枚键就是一份没人读的状态。
  local keys = sorted_keys(world.kv)
  local allowed = {
    "^quest%.seq$",
    "^quest%.types$",
    "^quest%.index%.%d+$",
    "^quest%.t%.[a-z][a-z0-9_-]*%.steps$",
    "^quest%.t%.[a-z][a-z0-9_-]*%.s%.%d+%.id$",
    "^quest%.t%.[a-z][a-z0-9_-]*%.s%.%d+%.guide$",
    "^quest%.t%.[a-z][a-z0-9_-]*%.s%.%d+%.on_enter$",
    "^quest%.t%.[a-z][a-z0-9_-]*%.m%.names$",
    "^quest%.t%.[a-z][a-z0-9_-]*%.m%.[A-Za-z_][A-Za-z0-9_.:-]*$",
    "^quest%.inst%.%d+%.type$",
    "^quest%.inst%.%d+%.step$",
    "^quest%.inst%.%d+%.state$",
    "^quest%.inst%.%d+%.giver$",
    "^quest%.inst%.%d+%.taker$",
    "^quest%.inst%.%d+%.m%.names$",
    "^quest%.inst%.%d+%.m%.[A-Za-z_][A-Za-z0-9_.:-]*$",
  }
  for i = 1, #keys do
    local ok = false
    for j = 1, #allowed do
      if keys[i]:find(allowed[j]) then
        ok = true
      end
    end
    is_true(ok, "越界键：" .. keys[i])
  end

  -- 前缀单写者：本包写的每一个键都属于 `quest.`，且实例的五个标量面**齐全**。
  for i = 1, #keys do
    eq(keys[i]:sub(1, 6), "quest.", "键前缀：" .. keys[i])
  end
  local inst = {}
  for i = 1, #keys do
    if keys[i]:find("^quest%.inst%." .. id .. "%.") then
      inst[#inst + 1] = keys[i]
    end
  end
  names_equal(inst, {
    "quest.inst." .. id .. ".giver",
    "quest.inst." .. id .. ".m.names",
    "quest.inst." .. id .. ".m.npc",
    "quest.inst." .. id .. ".m.weight",
    "quest.inst." .. id .. ".state",
    "quest.inst." .. id .. ".step",
    "quest.inst." .. id .. ".taker",
    "quest.inst." .. id .. ".type",
  }, "实例键集恰好")

  -- 值一律是标量（mock 的 set 已拒表；这里再断一次「读回也是标量」）。
  for i = 1, #keys do
    local kind = type(world.kv[keys[i]])
    is_true(
      kind == "number" or kind == "string" or kind == "boolean",
      "非标量值：" .. keys[i] .. " = " .. kind
    )
  end
end)

run("pkg.quest.keys-export", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("errand", errand_spec())
  local a = quest.open("errand", { meta = { urgent = true } })
  local b = quest.open("errand")
  quest.advance(b)
  quest.abandon(a, "gone")

  -- keys() = 本包在 KV 里**实际存在**的键，升序，逐项相等（不是「包含」）。
  local actual = {}
  for key in pairs(world.kv) do
    actual[#actual + 1] = key
  end
  table.sort(actual)
  names_equal(quest.keys(), actual, "keys() 与实际键集")
  eq(#quest.keys(), #actual, "keys() 数量")
  -- 升序（同一份输入两次调用字节相同）。
  names_equal(quest.keys(), quest.keys(), "keys() 稳定")

  -- codec 点名面（D5：`ns.keys` 已是 r1 落地的等价枚举面，故本包只消费、不改 codec）。
  package.loaded["pkg.codec"] = nil
  package.loaded["pkg.entity"] = nil
  local codec = require("pkg.codec")
  codec.install({ host = world.host })
  local bundle = codec.export({ entities = false, keys = quest.keys(), mod = "pkg-selftest" })
  local exported = {}
  for key in pairs(bundle.kv) do
    exported[#exported + 1] = key
  end
  table.sort(exported)
  names_equal(exported, actual, "bundle.kv 的键集 = keys()")
  for i = 1, #exported do
    eq(bundle.kv[exported[i]], world.kv[exported[i]], "bundle.kv 的值：" .. exported[i])
  end
  package.loaded["pkg.codec"] = nil
  package.loaded["pkg.entity"] = nil
end)

end
