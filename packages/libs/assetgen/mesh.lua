-- `packages/libs/assetgen/mesh.lua` —— 几何产物：参数化基元 + 变换组合。
--
-- 契约：`openwiki/contracts/packages/assetgen.md` §2（b）/ §3 I8。
--
-- # 单位与坐标系
--
-- 顶点单位 = **毫米**（1/1000），右手坐标系、Y 上。用毫米而不是「游戏单位」的理由：
-- 整数域需要一个物理量纲才能把「1 米」写成 `1000` 而不是浮点 `1.0`——`1.0` 是浮点
-- 字面量，静态用例直接红（§3 I1）。
--
-- # 三个硬帽（越界是 CAP 不是 PARAM）
--
-- 顶点数 ≤ 65535（OBJ 索引与 u16 索引缓冲双友好）、`|coord| ≤ 1000000`、
-- 细分数 `segs/rings ∈ 3..64`。帽在**建表之前**判：先算顶点数再决定要不要分配。

local mathx = require("pkg.assetgen.mathx")
local spec = require("pkg.assetgen.spec")

local M = {}

local fail = spec.fail
local E = spec.E

local MAX_VERTS = 65535
local MAX_COORD = 1000000
local Q15 = 32768
local FULL_TURN = 65536
local HALF_TURN_Q = 32768 -- 半转 = 180 度（球的纬度跨度）

local function push3(list, x, y, z)
  local n = #list
  list[n + 1] = x
  list[n + 2] = y
  list[n + 3] = z
end

local function tri(list, a, b, c)
  local n = #list
  list[n + 1] = a
  list[n + 2] = b
  list[n + 3] = c
end

--- 轴对齐包围盒尺寸（`max - min`，mm）。
local function extent(verts)
  local min_x, min_y, min_z = verts[1], verts[2], verts[3]
  local max_x, max_y, max_z = min_x, min_y, min_z
  for i = 2, #verts // 3 do
    local at = (i - 1) * 3
    local x, y, z = verts[at + 1], verts[at + 2], verts[at + 3]
    if x < min_x then
      min_x = x
    end
    if x > max_x then
      max_x = x
    end
    if y < min_y then
      min_y = y
    end
    if y > max_y then
      max_y = y
    end
    if z < min_z then
      min_z = z
    end
    if z > max_z then
      max_z = z
    end
  end
  return { max_x - min_x, max_y - min_y, max_z - min_z }
end

