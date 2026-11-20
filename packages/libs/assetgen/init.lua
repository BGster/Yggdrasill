-- `packages/libs/assetgen/init.lua` —— 包根模块 `pkg.assetgen`：导出面。
--
-- 契约：`docs/design/packages/assetgen.md`（§4 API 面 / §3 不变量）。
--
-- 顶层只有 `local` / `require` / 函数与常量定义（I7）：算法表的**组装**在
-- `M.install(deps)` 里做，装载期不写任何状态。未装时算法表为空，一切 generate
-- 报 `-ALGO`——空表比「半装」安全：半装会让一半素材可用、另一半静默失败。
--
-- # 与 procgen 的边界
--
-- 只吃它的 rng 面（`rng(seed, stream)` 句柄的 `:int` / `:next32`）。不调
-- `define` / `run` / `hash`：素材不是 procgen 的 def，也不借它的 FNV 当摘要。

local procgen = require("pkg.procgen")
local join = require("pkg.assetgen.join")
local spec = require("pkg.assetgen.spec")
local encode = require("pkg.assetgen.encode")
local texture_algos = require("pkg.assetgen.texture")
local mesh_algos = require("pkg.assetgen.mesh")
local sound_algos = require("pkg.assetgen.sound")
local raster = require("pkg.assetgen.raster")

local M = { VERSION = "0.1.0" }

--- 产物契约版本（I6）：字节形状一改就 +1，所有产物一起变。
M.V = 1

--- 错误码面（包内唯一一份住在 `spec.E`，这里做对外副本）。
M.ERRORS = {}
for key, code in pairs(spec.E) do
  M.ERRORS[key] = code
end

local E = spec.E
local fail = spec.fail

--- 三类产物的算法表（`kind → name → fn`）；`install` 是唯一的装入时点。
local ALGOS = { texture = {}, mesh = {}, sound = {} }

--- rng 来源：缺省是 procgen，`install({ procgen = ... })` 可换（测试替身 / 换实现）。
local FLOW = procgen

local function register(kind, source)
  local target = ALGOS[kind]
  for name, fn in pairs(source) do
    target[name] = fn
  end
end

-- ── canon ───────────────────────────────────────────────────────────────────

--- 单 token 转义：可打印 ASCII（空格与 `%` 除外）原样，其余写 `%XX`。
---
--- canon 必须**单行且无空格**（宿主的 `grep` / `cmp` 按行取证据），而 seed 是
--- 调用方给的任意字符串。转义不是摘要：可逆、无运算、不引入第二份哈希（I9）。
local function token(text)
  local out = {}
  for i = 1, #text do
    local b = text:byte(i)
    if b > 32 and b < 127 and b ~= 37 then
      out[i] = ("%c"):format(b)
    else
      out[i] = ("%%%02X"):format(b)
    end
  end
  return join.concat(out)
end

