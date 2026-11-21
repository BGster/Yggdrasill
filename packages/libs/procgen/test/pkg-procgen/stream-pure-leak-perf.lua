-- stream-pure-leak-perf：用例段自 pkg-procgen.lua 原样搬移（S-25 批十三测试件连拆）。
return function(C)
  local S, run, eq, eq_seq, is_true, expect_error, fresh, weight_pool, state_moved, KAT =
    C.S, C.run, C.eq, C.eq_seq, C.is_true, C.expect_error, C.fresh, C.weight_pool, C.state_moved, C.KAT
run("pkg.procgen.diff-seed-diff-hash", function()
  local procgen = fresh()
  local codec = require("pkg.codec")
  procgen.install()
  local params = { w = 12, h = 12, cell = 4, octaves = 2 }
  is_true(procgen.define("hill", { algo = "terrain.noise", params = params }), "define hill")
  local one = procgen.run("hill", "seed-a")
  local two = procgen.run("hill", "seed-b")
  is_true(procgen.hash(one) ~= procgen.hash(two), "异 seed ⇒ 异 hash")
  is_true(codec.encode(one) ~= codec.encode(two), "异 seed ⇒ 异字节")

  -- 种子文本口径：整数 7 与字符串 "7" 归一（同一段文本 ⇒ 同一条流），这是**已声明**
  -- 的性质而不是缺陷——契约 §2 写的就是「number → 文本，string → 原样」。
  is_true(procgen.define("hill2", { algo = "terrain.noise", params = params }), "define hill2")
  eq(procgen.hash(procgen.run("hill2", 7)), procgen.hash(procgen.run("hill2", "7")), "7 与 \"7\" 同一条流")
end)

-- ── 用例 9：流隔离（插入新流不改既有流）─────────────────────────────────────

run("pkg.procgen.stream-isolation", function()
  local procgen = fresh()
  local baseline = {}
  local first = procgen.rng("iso", "main")
  for i = 1, 5 do
    baseline[i] = first:next32()
  end

  -- 中间插一条新流并抽若干：若存在任何全局计数器，下面这段就会变。
  local extra = procgen.rng("iso", "extra")
  for _ = 1, 7 do
    extra:next32()
  end

  local again = {}
  local second = procgen.rng("iso", "main")
  for i = 1, 5 do
    again[i] = second:next32()
  end
  eq_seq(again, baseline, "插入新流后既有流的序列不变")

  local mob = procgen.rng("iso", "mob")
  local differs = false
  for i = 1, 5 do
    if mob:next32() ~= baseline[i] then
      differs = true
    end
  end
  is_true(differs, "两条流名 ⇒ 两段不同序列")
end)

-- ── 用例 10：存档续生成（state 是副本不是句柄）───────────────────────────────

run("pkg.procgen.state-resume", function()
  local procgen = fresh()
  local original = procgen.rng("resume", "main")
  for _ = 1, 5 do
    original:next32()
  end
  local snapshot = procgen.state(original)
  local tail = {}
  for i = 1, 5 do
    tail[i] = original:next32()
  end

  local resumed = procgen.rng(nil, nil, snapshot)
  for i = 1, 5 do
    eq(resumed:next32(), tail[i], "续生成第 " .. i .. " 个值")
  end

  -- 副本语义：改快照不影响句柄（否则「存档」与「继续玩」会互相踩）。
  local live = procgen.state(original)
  snapshot[1] = (snapshot[1] ~ 0xFF)
  eq(procgen.state(original)[1], live[1], "改快照不影响句柄状态")
end)

-- ── 用例 11：run 是纯的（不碰宿主面）────────────────────────────────────────

