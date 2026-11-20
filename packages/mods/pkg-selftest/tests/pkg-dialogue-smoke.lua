-- 夹具冒烟 · dialogue（1 条）：装载面 + 顶层纯度 + 两个派生事件名。
--
-- 与 `pkg-entity-smoke.lua` 成对，理由（`--filter` 需要 mod 侧同名文件接住、
-- 断言面只钉装载契约）写在那里。
--
-- 第三条断言：`pkg.dialogue.EVENTS` 必须恰好是 `dialogue.failed` /
-- `dialogue.depth_capped` 两个名字——它们是本包**唯一**的对外派生事件面
-- （回退链失败与深度帽各自一条，契约 I2 / I3），行为面由 `pkg-dialogue.lua` 钉。

local saved = rawget(_G, "ygg")
local touched = nil
_G.ygg = setmetatable({}, {
  __index = function(_, key)
    touched = tostring(key)
    error("模块顶层访问了 ygg." .. touched, 0)
  end,
})
for _, name in ipairs({
  "pkg.dialogue",
  "pkg.dialogue.store",
  "pkg.dialogue.handlers",
  "pkg.dialogue.dispatch",
  "pkg.dialogue.memory",
}) do
  package.loaded[name] = nil
end
local ok, dialogue = pcall(require, "pkg.dialogue")
_G.ygg = saved
for _, name in ipairs({
  "pkg.dialogue",
  "pkg.dialogue.store",
  "pkg.dialogue.handlers",
  "pkg.dialogue.dispatch",
  "pkg.dialogue.memory",
}) do
  package.loaded[name] = nil
end

if not ok then
  check("pkg.dialogue.loadable", function()
    return "require 触发了 ygg 访问：" .. tostring(dialogue)
  end)
  return
end

check("pkg.dialogue.loadable", function()
  local problems = {}
  if type(dialogue) ~= "table" then
    problems[#problems + 1] = "模块必须导出表，实得 " .. type(dialogue)
  end
  for _, name in ipairs({ "install", "on", "off", "emit", "say", "history", "frame" }) do
    if type(dialogue[name]) ~= "function" then
      problems[#problems + 1] = name .. " 必须是函数"
    end
  end
  if type(dialogue.VERSION) ~= "string" then
    problems[#problems + 1] = "VERSION 必须是字符串"
  end
  if touched ~= nil then
    problems[#problems + 1] = "模块顶层访问了 ygg." .. tostring(touched)
  end
  local want = { "dialogue.depth_capped", "dialogue.failed" }
  local got = {}
  for i = 1, #dialogue.EVENTS do
    got[#got + 1] = dialogue.EVENTS[i]
  end
  table.sort(got)
  if #got ~= #want then
    problems[#problems + 1] = "EVENTS 数量应为 2，实得 " .. #got
  end
  for i = 1, #want do
    if got[i] ~= want[i] then
      problems[#problems + 1] = "EVENTS[" .. i .. "] 应为 " .. want[i] .. "，实得 " .. tostring(got[i])
    end
  end
  -- install 无 deps 时不该碰 ygg（顶层纯度延伸到装配面：桥只在 deps.quest 时装）。
  local ok_install, err = pcall(dialogue.install)
  if not ok_install then
    problems[#problems + 1] = "install() 无参必须可调用：" .. tostring(err)
  end
  return table.concat(problems, "; ")
end)
