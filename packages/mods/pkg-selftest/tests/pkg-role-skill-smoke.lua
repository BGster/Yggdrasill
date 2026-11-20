-- 夹具冒烟 · role-skill（file 级断言，**不计数**）：装载面 + 顶层纯度 + 事件面。
--
-- # 为什么不计数
--
-- 包用例的计数口径是「90 + 本需求新增的 role-skill / procgen 用例」，本文件是
-- **载体的**冒烟：判定一样硬（`error` ⇒ `file FAIL` ⇒ `ok=0`），但不进 `cases passed=`。
--
-- # 为什么必须有它
--
-- `--filter` 对 mod 侧与包侧**同时**过滤，mod 侧筛完为空就「没有可执行的测试文件」
-- 直接 exit 1——`--filter pkg-role-skill` 因此需要一个 mod 侧的接应文件。
--
-- # 钉什么
--
-- 装载面（包索引可 require、顶层不访问宿主面、导出 `install` / `VERSION`）与
-- **事件名集合**：`role.granted` / `role.revoked` / `role.skill_gained` /
-- `role.skill_lost` 是跨包契约（订阅方按名字订，名字变了订阅静默失效）。

local saved = rawget(_G, "ygg")
local touched = nil
_G.ygg = setmetatable({}, {
  __index = function(_, key)
    touched = tostring(key)
    error("模块顶层访问了 ygg." .. touched, 0)
  end,
})
package.loaded["pkg.role_skill"] = nil
package.loaded["pkg.role_skill.store"] = nil
package.loaded["pkg.role_skill.actors"] = nil
local ok, role = pcall(require, "pkg.role_skill")
_G.ygg = saved
package.loaded["pkg.role_skill"] = nil
package.loaded["pkg.role_skill.store"] = nil
package.loaded["pkg.role_skill.actors"] = nil

if not ok then
  error("[pkg.role_skill.loadable] require 触发了 ygg 访问：" .. tostring(role), 0)
end

do
  local problems = {}
  if type(role) ~= "table" then
    problems[#problems + 1] = "模块必须导出表，实得 " .. type(role)
  end
  if type(role.VERSION) ~= "string" then
    problems[#problems + 1] = "VERSION 必须是字符串"
  end
  if touched ~= nil then
    problems[#problems + 1] = "模块顶层访问了 ygg." .. tostring(touched)
  end
  for _, name in ipairs({ "install", "define", "skill_define", "grant", "revoke", "has" }) do
    if type(role[name]) ~= "function" then
      problems[#problems + 1] = name .. " 必须是函数"
    end
  end

  local want = { "role.granted", "role.revoked", "role.skill_gained", "role.skill_lost" }
  local got = {}
  for i = 1, #(role.EVENTS or {}) do
    got[#got + 1] = role.EVENTS[i]
  end
  table.sort(got)
  if #got ~= #want then
    problems[#problems + 1] = "EVENTS 数量应为 4，实得 " .. #got
  end
  for i = 1, #want do
    if got[i] ~= want[i] then
      problems[#problems + 1] = "EVENTS[" .. i .. "] 应为 " .. want[i] .. "，实得 " .. tostring(got[i])
    end
  end

  -- KV 前缀纪律（D10）：本包只写 `role.*`，且四个键名是单源。
  for _, key in ipairs({ "DEFS", "SKILLS", "INDEX" }) do
    local value = (role.KEY or {})[key]
    if type(value) ~= "string" or value:find("^role%.") == nil then
      problems[#problems + 1] = "KEY." .. key .. " 必须以 role. 开头，实得 " .. tostring(value)
    end
  end

  if #problems > 0 then
    error("[pkg.role_skill.loadable] " .. table.concat(problems, "; "), 0)
  end
end
