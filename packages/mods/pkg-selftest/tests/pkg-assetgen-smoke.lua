-- 夹具冒烟 · assetgen（file 级断言，**不计数**）：装载面 + 顶层纯度 + 错误码面 + 导出面。
--
-- # 为什么不计数
--
-- 包用例的计数口径是「90 + role-skill 17 + procgen 18 + assetgen 28」，本文件是
-- **载体的**冒烟，不是包的行为用例；计入就会让 153 变成 154。判定一样硬：
-- `error` ⇒ `file FAIL` ⇒ 命令 exit 1（`ok=0`）。
--
-- # 为什么每个包都要一个 mod 侧同名文件
--
-- `examples/selftest` 的 `--filter` 对**两侧同时**过滤：mod 的 `tests/*.lua` 与包的
-- `test/*.lua`。`--filter pkg-assetgen` 命中包侧文件，mod 侧筛完为空就「没有可执行
-- 的测试文件」直接 exit 1——这个文件接住的正是这一格。
--
-- # 钉什么
--
-- ① 顶层不碰宿主面（`ygg.*` 访问即红）；② 导出面清单；③ `ERRORS` 五键的**码集**
-- （跨包契约：用例按码做子串断言，改名会在别处静默失效）。

local saved = rawget(_G, "ygg")
local touched = nil
_G.ygg = setmetatable({}, {
  __index = function(_, key)
    touched = tostring(key)
    error("模块顶层访问了 ygg." .. touched, 0)
  end,
})

local function clear()
  for name in pairs(package.loaded) do
    if type(name) == "string" and (name == "pkg.assetgen" or name:find("^pkg%.assetgen%.")) then
      package.loaded[name] = nil
    end
  end
end

clear()
local ok, assetgen = pcall(require, "pkg.assetgen")
_G.ygg = saved
clear()

if not ok then
  error("[pkg.assetgen.loadable] require 触发了 ygg 访问：" .. tostring(assetgen), 0)
end

do
  local problems = {}
  if type(assetgen) ~= "table" then
    problems[#problems + 1] = "模块必须导出表，实得 " .. type(assetgen)
  end
  if type(assetgen.VERSION) ~= "string" then
    problems[#problems + 1] = "VERSION 必须是字符串"
  end
  if touched ~= nil then
    problems[#problems + 1] = "模块顶层访问了 ygg." .. tostring(touched)
  end

  local exports = {
    "algo_list",
    "bundle",
    "canon",
    "encode",
    "install",
    "mesh",
    "perf_keys",
    "raster",
    "sound",
    "streams",
    "texture",
  }
  for i = 1, #exports do
    if type(assetgen[exports[i]]) ~= "function" then
      problems[#problems + 1] = exports[i] .. " 必须是函数"
    end
  end

  -- 性能键面：契约 §5 的十个键名（`perf.assetgen.` 前缀之外的完整集合）。
  local perf_want = {
    "bundle16_ms",
    "leak_100_equal",
    "leak_gc_pct",
    "load_ms",
    "mesh_torus_ms",
    "pcm_samples_per_s",
    "sound1s_mem_kib",
    "sound_1s_ms",
    "tex64_mem_kib",
    "tex64_ms",
  }
  if type(assetgen.perf_keys) == "function" then
    local got_keys = assetgen.perf_keys()
    if #got_keys ~= #perf_want then
      problems[#problems + 1] = "perf_keys 应为 10 个，实得 " .. #got_keys
    end
    local seen = {}
    for i = 1, #got_keys do
      seen[got_keys[i]] = true
    end
    for i = 1, #perf_want do
      if not seen[perf_want[i]] then
        problems[#problems + 1] = "perf_keys 缺 " .. perf_want[i]
      end
    end
  end

  -- 错误码面：五个键逐项相等（不是「包含」）。
  local want = { "ALGO", "CAP", "DEPS", "KIND", "PARAM" }
  local got = {}
  for key, value in pairs(assetgen.ERRORS or {}) do
    got[#got + 1] = key
    if type(value) ~= "string" or value:find("^YGG%-PKG%-ASSETGEN%-") == nil then
      problems[#problems + 1] = "ERRORS." .. tostring(key) .. " 必须形如 YGG-PKG-ASSETGEN-*"
    end
  end
  table.sort(got)
  if #got ~= #want then
    problems[#problems + 1] = "ERRORS 数量应为 5，实得 " .. #got
  end
  for i = 1, #want do
    if got[i] ~= want[i] then
      problems[#problems + 1] = "ERRORS[" .. i .. "] 应为 " .. want[i] .. "，实得 " .. tostring(got[i])
    end
  end

  if #problems > 0 then
    error("[pkg.assetgen.loadable] " .. table.concat(problems, "; "), 0)
  end
end

-- [pkg.assetgen.bmp24-header] r4-F1 回归：bmp24 的 54 字节头不得被 y=0 像素行覆盖。
-- 62 = 14 文件头 + 40 DIB + (2×3 像素 + 2 pad) × 1 行；头被覆盖时实得 8。
do
  assetgen.install()
  local bundle = assetgen.bundle({
    {
      key = "probe-bmp",
      kind = "texture",
      algo = "texture.checker",
      params = { w = 2, h = 1, ch = 3, size = 2 },
    },
  }, "bmp24-header")
  local bmp = assetgen.encode(bundle.items[1].product, "bmp24")
  if type(bmp) ~= "string" or #bmp ~= 62 then
    error("[pkg.assetgen.bmp24-header] w=2,h=1 的 bmp24 应为 62 字节，实得 "
      .. tostring(type(bmp) == "string" and #bmp or type(bmp)), 0)
  end
  if bmp:sub(1, 2) ~= "BM" then
    error("[pkg.assetgen.bmp24-header] bmp24 头两字节应为 'BM'（头被像素行覆盖即丢）", 0)
  end
end
