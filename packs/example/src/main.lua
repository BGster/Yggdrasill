-- 装配入口：声明组件 → 读棋盘 → 铺双方 → 注册三个系统 → 封盘 → 交命令。
--
-- 规则一行都不写在这里（全在 src/*.lua）；内核也不知道「回合」「格子」「攻击」，
-- 它只认识组件列与系统调用序。命令表交宿主，按名字派发（本步先交给用例调用）。

local ai = require("src.ai")
local board = require("src.board")
local combat = require("src.combat")
local command = require("src.command")
local turn = require("src.turn")
local units = require("src.units")

local BOARD_ID = "ygg:example/board"

--- 开局：每行最左的空地给 0 队、最右的空地给 1 队（墙行照铺，只要求有地）。
local function deploy(field)
  for y = 0, field.height - 1 do
    for x = 0, field.width - 1 do
      if board.open(field, x, y) then
        units.spawn(board.encode(x, y), units.DEFAULT_HP, 0)
        break
      end
    end
    for x = field.width - 1, 0, -1 do
      if board.open(field, x, y) then
        units.spawn(board.encode(x, y), units.DEFAULT_HP, 1)
        break
      end
    end
  end
end

units.declare()

local field = board.new(ygg.data(BOARD_ID))
deploy(field)
turn.install()
ai.install(field)
combat.install()
world.seal()

-- 命令表挂在 world 上：宿主/自测拿到同一份函数（模块缓存保证是同一实例）。
-- 封盘之后仍可赋值——它不是 `world.*` 调用，只是一张普通表。
world.commands = command.install(field)
world.log("example: %d×%d 棋盘，%d 个单位", field.width, field.height, #units.ids())

return { commands = world.commands }
