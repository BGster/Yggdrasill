-- src/domain/wire/settlement.lua —— S32 纯搬移自 wire.lua（零逻辑变更；接线见交付回执）。

local M = {}
local util = require("src.core.util")
local state = require("src.domain.wire.state")
local TEXTS = setmetatable({}, { __index = function(_, k) return state.TEXTS[k] end })
local mod_evolution = require("src.domain.wire.evolution")
local contains = util.contains
local cut = util.cut
local fill = util.fill
local join = util.join
local split = util.split
local trim = util.trim
local TODO_VERIFIED_MAX = mod_evolution.TODO_VERIFIED_MAX


local function sessions_shape_ok(moved, remaining, destination)
  if moved == nil or remaining == nil then
    return false
  end
  return moved >= 0 and remaining == 0 and destination ~= ""
end

local function todo_shape_ok(remaining, verified_by)
  if remaining == nil then
    return false
  end
  return remaining == 0 and verified_by ~= "" and #verified_by <= TODO_VERIFIED_MAX
end

-- 独立动作的载荷：`moved|remaining|destination`（`|` 分隔，恰三段）。
local function parse_sessions_payload(payload)
  if type(payload) ~= "string" then
    return nil, TEXTS.beats.settlement_sessions_usage
  end
  local parts = split(trim(payload), "|")
  if #parts ~= 3 then
    return nil, TEXTS.beats.settlement_sessions_usage
  end
  local moved = tonumber(trim(parts[1]))
  local remaining = tonumber(trim(parts[2]))
  local destination = trim(parts[3])
  if not sessions_shape_ok(moved, remaining, destination) then
    return nil, TEXTS.beats.settlement_sessions_usage
  end
  return { moved = math.floor(moved), remaining = 0, destination = destination }, nil
end

-- 独立动作的载荷：`remaining|verified_by`（`|` 分隔，恰两段）。
local function parse_todo_payload(payload)
  if type(payload) ~= "string" then
    return nil, TEXTS.beats.settlement_todo_usage
  end
  local parts = split(trim(payload), "|")
  if #parts ~= 2 then
    return nil, TEXTS.beats.settlement_todo_usage
  end
  local remaining = tonumber(trim(parts[1]))
  local verified_by = trim(parts[2])
  if not todo_shape_ok(remaining, verified_by) then
    return nil, TEXTS.beats.settlement_todo_usage
  end
  return { remaining = 0, verified_by = verified_by }, nil
end

-- 可选段的载荷：`moved/remaining/destination`（只切前两刀，destination 里的 `/` 原样保留）。
local function parse_sessions_segment(value)
  local moved_text, rest = cut(trim(value or ""), "/")
  local remaining_text, destination = cut(trim(rest or ""), "/")
  local moved = tonumber(trim(moved_text))
  local remaining = tonumber(trim(remaining_text))
  local text = trim(destination or "")
  if not sessions_shape_ok(moved, remaining, text) then
    return nil, TEXTS.beats.settlement_sessions_segment_bad
  end
  return { moved = math.floor(moved), remaining = 0, destination = text }, nil
end

-- 可选段的载荷：`remaining/verified_by`（只切第一刀，verified_by 里的 `/` 原样保留）。
local function parse_todo_segment(value)
  local remaining_text, verified_by = cut(trim(value or ""), "/")
  local remaining = tonumber(trim(remaining_text))
  local text = trim(verified_by or "")
  if not todo_shape_ok(remaining, text) then
    return nil, TEXTS.beats.settlement_todo_segment_bad
  end
  return { remaining = 0, verified_by = text }, nil
end

