-- src/config.lua —— 两层配置（动态覆盖 `settings.<path>` > 静态默认 `data/config.json`）+ 标量读取小工具。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 资源由入口 install(deps) 注入。本模块唯一写的 KV 前缀：`settings.*` 只读（写方：src/handlers/config.lua）。

local M = {}

local util = require("src.util")

local TEXTS
local CONFIG

local contains, split =
  util.contains, util.split

local function static(path)
  local node = CONFIG
  for _, part in ipairs(split(path, ".")) do
    if type(node) ~= "table" then
      return nil
    end
    node = node[part]
  end
  return node
end

local function effective(path)
  local override = ygg.kv.get("settings." .. path)
  if override ~= nil then
    return override
  end
  return static(path)
end

local MODES = { "off", "suggest", "auto" }

local function mode()
  local value = effective("evolution.mode")
  if type(value) == "string" then
    return value
  end
  return "off"
end

local function next_mode()
  local current = mode()
  for i = 1, #MODES do
    if MODES[i] == current then
      return MODES[(i % #MODES) + 1]
    end
  end
  return MODES[1]
end

local PLAY_MODES = { "goal", "guide" }

local DEFAULT_PLAY_MODE = "guide"

local function play_mode()
  local value = effective("play.mode")
  if type(value) == "string" and contains(PLAY_MODES, value) then
    return value
  end
  return DEFAULT_PLAY_MODE
end

local function next_play_mode()
  local current = play_mode()
  for i = 1, #PLAY_MODES do
    if PLAY_MODES[i] == current then
      return PLAY_MODES[(i % #PLAY_MODES) + 1]
    end
  end
  return PLAY_MODES[1]
end

local function strict_gates()
  return effective("gates.require_all_green") == true
end

local function max_per_run()
  local value = effective("evolution.max_per_run")
  if type(value) == "number" then
    return value
  end
  return 0
end

local function num(key, fallback)
  local value = ygg.kv.get(key)
  if type(value) == "number" then
    return value
  end
  return fallback
end

local function text_of(key, fallback)
  local value = ygg.kv.get(key)
  if type(value) == "string" and value ~= "" then
    return value
  end
  return fallback
end

local function task_title()
  return text_of("task.title", TEXTS.beats.default_task_title)
end

local function task_desc()
  return text_of("task.description", TEXTS.beats.default_task_desc)
end

function M.install(deps)
  TEXTS = deps.texts
  CONFIG = deps.config
end

M.DEFAULT_PLAY_MODE = DEFAULT_PLAY_MODE
M.MODES = MODES
M.PLAY_MODES = PLAY_MODES
M.effective = effective
M.max_per_run = max_per_run
M.mode = mode
M.next_mode = next_mode
M.next_play_mode = next_play_mode
M.num = num
M.play_mode = play_mode
M.static = static
M.strict_gates = strict_gates
M.task_desc = task_desc
M.task_title = task_title
M.text_of = text_of

return M
