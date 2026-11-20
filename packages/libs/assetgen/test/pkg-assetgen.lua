-- `pkg.assetgen.*` 自测。契约 `docs/design/packages/assetgen.md` §7。
--
-- # 为什么自带计数垫片
--
-- 包不依赖消费它的 mod（`packages/README.md` §5）：计数面直接挂 `__SELFTEST`。
-- 包**不碰宿主面**（不写 KV、不发事件、不注册 handler），所以不需要宿主 mock。
--
-- # 隔离口径
--
-- 每条用例 `fresh()`：清掉 `pkg.assetgen*` 全部 `package.loaded` 项再 require +
-- install，算法表与注入状态一起归零。用例之间不共享任何状态。
--
-- 命名 ⇒ 计数可复算：文件名 `pkg-assetgen.lua`，`--filter pkg-assetgen` 跑到它。

local S = rawget(_G, "__SELFTEST")
if type(S) ~= "table" then
  S = { pass = 0, fail = 0 }
  _G.__SELFTEST = S
end

local function run(name, body)
  local ok, err = pcall(body)
  if ok then
    S.pass = S.pass + 1
  else
    S.fail = S.fail + 1
    print("FAIL " .. name .. ": " .. tostring(err))
  end
end

local function eq(actual, expected, label)
  if actual ~= expected then
    error(
      ("%s: expected %s, got %s"):format(tostring(label or "eq"), tostring(expected), tostring(actual)),
      2
    )
  end
end

local function is_true(value, label)
  if not value then
    error((label or "is_true") .. ": expected truthy", 2)
  end
end

local function expect_error(code, body, label)
  local ok, err = pcall(body)
  if ok then
    error((label or "expect_error") .. ": expected " .. code .. ", but the call succeeded", 2)
  end
  if not tostring(err):find(code, 1, true) then
    error(("%s: expected %s, got %s"):format(label or "expect_error", code, tostring(err)), 2)
  end
end

--- 取一份**全新**的包实例（清缓存 = 算法表与 install 状态归零），并装配。
local function fresh()
  for name in pairs(package.loaded) do
    if type(name) == "string" and (name == "pkg.assetgen" or name:find("^pkg%.assetgen%.")) then
      package.loaded[name] = nil
    end
  end
  local assetgen = require("pkg.assetgen")
  assetgen.install()
  return assetgen
end

-- ── 静态面工具 ──────────────────────────────────────────────────────────────

