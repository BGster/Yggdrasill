-- 夹具冒烟 · procgen 摘要载体（file 级断言，**不计数**）：`--filter` 的 mod 侧接应。
--
-- # 为什么必须有这个文件
--
-- `examples/selftest` 的 `--filter` 对**两侧同时**过滤：mod 的 `tests/*.lua` 与包的
-- `test/*.lua`。`--filter pkg-procgen-digest` 只命中包侧的
-- `procgen/test/pkg-procgen-digest.lua`，mod 侧筛完为空就「没有可执行的测试文件」
-- 直接 exit 1——于是跨进程字节一致的门禁跑不起来。这个文件接住的正是这一格。
--
-- 它自己只钉一条格式契约：canon 行是「`pkg.procgen.canon=` + 单行 JSON」，宿主的
-- `grep | sed` 与 `cmp`/`shasum` 依赖它。多行或空值会让证据链静默变空（`cmp` 两个
-- 空文件相等，是最坏的一种「通过」）。

local saved = rawget(_G, "ygg")
local touched = nil
_G.ygg = setmetatable({}, {
  __index = function(_, key)
    touched = tostring(key)
    error("模块顶层访问了 ygg." .. touched, 0)
  end,
})
package.loaded["pkg.procgen"] = nil
local ok, procgen = pcall(require, "pkg.procgen")
_G.ygg = saved
package.loaded["pkg.procgen"] = nil

if not ok then
  error("[pkg.procgen.digest] require 触发了 ygg 访问：" .. tostring(procgen), 0)
end

do
  local problems = {}
  if type(procgen.hash) ~= "function" then
    problems[#problems + 1] = "hash 必须是函数"
  end
  if touched ~= nil then
    problems[#problems + 1] = "模块顶层访问了 ygg." .. tostring(touched)
  end
  -- 摘要格式：16 位小写十六进制（单行、无前后缀）。
  if type(procgen.hash) == "function" then
    local sample = procgen.hash({ probe = "digest" })
    if type(sample) ~= "string" or #sample ~= 16 or sample:find("^[0-9a-f]+$") == nil then
      problems[#problems + 1] = "hash 必须是 16 位小写十六进制，实得 " .. tostring(sample)
    end
  end
  if #problems > 0 then
    error("[pkg.procgen.digest] " .. table.concat(problems, "; "), 0)
  end
end
