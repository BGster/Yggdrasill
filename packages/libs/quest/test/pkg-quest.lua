-- `pkg.quest.*` 自测（20 条行为用例；perf 在 `pkg-quest-perf.lua`，同样计数）。
--
-- # 为什么自带 mock 而不复用 mod 的 harness
--
-- 包不依赖消费它的 mod（`packages/README.md` §5）。这里就地构造一个**语义等价**的
-- `ygg` 宿主面，四条都是内核的**真实行为**（照抄 `crates/ygg-sandbox/src/host_api.rs`）：
--
--   1. `kv.set` 只收四个标量型，表写入即 `unsupported payload type: table`；
--   2. `kv.get` 每次经 serde_json 重新编组 ⇒ 拿到的**不是**同一张表（标量无所谓，
--      但 mock 仍按「不共享引用」写，免得将来有人往里塞表时用例假绿）；
--   3. `ygg.emit` 的载荷同样只收标量（`value_from_lua` 同一份实现）；
--   4. `ygg.ticks()` 有 API 无推进者（恒 0）——本包**一次都不该读**，q17 用调用计数钉。
--
-- mock 若不允许这两条「只收标量」的拒绝，本包最核心的一条形状约束（扁平标量键）
-- 就永远测不到：把表塞进 KV 的实现在这里绿、在真实跑局红，正是 E6 那类假绿的形状。
--
-- # 隔离口径
--
-- 每条用例 `new_world()` 一次：新建 mock + 清 `package.loaded` 重新 require
-- （`install` 记的 host 归零）。用例之间不共享任何状态，失败不会串味。
--
-- 命名 ⇒ 计数可复算：文件名 `pkg-quest.lua`，`--filter pkg-quest` 只跑本包。

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
    error(("%s: expected %s, got %s"):format(label or "expect_error", code, tostring(err)), 2)
  end
end

local function run(name, fn)
  local ok, err = pcall(fn)
  check(name, ok, ok and nil or err)
end

-- ── mock 宿主面（语义对齐内核；见文件头注）───────────────────────────────────

--- 新建一个世界：mock `ygg` + 观测面（写/读计数、KV、事件流、外部面调用计数）。
local function new_world()
  local kv = {}
  local writes = 0
  local reads = 0
  local events = {}
  local entity_calls = 0
  local ticks_calls = 0
  local world = { kv = kv, events = events, tick = 0 }

  local function scalar_or_fail(kind, value)
    if type(value) == "table" or type(value) == "function" then
      error("unsupported payload type: " .. type(value), 0)
    end
    return kind, value
  end

  -- `ygg.entity.*` 一律计数：契约 I4 说本包「只把 giver/taker 当 ref 校验」，
  -- 一次调用都不该发生（q13 断 = 0）。这里不实现语义，只记「有没有被碰」。
  local entity = {}
  for _, name in ipairs({ "spawn", "destroy", "exists", "get_attribute", "set_attribute", "has_tag" }) do
    entity[name] = function()
      entity_calls = entity_calls + 1
      error("quest 不该调用 ygg.entity." .. name, 0)
    end
  end

  world.host = {
    kv = {
      get = function(key)
        reads = reads + 1
        return kv[key]
      end,
      set = function(key, value)
        scalar_or_fail("kv", value)
        writes = writes + 1
        kv[key] = value
      end,
    },
    emit = function(name, payload)
      if type(name) ~= "string" then
        error("emit 的事件名必须是字符串", 0)
      end
      scalar_or_fail("emit", payload)
      events[#events + 1] = { name = name, payload = payload }
    end,
    entity = entity,
    ticks = function()
      ticks_calls = ticks_calls + 1
      return world.tick
    end,
  }

  world.writes_now = function()
    return writes
  end
  world.reads_now = function()
    return reads
  end
  world.entity_calls = function()
    return entity_calls
  end
  world.ticks_calls = function()
    return ticks_calls
  end
  return world
end

--- 取一份**全新**的模块实例并装上 mock（清缓存 = `install` 记的 host 归零）。
local function fresh(world)
  for _, name in ipairs({
    "pkg.quest",
    "pkg.quest.store",
    "pkg.quest.types",
    "pkg.quest.flow",
  }) do
    package.loaded[name] = nil
  end
  local quest = require("pkg.quest")
  quest.install({ host = world.host })
  return quest
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

--- 事件名数组（保序）——「名字 + 次数」的精确断言用它，不用「包含」。
local function event_names(world)
  local out = {}
  for i = 1, #world.events do
    out[#out + 1] = world.events[i].name
  end
  return out
end

local function names_equal(actual, expected, label)
  eq(#actual, #expected, (label or "names_equal") .. " 数量")
  for i = 1, #expected do
    eq(actual[i], expected[i], (label or "names_equal") .. "[" .. i .. "]")
  end
end

--- 一个三步骤 + 两个 meta 键的标准流程（多数用例共用）。
local function errand_spec(steps)
  return {
    steps = steps or {
      { id = "accept", guide = "quests.errand.accept" },
      { id = "deliver", guide = "quests.errand.deliver", on_enter = "quest.check.deliver" },
      { id = "reward", guide = "quests.errand.reward" },
    },
    meta_spec = { source = "string", weight = "number", urgent = "boolean", npc = "ref" },
  }
end

-- ── 用例 1-5：定义与推进骨架 ────────────────────────────────────────────────

local function pkg_sources(id)
  local manifest = rawget(_G, "__MANIFEST")
  if type(manifest) ~= "table" or type(manifest.packages) ~= "table" then
    error("manifest 缺 packages 段（载体 pack.json 未声明包或未重算 manifest）", 0)
  end
  for i = 1, #manifest.packages do
    local pkg = manifest.packages[i]
    if pkg.id == id then
      local out = {}
      for j = 1, #pkg.files do
        out[#out + 1] = pkg.files[j].path
      end
      return out
    end
  end
  error("manifest 的 packages 段没有包 " .. id, 0)
end

local function count_in_sources(id, needle)
  local hits = 0
  local where = {}
  local paths = pkg_sources(id)
  for i = 1, #paths do
    local path = PKG_ROOT .. "/" .. paths[i]
    local handle = assert(io.open(path, "rb"), "读不到 " .. path)
    local text = handle:read("a")
    handle:close()
    local start = 1
    while true do
      local pos = text:find(needle, start, true)
      if pos == nil then
        break
      end
      hits = hits + 1
      where[#where + 1] = paths[i] .. "@" .. pos
      start = pos + #needle
    end
  end
  return hits, where
end


-- === S-25 批十三测试件连拆装配：用例段搬入兄弟件（test/quest/<theme>.lua），执行次序 = 装配次序 ===
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
  event_names = event_names,
  names_equal = names_equal,
  errand_spec = errand_spec,
  pkg_sources = pkg_sources,
  count_in_sources = count_in_sources
}

dofile(PKG_ROOT .. "/quest/test/pkg-quest/define-advance.lua")(C)
dofile(PKG_ROOT .. "/quest/test/pkg-quest/meta-events-ids.lua")(C)
dofile(PKG_ROOT .. "/quest/test/pkg-quest/purity-export.lua")(C)