--- 包的源文件清单：来自 manifest 的 packages 段（内容寻址那截，天然排除 `test/**`）。
local function pkg_sources(id)
  local manifest = rawget(_G, "__MANIFEST")
  if type(manifest) ~= "table" or type(manifest.packages) ~= "table" then
    error("manifest 缺 packages 段（载体 pack.json 未声明包或未重算 manifest）", 0)
  end
  for i = 1, #manifest.packages do
    local package = manifest.packages[i]
    if package.id == id then
      local out = {}
      for j = 1, #package.files do
        out[#out + 1] = package.files[j].path
      end
      return out
    end
  end
  error("manifest 的 packages 段没有包 " .. id, 0)
end

local function source_text(rel)
  local full = PKG_ROOT .. "/" .. rel
  local handle = assert(io.open(full, "rb"), "读不到 " .. full)
  local text = handle:read("a")
  handle:close()
  return text
end

--- 去掉行注释与**双引号字符串**后的源码。
---
--- 方法的局限一并声明（包内纪律）：不支持长括号注释 `--[[ ]]` 与单引号字符串——
--- 包源码禁用这两种写法，所以它们缺席不会让扫描漏判；真要写进来，本函数会把它们
--- 当普通字符留下（偏保守：宁可多报，不可漏报）。
local function strip(text)
  local out = {}
  local i = 1
  local n = #text
  while i <= n do
    local c = text:sub(i, i)
    if c == '"' then
      i = i + 1
      while i <= n do
        local ch = text:sub(i, i)
        if ch == "\\" then
          i = i + 2
        elseif ch == '"' then
          i = i + 1
          break
        else
          i = i + 1
        end
      end
    elseif c == "-" and text:sub(i + 1, i + 1) == "-" then
      while i <= n and text:sub(i, i) ~= "\n" do
        i = i + 1
      end
    else
      out[#out + 1] = c
      i = i + 1
    end
  end
  return table.concat(out)
end

-- ── 用例：静态面（零浮点 / 无标准库表 / 不复制 PRNG）──────────────────────────

run("pkg.assetgen.no-float-static", function()
  local paths = pkg_sources("assetgen")
  eq(#paths, 9, "manifest 里 assetgen 源文件数（加模块要同步抬这个数）")
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
local function samples_of(product)
  local out = {}
  local pcm = product.pcm
  local stride = product.ch * 2
  for j = 0, product.frames - 1 do
    local lo = pcm:byte(j * stride + 1)
    local hi = pcm:byte(j * stride + 2)
    local v = lo | (hi << 8)
    if v >= 32768 then
      v = v - 65536
    end
    out[j + 1] = v
  end
  return out
end

local OSC = { algo = "sound.osc", params = { wave = "sine", freq = 100, ms = 50, sr = 8000, amp = 800 } }

run("pkg.assetgen.sound-same-seed-same-bytes", function()
  local assetgen = fresh()
  local first = assetgen.sound(OSC, "sound-seed")
  local second = assetgen.sound(OSC, "sound-seed")
  eq(#first.bytes, #second.bytes, "同 seed 两次 WAV 长度")
  is_true(first.bytes == second.bytes, "同 seed 两次的字节必须逐字节相等")
  eq(first.frames, 400, "frames = ms*sr//1000")
  eq(#first.pcm, 400 * 2, "#pcm = frames*ch*2")
  eq(#first.bytes, 44 + #first.pcm, "WAV = 44 字节头 + pcm")
  -- 纯振荡器不消耗随机数（参数完全决定波形），所以 seed 差异必须用**消耗流**的
  -- 算法来验：白噪。
  local noisy = { algo = "sound.noise", params = { ms = 50, sr = 8000, amp = 800 } }
  local third = assetgen.sound(noisy, "sound-seed-2")
  local fourth = assetgen.sound(noisy, "sound-seed")
  is_true(third.bytes ~= fourth.bytes, "异 seed 的噪声字节必须不同")
  eq(#third.bytes, #fourth.bytes, "异 seed 的噪声长度必须相同")
end)

run("pkg.assetgen.sound-peak-cap", function()
  local assetgen = fresh()
  local product = assetgen.sound({ algo = "sound.noise", params = { ms = 40, sr = 8000, amp = 700 } }, "peak")
  eq(product.peak, 700, "peak 是入参 amp 的千分比")
  local limit = (product.peak * 32767) // 1000
  local samples = samples_of(product)
  local low = 0
  for i = 1, #samples do
    local v = samples[i]
    local size = v < 0 and -v or v
    is_true(size <= limit, "第 " .. i .. " 个样本 " .. v .. " 超过 peak 上限 " .. limit)
    if size > low then
      low = size
    end
  end
  -- 归一必须真的用满幅：只断「不超上限」对一个全静音的实现恒真。
  is_true(low >= limit - 64, "最大样本 " .. low .. " 必须接近上限 " .. limit)
  local quiet = assetgen.sound({ algo = "sound.silence", params = { ms = 10, sr = 8000 } }, "peak")
  eq(quiet.peak, 800, "silence 的 peak 仍是 amp（全零时约定如此）")
  for i = 1, #samples_of(quiet) do
    eq(samples_of(quiet)[i], 0, "silence 必须逐样本为 0")
  end
end)

run("pkg.assetgen.sound-waveforms", function()
  local assetgen = fresh()
  local flat = { ms = 100, sr = 8000, amp = 1000, env = "none" }

  -- 方波：只出现 ±amp 两类值。
  local square = assetgen.sound({
    algo = "sound.osc",
    params = { wave = "square", freq = 100, ms = flat.ms, sr = flat.sr, amp = flat.amp, env = flat.env },
  }, "wave")
  local seen = {}
  local distinct = 0
  for _, v in ipairs(samples_of(square)) do
    if seen[v] == nil then
      seen[v] = true
      distinct = distinct + 1
    end
  end
  eq(distinct, 2, "方波只允许两类取值")

  -- 锯齿：相邻差为正，除了回绕点（每周期一次，容许 ±1）。
  local saw = samples_of(assetgen.sound({
    algo = "sound.osc",
    params = { wave = "saw", freq = 100, ms = flat.ms, sr = flat.sr, amp = flat.amp, env = flat.env },
  }, "wave"))
  local wraps = 0
  for i = 2, #saw do
    if saw[i] <= saw[i - 1] then
      wraps = wraps + 1
    end
  end
  local cycles = (100 * flat.ms) // 1000
  is_true(wraps >= cycles - 1 and wraps <= cycles + 1, "锯齿回绕次数应约为 " .. cycles .. "，实得 " .. wraps)

  -- 正弦：零交叉数约为 2 × 周期数。
  local sine = samples_of(assetgen.sound({
    algo = "sound.osc",
    params = { wave = "sine", freq = 100, ms = flat.ms, sr = flat.sr, amp = flat.amp, env = flat.env },
  }, "wave"))
  local crossings = 0
  for i = 2, #sine do
    if (sine[i - 1] < 0 and sine[i] >= 0) or (sine[i - 1] >= 0 and sine[i] < 0) then
      crossings = crossings + 1
    end
  end
  is_true(crossings >= 2 * cycles - 2 and crossings <= 2 * cycles + 2,
    "正弦零交叉应约为 " .. (2 * cycles) .. "，实得 " .. crossings)
  is_true(assetgen.sound(OSC, "wave").bytes ~= assetgen.sound({ algo = "sound.osc", params = { wave = "square", freq = 100, ms = 50, sr = 8000, amp = 800 } }, "wave").bytes,
    "不同波形的字节必须不同")
end)

-- ── 用例：WAV 头 / 包络 / 分层 / 音频硬帽 ──────────────────────────────────

local function u16le(text, at)
  return text:byte(at) | (text:byte(at + 1) << 8)
end

local function u32le(text, at)
  return text:byte(at)
    | (text:byte(at + 1) << 8)
    | (text:byte(at + 2) << 16)
    | (text:byte(at + 3) << 24)
end

run("pkg.assetgen.sound-wav-header", function()
  local assetgen = fresh()
  local product = assetgen.sound({ algo = "sound.osc", params = { wave = "square", freq = 440, ms = 25, sr = 44100, ch = 2, amp = 900 } }, "wav")
  local text = product.bytes
  eq(text:sub(1, 4), "RIFF", "RIFF 魔数")
  eq(text:sub(9, 12), "WAVE", "WAVE 魔数")
  eq(text:sub(13, 16), "fmt ", "fmt 块")
  eq(text:sub(37, 40), "data", "data 块")
  eq(u32le(text, 17), 16, "fmt 块长度")
  eq(u16le(text, 21), 1, "formatTag 必须是 PCM(1)")
  eq(u16le(text, 23), 2, "声道数")
  eq(u32le(text, 25), 44100, "采样率")
  eq(u32le(text, 29), 44100 * 2 * 2, "byteRate = sr*ch*2")
  eq(u16le(text, 33), 2 * 2, "blockAlign = ch*2")
  eq(u16le(text, 35), 16, "bitsPerSample")
  eq(u32le(text, 41), #product.pcm, "data 块长度 = #pcm")
  eq(u32le(text, 5), 36 + #product.pcm, "riffSize = 36 + dataSize")
  eq(#text, 44 + #product.pcm, "文件总长 = 44 + pcm")

  -- s16 装箱的三个锚点：`>>` 是逻辑右移，`//` 会算错负数的字节。
  local bytes = require("pkg.assetgen.bytes")
  eq(bytes.s16le(-1), "\255\255", "s16(-1) 低/高字节都是 0xFF")
  eq(bytes.s16le(-32768), "\0\128", "s16(-32768) = 0x00 0x80")
  eq(bytes.s16le(32767), "\255\127", "s16(32767) = 0xFF 0x7F")
end)

--- 一段样本的均方根。测试跑在普通 Lua 上，`math` 表可用，直接用 `math.sqrt` 取整。
local function segment_rms(samples, from, to)
  local sum = 0
  for i = from, to do
    local v = samples[i]
    sum = sum + v * v
  end
  local count = to - from + 1
  if count <= 0 then
    return 0
  end
  return math.floor(math.sqrt(sum // count))
end

run("pkg.assetgen.sound-envelope-decay", function()
  local assetgen = fresh()
  local product = assetgen.sound({
    algo = "sound.noise",
    params = { ms = 200, sr = 8000, amp = 900, attack = 10, env = "exp" },
  }, "env")
  local samples = samples_of(product)
  local n = #samples
  local quarter = n // 4
  local head = segment_rms(samples, 1, quarter)
  local tail = segment_rms(samples, n - quarter + 1, n)
  is_true(head > tail, "前 1/4 RMS（" .. head .. "）必须大于后 1/4（" .. tail .. "）")

  -- 最后一个 64 样本窗口的能量必须远低于最响的窗口（1/8 以上算没衰减）。
  local best = 0
  for start = 1, n - 63, 64 do
    local energy = segment_rms(samples, start, start + 63)
    if energy > best then
      best = energy
    end
  end
  local last = segment_rms(samples, n - 63, n)
  is_true(best > 0, "必须存在非零窗口")
  is_true(last * 8 < best, "末窗口能量 " .. last .. " 必须小于峰值窗口 " .. best .. " 的 1/8")

  -- `env = "none"` 是恒等增益：全程都不衰减。
  local flat = assetgen.sound({
    algo = "sound.osc",
    params = { wave = "sine", freq = 100, ms = 100, sr = 8000, amp = 1000, env = "none" },
  }, "env")
  local flat_samples = samples_of(flat)
  local flat_head = segment_rms(flat_samples, 1, 100)
  local flat_tail = segment_rms(flat_samples, #flat_samples - 99, #flat_samples)
  is_true(flat_head * 10 > flat_tail * 9 and flat_tail * 10 > flat_head * 9, "env=none 的 RMS 应基本持平")
end)

run("pkg.assetgen.sound-mix-sum", function()
  local assetgen = fresh()
  local function tone(freq)
    return assetgen.sound({
      algo = "sound.osc",
      params = { wave = "sine", freq = freq, ms = 50, sr = 8000, amp = 400, env = "none" },
    }, "mix")
  end
  local low = tone(100)
  local high = tone(200)
  local mixed = assetgen.sound({
    algo = "sound.mix",
    params = {
      layers = {
        { product = low, at = 0 },
        { product = high, at = 25 },
      },
    },
  }, "mix")
  eq(mixed.frames, 400 + 200, "mix 的帧数 = 最长层的末端")
  eq(#mixed.pcm, mixed.frames * 2, "#pcm = frames*ch*2")

  local a = samples_of(low)
  local b = samples_of(high)
  local out = samples_of(mixed)
  for i = 1, 200 do
    eq(out[i], a[i], "偏移之前只应有第一层（第 " .. i .. " 个样本）")
  end
  for i = 201, 400 do
    eq(out[i], a[i] + b[i - 200], "重叠区必须是整数和（第 " .. i .. " 个样本）")
  end
  for i = 401, 600 do
    eq(out[i], b[i - 200], "第一层结束后只剩第二层（第 " .. i .. " 个样本）")
  end
  is_true(mixed.peak > 0, "mix 的 peak 必须非零")
end)

run("pkg.assetgen.sound-caps", function()
  local assetgen = fresh()
  expect_error("YGG-PKG-ASSETGEN-CAP", function()
    assetgen.sound({ algo = "sound.osc", params = { wave = "sine", freq = 100, ms = 9000, sr = 48000 } }, "caps")
  end, "帧数超帽")
  expect_error("YGG-PKG-ASSETGEN-PARAM", function()
    assetgen.sound({ algo = "sound.osc", params = { wave = "sine", freq = 100, ms = 50, sr = 44101 } }, "caps")
  end, "sr 不在白名单")
  expect_error("YGG-PKG-ASSETGEN-PARAM", function()
    assetgen.sound({ algo = "sound.osc", params = { wave = "sine", freq = 100, ms = 0, sr = 8000 } }, "caps")
  end, "ms=0")
  expect_error("YGG-PKG-ASSETGEN-PARAM", function()
    assetgen.sound({ algo = "sound.osc", params = { wave = "nope", freq = 100, ms = 50, sr = 8000 } }, "caps")
  end, "未知波形")
  expect_error("YGG-PKG-ASSETGEN-PARAM", function()
    assetgen.sound({ algo = "sound.mix", params = { layers = {} } }, "caps")
  end, "空 layers")
end)

-- ── 用例：bundle 顺序与键 / 流隔离 ──────────────────────────────────────────

local BUNDLE_ENTRIES = {
  { key = "grass", kind = "texture", algo = "texture.checker", params = { w = 8, h = 8, ch = 1, size = 2 } },
  { key = "crate", kind = "mesh", algo = "mesh.box", params = { sx = 600, sy = 600, sz = 600 } },
  { key = "blip", kind = "sound", algo = "sound.osc", params = { wave = "square", freq = 300, ms = 20, sr = 8000 } },
}

run("pkg.assetgen.bundle-order-and-keys", function()
  local assetgen = fresh()
  local bundle = assetgen.bundle(BUNDLE_ENTRIES, "bundle-seed")
  eq(#bundle.items, 3, "items 数量")
  for i = 1, #BUNDLE_ENTRIES do
    eq(bundle.items[i].key, BUNDLE_ENTRIES[i].key, "items 顺序必须等于入参顺序（第 " .. i .. " 项）")
    eq(bundle.items[i].kind, BUNDLE_ENTRIES[i].kind, "items kind（第 " .. i .. " 项）")
    eq(bundle.items[i].product.algo, BUNDLE_ENTRIES[i].algo, "items algo（第 " .. i .. " 项）")
  end
  eq(bundle.seed_text, "bundle-seed", "bundle.seed_text")
  eq(bundle.v, 1, "bundle.v 是单一常量")
  eq(bundle.canon, assetgen.canon(bundle), "bundle 的 canon 与 canon(bundle) 一致")
  is_true(bundle.items[1].product.stream == "assetgen/grass", "缺省流名 = assetgen/<key>")

  local function bundle_error(code, entries, label)
    expect_error(code, function()
      assetgen.bundle(entries, "bundle-seed")
    end, label)
  end
  bundle_error("YGG-PKG-ASSETGEN-PARAM", {
    BUNDLE_ENTRIES[1],
    { key = "grass", kind = "mesh", algo = "mesh.box", params = {} },
  }, "重复 key")
  bundle_error("YGG-PKG-ASSETGEN-PARAM", {
    { key = "", kind = "texture", algo = "texture.checker", params = { w = 4, h = 4 } },
  }, "空 key")
  bundle_error("YGG-PKG-ASSETGEN-KIND", {
    { key = "x", kind = "video", algo = "texture.checker", params = { w = 4, h = 4 } },
  }, "kind 不在三值")
  bundle_error("YGG-PKG-ASSETGEN-PARAM", {}, "空 entries")
end)

run("pkg.assetgen.stream-isolation", function()
  local assetgen = fresh()
  local texture_spec = { algo = "texture.fbm", params = { w = 16, h = 16, ch = 3, cell = 4, octaves = 2 } }
  local mesh_spec = { algo = "mesh.blob", params = { radius = 500, segs = 6, rings = 4, amp = 300 } }
  local sound_spec = { algo = "sound.noise", params = { ms = 30, sr = 8000 } }

  -- 先用后生成音频，再反过来：三条流各自的状态由 (seed, stream) 派生，顺序不得影响。
  local tex_a = assetgen.texture(texture_spec, "iso-seed")
  local mesh_a = assetgen.mesh(mesh_spec, "iso-seed")
  local snd_a = assetgen.sound(sound_spec, "iso-seed")
  local snd_b = assetgen.sound(sound_spec, "iso-seed")
  local mesh_b = assetgen.mesh(mesh_spec, "iso-seed")
  local tex_b = assetgen.texture(texture_spec, "iso-seed")

  is_true(tex_a.bytes == tex_b.bytes, "纹理字节不受生成顺序影响")
  is_true(mesh_a.bytes == mesh_b.bytes, "网格字节不受生成顺序影响")
  is_true(snd_a.bytes == snd_b.bytes, "音频字节不受生成顺序影响")

  -- 同一 (seed, stream) 显式共享时序列才共享；换类别的缺省流名互不相同。
  local streams = assetgen.streams("iso-seed")
  eq(#assetgen.algo_list("texture"), 7, "texture 算法数")
  eq(#assetgen.algo_list("mesh"), 6, "mesh 算法数")
  eq(#assetgen.algo_list("sound"), 6, "sound 算法数")
  local names = assetgen.algo_list("mesh")
  for i = 2, #names do
    is_true(names[i - 1] < names[i], "algo_list 必须字节序升序：" .. names[i - 1] .. " < " .. names[i])
  end
  eq(type(streams.texture), "table", "streams 返回三条句柄")
  eq(streams.texture:int(0, 0), 0, "句柄可用（单面区间不取数）")
  expect_error("YGG-PKG-ASSETGEN-KIND", function()
    assetgen.algo_list("video")
  end, "未知 kind")
end)
