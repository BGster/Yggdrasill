-- 夹具冒烟 · textframe（file 级断言，**不计数**）：装载面 + 顶层纯度 + 导出面 + 两处口径钉。
--
-- # 为什么不计数
--
-- 包用例的计数口径在 `openwiki/contracts/packages/README.md` §5（逐包复算），本文件是
-- **载体的**冒烟，不是包的行为用例；计入就会把全量抬高。判定一样硬：
-- `error` ⇒ `file FAIL` ⇒ 命令 exit 1（`ok=0`）。
--
-- # 为什么每个包都要一个 mod 侧同名文件
--
-- `examples/selftest` 的 `--filter` 对**两侧同时**过滤：mod 的 `tests/*.lua` 与包的
-- `test/*.lua`。`--filter pkg-textframe` 命中包侧文件，mod 侧筛完为空就「没有可执行
-- 的测试文件」直接 exit 1——这个文件接住的正是这一格。
--
-- # 钉什么
--
-- ① 顶层不碰宿主面（`ygg.*` 访问即红）；② 导出面清单（7 函数 + 2 常量表）；
-- ③ `ERRORS` 四码的**码集**（跨包契约：用例按码做子串断言，改名会在别处静默失效）；
-- ④ `perf_keys` 九键齐备（§5 的打印键，改表不打印即静默少指标）；
-- ⑤ **行宽不 trim**——这是与 mod 侧 golden 帧的唯一接缝（I6），在载体层再钉一次：
--    空白帧的每一行必须是 `w` 个空格，不是空串。

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
    if type(name) == "string" and (name == "pkg.textframe" or name:find("^pkg%.textframe%.")) then
      package.loaded[name] = nil
    end
  end
end

clear()
local ok, textframe = pcall(require, "pkg.textframe")
_G.ygg = saved
clear()

if not ok then
  error("[pkg.textframe.loadable] require 触发了 ygg 访问：" .. tostring(textframe), 0)
end

do
  local problems = {}
  if type(textframe) ~= "table" then
    problems[#problems + 1] = "模块必须导出表，实得 " .. type(textframe)
  end
  if type(textframe.VERSION) ~= "string" then
    problems[#problems + 1] = "VERSION 必须是字符串"
  end
  if touched ~= nil then
    problems[#problems + 1] = "模块顶层访问了 ygg." .. tostring(touched)
  end

  local exports = {
    "blit",
    "frame",
    "install",
    "perf_keys",
    "put",
    "stack",
    "to_lines",
  }
  for i = 1, #exports do
    if type(textframe[exports[i]]) ~= "function" then
      problems[#problems + 1] = exports[i] .. " 必须是函数"
    end
  end

  -- 性能键面：契约 §5 的七个键名（`perf.textframe.` 前缀之外的完整集合）。
  local perf_want = {
    "blit_us",
    "compose_us",
    "frame_kib",
    "leak_100_equal",
    "leak_gc_pct",
    "load_ms",
    "put_ns",
    "throughput_fps",
    "vs_naive",
  }
  if type(textframe.perf_keys) == "function" then
    local got_keys = textframe.perf_keys()
    if #got_keys ~= #perf_want then
      problems[#problems + 1] = "perf_keys 应为 9 个，实得 " .. #got_keys
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

  -- 错误码面：四个键逐项相等（不是「包含」）。
  local want = { "CELL", "PARAM", "RANGE", "SHAPE" }
  local got = {}
  for key, value in pairs(textframe.ERRORS or {}) do
    got[#got + 1] = key
    if type(value) ~= "string" or value:find("^YGG%-PKG%-TEXTFRAME%-") == nil then
      problems[#problems + 1] = "ERRORS." .. tostring(key) .. " 必须形如 YGG-PKG-TEXTFRAME-*"
    end
  end
  table.sort(got)
  if #got ~= #want then
    problems[#problems + 1] = "ERRORS 数量应为 4，实得 " .. #got
  end
  for i = 1, #want do
    if got[i] ~= want[i] then
      problems[#problems + 1] = "ERRORS[" .. i .. "] 应为 " .. want[i] .. "，实得 " .. tostring(got[i])
    end
  end

  if #problems > 0 then
    error("[pkg.textframe.loadable] " .. table.concat(problems, "; "), 0)
  end
end

-- [pkg.textframe.width-not-trimmed] 载体层再钉一次 I6：空白帧每行 = `w` 个空格。
-- 尾空格被 trim 掉时实得 0；行尾多补一格时实得 5——两种都会让 golden 帧静默失配。
do
  textframe.install()
  local blank = textframe.to_lines(textframe.frame(4, 2))
  if #blank ~= 2 then
    error("[pkg.textframe.width-not-trimmed] frame(4,2) 应出 2 行，实得 " .. #blank, 0)
  end
  for i = 1, #blank do
    if blank[i] ~= "    " then
      error("[pkg.textframe.width-not-trimmed] 第 " .. i .. " 行应为 4 个空格（不 trim），实得 #"
        .. #blank[i] .. " [" .. blank[i] .. "]", 0)
    end
  end
end
