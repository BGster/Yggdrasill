-- 单位面：组件声明、生成、读写。规则不在这里——这里只有「组件叫什么、怎么写」。
--
-- 系统内外的分界：系统体只能摸 `ctx`（world.* 在系统内是 WORLD-BUSY），
-- 所以凡是要在两处共用的写操作都显式收一个 `host`（world 或 ctx）。

local M = {}

M.POS = "pos"
M.HP = "hp"
M.TEAM = "team"
M.MOVED = "moved"

M.NAMES = { M.POS, M.HP, M.TEAM, M.MOVED }
M.DEFAULT_HP = 3

--- 声明四个组件（声明顺序 = 组件 id，装载期固定，不允许重排）。
function M.declare()
  world.component{ name = M.POS, kind = "u64" }
  world.component{ name = M.HP, kind = "u8" }
  world.component{ name = M.TEAM, kind = "u8" }
  world.component{ name = M.MOVED, kind = "u8" }
end

--- 生成一个单位（四个组件一次写全：半拉单位比没有单位更难查）。
function M.spawn(pos, hp, team)
  if hp < 1 or hp > 255 then
    error("hp 超出 u8 范围：" .. tostring(hp))
  end
  if team < 0 or team > 255 then
    error("team 超出 u8 范围：" .. tostring(team))
  end
  return world.spawn{ pos = pos, hp = hp, team = team, moved = 0 }
end

--- 全部单位 id（eid 升序 = 内核的确定序）。
function M.ids(host)
  return (host or world).ids(M.NAMES)
end

--- 占位表 `pos → eid`：每格最多一个单位。
function M.occupancy(host)
  host = host or world
  local taken = {}
  local ids = host.ids(M.NAMES)
  for i = 1, #ids do
    taken[host.get(ids[i], M.POS)] = ids[i]
  end
  return taken
end

--- 一次扫表拿到 id 序与三列值（`order` 是 eid 升序 = 内核的确定序）。
--
-- 系统内没有单实体读口，所以「读一眼完整棋面」只能整表扫；返回值是快照，
-- 本 tick 的写入决策全部基于它（同一个 tick 内不会被自己刚写的值干扰）。
function M.snapshot(host)
  local row, order = {}, {}
  (host or world).each({ M.POS, M.HP, M.TEAM }, function(id, pos, hp, team)
    row[id] = { pos = pos, hp = hp, team = team }
    order[#order + 1] = id
  end)
  return row, order
end

--- 落一格（`moved` 同时置 1：动过就是动过，即使没换格）。
function M.move(host, id, pos)
  return host.set(id, { pos = pos, moved = 1 })
end

--- 伤害落地值：hp 下限 0（u8 也写不进负数）。系统内只用这个纯函数算值，
--- 因为列级写口按「先读整列、再全量回写」走，逐实体 `set` 在那里不可用。
function M.damage_to(hp, amount)
  local next_hp = hp - amount
  if next_hp < 0 then
    return 0
  end
  return next_hp
end

--- 伤害落地：hp 下限 0（u8 也写不进负数），返回值是实际写入的组件数。
function M.damage(host, id, amount)
  local hp = host.get(id, M.HP)
  if hp == nil then
    error("伤害目标没有 hp 组件：" .. tostring(id))
  end
  return host.set(id, { hp = M.damage_to(hp, amount) })
end

--- 把本轮「已行动」标记全清（`turn.advance` 与 `end_turn` 共用同一个写口）。
function M.clear_moved(host)
  (host or world).each_mut({ M.MOVED }, function(_, _)
    return 0
  end)
end

return M
