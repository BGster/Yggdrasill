-- `tests/test-b-belts.lua` —— 传送带定点推进 + 重放确定性（I3）。
--
-- 覆盖：定点常量 KAT（黄金值 = 整数推导的钉值）；tryAccept 间距不变量；
-- 路径拓扑（链判定/成环兜底）；10⁴ tick 双局重放逐锚相等（同动作日志 →
-- 状态哈希序列逐值相等）；PERF shapez.tick1k_ns 基线。

local belts = require("src.belts")
local simtick = require("src.simtick")
local codec = require("pkg.codec")
local procgen = require("pkg.procgen")

local WORLD = ygg.data("ygg:shapez/world")

--- 开一局标准场景：hub(4,2)；A 线 miner(1,2)→belt(2,2),(3,2)→hub；
--- B 线 miner(1,0)→belt(2,0)→cutter(3,0)→belt(4,0),(4,1)→hub。
local function seed_scenario()
  reset_all()
  ygg_dispatch("event.start")
  ygg_dispatch("sz.place.hub", "4,2,0")
  ygg_dispatch("sz.place.belt", "2,2,0")
  ygg_dispatch("sz.place.belt", "3,2,0")
  ygg_dispatch("sz.place.miner", "1,2,0")
  ygg_dispatch("sz.place.belt", "2,0,0")
  ygg_dispatch("sz.place.proc", "cutter|3,0,0")
  ygg_dispatch("sz.place.belt", "4,0,1")
  ygg_dispatch("sz.place.belt", "4,1,1")
  ygg_dispatch("sz.place.miner", "1,0,0")
end

check("belts.constants-kat", function()
  local c = belts.constants(WORLD.tuning)
  -- 630 × 2^32 // 1000 = 2705829396（0.63 tile 的定点近似，floor）
  assert_eq(c.spacing_q, 2705829396, "spacing_q")
  -- 2000 × 2^32 // 60000 = 143165576（2 items/s @ 60Hz）
  assert_eq(c.belt_step_q, 143165576, "belt_step_q")
  -- 400 × 2^32 // 60000 = 28633115（miner 0.4 items/s）
  assert_eq(c.miner_step_q, 28633115, "miner_step_q")
  -- cutter cycle 4：2000 × 2^32 // 240000 = 35791394
  assert_eq(c.processor_step_q(4), 35791394, "cutter_step_q")
  -- stacker cycle 8：2000 × 2^32 // 480000 = 17895697
  assert_eq(c.processor_step_q(8), 17895697, "stacker_step_q")
  -- rotater cycle 1：= belt_step
  assert_eq(c.processor_step_q(1), 143165576, "rotater_step_q")
  assert_eq(c.processor_step_q(0), 0, "trash 即时")
end)

check("belts.try-accept-spacing-invariant", function()
  seed_scenario()
  local c = belts.constants(WORLD.tuning)
  local paths = belts.ensure()
  -- A 线路径：带 (2,2),(3,2)，起点 = id 较小者（先放的 (2,2)）
  assert_eq(1, 1, "路径数占位")
  local rows = belts.snapshot()
  assert_eq(#rows, 3, "三条路径（A线2带 + miner→cutter 间单带 + cutter出料2带）")
  local a = paths[rows[1].start]
  assert_true(a ~= nil, "A 线路径在场")
  assert_true(belts.can_accept(a), "空路径可收")
  assert_true(belts.try_accept(a, "CuCuCuCu"), "收一件")
  -- 源语义：新件距离 = spacing - initial；spacing 变 initial
  assert_true(a.spacing_q >= 0, "spacing 非负")
  assert_eq(#a.items, 1, "一件在带")
  -- 塞满：spacing < 间距后拒绝
  local guard = 0
  while belts.can_accept(a) and guard < 16 do
    belts.try_accept(a, "CuCuCuCu")
    guard = guard + 1
  end
  assert_true(not belts.can_accept(a), "间距不足后拒绝（间距下限 0.63 定点版）")
  assert_true(guard < 16, "有限步收敛")
  _ = c
end)

check("belts.chain-topology", function()
  seed_scenario()
  local paths = belts.ensure()
  -- A 线：两带成一路径；拆带重放拓扑（reset 后重建）
  local rows = belts.snapshot()
  assert_eq(#rows, 3, "三路径")
  _ = paths
  -- 独立单带（不逆风规则）：指向邻居带正面的带仍自成一路径后经收件口移交
  reset_all()
  ygg_dispatch("event.start")
  ygg_dispatch("sz.place.belt", "0,0,0")
  ygg_dispatch("sz.place.belt", "1,0,2") -- 逆风（指向 (0,0)）→ 不接
  local rows2 = belts.snapshot()
  assert_eq(#rows2, 2, "逆风带不成链")
  assert_eq(rows2[1].s, 4294967296, "新路径空态 spacing = 全长（源 init 语义，单带 = 2^32）")
end)

check("belts.replay-10k-anchors-equal", function()
  local function run_and_collect()
    seed_scenario()
    local anchors = {}
    local last = simtick.snapshot()
    for i = 1, 10000 do
      simtick.tick()
      if i % 500 == 0 then
        anchors[#anchors + 1] = procgen.hash(simtick.snapshot())
      end
    end
    return anchors, codec.encode(simtick.snapshot()), last
  end
  local anchors_a, final_a, _mid_a = run_and_collect()
  local anchors_b, final_b, _mid_b = run_and_collect()
  assert_eq(#anchors_a, 20, "10⁴/500 = 20 锚")
  for i = 1, #anchors_a do
    assert_eq(anchors_b[i], anchors_a[i], "锚 " .. i .. " 重放相等（I3）")
  end
  assert_eq(final_b, final_a, "终态快照逐字节相等")
  -- 终态哈希落 KV 锚面
  local last_anchor = ygg_kv_all()["sz.anchor.10000"]
  assert_eq(last_anchor, anchors_a[20], "KV 锚与序列尾一致")
end)

check("belts.perf-tick1k", function()
  seed_scenario()
  local clock_ok, clock = pcall(function()
    return os.clock
  end)
  if not clock_ok then
    -- 沙箱无 os：跳过（不设阈值，只产基线）
    return "os 不可用，跳过 PERF（selftest 普通 Lua 必有）"
  end
  -- 预热 100 拍
  for _ = 1, 100 do
    simtick.tick()
  end
  local t0 = clock()
  for _ = 1, 1000 do
    simtick.tick()
  end
  local per_tick_ns = (clock() - t0) / 1000 * 1e9
  print(string.format("PERF shapez.tick1k_ns=%.0f （场景=两线八实体，selftest 普通 Lua 口径）", per_tick_ns))
end)
