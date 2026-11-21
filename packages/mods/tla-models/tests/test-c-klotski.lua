-- `tests/test-c-klotski.lua` —— Klotski 单步 KAT + Goal 初始绿 + 红例。
--
-- 黄金值 = 手算步表（源 SlidingPuzzles 逐字）：
--   开局空格 {(1,4),(2,4)}；三步轨 u(1,3)→(1,4)、u(2,3)→(2,4)、h2(1,2)→(1,3)
--   （各步 free 过滤手工核验）；被挡示例：三步后 v2(0,2) 右移 m={(1,2),(1,3)}
--   撞 h2 → 源 update 会把它从 free 滤掉，显式动作输入命中被滤候选 = BLOCKED。

local klotski = require("src.klotski")
local replay = require("src.replay")
local inv = require("src.inv")
local entity = require("pkg.entity")

--- 十件布局紧凑读："kind@ox,oy" 升序串（KAT 对照面）。
local function layout()
  local snap = klotski.snapshot()
  local out = {}
  for i = 1, #snap.pieces do
    local p = snap.pieces[i]
    out[#out + 1] = p.kind .. "@" .. p.ox .. "," .. p.oy
  end
  return table.concat(out, " ")
end

local INIT_LAYOUT = "v2@0,0 v2@0,2 u@0,4 sq@1,0 h2@1,2 u@1,3 u@2,3 v2@3,0 v2@3,2 u@3,4"

check("klotski.init-layout-kat", function()
  replay.reset(klotski)
  assert_eq(layout(), INIT_LAYOUT, "源 Init 十件布局")
  assert_eq(#klotski.empty_cells(), 2, "开局恰两空格")
  assert_eq(klotski.empty_cells()[1][1], 1, "空格 x=1")
  assert_eq(klotski.empty_cells()[1][2], 4, "空格 y=4")
  return nil
end)

check("klotski.init-invariants-green", function()
  replay.reset(klotski)
  local violations = inv.check("klotski")
  assert_eq(#violations, 0, "TypeOK+KlotskiGoal 双绿")
  local names = inv.names("klotski")
  assert_eq(#names, 2, "注册面两条")
  assert_eq(names[1], "KlotskiGoal", "注册名一")
  assert_eq(names[2], "TypeOK", "注册名二")
  return nil
end)

check("klotski.goal-green-at-init", function()
  replay.reset(klotski)
  -- KlotskiGoal = 2×2 方块不在底中四格：开局 sq 在 (1,0)，Goal 应绿。
  assert_true(klotski.piece_id_at("sq", 1, 0) ~= nil, "开局 sq 在顶中 (1,0)")
  assert_eq(#inv.check("klotski"), 0, "Goal 初始绿")
  return nil
end)

check("klotski.three-step-kat", function()
  replay.reset(klotski)
  replay.tick(klotski, "e=1,4|d=0,-1")
  assert_true(klotski.piece_id_at("u", 1, 4) ~= nil, "步1: u(1,3)→(1,4)")
  replay.tick(klotski, "e=2,4|d=0,-1")
  assert_true(klotski.piece_id_at("u", 2, 4) ~= nil, "步2: u(2,3)→(2,4)")
  replay.tick(klotski, "e=1,3|d=0,-1")
  assert_true(klotski.piece_id_at("h2", 1, 3) ~= nil, "步3: h2(1,2)→(1,3)")
  assert_eq(#inv.check("klotski"), 0, "三步后仍绿")
  assert_eq(#klotski.empty_cells(), 2, "空格数守恒")
  return nil
end)

check("klotski.piece-identity-preserved", function()
  replay.reset(klotski)
  local before = klotski.piece_id_at("u", 1, 3)
  replay.tick(klotski, "e=1,4|d=0,-1")
  local after = klotski.piece_id_at("u", 1, 4)
  assert_eq(after, before, "件恒等：移动保持实体 id")
  return nil
end)

check("klotski.blocked-move-fails-closed", function()
  replay.reset(klotski)
  replay.tick(klotski, "e=1,4|d=0,-1")
  replay.tick(klotski, "e=2,4|d=0,-1")
  replay.tick(klotski, "e=1,3|d=0,-1")
  -- v2(0,2) 右移：m={(1,2),(1,3)}，(1,3) 已被 h2 占 → 源 free 过滤会滤掉，
  -- 显式动作命中被滤候选 = BLOCKED（不静默跳过）。
  local ok, err = pcall(function()
    klotski.step("e=1,2|d=-1,0")
  end)
  assert_true(not ok, "撞件平移必报错")
  assert_true(tostring(err):find("YGG-MOD-TLA-BLOCKED", 1, true) ~= nil, "错误码 -BLOCKED")
  return nil
end)

check("klotski.guard-red-nonempty-and-direction", function()
  replay.reset(klotski)
  -- e 非空格：GUARD。
  local ok, err = pcall(function()
    klotski.step("e=0,0|d=1,0")
  end)
  assert_true(not ok and tostring(err):find("YGG-MOD-TLA-GUARD", 1, true) ~= nil, "非空格 GUARD")
  -- 方向不使能：从空格 (1,4) 指向另一空格 (2,4)。
  local ok2, err2 = pcall(function()
    klotski.step("e=1,4|d=1,0")
  end)
  assert_true(not ok2 and tostring(err2):find("YGG-MOD-TLA-GUARD", 1, true) ~= nil, "指向空格 GUARD")
  return nil
end)

check("klotski.red-typeok-out-of-bounds", function()
  replay.reset(klotski)
  local ids = entity.ids(nil, "tla.piece")
  entity.set(ids[1], "ox", 9)
  local violations = inv.check("klotski")
  assert_true(#violations > 0, "出界必报非空")
  assert_true(table.concat(violations):find("出界格", 1, true) ~= nil, "违例点名出界格")
  return nil
end)

check("klotski.red-overlap-detected", function()
  replay.reset(klotski)
  -- 非法态：两 unit 叠同格（运行期加强面：件 cells 两两不交）。
  local ids = entity.ids(nil, "tla.piece")
  local moved = 0
  for i = 1, #ids do
    if entity.get(ids[i], "kind") == "u" and moved < 2 then
      entity.set(ids[i], "ox", 1)
      entity.set(ids[i], "oy", 3)
      moved = moved + 1
    end
  end
  local violations = inv.check("klotski")
  assert_true(#violations > 0, "重叠必报非空")
  assert_true(table.concat(violations):find("重叠格", 1, true) ~= nil, "违例点名重叠格")
  return nil
end)

check("klotski.red-choose-not-unique", function()
  replay.reset(klotski)
  -- 红例③ CHOOSE 唯一化：两 unit 叠 (1,3) 后，从空格 (1,4) 收件命中两件。
  local ids = entity.ids(nil, "tla.piece")
  local moved = 0
  for i = 1, #ids do
    if entity.get(ids[i], "kind") == "u" and moved < 2 then
      entity.set(ids[i], "ox", 1)
      entity.set(ids[i], "oy", 3)
      moved = moved + 1
    end
  end
  local ok, err = pcall(function()
    klotski.step("e=1,4|d=0,-1")
  end)
  assert_true(not ok, "命中两件必报错不静默")
  assert_true(tostring(err):find("YGG-MOD-TLA-CHOOSE", 1, true) ~= nil, "错误码 -CHOOSE")
  return nil
end)

check("klotski.dual-run-anchor-reconciliation", function()
  -- 五步：三步 KAT + sq 下移一格 + sq 回移（round-trip，末态 = 三步后态）。
  local actions = { "e=1,4|d=0,-1", "e=2,4|d=0,-1", "e=1,3|d=0,-1", "e=1,2|d=0,-1", "e=1,0|d=0,1" }
  local golden = { hashes = {} }
  local run = replay.run_track(klotski, actions)
  for i = 1, #run.hashes do
    golden.hashes[i] = run.hashes[i]
  end
  golden.final = run.final
  local violations = replay.verify_track(klotski, actions, golden)
  assert_eq(#violations, 0, "五步轨双局逐锚对账绿")
  return nil
end)

check("klotski.trace-freeze-reconciliation", function()
  local trace = ygg.data("ygg:tla/trace/klotski")
  assert_eq(trace.model, "klotski", "trace 模型名")
  local violations = replay.verify_track(klotski, trace.actions, trace)
  assert_eq(#violations, 0, "黄金轨道（开局三步）逐拍对账绿")
  return nil
end)
