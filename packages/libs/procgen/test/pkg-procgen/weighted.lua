-- weighted：用例段自 pkg-procgen.lua 原样搬移（S-25 批十三测试件连拆）。
return function(C)
  local S, run, eq, eq_seq, is_true, expect_error, fresh, weight_pool, state_moved, KAT =
    C.S, C.run, C.eq, C.eq_seq, C.is_true, C.expect_error, C.fresh, C.weight_pool, C.state_moved, C.KAT
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

end
