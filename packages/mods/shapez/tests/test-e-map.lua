-- `tests/test-e-map.lua` —— v2 分块地图 procgen（map_chunk.js 等价面）。
--
-- 覆盖：整数距离 KAT；种子/版本守护（first-wins）；预定义五斑钉值；
-- 黄金 chunk KAT（自建钉值）；同 seed 双局逐值相等 + 跨 seed 不等（否定断言，
-- 方法=双 seed 各 3×3 chunk 全量 codec.encode 比对）；patch_at 内/外/重叠三态；
-- 负坐标 floor 分块（禁 %）；生成不 bump layout_v；KV 缓存命中；patch 序 =
-- 着色优先序；PERF shapez.chunkgen_ns。

local mapgen = require("src.mapgen")
local codec = require("pkg.codec")

local GOLDEN_SEED = "s26-v2-golden"

--- 开一局黄金种子局。
local function golden_game()
  reset_all()
  ygg_dispatch("event.start", GOLDEN_SEED)
end

--- 3×3 chunk 全量编码拼接（否定/肯定断言的统一比对面）。
local function area_encode(c0x, c0y)
  local out = {}
  for cy = c0y, c0y + 2 do
    for cx = c0x, c0x + 2 do
      out[#out + 1] = cx .. "," .. cy .. "=" .. codec.encode({ v = mapgen.MAPGEN_V, patches = mapgen.ensure_chunk(cx, cy) })
    end
  end
  return table.concat(out, "\n")
end

check("mapgen.chunk-dist-kat", function()
  -- round(hypot(cx+0.5, cy+0.5)) 的整数域同构钉值
  assert_eq(mapgen.chunk_dist(0, 0), 1, "chunk(0,0)")
  assert_eq(mapgen.chunk_dist(1, 0), 2, "chunk(1,0)")
  assert_eq(mapgen.chunk_dist(2, 0), 3, "chunk(2,0)")
  assert_eq(mapgen.chunk_dist(3, 0), 4, "chunk(3,0)")
  assert_eq(mapgen.chunk_dist(5, -2), 6, "chunk(5,-2)")
  assert_eq(mapgen.chunk_dist(12, 0), 13, "chunk(12,0)")
  assert_eq(mapgen.chunk_dist(-1, 0), 1, "chunk(-1,0)")
  assert_eq(mapgen.chunk_dist(-3, -3), 4, "chunk(-3,-3)")
end)

check("mapgen.seed-version-firstwins", function()
  reset_all()
  ygg_dispatch("event.start", GOLDEN_SEED)
  assert_eq(ygg.kv.get("sz.seed"), GOLDEN_SEED, "载荷种子落 KV")
  assert_eq(ygg.kv.get("sz.mapgen_v"), mapgen.MAPGEN_V, "生成器版本落 KV")
  -- first-wins：再开局不带种子不改写（读档局以档回写为准的同构面）
  ygg_dispatch("event.start", "")
  assert_eq(ygg.kv.get("sz.seed"), GOLDEN_SEED, "空载荷不夺种子")
  ygg_dispatch("event.start", "another")
  assert_eq(ygg.kv.get("sz.seed"), GOLDEN_SEED, "异载荷不夺种子")
end)

check("mapgen.predefined-five", function()
  golden_game()
  -- 五块预定义斑（源 generatePredefined）：钉子形/色/锚位；(-1,-1) 绿斑不钉位。
  local five = {
    [{ 0, 0 }] = { subshape = "C", color = "r", px = 7, py = 7 },
    [{ -1, 0 }] = { subshape = "C", color = "u", px = 7, py = 7 },
    [{ 0, -1 }] = { subshape = "R", color = "u", px = 5, py = 9 },
    [{ -1, -1 }] = { subshape = "C", color = "g" },
    [{ 5, -2 }] = { subshape = "S", color = "u", px = 5, py = 9 },
  }
  for coords, want in pairs(five) do
    local patches = mapgen.ensure_chunk(coords[1], coords[2])
    assert_eq(#patches, 1, "chunk(" .. coords[1] .. "," .. coords[2] .. ") 恰一斑")
    local p = patches[1]
    assert_true(p ~= nil, "斑记录在场")
    assert_eq(p.subshape, want.subshape, "子形")
    assert_eq(p.color, want.color, "颜色")
    if want.px ~= nil then
      assert_eq(p.x, want.px, "钉位 x")
      assert_eq(p.y, want.py, "钉位 y")
    else
      assert_true(p.x >= 0 and p.x < 16 and p.y >= 0 and p.y < 16, "随机落点在块内")
    end
    assert_eq(#p.circles, 3, "size=2 → 3 圆（源 i=0..size 含端点）")
  end
end)

check("mapgen.golden-kat", function()
  golden_game()
  -- 黄金钉值（自建）：seed="s26-v2-golden" 的 chunk(0,0) 整数化产物逐字节。
  local c00 = mapgen.ensure_chunk(0, 0)
  assert_eq(
    codec.encode({ v = mapgen.MAPGEN_V, patches = c00 }),
    '{"patches":[{"circles":[[10,9,1,2],[9,6,3,3],[5,7,3,1]],"color":"r","subshape":"C","x":7,"y":7}],"v":1}',
    "chunk(0,0) 黄金编码"
  )
end)

check("mapgen.same-seed-two-games-equal", function()
  golden_game()
  local a = area_encode(-1, -1)
  golden_game()
  local b = area_encode(-1, -1)
  assert_eq(b, a, "同 seed 双局 3×3 chunk 全量编码逐字节相等（I1）")
end)

check("mapgen.cross-seed-unequal", function()
  -- 否定断言（方法=双 seed 各 3×3 chunk 全量 codec.encode 比对）
  golden_game()
  local a = area_encode(-1, -1)
  reset_all()
  ygg_dispatch("event.start", "s26-b")
  local b = area_encode(-1, -1)
  assert_true(a ~= b, "跨 seed 3×3 编码不等（种子唯一真源）")
end)

check("mapgen.patch-at-three-states", function()
  golden_game()
  -- 内：Cu 斑 circles[1] 圆心格（白盒派生；圆心恒命中）
  local cu = mapgen.ensure_chunk(-1, 0)[1]
  local c = cu.circles[1]
  local wx, wy = -16 + c[1], c[2]
  local inside = mapgen.patch_at(wx, wy)
  assert_true(inside ~= nil, "圆心格命中")
  assert_eq(inside.subshape .. inside.color, "Cu", "命中 Cu 斑")
  -- 外：空斑块 (-3,0)（黄金种子实测 0 斑）任取格必不中
  assert_eq(#mapgen.ensure_chunk(-3, 0), 0, "(-3,0) 空斑块（钉值前提）")
  assert_true(mapgen.patch_at(-48 + 5, 5) == nil, "空斑块内不中")
  -- 重叠：chunk(-3,-3) 白盒钉值格，p1(Cg) 与 p2(Ru) 圆域相交 → 首斑胜
  local two = mapgen.ensure_chunk(-3, -3)
  assert_eq(#two, 2, "(-3,-3) 恰两斑（钉值前提）")
  assert_true(two[1] ~= nil and two[2] ~= nil, "两斑记录在场")
  local overlap = mapgen.patch_at(-48 + 9, -48 + 10)
  assert_true(overlap ~= nil, "重叠格命中")
  assert_eq(overlap.subshape .. overlap.color, "Cg", "先到先得：色斑（先生成序）胜形斑")
end)

check("mapgen.negative-chunk-floor-div", function()
  golden_game()
  -- 负世界坐标 floor 分块：(-9,7) → chunk(-1,0) 局部(7,7)（Cu 锚位）
  local p = mapgen.patch_at(-9, 7)
  assert_true(p ~= nil, "(-9,7) 命中")
  assert_eq(p.subshape .. p.color, "Cu", "Cu 斑")
  -- 深负坐标（-25,-1）→ chunk(-2,-1)：patch_at 与白盒手扫同一圆表逐值一致
  local deep = mapgen.patch_at(-25, -1)
  local patches = mapgen.ensure_chunk(-2, -1)
  local lx, ly = -25 - (-2) * 16, -1 - (-1) * 16
  assert_eq(lx, 7, "局部 x ∈ [0,15]（floor 除，非截断）")
  local manual = nil
  for i = 1, #patches do
    local pp = patches[i]
    for k = 1, #pp.circles do
      local cc = pp.circles[k]
      local dx, dy = lx - cc[1], ly - cc[2]
      local a, b = dx * cc[4], dy * cc[3]
      if a * a + b * b <= cc[3] * cc[3] * cc[4] * cc[4] then
        manual = pp
        break
      end
    end
    if manual ~= nil then
      break
    end
  end
  assert_eq(deep, manual, "深负坐标与白盒手扫一致（禁 % 的 floor 语义）")
end)

check("mapgen.layout-v-untouched", function()
  golden_game()
  for cx = -2, 2 do
    for cy = -2, 2 do
      mapgen.ensure_chunk(cx, cy)
      mapgen.patch_at(cx * 16 + 3, cy * 16 + 3)
    end
  end
  assert_true(ygg.kv.get("sz.layout_v") == nil, "生成/查询零 layout bump（地图与带网正交）")
end)

check("mapgen.cache-kv-hit", function()
  golden_game()
  -- 缓存哨兵：KV 已有同版本产物时不重生成（读档局惰性重生成前的日常路径）
  ygg.kv.set("sz.chunk.50.50", codec.encode({ v = mapgen.MAPGEN_V, patches = { "sentinel" } }))
  local patches = mapgen.ensure_chunk(50, 50)
  assert_eq(patches[1], "sentinel", "KV 缓存命中（不重生成）")
  -- 版本不符视同未中：重生成覆盖
  ygg.kv.set("sz.chunk.51.51", codec.encode({ v = mapgen.MAPGEN_V + 9, patches = { "stale" } }))
  local fresh = mapgen.ensure_chunk(51, 51)
  assert_true(fresh[1] ~= "stale", "旧版本缓存作废重生成")
end)

check("mapgen.color-first-patch-order", function()
  golden_game()
  -- patch 序 = 着色优先序：源先色斑后形斑；2 斑块首斑必非未着色形斑
  local two_patch = 0
  for cy = -1, 1 do
    for cx = -1, 1 do
      local patches = mapgen.ensure_chunk(cx, cy)
      if #patches >= 2 then
        two_patch = two_patch + 1
        assert_true(patches[1].color ~= "u", "首斑是色斑（先色后形）")
        assert_eq(patches[2].color, "u", "次斑是形斑")
      end
    end
  end
  assert_true(two_patch >= 1, "3×3 内存在 2 斑块（钉值前提，实得 " .. two_patch .. "）")
end)

check("mapgen.perf-chunkgen", function()
  golden_game()
  local clock_ok, clock = pcall(function()
    return os.clock
  end)
  if not clock_ok then
    return "os 不可用，跳过 PERF（selftest 普通 Lua 必有）"
  end
  -- 预热 3 块（def 注册 + 编码路径）
  for i = 1, 3 do
    mapgen.ensure_chunk(100, i)
  end
  local t0 = clock()
  for i = 1, 20 do
    mapgen.ensure_chunk(101, i)
  end
  local per_chunk_ns = (clock() - t0) / 20 * 1e9
  print(string.format("PERF shapez.chunkgen_ns=%.0f （单 chunk 生成+encode，预算 ≤1ms=1e6ns）", per_chunk_ns))
  assert_true(per_chunk_ns < 1e6, "单 chunk ≤1ms（§1.7 性能预算）")
end)
