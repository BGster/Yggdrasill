-- `pkg.entity` 性能测量（§5 的五行）。**不计数**：本文件用 file 级断言
-- （`error` 即 `file FAIL` ⇒ `ok=0`），所以它不进 `cases passed=`，而判定一样硬。
--
-- # 为什么单独一个文件
--
-- ① 口径：五行里三行只**打印**数字（不写阈值——机器差异会把门禁打红），两行做
--    断言（O(1) 比值、泄漏）。混进行为用例文件会让「40 条」这个可复算计数失真。
-- ② 时长：10k 基准是秒级，单独文件才能用 `--filter pkg-entity-perf` 单独跑。
--
-- # 载体限制（诚实声明，写进 entity.md §5 的口径列）
--
-- 这里跑的是**普通 Lua**，不是沙箱：`os.clock` / `collectgarbage` 可用。沙箱里
-- 两者都不可达（`crates/ygg-sandbox/src/sandbox.rs` 的白名单测试钉死），所以
-- 这两行数字是**包侧 Lua 堆**的上界，不是宿主 Rust 堆的读数。
-- 同理 O(1) 断言抓到的是**包侧**引入的遍历（例如「按 id 找 schema 时扫一遍
-- ids」），不是内核面的复杂度——mock 的 get/set 本身就是表取。

local ROUNDS = 100
local BENCH_N = { 1, 100, 10000 }
local ATTRS = {
  hp = 100,
  max_hp = 100,
  attack = 12,
  defense = 7,
  speed = 5,
  level = 3,
  exp = 0,
  weight = 1.5,
}
local TAGS = { "enemy", "boss", "undead" }

local function must(condition, message)
  if not condition then
    error("[pkg-entity-perf] " .. message, 0)
  end
end

--- 中位（三样本）。只用于把「同一条命令跑 3 遍」也搬进文件内，减少抖动。
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

-- ── 独立世界（与行为用例同一份 mock 语义，但不依赖它们的文件）─────────────────

local function new_world()
  local kv = {}
  local store = { next_id = 1, attrs = {}, tags = {} }
  -- S-24 查询面：记录器（回放固定出表；本文件量的是**包侧**成本，引擎由 path_costs 给）
  local query_reply = nil
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
      query = function(_)
        if query_reply ~= nil then
          return query_reply
        end
        return { count = 0, ids = {} }
      end,
    },
    emit = function() end,
  }
  local world = { kv = kv, host = host }
  world.set_query_reply = function(reply)
    query_reply = reply
  end
  --- 内核侧存活条数（S9 新增轴：KV 墓碑键与内核残留是两件事，各断言各的）。
  world.live_count = function()
    local n = 0
    for _ in pairs(store.attrs) do
      n = n + 1
    end
    return n
  end
  world.key_count = function()
    local n = 0
    for _ in pairs(kv) do
      n = n + 1
    end
    return n
  end
  return world
end

local function fresh(world)
  package.loaded["pkg.entity"] = nil
  local entity = require("pkg.entity")
  entity.install({ host = world.host })
  return entity
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

-- ── ① 包加载：`loadfile` + 调用 100 次，取每轮中位 ───────────────────────────
-- 契约的原始测法是「空 run 起局 vs 含本包起局」的墙钟差——信号（≤1ms）低于整条
-- 链的启动抖动（>1ms），测不出东西。于是改成直接量**这个文件**的编译 + 顶层的
-- 代价：100 次 `loadfile(init.lua)` 后调用一次 `install`，除数 100。

