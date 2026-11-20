-- `packages/libs/assetgen/texture.lua` —— 纹理产物：标量场 → 像素行 → 连续字节。
--
-- 契约：`docs/design/packages/assetgen.md` §2（a）/ §3 I8。
--
-- # 一个形状，七个算法
--
-- 每个算法只产出一件事：`field(x, y) → 0..255` 的标量场。像素化是**共用**的：
-- 场值经 `color`/`color2` 调色（`ch = 1` 时直接用场值），再按行拼成字符串。
-- 于是「加一个图案」= 加一个十行函数，而不是加一条像素管线。
--
-- # 为什么按行拼
--
-- 纹理的工作集必须是「成品串 + 每行一个子串」，不留 `w*h*ch` 个整数的数组：
-- 4096×4096×4 的整数数组在 Lua 里是 500 MB 量级，而行块拼接是 O(1) 倍于成品。
-- 每行内部仍是一次 `join.concat`（O(n·log n)），不是逐字节 `..`。

local join = require("pkg.assetgen.join")
local mathx = require("pkg.assetgen.mathx")
local color = require("pkg.assetgen.color")
local spec = require("pkg.assetgen.spec")
local bytes = require("pkg.assetgen.bytes")

local M = {}

local fail = spec.fail
local E = spec.E

--- 单件成品帽 = 挂载桥的 `MOUNT_FILE_MAX_BYTES`（4 MiB，契约 §2）。
local MAX_PIXEL_BYTES = 4194304

--- 通用像素参数（七个算法共用）：`w, h, ch, color, color2, alpha`。
local function common(params)
  local w = spec.int(params, "w", 32, 1, 4096)
  local h = spec.int(params, "h", 32, 1, 4096)
  local ch = spec.range(params, "ch", 3, 1, 4)
  if ch == 2 then
    fail(E.PARAM, "params.ch 必须是 1 / 3 / 4，得到 2")
  end
  if w * h * ch > MAX_PIXEL_BYTES then
    fail(E.CAP, "w*h*ch 超过单件帽 " .. MAX_PIXEL_BYTES .. " 字节（" .. w .. "×" .. h .. "×" .. ch .. "）")
  end
  return w, h, ch, spec.rgb(params, "color", 0x000000), spec.rgb(params, "color2", 0xFFFFFF),
    spec.range(params, "alpha", 255, 0, 255)
end

