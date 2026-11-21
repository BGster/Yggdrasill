-- `pkg.entity.*` 自测（18 条行为用例；perf 在 `pkg-entity-perf.lua`，file 级断言不计数）。
--
-- # 为什么自带 mock 而不复用 mod 的 harness
--
-- 包不依赖消费它的 mod（`packages/README.md` §5）。这里就地构造一个**语义等价**的
-- `ygg` 宿主面：KV 只收标量、`EntityStore` id 从 1 起致密单调、`set_attribute` 对
-- 不存在的 id 报错、`add_tag` / `remove_tag` 返回「是否实际改变」——四条都是内核
-- 的**真实行为**（`crates/ygg-core/src/entity.rs`、`crates/ygg-core/src/kv.rs`），
-- mock 若不照抄，用例绿而真实跑局红。
--
-- # 隔离口径
--
-- 每条用例 `fresh()` 一次：清 `package.loaded` 重新 require（模块级 memo / installed
-- 归零）+ 新建一份 mock。用例之间不共享任何状态，失败不会串味。
--
-- 命名 ⇒ 计数可复算：文件名 `pkg-entity.lua`，`--filter pkg-entity` 只跑本文件。

local S = rawget(_G, "__SELFTEST")
if type(S) ~= "table" then
  S = { pass = 0, fail = 0 }
  _G.__SELFTEST = S
end

local function check(name, ok, detail)
  if ok then
    S.pass = S.pass + 1
  else
    S.fail = S.fail + 1
    print("FAIL " .. name .. (detail and (": " .. tostring(detail)) or ""))
  end
end

local function eq(actual, expected, label)
  if actual ~= expected or math.type(actual) ~= math.type(expected) then
    error(
      ("%s: expected %s (%s), got %s (%s)"):format(
        label or "eq",
        tostring(expected),
        tostring(math.type(expected)),
        tostring(actual),
        tostring(math.type(actual))
      ),
      2
    )
  end
end

local function is_true(value, label)
  if not value then
    error((label or "is_true") .. ": expected truthy, got " .. tostring(value), 2)
  end
end

local function is_nil(value, label)
  if value ~= nil then
    error((label or "is_nil") .. ": expected nil, got " .. tostring(value), 2)
  end
end

local function expect_error(code, fn, label)
  local ok, err = pcall(fn)
  if ok then
    error((label or "expect_error") .. ": expected " .. code .. ", but the call succeeded", 2)
  end
  if not tostring(err):find(code, 1, true) then
    error(
      ("%s: expected %s, got %s"):format(label or "expect_error", code, tostring(err)),
      2
    )
  end
end

local function run(name, fn)
  local ok, err = pcall(fn)
  check(name, ok, ok and nil or err)
end

-- ── mock 宿主面（语义对齐内核；见文件头注）─────────────────────────────────────

--- 新建一个世界：mock `ygg` + 观测面（写计数、KV、事件流、实体表）。
local function new_world()
  local kv = {}
  local kv_writes = 0
  local kv_reads = 0
  local exists_reads = 0
  local store = { next_id = 1, attrs = {}, tags = {} }
  local events = {}
  local world = { kv = kv, store = store, events = events, kv_writes = 0 }

  local entity = {
    spawn = function()
      local id = store.next_id
      store.next_id = id + 1
      store.attrs[id] = {}
      store.tags[id] = {}
      return id
    end,
    get_attribute = function(id, name)
      local bag = store.attrs[id]
      if bag == nil then
        return nil
      end
      return bag[name]
    end,
    set_attribute = function(id, name, value)
      local bag = store.attrs[id]
      if bag == nil then
        error("entity " .. tostring(id) .. " not found", 0)
      end
      bag[name] = value
    end,
    add_tag = function(id, tag)
      local set = store.tags[id]
      if set == nil then
        error("entity " .. tostring(id) .. " not found", 0)
      end
      if set[tag] then
        return false
      end
      set[tag] = true
      return true
    end,
    remove_tag = function(id, tag)
      local set = store.tags[id]
      if set == nil then
        error("entity " .. tostring(id) .. " not found", 0)
      end
      if not set[tag] then
        return false
      end
      set[tag] = nil
      return true
    end,
    has_tag = function(id, tag)
      local set = store.tags[id]
      return set ~= nil and set[tag] == true
    end,
    -- S9 控制面两键的替身：存在性 = store 里还有这条记录。
    exists = function(id)
      exists_reads = exists_reads + 1
      return store.attrs[id] ~= nil
    end,
    destroy = function(id)
      if store.attrs[id] == nil then
        return false
      end
      store.attrs[id] = nil
      store.tags[id] = nil
      return true
    end,
  }

  world.host = {
    kv = {
      get = function(key)
        kv_reads = kv_reads + 1
        return kv[key]
      end,
      set = function(key, value)
        kv_writes = kv_writes + 1
        kv[key] = value
      end,
    },
    entity = entity,
    emit = function(name, payload)
      events[#events + 1] = { name = name, payload = payload }
    end,
  }

  world.kv_writes_now = function()
    return kv_writes
  end
  world.kv_reads_now = function()
    return kv_reads
  end
  world.exists_reads_now = function()
    return exists_reads
  end
  return world
end

--- 取一份**全新**的模块实例并装上 mock（清缓存 = memo / installed 归零）。
local function fresh(world)
  package.loaded["pkg.entity"] = nil
  local entity = require("pkg.entity")
  entity.install({ host = world.host })
  return entity
end

--- 升序键列表（测试里允许 `table.sort`：这里跑的是普通 Lua，不是沙箱）。
local function sorted_keys(map)
  local keys = {}
  for key in pairs(map) do
    keys[#keys + 1] = key
  end
  table.sort(keys)
  return keys
end

--- 事件流里按名字筛（保序）。
local function events_named(world, name)
  local out = {}
  for i = 1, #world.events do
    if world.events[i].name == name then
      out[#out + 1] = world.events[i]
    end
  end
  return out
end

-- ── 用例 1-2：schema 注册表 ──────────────────────────────────────────────────

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
local function basic_spec()
  return {
    attrs = {
      hp = { type = "number", required = true, default = 100 },
      title = { type = "string" },
      alive_flag = { type = "boolean", default = true },
      target = { type = "ref" },
    },
    tags = { enemy = {}, boss = {} },
  }
end

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
