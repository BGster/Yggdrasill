-- `src/mapgen.lua` —— 分块地图 procgen（源 map_chunk.js 的 v2 等价面）。
--
-- # 契约定位（openwiki/contracts/mod/shapez-game.md §2.5）
--
-- 纯 Lua over `pkg.procgen` 包面（包面零改）：每 chunk 一条命名流
-- `chunk.<cx>.<cy>`，`procgen.run` 以 `sz.seed` 为种子——流隔离（procgen I2）
-- 即分块隔离，与源 `x.."|"..y.."|"..seed` 同构但走包面正道。
--
-- 数据形状：patch = `{x, y, subshape, color, circles}`，circles 元素 =
-- `{cx, cy, rx, ry}`（chunk 本地坐标的圆心 + 整数半轴；R1' 源浮点椭圆
-- scale 0.9..1.1 弃用 → rx/ry 各自 ±1 整数抖动，黄金值自建）。
-- 命中判据（整数域）：`(dx·ry)² + (dy·rx)² ≤ (rx·ry)²`。
--
-- 着色优先序（源先色斑后形斑、先到先得）：patch 表保生成序，`patch_at`
-- 按序扫「patch 序 × 圆序」首中即胜——与源 lowerLayer 首漆者赢的点查等价。
--
-- 产物落 KV `sz.chunk.<cx>.<cy>`（codec.encode 的 `{v, patches}`）；存档
-- 不点名 chunk 键（seed 唯真源，读档后 patch_at 惰性重生成，I1 保逐值）。
-- 负坐标分块用 floor 除 `//`（与 JS Math.floor 同语义）；**全程禁 `%`**
-- （R2'：JS/Lua 负余差异）。
--
-- 沙箱方言：模块顶层纯（无 math/string/table 表）；ygg 经 host() 单点取。

local procgen = require("pkg.procgen")
local codec = require("pkg.codec")

local M = { VERSION = "1.0.0" }

--- 生成器契约版本：进存档（`sz.mapgen_v`），不符即拒（R5'）。
M.MAPGEN_V = 1

--- chunk 边长（源 globalConfig.mapChunkSize）。
M.CHUNK = 16

local ALGO_NAME = "shapez-chunk"
local SEED_KEY = "sz.seed"
local VERSION_KEY = "sz.mapgen_v"
local DEFAULT_SEED = "shapez-v2" -- R6'：禁缺省熵；缺省也是钉死的常量串

local function host()
  return ygg
end

local function fail(code, detail)
  error("YGG-MOD-MAP-" .. code .. ": " .. detail, 0)
end

local function clamp(value, lo, hi)
  if value < lo then
    return lo
  elseif value > hi then
    return hi
  end
  return value
end

