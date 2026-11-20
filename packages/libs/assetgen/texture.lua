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
  local ch = spec.int(params, "ch", 3, 1, 4)
  if ch == 2 then
    fail(E.PARAM, "params.ch 必须是 1 / 3 / 4，得到 2")
  end
  if w * h * ch > MAX_PIXEL_BYTES then
    fail(E.CAP, "w*h*ch 超过单件帽 " .. MAX_PIXEL_BYTES .. " 字节（" .. w .. "×" .. h .. "×" .. ch .. "）")
  end
  return w, h, ch, spec.rgb(params, "color", 0x000000), spec.rgb(params, "color2", 0xFFFFFF),
    spec.int(params, "alpha", 255, 0, 255)
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
  local octaves = spec.int(params, "octaves", 3, 1, 8)
  local persist = spec.int(params, "persist", 500, 0, 1000)

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

return M
