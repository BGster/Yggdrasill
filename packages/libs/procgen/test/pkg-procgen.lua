-- `pkg.procgen.*` 自测。契约 `openwiki/contracts/packages/procgen.md` §7。
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

--- 加权用例的共享夹具：合成权重池，`w[i] = 1 + (i*7) % 11`，首末置 0。
---
--- 不抄任何业务数据（数据红线），但形状与真用点同：n=20、Σw=109、含零权
--- （零权落在最外侧，最容易被「回落首项」的实现吃掉）、Σ 不是 2 的幂。
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

--- xoshiro128** 的 KAT（参考 C 实现，state = {1,2,3,4} 的前 8 个 next）。
local KAT = { 11520, 0, 5927040, 70819200, 2031721883, 1637235492, 1287239034, 3734860849 }

-- ── 用例 1：PRNG 对齐参考实现 ────────────────────────────────────────────────


-- === S-25 批十三测试件连拆装配：用例段搬入兄弟件（test/procgen/<theme>.lua），执行次序 = 装配次序 ===
local C = {
  S = S,
  run = run,
  eq = eq,
  eq_seq = eq_seq,
  is_true = is_true,
  expect_error = expect_error,
  fresh = fresh,
  weight_pool = weight_pool,
  state_moved = state_moved,
  KAT = KAT
}

dofile(PKG_ROOT .. "/procgen/test/pkg-procgen/rng-hash.lua")(C)
dofile(PKG_ROOT .. "/procgen/test/pkg-procgen/stream-pure-leak-perf.lua")(C)
dofile(PKG_ROOT .. "/procgen/test/pkg-procgen/weighted.lua")(C)
