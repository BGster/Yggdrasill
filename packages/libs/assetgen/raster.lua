-- `packages/libs/assetgen/raster.lua` —— 网格 → 纹理：正交投影 + 三角形填充。
--
-- 契约：`docs/design/packages/assetgen.md` §4（`assetgen.raster`）/ §6（几何的第二用点）。
--
-- # 为什么它是几何的第二个用点
--
-- 3D 网格只有 Saltline 一个确定用点；PokeRogue 的 2D 图标消费的是**同一份**数据
-- （顶点 + 面 + 包围盒）——栅格化只是换一种投影，不是另一套形状。
--
-- # 为什么用亚像素整数
--
-- 直接在像素网格上做点内测试会把斜边量化成锯齿块，且「覆盖面积」随角度跳变。
-- 把坐标放大 1024 倍后用整数叉积判定，既保持零浮点（§3 I1），又让边缘落在
-- 1/1024 像素上：像素中心（`+512`）是否落在三角形内是**精确**判定，不是近似。

local mathx = require("pkg.assetgen.mathx")
local color = require("pkg.assetgen.color")
local spec = require("pkg.assetgen.spec")
local bytes = require("pkg.assetgen.bytes")
local join = require("pkg.assetgen.join")

local M = {}

local fail = spec.fail
local E = spec.E

local SUB = 1024 -- 亚像素分辨率（1 像素 = 1024 单位）
local HALF = 512 -- 像素中心相对像素原点的偏移

--- 光照缩放的输入：法线先按 256 降尺度，避免 `1e6²` 的叉积把 i64 顶爆。
local NORMAL_SHIFT = 256

--- `raster(mesh, opts)` → 纹理产物（`ch = 4`，画面外 alpha = 0）。
function M.raster(mesh, opts)
  if type(mesh) ~= "table" or type(mesh.verts) ~= "table" or type(mesh.faces) ~= "table" then
    fail(E.PARAM, "raster 的第一个参数必须是 mesh 产物（需要 verts / faces）")
  end
  if type(opts) ~= "table" then
    fail(E.PARAM, "raster 的 opts 必须是表，得到 " .. type(opts))
  end
  local w = spec.int(opts, "w", 32, 1, 4096)
  local h = spec.int(opts, "h", 32, 1, 4096)
  local scale = spec.int(opts, "scale", 1000, 1, 65536)
  local ox = spec.range(opts, "ox", 0, -65536, 65536)
  local oy = spec.range(opts, "oy", 0, -65536, 65536)
  local angle = spec.range(opts, "angle", 0, 0, 255)
  local fill = spec.rgb(opts, "fill", 0xFFFFFF)
  local shades = spec.range(opts, "shades", 0, 0, 1000)

  local phase = (angle * 65536) // 256
  local ct = mathx.cos_q15(phase)
  local st = mathx.sin_q15(phase)

  --- 顶点 → 亚像素坐标（XY 正交投影；`y` 上 → 像素 y 下）。
  local function project(x, y)
    local rx = mathx.q15_div(x * ct - y * st)
    local ry = mathx.q15_div(x * st + y * ct)
    local px = (w * SUB) // 2 + (rx * scale * SUB) // 1000 + ox * SUB
    local py = (h * SUB) // 2 - (ry * scale * SUB) // 1000 + oy * SUB
    return px, py
  end

  if w * h * 4 > 4194304 then
    fail(E.CAP, "w*h*4 超过单件帽 4194304 字节（" .. w .. "×" .. h .. "）")
  end
  local canvas = {}
  local verts = mesh.verts
  local faces = mesh.faces

  for f = 1, #faces // 3 do
    local i0 = faces[(f - 1) * 3 + 1]
    local i1 = faces[(f - 1) * 3 + 2]
    local i2 = faces[(f - 1) * 3 + 3]
    local at0 = (i0 - 1) * 3
    local at1 = (i1 - 1) * 3
    local at2 = (i2 - 1) * 3
    local x0, y0 = verts[at0 + 1], verts[at0 + 2]
    local x1, y1 = verts[at1 + 1], verts[at1 + 2]
    local x2, y2 = verts[at2 + 1], verts[at2 + 2]
    local ax, ay = project(x0, y0)
    local bx, by = project(x1, y1)
    local cx, cy = project(x2, y2)

    -- 光照：面法线的 Y 分量（顶光），`shades` 千分比混入。
    local tone = 1000
    if shades > 0 then
      local e1x = (verts[at1 + 1] - x0) // NORMAL_SHIFT
      local e1y = (verts[at1 + 2] - y0) // NORMAL_SHIFT
      local e1z = (verts[at1 + 3] - verts[at0 + 3]) // NORMAL_SHIFT
      local e2x = (verts[at2 + 1] - x0) // NORMAL_SHIFT
      local e2y = (verts[at2 + 2] - y0) // NORMAL_SHIFT
      local e2z = (verts[at2 + 3] - verts[at0 + 3]) // NORMAL_SHIFT
      local nx = e1y * e2z - e1z * e2y
      local ny = e1z * e2x - e1x * e2z
      local nz = e1x * e2y - e1y * e2x
      local length = mathx.isqrt(nx * nx + ny * ny + nz * nz)
      if length > 0 then
        local lit = 500 + (500 * ny) // length
        tone = 1000 - shades + (shades * lit) // 1000
      end
    end
    local shade = color.scale(fill, tone)

    local min_x = mathx.min(ax, mathx.min(bx, cx))
    local max_x = mathx.max(ax, mathx.max(bx, cx))
    local min_y = mathx.min(ay, mathx.min(by, cy))
    local max_y = mathx.max(ay, mathx.max(by, cy))
    local left = mathx.max(0, min_x // SUB)
    local right = mathx.min(w - 1, max_x // SUB + 1)
    local top = mathx.max(0, min_y // SUB)
    local bottom = mathx.min(h - 1, max_y // SUB + 1)

    local area = (bx - ax) * (cy - ay) - (by - ay) * (cx - ax)
    if area ~= 0 then
      for py = top, bottom do
        local sy = py * SUB + HALF
        for px = left, right do
          local sx = px * SUB + HALF
          local d1 = (bx - ax) * (sy - ay) - (by - ay) * (sx - ax)
          local d2 = (cx - bx) * (sy - by) - (cy - by) * (sx - bx)
          local d3 = (ax - cx) * (sy - cy) - (ay - cy) * (sx - cx)
          if area > 0 then
            if d1 >= 0 and d2 >= 0 and d3 >= 0 then
              canvas[py * w + px + 1] = shade
            end
          elseif d1 <= 0 and d2 <= 0 and d3 <= 0 then
            canvas[py * w + px + 1] = shade
          end
        end
      end
    end
  end

  local rows = {}
  for y = 0, h - 1 do
    local parts = {}
    for x = 0, w - 1 do
      local packed = canvas[y * w + x + 1]
      if packed == nil then
        parts[x + 1] = bytes.u8(0) .. bytes.u8(0) .. bytes.u8(0) .. bytes.u8(0)
      else
        local r, g, b = color.unpack(packed)
        parts[x + 1] = bytes.u8(r) .. bytes.u8(g) .. bytes.u8(b) .. bytes.u8(255)
      end
    end
    rows[y + 1] = join.concat(parts)
  end
  return { w = w, h = h, ch = 4, bytes = join.concat(rows) }
end

return M