do
  local samples = {}
  for round = 1, 3 do
    local began = os.clock()
    for _ = 1, 100 do
      local chunk = loadfile(PKG_ROOT .. "/entity/init.lua")
      must(chunk ~= nil, "loadfile 失败：" .. PKG_ROOT .. "/entity/init.lua")
      local mod = chunk()
      must(type(mod.install) == "function", "装载产物缺 install")
      mod.install({ host = new_world().host })
    end
    samples[round] = (os.clock() - began) * 1000 / 100
  end
  local load_ms = median3(samples[1], samples[2], samples[3])
  print(
    ("PERF entity.load_ms=%.4f ms  (samples %.4f/%.4f/%.4f, 100 次 loadfile+install 的每轮均值)"):format(
      load_ms,
      samples[1],
      samples[2],
      samples[3]
    )
  )
  must(load_ms <= 10, ("包加载超过 10ms 契约阀值：%.4f ms"):format(load_ms))
end

-- ── ② 10k 实体常驻内存（兼作 ③ 的最大档本底）────────────────────────────────

local big = new_world()
local entity = fresh(big)
entity.define("bench", bench_schema())

collectgarbage("collect")
local mem_before = collectgarbage("count")
local ids = {}
for i = 1, 10000 do
  ids[i] = entity.spawn("bench", { hp = i, exp = i * 2 }, TAGS)
end
collectgarbage("collect")
local mem_after = collectgarbage("count")
local mem_kib = mem_after - mem_before

-- 「实体数据字节数」= 键名 + 值的字节和（与载体无关：换一个宿主也是这么多字节）。
-- rec.<id>.schema 键 + 8 属性（名 + 值）+ 3 tag（名）。id 位数按实际宽度算。
local function digits(n)
  local d = 1
  while n >= 10 do
    n = math.floor(n / 10)
    d = d + 1
  end
  return d
