-- `pkg.procgen.*` 自测。契约 `docs/design/packages/procgen.md` §7。
--
-- # 为什么自带计数垫片与 mock
--
-- 包不依赖消费它的 mod（`packages/README.md` §5）：计数面直接挂 `__SELFTEST`，
-- 宿主面自己造。本包**不碰宿主面**（不写 KV、不发事件、不注册 handler），所以
-- mock 只在「证明它没碰」时出现（`run-pure`），其余用例不需要任何 mock。
--
-- # 隔离口径
--
-- 每条用例 `fresh()` 一次：清掉 `pkg.procgen*` 全部 `package.loaded` 项再 require，
-- 注册表与 install 状态一起归零。用例之间不共享任何状态。
--
-- 命名 ⇒ 计数可复算：文件名 `pkg-procgen.lua`，`--filter pkg-procgen` 跑到它。

local S = rawget(_G, "__SELFTEST")
if type(S) ~= "table" then
  S = { pass = 0, fail = 0 }
  _G.__SELFTEST = S
end

local function run(name, body)
  local ok, err = pcall(body)
  if ok then
    S.pass = S.pass + 1
  else
    S.fail = S.fail + 1
    print("FAIL " .. name .. ": " .. tostring(err))
  end
end

local function eq(actual, expected, label)
  if actual ~= expected then
    error(
      ("%s: expected %s, got %s"):format(tostring(label or "eq"), tostring(expected), tostring(actual)),
      2
    )
  end
end

