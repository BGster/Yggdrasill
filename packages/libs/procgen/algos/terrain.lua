-- `packages/libs/procgen/algos/terrain.lua` —— 地形生成器（整数域，两张产物）。
--
-- 契约：`docs/design/packages/procgen.md` I5（参数化而非分支）/ §2（产物形状由
-- algo 决定）。本模块只产出**内容**：`algo` 与 `v` 由 `registry.run` 统一附着。
--
-- # 为什么只用整数与稠密数组
--
-- 稠密数组的字节序是唯一确定的（`codec.encode` 对数组按 1..n 走），而 `pairs`
-- 序依赖宿主哈希种子。浮点只出现在 `rng.float`（本模块一个都不用）——地形里所有
-- 「比例」都写成定点：`t = (小数坐标 << 8) // cell`，插值 `(a*(256-t) + b*t) >> 8`。
--
-- # 插值不取随机数
--
-- 晶格值由 rng 填，插值只读坐标。这条让「同 (seed, stream, params) 同产物」在
-- 多倍频叠加下依然成立：随机数消耗量只由晶格尺寸决定，与 `octaves` 的解码顺序
-- 无关。

local rng = require("pkg.procgen.rng")

local M = {}

--- 边长上限：`4096 × 4096 = 16M` 个格已远超沙箱 64 MB 内存帽，再大只会先撞帽。
local MAX_SIDE = 4096

--- 倍频上限：权重 `1 << (oct-1)` 在 16 层已到 32768，再叠没有视觉意义。
local MAX_OCTAVES = 16

--- `terrain.noise` v1：值噪声多倍频叠加，产物 `cells`（0..255，行主序）。
---
--- params：`w` / `h` / `cell` 必填，`octaves` 必填。`cell` 是晶格间距（格）。
--- 权重 `1 << (octaves - oct)`，末除 `(1 << octaves) - 1` 取整——权重和恰好是该值，
--- 所以最亮值不会溢出 255。
local function noise(params, handle)
  local w = rng.param_int(params, "w", nil, 1, MAX_SIDE)
  local h = rng.param_int(params, "h", nil, 1, MAX_SIDE)
  local cell = rng.param_int(params, "cell", nil, 1, MAX_SIDE)
  local octaves = rng.param_int(params, "octaves", nil, 1, MAX_OCTAVES)

  local total_weight = 0
  local weights = {}
  for oct = 1, octaves do
    weights[oct] = 1 << (octaves - oct)
    total_weight = total_weight + weights[oct]
  end

  local count = w * h
  local acc = {}
  for i = 1, count do
    acc[i] = 0
  end

  -- 晶格每行的列数：`(x-1)//cell` 最大到 `(w-1)//cell`，还要取它的右邻居。
  local lattice_w = (w - 1) // cell + 2
  local lattice_h = (h - 1) // cell + 2

  for oct = 1, octaves do
    local lattice = {}
    for i = 1, lattice_w * lattice_h do
      lattice[i] = handle:int(0, 255)
    end
    local weight = weights[oct]
    for y = 1, h do
      local ty = (((y - 1) % cell) << 8) // cell
      local lattice_row = ((y - 1) // cell) * lattice_w
      for x = 1, w do
        local tx = (((x - 1) % cell) << 8) // cell
        local col = (x - 1) // cell
        local top_left = lattice[lattice_row + col + 1]
        local top_right = lattice[lattice_row + col + 2]
        local bottom_left = lattice[lattice_row + lattice_w + col + 1]
        local bottom_right = lattice[lattice_row + lattice_w + col + 2]
        local top = (top_left * (256 - tx) + top_right * tx) >> 8
        local bottom = (bottom_left * (256 - tx) + bottom_right * tx) >> 8
        local index = (y - 1) * w + x
        acc[index] = acc[index] + (((top * (256 - ty) + bottom * ty) >> 8) * weight)
      end
    end
  end

  local cells = {}
  for i = 1, count do
    cells[i] = acc[i] // total_weight
  end
  return { cells = cells }
end

--- `terrain.cellular` v1：元胞自动机，产物 `cells`（0/1，行主序）。
---
--- params：`w` / `h` 必填；`fill` 初始活率（百分比，0..100）；`steps` 迭代步数；
--- `birth` / `death` 是**邻域计数阈值**（0..8）：死格邻域 ≥ birth 则出生，活格
--- 邻域 < death 则死亡。越界算墙（= 1），所以边缘天然偏向「活」——这是刻意的：
--- 地图边缘不该是悬崖。
---
--- 每步都写新数组，**不原地改**：原地改会让同一行的左邻居已经更新过、右邻居没有，
--- 于是产物取决于遍历方向（顺序依赖就是不确定性）。
local function cellular(params, handle)
  local w = rng.param_int(params, "w", nil, 1, MAX_SIDE)
  local h = rng.param_int(params, "h", nil, 1, MAX_SIDE)
  local fill = rng.param_int(params, "fill", 45, 0, 100)
  local steps = rng.param_int(params, "steps", 4, 0, 64)
  local birth = rng.param_int(params, "birth", 5, 0, 8)
  local death = rng.param_int(params, "death", 4, 0, 8)

  local count = w * h
  local current = {}
  for i = 1, count do
    if handle:int(1, 100) <= fill then
      current[i] = 1
    else
      current[i] = 0
    end
  end

  for _ = 1, steps do
    local next_cells = {}
    for y = 1, h do
      for x = 1, w do
        local neighbours = 0
        for dy = -1, 1 do
          for dx = -1, 1 do
            if dx ~= 0 or dy ~= 0 then
              local nx = x + dx
              local ny = y + dy
              if nx < 1 or nx > w or ny < 1 or ny > h then
                neighbours = neighbours + 1
              else
                neighbours = neighbours + current[(ny - 1) * w + nx]
              end
            end
          end
        end
        local alive = current[(y - 1) * w + x]
        local value = 0
        if alive == 1 then
          if neighbours >= death then
            value = 1
          end
        elseif neighbours >= birth then
          value = 1
        end
        next_cells[(y - 1) * w + x] = value
      end
    end
    current = next_cells
  end

  return { cells = current }
end

--- 算法名 → 实现（`registry.register_builtins` 按这张表注册）。
M.ALGOS = {
  ["terrain.cellular"] = cellular,
  ["terrain.noise"] = noise,
}

return M
