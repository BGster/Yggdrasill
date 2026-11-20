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