--- 整数平方根（牛顿迭代；沙箱无 math.sqrt，全整数域）。
local function isqrt(n)
  if n <= 1 then
    return n
  end
  local x = n
  local y = (x + 1) // 2
  while y < x do
    x = y
    y = (y + n // y) // 2
  end
  return x
end

--- 距原点 chunk 距离：`round(hypot(cx+0.5, cy+0.5))` 的整数域同构。
--- n = cx²+cx+cy²+cy（= 中心和 −1/2）；dist = ⌊(√(4n+2)+1)/2⌋ = (isqrt(4n+2)+1)//2。
function M.chunk_dist(cx, cy)
  local n = cx * cx + cx + cy * cy + cy
  local s = isqrt(4 * n + 2)
  return (s + 1) // 2
end

-- ============================================================================
-- 预定义斑（源 generatePredefined：五块撑 level 1/2/3 目标可达）
-- 位置 = chunk 本地坐标；px/py 缺席 = 不钉位（源 (-1,-1) 绿斑随机落点同构）。
-- ============================================================================

local PREDEFINED = {
  ["0|0"] = { subshape = "C", color = "r", px = 7, py = 7 },
  ["-1|0"] = { subshape = "C", color = "u", px = 7, py = 7 }, -- 源 CHUNK-9=7
  ["0|-1"] = { subshape = "R", color = "u", px = 5, py = 9 }, -- 源 CHUNK-7=9
  ["-1|-1"] = { subshape = "C", color = "g", px = nil, py = nil },
  ["5|-2"] = { subshape = "S", color = "u", px = 5, py = 9 },
}

-- ============================================================================
-- 斑生成（源 internalGeneratePatch 的整数化）
-- ============================================================================

--- 生成一个斑：`size` 个圆（源 i = 0..size 含端点），圆参数抽取序固定：
--- rx 抖动 → ry 抖动 → 圆心 x 偏移 → 圆心 y 偏移（KAT 钉序）。
--- `ox/oy` 非 nil 时钉位（预定义斑），不取位置数。
local function gen_patch(rng, size, subshape, color, ox, oy)
  local border = (size + 1) // 2 + 3 -- 源 ceil(size/2 + 3) 的整数同构
  local px = ox
  local py = oy
  if px == nil then
    px = rng:int(border, M.CHUNK - border - 1)
  end
  if py == nil then
    py = rng:int(border, M.CHUNK - border - 1)
  end
  local circles = {}
  for i = 0, size do
    local radius = 1 + i
    if radius > size then
      radius = size
    end
    local off = (size - i) // 2 + 2
    local rx = radius + rng:int(-1, 1)
    if rx < 1 then
      rx = 1
    end
    local ry = radius + rng:int(-1, 1)
    if ry < 1 then
      ry = 1
    end
    local ccx = px + rng:int(-off, off)
    local ccy = py + rng:int(-off, off)
    circles[#circles + 1] = { ccx, ccy, rx, ry }
  end
  return { x = px, y = py, subshape = subshape, color = color, circles = circles }
end

--- 色斑颜色：dist>2 时蓝加入池（源 availableColors 同构）；`rng:int` 池内抽。
local COLORS = { "r", "g", "b" }

local function color_pick(rng, dist)
  if dist > 2 then
    return COLORS[rng:int(1, 3)]
  end
  return COLORS[rng:int(1, 2)]
end

--- 形斑子形：dist<7 恒 rect（源初圈全 rect 同构，不取数）；否则显式权重数组
--- 喂 `rng:weighted`（R3'：pairs 无序 → 数组声明序即池序）。风车不成整斑
--- （源 windmillCount 守卫同构）→ 抽中即降级 rect。返回索引再映射，见调用处。
local SHAPE_LABELS = { "R", "C", "S", "W" }

local function shape_weights(dist)
  -- 源 round(50+clamp(d*2,0,50)) / round(20+clamp(d,0,30)) / round(6+clamp(d/2,0,20))
  return {
    100,
    50 + clamp(dist * 2, 0, 50),
    20 + clamp(dist, 0, 30),
    6 + clamp((dist + 1) // 2, 0, 20),
  }
end

local function shape_pick(rng, dist)
  if dist < 7 then
    return "R"
  end
  local picked = SHAPE_LABELS[rng:weighted(shape_weights(dist))]
  if picked == "W" then
    return "R"
  end
  return picked
end

-- ============================================================================
-- chunk 算法（经 procgen.run 调；产物 = { patches = {…} }）
-- ============================================================================

--- 单 chunk 生成。抽取序（KAT 钉序）：预定义斑短路；否则一次距离chance抽
--- （色斑）+ 一次 chance 抽（形斑），各自通过才生成。
local function chunk_algo(params, rng)
  local cx = params.cx
  local cy = params.cy
  local patches = {}
  local pre = PREDEFINED[cx .. "|" .. cy]
  if pre ~= nil then
    patches[1] = gen_patch(rng, 2, pre.subshape, pre.color, pre.px, pre.py)
    return { patches = patches }
  end
  local dist = M.chunk_dist(cx, cy)
  -- 源 chance = 0.9 − clamp(dist/25,0,1)·0.5，抽 `next() < chance/4`；
  -- 整数化为千分位：chance_milli = 900 − clamp(dist·20, 0, 500)，`rng:int(0,999)` 抽。
  local chance_milli = 900 - clamp(dist * 20, 0, 500)
  -- 源 size = max(2, round(1 + clamp(dist/8,0,4)))；(dist+4)//8 = round-half-up(dist/8)。
  local size = 1 + clamp((dist + 4) // 8, 0, 4)
  if size < 2 then
    size = 2
  end
  if rng:int(0, 999) < chance_milli then
    patches[#patches + 1] = gen_patch(rng, size, "C", color_pick(rng, dist), nil, nil)
  end
  if rng:int(0, 999) < chance_milli then
    patches[#patches + 1] = gen_patch(rng, size, shape_pick(rng, dist), "u", nil, nil)
  end
  return { patches = patches }
end

--- 注册面（simtick.setup 调）：装 procgen 自带算法 + 注册本 mod 的 chunk 算法。
--- 幂等（algo 同名覆盖；install 幂等）。algo 在此、def 在 ensure 时——chunk
--- 上下文（cx/cy）只能走 def.params（registry.run 只传 def.params 给算法），
--- 运行期 def 不跨存档（D4）无碍：chunk 产物落 KV，读档不靠 def。
function M.register()
  procgen.install()
  procgen.algo(ALGO_NAME, chunk_algo)
  return true
end

--- 种子守护（R6'）：`sz.seed` 显式；首局写入（载荷串或钉死缺省），随后
--- first-wins（读档局由存档回写的 seed 说话）。`sz.mapgen_v` 同拍落 KV。
function M.ensure_seed(seed)
  local h = host()
  local current = h.kv.get(SEED_KEY)
  if current ~= nil then
    return current
  end
  if type(seed) ~= "string" or #seed == 0 then
    seed = DEFAULT_SEED
  end
  h.kv.set(SEED_KEY, seed)
  h.kv.set(VERSION_KEY, M.MAPGEN_V)
  return seed
end

-- ============================================================================
-- 生成与查询（懒生成 + KV 缓存）
-- ============================================================================

local function generate(cx, cy)
  local h = host()
  local seed = h.kv.get(SEED_KEY)
  if seed == nil then
    fail("SEED", "sz.seed 未设（先 event.start）")
  end
  local def_key = ALGO_NAME .. "." .. cx .. "." .. cy
  procgen.define(def_key, { algo = ALGO_NAME, params = { cx = cx, cy = cy } })
  local product = procgen.run(def_key, seed, "chunk." .. cx .. "." .. cy)
  return product.patches
end

--- `ensure_chunk(cx, cy)` → patch 数组（先 KV 缓存，未中才生成并落 KV；
--- 缓存版本不符 `MAPGEN_V` 视同未中，重生成覆盖——生成器升级的会话内收敛）。
function M.ensure_chunk(cx, cy)
  local h = host()
  local key = "sz.chunk." .. cx .. "." .. cy
  local raw = h.kv.get(key)
  if type(raw) == "string" and #raw > 0 then
    local cached = codec.decode(raw)
    if type(cached) == "table" and cached.v == M.MAPGEN_V and type(cached.patches) == "table" then
      return cached.patches
    end
  end
  local patches = generate(cx, cy)
  h.kv.set(key, codec.encode({ v = M.MAPGEN_V, patches = patches }))
  return patches
end

--- `patch_at(wx, wy)` → patch | nil。负坐标 floor 分块（`//`），局部坐标
--- `wx − cx·16 ∈ [0,15]`（**禁 `%`**，R2'）。命中 = patch 序 × 圆序首中
--- （整数椭圆判据），先到先得复刻源重叠裁决。
function M.patch_at(wx, wy)
  local c = M.CHUNK
  local cx = wx // c
  local cy = wy // c
  local lx = wx - cx * c
  local ly = wy - cy * c
  local patches = M.ensure_chunk(cx, cy)
  for i = 1, #patches do
    local p = patches[i]
    local circles = p.circles
    for k = 1, #circles do
      local circle = circles[k]
      local dx = lx - circle[1]
      local dy = ly - circle[2]
      local rx = circle[3]
      local ry = circle[4]
      local a = dx * ry
      local b = dy * rx
      if a * a + b * b <= rx * rx * ry * ry then
        return p
      end
    end
  end
  return nil
end

return M
