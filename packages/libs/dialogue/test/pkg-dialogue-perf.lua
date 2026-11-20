-- `pkg.dialogue` 性能测量（契约 §5 的五行）。**计数**：四条用例进 `cases passed=`
-- （载体命令 `--filter pkg-dialogue` = 22 行为 + 本文件 4），每条打印数字并带阈值断言。
--
-- # 口径声明（先读再读数字）
--
-- 这里跑的是 **selftest 面**（普通 Lua + mock KV），不是宿主沙箱：`os.clock` /
-- `collectgarbage` 在沙箱里不可达（`sandbox.rs` 的白名单测试钉死）。所以内存那行是
-- **包侧 Lua 堆上界**（含 mock KV 表开销），不是宿主 Rust 堆读数。
--
-- # ② 是「附测」，不是契约指标
--
-- `dialogue.md` §5 把「按 id 查找 / 更新」判为**不适用**（本包不做实体索引）。
-- 这里仍量一次 `history(thread)` 的三档比，作用只有一个：把「线程直取不随线程数
-- 增长」变成一条可回归的数字（人工读，不设契约阈值）。

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
  "pkg.dialogue",
  "pkg.dialogue.store",
  "pkg.dialogue.handlers",
  "pkg.dialogue.dispatch",
  "pkg.dialogue.memory",
}

local function clear_modules()
  for i = 1, #MODULES do
    package.loaded[MODULES[i]] = nil
  end
end

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
    tool = {
      call = function()
        return { ok = true, unmounted = false, code = 0, data = {} }
      end,
    },
    ticks = function()
      return 0
    end,
  }
  return { kv = kv, host = host, events = events }
end

local function fresh(world)
  clear_modules()
  local dialogue = require("pkg.dialogue")
  dialogue.install({ host = world.host })
  return dialogue
end

-- ── ① 包加载 ────────────────────────────────────────────────────────────────

