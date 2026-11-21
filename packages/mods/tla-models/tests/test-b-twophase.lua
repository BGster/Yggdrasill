-- `tests/test-b-twophase.lua` —— TwoPhase 4 态循环 KAT + stutter 锚 + 红例。
--
-- 黄金值 = 手算步表（源 MCTwoPhase 逐字）：Init (0,0,0) → producer (1,0,0)
-- → consumer (1,1,0) → producer (0,1,0) → consumer (0,0,0)。Alternate 抽象
-- 投影 vBar = (p+c)%2 沿轨 0→1→0→1→0（refinement 映射数值面）。

local twophase = require("src.twophase")
local replay = require("src.replay")
local inv = require("src.inv")

--- (p,c,x) 读。
local function pcx()
  local snap = twophase.snapshot()
  return snap.p .. snap.c .. snap.x
end

--- 终态 (p,c,x) 紧凑读。
local function pc_x_of(snap)
  return snap.p .. snap.c .. snap.x
end

check("twophase.cycle4-kat", function()
  replay.reset(twophase)
  assert_eq(pcx(), "000", "Init 钉 (0,0,0)")
  assert_eq(twophase.vbar(), 0, "vBar 初值 0")
  local expect = { "100", "110", "010", "000" }
  local vbars = { 1, 0, 1, 0 }
  local actions = { "producer", "consumer", "producer", "consumer" }
  for i = 1, 4 do
    replay.tick(twophase, actions[i])
    assert_eq(pcx(), expect[i], "第 " .. i .. " 拍后 (p,c,x)")
    assert_eq(twophase.vbar(), vbars[i], "第 " .. i .. " 拍后 vBar（Alternate 投影）")
    assert_eq(#inv.check("twophase"), 0, "第 " .. i .. " 拍 Inv 绿")
  end
  return nil
end)

check("twophase.x-preserved-by-both-actions", function()
  replay.reset(twophase)
  ygg.kv.set("tla.tp.x", 7) -- x 被抽象参数实例化为恒等（XAct: x'=x）
  replay.tick(twophase, "producer")
  replay.tick(twophase, "consumer")
  assert_eq(ygg.kv.get("tla.tp.x"), 7, "两个动作都不改 x")
  return nil
end)

check("twophase.stutter-anchor-same-hash", function()
  replay.reset(twophase)
  -- 轨道：producer→consumer→stutter→stutter（第 4 拍落锚）。
  replay.reset(twophase)
  local t1 = replay.tick(twophase, "producer")
  local t2 = replay.tick(twophase, "consumer")
  local t3 = replay.tick(twophase, "stutter")
  local t4 = replay.tick(twophase, "stutter")
  assert_eq(t3.hash, t2.hash, "stutter 步哈希不变")
  assert_eq(t4.hash, t2.hash, "第二个 stutter 同态")
  local anchor4 = ygg.kv.get("tla.anchor.twophase.4")
  assert_true(anchor4 ~= nil, "第 4 拍（stutter）落锚")
  assert_eq(anchor4, t2.hash, "锚 = 同态哈希（[][Next]_<<p,c,x>> 的 v'=v 分支）")
  assert_true(t2.hash ~= t1.hash, "真 step 哈希必变")
  return nil
end)

check("twophase.auto-deterministic-nonstutter", function()
  replay.reset(twophase)
  -- enabled 表形：任何状态下真动作 + stutter 恰两条，stutter 恒在尾。
  local enabled = twophase.auto_actions()
  assert_eq(#enabled, 2, "恒恰两动作")
  assert_true(enabled[2] == "stutter", "stutter 恒在尾")
  -- auto 的属性 = 确定性：同 seed 同轨（f(seed,tick,state) 纯函数）。
  local a = replay.run_auto(twophase, "seed-a", 6)
  local b = replay.run_auto(twophase, "seed-a", 6)
  assert_true(replay.snap_eq(a.final, b.final), "同 seed 双局终态逐表等")
  for i = 1, #a.hashes do
    assert_eq(a.hashes[i], b.hashes[i], "同 seed 第 " .. i .. " 拍哈希等")
  end
  assert_eq(pc_x_of(a.final), pc_x_of(b.final), "终态一致")
  return nil
end)

check("twophase.dual-run-reconciliation", function()
  local actions = { "producer", "stutter", "consumer", "producer", nil, "consumer" }
  local golden = { hashes = {} }
  local run = replay.run_track(twophase, actions)
  for i = 1, #run.hashes do
    golden.hashes[i] = run.hashes[i]
  end
  golden.final = run.final
  local violations = replay.verify_track(twophase, actions, golden)
  assert_eq(#violations, 0, "含 nil（stutter）动作的双局逐锚对账绿")
  return nil
end)

check("twophase.trace-freeze-reconciliation", function()
  local trace = ygg.data("ygg:tla/trace/twophase")
  assert_eq(trace.model, "twophase", "trace 模型名")
  local violations = replay.verify_track(twophase, trace.actions, trace)
  assert_eq(#violations, 0, "黄金轨道（4 态循环）逐拍对账绿")
  return nil
end)

check("twophase.red-inv-p2", function()
  replay.reset(twophase)
  ygg.kv.set("tla.tp.p", 2) -- 非法态：p=2 ∉ {0,1}
  local violations = inv.check("twophase")
  assert_true(#violations > 0, "p=2 必报非空")
  assert_true(table.concat(violations):find("p ∉ {0,1}", 1, true) ~= nil, "违例点名 p")
  return nil
end)

check("twophase.red-guard-wrong-phase", function()
  replay.reset(twophase)
  -- producer 守卫 p=c：初态 (0,0) 使能；走一步后 (1,0) p≠c，producer 不使能。
  replay.tick(twophase, "producer")
  local ok, err = pcall(function()
    twophase.step("producer")
  end)
  assert_true(not ok, "p≠c 时 producer 必报错")
  assert_true(tostring(err):find("YGG-MOD-TLA-GUARD", 1, true) ~= nil, "错误码 -GUARD")
  return nil
end)
