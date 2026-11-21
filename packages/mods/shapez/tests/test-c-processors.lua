-- `tests/test-c-processors.lua` —— 处理器四型单元面（间谍落点 + 直注入）。
--
-- 用例不经整局场景：单独放处理器，`processors.accept_input` 直注入，
-- resolve 用间谍（记录出料落点与物品），断言产物 KAT / 节拍 / 串行出料序。

local processors = require("src.processors")
local simtick = require("src.simtick")

local WORLD = ygg.data("ygg:shapez/world")

--- 裸机一局（只 setup，不放场景）。
local function bare()
  reset_all()
  ygg_dispatch("event.start")
end

--- 间谍落点：记录 (fx, fy, item)，一律收下。
local function spy()
  local log = {}
  return function(fx, fy)
    return function(item)
      log[#log + 1] = { fx = fx, fy = fy, item = item }
      return true
    end
  end, log
end

local function step_of(kind)
  local c = simtick.C
  return c.processor_step_q(WORLD.tuning.processorCycle[kind])
end

check("procs.cutter-kat-and-cycle", function()
  bare()
  local resolve, log = spy()
  local id = processors.place("cutter", 0, 0, 0)
  processors.accept_input(id, "CuCuCuCu")
  -- cutter cycle 4：step=35791394；121 拍越 Q32（120×step=4294967280 差 16）
  for _ = 1, 120 do
    processors.tick(simtick.C, resolve)
  end
  assert_eq(#log, 0, "120 拍未完成（定点下界钉住）")
  processors.tick(simtick.C, resolve)
  -- 完成在 121 拍；出料在空闲拍（完成拍本身 progress=0 → 下一拍才出?）
  -- 本实现：完成拍 progress 置 0，同拍 try_eject 已跑过（当时 progress>0）→ 下一拍出
  for _ = 1, 3 do
    processors.tick(simtick.C, resolve)
  end
  assert_eq(#log, 2, "两件产出")
  assert_eq(log[1].item, "----CuCu", "out1 = 源序 right（象限2,3）")
  assert_eq(log[2].item, "CuCu----", "out2 = 源序 left")
  assert_eq(log[1].fx, 1, "出料口 = 正面格 x")
end)

check("procs.rotater-kat", function()
  bare()
  local resolve, log = spy()
  local id = processors.place("rotater", 0, 0, 0)
  processors.accept_input(id, "RuRu----")
  for _ = 1, 70 do
    processors.tick(simtick.C, resolve)
  end
  assert_eq(#log, 1, "一件产出")
  assert_eq(log[1].item, "--RuRu--", "CW 旋转 KAT")
end)

check("procs.stacker-two-inputs-kat", function()
  bare()
  local resolve, log = spy()
  local id = processors.place("stacker", 0, 0, 0)
  assert_true(processors.input_free(id), "空槽可入")
  assert_true(processors.accept_input(id, "----CuCu"), "in1")
  assert_true(processors.accept_input(id, "CuCu----"), "in2")
  assert_true(not processors.input_free(id), "双槽满")
  for _ = 1, 245 do
    processors.tick(simtick.C, resolve)
  end
  assert_eq(#log, 1, "一件产出")
  assert_eq(log[1].item, "CuCuCuCu", "互锁堆叠 KAT（gap=1 回单层）")
end)

check("procs.trash-instant", function()
  bare()
  local resolve, log = spy()
  local id = processors.place("trash", 0, 0, 0)
  assert_true(processors.accept_input(id, "RuRuRuRu"), "收")
  processors.tick(simtick.C, resolve)
  assert_eq(#log, 0, "trash 吞料不出料")
  assert_eq(require("pkg.entity").get(id, "in1"), "", "入口已清")
end)

check("procs.miner-rate-and-shape", function()
  bare()
  local resolve, log = spy()
  local id = processors.place_miner(0, 0, 0, { subshape = "C", color = "u" })
  for _ = 1, 150 do
    processors.tick(simtick.C, resolve)
  end
  assert_eq(#log, 0, "150 拍未出（151 拍越线：150×28633115=4294967250 差 46）")
  processors.tick(simtick.C, resolve)
  assert_eq(#log, 1, "一件产出")
  assert_eq(log[1].item, "CuCuCuCu", "圆斑产形")
  _ = id
end)

check("procs.eject-serial-order", function()
  bare()
  local resolve, log = spy()
  local id = processors.place("cutter", 0, 0, 0)
  processors.accept_input(id, "CuCuCuCu")
  for _ = 1, 130 do
    processors.tick(simtick.C, resolve)
  end
  -- 出料串行：先 out1 后 out2，各占一拍
  assert_eq(#log, 2, "两件均已出")
  assert_eq(log[1].item, "----CuCu", "先 right")
  assert_eq(log[2].item, "CuCu----", "后 left")
  assert_true(log[2].fx == log[1].fx and log[2].fy == log[1].fy, "同一前向口")
  _ = step_of
end)

check("procs.snapshot-deterministic", function()
  bare()
  local resolve = spy()
  local a = processors.place("rotater", 3, 1, 0)
  local b = processors.place_miner(1, 1, 0, { subshape = "R", color = "u" })
  processors.tick(simtick.C, resolve)
  local snap = processors.snapshot()
  assert_eq(#snap, 2, "两行")
  assert_eq(snap[1].id, a, "rotater 先放 id 小在前")
  assert_eq(snap[2].id, b, "矿机在后")
  assert_eq(snap[1].kind, "rotater", "kind 在案")
  assert_eq(snap[2].kind, "miner", "矿机行 kind=miner")
end)
