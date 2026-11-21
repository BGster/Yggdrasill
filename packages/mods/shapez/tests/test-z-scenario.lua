-- `tests/test-z-scenario.lua` —— 计划验收③：最小玩法场景脚本化用例。
--
-- 场景（v2 地理白盒派生）：Cu 斑（chunk -1,0）circles[1] 圆心 C；A 线
-- miner@C rot1 → belt@(C↓) → hub@(C↓↓) 产 CuCuCuCu 过 1→2；B 线 miner@(C→)
-- rot1 → cutter@(C→↓) rot2 出料正向交割进 hub，产 ----CuCu（2 级目标）与
-- CuCu----（非目标，收下不计数）过 2→3。
-- 断言：过级 tick 界、事件恰一次、计数口径、放置面拒绝路径。

local hub = require("src.hub")
local mapgen = require("src.mapgen")

local function seed_scenario()
  reset_all()
  ygg_dispatch("event.start")
  local c = mapgen.ensure_chunk(-1, 0)[1].circles[1]
  local cx, cy = -16 + c[1], c[2]
  ygg_dispatch("sz.place.hub", cx .. "," .. (cy + 2) .. ",0")
  ygg_dispatch("sz.place.belt", cx .. "," .. (cy + 1) .. ",1")
  ygg_dispatch("sz.place.miner", cx .. "," .. cy .. ",1")
  ygg_dispatch("sz.place.proc", "cutter|" .. (cx + 1) .. "," .. (cy + 1) .. ",2")
  ygg_dispatch("sz.place.miner", (cx + 1) .. "," .. cy .. ",1")
end

local function count_events(name)
  local n = 0
  for _, e in ipairs(ygg_emitted_events()) do
    if e.name == name then
      n = n + 1
    end
  end
  return n
end

--- 跑到 level >= target 或拍数耗尽；返回 { ok, tick }。
local function run_until_level(target, cap)
  for _ = 1, cap do
    ygg_dispatch("sz.tick", "1")
    if (ygg.kv.get("sz.level") or 0) >= target then
      return { ok = true, tick = ygg.kv.get("sz.tick") }
    end
  end
  return { ok = false, tick = ygg.kv.get("sz.tick") }
end

check("scenario.place-rejections", function()
  seed_scenario()
  -- hub 二台拒绝
  assert_error("YGG-MOD-HUB-ONE", function()
    hub.place(9, 9, 0)
  end, "一局一台")
  -- 矿机离斑拒绝：白盒派生一个确定斑外的格（Cu 圆心正上方 6 格起逐格北上，
  -- 首个不中即用——确定性且与斑表同步）
  local c = mapgen.ensure_chunk(-1, 0)[1].circles[1]
  local ox, oy = -16 + c[1], c[2] - 6
  while mapgen.patch_at(ox, oy) ~= nil do
    oy = oy - 1
  end
  local before = #ygg_narrated_lines()
  ygg_dispatch("sz.place.miner", ox .. "," .. oy .. ",0")
  local lines = ygg_narrated_lines()
  assert_eq(#lines, before + 1, "拒绝行恰一条")
  assert_true(lines[#lines]:find("【拒绝】", 1, true) ~= nil, "拒绝前缀")
end)

check("scenario.level1-to-2-via-miner-line", function()
  seed_scenario()
  local r = run_until_level(2, 3000)
  assert_true(r.ok, "3000 拍内过到 2 级（实到 tick=" .. tostring(r.tick) .. "）")
  assert_eq(ygg.kv.get("sz.level"), 2, "2 级")
  assert_eq(count_events("sz.levelup"), 1, "levelup 恰一次")
  assert_eq(count_events("quest.completed"), 1, "一级 quest completed 恰一次")
  assert_eq(count_events("sz.delivered"), 10, "十件目标交付投影")
end)

check("scenario.level2-to-3-via-cutter-line", function()
  seed_scenario()
  local r = run_until_level(3, 2 * 10 ^ 4)
  assert_true(r.ok, "2×10⁴ 拍内过到 3 级（实到 tick=" .. tostring(r.tick) .. "）")
  assert_eq(ygg.kv.get("sz.level"), 3, "3 级")
  assert_eq(ygg.kv.get("sz.got"), 0, "计数已随过级归零")
  assert_eq(count_events("sz.levelup"), 2, "两次过级")
  -- 2 级计数口径：恰 20 件 ----CuCu 计数（CuCu---- 收下不计数）
  local deliveries = 0
  for _, e in ipairs(ygg_emitted_events()) do
    if e.name == "sz.delivered" then
      deliveries = deliveries + 1
    end
  end
  assert_eq(deliveries, 30, "两级合计恰 30 次目标命中投影（10+20）")
end)

check("scenario.state-hash-stable-across-rerun", function()
  local function run_and_hash(cap)
    seed_scenario()
    local r = run_until_level(2, cap)
    assert_true(r.ok, "过级")
    return require("pkg.procgen").hash(require("src.simtick").snapshot())
  end
  local h1 = run_and_hash(3000)
  local h2 = run_and_hash(3000)
  assert_eq(h2, h1, "同动作日志两局终态哈希逐字节相等（I3 场景面）")
end)