--- 场值 → 一行字节。
local function row_bytes(field, w, ch, base, accent, alpha)
  local parts = {}
  for x = 1, w do
    local v = field[x]
    if ch == 1 then
      parts[x] = bytes.u8(v)
    else
      local r, g, b = color.unpack(color.lerp(base, accent, v * 1000 // 255))
      if ch == 3 then
        parts[x] = bytes.u8(r) .. bytes.u8(g) .. bytes.u8(b)
      else
        parts[x] = bytes.u8(r) .. bytes.u8(g) .. bytes.u8(b) .. bytes.u8(alpha)
      end
    end
  end
  return join.concat(parts)
end

--- 把 `field(x, y)` 铺成纹理产物的成品字段（`bytes` 行主序、无行内边距、无文件头）。
local function pack(w, h, ch, base, accent, alpha, field)
  local rows = {}
  local line = {}
  for y = 0, h - 1 do
    for x = 0, w - 1 do
      line[x + 1] = field(x, y)
    end
    rows[y + 1] = row_bytes(line, w, ch, base, accent, alpha)
  end
  return { w = w, h = h, ch = ch, bytes = join.concat(rows) }
end

-- ── 算法 ────────────────────────────────────────────────────────────────────

--- `texture.fbm`：格点值噪声的 fBm（`octaves` 个倍频，千分比 `persist` 衰减）。
---
--- 每倍频一张 `(w//cell + 2) × (h//cell + 2)` 的格点表，取数只发生在建表时——
--- 消耗量**只由参数决定**（同 procgen 的 terrain 纪律：权重调度不改变消耗量）。
--- 两轴都用 smoothstep（Q15）而不是线性插值：线性插值在格点处导数不连续，
--- 64×64 的贴图上能看见网格线。
function M.fbm(params, rng)
  local w, h, ch, base, accent, alpha = common(params)
  local cell = spec.int(params, "cell", 8, 1, 4096)
  local octaves = spec.range(params, "octaves", 3, 1, 8)
  local persist = spec.range(params, "persist", 500, 0, 1000)

  local grids, spans, weights = {}, {}, {}
  local amp = 32768
  local total = 0
  for o = 1, octaves do
    local span = cell >> (o - 1)
    if span < 1 then
      span = 1
    end
    local lx = w // span + 2
    local ly = h // span + 2
    local grid = {}
    for i = 1, lx * ly do
      grid[i] = rng:int(0, 255)
    end
    grids[o] = grid
    spans[o] = { span = span, lx = lx }
    weights[o] = amp
    total = total + amp
    amp = (amp * persist) // 1000
    if amp < 1 then
      amp = 1
    end
  end

  return pack(w, h, ch, base, accent, alpha, function(x, y)
    local acc = 0
    for o = 1, octaves do
      local grid = grids[o]
      local span = spans[o].span
      local lx = spans[o].lx
      local cell_x = x // span
      local cell_y = y // span
      local fx = mathx.smoothstep_q15((x % span) * 32768 // span)
      local fy = mathx.smoothstep_q15((y % span) * 32768 // span)
      local at = cell_y * lx + cell_x + 1
      local top = grid[at] + ((grid[at + 1] - grid[at]) * fx) // 32768
      local bottom = grid[at + lx] + ((grid[at + lx + 1] - grid[at + lx]) * fx) // 32768
      acc = acc + (top + ((bottom - top) * fy) // 32768) * weights[o]
    end
    return acc // total
  end)
end

--- `texture.checker`：`size` 见方的棋盘（0 = 底色，255 = 亮色）。
function M.checker(params, _)
  local w, h, ch, base, accent, alpha = common(params)
  local size = spec.int(params, "size", 8, 1, 4096)
  return pack(w, h, ch, base, accent, alpha, function(x, y)
    if (((x // size) + (y // size)) & 1) == 0 then
      return 255
    end
    return 0
  end)
end

--- 双倍坐标到中心的距离（`isqrt` 前）：用 `2x-(w-1)` 而不是 `x - (w-1)/2`，
--- 全程整数且对称轴落在两条中线之间（偶数边长时不会偏向一侧）。
local function radial_distance(x, y, w, h)
  local dx = 2 * x - (w - 1)
  local dy = 2 * y - (h - 1)
  return mathx.isqrt(dx * dx + dy * dy)
end

--- `texture.stripes`：沿 `axis` 的条纹（`period` 像素一周期，`duty` 千分比占空）。
function M.stripes(params, _)
  local w, h, ch, base, accent, alpha = common(params)
  local period = spec.int(params, "period", 8, 1, 4096)
  local duty = spec.range(params, "duty", 500, 0, 1000)
  local axis = spec.range(params, "axis", 0, 0, 1)
  return pack(w, h, ch, base, accent, alpha, function(x, y)
    local at = x
    if axis == 1 then
      at = y
    end
    if ((at % period) * 1000) // period < duty then
      return 255
    end
    return 0
  end)
end

--- `texture.gradient`：线性（`axis` = 0 x / 1 y / 2 对角）或径向（中心亮、边缘暗）。
function M.gradient(params, _)
  local w, h, ch, base, accent, alpha = common(params)
  local shape = spec.one_of(params, "shape", "linear", { linear = true, radial = true })
  local axis = spec.range(params, "axis", 0, 0, 2)
  if shape == "radial" then
    local span = mathx.isqrt((w - 1) * (w - 1) + (h - 1) * (h - 1))
    return pack(w, h, ch, base, accent, alpha, function(x, y)
      if span == 0 then
        return 255
      end
      return 255 - radial_distance(x, y, w, h) * 255 // span
    end)
  end
  if axis == 2 then
    local span = w + h - 2
    if span <= 0 then
      span = 1
    end
    return pack(w, h, ch, base, accent, alpha, function(x, y)
      return (x + y) * 255 // span
    end)
  end
  if axis == 0 then
    local span = w - 1
    if span <= 0 then
      return pack(w, h, ch, base, accent, alpha, function()
        return 0
      end)
    end
    return pack(w, h, ch, base, accent, alpha, function(x)
      return x * 255 // span
    end)
  end
  local span = h - 1
  if span <= 0 then
    return pack(w, h, ch, base, accent, alpha, function()
      return 0
    end)
  end
  return pack(w, h, ch, base, accent, alpha, function(_, y)
    return y * 255 // span
  end)
end

--- `texture.rings`：以画面中心为圆心的同心环（`period` = 一个环带的双倍直径宽度）。
function M.rings(params, _)
  local w, h, ch, base, accent, alpha = common(params)
  local period = spec.int(params, "period", 8, 1, 4096)
  return pack(w, h, ch, base, accent, alpha, function(x, y)
    if ((radial_distance(x, y, w, h) // period) & 1) == 0 then
      return 255
    end
    return 0
  end)
end

--- `texture.bricks`：错缝砖墙（`bw` × `bh` 的砖，`mortar` 像素灰缝）。
function M.bricks(params, _)
  local w, h, ch, base, accent, alpha = common(params)
  local bw = spec.int(params, "bw", 16, 2, 4096)
  local bh = spec.int(params, "bh", 8, 2, 4096)
  local mortar = spec.range(params, "mortar", 1, 0, 64)
  local half = bw // 2
  return pack(w, h, ch, base, accent, alpha, function(x, y)
    local row = y // bh
    local offset = (row & 1) * half
    if ((x + offset) % bw) < mortar or (y % bh) < mortar then
      return 0
    end
    return 255
  end)
end

--- 站点表：显式给了就用它（可判定用例走这条），否则按 `cells` 从流里抽。
---
--- 等距点取**最小站点索引**（`<` 严格比较 → 先到者胜）：这是 T6 钉的那条。
function M.voronoi(params, rng)
  local w, h, ch, base, accent, alpha = common(params)
  local given = spec.array(params, "sites", nil)
  local count = spec.range(params, "cells", 8, 1, 64)
  local sites = {}
  if given ~= nil then
    count = #given
    for i = 1, count do
      local site = given[i]
      if type(site) ~= "table" then
        fail(E.PARAM, "params.sites[" .. i .. "] 必须是 {x, y}")
      end
      sites[i] = {
        x = spec.int(site, "x", 0, 0, w - 1),
        y = spec.int(site, "y", 0, 0, h - 1),
      }
    end
  else
    for i = 1, count do
      sites[i] = { x = rng:int(0, w - 1), y = rng:int(0, h - 1) }
    end
  end
  local tint = spec.array(params, "values", nil)
  local values = {}
  for i = 1, count do
    if tint ~= nil then
      local v = tint[i]
      if not spec.is_int(v) or v < 0 or v > 255 then
        fail(E.PARAM, "params.values[" .. i .. "] 必须是 0..255 的整数")
      end
      values[i] = v
    else
      values[i] = rng:int(0, 255)
    end
  end
  return pack(w, h, ch, base, accent, alpha, function(x, y)
    local best = 0
    local best_distance = 0
    for i = 1, count do
      local site = sites[i]
      local dx = x - site.x
      local dy = y - site.y
      local distance = dx * dx + dy * dy
      if best == 0 or distance < best_distance then
        best = i
        best_distance = distance
      end
    end
    return values[best]
  end)
end

return M
