-- 参考包的规则用例（普通 Lua + `test/harness.lua` 的世界替身）。
--
-- 覆盖：坐标往返、相邻判定、步距校验、越界/占位拒绝、攻击前提、hp 下限、
-- end_turn 清标记、50 tick 轮转面、贴脸不后退、真实棋盘的铺子形状、
-- 两次独立装载的确定性。

local board = require("src.board")

-- 这些全局由 `test/harness.lua` 注入（selftest 先跑 harness、再跑用例）。
local fresh = _G.fresh
local copy = _G.copy
local board_data = _G.board_data
local check = _G.check
local eq = _G.eq
local eq_list = _G.eq_list

local OPEN = board.OPEN
local WALL = board.WALL

--- n 个格子的空棋盘（宽 n、高 1 时就是一条走廊）。
local function lane(n, walls)
  local cells = {}
  for i = 1, n do
    cells[i] = OPEN
  end
  for _, x in ipairs(walls or {}) do
    cells[x + 1] = WALL
  end
  return cells
end

--- 小世界装配：清缓存 → 声明组件 → 铺给定单位 → 注册系统。
local function setup(cells, width, height, spec)
  local w = fresh()
  local units = require("src.units")
  local turn = require("src.turn")
  local ai = require("src.ai")
  local combat = require("src.combat")
  local command = require("src.command")
  units.declare()
  local field = board.new({ width = width, height = height, cells = cells })
  local ids = {}
  for i = 1, #spec do
    local u = spec[i]
    ids[i] = units.spawn(board.encode(u.x, u.y or 0), u.hp or units.DEFAULT_HP, u.team)
  end
  turn.install()
  ai.install(field)
  combat.install()
  return {
    world = w,
    units = units,
    turn = turn,
    commands = command.install(field),
    field = field,
    ids = ids,
  }
end

--- 全量快照（用于「拒绝后状态不变」这类断言）。
local function snapshot(s)
  local out = {}
  for i = 1, #s.ids do
    local id = s.ids[i]
    out[#out + 1] = string.format(
      "%d/%d/%d/%d/%d",
      id,
      s.world.get(id, s.units.POS),
      s.world.get(id, s.units.HP),
      s.world.get(id, s.units.TEAM),
      s.world.get(id, s.units.MOVED)
    )
  end
  return table.concat(out, "|")
end

--- 世界指纹（id 升序，取 pos/hp/moved）。
local function fingerprint(s)
  local parts = {}
  local ids = s.units.ids()
  for i = 1, #ids do
    parts[#parts + 1] = string.format(
      "%d/%d/%d/%d",
      ids[i],
      s.world.get(ids[i], s.units.POS),
      s.world.get(ids[i], s.units.HP),
      s.world.get(ids[i], s.units.MOVED)
    )
  end
  return table.concat(parts, "|")
end

-- ── 用例 ────────────────────────────────────────────────────────────────

check("coordinate_round_trip", function()
  for x = 0, 9 do
    for y = 0, 4 do
      local pos = board.encode(x, y)
      local dx, dy = board.decode(pos)
      eq(dx, x, string.format("decode(%d) 的 x", pos))
      eq(dy, y, string.format("decode(%d) 的 y", pos))
    end
  end
  eq(board.encode(0, 0), 0, "原点")
  eq(board.encode(9, 4), 9004, "右下角")
  eq(board.encode(3, 7), 3007, "x 段权重")
end)

check("adjacency", function()
  local center = board.encode(4, 2)
  eq(board.adjacent(center, board.encode(5, 2)), true, "东邻")
  eq(board.adjacent(center, board.encode(3, 2)), true, "西邻")
  eq(board.adjacent(center, board.encode(4, 3)), true, "南邻")
  eq(board.adjacent(center, board.encode(4, 1)), true, "北邻")
  eq(board.adjacent(center, board.encode(5, 3)), false, "斜向不相邻")
  eq(board.adjacent(center, center), false, "自身不相邻")
  eq(board.adjacent(board.encode(0, 0), board.encode(1, 1)), false, "对角不相邻")
end)

