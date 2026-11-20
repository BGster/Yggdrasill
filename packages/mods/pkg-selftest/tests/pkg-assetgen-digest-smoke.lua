-- 夹具冒烟 · assetgen 摘要载体（file 级断言，**不计数**）：`--filter` 的 mod 侧接应。
--
-- # 为什么必须有这个文件
--
-- `examples/selftest` 的 `--filter` 对**两侧同时**过滤：mod 的 `tests/*.lua` 与包的
-- `test/*.lua`。`--filter pkg-assetgen-digest` 只命中包侧的
-- `assetgen/test/pkg-assetgen-digest.lua`，mod 侧筛完为空就「没有可执行的测试文件」
-- 直接 exit 1——于是跨进程字节一致的门禁跑不起来。
--
-- # 它自己钉什么
--
-- 证据行的**格式**：`pkg.assetgen.lengths=` 与 `pkg.assetgen.hex=` 各一行、单行、
-- 无空值、hex 长度与 lengths 之和自洽。多行或空值会让宿主的 `cmp` 静默变空
-- （两个空文件相等，是最坏的一种「通过」）。

local saved = rawget(_G, "ygg")
local touched = nil
_G.ygg = setmetatable({}, {
  __index = function(_, key)
    touched = tostring(key)
    error("模块顶层访问了 ygg." .. touched, 0)
  end,
})
package.loaded["pkg.assetgen"] = nil
local ok, assetgen = pcall(require, "pkg.assetgen")
_G.ygg = saved
package.loaded["pkg.assetgen"] = nil

if not ok then
  error("[pkg.assetgen.digest] require 触发了 ygg 访问：" .. tostring(assetgen), 0)
end

do
  local problems = {}
  if touched ~= nil then
    problems[#problems + 1] = "模块顶层访问了 ygg." .. tostring(touched)
  end
  if type(assetgen.bundle) ~= "function" or type(assetgen.encode) ~= "function" then
    problems[#problems + 1] = "bundle / encode 必须是函数"
  end
  if type(assetgen.bundle) == "function" then
    assetgen.install()
    local bundle = assetgen.bundle({
      {
        key = "probe",
        kind = "texture",
        algo = "texture.checker",
        params = { w = 4, h = 4, ch = 1, size = 2 },
      },
    }, "digest-smoke")
    if type(bundle.canon) ~= "string" or bundle.canon:find("\n", 1, true) ~= nil then
      problems[#problems + 1] = "bundle.canon 必须是单行字符串"
    end
    local hex = assetgen.encode(bundle.items[1].product, "hex")
    if type(hex) ~= "string" or #hex ~= #bundle.items[1].product.bytes * 2 then
      problems[#problems + 1] = "hex 长度必须是 2×字节数"
    end
    if hex:find("^[0-9a-f]+$") == nil then
      problems[#problems + 1] = "hex 必须是小写十六进制"
    end
  end
  if #problems > 0 then
    error("[pkg.assetgen.digest] " .. table.concat(problems, "; "), 0)
  end
end
