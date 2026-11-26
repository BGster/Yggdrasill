-- 结算面：相邻的敌对各扣 1 点 hp；hp 归零者离场。
--
-- 「相邻」是棋盘的事（`board.adjacent`），「谁死」是这条规则的事，内核只看到
-- 一条 Post 阶段、只写 `hp` 列的系统（离场走 `ctx.despawn`，tick 边界落地）。

local board = require("src.board")
local units = require("src.units")

local M = {}

--- 一次攻击的伤害（命令与系统共用同一个数，不各写一份）。
M.ATTACK_DAMAGE = 1

--- 结算（Post）：先统计每单位本 tick 挨了几下，再一次性写回 hp。
--
-- 无人相邻时**一格都不写**：`each_mut` 是列级写口，回写「原值」也会被记成
-- 写过（内核的写集记录按写入动作，不按值变没变），delta 会因此虚胖。
local function resolve(_, ctx)
  local row, order = units.snapshot(ctx)
  local hits, any = {}, false
  for i = 1, #order do
    local a = row[order[i]]
    for j = i + 1, #order do
      local b = row[order[j]]
      if a.team ~= b.team and board.adjacent(a.pos, b.pos) then
        hits[order[i]] = (hits[order[i]] or 0) + 1
        hits[order[j]] = (hits[order[j]] or 0) + 1
        any = true
      end
    end
  end
  if not any then
    return
  end
  ctx.each_mut({ units.HP }, function(id, hp)
    return units.damage_to(hp, hits[id] or 0)
  end)
  for i = 1, #order do
    local id = order[i]
    if row[id].hp - (hits[id] or 0) <= 0 then
      ctx.despawn(id)
    end
  end
end

--- 注册 `combat.resolve`（main.lua 装配一次）。
function M.install()
  world.system{
    name = "combat.resolve",
    stage = "post",
    writes = { units.HP },
    run = resolve,
  }
end

return M