check("out_of_bounds_rejected", function()
  local s = setup(lane(3), 3, 1, { { x = 0, team = 0 } })
  local before = snapshot(s)
  local ok, why = s.commands.move({ id = s.ids[1], dx = -1, dy = 0 })
  eq(ok, false, "越界必须拒绝")
  eq(type(why), "string", "拒绝要给原因")
  eq(snapshot(s), before, "拒绝后状态不变")
  eq(s.commands.move({ id = s.ids[1], dx = 1, dy = 0 }), true, "界内可走")
  eq(s.world.get(s.ids[1], s.units.POS), board.encode(1, 0), "落点")
end)

check("occupied_cell_rejected", function()
  local s = setup(lane(3), 3, 1, { { x = 0, team = 0 }, { x = 1, team = 1 } })
  local before = snapshot(s)
  local ok = s.commands.move({ id = s.ids[1], dx = 1, dy = 0 })
  eq(ok, false, "撞人必须拒绝")
  eq(snapshot(s), before, "拒绝后状态不变")
  eq(s.commands.move({ id = s.ids[2], dx = 1, dy = 0 }), true, "空格可走")
end)

check("attack_only_when_adjacent", function()
  local s = setup(lane(4), 4, 1, { { x = 0, team = 0 }, { x = 2, team = 1 } })
  local before = snapshot(s)
  local ok = s.commands.attack({ attacker = s.ids[1], target = s.ids[2] })
  eq(ok, false, "隔一格不能攻击")
  eq(snapshot(s), before, "拒绝后状态不变")
  eq(s.commands.move({ id = s.ids[1], dx = 1, dy = 0 }), true, "走近一格")
  eq(s.commands.attack({ attacker = s.ids[1], target = s.ids[2] }), true, "相邻可攻击")
  eq(s.world.get(s.ids[2], s.units.HP), 2, "掉 1 点")
  eq(s.world.get(s.ids[1], s.units.HP), 3, "攻击方不掉血")
  eq(s.commands.attack({ attacker = s.ids[1], target = s.ids[1] }), false, "不能打自己")
end)

check("hp_floor_zero", function()
  local s = setup(lane(2), 2, 1, { { x = 0, team = 0 }, { x = 1, team = 1 } })
  local target = s.ids[2]
  for i = 1, 3 do
    eq(s.commands.attack({ attacker = s.ids[1], target = target }), true, "第 " .. i .. " 击")
  end
  eq(s.world.get(target, s.units.HP), 0, "3 击归零")
  eq(s.commands.attack({ attacker = s.ids[1], target = target }), true, "对 0 血再攻击不报错")
  eq(s.world.get(target, s.units.HP), 0, "hp 不会变负")
end)

check("end_turn_resets_moved", function()
  local s = setup(lane(4), 4, 1, {
    { x = 0, team = 0 },
    { x = 1, team = 0 },
    { x = 2, team = 0 },
  })
  s.world.step(2)
  local moved = 0
  for i = 1, #s.ids do
    moved = moved + s.world.get(s.ids[i], s.units.MOVED)
  end
  eq(moved, 2, "两 tick 后两人已行动")
  eq(s.commands.end_turn({}), true, "end_turn 成功")
  for i = 1, #s.ids do
    eq(s.world.get(s.ids[i], s.units.MOVED), 0, "end_turn 后标记清零")
  end
end)

