-- `tests/test-d-ewd998.lua` —— EWD998 N=2 轨道 Inv 全程绿 + 终态 KAT + 红例。
--
-- 黄金值 = Python 独立推导十动作轨道（手算与脚本双推导，含每步 Inv）：
--   send:0:1 → recv:1 → deact:0 → deact:1 → probe → pass:1 → probe → pass:1
--   → stutter → stutter；terminationDetected 首真 @t8；终态
--   active=(F,F) color=(W,W) counter=(1,-1) pending=(0,0) token=(0,-1,white)。
--   注意两个反直觉语义（都逐字对齐源）：probe 守卫是「前轮不结论」才点火
--   （黑令牌 ∨ 黑 0 号 ∨ 计数和>0）；pass 会把黑节点色传给令牌（t6 令牌
--   变黑，须第二轮探测才能达检测态）。

local ewd998 = require("src.ewd998")
local replay = require("src.replay")
local inv = require("src.inv")
local entity = require("pkg.entity")

--- 终态紧凑读："active|color|counter|pending|token"。
local function state_line()
  local snap = ewd998.snapshot()
  local a, c, k, p = {}, {}, {}, {}
  for i = 1, #snap.nodes do
    local n = snap.nodes[i]
    a[i] = n.active and "T" or "F"
    c[i] = n.color:sub(1, 1):upper()
    k[i] = tostring(n.counter)
    p[i] = tostring(n.pending)
  end
  return table.concat(a, "") .. "|" .. table.concat(c, "") .. "|" .. table.concat(k, ",")
    .. "|" .. table.concat(p, ",") .. "|(" .. snap.token.pos .. "," .. snap.token.q .. ","
    .. snap.token.color .. ")"
end

local TRACK = { "send:0:1", "recv:1", "deact:0", "deact:1", "probe", "pass:1", "probe", "pass:1", "stutter", "stutter" }
local FINAL = "FF|WW|1,-1|0,0|(0,-1,white)"

