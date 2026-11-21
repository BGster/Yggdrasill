-- lifecycle-ids：用例段自 pkg-entity.lua 原样搬移（S-25 批十三测试件连拆）。
return function(C)
  local S, check, eq, is_true, is_nil, expect_error, run, new_world, fresh, sorted_keys, events_named, basic_spec =
    C.S, C.check, C.eq, C.is_true, C.is_nil, C.expect_error, C.run, C.new_world, C.fresh, C.sorted_keys, C.events_named, C.basic_spec
run("pkg.entity.created-once", function()
  local world = new_world()
  local entity = fresh(world)
  entity.define("creature", basic_spec())
  local id = entity.spawn("creature", { hp = 7 }, { "enemy" })

  local ev = events_named(world, "entity.created")
  eq(#ev, 1, "一次 spawn 恰好一条 entity.created")
  eq(ev[1].payload, id, "载荷就是 id")
  eq(math.type(ev[1].payload), "integer", "载荷是数字（不是字符串 id）")
  -- 同一次 spawn 不得捎带属性/标签事件（那是 set / tag 的语义）。
  eq(#events_named(world, "entity.attr_changed"), 0, "spawn 不得发 attr_changed")
  eq(#events_named(world, "entity.tagged"), 0, "spawn 不得发 tagged")

  -- 属性落盘：调用方给的值覆盖缺省值，没给的走缺省。
  eq(entity.get(id, "hp"), 7, "显式值覆盖缺省")
  eq(entity.get(id, "alive_flag"), true, "未给值走缺省")
  is_nil(entity.get(id, "title"), "无缺省的可选属性仍是 nil")
end)

-- ── 用例 3、10-17：存活集合 / 生命周期 / 悬垂引用 / 顶层纯度 ─────────────────

run("pkg.entity.spawn-monotonic-ids", function()
  local world = new_world()
  local entity = fresh(world)
  entity.define("creature", basic_spec())

  local a = entity.spawn("creature")
  local b = entity.spawn("creature")
  local c = entity.spawn("creature")
  is_true(b > a and c > b, "id 严格递增（契约 I1）")

  -- 分配者是内核（`EntityStore::next_id` 只增不减），本包只记水位镜像。
  is_true(entity.destroy(c), "destroy 返回 true")
  local d = entity.spawn("creature")
  is_true(d > c, "destroy 后不复用 id（契约 I1 的后半句）")
  eq(world.kv["pkg.entity.seq"], d, "seq 是已分配最大 id 的水位镜像")
end)

run("pkg.entity.destroyed-once", function()
  local world = new_world()
  local entity = fresh(world)
  entity.define("creature", basic_spec())
  local id = entity.spawn("creature", { hp = 3 }, { "enemy", "boss" })

  is_true(entity.destroy(id), "首次 destroy 返回 true")
  local ev = events_named(world, "entity.destroyed")
  eq(#ev, 1, "恰好一条 entity.destroyed")
  eq(ev[1].payload, id, "载荷就是 id")
  -- S9：存活位退役（键压根不写），内核侧才是真相——记录整条不在。
  eq(world.kv["pkg.entity.alive." .. id], nil, "不再写 pkg.entity.alive.<id>（S9 位退役）")
  eq(world.store.attrs[id], nil, "内核记录随真删消失")
  eq(world.store.tags[id], nil, "tag 集合随记录一起回收")
  eq(#entity.tags(id), 0, "destroy 之后读不到任何 tag")

  -- 二次 destroy：`false` 且零事件（事件恰好一次的另一个方向）。
  eq(entity.destroy(id), false, "二次 destroy 返回 false")
  eq(#events_named(world, "entity.destroyed"), 1, "二次 destroy 不得发事件")
end)

run("pkg.entity.exists-reads-kernel", function()
  local world = new_world()
  local entity = fresh(world)
  entity.define("creature", basic_spec())
  local id = entity.spawn("creature", { hp = 9 }, { "enemy" })

  -- I5 的硬要求：真源只有一个（内核查），**零 KV 读**、绝不扫键。用 mock 的两个
  -- 读计数钉死：exists 一次 = 一次存在性读 + 0 次 KV 读（S9 前是 1 次 KV 读）。
  local kv_before = world.kv_reads_now()
  local exists_before = world.exists_reads_now()
  is_true(entity.exists(id), "存活实体 exists = true")
  eq(world.kv_reads_now() - kv_before, 0, "exists 不得读 KV（S9：真源在内核）")
  eq(world.exists_reads_now() - exists_before, 1, "exists 必须问一次内核存在性")

  is_true(entity.destroy(id), "destroy")
  eq(entity.exists(id), false, "死亡后 exists = false")
  eq(entity.exists(999), false, "从未存在过的 id = false（不报错）")

  -- 真删对外不可见：四个读口子一致遮蔽（不再是同一条记录 + 遮蔽位）。
  is_nil(entity.get(id, "hp"), "get(dead) = nil")
  eq(next(entity.attrs(id)), nil, "attrs(dead) 是空表")
  is_nil(entity.schema_of(id), "schema_of(dead) = nil")
  eq(entity.has(id, "enemy"), false, "has(dead) = false")
end)

run("pkg.entity.destroy-reclaims-kernel-record", function()
  local world = new_world()
  local entity = fresh(world)
  entity.define("creature", basic_spec())

  -- 基线：先走一轮 spawn+destroy，把 count 这类「首次写才出现」的常量键建起来；
  -- 之后一轮的增量才是**残留**（否则会把建键算进 destroy 的账）。
  local function key_count()
    local n = 0
    for _ in pairs(world.kv) do
      n = n + 1
    end
    return n
  end
  local warm = entity.spawn("creature", { hp = 1 })
  is_true(entity.destroy(warm), "warmup destroy")
  local keys_before = key_count()

  local id = entity.spawn("creature", { hp = 9 }, { "enemy" })
  local keys_spawned = key_count()
  eq(keys_spawned - keys_before, 1, "spawn 只新增 rec.<id>.schema（count/seq 已存在）")

  is_true(entity.destroy(id), "destroy")
  eq(world.store.attrs[id], nil, "属性袋已被回收（r2-G4 的「永占内存」归零）")
  eq(world.store.tags[id], nil, "tag 集合已被回收")
  eq(entity.count(), 0, "count 归零")
  eq(#entity.ids(), 0, "#ids 归零")
  -- 残留恰好 1 枚 KV 键（rec.<id>.schema）：KV 无删除面，这是可数的剩余代价（I5）。
  eq(key_count() - keys_spawned, 0, "destroy 自己不新增任何键（alive 位已退役）")
  eq(key_count() - keys_before, 1, "一轮 spawn+destroy 的净残留恰好 1 枚（rec.schema）")
  eq(world.kv["pkg.entity.rec." .. id .. ".schema"], "creature", "残留的就是 rec 的 schema 键")
end)

run("pkg.entity.ids-count", function()
  local world = new_world()
  local entity = fresh(world)
  entity.define("creature", basic_spec())
  local a = entity.spawn("creature")
  local b = entity.spawn("creature")
  local c = entity.spawn("creature")

  local ids = entity.ids()
  eq(#ids, 3, "三条存活")
  eq(entity.count(), 3, "count 与 ids 同数")
  is_true(ids[1] < ids[2] and ids[2] < ids[3], "ids 升序")
  eq(ids[1], a, "首个 id")
  eq(ids[3], c, "末个 id")

  entity.destroy(b)
  local after = entity.ids()
  eq(#after, 2, "destroy 后 ids 减一")
  eq(entity.count(), 2, "destroy 后 count 减一")
  local listed = false
  for i = 1, #after do
    if after[i] == b then
      listed = true
    end
  end
  eq(listed, false, "死亡 id 不出现在 ids 里")
end)

run("pkg.entity.ids-tag-filter", function()
  local world = new_world()
  local entity = fresh(world)
  entity.define("creature", basic_spec())
  local a = entity.spawn("creature", nil, { "enemy" })
  local b = entity.spawn("creature", nil, { "boss" })
  local c = entity.spawn("creature", nil, { "enemy", "boss" })
  entity.spawn("creature")

  local enemies = entity.ids("enemy")
  eq(#enemies, 2, "enemy 两名")
  eq(enemies[1], a, "enemy 首个")
  eq(enemies[2], c, "enemy 末个")
  local bosses = entity.ids("boss")
  eq(#bosses, 2, "boss 两名")
  eq(bosses[1], b, "boss 首个")
  eq(bosses[2], c, "boss 末个")
  eq(#entity.ids("enemy", nil), 2, "单 tag 过滤")
end)

run("pkg.entity.ids-schema-filter", function()
  local world = new_world()
  local entity = fresh(world)
  entity.define("creature", basic_spec())
  entity.define("item", { attrs = { weight = { type = "number" } } })
  local a = entity.spawn("creature", nil, { "enemy" })
  local b = entity.spawn("item")
  local c = entity.spawn("creature")

  local creatures = entity.ids(nil, "creature")
  eq(#creatures, 2, "creature 两名")
  eq(creatures[1], a, "首个 creature")
  eq(creatures[2], c, "末个 creature")
  local items = entity.ids(nil, "item")
  eq(#items, 1, "item 一名")
  eq(items[1], b, "item id")
  eq(#entity.ids("enemy", "creature"), 1, "tag + schema 同时过滤")
  eq(entity.schema_of(b), "item", "schema_of 读回声明名")
end)

run("pkg.entity.undeclared-tag", function()
  local world = new_world()
  local entity = fresh(world)
  entity.define("creature", basic_spec())
  local id = entity.spawn("creature")

  -- D8 的收紧：未声明的 tag = 错误。理由不是洁癖——`destroy` 只能清「已声明的
  -- tag 名单」，未声明的 tag 一旦允许，就会在「同 id 不复用」下永久占内存，而
  -- 宿主没有任何面能枚举它、更谈不上擦掉。
  expect_error("YGG-PKG-ENTITY-SCHEMA", function()
    entity.tag(id, "没声明")
  end, "未声明 tag 写入被拒")
  expect_error("YGG-PKG-ENTITY-SCHEMA", function()
    entity.spawn("creature", nil, { "没声明" })
  end, "未声明 tag spawn 被拒")
  is_true(entity.tag(id, "boss"), "已声明 tag 写入")
  eq(entity.has(id, "boss"), true, "has 读回")
  eq(#events_named(world, "entity.tagged"), 1, "一次 tag 一条事件")
  eq(entity.untag(id, "boss"), true, "untag 返回变化")
  eq(entity.untag(id, "boss"), false, "重复 untag 返回 false")
  eq(#events_named(world, "entity.untagged"), 1, "只有真的变化才发事件")
end)

run("pkg.entity.dangling-ref-not-an-error", function()
  local world = new_world()
  local entity = fresh(world)
  entity.define("creature", basic_spec())
  local target = entity.spawn("creature")
  local holder = entity.spawn("creature", { target = target })

  is_true(entity.destroy(target), "destroy 引用目标")
  -- I4：不遍历、不清引用槽。清理归 mod 语义（「召唤物死亡，主人失去它」是规则）。
  eq(entity.get(holder, "target"), target, "引用值不变")
  eq(entity.exists(entity.get(holder, "target")), false, "引用指向的实体不存在")
  eq(entity.get(holder, "hp"), 100, "持有者自身不受影响")
  eq(#events_named(world, "entity.destroyed"), 1, "只发了一条 destroyed")
end)

end