-- `artifact|summary[|c=<kept>/<screened>[|cls=<kind>:<n>,...]]
--          [|sessions=<moved>/<remaining>/<destination>][|todo=<remaining>/<verified_by>]`。
-- 本函数只解析载荷形状（{artifact, summary, segments, sessions, todo}），跨字段完整性交
-- normalize_evolution；c / cls 缺段才回落 settings。显式空段、ev=、重复段与未知段都在这里拒绝。
local function parse_settlement_evolution(payload)
  if type(payload) ~= "string" then
    return nil, fill(TEXTS.beats.bad_payload, { action = "settlement.evolution" })
  end
  local parts = split(trim(payload), "|")
  if #parts < 2 then
    return nil, TEXTS.beats.settlement_evolution_usage
  end
  local artifact = trim(parts[1] or "")
  local summary = trim(parts[2] or "")
  if summary == "" then
    return nil, TEXTS.beats.settlement_evolution_need_summary
  end
  local segments = {}
  local sessions, todo = nil, nil
  for i = 3, #parts do
    local segment = trim(parts[i])
    local key, value = cut(segment, "=")
    key = trim(key or "")
    value = trim(value or "")
    if key ~= "c" and key ~= "cls" and key ~= "sessions" and key ~= "todo" then
      return nil, fill(TEXTS.beats.settlement_evolution_bad_segment, { segment = segment })
    end
    if segments[key] ~= nil then
      return nil, TEXTS.beats.settlement_evolution_duplicate_segment
    end
    if value == "" then
      if key == "c" then
        return nil, TEXTS.beats.review_evolution_bad_candidates
      elseif key == "cls" then
        return nil, TEXTS.beats.review_evolution_bad_classes
      elseif key == "sessions" then
        return nil, TEXTS.beats.settlement_sessions_segment_bad
      end
      return nil, TEXTS.beats.settlement_todo_segment_bad
    end
    segments[key] = value
    if key == "sessions" then
      local parsed, segment_problem = parse_sessions_segment(value)
      if parsed == nil then
        return nil, segment_problem
      end
      sessions = parsed
    elseif key == "todo" then
      local parsed, segment_problem = parse_todo_segment(value)
      if parsed == nil then
        return nil, segment_problem
      end
      todo = parsed
    end
  end
  return { artifact = artifact, summary = summary, segments = segments, sessions = sessions, todo = todo }, nil
end

-- 菜单构造 review.note 载荷：`lesson|score` 两段，进化声明不从这里走。
local function format_review_note(lesson, score)
  return join({ lesson, score }, "|")
end

local function parse_review_note(payload)
  if type(payload) ~= "string" then
    return nil, fill(TEXTS.beats.bad_payload, { action = "review.note" })
  end
  local parts = split(trim(payload), "|")
  if #parts > 2 then
    -- 旧版把进化声明塞进 note（ev= / c= / cls=）：一律拒绝，进化只走 settlement.evolution。
    for i = 3, #parts do
      local key = cut(trim(parts[i]), "=")
      key = trim(key or "")
      if key == "ev" or key == "c" or key == "cls" then
        return nil, TEXTS.beats.review_note_legacy_evolution
      end
    end
    return nil, TEXTS.beats.review_note_usage
  end
  return { lesson = trim(parts[1] or ""), score_text = trim(parts[2] or "") }, nil
end

-- ============================================================================
-- §E2 登记面：goal 授权 + 执行者账本（`config.set` 的两个键族）
--
-- 两个都是**自声明**：状态机只能校形状与「换人必给原因」，读不到会话，也不验证 id 真伪。
-- 落点：`settings.goal.authorization` / `settings.executor.<rN>.<stage>`（`settings.*` 唯一写方 = config.set）。
-- ============================================================================
local EXECUTOR_STAGES = { "plan", "development" } -- R12：只剩两个活跃阶段，analysis 槽位删除

local GOAL_AUTH_MAX = 200

local function parse_goal_auth(value)
  local raw = trim(value or "")
  if raw == "" then
    return "", nil
  end
  if #raw > GOAL_AUTH_MAX or raw:find("[\r\n]") ~= nil then
    return nil, TEXTS.beats.goal_auth_bad
  end
  return raw, nil
end

local function executor_stage_ok(stage)
  return contains(EXECUTOR_STAGES, stage)
end

-- `executor.rN.<stage>` → req_id, stage；其它形状 → nil（由调用方报可读错误）。
local function parse_executor_key(key)
  if type(key) ~= "string" then
    return nil, nil
  end
  return key:match("^executor%.(r%d+)%.([a-z_]+)$")
end

local function format_executor(id, reason)
  if reason ~= nil and reason ~= "" then
    return id .. "|" .. reason
  end
  return id
end

-- 只取 id 部分（忽略 `|原因`）：列表展示与前序阶段比较都用它。
local function executor_id_of(raw)
  local id = cut(trim(raw or ""), "|")
  return trim(id or "")
end

M.EXECUTOR_STAGES = EXECUTOR_STAGES
M.executor_id_of = executor_id_of
M.executor_stage_ok = executor_stage_ok
M.format_executor = format_executor
M.parse_executor_key = parse_executor_key
M.parse_goal_auth = parse_goal_auth
M.format_review_note = format_review_note
M.parse_review_note = parse_review_note
M.parse_sessions_payload = parse_sessions_payload
M.parse_sessions_segment = parse_sessions_segment
M.parse_settlement_evolution = parse_settlement_evolution
M.parse_todo_payload = parse_todo_payload
M.parse_todo_segment = parse_todo_segment
M.sessions_shape_ok = sessions_shape_ok
M.todo_shape_ok = todo_shape_ok

return M
