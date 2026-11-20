-- 夹具冒烟 · codec（1 条）：装载面 + 顶层纯度 + 依赖序单向。
--
-- 与 `pkg-entity-smoke.lua` 成对，理由（`--filter` 需要 mod 侧同名文件接住、
-- 断言面只钉装载契约）写在那里。
--
-- 第三条断言单独记在这里：`pkg.codec` 必须 `require("pkg.entity")`（README §3 的
-- 依赖序），而 `pkg.entity` **不得**反向持有 codec——反向边一旦出现，两个包就成了
-- 一个环，「先装谁」开始取决于装载顺序，确定性判据当场破。

local saved = rawget(_G, "ygg")
local touched = nil
_G.ygg = setmetatable({}, {
  __index = function(_, key)
    touched = tostring(key)
    error("模块顶层访问了 ygg." .. touched, 0)
  end,
})
package.loaded["pkg.entity"] = nil
package.loaded["pkg.codec"] = nil
local ok, codec = pcall(require, "pkg.codec")
_G.ygg = saved
package.loaded["pkg.entity"] = nil
package.loaded["pkg.codec"] = nil

if not ok then
  check("pkg.codec.loadable", false, "require 触发了 ygg 访问：" .. tostring(codec))
  return
end

check("pkg.codec.loadable", function()
  local problems = {}
  if type(codec) ~= "table" then
    problems[#problems + 1] = "模块必须导出表，实得 " .. type(codec)
  end
  if type(codec.install) ~= "function" then
    problems[#problems + 1] = "install 必须是函数"
  end
  if type(codec.VERSION) ~= "string" then
    problems[#problems + 1] = "VERSION 必须是字符串"
  end
  if touched ~= nil then
    problems[#problems + 1] = "模块顶层访问了 ygg." .. tostring(touched)
  end
  -- 依赖序单向：entity 不该持有 codec（它不需要编码器，见 entity.md §1）。
  package.loaded["pkg.entity"] = nil
  local entity = require("pkg.entity")
  if rawget(entity, "codec") ~= nil then
    problems[#problems + 1] = "pkg.entity 持有了 codec（依赖序必须单向）"
  end
  package.loaded["pkg.entity"] = nil
  return table.concat(problems, "; ")
end)
