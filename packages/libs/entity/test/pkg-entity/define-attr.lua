-- define-attr：用例段自 pkg-entity.lua 原样搬移（S-25 批十三测试件连拆）。
return function(C)
  local S, check, eq, is_true, is_nil, expect_error, run, new_world, fresh, sorted_keys, events_named, basic_spec =
    C.S, C.check, C.eq, C.is_true, C.is_nil, C.expect_error, C.run, C.new_world, C.fresh, C.sorted_keys, C.events_named, C.basic_spec
run("pkg.entity.define-idempotent", function()
  local world = new_world()
  local entity = fresh(world)
  local spec = {
    attrs = {
      hp = { type = "number", required = true, default = 100 },
      title = { type = "string" },
    },
    tags = { enemy = {} },
  }

  is_true(entity.define("creature", spec), "首次 define 必须返回 true")
  eq(world.kv["pkg.entity.schemas_rev"], 1, "首次 define 后 schemas_rev")
  local writes_after_first = world.kv_writes_now()

  is_true(entity.define("creature", spec), "同 spec 二次 define 必须返回 true")
  eq(world.kv["pkg.entity.schemas_rev"], 1, "幂等 define 不得推进 schemas_rev")
  eq(world.kv_writes_now(), writes_after_first, "幂等 define 必须是零写")

  -- 异 spec 不覆盖：不新增属性、不改类型、不改缺省值。
  expect_error("YGG-PKG-ENTITY-SCHEMA", function()
    entity.define("creature", { attrs = { hp = { type = "string" } } })
  end, "异 spec 覆盖被拒")
  eq(world.kv["pkg.entity.schemas_rev"], 1, "被拒的 define 不得推进 schemas_rev")
  eq(world.kv["pkg.entity.schema.creature.a.hp.type"], "number", "被拒的 define 不得改类型")

  -- 缺省值的形式差异也算「不同 spec」（`100` vs `100.0` 落盘即两样字节）。
  expect_error("YGG-PKG-ENTITY-SCHEMA", function()
    entity.define("creature", {
      attrs = {
        hp = { type = "number", required = true, default = 100.0 },
        title = { type = "string" },
      },
      tags = { enemy = {} },
    })
  end, "缺省值 math.type 不同也算异 spec")
end)

