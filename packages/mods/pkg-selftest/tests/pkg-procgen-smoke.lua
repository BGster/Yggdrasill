-- 夹具冒烟 · procgen（file 级断言，**不计数**）：装载面 + 顶层纯度 + 错误码面。
--
-- # 为什么不计数
--
-- 包用例的计数口径是「90 + 本需求新增的 role-skill / procgen 用例」，本文件是
-- **载体的**冒烟，不是包的行为用例；计入就会让 125 变成 126。判定一样硬：
-- `error` ⇒ `file FAIL` ⇒ 命令 exit 1（`ok=0`）。
--
-- 与 `pkg-entity-smoke.lua` 成对，理由（`--filter` 需要 mod 侧同名文件接住、
-- 断言面只钉装载契约）写在那里。
--
-- 另一条单独记在这里：`pkg.procgen.ERRORS` 的**码集**是跨包契约——role-skill 与
-- 将来的 assetgen 都按码做子串断言，改名会在别处静默失效，所以装载面钉一次全集。

local saved = rawget(_G, "ygg")
local touched = nil
_G.ygg = setmetatable({}, {
  __index = function(_, key)
    touched = tostring(key)
    error("模块顶层访问了 ygg." .. touched, 0)
  end,
})
package.loaded["pkg.procgen"] = nil
package.loaded["pkg.procgen.rng"] = nil
local ok, procgen = pcall(require, "pkg.procgen")
_G.ygg = saved
package.loaded["pkg.procgen"] = nil
package.loaded["pkg.procgen.rng"] = nil

if not ok then
  error("[pkg.procgen.loadable] require 触发了 ygg 访问：" .. tostring(procgen), 0)
end

do
  local problems = {}
  if type(procgen) ~= "table" then
    problems[#problems + 1] = "模块必须导出表，实得 " .. type(procgen)
  end
  if type(procgen.VERSION) ~= "string" then
    problems[#problems + 1] = "VERSION 必须是字符串"
  end
  if touched ~= nil then
    problems[#problems + 1] = "模块顶层访问了 ygg." .. tostring(touched)
  end

  local exports = { "rng", "state", "next32" }
  for i = 1, #exports do
    if type(procgen[exports[i]]) ~= "function" then
      problems[#problems + 1] = exports[i] .. " 必须是函数"
    end
  end

  -- 错误码面：六个键逐项相等（不是「包含」）。
  local want = {
    "ALGO",
    "DEF",
    "DEPS",
    "PARAM",
    "RANGE",
    "SEED",
  }
  local got = {}
  for key, value in pairs(procgen.ERRORS or {}) do
    got[#got + 1] = key
    if type(value) ~= "string" or value:find("^YGG%-PKG%-PROCGEN%-") == nil then
      problems[#problems + 1] = "ERRORS." .. tostring(key) .. " 必须形如 YGG-PKG-PROCGEN-*"
    end
  end
  table.sort(got)
  if #got ~= #want then
    problems[#problems + 1] = "ERRORS 数量应为 6，实得 " .. #got
  end
  for i = 1, #want do
    if got[i] ~= want[i] then
      problems[#problems + 1] = "ERRORS[" .. i .. "] 应为 " .. want[i] .. "，实得 " .. tostring(got[i])
    end
  end

  if #problems > 0 then
    error("[pkg.procgen.loadable] " .. table.concat(problems, "; "), 0)
  end
end
