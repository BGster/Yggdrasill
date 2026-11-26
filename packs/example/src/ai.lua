-- 行动面：每 tick 恰好一个单位行动——朝最近的敌人挪一格，并置 `moved`。
--
-- 「谁动」是纯规则：eid 升序轮转（一轮 = 单位数），不掷骰子；只有「并列的邻格
-- 选哪个」用 `world.rng("ai.move")` 定序——这是本包唯一的随机出口，同种子同结果。
-- 内核不认识「回合」，它只看到一条 `rules` 阶段、写 `pos/moved` 两列的系统。

local board = require("src.board")
local units = require("src.units")

local M = {}

local field -- 装配期注入的棋盘

--- 本 tick 的行动单位下标（`order` 是 eid 升序，内核的确定序）。
local function actor_index(tick, count)
  return tick % count + 1
end

--- 最近敌人的坐标；同距取 eid 小的。没有敌人返回 nil。
local function nearest_enemy(row, order, me)
  local best_pos, best_dist = nil, nil
  for i = 1, #order do
    local other = row[order[i]]
    if other.team ~= me.team then
      local dist = board.distance(me.pos, other.pos)
      if best_dist == nil or dist < best_dist then
        best_pos, best_dist = other.pos, dist
      end
    end
  end
  return best_pos
end

--- 朝目标挪一格：只走**严格更近**的可通行空格；并列时用 `ai.move` 流定序。
--- 贴身（距离 ≤ 1）不动——退让与侧移都不算「朝目标走」，僵持就站着打。
--- 四面都不更近时返回 nil（本回合原地不动）。
local function step_toward(host, me, taken, goal)
  local cur_dist = board.distance(me.pos, goal)
  if cur_dist <= 1 then
    return nil
  end
  local fx, fy = board.decode(me.pos)
  local best_dist, candidates = nil, {}
  for i = 1, #board.DIRECTIONS do
    local d = board.DIRECTIONS[i]
    local x, y = fx + d.dx, fy + d.dy
    if board.open(field, x, y) then
      local pos = board.encode(x, y)
      if taken[pos] == nil then
        local dist = board.distance(pos, goal)
        if dist < cur_dist then
          if best_dist == nil or dist < best_dist then
            best_dist, candidates = dist, { pos }
          elseif dist == best_dist then
            candidates[#candidates + 1] = pos
          end
        end
      end
    end
  end
  if best_dist == nil then
    return nil
  end
  if #candidates == 1 then
    return candidates[1]
  end
  return candidates[host.rng("ai.move") % #candidates + 1]
end

--- 行动（Rules 次，`after` 保证在 turn.advance 之后）：动一个单位并置 moved。
local function move(_, ctx)
  local row, order = units.snapshot(ctx)
  local count = #order
  if count == 0 then
    return
  end
  local actor = order[actor_index(ctx.tick(), count)]
  local me = row[actor]

  local target = nil
  local goal = nearest_enemy(row, order, me)
  if goal ~= nil then
    local taken = {}
    for i = 1, count do
      taken[row[order[i]].pos] = order[i]
    end
    target = step_toward(ctx, me, taken, goal)
  end

  -- 一次写回：只有 actor 一行可能换格，其余行原值回写（列级写口没有单行口）。
  ctx.each_mut({ units.POS, units.MOVED }, function(id, pos, moved)
    if id ~= actor then
      return pos, moved
    end
    return target or pos, 1
  end)
end

--- 注册 `ai.move`（main.lua 装配一次）。
---
--- 不写 `after = {"turn.advance"}`：内核只允许 after 指向**同阶段**的系统，
--- 而清标记那一步在 `pre`（同段写写冲突逼出来的）；跨阶段顺序由阶段本身保证。
function M.install(board_spec)
  field = board_spec
  world.system{
    name = "ai.move",
    stage = "rules",
    writes = { units.POS, units.MOVED },
    run = move,
  }
end

return M
