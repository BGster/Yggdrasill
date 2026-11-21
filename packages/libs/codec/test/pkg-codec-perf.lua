-- `pkg.codec` 性能测量（§5 的五行）。**不计数**：file 级断言（`error` ⇒
-- `file FAIL` ⇒ `ok=0`），所以不进 `cases passed=`，判定一样硬。
--
-- # 基准集（契约 §5 定的唯一口径，不在这里另立）
--
-- 10k 实体 × (8 属性 + 3 tag) + KV 前缀 `quest` / `dlg` 各 1k 条；同机、跑三次取中位。
--
-- # 为什么 sha256 单独报、并且只采样 64 KiB
--
-- 纯 Lua sha256 是逐字节 + 逐轮位运算，吞吐比 C 实现低两三个数量级；把整份
-- ~2 MB 的 bundle 哈希一遍会让「五行数字」变成分钟级命令。所以这一行量
-- **MB/s 速率**（64 KiB 样本），并**显式标注**「整份约需 N 秒」是估算而非实测——
-- 把估算冒充实测才是问题，采样本身不是。
--
-- # 与其它包一样的两处载体限制
--
-- `os.clock` / `collectgarbage` 在沙箱里不可达（`sandbox.rs` 白名单测试钉死），
-- 所以数字是**普通 Lua** 上、包侧内存的读数。

local ENTITY_COUNT = 10000
local KV_PER_PREFIX = 1000
local ROUNDS = 100
local LEAK_ENTITY_COUNT = 50
local SHA_SAMPLE_BYTES = 65536

local ATTRS = {
  attack = 12,
  defense = 7,
  speed = 5,
  level = 3,
  exp = 0,
  hp = 100,
  max_hp = 100,
  weight = 1.5,
}
local TAGS = { "boss", "enemy", "undead" }

local function must(condition, message)
  if not condition then
    error("[pkg-codec-perf] " .. message, 0)
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

-- ── 独立世界（与行为用例同一份 mock 语义）────────────────────────────────────

