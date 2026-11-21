-- `tests/test-a-gol.lua` —— GameOfLife KAT + 哈希双局 + 红例。
--
-- 黄金值 = Python 独立推导 + 手算双推导一致（N=4 边界恒死）：
--   blinker {(2,1),(2,2),(2,3)} → g1 {(1,2),(2,2),(3,2)} → g2 = g0（往返）；
--   block {(1,1),(2,1),(1,2),(2,2)} 逐代稳定（每活格 3 邻，邻亡格 ≤2 邻）；
--   toad {(2,1),(3,1),(4,1),(1,2),(2,2),(3,2)} → g1 {(1,1),(4,1),(1,2),(4,2),(2,3)}
--   （边界裁剪相，Python 推导与手算逐格一致）。

local gol = require("src.gol")
local replay = require("src.replay")
local inv = require("src.inv")
local entity = require("pkg.entity")

--- 活格键集读（KAT 对照的紧凑形）。
local function alive_keys()
  local out = {}
  local snap = gol.snapshot()
  for i = 1, #snap.cells do
    local cell = snap.cells[i]
    if cell.alive then
      out[#out + 1] = cell.x .. "," .. cell.y
    end
  end
  for i = 1, #out do
    local min = i
    for j = i + 1, #out do
      if out[j] < out[min] then
        min = j
      end
    end
    if min ~= i then
      out[i], out[min] = out[min], out[i]
    end
  end
  return table.concat(out, " ")
end

check("gol.blinker-roundtrip-kat", function()
  replay.reset(gol, { alive = { "2,1", "2,2", "2,3" } })
  assert_eq(alive_keys(), "2,1 2,2 2,3", "g0 竖三连")
  replay.tick(gol, "step")
  assert_eq(alive_keys(), "1,2 2,2 3,2", "g1 横三连")
  replay.tick(gol, "step")
  assert_eq(alive_keys(), "2,1 2,2 2,3", "g2 = g0 往返")
  assert_eq(#inv.check("gol"), 0, "全程 TypeOK 绿")
  return nil
end)

check("gol.block-stable-kat", function()
  replay.reset(gol, { alive = { "1,1", "2,1", "1,2", "2,2" } })
  for _ = 1, 3 do
    replay.tick(gol, "step")
    assert_eq(alive_keys(), "1,1 1,2 2,1 2,2", "block 稳定")
  end
  return nil
end)

check("gol.toad-one-step-kat", function()
  replay.reset(gol, { alive = { "2,1", "3,1", "4,1", "1,2", "2,2", "3,2" } })
  replay.tick(gol, "step")
  assert_eq(alive_keys(), "1,1 1,2 2,3 4,1 4,2", "toad g1（边界裁剪相）")
  replay.tick(gol, "step")
  assert_eq(alive_keys(), "1,2 2,2 3,2", "toad g2（收敛相）")
  return nil
end)

check("gol.deterministic-unique-successor", function()
  replay.reset(gol, { alive = { "2,1", "2,2", "2,3" } })
  local a = replay.run_track(gol, { "step", "step" })
  local b = replay.run_track(gol, { "step", "step" })
  assert_true(replay.snap_eq(a.final, b.final), "唯一后继：双局终态逐表等")
  assert_eq(replay.snap_bytes(a.final), replay.snap_bytes(b.final), "唯一后继：encode 字节等")
  return nil
end)

check("gol.stutter-hash-invariant", function()
  replay.reset(gol, { alive = { "2,1", "2,2", "2,3" } })
  local stepped = replay.tick(gol, "step")
  local stut1 = replay.tick(gol, "stutter")
  local stut2 = replay.tick(gol, nil)
  assert_eq(stut1.hash, stepped.hash, "stutter 步哈希不变（[][Next]_grid 的 v'=v）")
  assert_eq(stut2.hash, stepped.hash, "nil 动作 = stutter")
  local stepped2 = replay.tick(gol, "step")
  assert_true(stepped2.hash ~= stepped.hash, "真 step 哈希必变")
  return nil
end)

check("gol.snapshot-canonical-kat", function()
  replay.reset(gol, nil)
  local snap = gol.snapshot()
  assert_eq(snap.n, 4, "N=4")
  assert_eq(#snap.cells, 16, "16 格")
  assert_eq(snap.cells[1].x, 1, "cells[1] = (1,1)（y,x 升序）")
  assert_eq(snap.cells[1].y, 1, "")
  assert_eq(snap.cells[16].x, 4, "cells[16] = (4,4)")
  assert_eq(snap.cells[16].alive, false, "规范初态全灭")
  local bytes = replay.snap_bytes(snap)
  assert_true(bytes:find('"n":4', 1, true) ~= nil, "encode 含 n:4")
  return nil
end)

check("gol.dual-run-anchor-reconciliation", function()
  local actions = { "step", "stutter", "step", "step", "stutter", "step", "step", "step" }
  local golden = { hashes = {} }
  local run = replay.run_track(gol, actions)
  for i = 1, #run.hashes do
    golden.hashes[i] = run.hashes[i]
  end
  golden.final = run.final
  local violations = replay.verify_track(gol, actions, golden)
  assert_eq(#violations, 0, "双局逐锚+黄金对照全绿")
  return nil
end)

check("gol.trace-freeze-reconciliation", function()
  local trace = ygg.data("ygg:tla/trace/gol")
  assert_eq(trace.model, "gol", "trace 模型名")
  assert_eq(#trace.actions, #trace.hashes, "动作向量与哈希等长")
  local violations = replay.verify_track(gol, trace.actions, trace)
  assert_eq(#violations, 0, "黄金轨道（blinker×4）逐拍对账绿")
  return nil
end)

check("gol.red-duplicate-and-count", function()
  replay.reset(gol, nil)
  -- 非法态①双值：两个被追踪实体同格（追踪面内构造，不依赖底座直写的可见性）。
  local ids = entity.ids(nil, "tla.cell")
  assert_eq(#ids, 16, "重置后 16 格")
  entity.set(ids[2], "x", entity.get(ids[1], "x"))
  entity.set(ids[2], "y", entity.get(ids[1], "y"))
  local violations = inv.check("gol")
  assert_true(#violations > 0, "重复坐标必报非空")
  assert_true(table.concat(violations):find("坐标重复", 1, true) ~= nil, "违例点名坐标重复")
  -- 非法态②格数：schema 合法地多生第 17 格，TypeOK 的格数检查必报。
  entity.spawn("tla.cell", { x = 3, y = 3, alive = true })
  violations = inv.check("gol")
  local joined = table.concat(violations)
  assert_true(joined:find("格数", 1, true) ~= nil, "17 格必报格数违例")
  return nil
end)

check("gol.red-out-of-bounds-cell-detected", function()
  replay.reset(gol, nil)
  local ids = entity.ids(nil, "tla.cell")
  entity.set(ids[1], "x", 5)
  local violations = inv.check("gol")
  assert_true(#violations > 0, "越界必报非空")
  assert_true(table.concat(violations):find("x 越界", 1, true) ~= nil, "违例点名 x 越界")
  return nil
end)

check("gol.unknown-action-fails-closed", function()
  replay.reset(gol, nil)
  local ok, err = pcall(function()
    gol.step("explode")
  end)
  assert_true(not ok, "未知动作必报错")
  assert_true(tostring(err):find("YGG-MOD-TLA-ACTION", 1, true) ~= nil, "错误码 -ACTION")
  return nil
end)

check("gol.type-invariant-registry-shape", function()
  local names = inv.names("gol")
  assert_eq(#names, 1, "gol 注册面 = TypeOK 一条")
  assert_eq(names[1], "TypeOK", "注册名")
  return nil
end)
