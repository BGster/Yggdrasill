-- `pkg.quest` 性能测量（契约 §5 的五行）。**计数**：四条用例进 `cases passed=`
-- （载体命令 `--filter pkg-quest` = 20 行为 + 本文件 4），每条都打印数字并带阈值断言。
--
-- # 口径声明（先读再读数字）
--
-- 这里跑的是 **selftest 面**（普通 Lua + mock KV），不是宿主沙箱：`os.clock` /
-- `collectgarbage` 在沙箱里不可达（`crates/ygg-sandbox/src/sandbox.rs` 的白名单
-- 测试钉死）。所以：
--   * 内存那行是**包侧 Lua 堆上界**（含 mock KV 的表开销），不是宿主 Rust 堆读数；
--   * 「序列化吞吐」本包**不适用**（`codec.md` §5 独占那个口径，不造第二个数字）；
--   * 所有打印串带 `PERF quest.` 前缀，第三次取中位（同一文件内三批取中位，
--     与 `pack_packages.rs:312-318` 同一做法）。
--
-- # 为什么 O(1) 断在「档间比」而不是绝对纳秒
--
-- 绝对纳秒随机器与 GC 脉冲漂（r1 实证：小分母档出现过 1.4× 假红）。判据取
-- `t(10k)/t(1) ≤ 1.5`：真正的遍历是 10³–10⁴×，这条阈值挡得住它，又不挡 GC 噪声。

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

local function is_true(value, message)
  if not value then
    error(message, 2)
  end
end

local function run(name, fn)
  local ok, err = pcall(fn)
  check(name, ok, ok and nil or err)
end

--- 中位（三样本）。
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

local MODULES = {
  "pkg.quest",
  "pkg.quest.store",
  "pkg.quest.types",
  "pkg.quest.flow",
}

local function clear_modules()
  for i = 1, #MODULES do
    package.loaded[MODULES[i]] = nil
  end
end

-- ── 独立世界（与行为用例同一份 mock 语义，但不依赖它们的文件）─────────────────