local function new_world()
  local kv = {}
  local store = { next_id = 1, attrs = {}, tags = {} }
  local files = {}
  local world = { kv = kv, store = store, files = files }

  world.host = {
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
    fs = {
      write = function(path, content)
        files[path] = content
        return { path = path, bytes = #content }
      end,
      read = function(path)
        return files[path]
      end,
    },
  }
  return world
end

local function fresh(world)
  package.loaded["pkg.entity"] = nil
  package.loaded["pkg.codec"] = nil
  local entity = require("pkg.entity")
  entity.install({ host = world.host })
  local codec = require("pkg.codec")
  codec.install({ host = world.host })
  return entity, codec
end

local function bench_schema()
  local attrs = {}
  for name, value in pairs(ATTRS) do
    attrs[name] = { type = "number", default = value }
  end
  local tags = {}
  for i = 1, #TAGS do
    tags[TAGS[i]] = {}
  end
  return { attrs = attrs, tags = tags }
end

--- 把 KV 的点名键摊开：`quest.0..999` 与 `dlg.0..999`，值为一小段文本。
local function seed_kv(world)
  local keys = {}
  for i = 0, KV_PER_PREFIX - 1 do
    local quest = "quest." .. i
    local dialogue = "dlg." .. i
    world.kv[quest] = "任务 " .. i
    world.kv[dialogue] = i
    keys[#keys + 1] = quest
    keys[#keys + 1] = dialogue
  end
  return keys
end

-- ── ① 包加载：`loadfile` × 100（两个文件）+ install ─────────────────────────

do
  local samples = {}
  for round = 1, 3 do
    local began = os.clock()
    for _ = 1, 100 do
      local root = loadfile(PKG_ROOT .. "/codec/init.lua")
      local hash = loadfile(PKG_ROOT .. "/codec/sha256.lua")
      must(root ~= nil, "loadfile codec/init.lua 失败")
      must(hash ~= nil, "loadfile codec/sha256.lua 失败")
      local mod = root()
      hash()
      mod.install({ host = new_world().host })
    end
    samples[round] = (os.clock() - began) * 1000 / 100
  end
  local load_ms = median3(samples[1], samples[2], samples[3])
  print(
    ("PERF codec.load_ms=%.4f ms  (samples %.4f/%.4f/%.4f, 100 次 loadfile(2 文件)+install)"):format(
      load_ms,
      samples[1],
      samples[2],
      samples[3]
    )
  )
  must(load_ms <= 10, ("包加载超过 10ms 契约阀值：%.4f ms"):format(load_ms))
end

-- ── ② 序列化吞吐（基准集：10k 实体 + 2k KV 键）──────────────────────────────

local world = new_world()
local entity, codec = fresh(world)
entity.define("bench", bench_schema())
local kv_keys = seed_kv(world)

collectgarbage("collect")
local mem_before = collectgarbage("count")
for i = 1, ENTITY_COUNT do
  entity.spawn("bench", { hp = i, exp = i * 2 }, TAGS)
end

local export_began = os.clock()
local bundle = codec.export({ entities = true, ids = "index", keys = kv_keys, mod = "bench" })
local export_elapsed = os.clock() - export_began
collectgarbage("collect")
local mem_after = collectgarbage("count")

local export_rate = ENTITY_COUNT / export_elapsed
print(
  ("PERF codec.export=%.0f entities/s  (%d 实体 + %d KV 键，共 %.1f ms)"):format(
    export_rate,
    ENTITY_COUNT,
    #kv_keys,
    export_elapsed * 1000
  )
)
must(export_rate >= 5000, ("导出吞吐 %.0f entities/s 低于 5000"):format(export_rate))

local encode_began = os.clock()
local text = codec.encode(bundle)
local encode_elapsed = os.clock() - encode_began
print(
  ("PERF codec.encode=%.3f MB in %.1f ms  (%.2f MB/s，%d 字节)"):format(
    #text / (1024 * 1024),
    encode_elapsed * 1000,
    (#text / (1024 * 1024)) / encode_elapsed,
    #text
  )
)

-- decode：同一份文本解回 Lua 值（契约 §5 的第五条；此前没有这一行——`decode`
-- 一直被 import 路径掩盖着，而它的吞吐是 `read` 的天花板）。
local decode_began = os.clock()
local decoded = codec.decode(text)
local decode_elapsed = os.clock() - decode_began
print(
  ("PERF codec.decode=%.3f MB in %.1f ms  (%.2f MB/s)"):format(
    #text / (1024 * 1024),
    decode_elapsed * 1000,
    (#text / (1024 * 1024)) / decode_elapsed
  )
)
must(decoded ~= nil, "decode 必须解出值")
must((#text / (1024 * 1024)) / decode_elapsed >= 1.0, "decode 吞吐跌破 1 MB/s（解释器路径的回退口径）")

-- sha256：64 KiB 样本的 MB/s + 整份耗时**估算**（显式标注为估算）。
local sample = text:sub(1, SHA_SAMPLE_BYTES)
local sha_began = os.clock()
codec.sha256(sample)
local sha_elapsed = os.clock() - sha_began
local sha_mbps = (#sample / (1024 * 1024)) / sha_elapsed
print(
  ("PERF codec.sha256=%.3f MB/s  (64 KiB 样本；整份 %.2f MB 估算 %.1f s)"):format(
    sha_mbps,
    #text / (1024 * 1024),
    (#text / (1024 * 1024)) / sha_mbps
  )
)

print(
  ("PERF codec.memory_kib=%.1f KiB  (导出侧增量；bundle 文本 %d 字节，包侧 Lua 堆上界)"):format(
    mem_after - mem_before,
    #text
  )
)

-- ── ③ 导入吞吐：已在库的 10k 实体上 `overwrite` ──────────────────────────────

do
  local target = new_world()
  local entity_b, codec_b = fresh(target)
  entity_b.define("bench", bench_schema())
  for i = 1, ENTITY_COUNT do
    entity_b.spawn("bench", { hp = i, exp = i * 2 }, TAGS)
  end

  local sealed = codec_b.export({ entities = true, ids = "index", mod = "bench" })
  sealed.checksum = codec_b.checksum(sealed)

  local began = os.clock()
  local report = codec_b.import(sealed, { on_conflict = "overwrite" })
  local elapsed = os.clock() - began
  local rate = ENTITY_COUNT / elapsed
  print(
    ("PERF codec.import=%.0f entities/s  (%d 实体覆盖导入，共 %.1f ms；created=%d overwritten=%d)"):format(
      rate,
      ENTITY_COUNT,
      elapsed * 1000,
      report.created,
      report.overwritten
    )
  )
  must(rate >= 3000, ("导入吞吐 %.0f entities/s 低于 3000"):format(rate))
  must(report.created == 0, "覆盖导入不得新建实体")
  must(report.overwritten == ENTITY_COUNT, "覆盖导入必须全部走覆盖")
end

-- ── ④ 100 轮泄漏：`export → import(skip)`，结构全等 + 内存 ≤1% ────────────────
--
-- 轮次口径：`import(skip)` 是**零写**路径，所以实体数、KV 键数、count 都必须
-- 一动不动——这正是检验「导出/导入只在临时表里活动」的最干净形状。

do
  local leaky = new_world()
  local entity_c, codec_c = fresh(leaky)
  entity_c.define("bench", bench_schema())
  for i = 1, LEAK_ENTITY_COUNT do
    entity_c.spawn("bench", { hp = i }, TAGS)
  end
  local base_count = entity_c.count()
  local base_ids = #entity_c.ids()
  local base_keys = 0
  for _ in pairs(leaky.kv) do
    base_keys = base_keys + 1
  end

  local payload = codec_c.export({ entities = true, ids = "index", mod = "bench" })
  payload.checksum = codec_c.checksum(payload)

  collectgarbage("collect")
  local mem0 = collectgarbage("count")
  for _ = 1, ROUNDS do
    local report = codec_c.import(payload, { on_conflict = "skip" })
    must(report.skipped == LEAK_ENTITY_COUNT, "skip 轮次必须全部跳过")
    must(report.created == 0, "skip 轮次不得新建")
  end
  collectgarbage("collect")
  local mem1 = collectgarbage("count")

  local keys_after = 0
  for _ in pairs(leaky.kv) do
    keys_after = keys_after + 1
  end

  must(entity_c.count() == base_count, "泄漏轮次后 count 变了")
  must(#entity_c.ids() == base_ids, "泄漏轮次后 #ids 变了")
  must(keys_after == base_keys, "泄漏轮次后 KV 键数变了")
  local drift = (mem1 - mem0) / (mem0 > 0 and mem0 or 1)
  print(
    ("PERF codec.leak_ok=true  (%d 轮 export→import(skip)：count/#ids/KV 键数全等, 内存 △%.3f%%)"):format(
      ROUNDS,
      drift * 100
    )
  )
  must(drift <= 0.01, ("100 轮后内存增长 %.3f%% > 1%%"):format(drift * 100))
end

-- ── ⑤ 键序排序的规模行为（归并排序 O(n log n) vs 旧插入排序 O(n²)）──────────
--
-- 口径：同一 `codec` 实例、同一台机、逆序插入逼最坏路径。旧插入排序实测
-- 2 000 键 51.4 ms、4 000 键 174.8 ms（键数 ×2 ⇒ 耗时 ×3.4，二次行为）。
-- 这里给**绝对**上界而不是比值：比值在 3.4 与 2.2 之间太窄，机器抖动会打架。

do
  local target = new_world()
  local _, codec_w = fresh(target)
  local function measure(width)
    local value = {}
    for i = width, 1, -1 do
      value[("k%05d"):format(i)] = i
    end
    local began = os.clock()
    codec_w.encode(value)
    return (os.clock() - began) * 1000
  end
  local small = measure(2000)
  local large = measure(4000)
  print(
    ("PERF codec.wide_keys_ms=2000:%.2f/4000:%.2f  (ns/key %.0f/%.0f)"):format(
      small,
      large,
      small * 1e6 / 2000,
      large * 1e6 / 4000
    )
  )
  must(large <= 40, ("4 000 键耗时 %.2f ms > 40：键序排序退回二次行为？"):format(large))
end