run("pkg.entity.define-kv-layout", function()
  local world = new_world()
  local entity = fresh(world)
  entity.define("creature", {
    attrs = {
      hp = { type = "number", required = true, default = 100 },
      title = { type = "string" },
    },
    tags = { enemy = {} },
  })

  -- §1.1 的物理落点表**逐键**：多一个键就是多一份没人读的状态，少一个键就是
  -- 某个字段只能靠猜。`hp` 带缺省值、`title` 不带，于是两者的键数恰好不同。
  local expected = {
    "pkg.entity.schema.creature.a.hp.default",
    "pkg.entity.schema.creature.a.hp.has_default",
    "pkg.entity.schema.creature.a.hp.required",
    "pkg.entity.schema.creature.a.hp.type",
    "pkg.entity.schema.creature.a.title.has_default",
    "pkg.entity.schema.creature.a.title.required",
    "pkg.entity.schema.creature.a.title.type",
    "pkg.entity.schema.creature.attrs",
    "pkg.entity.schema.creature.t.enemy.required",
    "pkg.entity.schema.creature.tags",
    "pkg.entity.schemas",
    "pkg.entity.schemas_rev",
  }
  table.sort(expected)

  local actual = sorted_keys(world.kv)
  eq(#actual, #expected, "KV 键数")
  for i = 1, #expected do
    eq(actual[i], expected[i], "KV 第 " .. i .. " 个键")
  end

  -- 名表是名字升序、换行连接（KV 存不了数组）。
  eq(world.kv["pkg.entity.schema.creature.attrs"], "hp\ntitle", "属性名表")
  eq(world.kv["pkg.entity.schema.creature.tags"], "enemy", "tag 名表")
  eq(world.kv["pkg.entity.schemas"], "creature", "schema 名表")
  -- 存在性位：没有缺省值的属性也必须落 `has_default=false` 的键在场。
  eq(world.kv["pkg.entity.schema.creature.a.title.has_default"], false, "title.has_default 必须在场")
  eq(world.kv["pkg.entity.schema.creature.a.hp.has_default"], true, "hp.has_default")
  eq(world.kv["pkg.entity.schema.creature.a.hp.default"], 100, "hp.default")

  -- 读回：`schema(name)` 的形状与写入一致（深副本，改坏不影响注册表）。
  local spec = entity.schema("creature")
  eq(spec.name, "creature", "schema 名")
  eq(#spec.attrs, 2, "读回属性数")
  eq(spec.attrs[1].name, "hp", "属性序（升序）")
  eq(spec.attrs[2].name, "title", "属性序（升序）")
  is_nil(entity.schema("不存在"), "未注册 schema 读 nil")

  spec.attrs[1].type = "string"
  eq(entity.schema("creature").attrs[1].type, "number", "schema() 必须返回深副本")
end)

-- ── 用例 4-9：构造 / 读 / 写 ────────────────────────────────────────────────

--- 四类属性 + 两个 tag 的标准 fixture：`hp` 必填带缺省、`title` 可选无缺省、
--- `alive_flag` 布尔带缺省、`target` 引用。

run("pkg.entity.attr-domain", function()
  local world = new_world()
  local entity = fresh(world)
  entity.define("creature", basic_spec())
  local id = entity.spawn("creature")

  -- 表与函数不是属性值：一旦允许嵌套表，「按属性查找」与「序列化形状」立刻分家。
  expect_error("YGG-PKG-ENTITY-ATTR", function()
    entity.set(id, "hp", {})
  end, "表值被拒")
  expect_error("YGG-PKG-ENTITY-ATTR", function()
    entity.set(id, "hp", function()
      return 1
    end)
  end, "函数值被拒")
  expect_error("YGG-PKG-ENTITY-ATTR", function()
    entity.set(id, "title", print)
  end, "非 string 值被拒")

  -- 被拒的写不得改状态（不是「报错后再写一半」）。
  eq(entity.get(id, "hp"), 100, "被拒的写不得改值")
  is_nil(entity.get(id, "title"), "title 从未写过")

  -- 四类合法值直通。
  is_true(entity.set(id, "hp", 250), "number 写入")
  is_true(entity.set(id, "title", "流浪者"), "string 写入")
  is_true(entity.set(id, "alive_flag", false), "boolean 写入")
  is_true(entity.set(id, "target", 7), "ref 写入")
  eq(entity.get(id, "hp"), 250, "number 读回")
  eq(entity.get(id, "title"), "流浪者", "string 读回")
  eq(entity.get(id, "alive_flag"), false, "boolean 读回（false 不是「没有值」）")
  eq(entity.get(id, "target"), 7, "ref 读回")
end)

run("pkg.entity.attr-undeclared-key", function()
  local world = new_world()
  local entity = fresh(world)
  entity.define("creature", basic_spec())
  local id = entity.spawn("creature")

  -- 未声明键写入是**错误**，不是静默 `false`：静默丢写会让「为什么这个属性永远
  -- 是初始值」变成一场考古（契约 I3）。
  expect_error("YGG-PKG-ENTITY-ATTR", function()
    entity.set(id, "mana", 10)
  end, "未声明键 set")
  expect_error("YGG-PKG-ENTITY-ATTR", function()
    entity.spawn("creature", { mana = 10 })
  end, "未声明键 spawn")
  is_nil(entity.get(id, "mana"), "未声明键读不到值")
end)

run("pkg.entity.attr-ref-domain", function()
  local world = new_world()
  local entity = fresh(world)
  entity.define("creature", basic_spec())
  local id = entity.spawn("creature")

  -- `ref` 是正整数 id：`0` / 负数 / 非整数 / 字符串数字全拒——「字符串 "3" 看着
  -- 像 3」正是跨档导入时炸得最晚的那一类。
  for _, bad in ipairs({ 0, -1, 2.5, "3" }) do
    expect_error("YGG-PKG-ENTITY-ATTR", function()
      entity.set(id, "target", bad)
    end, "ref 拒 " .. tostring(bad))
  end
  is_true(entity.set(id, "target", 3), "ref 收正整数")
  eq(entity.get(id, "target"), 3, "ref 读回")
  eq(math.type(entity.get(id, "target")), "integer", "ref 读回必须是整数")
end)

run("pkg.entity.attr-required-missing", function()
  local world = new_world()
  local entity = fresh(world)
  -- 必填且**无缺省**：缺省值能补上的必填不算缺失。
  entity.define("strict", { attrs = { hp = { type = "number", required = true } } })
  local writes_before = world.kv_writes_now()

  expect_error("YGG-PKG-ENTITY-SCHEMA", function()
    entity.spawn("strict")
  end, "必填缺失")

  -- 先校验后写：失败调用是零写且不分配 id（用例名里的「先于任何写」在这里。
  eq(world.kv_writes_now(), writes_before, "校验失败不得写 KV")
  eq(world.store.next_id, 1, "校验失败不得分配 id")
  eq(#events_named(world, "entity.created"), 0, "校验失败不得发事件")

  -- 缺省值能补上时，同一个 schema 就该成。
  entity.define("lenient", { attrs = { hp = { type = "number", required = true, default = 5 } } })
  local id = entity.spawn("lenient")
  eq(entity.get(id, "hp"), 5, "缺省值补齐必填")
end)

run("pkg.entity.attr-changed-once", function()
  local world = new_world()
  local entity = fresh(world)
  entity.define("creature", basic_spec())
  local id = entity.spawn("creature")

  is_true(entity.set(id, "hp", 250), "首次改值返回 true")
  eq(#events_named(world, "entity.attr_changed"), 1, "首次改值发一条事件")

  -- 同值写：返回 false、**不发事件**（契约 I6 的「仅在真的变化时」）。
  eq(entity.set(id, "hp", 250), false, "同值写返回 false")
  eq(#events_named(world, "entity.attr_changed"), 1, "同值写不得发事件")

  is_true(entity.set(id, "hp", 300), "异值写返回 true")
  eq(#events_named(world, "entity.attr_changed"), 2, "异值写再发一条")

  -- 事件载荷是标量字符串（`ygg.emit` 只收四类标量）：`<id>|<key>|<wire>`。
  local ev = events_named(world, "entity.attr_changed")
  eq(ev[2].payload, id .. "|hp|300", "attr_changed 载荷定形")
end)

end
