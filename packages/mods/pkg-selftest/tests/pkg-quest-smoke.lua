-- 夹具冒烟 · quest（file 级断言，**不计数**）：装载面 + 顶层纯度 + 派生事件面清单。
--
-- # 为什么不计数
--
-- 包用例的计数口径是「r1 的 40 + 本需求新增的 50」（`packages/README.md` §5），
-- 本文件是**载体的**冒烟，不是包的行为用例；它一旦计入就会把 90 变成 92。
-- 判定一样硬：`error` ⇒ `file FAIL` ⇒ 命令 exit 1（`ok=0`），与 perf 文件同一口径。
--
-- 与 `pkg-entity-smoke.lua` 成对，理由（`--filter` 需要 mod 侧同名文件接住、
-- 断言面只钉装载契约）写在那里。
--
-- 第三条断言单独记在这里：`pkg.quest.EVENTS` 必须恰好是五个名字——dialogue 的
-- 桥只读这张表（不猜名字，用例 d17/d18），所以这张表的**内容**是包间契约，
-- 值得在装载面钉一次；行为面（谁在什么时候发它）由 `pkg-quest.lua` 逐条钉。

local saved = rawget(_G, "ygg")
local touched = nil
_G.ygg = setmetatable({}, {
  __index = function(_, key)
    touched = tostring(key)
    error("模块顶层访问了 ygg." .. touched, 0)
  end,
})
package.loaded["pkg.quest"] = nil
package.loaded["pkg.quest.store"] = nil
package.loaded["pkg.quest.types"] = nil
package.loaded["pkg.quest.flow"] = nil
local ok, quest = pcall(require, "pkg.quest")
_G.ygg = saved
package.loaded["pkg.quest"] = nil
package.loaded["pkg.quest.store"] = nil
package.loaded["pkg.quest.types"] = nil
package.loaded["pkg.quest.flow"] = nil

if not ok then
  error("[pkg.quest.loadable] require 触发了 ygg 访问：" .. tostring(quest), 0)
end

do
  local problems = {}
  if type(quest) ~= "table" then
    problems[#problems + 1] = "模块必须导出表，实得 " .. type(quest)
  end
  if type(quest.install) ~= "function" then
    problems[#problems + 1] = "install 必须是函数"
  end
  if type(quest.VERSION) ~= "string" then
    problems[#problems + 1] = "VERSION 必须是字符串"
  end
  if touched ~= nil then
    problems[#problems + 1] = "模块顶层访问了 ygg." .. tostring(touched)
  end
  -- 派生事件面：名字逐项相等（不是「包含」）。
  local want = {
    "quest.abandoned",
    "quest.advanced",
    "quest.completed",
    "quest.jumped",
    "quest.opened",
  }
  local got = {}
  for i = 1, #quest.EVENTS do
    got[#got + 1] = quest.EVENTS[i]
  end
  table.sort(got)
  if #got ~= #want then
    problems[#problems + 1] = "EVENTS 数量应为 5，实得 " .. #got
  end
  for i = 1, #want do
    if got[i] ~= want[i] then
      problems[#problems + 1] = "EVENTS[" .. i .. "] 应为 " .. want[i] .. "，实得 " .. tostring(got[i])
    end
  end
  if type(quest.TRANSITIONS) ~= "table" or type(quest.TRANSITIONS.open) ~= "table" then
    problems[#problems + 1] = "TRANSITIONS 必须导出 open 行（迁移表是数据）"
  end
  if #problems > 0 then
    error("[pkg.quest.loadable] " .. table.concat(problems, "; "), 0)
  end
end