run("pkg.procgen.run-pure", function()
  local procgen = fresh()
  local touches = {}
  local host = setmetatable({}, {
    __index = function(_, key)
      touches[#touches + 1] = tostring(key)
      return nil
    end,
  })
  procgen.install({ host = host })
  procgen.define("hill", { algo = "terrain.noise", params = { w = 8, h = 8, cell = 4, octaves = 2 } })
  local product = procgen.run("hill", "pure")
  eq(#product.cells, 64, "产物照常返回")
  eq(#touches, 0, "一次 run 不摸宿主面（KV / emit / on）")
end)

-- ── 用例 15：静态面（无外部熵、无标准库表、无宿主面）────────────────────────

--- 包的源文件清单：来自 manifest 的 packages 段（内容寻址的那份，天然排除 test/**）。
local function pkg_sources(id)
  local manifest = rawget(_G, "__MANIFEST")
  if type(manifest) ~= "table" or type(manifest.packages) ~= "table" then
    error("manifest 缺 packages 段（载体 pack.json 未声明包或未重算 manifest）", 0)
  end
  for i = 1, #manifest.packages do
    local package = manifest.packages[i]
    if package.id == id then
      local out = {}
      for j = 1, #package.files do
        out[#out + 1] = package.files[j].path
      end
      return out
    end
  end
  error("manifest 的 packages 段没有包 " .. id, 0)
end

local function source_text(rel)
  local full = PKG_ROOT .. "/" .. rel
  local handle = assert(io.open(full, "rb"), "读不到 " .. full)
  local text = handle:read("a")
  handle:close()
  return text
end

run("pkg.procgen.static-no-entropy", function()
  local paths = pkg_sources("procgen")
  is_true(#paths >= 7, "manifest 里 procgen 源文件 ≥ 7，实得 " .. #paths)
  local plain = {
    "math.random",
    "math.randomseed",
    "io.",
    "collectgarbage",
    "print(",
    "string.",
    "table.",
    "math.",
    "ygg.",
  }
  local problems = {}
  for i = 1, #paths do
    local text = source_text(paths[i])
    for j = 1, #plain do
      if text:find(plain[j], 1, true) ~= nil then
        problems[#problems + 1] = paths[i] .. " 含 " .. plain[j]
      end
    end
    -- `os.` 必须带边界：`algos.` 与 `pkg.procgen.algos.*` 都含裸子串 "os."
    if text:find("[^%w_]os%.") ~= nil then
      problems[#problems + 1] = paths[i] .. " 含 os."
    end
  end
  is_true(#problems == 0, table.concat(problems, "; "))

  -- 键序铁律（`rng.weighted` 的确定性地基）：取数面**不得**用 `pairs` / `next`。
  -- 池序必须是数组声明序；表遍历序一旦掺进来，「同 seed 同产物」就取决于虚拟机实现。
  -- 只扫 `rng.lua`：`init.lua` 的 `pairs` 是 `ERRORS` 副本面，不是取数路径。
  local rng_source = source_text("procgen/rng.lua")
  local unordered = {}
  for _, needle in ipairs({ "pairs(", "next(" }) do
    if rng_source:find(needle, 1, true) ~= nil then
      unordered[#unordered + 1] = "procgen/rng.lua 含 " .. needle
    end
  end
  is_true(#unordered == 0, table.concat(unordered, "; "))
end)

-- ── 用例 16：100 轮不增长 + 一张 64×64 产物的常驻内存 ────────────────────────

run("pkg.procgen.leak-100-runs", function()
  local procgen = fresh()
  procgen.install()
  procgen.define("leak", { algo = "terrain.noise", params = { w = 16, h = 16, cell = 4, octaves = 2 } })

  collectgarbage("collect")
  collectgarbage("collect")
  local before = collectgarbage("count")
  local baseline = procgen.hash(procgen.run("leak", "leak-seed"))
  local all_equal = true
  for _ = 1, 100 do
    if procgen.hash(procgen.run("leak", "leak-seed")) ~= baseline then
      all_equal = false
    end
  end
  collectgarbage("collect")
  collectgarbage("collect")
  local growth = (collectgarbage("count") - before) / before * 100
  print(string.format("perf.procgen.leak_100_equal=%d", all_equal and 1 or 0))
  print(string.format("perf.procgen.leak_gc_pct=%.3f", growth))
  is_true(all_equal, "100 轮同 seed 的 hash 全相等")
  is_true(growth <= 1, "100 轮后内存差 ≤ 1%，实得 " .. string.format("%.3f", growth) .. "%")

  procgen.define("map64", { algo = "terrain.noise", params = { w = 64, h = 64, cell = 8, octaves = 3 } })
  collectgarbage("collect")
  collectgarbage("collect")
  local before_map = collectgarbage("count")
  local map = procgen.run("map64", "mem")
  local map_kib = collectgarbage("count") - before_map
  print(string.format("perf.procgen.map64_mem_kib=%.1f", map_kib))
  eq(#map.cells, 64 * 64, "64×64 产物的格数")
  is_true(map_kib <= 256, "一张 64×64 产物 ≤ 256 KiB，实得 " .. string.format("%.1f", map_kib) .. " KiB")
end)

-- ── 用例 18：包加载与取数吞吐（三次取中位；print 面就是契约 §5 的口径）───────

run("pkg.procgen.perf-load", function()
  local samples = {}
  for round = 1, 5 do
    for name in pairs(package.loaded) do
      if type(name) == "string" and (name == "pkg.procgen" or name:find("^pkg%.procgen%.")) then
        package.loaded[name] = nil
      end
    end
    local began = os.clock()
    local module = require("pkg.procgen")
    module.install()
    samples[round] = (os.clock() - began) * 1000
  end
  table.sort(samples)
  local median = samples[3]
  print(string.format("perf.procgen.load_ms=%.3f", median))
  is_true(median < 10, "包加载 < 10ms，实得 " .. string.format("%.3f", median) .. "ms")

  local rates = {}
  for round = 1, 3 do
    local handle = require("pkg.procgen").rng("throughput")
    local began = os.clock()
    local total = 0
    for _ = 1, 200000 do
      total = total + handle:int(0, 255)
    end
    rates[round] = 200000 / (os.clock() - began)
    is_true(total >= 0, "取数结果参与累加（不让整段被当成不可观测）")
  end
  table.sort(rates)
  local rate = rates[2]
  print(string.format("perf.procgen.rng_int_per_s=%.0f", rate))
  is_true(rate >= 100000, "取数吞吐 ≥ 100k/s，实得 " .. string.format("%.0f", rate))

  -- 加权抽样吞吐：同一 20 项合成池，**复用权重数组**（调用方的正确用法）。
  -- 每次现建权重表会翻倍（表分配是主成本），那种用法不作为本包口径。
  local pool = weight_pool()
  local weighted_rates = {}
  for round = 1, 3 do
    local handle = require("pkg.procgen").rng("throughput/weighted")
    local began = os.clock()
    local total = 0
    for _ = 1, 200000 do
      total = total + handle:weighted(pool)
    end
    weighted_rates[round] = 200000 / (os.clock() - began)
    is_true(total >= 0, "加权取数结果参与累加（不让整段被当成不可观测）")
  end
  table.sort(weighted_rates)
  local weighted_rate = weighted_rates[2]
  print(string.format("perf.procgen.rng_weighted_per_s=%.0f", weighted_rate))
  print(string.format("perf.procgen.weighted_vs_int_ratio=%.2f", rate / weighted_rate))
  is_true(
    weighted_rate >= 100000,
    "加权取数吞吐 ≥ 100k/s，实得 " .. string.format("%.0f", weighted_rate)
  )
end)

-- ── 用例 19-23：加权抽样（`rng.weighted`）────────────────────────────────────
--
-- 权重池见文件头的共享夹具 `weight_pool()`（合成、n=20、Σw=109、首末零权）。

-- ── 用例 19：边界与错误码（五个失败路径全落既有六码）─────────────────────────

end
