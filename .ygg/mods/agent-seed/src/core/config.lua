-- src/core/config.lua —— 两层配置（动态覆盖 `settings.<path>` > 静态默认 `data/config.json`）+ 标量读取小工具。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 资源由入口 install(deps) 注入。本模块唯一写的 KV 前缀：`settings.*` 只读（写方：src/handlers/system/config.lua）。

local M = {}

local util = require("src.core.util")

local TEXTS
local CONFIG

local contains, split, trim =
  util.contains, util.split, util.trim

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

-- E2 登记面读取（写入方：src/handlers/system/config.lua）。两者都是自声明，不是身份凭据。
local function goal_authorization()
  local value = effective("goal.authorization")
  if type(value) == "string" then
    return trim(value)
  end
  return ""
end

-- r9 退役武装（写方 = config.set `run.retire`，值域 "true" / ""）：
-- 唯一真值是字符串 "true"；其余值或缺省一律视为未武装（批收口常态）。
-- 与 goal.authorization 同构：run 级自声明预登记，由门禁动作（review.note）消费。
local function run_retire_armed()
  return effective("run.retire") == "true"
end

local function executor_raw(req_id, stage)
  local value = effective("executor." .. tostring(req_id) .. "." .. tostring(stage))
  if type(value) == "string" then
    return trim(value)
  end
  return ""
end

function M.install(deps)
  TEXTS = deps.texts
  CONFIG = deps.config
end

M.DEFAULT_PLAY_MODE = DEFAULT_PLAY_MODE
M.MODES = MODES
M.PLAY_MODES = PLAY_MODES
M.effective = effective
M.executor_raw = executor_raw
M.goal_authorization = goal_authorization
M.max_per_run = max_per_run
M.mode = mode
M.next_mode = next_mode
M.next_play_mode = next_play_mode
M.num = num
M.play_mode = play_mode
M.run_retire_armed = run_retire_armed
M.static = static
M.strict_gates = strict_gates
M.task_desc = task_desc
M.task_title = task_title
M.text_of = text_of

return M
