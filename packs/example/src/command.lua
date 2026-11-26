-- 命令面：宿主 `submit` 帧的三条入口（系统外执行，所以可以逐实体读写）。
--
-- 为什么是「导出函数」而不是系统：命令的时序由人决定（谁在什么时候点格子），
-- 不能挂在 tick 上；而它又必须与系统共用同一套规则常量与棋盘边界，
-- 所以住在同一个包里、通过 `main.lua` 交给宿主（`world.commands`）。
--
-- 拒绝一律返回 `(false, 原因)` 且**不改状态**：宿主拿到的是可读原因，
-- 用例也靠「拒绝后快照不变」来钉这条纪律。

local board = require("src.board")
local combat = require("src.combat")
local units = require("src.units")

local M = {}

local field -- 装配期注入的棋盘

--- `{ id, dx, dy }` 挪一格。不存在 / 越界 / 撞墙 / 撞人 = 拒绝且不改状态。
local function move(args)
  local id = args.id
  local pos = world.get(id, units.POS)
  if pos == nil then
    return false, "move：单位不存在 " .. tostring(id)
  end
  local x, y = board.decode(pos)
  local nx, ny = x + args.dx, y + args.dy
  if not board.open(field, nx, ny) then
    return false, "move：目标格越界或是墙"
  end
  local target = board.encode(nx, ny)
  local blocker = units.occupancy(world)[target]
  if blocker ~= nil and blocker ~= id then
    return false, "move：目标格已被单位 " .. tostring(blocker) .. " 占用"
  end
  units.move(world, id, target)
  return true
end

--- `{ attacker, target }` 攻击。缺单位 / 同队 / 不相邻 = 拒绝。
local function attack(args)
  local attacker, target = args.attacker, args.target
  local apos = world.get(attacker, units.POS)
  local tpos = world.get(target, units.POS)
  if apos == nil or tpos == nil then
    return false, "attack：攻击方或目标不存在"
  end
  if world.get(attacker, units.TEAM) == world.get(target, units.TEAM) then
    return false, "attack：不能攻击同队单位"
  end
  if not board.adjacent(apos, tpos) then
    return false, "attack：目标不相邻"
  end
  units.damage(world, target, combat.ATTACK_DAMAGE)
  return true
end

--- 无参数：清掉本轮「已行动」标记（提前结束回合）。
local function end_turn(_)
  units.clear_moved(world)
  return true
end

--- 取命令表（`main.lua` 装配一次，交给宿主挂到派发面上）。
function M.install(board_spec)
  field = board_spec
  return { move = move, attack = attack, end_turn = end_turn }
end

return M