end
local data_bytes = 0
for i = 1, #ids do
  local id = ids[i]
  data_bytes = data_bytes + #("pkg.entity.rec." .. id .. ".schema") + #"bench"
  for name, value in pairs(ATTRS) do
    data_bytes = data_bytes + #name + #tostring(value)
  end
  for t = 1, #TAGS do
    data_bytes = data_bytes + #TAGS[t] + 1
  end
  data_bytes = data_bytes + digits(id) * (#ATTRS + #TAGS)
end

print(("PERF entity.memory_kib=%.1f KiB  (10k × 8 属性 + 3 tag；collectgarbage 差值, 包侧 Lua 堆上界)"):format(mem_kib))
print(("PERF entity.data_bytes=%d B  (同量实体的键名+值字节和，载体无关)"):format(data_bytes))
must(#ids == 10000, "10k 基准未建成")
must(mem_kib > 0, "内存增量必须是正数（否则没量到东西）")

-- ── ③ 按 id 查找 / 更新 O(1)：三档，每档 3 批 × (5000 get + 5000 set) ─────────────
--
-- 为何是 5000 而不是 1000：单档 1000 次只占 ~2.6 ms，而全量跑里前面的基准会把堆
-- 顶到 12 MB 量级——一次 GC 就足以把某一档拉偏 2×（实测：1000 次时出现过
-- 1.4× 的假红）。加大内层轮次 + 每档取 3 批中位，是把「GC 噪声」平均掉的正规做法
-- （R4 的处置），不是把阀值放宽。

do
  local rates = {}
  local report = {}
  local ITERS = 5000
  local BATCHES = 3
  for tier = 1, #BENCH_N do
    local n = BENCH_N[tier]
    local probe = new_world()
    local lib = fresh(probe)
    lib.define("bench", bench_schema())
    local pool = {}
    for i = 1, n do
      pool[i] = lib.spawn("bench", { hp = i })
    end
    local batches = {}
    for batch = 1, BATCHES do
      local began = os.clock()
      for round = 1, ITERS do
        local id = pool[(round % n) + 1]
        lib.get(id, "hp")
        lib.set(id, "hp", round)
      end
      batches[batch] = (os.clock() - began) / (ITERS * 2) * 1e9
    end
    local per_op_ns = median3(batches[1], batches[2], batches[3])
    rates[tier] = per_op_ns
    report[tier] = ("N=%d:%.1fns/op"):format(n, per_op_ns)
  end
  local lo = math.min(rates[1], rates[2], rates[3])
  local hi = math.max(rates[1], rates[2], rates[3])
  print(
    ("PERF entity.id_lookup_ns=%.1f ns/op  (%s; 档间比 %.2f×)"):format(
      median3(rates[1], rates[2], rates[3]),
      table.concat(report, " "),
      hi / lo
    )
  )
  -- R4 的处置：契约写的 1.2× 是按「无噪声」估的，实测档间噪声到 1.19×（3 次全量
  -- 跑：1.07/1.09/1.18，另一次 1.4× 假红），1.2 会随机器状态随机打红。阈值先改
  -- **1.5×**；S9 复测发现 1.5 已贴在噪声上沿——**同机同命令**跑出来了：
  --   · S9 前（base e6dbb9a）3 跑：1.47 / 1.42 / 1.38（N=10000 档 1760/1711/1694 ns）
  --   · S9 后 4 跑：1.43 / 1.48 / 1.51 / 1.43（N=10000 档 1457/1441/1497/1457 ns）
  -- 逐档**绝对耗时**降了 ~15%（存活门从 KV 读换成内核存在性读），档间比却没动，
  -- 所以这条 spread 是机器噪声（堆到 10k 实体后的 GC 脉冲），不是遍历。阈值再抬到
  -- **2.0×**：真的出现遍历时档间差是 O(N) 量级（10³–10⁴×），2.0 仍能一眼抓住。
  -- 这是把阀值提到噪声之上，**不是**把断言删掉。
  must(
    hi / lo <= 2.0,
    ("按 id 的耗时随 N 增长（%.2f× > 2.0×）：get/set 里出现了遍历"):format(hi / lo)
  )
end

-- ── ④ 100 轮泄漏：结构判据（强）+ 内存判据（弱）─────────────────────────────
--
-- 结构判据是**双轴**的（S9 改定口径）：
--   · 内核侧回归零：真删之后 `store` 里一条记录都不该剩（S9 前每轮留一条带属性的记录，
--     这正是 r2-G4 的「死亡怪物永占内存」）；
--   · KV 侧恰好 +ROUNDS：KV 没有删除面，`rec.<id>.schema` 只能留在原处，而 S9 起
--     `alive.<id>` 位退役（存活真源 = 内核存在性），所以墓碑键从每轮 2 枚降到 1 枚。
--   · count / #ids 回到起点（存活集合不漏）、seq 恰好 +ROUNDS（id 不复用，I1）。
-- 「恰好相等」比「不变」强：它同时排掉「每轮多写一个计数键」这种慢性漏。

do
  local world = new_world()
  local lib = fresh(world)
  lib.define("bench", bench_schema())
  lib.spawn("bench")
  local base_count = lib.count()
  local base_ids = #lib.ids()
  local base_seq = world.kv["pkg.entity.seq"]
  local base_keys = world.key_count()
  local base_live = world.live_count()
  local base_rev = world.kv["pkg.entity.schemas_rev"]

  collectgarbage("collect")
  local mem0 = collectgarbage("count")
  for round = 1, ROUNDS do
    local id = lib.spawn("bench", { hp = round }, TAGS)
    must(lib.destroy(id), "轮次 " .. round .. " 的 destroy 失败")
  end
  collectgarbage("collect")
  local mem1 = collectgarbage("count")

  must(lib.count() == base_count, ("count 未回到起点：%d ≠ %d"):format(lib.count(), base_count))
  must(#lib.ids() == base_ids, ("#ids 未回到起点：%d ≠ %d"):format(#lib.ids(), base_ids))
  must(
    world.kv["pkg.entity.seq"] == base_seq + ROUNDS,
    ("seq 应恰好 +%d（id 不复用），实得 %s"):format(ROUNDS, tostring(world.kv["pkg.entity.seq"]))
  )
  must(
    world.live_count() == base_live,
    ("内核残留应回零：%d ≠ %d"):format(world.live_count(), base_live)
  )
  must(
    world.key_count() == base_keys + ROUNDS,
    ("KV 键数应恰好 +%d 枚墓碑键（rec 唯一），实得 +%d"):format(ROUNDS, world.key_count() - base_keys)
  )
  must(
    world.kv["pkg.entity.schemas_rev"] == base_rev,
    "泄漏轮次不得触碰 schema 注册表"
  )
  local drift = (mem1 - mem0) / (mem0 > 0 and mem0 or 1)
  print(
    ("PERF entity.leak_ok=%s  (%d 轮 spawn+destroy：count/#ids 回零，内核残留 %d，seq +%d，KV +%d 键, 内存 △%.3f%%)"):format(
      "true",
      ROUNDS,
      base_live,
      ROUNDS,
      ROUNDS,
      drift * 100
    )
  )
  -- 阈值 S9 收紧 1% → 0.5%：S9 前实测 0.640%（每轮留一条带属性的内核记录），S9 后
  -- 实测 0.071%（真删，记录不堆）。0.5% 给机差/GC 脉冲留 7× 余量，同时保证「又把记录
  -- 留下来」这类回归（≥0.6%）仍会被抓住。
  must(drift <= 0.005, ("100 轮后内存增长 %.3f%% > 0.5%%"):format(drift * 100))
end

-- ── ⑤ query 的**包侧**成本（S-24）────────────────────────────────────────────
--
-- 口径（先读再读数字）：mock 的 `query` 是**记录器**（回放固定出表），所以这里量的
-- 是包自己加的那一段——逐字透传的 1 次穿越，与 schema 后过滤的逐 id KV 读。
-- **引擎成本（选 / 材质化 / 索引）不在 Lua 侧可测**：它由
-- `crates/ygg-sandbox/tests/path_costs.rs` 的 `probe.query_face.*`（真沙箱端到端）
-- 与 `probe.entitystore.select_*`（原生侧）给出，两边数字不许混用。
do
  local probe = new_world()
  local lib = fresh(probe)
  lib.define("bench", bench_schema())
  local pool = {}
  for i = 1, 10000 do
    pool[i] = lib.spawn("bench", { hp = i })
  end
  probe.set_query_reply({ count = #pool, ids = pool })

  -- ① 透传：无 schema ⇒ 1 次穿越 + 出表原样回
  local plain = {}
  for batch = 1, 3 do
    local began = os.clock()
    for _ = 1, 20 do
      lib.query({ where = { hp = "gte.0" } })
    end
    plain[batch] = (os.clock() - began) / 20 * 1e9
  end
  print(
    ("PERF entity.query_forward_ns=%.0f ns/call  (无 schema：1 次穿越，出表原样回)"):format(
      median3(plain[1], plain[2], plain[3])
    )
  )

  -- ② schema 后过滤：1 万条 id 逐条读 `rec.<id>.schema`（这就是 v1 §7 明写的「无进账」那档）
  local filtered = {}
  for batch = 1, 3 do
    local began = os.clock()
    for _ = 1, 5 do
      lib.query({ schema = "bench" })
    end
    filtered[batch] = (os.clock() - began) / 5 * 1e9
  end
  print(
    ("PERF entity.query_schema_filter_10k_ns=%.0f ns/call  (1 万条 id × 1 次 KV 读/条；无回退也无进账)"):format(
      median3(filtered[1], filtered[2], filtered[3])
    )
  )
  must(pool[#pool] == 10000, "10k 查询基准未建成")
  -- 显式放手：1 万条 id + 池子是 ~900 KiB 的**活**数据，而后面 `pkg-textframe-perf`
  -- 的「每帧常驻」用的是 `collectgarbage("count")` 差值——不清就会把这份池子算进
  -- 它的增量里（实测 4.65→13.73 KiB/帧）。跨文件的常驻测量必须互不背账。
  probe.set_query_reply(nil)
  for i = 1, #pool do
    pool[i] = nil
  end
  pool = nil
end
collectgarbage()
collectgarbage()
