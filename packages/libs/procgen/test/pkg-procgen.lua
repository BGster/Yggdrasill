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