local function new_world()
  local kv = {}
  local events = {}
  local host = {
    kv = {
      get = function(key)
        return kv[key]
      end,
      set = function(key, value)
        if type(value) == "table" or type(value) == "function" then
          error("unsupported payload type: " .. type(value), 0)
        end
        kv[key] = value
      end,
    },
    emit = function(name, payload)
      events[#events + 1] = { name = name, payload = payload }
    end,
    ticks = function()
      return 0
    end,
  }
  return { kv = kv, host = host, events = events }
end

local function fresh(world)
  clear_modules()
  local quest = require("pkg.quest")
  quest.install({ host = world.host })
  return quest
end

-- ── ① 包加载 ────────────────────────────────────────────────────────────────

run("pkg.quest.perf-load", function()
  local ROUNDS = 50
  local samples = {}
  for round = 1, 3 do
    local began = os.clock()
    for _ = 1, ROUNDS do
      clear_modules()
      local quest = require("pkg.quest")
      is_true(type(quest.install) == "function", "装载产物缺 install")
    end
    samples[round] = (os.clock() - began) * 1000 / ROUNDS
  end
  local load_ms = median3(samples[1], samples[2], samples[3])
  print(
    ("PERF quest.load_ms=%.4f ms  (samples %.4f/%.4f/%.4f, %d 次 clear+require 的每轮均值, selftest-face)"):format(
      load_ms,
      samples[1],
      samples[2],
      samples[3],
      ROUNDS
    )
  )
  is_true(load_ms < 10.0, ("包加载超过契约阈值 10ms：%.4f ms"):format(load_ms))
  clear_modules()
end)

-- ── ② 按 id 查找 / 更新 O(1)：N = 1 / 100 / 10k 三档 ─────────────────────────
--
-- 三档的实例直接**写进 mock KV**（不走 `open`）：要量的是 `get` / `advance`，
-- 建库成本不该进这个数字（那是 `open` 的账，单独打印在下面）。模板固定 1000 步，
-- 于是三档之间只有「实例数」一个变量。

local BENCH_STEPS = 1000
local BENCH_META = { "k1", "k2", "k3" }
local TIERS = { 1, 100, 10000 }
local ITERS_GET = 5000
local ITERS_ADV = 300
local BATCHES = 3

--- 直接把一份 bench 世界写进 KV（模板 + N 个 open 实例）。
local function seed_bench(world, n)
  local kv = world.kv
  kv["quest.types"] = "bench"
  kv["quest.t.bench.steps"] = BENCH_STEPS
  for i = 1, BENCH_STEPS do
    kv["quest.t.bench.s." .. i .. ".id"] = "s" .. i
    kv["quest.t.bench.s." .. i .. ".guide"] = "g" .. i
    kv["quest.t.bench.s." .. i .. ".on_enter"] = ""
  end
  kv["quest.t.bench.m.names"] = table.concat(BENCH_META, "\n")
  for i = 1, #BENCH_META do
    kv["quest.t.bench.m." .. BENCH_META[i]] = "number"
  end
  for id = 1, n do
    kv["quest.inst." .. id .. ".type"] = "bench"
    kv["quest.inst." .. id .. ".step"] = 1
    kv["quest.inst." .. id .. ".state"] = "open"
    kv["quest.inst." .. id .. ".giver"] = 0
    kv["quest.inst." .. id .. ".taker"] = 0
    kv["quest.inst." .. id .. ".m.names"] = table.concat(BENCH_META, "\n")
    for m = 1, #BENCH_META do
      kv["quest.inst." .. id .. ".m." .. BENCH_META[m]] = m
    end
    kv["quest.index." .. id] = true
  end
  kv["quest.seq"] = n
end

run("pkg.quest.perf-o1", function()
  local get_report = {}
  local adv_report = {}
  local ratios = {}

  -- `open` 的代价单独记一笔（10k 档，100 次取均值）：它读一次模板（O(步数)）+
  -- 写实例与索引，与「按 id 查找/更新」不是同一格指标，只落 note 不设阈值。
  local open_us = nil

  for tier = 1, #TIERS do
    local n = TIERS[tier]
    local world = new_world()
    local quest = fresh(world)
    seed_bench(world, n)

    local get_batches = {}
    local adv_batches = {}
    for batch = 1, BATCHES do
      local began = os.clock()
      for round = 1, ITERS_GET do
        quest.get((round % n) + 1)
      end
      get_batches[batch] = (os.clock() - began) / ITERS_GET * 1e6

      began = os.clock()
      for round = 1, ITERS_ADV do
        quest.advance((round % n) + 1)
      end
      adv_batches[batch] = (os.clock() - began) / ITERS_ADV * 1e6
    end
    get_report[tier] = median3(get_batches[1], get_batches[2], get_batches[3])
    adv_report[tier] = median3(adv_batches[1], adv_batches[2], adv_batches[3])

    if n == TIERS[#TIERS] then
      local began = os.clock()
      for round = 1, 100 do
        quest.open("bench", { meta = { k1 = round } })
      end
      open_us = (os.clock() - began) / 100 * 1e6
    end

    -- 档间比：这一档相对最小档的耗时倍数（两条读/写面各一份）。
    if tier > 1 then
      ratios[#ratios + 1] = get_report[tier] / get_report[1]
      ratios[#ratios + 1] = adv_report[tier] / adv_report[1]
    end
  end

  local worst = 0
  for i = 1, #ratios do
    if ratios[i] > worst then
      worst = ratios[i]
    end
  end

  print(
    ("PERF quest.get_ns_1=%.1f / 100=%.1f / 10k=%.1f  (ns/op 三档, %d 次/批 ×%d 批取中位, selftest-face)"):format(
      get_report[1] * 1000,
      get_report[2] * 1000,
      get_report[3] * 1000,
      ITERS_GET,
      BATCHES
    )
  )
  print(
    ("PERF quest.advance_ns_1=%.1f / 100=%.1f / 10k=%.1f  (%d 次/批 ×%d 批取中位)"):format(
      adv_report[1],
      adv_report[2],
      adv_report[3],
      ITERS_ADV,
      BATCHES
    )
  )
  print(("PERF quest.ratio=%.3f  (10k 档 / 1 档 的最大倍数；判据 ≤ 1.5)"):format(worst))
  print(("PERF quest.open_us_10k=%.3f  (note：模板 %d 步的一次读 + 实例/索引写，不设阈值)"):format(open_us or -1, BENCH_STEPS))
  print("PERF quest.serialize=n/a  (codec.md §5 独占该口径，本包不造第二个数字)")

  is_true(worst <= 1.5, ("按 id 查找/更新出现随规模增长：档间比 %.3f > 1.5"):format(worst))
  is_true(worst > 0, "档间比必须量到正数（否则没量到东西）")
  is_true(open_us ~= nil and open_us > 0, "open 成本必须量到正数")
  clear_modules()
end)

-- ── ④ 常驻内存：1k 任务（4 步 + 3 个 meta 键）────────────────────────────────

run("pkg.quest.perf-memory", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("task", {
    steps = {
      { id = "accept", guide = "t.accept" },
      { id = "travel", guide = "t.travel" },
      { id = "deliver", guide = "t.deliver" },
      { id = "reward", guide = "t.reward" },
    },
    meta_spec = { difficulty = "number", giver_name = "string", urgent = "boolean" },
  })
  -- 先开一条：模板读与常量串的驻留发生在热身里，不进增量。
  quest.open("task", { meta = { difficulty = 1, giver_name = "warm", urgent = false } })

  collectgarbage("collect")
  local before = collectgarbage("count")
  for i = 2, 1000 do
    quest.open("task", { meta = { difficulty = i, giver_name = "npc", urgent = i % 2 == 0 } })
  end
  collectgarbage("collect")
  local after = collectgarbage("count")
  local mem_kib = after - before

  print(
    ("PERF quest.memory_kib=%.1f KiB  (1k × 4 步 + 3 meta 键；collectgarbage 差值, 包侧 Lua 堆上界, 含 mock KV 表开销)"):format(
      mem_kib
    )
  )
  is_true(mem_kib > 0, "内存增量必须是正数（否则没量到东西）")
  -- 断的是 P0 的 1 MiB **目标值**（不是硬帽 16 MiB）：这条数字当场可比，超了就红。
  -- mock KV 的表开销比宿主 SQLite/JSON 后端更重，所以真实跑局只会更小。
  is_true(mem_kib <= 1024, ("1k 任务的包侧堆上界超 1 MiB 目标：%.1f KiB"):format(mem_kib))
  clear_modules()
end)

-- ── ⑤ 泄漏：100 轮 open → advance×末步，每轮 index 条目恒为 1 ────────────────
--
-- 为什么是「每轮重置后恒为 1」而不是「100 轮后回到起点」：包**没有删除面**
-- （契约 §4 的 11 个函数里没有 delete，codec I5 亦明令不删），所以「回到起点」
-- 只能在每轮一份新 KV 的语义下成立（回写 D7 的措辞即此口径）。

run("pkg.quest.perf-leak", function()
  local function round_once()
    local world = new_world()
    local quest = fresh(world)
    quest.define("short", {
      steps = { { id = "one" }, { id = "two" }, { id = "three" }, { id = "four" } },
    })
    local id = quest.open("short")
    quest.advance(id)
    quest.advance(id)
    local state = quest.advance(id)
    local index = 0
    for key in pairs(world.kv) do
      if key:find("^quest%.index%.") then
        index = index + 1
      end
    end
    return index, state
  end

  -- 10 轮热身：键名字符串的驻留是**一次性的**（同 id、同类型名），冷启动的驻留
  -- 不该记在泄漏账上；热身完再取基线，量到的才是「每轮该被收走的东西」。
  for _ = 1, 10 do
    round_once()
  end
  collectgarbage("collect")
  local before = collectgarbage("count")

  local worst_index = 0
  for _ = 1, 100 do
    local index, state = round_once()
    is_true(state == "open", "第三跳不该到终态（四步模板）")
    if index ~= 1 then
      worst_index = index
    end
  end
  collectgarbage("collect")
  local after = collectgarbage("count")
  local pct = (after - before) / before * 100

  print(
    ("PERF quest.leak_index=%d  (100 轮 open→advance×末步，每轮 quest.index 条目数)"):format(worst_index == 0 and 1 or worst_index)
  )
  print(
    ("PERF quest.leak_gc_pct=%.3f%%  (热身 10 轮后取基线，判据「增长 ≤ 1%%」；负值 = 基线里还留着可回收物)"):format(
      pct
    )
  )
  is_true(worst_index == 0, ("某轮的 index 条目数不是 1：%d"):format(worst_index))
  is_true(pct <= 1.0, ("100 轮后内存增长 %.3f%% > 1%%"):format(pct))
  clear_modules()
end)
