-- purity-query：用例段自 pkg-entity.lua 原样搬移（S-25 批十三测试件连拆）。
return function(C)
  local S, check, eq, is_true, is_nil, expect_error, run, new_world, fresh, sorted_keys, events_named, basic_spec =
    C.S, C.check, C.eq, C.is_true, C.is_nil, C.expect_error, C.run, C.new_world, C.fresh, C.sorted_keys, C.events_named, C.basic_spec
run("pkg.entity.top-level-pure", function()
  local saved = rawget(_G, "ygg")
  local touched = nil
  _G.ygg = setmetatable({}, {
    __index = function(_, key)
      touched = tostring(key)
      error("模块顶层访问了 ygg." .. touched, 0)
    end,
  })
  package.loaded["pkg.entity"] = nil
  local ok, err = pcall(require, "pkg.entity")
  _G.ygg = saved
  package.loaded["pkg.entity"] = nil
  is_true(ok, "require 触发了 ygg 访问：" .. tostring(err))
  is_nil(touched, "模块顶层访问了 ygg." .. tostring(touched))
end)

run("pkg.entity.no-time-keys", function()
  local world = new_world()
  local entity = fresh(world)
  entity.define("creature", basic_spec())

  -- 把**全部**写路径跑一遍：schema 注册、spawn、set、tag/untag、destroy。
  local a = entity.spawn("creature", { hp = 1 }, { "enemy" })
  entity.set(a, "hp", 2)
  entity.tag(a, "boss")
  entity.untag(a, "enemy")
  local b = entity.spawn("creature", { target = a })
  entity.destroy(a)
  entity.destroy(b)

  -- I8 / D6：实体不得带时间戳字段。沙箱里 `ygg.ticks()` 恒 0、事件 seq 不注入
  -- Lua，所以时间既拿不到也不该被发明；先后由宿主派发序保证。
  local offenders = {}
  for key in pairs(world.kv) do
    if key:find("%.ts$") or key:find("%.time$") or key:find("%d%d%d%d%-%d%d%-%d%d") then
      offenders[#offenders + 1] = key
    end
  end
  eq(#offenders, 0, "KV 键里不得有时间戳形状：" .. table.concat(offenders, ", "))

  -- 事件载荷同样不得携带时间（载荷是标量，时间只能以数字/字符串形式混进去）。
  for i = 1, #world.events do
    local payload = world.events[i].payload
    if type(payload) == "string" then
      eq(payload:find("%d%d%d%d%-%d%d%-%d%d") == nil, true, "事件载荷不得带日期")
    end
  end
end)

-- ── 导出面键集恰等（I6 的硬钉：一个不缺、一个不多）─────────────────────────────
--
-- 为什么这条要「恰等」而不是「至少含」：`pkg.entity` 的对外面是七包消费方的契约
-- （`codec` 逐函数调用它），少一个键是断裂、多一个键是**未申报的能力面**。下沉到
-- 原生面之后最容易发生的漂移就是「顺手加个批量函数上来」，这条断言把那种改动挡在
-- 门禁里，而不是等 pm 发现「同一个包两种批量风格」。
--
-- 键名全部 ASCII，故升序 = 字节序（`sorted_keys` 用 table.sort，本文件跑在普通 Lua）。
run("pkg.entity.exports", function()
  local entity = fresh(new_world())
  local EXPECTED = {
    "VERSION",
    "attrs",
    "count",
    "define",
    "destroy",
    "exists",
    "get",
    "has",
    "ids",
    "install",
    "query",
    "schema",
    "schema_of",
    "set",
    "spawn",
    "tag",
    "tags",
    "untag",
  }
  local seen = sorted_keys(entity)

  -- 双向差集分开报：红的时候要一眼看出是「多」还是「缺」，不要再去手数。
  local extra = {}
  local want = {}
  for i = 1, #EXPECTED do
    want[EXPECTED[i]] = true
  end
  for i = 1, #seen do
    if not want[seen[i]] then
      extra[#extra + 1] = seen[i]
    end
  end
  local missing = {}
  for i = 1, #EXPECTED do
    local found = false
    for j = 1, #seen do
      if seen[j] == EXPECTED[i] then
        found = true
      end
    end
    if not found then
      missing[#missing + 1] = EXPECTED[i]
    end
  end

  eq(#extra, 0, "导出面多出键（未申报的能力面）：" .. table.concat(extra, ", "))
  eq(#missing, 0, "导出面缺键（契约 §4 的 API 面）：" .. table.concat(missing, ", "))
  eq(#seen, #EXPECTED, "键数必须恰等")
  for i = 1, #EXPECTED do
    eq(seen[i], EXPECTED[i], "第 " .. i .. " 个键")
  end
end)

-- ── 用例 S-24：`entity.query`（内核查询面的包面 = 透传 + schema 后过滤）──────────
--
-- mock 只做**记录器**（记 selector、回放固定出表）：本文件要证的是「透传逐字」与
-- 「后过滤只按 KV 的 schema 记录」，不是第二份引擎的语义（那会与内核漂）。

run("pkg.entity.query-forwards-selector-verbatim", function()
  local world = new_world()
  local entity = fresh(world)
  local where = { hp = "gte.10", kind = { op = "in", value = { "orc", "rat" } } }
  local reply = { count = 2, ids = { 1, 2 } }
  world.query_reply = reply
  local got = entity.query({ tag = "enemy", where = where, order = "hp.desc", select = { "hp" } })

  eq(#world.queries, 1, "无 schema ⇒ 恰 1 次穿越")
  local sent = world.queries[1]
  eq(sent.tag, "enemy")
  eq(sent.where, where, "`where` 逐字透传（同一张表，不重建）")
  eq(sent.order, "hp.desc")
  eq(#sent.select, 1, "`select` 原样")
  eq(sent.select[1], "hp")
  is_nil(sent.schema, "`schema` 不是内核键")
  eq(got, reply, "出表原样回（包不重造形态）")
end)

run("pkg.entity.query-strips-schema-key", function()
  local world = new_world()
  local entity = fresh(world)
  entity.define("monster", { attrs = { hp = { type = "number" } } })
  local id = entity.spawn("monster", { hp = 10 })
  world.query_reply = { count = 1, ids = { id } }
  local got = entity.query({ schema = "monster", where = { hp = "gte.10" } })

  eq(#world.queries, 1)
  is_nil(world.queries[1].schema, "`schema` 必须在跨界前剥掉（内核不认这个键）")
  eq(world.queries[1].where.hp, "gte.10", "其余字段逐字保留")
  eq(got.count, 1)
  eq(got.ids[1], id)
end)

run("pkg.entity.query-schema-filters-by-kv-record", function()
  local world = new_world()
  local entity = fresh(world)
  entity.define("monster", { attrs = { hp = { type = "number" } } })
  entity.define("npc", { attrs = { hp = { type = "number" } } })
  local a = entity.spawn("monster", { hp = 10 })
  local b = entity.spawn("npc", { hp = 20 })
  local c = entity.spawn("monster", { hp = 30 })
  world.query_reply = { count = 3, ids = { a, b, c } }

  local got = entity.query({ schema = "monster" })
  eq(got.count, 2, "只剩 monster 的两条")
  eq(got.ids[1], a)
  eq(got.ids[2], c)
  eq(#world.queries, 1, "id 形态：1 次穿越 + R 次 KV 读")
end)

run("pkg.entity.query-select-filters-rows-in-place", function()
  local world = new_world()
  local entity = fresh(world)
  entity.define("monster", { attrs = { hp = { type = "number" } } })
  entity.define("npc", { attrs = { hp = { type = "number" } } })
  local a = entity.spawn("monster", { hp = 10 })
  local b = entity.spawn("npc", { hp = 20 })
  local c = entity.spawn("monster", { hp = 30 })
  world.query_reply = {
    count = 3,
    rows = { { id = a, hp = 10 }, { id = b, hp = 20 }, { id = c, hp = 30 } },
  }

  local got = entity.query({ schema = "monster", select = { "hp" } })
  eq(got.count, 2)
  eq(#got.rows, 2)
  eq(got.rows[1].id, a)
  eq(got.rows[2].hp, 30, "行表原地过滤，投影值不重取")
  eq(#world.queries, 1, "行形态不二次穿越（再走 attr_fetch 只是多一次材质化）")
  eq(world.queries[1].select[1], "hp")
end)

run("pkg.entity.query-undeclared-attr-rejected", function()
  local world = new_world()
  local entity = fresh(world)
  entity.define("monster", { attrs = { hp = { type = "number" } } })

  expect_error("YGG-PKG-ENTITY-ATTR", function()
    entity.query({ schema = "monster", where = { hp = "gte.1", speed = "gt.1" } })
  end, "where 的未声明属性")
  expect_error("YGG-PKG-ENTITY-ATTR", function()
    entity.query({ schema = "monster", select = { "hp", "speed" } })
  end, "select 的未声明属性")
  eq(#world.queries, 0, "门在查询之前：零穿越")

  -- `id` 是内核保留伪属性（不在 schema 里），必须放过——否则 `where = { id = ... }` 不可用
  world.query_reply = { count = 0, ids = {} }
  entity.query({ schema = "monster", where = { id = "gte.1" } })
  eq(#world.queries, 1)
end)

run("pkg.entity.query-unknown-schema-rejected", function()
  local world = new_world()
  local entity = fresh(world)
  expect_error("YGG-PKG-ENTITY-SCHEMA", function()
    entity.query({ schema = "ghost" })
  end, "未注册 schema")
  expect_error("YGG-PKG-ENTITY-SCHEMA", function()
    entity.query({ schema = 5 })
  end, "schema 非字符串")
  eq(#world.queries, 0, "零穿越")
end)

run("pkg.entity.query-empty-result-keeps-shape", function()
  local world = new_world()
  local entity = fresh(world)
  entity.define("monster", { attrs = { hp = { type = "number" } } })

  world.query_reply = { count = 0, ids = {} }
  local got = entity.query({ schema = "monster" })
  eq(got.count, 0, "count 恒在（空结果可判）")
  eq(type(got.ids), "table")
  eq(#got.ids, 0)
  is_nil(got.rows, "id 形态不带 rows")

  world.query_reply = { count = 0, rows = {} }
  local rows = entity.query({ schema = "monster", select = { "hp" } })
  eq(rows.count, 0)
  eq(type(rows.rows), "table")
  is_nil(rows.ids, "行形态不带 ids")
end)

run("pkg.entity.query-schema-operand-type", function()
  local world = new_world()
  local entity = fresh(world)
  entity.define("monster", {
    attrs = { hp = { type = "number" }, title = { type = "string" } },
  })

  -- 结构态的类型门：number 属性收到字符串操作数 ⇒ 错（内核面会变成「跨族即假」）
  expect_error("YGG-PKG-ENTITY-ATTR", function()
    entity.query({ schema = "monster", where = { hp = { op = "gte", value = "ten" } } })
  end, "number 属性收到字符串操作数")
  expect_error("YGG-PKG-ENTITY-ATTR", function()
    entity.query({ schema = "monster", where = { hp = { op = "in", value = { 1, "two" } } } })
  end, "in 的元素类型不相容")
  eq(#world.queries, 0, "门在查询之前")

  -- 相容的操作数放过；字符串态的类型由内核判（跨族即假），包不二次解析
  world.query_reply = { count = 0, ids = {} }
  entity.query({ schema = "monster", where = { hp = "gte.10", title = "eq.x" } })
  eq(#world.queries, 1)
end)

end