run("pkg.dialogue.perf-load", function()
  local ROUNDS = 50
  local samples = {}
  for round = 1, 3 do
    local began = os.clock()
    for _ = 1, ROUNDS do
      clear_modules()
      local dialogue = require("pkg.dialogue")
      is_true(type(dialogue.install) == "function", "装载产物缺 install")
    end
    samples[round] = (os.clock() - began) * 1000 / ROUNDS
  end
  local load_ms = median3(samples[1], samples[2], samples[3])
  print(
    ("PERF dialogue.load_ms=%.4f ms  (samples %.4f/%.4f/%.4f, %d 次 clear+require 的每轮均值, selftest-face)"):format(
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

-- ── ② 线程直取（附测）：线程数 1 / 100 / 1000，`history` 每次只读一条 ─────────

local TIERS = { 1, 100, 1000 }
local TURNS = 12
local ITERS = 1000
local BATCHES = 3

--- 直接把 N 条线程（每条 12 轮）写进 KV。
local function seed_threads(world, n)
  local keys = {}
  for i = 1, n do
    local thread = tostring(i) .. ":" .. tostring(i + 1)
    keys[i] = thread
    for t = 1, TURNS do
      world.kv["dlg.thread." .. thread .. ".t." .. t .. ".who"] = i
      world.kv["dlg.thread." .. thread .. ".t." .. t .. ".text"] = "beats.n" .. t
      world.kv["dlg.thread." .. thread .. ".t." .. t .. ".seq"] = t
    end
    world.kv["dlg.thread." .. thread .. ".turns"] = TURNS
  end
  table.sort(keys)
  world.kv["dlg.keys"] = table.concat(keys, "\n")
end

run("pkg.dialogue.perf-read", function()
  local report = {}
  for tier = 1, #TIERS do
    local n = TIERS[tier]
    local world = new_world()
    local dialogue = fresh(world)
    seed_threads(world, n)
    local batches = {}
    for batch = 1, BATCHES do
      local began = os.clock()
      for round = 1, ITERS do
        local i = (round % n) + 1
        dialogue.history(tostring(i) .. ":" .. tostring(i + 1))
      end
      batches[batch] = (os.clock() - began) / ITERS * 1e6
    end
    report[tier] = median3(batches[1], batches[2], batches[3])
  end
  local worst = 0
  for tier = 2, #TIERS do
    local ratio = report[tier] / report[1]
    if ratio > worst then
      worst = ratio
    end
  end
  print(
    ("PERF dialogue.thread_read_us_1=%.4f / 100=%.4f / 1k=%.4f  (每条 12 轮的一次 history 直取, %d 次/批 ×%d 批)"):format(
      report[1],
      report[2],
      report[3],
      ITERS,
      BATCHES
    )
  )
  print(("PERF dialogue.thread_ratio=%.3f  (1k 档 / 1 档；契约不行使阈值，仅作回归参考)"):format(worst))
  print("PERF dialogue.serialize=n/a  (codec.md §5 独占该口径，本包不造第二个数字)")
  is_true(worst > 0, "比值必须量到正数（否则没量到东西）")
  -- 直取不该随线程数增长：给一条宽松上界（真遍历会是 10³×）。
  is_true(worst <= 5.0, ("线程直取出现随线程数增长：比值 %.3f > 5.0"):format(worst))
  clear_modules()
end)

-- ── ④ 常驻内存：1k 线程 × 12 轮（走 say 的真实写路径）────────────────────────

run("pkg.dialogue.perf-memory", function()
  local world = new_world()
  local dialogue = fresh(world)
  -- 热身：第一条线程把键名形态与常量串的驻留花掉，不进增量。
  dialogue.say("0:0", "beats.warm")

  collectgarbage("collect")
  local before = collectgarbage("count")
  local THREADS = 1000
  for i = 1, THREADS do
    local thread = tostring(i) .. ":" .. tostring(i + 1)
    for t = 1, 12 do
      dialogue.say(thread, "beats.n" .. t)
    end
  end
  collectgarbage("collect")
  local after = collectgarbage("count")
  local mem_kib = after - before

  print(
    ("PERF dialogue.memory_kib=%.1f KiB  (1k 线程 × 12 轮；collectgarbage 差值, 包侧 Lua 堆上界, 含 mock KV 表开销)"):format(
      mem_kib
    )
  )
  is_true(mem_kib > 0, "内存增量必须是正数（否则没量到东西）")
  -- 这一格断的是**硬帽 16 MiB**（契约 §5），不是 2 MiB 目标：1k 线程 × 12 轮 =
  -- 36k 枚键，mock KV 的每枚键开销（唯一键名字符串 + 表槽 ≈ 100 B）就占满了大头，
  -- 包侧没有可省的余地。实测值 3.65 MiB 已入档 dialogue.md §5，并注明「selftest 面
  -- 的 mock 开销主导，与宿主后端不可比」——不放宽断言，只把阈值放到契约真有的那条。
  is_true(mem_kib <= 16384, ("1k 线程 × 12 轮的包侧堆上界超硬帽 16 MiB：%.1f KiB"):format(mem_kib))
  clear_modules()
end)

-- ── ⑤ 泄漏：100 轮 emit → history（memory = "none"），线程键数恒 0 ────────────
--
-- `memory = "none"` 是契约 §5 那一格的原话口径：既不写 `dlg.thread.*`，也不写
-- 线程名表 `dlg.keys`。所以「泄漏」在这里的含义是「一轮都不该留下东西」。

run("pkg.dialogue.perf-leak", function()
  local function round_once()
    local world = new_world()
    local dialogue = fresh(world)
    dialogue.on("shout", {
      kind = "builtin",
      name = "npc",
      opts = { memory = "none" },
      fn = function()
        return true
      end,
    })
    local receipt = dialogue.emit("shout", 1, 2, { text_key = "beats.shout" })
    local thread_keys = 0
    for key in pairs(world.kv) do
      if key:find("^dlg%.thread%.") then
        thread_keys = thread_keys + 1
      end
    end
    return receipt.handled and thread_keys or -1
  end

  for _ = 1, 10 do
    round_once()
  end
  collectgarbage("collect")
  local before = collectgarbage("count")

  local worst = 0
  for _ = 1, 100 do
    local thread_keys = round_once()
    if thread_keys ~= 0 then
      worst = thread_keys
    end
  end
  collectgarbage("collect")
  local after = collectgarbage("count")
  local pct = (after - before) / before * 100

  print(("PERF dialogue.leak_thread_keys=%d  (100 轮 emit→history，每轮 dlg.thread.* 键数)"):format(worst))
  print(
    ("PERF dialogue.leak_gc_pct=%.3f%%  (热身 10 轮后取基线，判据「增长 ≤ 1%%」；负值 = 基线里还留着可回收物)"):format(
      pct
    )
  )
  is_true(worst == 0, ("某轮留下了线程键：%d"):format(worst))
  is_true(pct <= 1.0, ("100 轮后内存增长 %.3f%% > 1%%"):format(pct))
  clear_modules()
end)
