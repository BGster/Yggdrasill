-- 棋盘面：坐标编解码、边界、相邻、通行判定。纯函数，不碰 world / ygg。
--
-- 坐标是 u64 `x*CELL_SIZE + y`：内核只看见一个整数列，看不见「格子」。
-- 满宽 1000 是上限而非假设——棋盘宽高由 data/board.json 决定。

local M = {}

M.CELL_SIZE = 1000
M.OPEN = 0
M.WALL = 1

--- 四邻偏移（确定序：东、南、西、北）。规则层只按这个顺序做无 rng 的兜底选择。
M.DIRECTIONS = {
  { dx = 1, dy = 0 },
  { dx = 0, dy = 1 },
  { dx = -1, dy = 0 },
  { dx = 0, dy = -1 },
}

--- (x, y) → u64 坐标。
function M.encode(x, y)
  return x * M.CELL_SIZE + y
end

--- u64 坐标 → (x, y)。
function M.decode(pos)
  local y = pos % M.CELL_SIZE
  return (pos - y) // M.CELL_SIZE, y
end

--- 格子是否落在棋盘内。
function M.in_bounds(board, x, y)
  return x >= 0 and y >= 0 and x < board.width and y < board.height
end

--- 格子是否可通行（越界 = 不可通行，墙 = 不可通行）。
function M.open(board, x, y)
  if not M.in_bounds(board, x, y) then
    return false
  end
  return board.cells[y * board.width + x + 1] == M.OPEN
end

--- 两点是否正交相邻（距离恰为 1）。
function M.adjacent(a, b)
  local ax, ay = M.decode(a)
  local bx, by = M.decode(b)
  return math.abs(ax - bx) + math.abs(ay - by) == 1
end

--- 曼哈顿距离（格子数）。
function M.distance(a, b)
  local ax, ay = M.decode(a)
  local bx, by = M.decode(b)
  return math.abs(ax - bx) + math.abs(ay - by)
end

--- 载入棋盘声明（`ygg:example/board` 的 Lua 形态）；形状不对即报错。
function M.new(spec)
  if type(spec) ~= "table" then
    error("棋盘声明必须是表，得到 " .. type(spec))
  end
  local width, height, cells = spec.width, spec.height, spec.cells
  if type(width) ~= "number" or width < 1 then
    error("board.width 必须是正整数")
  end
  if type(height) ~= "number" or height < 1 then
    error("board.height 必须是正整数")
  end
  if type(cells) ~= "table" then
    error("board.cells 必须是数组")
  end
  if #cells ~= width * height then
    error(string.format("board.cells 长度 %d ≠ width*height %d", #cells, width * height))
  end
  return { width = width, height = height, cells = cells }
end

--- 行优先遍历格子（y 外层、x 内层 = 确定序）。
function M.cells(board)
  local out = {}
  for y = 0, board.height - 1 do
    for x = 0, board.width - 1 do
      out[#out + 1] = { x = x, y = y, pos = M.encode(x, y), open = M.open(board, x, y) }
    end
  end
  return out
end

return M