--- 顶点数 / 坐标帽（在建表之后、返回之前；`-CAP` 不截断）。
local function guard(verts, faces)
  if #verts // 3 > MAX_VERTS then
    fail(E.CAP, "顶点数 " .. (#verts // 3) .. " 超过 " .. MAX_VERTS)
  end
  for i = 1, #verts do
    local v = verts[i]
    if v > MAX_COORD or v < -MAX_COORD then
      fail(E.CAP, "顶点坐标 " .. v .. " 超出 ±" .. MAX_COORD .. "（mm）")
    end
  end
  return { verts = verts, faces = faces, size = extent(verts) }
end

-- ── 基元 ────────────────────────────────────────────────────────────────────

--- `mesh.box`：轴对齐长方体（8 顶点 / 12 三角），中心在原点。
function M.box(params, _)
  local sx = spec.int(params, "sx", 1000, 1, MAX_COORD)
  local sy = spec.int(params, "sy", 1000, 1, MAX_COORD)
  local sz = spec.int(params, "sz", 1000, 1, MAX_COORD)
  local x0 = -(sx // 2)
  local y0 = -(sy // 2)
  local z0 = -(sz // 2)
  local x1 = x0 + sx
  local y1 = y0 + sy
  local z1 = z0 + sz
  local verts = {}
  push3(verts, x0, y0, z0)
  push3(verts, x1, y0, z0)
  push3(verts, x1, y1, z0)
  push3(verts, x0, y1, z0)
  push3(verts, x0, y0, z1)
  push3(verts, x1, y0, z1)
  push3(verts, x1, y1, z1)
  push3(verts, x0, y1, z1)
  local faces = {}
  -- 六个面，每个面两个三角（绕序朝外）。
  tri(faces, 1, 3, 2)
  tri(faces, 1, 4, 3) -- z0
  tri(faces, 5, 6, 7)
  tri(faces, 5, 7, 8) -- z1
  tri(faces, 1, 2, 6)
  tri(faces, 1, 6, 5) -- y0
  tri(faces, 2, 3, 7)
  tri(faces, 2, 7, 6) -- x1
  tri(faces, 3, 4, 8)
  tri(faces, 3, 8, 7) -- y1
  tri(faces, 4, 1, 5)
  tri(faces, 4, 5, 8) -- x0
  return guard(verts, faces)
end

--- `mesh.cylinder`：直圆柱（`2*segs` 顶点；侧面 2 三角/段，两端扇形封盖）。
function M.cylinder(params, _)
  local radius = spec.int(params, "radius", 500, 1, MAX_COORD)
  local height = spec.int(params, "height", 1000, 1, MAX_COORD)
  local segs = spec.int(params, "segs", 16, 3, 64)
  local half = height // 2
  local verts, faces = {}, {}
  for i = 0, segs - 1 do
    local phase = (i * FULL_TURN) // segs
    local x = mathx.q15_div(radius * mathx.cos_q15(phase))
    local z = mathx.q15_div(radius * mathx.sin_q15(phase))
    push3(verts, x, half, z) -- 顶环 1..segs
    push3(verts, x, -half, z) -- 底环 segs+1..2*segs
  end
  for i = 1, segs do
    local top0 = i
    local top1 = (i % segs) + 1
    local bottom0 = top0 + segs
    local bottom1 = top1 + segs
    tri(faces, top0, top1, bottom1)
    tri(faces, top0, bottom1, bottom0)
  end
  -- 封盖用扇形（不新增圆心顶点：顶点数必须恰好是 `2*segs`）。
  for i = 2, segs - 1 do
    tri(faces, 1, i, i + 1)
    tri(faces, segs + 1, segs + i + 1, segs + i)
  end
  return guard(verts, faces)
end

--- `mesh.plane`：`n × n` 的方格平面（`(n+1)²` 顶点、`2n²` 三角），中心在原点。
---
--- `axis`：0 = XY（法线 +Z，正交投影到画面就成了一张图）、1 = XZ（法线 +Y，地板）、
--- 2 = YZ。默认 0 是因为它的两个用途都很直接：栅格化的输入、以及 2D 图标。
function M.plane(params, _)
  local size = spec.int(params, "size", 1000, 1, MAX_COORD)
  local n = spec.int(params, "n", 1, 1, 256)
  local axis = spec.range(params, "axis", 0, 0, 2)
  local count = (n + 1) * (n + 1)
  if count > MAX_VERTS then
    fail(E.CAP, "平面 " .. (n + 1) .. "×" .. (n + 1) .. " 的顶点数超过 " .. MAX_VERTS)
  end
  local step = size // n
  local start = -(size // 2)
  local verts, faces = {}, {}
  for j = 0, n do
    for i = 0, n do
      local a = start + i * step
      local b = start + j * step
      if axis == 1 then
        push3(verts, a, 0, b)
      elseif axis == 2 then
        push3(verts, 0, a, b)
      else
        push3(verts, a, b, 0)
      end
    end
  end
  local width = n + 1
  for j = 0, n - 1 do
    for i = 0, n - 1 do
      local a = j * width + i + 1
      local b = a + 1
      local d = a + width
      local c = d + 1
      tri(faces, a, b, c)
      tri(faces, a, c, d)
    end
  end
  return guard(verts, faces)
end


--- `mesh.torus`：环面（`segs × rings` 顶点、`2·segs·rings` 三角）。
---
--- 两个角度都走 Q15 三角核：`ring_r = R + r·cos(φ)`，再 `x = ring_r·cos(θ)`、
--- `z = ring_r·sin(θ)`、`y = r·sin(φ)`。中间量最大 `2e6 × 32768 ≈ 6.6e10`，远在 i64 内。
function M.torus(params, _)
  local radius = spec.int(params, "radius", 1000, 1, MAX_COORD)
  local tube = spec.int(params, "tube", 300, 1, MAX_COORD)
  local segs = spec.int(params, "segs", 24, 3, 64)
  local rings = spec.int(params, "rings", 12, 3, 64)
  local count = segs * rings
  if count > MAX_VERTS then
    fail(E.CAP, "环面 " .. segs .. "×" .. rings .. " 的顶点数超过 " .. MAX_VERTS)
  end
  local verts, faces = {}, {}
  for i = 0, segs - 1 do
    local theta = (i * FULL_TURN) // segs
    local ct = mathx.cos_q15(theta)
    local st = mathx.sin_q15(theta)
    for j = 0, rings - 1 do
      local phi = (j * FULL_TURN) // rings
      local ring_r = radius + mathx.q15_div(tube * mathx.cos_q15(phi))
      push3(verts, mathx.q15_div(ring_r * ct), mathx.q15_div(tube * mathx.sin_q15(phi)), mathx.q15_div(ring_r * st))
    end
  end
  for i = 0, segs - 1 do
    local next_i = (i + 1) % segs
    for j = 0, rings - 1 do
      local next_j = (j + 1) % rings
      local a = i * rings + j + 1
      local b = next_i * rings + j + 1
      local c = next_i * rings + next_j + 1
      local d = i * rings + next_j + 1
      tri(faces, a, b, c)
      tri(faces, a, c, d)
    end
  end
  return guard(verts, faces)
end

--- `mesh.blob`：径向位移的球（`segs × rings` 顶点，拓扑同环面 —— 不带极点）。
---
--- 纬度取 `(2j+1)` 的中点角，两端都不落在极点上：极点会被重复 rings 次，而重复顶点
--- 在 OBJ 里是合法的，却让「顶点数 = segs·rings」这条自省断言失去意义。
function M.blob(params, rng)
  local radius = spec.int(params, "radius", 1000, 1, MAX_COORD)
  local segs = spec.int(params, "segs", 12, 3, 64)
  local rings = spec.int(params, "rings", 8, 3, 64)
  local amp = spec.range(params, "amp", 200, 0, 1000)
  local count = segs * rings
  if count > MAX_VERTS then
    fail(E.CAP, "位移球 " .. segs .. "×" .. rings .. " 的顶点数超过 " .. MAX_VERTS)
  end
  local verts, faces = {}, {}
  for j = 0, rings - 1 do
    local beta = ((2 * j + 1) * HALF_TURN_Q) // (2 * rings) -- (0, 32768) = 0..180 度
    local latitude_r = radius + (radius * rng:int(-amp, amp)) // 1000
    local y = mathx.q15_div(latitude_r * mathx.cos_q15(beta))
    local ring_r = mathx.q15_div(latitude_r * mathx.sin_q15(beta))
    for i = 0, segs - 1 do
      local theta = (i * FULL_TURN) // segs
      push3(verts, mathx.q15_div(ring_r * mathx.cos_q15(theta)), y, mathx.q15_div(ring_r * mathx.sin_q15(theta)))
    end
  end
  for i = 0, segs - 1 do
    local next_i = (i + 1) % segs
    for j = 0, rings - 1 do
      local next_j = (j + 1) % rings
      local a = j * segs + i + 1
      local b = j * segs + next_i + 1
      local c = next_j * segs + next_i + 1
      local d = next_j * segs + i + 1
      tri(faces, a, b, c)
      tri(faces, a, c, d)
    end
  end
  return guard(verts, faces)
end

--- `mesh.compose`：把若干网格按**入参顺序**做「缩放 → 绕 Y 旋转 → 平移」后合并。
---
--- 顺序有意义：`parts` 反序产出的 OBJ 字节点序不同（`mesh-compose-order` 钉这条）。
--- 变换参数：`scale` 千分比（1000 = 原大小）、`turn` 1/256 转（0..255）、
--- `at = {dx, dy, dz}` 平移（mm）。
function M.compose(params, _)
  local parts = spec.array(params, "parts", nil)
  if parts == nil or #parts == 0 then
    fail(E.PARAM, "mesh.compose 需要非空的 params.parts")
  end
  local verts, faces = {}, {}
  for index = 1, #parts do
    local part = parts[index]
    if type(part) ~= "table" then
      fail(E.PARAM, "params.parts[" .. index .. "] 必须是表")
    end
    local source = part.product
    if type(source) ~= "table" or type(source.verts) ~= "table" or type(source.faces) ~= "table" then
      fail(E.PARAM, "params.parts[" .. index .. "].product 必须是 mesh 产物")
    end
    local scale = spec.range(part, "scale", 1000, 1, 100000)
    local turn = spec.range(part, "turn", 0, 0, 255)
    local at = spec.array(part, "at", { 0, 0, 0 })
    local phase = (turn * FULL_TURN) // 256
    local ct = mathx.cos_q15(phase)
    local st = mathx.sin_q15(phase)
    local offset = #verts
    local source_verts = source.verts
    for i = 1, #source_verts // 3 do
      local at3 = (i - 1) * 3
      local x = (source_verts[at3 + 1] * scale) // 1000
      local y = (source_verts[at3 + 2] * scale) // 1000
      local z = (source_verts[at3 + 3] * scale) // 1000
      push3(verts, mathx.q15_div(x * ct + z * st) + at[1], y + at[2], mathx.q15_div(z * ct - x * st) + at[3])
    end
    local source_faces = source.faces
    for i = 1, #source_faces do
      faces[#faces + 1] = source_faces[i] + offset
    end
  end
  return guard(verts, faces)
end

return M