local function put(parts, key, value)
  local text = key .. "=" .. tostring(value)
  if #parts == 0 then
    parts[1] = text
  else
    parts[#parts + 1] = " " .. text
  end
end

--- `canon(product)` → 单行 ASCII 描述串；与 `product.canon` 逐字符相等。
---
--- 它是**变更检测**载体而不是摘要：只写形状字段与长度，不做任何压缩或散列。
function M.canon(product)
  if type(product) ~= "table" then
    fail(E.PARAM, "canon 需要 product 表，得到 " .. type(product))
  end
  local kind = product.kind
  local parts = {}
  put(parts, "kind", kind)
  if product.algo ~= nil then
    put(parts, "algo", product.algo)
  end
  put(parts, "v", product.v)
  if product.seed ~= nil then
    put(parts, "seed", token(tostring(product.seed)))
  end
  if product.stream ~= nil then
    put(parts, "stream", token(tostring(product.stream)))
  end
  local length = 0
  if kind == "texture" then
    put(parts, "w", product.w)
    put(parts, "h", product.h)
    put(parts, "ch", product.ch)
    length = #product.bytes
  elseif kind == "mesh" then
    local size = product.size
    put(parts, "verts", #product.verts // 3)
    put(parts, "faces", #product.faces // 3)
    put(parts, "size", size[1] .. "x" .. size[2] .. "x" .. size[3])
    length = #product.bytes
  elseif kind == "sound" then
    put(parts, "sr", product.sr)
    put(parts, "ch", product.ch)
    put(parts, "frames", product.frames)
    put(parts, "peak", product.peak)
    length = #product.bytes
  elseif kind == "bundle" then
    local items = product.items
    put(parts, "items", #items)
    for i = 1, #items do
      length = length + #items[i].product.bytes
    end
  else
    fail(E.KIND, "canon 的 kind 必须是 texture / mesh / sound / bundle，得到 " .. tostring(kind))
  end
  put(parts, "bytes", length)
  return join.concat(parts)
end

-- ── 生成 ────────────────────────────────────────────────────────────────────

--- 装配产物表的公共字段；网格 / 音频的 canonical 字节在这里装箱。
local function finish(kind, algo_name, product, seed, stream)
  product.kind = kind
  product.algo = algo_name
  product.v = M.V
  product.seed = tostring(seed)
  product.stream = stream
  if kind == "mesh" then
    product.bytes = encode.obj(product)
  elseif kind == "sound" then
    product.bytes = encode.wav16(product)
  end
  product.canon = M.canon(product)
  return product
end

local function generate(kind, spec_input, seed, stream)
  local name, params = spec.algo_of(kind, spec_input)
  local algo = ALGOS[kind][name]
  if algo == nil then
    fail(E.ALGO, "未注册的算法 " .. tostring(spec_input.algo))
  end
  local flow = stream
  if flow == nil then
    flow = spec.default_stream(kind, name)
  end
  if type(flow) ~= "string" or flow == "" then
    fail(E.PARAM, "stream 必须是非空字符串，得到 " .. type(flow))
  end
  return finish(kind, spec_input.algo, algo(params, FLOW.rng(seed, flow)), seed, flow)
end

--- `assetgen.texture(spec, seed, stream?)` → 纹理产物（纯）。
function M.texture(spec_input, seed, stream)
  return generate("texture", spec_input, seed, stream)
end

--- `assetgen.mesh(spec, seed, stream?)` → 几何产物（纯）。
function M.mesh(spec_input, seed, stream)
  return generate("mesh", spec_input, seed, stream)
end

--- `assetgen.sound(spec, seed, stream?)` → 音频产物（纯）。
function M.sound(spec_input, seed, stream)
  return generate("sound", spec_input, seed, stream)
end

--- `assetgen.raster(mesh, opts)` → 纹理产物（`ch = 4`，画面外 alpha = 0）。
---
--- 不消耗随机数：它是网格的一种**投影**，同一份顶点必须逐字节复现同一张图。
--- 流的名字继承网格的那条（`<mesh.stream>/raster`），让 canon 能区分来源。
function M.raster(mesh, opts)
  local product = raster.raster(mesh, opts)
  return finish("texture", "texture.raster", product, mesh.seed, tostring(mesh.stream) .. "/raster")
end

--- `assetgen.bundle(entries, seed)` → 成套素材（`items` 顺序 = 入参数组顺序）。
---
--- `entries[i] = { key, kind, algo, params, stream? }`；`key` 非空且唯一，缺省流名
--- `"assetgen/<key>"`（I2）。一条 seed 派生成套素材的入口——四个复现都只需要这一个
--- 调用就能拿到一整套零外部素材。
function M.bundle(entries, seed)
  if type(entries) ~= "table" or #entries == 0 then
    fail(E.PARAM, "bundle 需要非空的 entries 数组")
  end
  local items = {}
  local seen = {}
  for i = 1, #entries do
    local entry = entries[i]
    if type(entry) ~= "table" then
      fail(E.PARAM, "entries[" .. i .. "] 必须是表")
    end
    local key = entry.key
    if type(key) ~= "string" or key == "" then
      fail(E.PARAM, "entries[" .. i .. "].key 必须是非空字符串")
    end
    if seen[key] then
      fail(E.PARAM, "entries 的 key 重复：" .. key)
    end
    seen[key] = true
    local kind = entry.kind
    if kind ~= "texture" and kind ~= "mesh" and kind ~= "sound" then
      fail(E.KIND, "entries[" .. i .. "].kind 必须是 texture / mesh / sound，得到 " .. tostring(kind))
    end
    local stream = entry.stream
    if stream == nil then
      stream = "assetgen/" .. key
    end
    local product = generate(kind, { algo = entry.algo, params = entry.params }, seed, stream)
    items[i] = { key = key, kind = kind, product = product }
  end
  local bundle = { kind = "bundle", v = M.V, seed_text = tostring(seed), items = items }
  bundle.canon = M.canon(bundle)
  return bundle
end

--- `assetgen.encode(product, container?)` → 字节串（纯；容器表见契约 §4）。
function M.encode(product, container)
  return encode.encode(product, container)
end

-- ── 自省与装配 ──────────────────────────────────────────────────────────────

--- `algo_list(kind)` → 算法名数组（字节序升序；插入排序，沙箱没有 `table.sort`）。
function M.algo_list(kind)
  local source = ALGOS[kind]
  if source == nil then
    fail(E.KIND, "kind 必须是 texture / mesh / sound，得到 " .. tostring(kind))
  end
  local names = {}
  for name in pairs(source) do
    names[#names + 1] = name
  end
  for i = 2, #names do
    local value = names[i]
    local j = i - 1
    while j >= 1 and names[j] > value do
      names[j + 1] = names[j]
      j = j - 1
    end
    names[j + 1] = value
  end
  return names
end

--- `perf_keys()` → 契约 §5 的十个性能键名（不含 `perf.assetgen.` 前缀）。
---
--- 自省面：性能用例按它逐个打印，冒烟按它逐个核对。把键名写进一个函数而不是散在
--- 用例里，是为了让「§5 的十个键齐备」变成**机检**而不是人去数（Goal 反例 4）。
function M.perf_keys()
  return {
    "load_ms",
    "pcm_samples_per_s",
    "tex64_mem_kib",
    "sound1s_mem_kib",
    "leak_100_equal",
    "leak_gc_pct",
    "tex64_ms",
    "mesh_torus_ms",
    "sound_1s_ms",
    "bundle16_ms",
  }
end

--- `streams(seed)` → 三条缺省流句柄（`assetgen.streams` 的用途是「一条 seed 成套」）。
function M.streams(seed)
  return {
    texture = FLOW.rng(seed, "assetgen/texture"),
    mesh = FLOW.rng(seed, "assetgen/mesh"),
    sound = FLOW.rng(seed, "assetgen/sound"),
  }
end

--- 装配：`deps = { procgen? }`。重复装幂等（算法表同名覆盖、`deps` 同值幂等）。
function M.install(deps)
  if deps ~= nil then
    if type(deps) ~= "table" then
      fail(E.PARAM, "install 的 deps 必须是表或 nil，得到 " .. type(deps))
    end
    local injected = deps.procgen
    if injected ~= nil then
      if type(injected) ~= "table" or type(injected.rng) ~= "function" then
        fail(E.DEPS, "deps.procgen 必须导出 rng(seed, stream?)")
      end
      FLOW = injected
    end
  end
  register("texture", texture_algos)
  register("mesh", mesh_algos)
  register("sound", sound_algos)
  return M
end

return M
