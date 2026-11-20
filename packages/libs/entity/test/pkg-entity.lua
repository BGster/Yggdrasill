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
  }

  world.host = {
    kv = {
      get = function(key)
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
