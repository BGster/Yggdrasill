-- 回合面：轮次边界与「已行动」标记。
--
-- 谁动（`ai`）、谁挨打（`combat`）、谁离场（`combat`）都是别的文件的事；
-- 本文件只回答一个问题：这一 tick 是不是新一轮的第一 tick。
--
-- 为什么在 `pre` 而不在 `rules`：内核禁止同一阶段内两个系统写同一个组件
-- （seal 时就会报写写冲突），而 `moved` 既是本系统要清的列、也是 `ai.move`
-- 要置的列。放到 `pre` 后三段各写各的列：pre 清标记 → rules 挪一格并置标记
-- → post 结算伤害，「先清后置」的语义与同段的 `after` 完全一致。

local units = require("src.units")

local M = {}

--- 回合推进（Rules 首）：新一轮开局清掉上一轮的「已行动」标记。
--
-- `ctx.tick()` 是 0 基（第 n 次推进看到 tick = n-1），所以「跨过边界」写成
-- `tick > 0 and tick % count == 0`：tick=count 的那次推进正是第二轮的第一 tick。
local function advance(_, ctx)
  local count = #ctx.ids(units.NAMES)
  if count == 0 or ctx.tick() == 0 or ctx.tick() % count ~= 0 then
    return
  end
  units.clear_moved(ctx)
end

--- 注册 `turn.advance`（main.lua 装配一次；重复调用是内核层面的错误）。
---
--- 同一阶段内「谁先谁后」用 `after` 声明；跨阶段（pre → rules）顺序由阶段本身
--- 保证，再写 `after` 只是重复声明同一件事。
function M.install()
  world.system{
    name = "turn.advance",
    stage = "pre",
    writes = { units.MOVED },
    run = advance,
  }
end

return M
