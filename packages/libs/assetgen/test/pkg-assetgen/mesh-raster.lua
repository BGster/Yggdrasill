-- mesh-raster：用例段自 pkg-assetgen.lua 原样搬移（S-25 批十三测试件连拆）。
return function(C)
  local S, run, eq, is_true, expect_error, fresh, pkg_sources, source_text, strip, samples_of, OSC, BUNDLE_ENTRIES =
    C.S, C.run, C.eq, C.is_true, C.expect_error, C.fresh, C.pkg_sources, C.source_text, C.strip, C.samples_of, C.OSC, C.BUNDLE_ENTRIES
run("pkg.assetgen.mesh-primitives", function()
  local assetgen = fresh()

  local box = assetgen.mesh({ algo = "mesh.box", params = { sx = 1000, sy = 500, sz = 200 } }, "m")
  eq(#box.verts // 3, 8, "box 顶点数")
  eq(#box.faces // 3, 12, "box 三角数")
  eq(box.size[1], 1000, "box size.x")
  eq(box.size[2], 500, "box size.y")
  eq(box.size[3], 200, "box size.z")

  local cylinder = assetgen.mesh({ algo = "mesh.cylinder", params = { radius = 300, height = 900, segs = 8 } }, "m")
  eq(#cylinder.verts // 3, 16, "cylinder 顶点数 = 2*segs")
  eq(#cylinder.faces // 3, 2 * 8 + 2 * 6, "cylinder 三角数 = 侧面 2*segs + 两扇形 2*(segs-2)")
  eq(cylinder.size[2], 900, "cylinder 高度")

  local torus = assetgen.mesh({ algo = "mesh.torus", params = { radius = 800, tube = 200, segs = 12, rings = 6 } }, "m")
  eq(#torus.verts // 3, 12 * 6, "torus 顶点数 = segs*rings")
  eq(#torus.faces // 3, 2 * 12 * 6, "torus 三角数 = 2*segs*rings")

  local plane = assetgen.mesh({ algo = "mesh.plane", params = { size = 1000, n = 4 } }, "m")
  eq(#plane.verts // 3, 25, "plane 顶点数 = (n+1)^2")
  eq(#plane.faces // 3, 32, "plane 三角数 = 2n^2")
  eq(plane.size[1], 1000, "plane size.x")
  eq(plane.size[2], 1000, "plane size.y")
  eq(plane.size[3], 0, "plane size.z（XY 平面）")

  local blob = assetgen.mesh({ algo = "mesh.blob", params = { radius = 700, segs = 8, rings = 5, amp = 300 } }, "m")
  eq(#blob.verts // 3, 8 * 5, "blob 顶点数 = segs*rings")
  eq(#blob.faces // 3, 2 * 8 * 5, "blob 三角数 = 2*segs*rings")
end)

run("pkg.assetgen.mesh-compose-order", function()
  local assetgen = fresh()
  local unit = assetgen.mesh({ algo = "mesh.box", params = { sx = 1000, sy = 1000, sz = 1000 } }, "m")
  local parts = {
    { product = unit, at = { 0, 0, 0 } },
    { product = unit, at = { 2000, 0, 0 } },
  }
  local forward = assetgen.mesh({ algo = "mesh.compose", params = { parts = parts } }, "m")
  local backward = assetgen.mesh({
    algo = "mesh.compose",
    params = { parts = { parts[2], parts[1] } },
  }, "m")

  -- 包围盒并集：x ∈ [-500, 2500] → 3000；订单序不影响包围盒，但影响顶点序。
  eq(forward.size[1], 3000, "compose 并集 size.x")
  eq(forward.size[2], 1000, "compose 并集 size.y")
  is_true(forward.bytes ~= backward.bytes, "parts 反序必须产出不同字节（变换顺序参与产物）")

  -- 第二个锚点：平移不改变**尺寸**（`max-min` 是平移不变量），但会改变顶点本身。
  local shifted = assetgen.mesh({
    algo = "mesh.compose",
    params = { parts = { { product = unit, at = { 500, 0, 0 } } } },
  }, "m")
  eq(shifted.size[1], 1000, "平移不改变 size.x（max-min）")
  local lowest = shifted.verts[1]
  for i = 1, #shifted.verts // 3 do
    local v = shifted.verts[(i - 1) * 3 + 1]
    if v < lowest then
      lowest = v
    end
  end
  eq(lowest, 0, "平移 500 后最小 x（-500 + 500）")

  -- 千分比缩放参与包围盒：500‰ → 500mm。
  local halved = assetgen.mesh({
    algo = "mesh.compose",
    params = { parts = { { product = unit, scale = 500 } } },
  }, "m")
  eq(halved.size[1], 500, "scale=500 后的 size.x")
end)

run("pkg.assetgen.mesh-ints-only", function()
  local assetgen = fresh()
  local cases = {
    { algo = "mesh.box", params = { sx = 999, sy = 501, sz = 333 } },
    { algo = "mesh.cylinder", params = { radius = 617, height = 1234, segs = 7 } },
    { algo = "mesh.torus", params = { radius = 913, tube = 217, segs = 9, rings = 5 } },
    { algo = "mesh.blob", params = { radius = 811, segs = 8, rings = 6, amp = 400 } },
    { algo = "mesh.plane", params = { size = 777, n = 3 } },
  }
  for c = 1, #cases do
    local product = assetgen.mesh(cases[c], "ints")
    eq(#product.verts % 3, 0, cases[c].algo .. " 的 verts 必须是 3 的倍数")
    eq(#product.faces % 3, 0, cases[c].algo .. " 的 faces 必须是 3 的倍数")
    for i = 1, #product.verts do
      local v = product.verts[i]
      eq(math.type(v), "integer", cases[c].algo .. " 顶点必须是 integer（第 " .. i .. " 个）")
      is_true(v <= 1000000 and v >= -1000000, cases[c].algo .. " 顶点超出 ±1000000：" .. tostring(v))
    end
    for i = 1, #product.faces do
      local f = product.faces[i]
      is_true(f >= 1 and f <= #product.verts // 3, cases[c].algo .. " 面索引越界：" .. tostring(f))
    end
  end
end)

run("pkg.assetgen.mesh-obj-text", function()
  local assetgen = fresh()
  local product = assetgen.mesh({
    algo = "mesh.torus",
    params = { radius = 1000, tube = 250, segs = 6, rings = 4 },
  }, "obj")
  local text = product.bytes
  local vertices, faces = 0, 0
  local bad_decimal = nil
  local has_cr = text:find("\r", 1, true) ~= nil
  for line in text:gmatch("[^\n]+") do
    if line:sub(1, 2) == "v " then
      vertices = vertices + 1
      local count = 0
      for token in line:gmatch("%S+") do
        if token ~= "v" then
          count = count + 1
          if token:find("^%-?%d+%.%d%d%d$") == nil then
            bad_decimal = token
          end
        end
      end
      if count ~= 3 then
        error("v 行必须有 3 个坐标，实得 " .. count .. "：" .. line, 0)
      end
    elseif line:sub(1, 2) == "f " then
      faces = faces + 1
    end
  end
  eq(vertices, #product.verts // 3, "v 行数")
  eq(faces, #product.faces // 3, "f 行数")
  is_true(not has_cr, "OBJ 不得含 \\r（跨平台换行差异会污染字节一致）")
  is_true(bad_decimal == nil, "顶点必须恰好三位小数，实得 " .. tostring(bad_decimal))
  eq(text:sub(1, #("# assetgen mesh v1 mesh.torus")), "# assetgen mesh v1 mesh.torus", "首行是版本 + 算法名")
end)

run("pkg.assetgen.mesh-caps", function()
  local assetgen = fresh()
  expect_error("YGG-PKG-ASSETGEN-CAP", function()
    assetgen.mesh({ algo = "mesh.cylinder", params = { segs = 65 } }, "caps")
  end, "segs=65")
  expect_error("YGG-PKG-ASSETGEN-CAP", function()
    assetgen.mesh({ algo = "mesh.plane", params = { n = 256 } }, "caps")
  end, "顶点数超 65535")
  expect_error("YGG-PKG-ASSETGEN-ALGO", function()
    assetgen.mesh({ algo = "mesh.nope", params = {} }, "caps")
  end, "未知算法")
  expect_error("YGG-PKG-ASSETGEN-PARAM", function()
    assetgen.mesh({ algo = "mesh.compose", params = { parts = {} } }, "caps")
  end, "空 parts")
end)

-- ── 用例：栅格化（正交投影 + 三角形填充）────────────────────────────────────

--- 不透明像素计数（`ch = 4`，alpha 在第 4 个通道）。
local function opaque_count(product)
  local count = 0
  for i = 4, #product.bytes, 4 do
    if product.bytes:byte(i) ~= 0 then
      count = count + 1
    end
  end
  return count
end

run("pkg.assetgen.raster-fill", function()
  local assetgen = fresh()
  local plate = assetgen.mesh({ algo = "mesh.plane", params = { size = 1000, n = 1 } }, "raster")
  local image = assetgen.raster(plate, { w = 64, h = 64, scale = 60, fill = 0x40C0FF })
  eq(image.ch, 4, "raster 产物必须是 RGBA")
  eq(#image.bytes, 64 * 64 * 4, "#bytes = w*h*ch")

  -- 人算面积：1000mm × 60‰（px/mm）= 60px 见方 → A = 3600。
  local area = 60 * 60
  local filled = opaque_count(image)
  is_true(filled >= (area * 85) // 100, "填充像素数必须 ≥ 0.85·A（实得 " .. filled .. "）")
  is_true(filled <= area, "填充像素数必须 ≤ A（实得 " .. filled .. "）")

  -- 四角必须透明（板子占了中间 60×60，四角在板外）。
  for _, corner in ipairs({ { 0, 0 }, { 63, 0 }, { 0, 63 }, { 63, 63 } }) do
    local at = (corner[2] * 64 + corner[1]) * 4 + 4
    eq(image.bytes:byte(at), 0, "角 (" .. corner[1] .. "," .. corner[2] .. ") 必须透明")
  end
end)

run("pkg.assetgen.raster-input", function()
  local assetgen = fresh()
  local plate = assetgen.mesh({ algo = "mesh.plane", params = { size = 1000, n = 1 } }, "raster")
  expect_error("YGG-PKG-ASSETGEN-PARAM", function()
    assetgen.raster({ verts = { 0, 0, 0 }, size = { 0, 0, 0 } }, { w = 8, h = 8 })
  end, "缺 faces")
  expect_error("YGG-PKG-ASSETGEN-PARAM", function()
    assetgen.raster(plate, { w = 8, h = 8, scale = 0 })
  end, "scale=0")
  expect_error("YGG-PKG-ASSETGEN-PARAM", function()
    assetgen.raster(plate, { w = 8, h = 8, scale = 10, angle = 256 })
  end, "angle 越界")
  -- 旋转与填充色参与产物：同输入两次逐字节相等，改角度就不同。
  local a = assetgen.raster(plate, { w = 32, h = 32, scale = 30, angle = 0 })
  local b = assetgen.raster(plate, { w = 32, h = 32, scale = 30, angle = 0 })
  local c = assetgen.raster(plate, { w = 32, h = 32, scale = 30, angle = 32 })
  is_true(a.bytes == b.bytes, "同输入两次栅格化必须逐字节相等")
  is_true(a.bytes ~= c.bytes, "旋转 45° 后的字节必须不同（正方形转 90° 是恒等，不能当锚点）")
end)

-- ── 用例：音频（同 seed 同字节 / 峰值帽 / 波形形态）──────────────────────────

--- s16 小端解码成整数数组（只取第一声道）。


end
