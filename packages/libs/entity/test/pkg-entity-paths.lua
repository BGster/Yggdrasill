-- `pkg.entity` 逐路径基线（§9 下沉边界的 before 列）。**不计数**：本文件只用
-- file 级断言（`error` ⇒ `file FAIL` ⇒ `ok=0`），与 `pkg-entity-perf.lua` 同款纪律。
--
-- # 为什么单开一个文件
--
-- `pkg-entity-perf.lua` 的五条是契约 §5 的**复算锚**（名字与口径不许动）；这里量的是
-- 下沉边界逐函数表（§9.1）里另外七条：`spawn` / `destroy` / `get` / `set` 复合 /
-- `attrs` / `tags` / `ids` 全扫。前者回答「按 id 的 O(1) 没退化」，后者回答
-- 「每条路径的 before 是多少、下沉后 after 是多少」——两组数字不能互相顶替。
--
-- # 口径（与 §5 同）
--
-- 普通 Lua（selftest）+ mock 宿主：`kv` 是 Lua 表、`entity.*` 是 Lua 函数 ⇒ **不含
-- 沙箱跨边界成本**，所以这是每路径的**包侧下限**。真沙箱见 `packages/mods/pkg-entity-probe`。
-- 10⁴ 迭代 × 三批取中位 × 整文件跑三遍看稳定性；实体集 = 8 属性 + 3 tag（与 §5 同一套）。

local ATTRS = {
  hp = 100,
  max_hp = 100,
  attack = 12,
  defense = 7,
  speed = 5,
  level = 3,
  exp = 0,
  weight = 1,
}
local TAGS = { "enemy", "boss", "undead" }
local ITERS = 10000

local function must(condition, message)
  if not condition then
    error("[pkg-entity-paths] " .. message, 0)
  end
end

local function median3(a, b, c)
  if a > b then
    a, b = b, a
  end
  if b > c then
    b, c = c, b
  end
  if a > b then
    a, b = b, a
  end
  return b
end

-- ── mock 宿主（与 pkg-entity.lua / pkg-entity-perf.lua 各自一份；刻意不抽公共夹具）──

local function new_world()
  local kv = {}
  local store = { next_id = 1, attrs = {}, tags = {} }
  local host = {
    kv = {
      get = function(key)
        return kv[key]
      end,
      set = function(key, value)
        kv[key] = value
      end,
    },
    entity = {
      spawn = function()
        local id = store.next_id
        store.next_id = id + 1
        store.attrs[id] = {}
        store.tags[id] = {}
        return id
      end,
      get_attribute = function(id, name)
        local bag = store.attrs[id]
        return bag and bag[name] or nil
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
    },
    emit = function() end,
  }
  return { kv = kv, host = host }
end

local function fresh(world)
  package.loaded["pkg.entity"] = nil
  local entity = require("pkg.entity")
  entity.install({ host = world.host })
  return entity
end

local SPEC = { attrs = {}, tags = { enemy = {}, boss = {}, undead = {} } }
for name in pairs(ATTRS) do
  SPEC.attrs[name] = { type = "number" }
end

--- 一批 ITERS 次调用的每 op 纳秒（三批取中位）。
local function bench(fn)
  local samples = {}
  for batch = 1, 3 do
    local began = os.clock()
    for round = 1, ITERS do
      fn(round)
    end
    samples[batch] = (os.clock() - began) / ITERS * 1e9
  end
  local ns = median3(samples[1], samples[2], samples[3])
  must(ns > 0, "测得的每 op 耗时必须是正数（否则没量到东西）")
  return ns, samples
end

local function report(label, fn, unit)
  local ns, s = bench(fn)
  print(
    ("PERF entity.paths_%s=%s %s  (samples %.1f/%.1f/%.1f, iters=%d)"):format(
      label,
      unit == "ms" and ("%.3f"):format(ns / 1e6) or ("%.1f"):format(ns),
      unit or "ns/op",
      s[1],
      s[2],
      s[3],
      ITERS
    )
  )
end

-- ① spawn（一次 8 属性 + 3 tag；同一世界里累计到 3×10⁴，含 KV 长表）──────────────
--    刻意不每批新建世界：批量造世界时 KV / 实体表都在长，GC 与重哈希属于被测成本。
do
  local world = new_world()
  local entity = fresh(world)
  entity.define("bench", SPEC)
  report("spawn", function()
    entity.spawn("bench", ATTRS, TAGS)
  end)
  must(entity.count() == ITERS * 3, "spawn 批必须真的造出 3×10⁴ 个实体")
end

-- ② destroy（8 属性 + 3 tag 的实体；先备好池子再量）────────────────────────────
do
  local world = new_world()
  local entity = fresh(world)
  entity.define("bench", SPEC)
  local pool = {}
  for i = 1, ITERS do
    pool[i] = entity.spawn("bench", ATTRS, TAGS)
  end
  report("destroy", function(round)
    entity.destroy(pool[round])
  end)
  must(entity.count() == 0, "destroy 批跑完存活数必须归零")
  must(#entity.ids() == 0, "destroy 批跑完 ids 必须为空")
end

-- ③ get / set / 复合 / attrs / tags / ids 全扫（同一 10⁴ 实体池）─────────────────
do
  local world = new_world()
  local entity = fresh(world)
  entity.define("bench", SPEC)
  local pool = {}
  for i = 1, ITERS do
    pool[i] = entity.spawn("bench", { hp = i })
  end
  report("get", function(round)
    entity.get(pool[round], "hp")
  end)
  report("set", function(round)
    entity.set(pool[round], "hp", round)
  end)
  report("get_set_pair", function(round)
    entity.get(pool[round], "hp")
    entity.set(pool[round], "hp", round)
  end)
  report("attrs8", function(round)
    entity.attrs(pool[round])
  end)
  report("tags3", function(round)
    entity.tags(pool[round])
  end)

  -- ids 全扫是**一次调用**：报总量与每实体摊薄（10⁴ 实体各调一次没有意义）。
  local began = os.clock()
  local ids = entity.ids()
  local total_ns = (os.clock() - began) * 1e9
  must(#ids == ITERS, "ids 全扫必须返回 10⁴ 个 id")
  print(
    ("PERF entity.paths_ids_10k=%.0f ns/entity  (total=%.3f ms, #ids=%d)"):format(
      total_ns / #ids,
      total_ns / 1e6,
      #ids
    )
  )
  must(ids[1] == 1 and ids[#ids] == ITERS, "ids 必须是升序稠密区间")
end