check("ewd998.init-pinned-kat", function()
  replay.reset(ewd998)
  assert_eq(state_line(), "TT|WW|0,0|0,0|(0,0,black)", "钉规范初态（全active/全white/token黑）")
  assert_eq(#inv.check("ewd998"), 0, "三不变量初始绿")
  local names = inv.names("ewd998")
  assert_eq(#names, 3, "注册面三条")
  assert_eq(names[1] .. "," .. names[2] .. "," .. names[3], "Inv,TerminationDetection,TypeOK", "注册名")
  return nil
end)

check("ewd998.track-inv-green-throughout", function()
  replay.reset(ewd998)
  for t = 1, #TRACK do
    replay.tick(ewd998, TRACK[t])
    local violations = inv.check("ewd998")
    assert_eq(#violations, 0, "t" .. t .. "（" .. TRACK[t] .. "）后 Inv/TypeOK/TD 全绿")
  end
  return nil
end)

check("ewd998.final-state-kat", function()
  replay.reset(ewd998)
  for t = 1, #TRACK do
    replay.tick(ewd998, TRACK[t])
  end
  assert_eq(state_line(), FINAL, "终态逐项 KAT")
  assert_true(ewd998.termination_detected(), "terminationDetected @终态")
  return nil
end)

check("ewd998.td-first-true-at-t8", function()
  replay.reset(ewd998)
  for t = 1, #TRACK do
    replay.tick(ewd998, TRACK[t])
    if t < 8 then
      assert_true(not ewd998.termination_detected(), "t" .. t .. " 未达检测态")
    else
      assert_true(ewd998.termination_detected(), "t" .. t .. " 已达检测态")
    end
  end
  return nil
end)

check("ewd998.stutter-tail-hash-invariant", function()
  replay.reset(ewd998)
  local hashes = {}
  for t = 1, #TRACK do
    hashes[#hashes + 1] = replay.tick(ewd998, TRACK[t]).hash
  end
  assert_eq(hashes[9], hashes[8], "t9 stutter 同态哈希")
  assert_eq(hashes[10], hashes[8], "t10 stutter 同态哈希")
  assert_true(hashes[8] ~= hashes[7], "t8 真动作哈希必异")
  return nil
end)

check("ewd998.probe-guard-inconclusive-semantics", function()
  replay.reset(ewd998)
  -- 反直觉语义对账：黑令牌时 probe 点火（前轮不结论）。
  replay.tick(ewd998, "probe")
  assert_eq(ygg.kv.get("tla.ewd.token.pos"), 1, "probe 后令牌到 N-1")
  assert_eq(ygg.kv.get("tla.ewd.token.color"), "white", "probe 后令牌白")
  return nil
end)

check("ewd998.pass-carries-black", function()
  replay.reset(ewd998)
  -- 反直觉语义对账：recv 染黑节点 1，pass 把黑传给令牌（源 IF 分支）。
  replay.tick(ewd998, "send:0:1")
  replay.tick(ewd998, "recv:1")
  replay.tick(ewd998, "deact:0")
  replay.tick(ewd998, "deact:1")
  replay.tick(ewd998, "probe")
  replay.tick(ewd998, "pass:1")
  assert_eq(ygg.kv.get("tla.ewd.token.color"), "black", "黑色节点把令牌染黑")
  assert_true(not ewd998.termination_detected(), "第一轮探测不达检测态")
  return nil
end)

check("ewd998.dual-run-anchor-reconciliation", function()
  local golden = { hashes = {} }
  local run = replay.run_track(ewd998, TRACK)
  for i = 1, #run.hashes do
    golden.hashes[i] = run.hashes[i]
  end
  golden.final = run.final
  local violations = replay.verify_track(ewd998, TRACK, golden)
  assert_eq(#violations, 0, "十动作轨双局逐锚对账绿（含 8/12 锚点）")
  return nil
end)

check("ewd998.trace-freeze-reconciliation", function()
  local trace = ygg.data("ygg:tla/trace/ewd998")
  assert_eq(trace.model, "ewd998", "trace 模型名")
  assert_eq(#trace.actions, 10, "动作向量长度")
  local violations = replay.verify_track(ewd998, trace.actions, trace)
  assert_eq(#violations, 0, "黄金轨道逐拍对账绿")
  return nil
end)

check("ewd998.red-typeok-pending-negative", function()
  replay.reset(ewd998)
  -- 红例①：pending = -1 ∉ Nat（schema 放行负数，TypeOK 必拦）。
  local ids = entity.ids(nil, "tla.node")
  entity.set(ids[2], "pending", -1)
  local violations = inv.check("ewd998")
  assert_true(#violations > 0, "pending=-1 必报非空")
  local joined = table.concat(violations)
  assert_true(joined:find("pending[1]", 1, true) ~= nil, "TypeOK 点名 pending ∉ Nat")
  assert_true(joined:find("P0", 1, true) ~= nil, "Inv P0（B≠Sum）同报")
  return nil
end)

check("ewd998.red-guards-fail-closed", function()
  replay.reset(ewd998)
  -- recv 守卫：无在途消息。
  local ok, err = pcall(function()
    ewd998.step("recv:1")
  end)
  assert_true(not ok and tostring(err):find("YGG-MOD-TLA-GUARD", 1, true) ~= nil, "recv GUARD")
  -- pass 守卫：令牌不在 1。
  local ok2, err2 = pcall(function()
    ewd998.step("pass:1")
  end)
  assert_true(not ok2 and tostring(err2):find("YGG-MOD-TLA-GUARD", 1, true) ~= nil, "pass GUARD")
  -- 未知动作。
  local ok3, err3 = pcall(function()
    ewd998.step("broadcast")
  end)
  assert_true(not ok3 and tostring(err3):find("YGG-MOD-TLA-ACTION", 1, true) ~= nil, "未知动作 -ACTION")
  return nil
end)

check("ewd998.red-probe-at-detection-state", function()
  replay.reset(ewd998)
  for t = 1, #TRACK do
    replay.tick(ewd998, TRACK[t])
  end
  -- 结论态（全结论条件均假）probe 守卫不满足 → GUARD。
  local ok, err = pcall(function()
    ewd998.step("probe")
  end)
  assert_true(not ok and tostring(err):find("YGG-MOD-TLA-GUARD", 1, true) ~= nil, "结论态 probe GUARD")
  return nil
end)
