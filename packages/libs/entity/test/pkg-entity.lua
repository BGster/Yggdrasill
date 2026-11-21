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
  local queries = {}
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
    -- S-24 查询面：**记录器**（记下收到的 selector、回放 `world.query_reply`）。
    -- 刻意不实现第二份引擎——两套语义必然漂，而本包只该证明「透传与后过滤」。
    query = function(selector)
      queries[#queries + 1] = selector
      if world.query_reply ~= nil then
        return world.query_reply
      end
      return { count = 0, ids = {} }
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
  world.queries = queries
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


-- === S-25 批十三测试件连拆装配：用例段搬入兄弟件（test/entity/<theme>.lua），执行次序 = 装配次序 ===
local C = {
  S = S,
  check = check,
  eq = eq,
  is_true = is_true,
  is_nil = is_nil,
  expect_error = expect_error,
  run = run,
  new_world = new_world,
  fresh = fresh,
  sorted_keys = sorted_keys,
  events_named = events_named,
  basic_spec = basic_spec
}

dofile(PKG_ROOT .. "/entity/test/pkg-entity/define-attr.lua")(C)
dofile(PKG_ROOT .. "/entity/test/pkg-entity/lifecycle-ids.lua")(C)
dofile(PKG_ROOT .. "/entity/test/pkg-entity/purity-query.lua")(C)