check("one_actor_per_tick_50", function()
  local s = setup(lane(4), 4, 1, {
    { x = 0, team = 0 },
    { x = 1, team = 0 },
    { x = 2, team = 0 },
    { x = 3, team = 0 },
  })
  local acted = { 0, 0, 0, 0 }
  for tick = 1, 50 do
    s.world.step(1)
    local seen = {}
    for i = 1, #s.ids do
      if s.world.get(s.ids[i], s.units.MOVED) == 1 then
        seen[#seen + 1] = i
      end
    end
    local expected = ((tick - 1) % 4) + 1
    eq(#seen, expected, string.format("第 %d tick 的已行动人数", tick))
    eq(seen[expected], expected, string.format("第 %d tick 的行动者", tick))
    acted[expected] = acted[expected] + 1
  end
  eq_list(acted, { 13, 13, 12, 12 }, "50 tick 内各单位的行动次数")
  for i = 1, #s.ids do
    eq(
      s.world.get(s.ids[i], s.units.POS),
      board.encode(i - 1, 0),
      "没有敌人时位置不动"
    )
  end
end)

check("deploy_shape", function()
  fresh()
  require("src.main") -- v2 入口：装载即装配（声明 → 铺子 → 注册 → seal）
  local units = require("src.units")
  local field = board.new(copy(board_data("ygg:example/board")))
  local ids = units.ids()
  eq(#ids, 10, "每行两个、五行十个")
  local teams = { 0, 0 }
  local per_row = {}
  for i = 1, #ids do
    local x, y = board.decode(world.get(ids[i], units.POS))
    eq(board.open(field, x, y), true, "单位必须站在可通行格")
    local team = world.get(ids[i], units.TEAM)
    teams[team + 1] = teams[team + 1] + 1
    per_row[y] = (per_row[y] or 0) + 1
  end
  eq(teams[1], 5, "0 队人数")
  eq(teams[2], 5, "1 队人数")
  for y = 0, 4 do
    eq(per_row[y], 2, string.format("第 %d 行单位数", y))
  end
  eq(type(world.commands), "table", "命令表已导出")
  eq(type(world.commands.move), "function", "move 可调用")
  eq(pcall(function()
    world.component{ name = "late", kind = "u8" }
  end), false, "seal 后不能再声明组件")
end)

check("move_rejects_multi_step_and_diagonal", function()
  local s = setup(lane(5), 5, 1, { { x = 0, team = 0 } })
  local before = snapshot(s)
  local bad = {
    { args = { id = s.ids[1], dx = 2, dy = 0 }, what = "一次两格" },
    { args = { id = s.ids[1], dx = 1, dy = 1 }, what = "斜着走" },
    { args = { id = s.ids[1], dx = 0, dy = 0 }, what = "原地不动" },
    { args = { id = s.ids[1], dx = 1.0, dy = 0 }, what = "小数步长" },
    { args = { id = s.ids[1], dx = "1", dy = 0 }, what = "dx 不是数" },
    { args = { id = s.ids[1] }, what = "缺 dx/dy" },
    { args = { id = s.ids[1], dx = 1 }, what = "缺 dy" },
  }
  for i = 1, #bad do
    local ok, why = s.commands.move(bad[i].args)
    eq(ok, false, bad[i].what .. " 必须拒绝")
    eq(type(why), "string", bad[i].what .. " 要给原因")
    eq(snapshot(s), before, bad[i].what .. " 拒绝后状态不变")
  end
  -- 合法的一格正交仍然放行（别把好步子也堵死）。
  eq(s.commands.move({ id = s.ids[1], dx = 1, dy = 0 }), true, "一格正交可走")
  eq(s.world.get(s.ids[1], s.units.POS), board.encode(1, 0), "落点")
end)

check("ai_holds_position_when_adjacent_to_enemy", function()
  local s = setup(lane(3), 3, 1, { { x = 0, team = 0 }, { x = 1, team = 1 } })
  local p0, p1 = board.encode(0, 0), board.encode(1, 0)
  s.world.step(1)
  eq(s.world.get(s.ids[1], s.units.POS), p0, "贴脸的一方不后退")
  eq(s.world.get(s.ids[2], s.units.POS), p1, "贴脸的一方不侧移")
  eq(s.world.get(s.ids[1], s.units.HP), 2, "站着挨打")
  eq(s.world.get(s.ids[2], s.units.HP), 2, "站着挨打")
  -- 僵持到结算：hp 归零者离场。
  s.world.step(1)
  eq(s.world.get(s.ids[1], s.units.HP), 1, "第二轮各掉 1 点")
  eq(s.world.get(s.ids[2], s.units.HP), 1, "第二轮各掉 1 点")
  s.world.step(1)
  eq(#s.units.ids(), 0, "打满三轮 hp 归零者离场")
end)

check("determinism_two_loads", function()
  local function run()
    fresh()
    require("src.main")
    local units = require("src.units")
    local w = world
    w.step(20)
    return fingerprint({ world = w, units = units })
  end
  local first = run()
  local second = run()
  eq(first, second, "两次独立装载 + 20 tick 的指纹")
  eq(#first > 0, true, "指纹非空")
end)
