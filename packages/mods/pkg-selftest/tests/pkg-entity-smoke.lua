-- 夹具冒烟 · entity（1 条）：装载面 + 顶层纯度。
--
-- # 为什么每个包一个冒烟文件
--
-- `examples/selftest` 的 `--filter` 同时过滤 **mod 测试文件**与**包测试文件**，
-- 且「一处都匹配不到」会以 `没有可执行的测试文件` 直接 exit 1（实测）。于是
-- `--filter pkg-entity` / `--filter pkg-codec` 这两条按包跑的命令，必须各有
-- 一个**文件名里带该包名**的 mod 侧测试文件接住，否则它们跑不了。
--
-- # 断言面刻意窄
--
-- 只钉两条**装载契约**事实：包经包索引可 require 且导出 `install` / `VERSION`；
-- 模块顶层不访问 `ygg`（用反证：把 `ygg` 换成「一访问就报错」的代理后 require
-- 仍须成功，契约 entity.md §3 I7 的最硬形态）。
--
-- 不在这里复述 API 函数表：那份表已经由行为用例逐条调用来钉（`pkg-entity.lua`），
-- 在冒烟里再抄一遍就是第二份真源，加一个函数要改两处——正是本仓反的漂移形状。

local saved = rawget(_G, "ygg")
local touched = nil
_G.ygg = setmetatable({}, {
  __index = function(_, key)
    touched = tostring(key)
    error("模块顶层访问了 ygg." .. touched, 0)
  end,
})
package.loaded["pkg.entity"] = nil
local ok, entity = pcall(require, "pkg.entity")
_G.ygg = saved
package.loaded["pkg.entity"] = nil

if not ok then
  check("pkg.entity.loadable", false, "require 触发了 ygg 访问：" .. tostring(entity))
  return
end

check("pkg.entity.loadable", function()
  local problems = {}
  if type(entity) ~= "table" then
    problems[#problems + 1] = "模块必须导出表，实得 " .. type(entity)
  end
  if type(entity.install) ~= "function" then
    problems[#problems + 1] = "install 必须是函数"
  end
  if type(entity.VERSION) ~= "string" then
    problems[#problems + 1] = "VERSION 必须是字符串"
  end
  if touched ~= nil then
    problems[#problems + 1] = "模块顶层访问了 ygg." .. tostring(touched)
  end
  return table.concat(problems, "; ")
end)
