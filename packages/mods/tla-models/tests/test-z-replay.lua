-- `tests/test-z-replay.lua` —— 跨模型双局逐锚 + 篡改红三型 + PERF。
--
-- 红例②轨道篡改在此集中演练：hash1 / final / anchor 三变异面各断言
-- verify_track 必报非空（对照同参不篡改 = 绿）。PERF 三键
-- `tla.{gol,klotski,ewd998}_tick1k_ns`：selftest 普通 Lua 口径（沿 shapez
-- v1 仪表盘法，os.clock pcall 探测）。
--
-- 预算注记（实测改定，方法带）：计划原估单拍 ≤100µs；实测（本门禁 print 行）
-- ewd998 ≈91µs 达标，gol/klotski ≈0.31-0.33ms——gol 16 格×每格一实体、
-- klotski 10 件，每拍下限 ≈30 次 pkg.entity 主机往返（每次 ≈7µs 实测），
-- 实体每格一存的映射契约下 100µs 不可达。阈值改定 ≤1ms/拍（≈3× 观测，
-- 防机器抖动红门），实测数入契约 §性能预算与交付工程量表。

local replay = require("src.replay")
local gol = require("src.gol")
local twophase = require("src.twophase")
local klotski = require("src.klotski")
local ewd998 = require("src.ewd998")

--- 预算阈值：≤1ms/拍（实测改定 ≈3× 观测余量，依据见头注预算注记）。
local PERF_BUDGET_NS = 1000000

--- 高分辨时钟（selftest 普通 Lua 面；缺席 = nil，跳过阈断只落读数）。
local function now_ms()
  local ok, value = pcall(function()
    return os.clock() * 1000
  end)
  if ok then
    return value
  end
  return nil
end

--- 1k 拍均摊每拍纳秒读数（PERF print 行 + 返回值）。全部走 run_auto
--- （确定性 f(seed,tick,state)，同 seed 同轨； stutter 在域内不影响均摊）。
local function perf_tick1k(mod, key)
  replay.run_auto(mod, "perf-warmup", 50) -- 预热（JIT/路径热身）
  local t0 = now_ms()
  local run = replay.run_auto(mod, "perf", 1000)
  local t1 = now_ms()
  local per_tick_ns = -1
  if t0 ~= nil and t1 ~= nil then
    per_tick_ns = math.floor(((t1 - t0) * 1e6) / 1000 + 0.5)
  end
  print("PERF " .. key .. "=" .. per_tick_ns .. "  (ticks=" .. #run.hashes .. ")")
  return per_tick_ns, run
end

check("z.cross-model-dual-run-anchors", function()
  -- 跨模型：四条黄金轨逐一 verify（内部各做双局逐锚+黄金对照+终态绿）。
  for _, name in ipairs(replay.model_names()) do
    local trace = ygg.data("ygg:tla/trace/" .. name)
    local violations = replay.verify_track(replay.model(name), trace.actions, trace)
    assert_eq(#violations, 0, name .. " 双局逐锚对账绿")
  end
  return nil
end)

check("z.red-tamper-hash1", function()
  local trace = ygg.data("ygg:tla/trace/gol")
  local violations = replay.tamper(gol, trace.actions, trace, "hash1")
  assert_true(#violations > 0, "篡改第 1 拍哈希一字节必败")
  assert_true(table.concat(violations):find("hashes[1]", 1, true) ~= nil, "点名第 1 拍")
  -- 对照：不篡改 = 绿（红例必须真红对照）。
  assert_eq(#replay.verify_track(gol, trace.actions, trace), 0, "同参不篡改绿")
  return nil
end)

check("z.red-tamper-final", function()
  local trace = ygg.data("ygg:tla/trace/klotski")
  local violations = replay.tamper(klotski, trace.actions, trace, "final")
  assert_true(#violations > 0, "篡改终态快照必败")
  assert_true(table.concat(violations):find("final", 1, true) ~= nil, "点名 final")
  return nil
end)

check("z.red-tamper-anchor", function()
  local trace = ygg.data("ygg:tla/trace/twophase")
  local violations = replay.tamper(twophase, trace.actions, trace, "anchor")
  assert_true(#violations > 0, "篡改黄金锚必败")
  assert_true(table.concat(violations):find("golden.anchor", 1, true) ~= nil, "点名锚")
  return nil
end)

check("z.final-state-bytes-equality", function()
  -- 双半断言：快照表深相等 + codec.encode 字节相等（ewd998 十动作轨）。
  local a = replay.run_track(ewd998, { "send:0:1", "recv:1", "deact:0", "deact:1", "probe", "pass:1", "probe", "pass:1" })
  local b = replay.run_track(ewd998, { "send:0:1", "recv:1", "deact:0", "deact:1", "probe", "pass:1", "probe", "pass:1" })
  assert_true(replay.snap_eq(a.final, b.final), "终态快照深相等")
  assert_eq(replay.snap_bytes(a.final), replay.snap_bytes(b.final), "终态 encode 字节逐字节相等")
  assert_true(#replay.snap_bytes(a.final) > 0, "字节串非空")
  return nil
end)

check("z.perf-gol-tick1k", function()
  local per_tick_ns = perf_tick1k(gol, "tla.gol_tick1k_ns")
  if per_tick_ns >= 0 then
    assert_true(per_tick_ns <= PERF_BUDGET_NS, "单拍预算 ≤1ms（实得 " .. per_tick_ns .. "ns）")
  end
  return nil
end)

check("z.perf-klotski-tick1k", function()
  local per_tick_ns = perf_tick1k(klotski, "tla.klotski_tick1k_ns")
  if per_tick_ns >= 0 then
    assert_true(per_tick_ns <= PERF_BUDGET_NS, "单拍预算 ≤1ms（实得 " .. per_tick_ns .. "ns）")
  end
  return nil
end)

check("z.perf-ewd998-tick1k", function()
  local per_tick_ns = perf_tick1k(ewd998, "tla.ewd998_tick1k_ns")
  if per_tick_ns >= 0 then
    assert_true(per_tick_ns <= PERF_BUDGET_NS, "单拍预算 ≤1ms（实得 " .. per_tick_ns .. "ns）")
  end
  return nil
end)
