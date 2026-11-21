-- texture：用例段自 pkg-assetgen.lua 原样搬移（S-25 批十三测试件连拆）。
return function(C)
  local S, run, eq, is_true, expect_error, fresh, pkg_sources, source_text, strip, samples_of, OSC, BUNDLE_ENTRIES =
    C.S, C.run, C.eq, C.is_true, C.expect_error, C.fresh, C.pkg_sources, C.source_text, C.strip, C.samples_of, C.OSC, C.BUNDLE_ENTRIES
run("pkg.assetgen.no-float-static", function()
  local paths = pkg_sources("assetgen")
  eq(#paths, 13, "manifest 里 assetgen 源文件数（加模块要同步抬这个数）")
  local forbidden = { "math.", "string.", "table.", "print(", "collectgarbage", "ygg.", ":float(", ".float(" }
  local problems = {}
  for i = 1, #paths do
    local text = strip(source_text(paths[i]))
    for j = 1, #forbidden do
      if text:find(forbidden[j], 1, true) ~= nil then
        problems[#problems + 1] = paths[i] .. " 含 " .. forbidden[j]
      end
    end
    if text:find("[^%w_]os%.") ~= nil then
      problems[#problems + 1] = paths[i] .. " 含 os."
    end
    if text:find("%d%.%d") ~= nil then
      problems[#problems + 1] = paths[i] .. " 含浮点字面量"
    end
    -- 单斜杠除法：`//` 合法，其余 `/` 都是浮点除法。
    for at = 1, #text do
      if text:sub(at, at) == "/" and text:sub(at - 1, at - 1) ~= "/" and text:sub(at + 1, at + 1) ~= "/" then
        problems[#problems + 1] = paths[i] .. " 含单斜杠除法（偏移 " .. at .. "）"
        break
      end
    end
  end
  is_true(#problems == 0, table.concat(problems, "; "))
end)

run("pkg.assetgen.rng-not-copied", function()
  local paths = pkg_sources("assetgen")
  local forbidden = {
    "xoshiro",
    "CBF29CE484222325",
    "100000001B3",
    "BF58476D1CE4E5B9",
    "94D049BB133111EB",
    "9E3779B9",
    "fnv64",
  }
  local problems = {}
  local injected = false
  for i = 1, #paths do
    local text = source_text(paths[i])
    for j = 1, #forbidden do
      if text:find(forbidden[j], 1, true) ~= nil then
        problems[#problems + 1] = paths[i] .. " 复制了 procgen 的常量 " .. forbidden[j]
      end
    end
    if text:find('require("pkg.procgen")', 1, true) ~= nil then
      injected = true
    end
  end
  is_true(injected, "init.lua 必须 require(\"pkg.procgen\")（共享 rng 契约单份）")
  is_true(#problems == 0, table.concat(problems, "; "))
end)

-- ── 用例：纹理（I1 同 seed 同字节 / 异 seed 不同 / 形状）──────────────────────

local FBM = { algo = "texture.fbm", params = { w = 24, h = 16, ch = 3, cell = 4, octaves = 3 } }

run("pkg.assetgen.texture-same-seed-same-bytes", function()
  local assetgen = fresh()
  local first = assetgen.texture(FBM, "tex-seed")
  local second = assetgen.texture(FBM, "tex-seed")
  eq(#first.bytes, #second.bytes, "同 seed 两次的长度")
  is_true(first.bytes == second.bytes, "同 seed 两次的字节必须逐字节相等")
  eq(first.canon, second.canon, "同 seed 两次的 canon")
  eq(first.canon, assetgen.canon(first), "canon(product) 与 product.canon")
end)

run("pkg.assetgen.texture-diff-seed-diff-bytes", function()
  local assetgen = fresh()
  local a = assetgen.texture(FBM, "tex-seed-a")
  local b = assetgen.texture(FBM, "tex-seed-b")
  is_true(a.bytes ~= b.bytes, "异 seed 的字节必须不同")
  eq(#a.bytes, #b.bytes, "异 seed 的长度必须相同（防「长度不同也算通过」的假阳性）")
end)

run("pkg.assetgen.texture-size-and-shape", function()
  local assetgen = fresh()
  local product = assetgen.texture(FBM, "tex-shape")
  eq(product.w, 24, "w")
  eq(product.h, 16, "h")
  eq(product.ch, 3, "ch")
  eq(#product.bytes, 24 * 16 * 3, "#bytes = w*h*ch")
  eq(math.type(product.w), "integer", "w 必须是 integer")
  eq(math.type(product.h), "integer", "h 必须是 integer")
  eq(math.type(product.ch), "integer", "ch 必须是 integer")
  local low, high = 255, 0
  for i = 1, #product.bytes do
    local v = product.bytes:byte(i)
    if v < low then
      low = v
    end
    if v > high then
      high = v
    end
  end
  -- fbm 必须是非退化场：全黑或全白说明格点/插值写错了。
  is_true(low < 64, "fbm 的最小值应 < 64，实得 " .. low)
  is_true(high > 192, "fbm 的最大值应 > 192，实得 " .. high)
end)

-- ── 用例：纹理图案逐值 ───────────────────────────────────────────────────────

--- 单通道纹理的像素取值（行主序、无行内边距）。
local function pixel(product, x, y)
  return product.bytes:byte((y * product.w + x) * product.ch + 1)
end

run("pkg.assetgen.texture-pattern-exact", function()
  local assetgen = fresh()

  -- 棋盘 8×8 `size=2`：(x//2 + y//2) 的奇偶。
  local checker = assetgen.texture(
    { algo = "texture.checker", params = { w = 8, h = 8, ch = 1, size = 2 } },
    "pattern"
  )
  eq(pixel(checker, 0, 0), 255, "checker(0,0)")
  eq(pixel(checker, 2, 0), 0, "checker(2,0)")
  eq(pixel(checker, 2, 2), 255, "checker(2,2)")
  eq(pixel(checker, 3, 1), 0, "checker(3,1)")

  -- 条纹 `period=4 duty=500`：一周期的前一半亮。
  local stripes = assetgen.texture(
    { algo = "texture.stripes", params = { w = 8, h = 2, ch = 1, period = 4, duty = 500 } },
    "pattern"
  )
  eq(pixel(stripes, 0, 0), 255, "stripes(0,0)")
  eq(pixel(stripes, 1, 1), 255, "stripes(1,1)")
  eq(pixel(stripes, 2, 0), 0, "stripes(2,0)")
  eq(pixel(stripes, 3, 1), 0, "stripes(3,1)")

  -- 同心环 `period=4`：双倍半径落在第 0 / 1 带。
  local rings = assetgen.texture(
    { algo = "texture.rings", params = { w = 8, h = 8, ch = 1, period = 4 } },
    "pattern"
  )
  eq(pixel(rings, 4, 4), 255, "rings(4,4)（中心，半径 1 → 带 0）")
  eq(pixel(rings, 2, 4), 255, "rings(2,4)（半径 3 → 带 0）")
  eq(pixel(rings, 1, 4), 0, "rings(1,4)（半径 5 → 带 1）")
  eq(pixel(rings, 0, 4), 0, "rings(0,4)（半径 7 → 带 1）")

  -- 错缝砖墙 `bw=8 bh=4 mortar=1`：奇数行右移半砖。
  local bricks = assetgen.texture(
    { algo = "texture.bricks", params = { w = 16, h = 8, ch = 1, bw = 8, bh = 4, mortar = 1 } },
    "pattern"
  )
  eq(pixel(bricks, 0, 0), 0, "bricks(0,0)（灰缝）")
  eq(pixel(bricks, 1, 1), 255, "bricks(1,1)（砖面）")
  eq(pixel(bricks, 0, 4), 0, "bricks(0,4)（第二行灰缝）")
  eq(pixel(bricks, 5, 5), 255, "bricks(5,5)（错缝后的砖面）")
end)

run("pkg.assetgen.texture-gradient-endpoints", function()
  local assetgen = fresh()

  -- 线性 x：0 → 127 → 255（`w-1 = 4`，中点是 510//4 = 127，不是 128：向下取整）。
  local along_x = assetgen.texture(
    { algo = "texture.gradient", params = { w = 5, h = 1, ch = 1, shape = "linear", axis = 0 } },
    "grad"
  )
  eq(pixel(along_x, 0, 0), 0, "gradient x 起点")
  eq(pixel(along_x, 2, 0), 127, "gradient x 中点")
  eq(pixel(along_x, 4, 0), 255, "gradient x 末点")

  local along_y = assetgen.texture(
    { algo = "texture.gradient", params = { w = 1, h = 5, ch = 1, shape = "linear", axis = 1 } },
    "grad"
  )
  eq(pixel(along_y, 0, 0), 0, "gradient y 起点")
  eq(pixel(along_y, 0, 2), 127, "gradient y 中点")
  eq(pixel(along_y, 0, 4), 255, "gradient y 末点")

  -- 径向：中心最亮、四角最暗，且四象限逐点对称（`2x-(w-1)` 保证对称轴不偏）。
  local radial = assetgen.texture(
    { algo = "texture.gradient", params = { w = 9, h = 9, ch = 1, shape = "radial" } },
    "grad"
  )
  eq(pixel(radial, 4, 4), 255, "radial 中心（d=0）")
  eq(pixel(radial, 0, 0), 0, "radial 角落（d=span）")
  eq(pixel(radial, 2, 2), pixel(radial, 6, 2), "radial 左右对称")
  eq(pixel(radial, 2, 2), pixel(radial, 2, 6), "radial 上下对称")
  eq(pixel(radial, 2, 2), pixel(radial, 6, 6), "radial 对角对称")
end)

-- ── 用例：voronoi 等距取最小索引 + 异 seed 结构不同 ──────────────────────────

--- 相邻像素取值不同的对数（「边界」的可数定义；两 seed 必须给出不同的结构）。
local function boundary_pairs(product)
  local count = 0
  for y = 0, product.h - 1 do
    for x = 0, product.w - 2 do
      if pixel(product, x, y) ~= pixel(product, x + 1, y) then
        count = count + 1
      end
    end
  end
  return count
end

run("pkg.assetgen.texture-voronoi-tiebreak", function()
  local assetgen = fresh()

  -- 显式站点：两条站点在 (8,8) 等距，取**最小索引**（1 → 值 255）。
  local tied = assetgen.texture({
    algo = "texture.voronoi",
    params = {
      w = 16,
      h = 16,
      ch = 1,
      sites = { { x = 4, y = 8 }, { x = 12, y = 8 } },
      values = { 255, 0 },
    },
  }, "voronoi")
  eq(pixel(tied, 8, 8), 255, "等距点必须取最小站点索引（1 → 255）")
  eq(pixel(tied, 0, 8), 255, "近站点 1 的一侧")
  eq(pixel(tied, 15, 8), 0, "近站点 2 的一侧")

  -- 站点由流抽出：两个 seed 的边界像素集合必须不同（结构不同，不是同一张图）。
  local spec_v = { algo = "texture.voronoi", params = { w = 32, h = 32, ch = 1, cells = 6 } }
  local a = assetgen.texture(spec_v, "voronoi-a")
  local b = assetgen.texture(spec_v, "voronoi-b")
  local edges_a = boundary_pairs(a)
  local edges_b = boundary_pairs(b)
  is_true(edges_a > 0, "seed A 必须产生边界（实得 " .. edges_a .. "）")
  is_true(edges_b > 0, "seed B 必须产生边界（实得 " .. edges_b .. "）")
  is_true(edges_a ~= edges_b, "两 seed 的边界像素集合必须不同（" .. edges_a .. " vs " .. edges_b .. "）")
end)

-- ── 用例：纹理硬帽与值域三段 ────────────────────────────────────────────────

run("pkg.assetgen.texture-caps", function()
  local assetgen = fresh()
  local function tex(params)
    return function()
      assetgen.texture({ algo = "texture.checker", params = params }, "caps")
    end
  end
  -- 超过 4 MiB（2048×1024×3 = 6.3 MB）→ CAP。
  expect_error("YGG-PKG-ASSETGEN-CAP", tex({ w = 2048, h = 1024, ch = 3 }), "w*h*ch 超帽")
  -- `ch = 2` 不是三值之一 → PARAM（它在 [1,4] 内，但语义上不存在）。
  expect_error("YGG-PKG-ASSETGEN-PARAM", tex({ w = 8, h = 8, ch = 2 }), "ch=2")
  -- 低于下界 → PARAM。
  expect_error("YGG-PKG-ASSETGEN-PARAM", tex({ w = 0, h = 8 }), "w=0")
  -- 高于上界（4096）→ CAP。
  expect_error("YGG-PKG-ASSETGEN-CAP", tex({ w = 4097, h = 1 }), "w=4097")
  -- 未知算法 → ALGO。
  expect_error("YGG-PKG-ASSETGEN-ALGO", function()
    assetgen.texture({ algo = "texture.nope", params = { w = 4, h = 4 } }, "caps")
  end, "未知算法")
  -- 前缀不符是**形状**错（调用方把 kind 写错了），不是未知算法。
  expect_error("YGG-PKG-ASSETGEN-PARAM", function()
    assetgen.texture({ algo = "mesh.box", params = { w = 4, h = 4 } }, "caps")
  end, "kind 前缀不符")
end)

-- ── 用例：几何基元 / 变换 / 整数域 / OBJ 文本 / 硬帽 ─────────────────────────

end
