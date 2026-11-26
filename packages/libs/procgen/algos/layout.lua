-- `packages/libs/procgen/algos/layout.lua` —— 布局生成器（整数域，两张产物）。
--
-- 契约：`openwiki/contracts/packages/procgen.md` I5（参数化而非分支）/ §2。本模块只产出
-- 内容：`algo` 与 `v` 由 `registry.run` 统一附着。
--
-- # 组合不新增 API
--
-- 「先地形、后散布」由 mod 在运行期 `procgen.define("marks", { algo = "layout.scatter",
-- params = { ..., blocked = <地形 cells> } })` 再 `run` 完成。这正是 `define` 存在
-- 的主要理由：生成器之间不需要一条流水线 DSL，只需要「产物能当参数」。
--
-- # 集合语义只用 `pairs` 做成员判定
--
-- 占位去重靠一张 `used` 映射，它只被查、不被遍历——所以 `pairs` 的序不进产物。
-- 一切会进产物的顺序都由游标或房间序号决定。

local rng = require("pkg.procgen.rng")

local fail = rng.fail
local E = rng.E

local M = {}

local MAX_SIDE = 4096
local MAX_ROOMS = 1024
local MAX_POINTS = 1 << 20

--- 矩形是否与已放置的房间冲突（`padding` = 房间之间要求的最小空行/空列）。
local function crowded(placed, x, y, width, height, padding)
  for i = 1, #placed do
    local other = placed[i]
    if
      x - padding <= other.x + other.w - 1
      and other.x - padding <= x + width - 1
      and y - padding <= other.y + other.h - 1
      and other.y - padding <= y + height - 1
    then
      return true
    end
  end
  return false
end

--- `layout.rooms` v1：随机矩形 + L 形走廊，产物 `rooms` / `corridors`。
---
--- params：`w` / `h` / `count` / `min_w` / `min_h` / `max_w` / `max_h` 必填，
--- `padding`（缺省 1）是房间之间的最小间隔，`tries`（缺省 32）是每间房的尝试上限。
---
--- 每间房最多试 `tries` 次，越界或与已放置房间冲突（含 `padding`）即弃——所以
--- `count` 是**上限**，产物可以更少；放不下就是放不下，不挪位置也不加大网格。
--- 走廊连**相邻**两间（按放置顺序）的中心，先横后竖；顺序由房间顺序决定，不用排序。
local function rooms(params, handle)
  local w = rng.param_int(params, "w", nil, 1, MAX_SIDE)
  local h = rng.param_int(params, "h", nil, 1, MAX_SIDE)
  local count = rng.param_int(params, "count", nil, 0, MAX_ROOMS)
  local min_w = rng.param_int(params, "min_w", nil, 1, MAX_SIDE)
  local min_h = rng.param_int(params, "min_h", nil, 1, MAX_SIDE)
  local max_w = rng.param_int(params, "max_w", nil, min_w, MAX_SIDE)
  local max_h = rng.param_int(params, "max_h", nil, min_h, MAX_SIDE)
  local padding = rng.param_int(params, "padding", 1, 0, MAX_SIDE)
  local tries = rng.param_int(params, "tries", 32, 1, MAX_ROOMS)

  local placed = {}
  for _ = 1, count do
    for _ = 1, tries do
      local width = handle:int(min_w, max_w)
      local height = handle:int(min_h, max_h)
      if width <= w and height <= h then
        local x = handle:int(1, w - width + 1)
        local y = handle:int(1, h - height + 1)
        if not crowded(placed, x, y, width, height, padding) then
          placed[#placed + 1] = { x = x, y = y, w = width, h = height }
          break
        end
      end
    end
  end

  local corridors = {}
  for i = 2, #placed do
    local from = placed[i - 1]
    local to = placed[i]
    local from_x = from.x + (from.w - 1) // 2
    local from_y = from.y + (from.h - 1) // 2
    local to_x = to.x + (to.w - 1) // 2
    local to_y = to.y + (to.h - 1) // 2
    corridors[#corridors + 1] = { x1 = from_x, y1 = from_y, x2 = to_x, y2 = from_y }
    corridors[#corridors + 1] = { x1 = to_x, y1 = from_y, x2 = to_x, y2 = to_y }
  end

  return { corridors = corridors, rooms = placed }
end

--- `layout.scatter` v1：在空格上撒点，产物 `points` / `placed`。
---
--- params：`w` / `h` / `count` 必填；`blocked` 是可选的长度 `w*h` 的 0/1 稠密数组
--- （`nil` = 全空）。抽样上限 `count * 64` 次：稠密图里放不下就收工，不无限重试，
--- 也不改抽样策略——上限本身是**产物的一部分**（改它会改产物，改了就换 hash）。
local function scatter(params, handle)
  local w = rng.param_int(params, "w", nil, 1, MAX_SIDE)
  local h = rng.param_int(params, "h", nil, 1, MAX_SIDE)
  local count = rng.param_int(params, "count", nil, 0, MAX_POINTS)
  local total = w * h

  local blocked = params.blocked
  if blocked ~= nil then
    if type(blocked) ~= "table" or #blocked ~= total then
      fail(E.PARAM, "params.blocked 必须是长度 w*h = " .. total .. " 的稠密数组")
    end
    for i = 1, total do
      if blocked[i] ~= 0 and blocked[i] ~= 1 then
        fail(E.PARAM, "params.blocked[" .. i .. "] 必须是 0 或 1")
      end
    end
  end

  local points = {}
  local used = {}
  local placed = 0
  local attempts = 0
  local limit = count * 64
  while placed < count and attempts < limit do
    attempts = attempts + 1
    local index = handle:int(1, total)
    local free = used[index] == nil and (blocked == nil or blocked[index] == 0)
    if free then
      used[index] = true
      placed = placed + 1
      points[placed] = { x = (index - 1) % w + 1, y = (index - 1) // w + 1 }
    end
  end

  return { placed = placed, points = points }
end

--- 算法名 → 实现（`registry.register_builtins` 按这张表注册）。
M.ALGOS = {
  ["layout.rooms"] = rooms,
  ["layout.scatter"] = scatter,
}

return M