local function eq_seq(actual, expected, label)
  eq(#actual, #expected, (label or "eq_seq") .. " 长度")
  for i = 1, #expected do
    eq(actual[i], expected[i], (label or "eq_seq") .. "[" .. i .. "]")
  end
end

local function is_true(value, label)
  if not value then
    error((label or "is_true") .. ": expected truthy", 2)
  end
end

local function expect_error(code, body, label)
  local ok, err = pcall(body)
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

--- 取一份**全新**的包实例（清缓存 = 注册表 / install 状态归零）。
local function fresh()
  package.loaded["pkg.procgen"] = nil
  for name in pairs(package.loaded) do
    if type(name) == "string" and name:find("^pkg%.procgen%.") then
      package.loaded[name] = nil
    end
  end
  return require("pkg.procgen")
end

--- xoshiro128** 的 KAT（参考 C 实现，state = {1,2,3,4} 的前 8 个 next）。
local KAT = { 11520, 0, 5927040, 70819200, 2031721883, 1637235492, 1287239034, 3734860849 }

-- ── 用例 1：PRNG 对齐参考实现 ────────────────────────────────────────────────

run("pkg.procgen.kat-xoshiro128ss", function()
  local procgen = fresh()
  local handle = procgen.rng(0, nil, { 1, 2, 3, 4 })
  local got = {}
  for i = 1, 8 do
    got[i] = procgen.next32(handle)
  end
  eq_seq(got, KAT, "state{1,2,3,4} 前 8 个 next32")

  -- 句柄方法与模块函数同一条实现：两枚同态句柄各走一面，第一个值必须相等。
  local via_method = procgen.rng(0, nil, { 1, 2, 3, 4 })
  local via_module = procgen.rng(0, nil, { 1, 2, 3, 4 })
  eq(via_method:int(0, 0xFFFFFFFF), procgen.next32(via_module), "句柄方法 = 模块函数")
end)

-- ── 用例 2：闭区间与「单面不取数」────────────────────────────────────────────

run("pkg.procgen.int-bounds", function()
  local procgen = fresh()
  local handle = procgen.rng("bounds")
  local seen = {}
  local faces = 0
  for _ = 1, 200000 do
    local value = handle:int(3, 7)
    if value < 3 or value > 7 then
      error("int(3, 7) 越界：" .. tostring(value), 0)
    end
    if not seen[value] then
      seen[value] = true
      faces = faces + 1
    end
  end
  eq(faces, 5, "3..7 的五个面都出现过")

  -- span == 1：直接返回 lo，且**不推进状态**（推进会让「插一个常量参数」改后续序列）。
  local single = procgen.rng("single")
  local before = procgen.state(single)
  eq(single:int(9, 9), 9, "单面区间的返回值")
  eq_seq(procgen.state(single), before, "单面区间不消耗随机数")
end)

-- ── 用例 3：整数分布无模偏（拒绝采样的存在理由）─────────────────────────────

run("pkg.procgen.int-uniformity", function()
  local procgen = fresh()
  local handle = procgen.rng("uniform")
  local rounds = 10000
  local faces = { 0, 0, 0, 0, 0, 0 }
  for _ = 1, rounds do
    local value = handle:int(1, 6)
    faces[value] = faces[value] + 1
  end
  local expected = rounds / 6
  for face = 1, 6 do
    local delta = math.abs(faces[face] - expected) / expected
    is_true(delta <= 0.08, "第 " .. face .. " 面偏离 " .. string.format("%.2f", delta * 100) .. "%")
  end
end)

-- ── 用例 4：浮点值域与粒度 ───────────────────────────────────────────────────

run("pkg.procgen.float-range", function()
  local procgen = fresh()
  local handle = procgen.rng("floats")
  local lowest, highest = 2, -1
  for _ = 1, 100000 do
    local value = handle:float()
    if value < 0 or value >= 1 then
      error("float 越界：" .. tostring(value), 0)
    end
    if value < lowest then
      lowest = value
    end
    if value > highest then
      highest = value
    end
  end
  is_true(highest >= 0.999, "max ≥ 0.999，实得 " .. tostring(highest))
  is_true(lowest < 0.001, "min < 0.001，实得 " .. tostring(lowest))
end)

-- ── 用例 5：抽取与置换 ───────────────────────────────────────────────────────

run("pkg.procgen.pick-shuffle", function()
  local procgen = fresh()
  local handle = procgen.rng("shuffle")
  local list = { "a", "b", "c", "d", "e", "f" }
  local members = {}
  for i = 1, #list do
    members[list[i]] = true
  end
  for _ = 1, 50 do
    is_true(members[handle:pick(list)] == true, "pick 返回列表成员")
  end

  local base = { 1, 2, 3, 4, 5, 6, 7, 8 }
  local out = handle:shuffle(base)
  eq(#out, #base, "置换长度")
  local counts = {}
  for i = 1, #out do
    counts[out[i]] = (counts[out[i]] or 0) + 1
  end
  for i = 1, #base do
    eq(counts[base[i]], 1, "元素 " .. base[i] .. " 恰出现一次")
  end
  eq_seq(base, { 1, 2, 3, 4, 5, 6, 7, 8 }, "入参未被原地改")

  local differed = false
  for _ = 1, 30 do
    local perm = handle:shuffle(base)
    for i = 1, #base do
      if perm[i] ~= base[i] then
        differed = true
        break
      end
    end
    if differed then
      break
    end
  end
  is_true(differed, "至少一次顺序与入参不同")

  eq(#handle:shuffle({}), 0, "空表置换返回空表")
end)

-- ── 用例 6：同 seed 同产物（同进程一层）────────────────────────────────────────

run("pkg.procgen.same-seed-same-hash", function()
  local procgen = fresh()
  procgen.install()
  local params = { w = 16, h = 16, cell = 4, octaves = 3 }
  is_true(procgen.define("hill", { algo = "terrain.noise", params = params }), "define hill")

  local first = procgen.run("hill", "seed-1")
  local second = procgen.run("hill", "seed-1")

  eq(first.algo, "terrain.noise", "产物自带 algo 名")
  eq(first.v, 1, "产物自带算法版本")
  eq(#first.cells, 16 * 16, "cells 长度 = w * h（行主序稠密数组）")
  local lowest, highest = 255, 0
  for i = 1, #first.cells do
    local value = first.cells[i]
    if value < 0 or value > 255 then
      error("cells[" .. i .. "] 越出 0..255：" .. tostring(value), 0)
    end
    if value < lowest then
      lowest = value
    end
    if value > highest then
      highest = value
    end
  end
  is_true(highest > lowest, "噪声场不是常量场（min " .. lowest .. " / max " .. highest .. "）")
  eq_seq(first.cells, second.cells, "同 seed 两次产物逐值相等")
  eq(procgen.hash(first), procgen.hash(second), "同 seed 两次 hash 相等")

  -- `stream` 缺省 = 生成器名（契约 §4 的回写结论）：改名就是换流。
  is_true(procgen.define("hill2", { algo = "terrain.noise", params = params }), "define hill2")
  local other = procgen.run("hill2", "seed-1")
  is_true(procgen.hash(other) ~= procgen.hash(first), "缺省流名来自生成器名：改名 ⇒ 换流 ⇒ 换产物")
end)

-- ── 用例 12：版本进入哈希（I6）────────────────────────────────────────────────

run("pkg.procgen.algo-version-in-hash", function()
  local procgen = fresh()
  local v1 = { algo = "terrain.noise", v = 1, cells = { 1, 2, 3 } }
  local same = { v = 1, cells = { 1, 2, 3 }, algo = "terrain.noise" }
  local v2 = { algo = "terrain.noise", v = 2, cells = { 1, 2, 3 } }
  local renamed = { algo = "terrain.cellular", v = 1, cells = { 1, 2, 3 } }

  eq(procgen.hash(v1), procgen.hash(same), "同内容、异键序 ⇒ 同 hash（键序不参与字节）")
  is_true(procgen.hash(v1) ~= procgen.hash(v2), "`v` 从 1 改 2 ⇒ hash 变")
  is_true(procgen.hash(v1) ~= procgen.hash(renamed), "algo 名变 ⇒ hash 变")
  eq(#procgen.hash(v1), 16, "hash 恰好 16 位")
  is_true(procgen.hash(v1):find("^[0-9a-f]+$") ~= nil, "hash 是小写十六进制")
end)

-- ── 用例 13：未注册的 def / algo ──────────────────────────────────────────────

run("pkg.procgen.unknown-algo", function()
  local procgen = fresh()
  expect_error("YGG-PKG-PROCGEN-DEF", function()
    procgen.run("nope", "seed")
  end, "未定义的 def")

  -- define 只登记形状，不校验 algo 是否存在（算法可以后注册）。
  is_true(procgen.define("x", { algo = "nope" }), "define 未注册的 algo 名")
  is_true(procgen.define("x", { algo = "nope", params = {} }), "同 spec 幂等（params 缺省 = 空表）")
  expect_error("YGG-PKG-PROCGEN-DEF", function()
    procgen.define("x", { algo = "other" })
  end, "异 spec 重定义")
  expect_error("YGG-PKG-PROCGEN-ALGO", function()
    procgen.run("x", "seed")
  end, "def 在、algo 不在")
end)

-- ── 用例 14：参数与种子错误码 ────────────────────────────────────────────────

run("pkg.procgen.params-error", function()
  local procgen = fresh()
  procgen.install()
  local handle = procgen.rng("params")
  expect_error("YGG-PKG-PROCGEN-RANGE", function()
    handle:int(5, 1)
  end, "lo > hi")
  expect_error("YGG-PKG-PROCGEN-RANGE", function()
    handle:int(0, 0x100000000)
  end, "区间宽超 2^32")
  expect_error("YGG-PKG-PROCGEN-PARAM", function()
    handle:int(1.5, 3)
  end, "非整值边界")
  expect_error("YGG-PKG-PROCGEN-PARAM", function()
    handle:pick({})
  end, "pick 空表")
  expect_error("YGG-PKG-PROCGEN-PARAM", function()
    procgen.define("", { algo = "a" })
  end, "空 def 名")
  expect_error("YGG-PKG-PROCGEN-PARAM", function()
    procgen.define("y", { algo = "a", params = 3 })
  end, "params 不是表")
  expect_error("YGG-PKG-PROCGEN-SEED", function()
    procgen.rng({})
  end, "种子类型不合法")
  expect_error("YGG-PKG-PROCGEN-PARAM", function()
    procgen.rng("s", "")
  end, "空流名")

  -- 算法侧参数校验（也走同一个 `-PARAM`）：取器不接受未校验的外部数字。
  is_true(procgen.define("bad", { algo = "terrain.noise", params = { w = 0, h = 4, cell = 2, octaves = 1 } }), "define 不校验 params 语义")
  expect_error("YGG-PKG-PROCGEN-PARAM", function()
    procgen.run("bad", "seed")
  end, "w = 0")
  is_true(procgen.define("bad2", { algo = "terrain.cellular", params = { w = 4, h = 4, fill = 101 } }), "define 不校验 params 语义")
  expect_error("YGG-PKG-PROCGEN-PARAM", function()
    procgen.run("bad2", "seed")
  end, "fill = 101")
end)

-- ── 用例 17：运行期 define 组合两个算法（地形 → 散布）─────────────────────────

run("pkg.procgen.compose-via-define", function()
  local procgen = fresh()
  procgen.install()

  -- 第一步：洞穴（0/1 稠密数组）。
  is_true(procgen.define("cave", {
    algo = "terrain.cellular",
    params = { w = 24, h = 24, fill = 45, steps = 3 },
  }), "define cave")
  local cave = procgen.run("cave", "compose-seed")
  eq(#cave.cells, 24 * 24, "洞穴 cells 长度")

  -- 第二步：把洞穴当 `blocked` 撒点——组合不新增 API，产物直接当参数。
  is_true(procgen.define("marks", {
    algo = "layout.scatter",
    params = { w = 24, h = 24, count = 12, blocked = cave.cells },
  }), "define marks")
  local first = procgen.run("marks", "compose-seed")
  local second = procgen.run("marks", "compose-seed")
  eq(first.placed, #first.points, "placed 与实际点数一致")
  is_true(first.placed > 0, "空位足够时至少放下一个点")
  eq(procgen.hash(first), procgen.hash(second), "同 seed 两次组合产物 hash 相等")
  for i = 1, #first.points do
    local point = first.points[i]
    eq(cave.cells[(point.y - 1) * 24 + point.x], 0, "点 " .. i .. " 不落在墙上")
  end

  -- 第三步：房间布局（同一用例钉形状，不为一条断言多开用例）。
  is_true(procgen.define("keep", {
    algo = "layout.rooms",
    params = { w = 40, h = 30, count = 6, min_w = 3, min_h = 3, max_w = 8, max_h = 6, padding = 1, tries = 40 },
  }), "define keep")
  local keep = procgen.run("keep", "rooms-seed")
  is_true(#keep.rooms >= 1, "至少放下一间房")
  eq(#keep.corridors, (#keep.rooms - 1) * 2, "相邻房间之间恰好两条 L 形走廊")
  for i = 1, #keep.rooms do
    local room = keep.rooms[i]
    is_true(room.x >= 1 and room.y >= 1, "房间 " .. i .. " 起点在界内")
    is_true(room.x + room.w - 1 <= 40 and room.y + room.h - 1 <= 30, "房间 " .. i .. " 终点在界内")
    is_true(room.w >= 3 and room.w <= 8 and room.h >= 3 and room.h <= 6, "房间 " .. i .. " 尺寸在声明区间")
  end
  for i = 1, #keep.rooms do
    for j = i + 1, #keep.rooms do
      local left = keep.rooms[i]
      local right = keep.rooms[j]
      local apart = left.x + left.w - 1 + 1 < right.x
        or right.x + right.w - 1 + 1 < left.x
        or left.y + left.h - 1 + 1 < right.y
        or right.y + right.h - 1 + 1 < left.y
      is_true(apart, "房间 " .. i .. " 与 " .. j .. " 间隔 ≥ padding")
    end
  end
end)

-- ── 用例 8：异 seed ⇒ 异字节（反例证明）─────────────────────────────────────

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
end)

-- ── 用例 19-23：加权抽样（`rng.weighted`）────────────────────────────────────
--
-- 权重池一律**合成**（`w[i] = 1 + (i*7) % 11`，首末置 0）：不抄任何业务数据
-- （数据红线），但形状与真用点同（n=20、含零权、Σ 非 2 的幂）。

--- 合成权重池：n=20、Σw=109、首末两项零权（零权落在最外侧，最容易被「回落首项」实现吃掉）。
local function weight_pool()
  local pool = {}
  for i = 1, 20 do
    pool[i] = 1 + (i * 7) % 11
  end
  pool[1], pool[20] = 0, 0
  return pool
end

--- 四字状态是否不同（只关心「推进了没有」，不比绝对值）。
local function state_moved(before, after)
  for i = 1, 4 do
    if before[i] ~= after[i] then
      return true
    end
  end
  return false
end

-- ── 用例 19：边界与错误码（五个失败路径全落既有六码）─────────────────────────

run("pkg.procgen.weighted-bounds", function()
  local procgen = fresh()
  local codes = procgen.ERRORS
  local pool = weight_pool()

  -- 合法面：下标在域内，零权项不可达
  local handle = procgen.rng("weighted/bounds")
  for _ = 1, 500 do
    local index = handle:weighted(pool)
    is_true(index >= 1 and index <= #pool, "下标在 1..#pool，实得 " .. tostring(index))
    is_true(pool[index] > 0, "零权项不可达，实得 index=" .. tostring(index))
  end

  -- 单元素池：Σw == 1 ⇒ 继承 `int` 的 span==1 短路，**不推进状态**
  local one = procgen.rng("weighted/single")
  local before = procgen.state(one)
  eq(one:weighted({ 1 }), 1, "单元素权重 1 的下标")
  eq_seq(procgen.state(one), before, "单元素权重 1 不取数")

  local heavy = procgen.rng("weighted/single")
  local heavy_before = procgen.state(heavy)
  eq(heavy:weighted({ 5 }), 1, "单元素权重 5 的下标")
  is_true(state_moved(heavy_before, procgen.state(heavy)), "单元素权重 5 取数（Σw=5 不短路）")

  -- 失败面：形状类 → PARAM，抽取面宽类 → RANGE
  expect_error(codes.PARAM, function()
    return handle:weighted(nil)
  end, "nil 池")
  expect_error(codes.PARAM, function()
    return handle:weighted("abc")
  end, "字符串池")
  expect_error(codes.PARAM, function()
    return handle:weighted({})
  end, "空池")
  expect_error(codes.PARAM, function()
    return handle:weighted({ 1, -2 })
  end, "负权")
  expect_error(codes.PARAM, function()
    return handle:weighted({ 1, 1.5 })
  end, "小数权")
  expect_error(codes.PARAM, function()
    return handle:weighted({ 0, 0, 0 })
  end, "全零")
  expect_error(codes.PARAM, function()
    return handle:weighted({ 0 / 0 })
  end, "NaN 权")
  expect_error(codes.RANGE, function()
    return handle:weighted({ 4294967296, 1 })
  end, "Σw 超 2^32")

  -- 上界本身合法：Σw == 2^32 与 `int` 的 span 上限同界
  eq(procgen.rng("weighted/cap"):weighted({ 4294967296 }), 1, "Σw == 2^32 合法")
end)

-- ── 用例 20：等权是特例（`pick` ≡ `list[weighted(ones)]`）──────────────────────

run("pkg.procgen.weighted-equal-weight", function()
  local procgen = fresh()
  -- 逐值 + 句柄状态两侧：等权池上 weighted 与 int 是同一条序列
  for n = 1, 40 do
    local ones = {}
    for i = 1, n do
      ones[i] = 1
    end
    local via_int = procgen.rng("weighted/equal")
    local via_weighted = procgen.rng("weighted/equal")
    for round = 1, 20 do
      local expected = via_int:int(1, n)
      local actual = via_weighted:weighted(ones)
      eq(actual, expected, "n=" .. n .. " 第 " .. round .. " 次下标")
      eq_seq(procgen.state(via_weighted), procgen.state(via_int), "n=" .. n .. " 第 " .. round .. " 次状态")
    end
  end

  -- pick 是本函数的特例
  local list = { "a", "b", "c", "d", "e" }
  local ones = { 1, 1, 1, 1, 1 }
  for round = 1, 50 do
    local via_pick = procgen.rng("weighted/equal")
    local via_weighted = procgen.rng("weighted/equal")
    local picked = via_pick:pick(list)
    local weighted = list[via_weighted:weighted(ones)]
    eq(weighted, picked, "第 " .. round .. " 次 pick ≡ list[weighted(ones)]")
    eq_seq(procgen.state(via_weighted), procgen.state(via_pick), "第 " .. round .. " 次状态")
  end
end)

-- ── 用例 21：确定性（同 seed 同序列 + 定值锚 + 消费量）───────────────────────

--- 定值锚：`("s7-kat", "weighted")` 在 `weight_pool()` 上的前 16 个下标。
--- 由实现实测取得（不是推算）；任何语义漂移都会让它红。
local WEIGHTED_KAT = { 5, 2, 7, 7, 18, 19, 3, 3, 14, 10, 15, 2, 7, 7, 14, 4 }

run("pkg.procgen.weighted-determinism", function()
  local procgen = fresh()
  local pool = weight_pool()

  local a = procgen.rng("s7-kat", "weighted")
  local b = procgen.rng("s7-kat", "weighted")
  local kat = {}
  for i = 1, 16 do
    kat[i] = a:weighted(pool)
    local mirror = b:weighted(pool)
    eq(mirror, kat[i], "同 seed 两条流第 " .. i .. " 次逐值相等")
  end
  eq_seq(kat, WEIGHTED_KAT, "weighted KAT16")

  -- 不止前 16 个：再 1000 次逐值相等（并比句柄状态）
  for i = 1, 1000 do
    local first = a:weighted(pool)
    local second = b:weighted(pool)
    eq(second, first, "第 " .. (16 + i) .. " 次逐值相等")
  end
  eq_seq(procgen.state(b), procgen.state(a), "1016 次后的句柄状态")

  -- 消费量恒定：Σw > 1 时恰好推进一个 `next32`
  local m1 = procgen.rng("weighted/consume")
  local m2 = procgen.rng("weighted/consume")
  m1:weighted(pool)
  procgen.next32(m2)
  eq_seq(procgen.state(m1), procgen.state(m2), "weighted 只消费一个 next32")
end)

-- ── 用例 22：零权合法但不可达（首 / 中 / 末三种位置）─────────────────────────

run("pkg.procgen.weighted-zero-weight", function()
  local procgen = fresh()
  local names = { "first", "middle", "last" }
  local pools = {
    { 0, 5, 5, 5 },
    { 5, 0, 5, 5 },
    { 5, 5, 5, 0 },
  }
  for k = 1, #pools do
    local pool = pools[k]
    local name = names[k]
    local handle = procgen.rng("weighted/zero/" .. name)
    local seen = {}
    for _ = 1, 20000 do
      local index = handle:weighted(pool)
      seen[index] = (seen[index] or 0) + 1
      is_true(pool[index] > 0, name .. "：零权项不可达（index=" .. index .. "）")
    end
    -- 反向钉：正权项必须都被抽到（否则不是「不可达」而是「丢了一档」）
    for i = 1, #pool do
      if pool[i] > 0 then
        is_true(
          (seen[i] or 0) > 0,
          name .. "：正权项 " .. i .. " 在 20,000 次内至少抽中一次"
        )
      end
    end
  end

  -- 全零池 fail-closed：不回落首项（与参考实现的静默回落有意偏差，见契约 §8）
  expect_error(procgen.ERRORS.PARAM, function()
    return procgen.rng("weighted/zero/all"):weighted({ 0, 0 })
  end, "全零池")
end)

-- ── 用例 23：分布合理性（χ² + 带宽，双断言）──────────────────────────────────

--- dof = #pool - 1 = 19、p = 0.999 的卡方临界值（查表）。
local CHI2_CRITICAL = 43.82

run("pkg.procgen.weighted-distribution", function()
  local procgen = fresh()
  local pool = weight_pool()
  local total = 0
  for i = 1, #pool do
    total = total + pool[i]
  end
  local rounds = 1000000
  local counts = {}
  local handle = procgen.rng("weighted/distribution")
  for _ = 1, rounds do
    local index = handle:weighted(pool)
    counts[index] = (counts[index] or 0) + 1
  end

  local chi2 = 0
  local worst, worst_index = 0, 0
  for i = 1, #pool do
    if pool[i] > 0 then
      local expected = rounds * pool[i] / total
      local got = counts[i] or 0
      local diff = got - expected
      chi2 = chi2 + diff * diff / expected
      local delta = math.abs(diff) / expected
      if delta > worst then
        worst, worst_index = delta, i
      end
    end
  end
  print(string.format("check.procgen.weighted_chi2=%.2f", chi2))
  print(string.format("check.procgen.weighted_worst_dev=%.4f", worst))
  is_true(chi2 < CHI2_CRITICAL, "χ² < " .. CHI2_CRITICAL .. "（dof=19, p=0.999），实得 " .. string.format("%.2f", chi2))
  is_true(worst <= 0.05, "最差桶相对偏差 ≤ 5%，实得 " .. string.format("%.2f%%", worst * 100) .. "（第 " .. worst_index .. " 桶）")
  eq(counts[1], nil, "首项零权桶 10⁶ 次 0 命中")
  eq(counts[#pool], nil, "末项零权桶 10⁶ 次 0 命中")
end)
