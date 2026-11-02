-- src/wire.lua —— payload wire format 解析 + inbox 幂等键（`@ext=`）+ settlement.evolution 进化登记。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 资源由入口 install(deps) 注入。本模块唯一写的 KV 前缀：无（只读）。

local M = {}

local config = require("src.config")
local reqs = require("src.requirements")
local util = require("src.util")

local TEXTS

local effective, mode, text_of =
  config.effective, config.mode, config.text_of
local stage =
  reqs.stage
local contains, cut, fill, join, split, trim =
  util.contains, util.cut, util.fill, util.join, util.split, util.trim

local function probe_payload_ok(payload)
  if payload == nil then
    return true
  end
  if type(payload) == "number" then
    return payload == 0
  end
  if type(payload) == "string" then
    local raw = trim(payload)
    return raw == "" or raw == "0" or raw == "full"
  end
  return false
end

local function valid_ext(ext)
  if type(ext) ~= "string" then
    return false
  end
  local raw = trim(ext)
  if raw == "" or #raw > 64 then
    return false
  end
  return raw:find("%s") == nil
end

local function parse_intake(payload)
  if type(payload) ~= "string" then
    return nil, nil, fill(TEXTS.beats.bad_payload, { action = "task.intake" })
  end
  local raw = trim(payload)
  if raw == "" then
    return TEXTS.beats.default_task_title, TEXTS.beats.default_task_desc, nil
  end
  local title, description = cut(raw, "|")
  title = trim(title)
  description = trim(description or "")
  if title == "" then
    title = TEXTS.beats.default_task_title
  end
  if description == "" then
    description = TEXTS.beats.default_task_desc
  end
  return title, description, nil
end

local function parse_deps(raw)
  local out = {}
  if type(raw) ~= "string" then
    return out
  end
  local raw_text = trim(raw)
  if raw_text == "" or raw_text == "-" then
    return out
  end
  for _, part in ipairs(split(raw_text, ",")) do
    local piece = trim(part)
    if piece ~= "" then
      out[#out + 1] = piece
    end
  end
  return out
end

local function parse_requirement(payload)
  if type(payload) ~= "string" then
    return nil, nil, nil, nil, fill(TEXTS.beats.bad_payload, { action = "requirement.add" })
  end
  local raw = trim(payload)
  if raw == "" then
    return TEXTS.beats.default_req_title, TEXTS.beats.default_req_desc, "", {}, nil
  end
  local parts = split(raw, "|")
  local title = trim(parts[1] or "")
  local description = trim(parts[2] or "")
  local ext = ""
  local dep_raw = ""
  if #parts >= 3 then
    if valid_ext(parts[3]) then
      ext = trim(parts[3])
      dep_raw = trim(parts[4] or "")
    else
      description = trim(description .. "|" .. trim(parts[3]))
    end
  end
  if title == "" then
    title = TEXTS.beats.default_req_title
  end
  if description == "" then
    description = TEXTS.beats.default_req_desc
  end
  return title, description, ext, parse_deps(dep_raw), nil
end

local function strip_ext(payload)
  if type(payload) ~= "string" then
    return payload, ""
  end
  local marker = "|@ext="
  local index = payload:find(marker, 1, true)
  if index == nil then
    return payload, ""
  end
  return payload:sub(1, index - 1), trim(payload:sub(index + #marker))
end

local function inbox_replay(ext)
  if type(ext) ~= "string" or ext == "" then
    return false
  end
  if text_of("inbox.last", "") ~= ext then
    return false
  end
  ygg.narrate(fill(TEXTS.beats.req_replayed_op, { ext = ext }))
  return true
end

local function inbox_mark(ext)
  if type(ext) == "string" and ext ~= "" then
    ygg.kv.set("inbox.last", ext)
  end
end

local function parse_proposal(payload)
  if type(payload) ~= "string" then
    return nil, fill(TEXTS.beats.bad_payload, { action = "proposal.submit" })
  end
  local parts = split(trim(payload), "|")
  if #parts < 4 then
    return nil, TEXTS.beats.req_prop_usage
  end
  local req_id_text = trim(parts[1])
  local rev = tonumber(trim(parts[2] or ""))
  local stage_name = trim(parts[3] or "")
  if rev == nil or not contains({ "analysis", "plan", "development" }, stage_name) then
    return nil, TEXTS.beats.req_prop_usage
  end
  return {
    req = req_id_text,
    rev = math.floor(rev),
    stage = stage_name,
    summary = trim(parts[4] or ""),
    artifacts = trim(parts[5] or ""),
    changes = trim(parts[6] or ""),
    ext = trim(parts[7] or ""),
  }, nil
end

local function parse_split(payload)
  if type(payload) ~= "string" then
    return nil, nil, fill(TEXTS.beats.bad_payload, { action = "requirement.split" })
  end
  local parts = split(trim(payload), "|")
  local parent = trim(parts[1] or "")
  local children = {}
  for i = 2, #parts do
    local title = trim(parts[i])
    if title ~= "" then
      children[#children + 1] = title
    end
  end
  if parent == "" or #children < 2 then
    return nil, nil, TEXTS.beats.req_split_usage
  end
  return parent, children, nil
end

local function parse_merge(payload)
  if type(payload) ~= "string" then
    return nil, nil, nil, fill(TEXTS.beats.bad_payload, { action = "requirement.merge" })
  end
  local parts = split(trim(payload), "|")
  local ids = {}
  for _, part in ipairs(split(trim(parts[1] or ""), ",")) do
    local piece = trim(part)
    if piece ~= "" then
      ids[#ids + 1] = piece
    end
  end
  if #ids < 2 then
    return nil, nil, nil, TEXTS.beats.req_merge_usage
  end
  return ids, trim(parts[2] or ""), trim(parts[3] or ""), nil
end

local function parse_amend(payload)
  if type(payload) ~= "string" then
    return nil, nil, nil, nil, fill(TEXTS.beats.bad_payload, { action = "requirement.amend" })
  end
  local parts = split(trim(payload), "|")
  local id = trim(parts[1] or "")
  if id == "" then
    return nil, nil, nil, nil, TEXTS.beats.req_amend_usage
  end
  return id, trim(parts[2] or ""), trim(parts[3] or ""), trim(parts[4] or ""), nil
end

local function parse_conflict(payload)
  if type(payload) ~= "string" then
    return nil, nil, fill(TEXTS.beats.bad_payload, { action = "requirement.conflict" })
  end
  local parts = split(trim(payload), "|")
  local ids = {}
  for _, part in ipairs(split(trim(parts[1] or ""), ",")) do
    local piece = trim(part)
    if piece ~= "" then
      ids[#ids + 1] = piece
    end
  end
  if #ids ~= 2 then
    return nil, nil, TEXTS.beats.req_conflict_usage
  end
  local note = trim(parts[2] or "")
  if note == "" then
    note = TEXTS.beats.req_conflict_default
  end
  return ids, note, nil
end

local RESOLVE_MODES = { "keep_both", "keep_first", "revise" }

local function parse_resolve(payload)
  if type(payload) ~= "string" then
    return nil, nil, nil, fill(TEXTS.beats.bad_payload, { action = "requirement.resolve" })
  end
  local parts = split(trim(payload), "|")
  local ids = {}
  for _, part in ipairs(split(trim(parts[1] or ""), ",")) do
    local piece = trim(part)
    if piece ~= "" then
      ids[#ids + 1] = piece
    end
  end
  local mode = trim(parts[2] or "")
  if #ids ~= 2 or not contains(RESOLVE_MODES, mode) then
    return nil, nil, nil, fill(TEXTS.beats.req_resolve_usage, { modes = join(RESOLVE_MODES, " / ") })
  end
  return ids, mode, trim(parts[3] or ""), nil
end

-- ============================================================================
-- §settlement.evolution 进化登记 + review.note 复盘载荷
-- （artifact|summary[|c=<kept>/<screened>[|cls=<kind>:<n>,...]]）
--
-- 声明 ≠ 证明：这里只校验**形状与完整性**（summary、路径形状、两个整数、六类计数自洽、
-- screened > 0 时 artifact 与 cls 都不得缺），沙箱读不到文件，也不判断建议质量或复核是否真的独立执行过。
-- 分两层：parse_settlement_evolution 只认载荷形状，normalize_evolution 校跨字段完整性。
-- ============================================================================
local EVOLUTION_CLASSES = { "long_term", "unused", "one_off", "temp_tool", "redundant", "no_scene" }

-- 允许进入 backlog 的三类：长期机制 / 已有未用（需根因）/ 冗余退役。
local EVOLUTION_KEPT = { long_term = true, unused = true, redundant = true }

local function evolution_path_ok(path)
  if type(path) ~= "string" then
    return false
  end
  local raw = trim(path)
  if raw == "" or #raw > 128 then
    return false
  end
  if raw:sub(1, 10) ~= "artifacts/" or raw:sub(-3) ~= ".md" then
    return false
  end
  if #raw <= 13 or raw:find("..", 1, true) ~= nil or raw:find("%s") ~= nil then
    return false
  end
  return true
end

local function whole_number(value)
  local parsed = tonumber(value)
  if parsed == nil or parsed ~= math.floor(parsed) then
    return nil
  end
  return math.floor(parsed)
end

local function parse_candidates(text)
  local kept_text, screened_text = cut(trim(text), "/")
  local kept = whole_number(trim(kept_text or ""))
  local screened = whole_number(trim(screened_text or ""))
  if kept == nil or screened == nil then
    return nil
  end
  if kept < 0 or screened < 0 or kept > screened then
    return nil
  end
  return kept, screened
end

local function parse_classes(text)
  local out = {}
  local total = 0
  for _, part in ipairs(split(trim(text), ",")) do
    local piece = trim(part)
    if piece ~= "" then
      local kind, count_text = cut(piece, ":")
      kind = trim(kind or "")
      local count = whole_number(trim(count_text or ""))
      if not contains(EVOLUTION_CLASSES, kind) or count == nil or count < 0 or out[kind] ~= nil then
        return nil
      end
      out[kind] = count
      total = total + count
    end
  end
  return out, total
end

local function format_classes(classes)
  local out = {}
  for i = 1, #EVOLUTION_CLASSES do
    local kind = EVOLUTION_CLASSES[i]
    local count = classes[kind]
    if type(count) == "number" then
      out[#out + 1] = kind .. ":" .. tostring(count)
    end
  end
  return join(out, ",")
end

local function kept_total(classes)
  local total = 0
  for i = 1, #EVOLUTION_CLASSES do
    local kind = EVOLUTION_CLASSES[i]
    if EVOLUTION_KEPT[kind] == true then
      total = total + (classes[kind] or 0)
    end
  end
  return total
end

-- 四路输入一律是字符串（缺省 = ""）：artifact 可空；summary 去空白后必填；
-- candidates 空 = 0/0；classes 空 = 未分类。config.set 只逐键校形状，
-- 跨字段完整性集中在这里校（预登记阶段允许分次登记）。
-- 完整性是结算登记硬条件：screened > 0 就必须同时有复核 artifact 与逐类计数——
-- 「筛了几条、留了几分」而把复核记录与根因分类留在散文里，等于没登记。
local function normalize_evolution(artifact, summary, candidates, classes)
  local record = { artifact = "", summary = "", kept = 0, screened = 0, classes = {} }
  local summary_text = trim(summary or "")
  if summary_text == "" then
    return nil, TEXTS.beats.settlement_evolution_missing
  end
  record.summary = summary_text
  local artifact_text = trim(artifact or "")
  if artifact_text ~= "" then
    if not evolution_path_ok(artifact_text) then
      return nil, TEXTS.beats.review_evolution_bad_path
    end
    record.artifact = artifact_text
  end
  local candidates_text = trim(candidates or "")
  if candidates_text ~= "" then
    local kept, screened = parse_candidates(candidates_text)
    if kept == nil then
      return nil, TEXTS.beats.review_evolution_bad_candidates
    end
    record.kept, record.screened = kept, screened
  end
  local classes_text = trim(classes or "")
  local parsed, total = nil, 0
  if classes_text ~= "" then
    parsed, total = parse_classes(classes_text)
    if parsed == nil or total == 0 then
      return nil, TEXTS.beats.review_evolution_bad_classes
    end
  end
  if record.screened > 0 then
    if record.artifact == "" then
      return nil, TEXTS.beats.review_evolution_need_artifact
    end
    if parsed == nil then
      return nil, TEXTS.beats.review_evolution_need_classes
    end
  end
  if parsed ~= nil then
    if total ~= record.screened or kept_total(parsed) ~= record.kept then
      return nil, TEXTS.beats.review_evolution_mismatch
    end
    record.classes = parsed
  end
  return record, nil
end

-- 预登记面（`config.set review.evolution.*` → `settings.review.evolution.*`）与菜单载荷读同一份口径。
local function evolution_settings()
  local function read(name)
    local value = effective(name)
    if type(value) == "string" then
      return value
    end
    return ""
  end
  return read("review.evolution.artifact"), read("review.evolution.summary"),
    read("review.evolution.candidates"), read("review.evolution.classes")
end

-- 菜单构造结算载荷：零候选也有中性 summary 与显式 c=0/0，不需要伪 artifact。
local function format_settlement_evolution(artifact, summary, candidates, classes)
  local summary_text = trim(summary or "")
  if summary_text == "" then
    summary_text = TEXTS.beats.settlement_evolution_default_summary
  end
  local candidates_text = trim(candidates or "")
  local parts = {
    trim(artifact or ""),
    summary_text,
    "c=" .. (candidates_text ~= "" and candidates_text or "0/0"),
  }
  local classes_text = trim(classes or "")
  if classes_text ~= "" then
    parts[#parts + 1] = "cls=" .. classes_text
  end
  return join(parts, "|")
end

-- `artifact|summary[|c=<kept>/<screened>[|cls=<kind>:<n>,...]]`。
-- 本函数只解析载荷形状（{artifact, summary, segments}），跨字段完整性交 normalize_evolution；
-- c / cls 缺段才回落 settings。显式空段、ev=、重复段与未知段都在这里拒绝，未知段点名违规片段。
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
  for i = 3, #parts do
    local segment = trim(parts[i])
    local key, value = cut(segment, "=")
    key = trim(key or "")
    value = trim(value or "")
    if key ~= "c" and key ~= "cls" then
      return nil, fill(TEXTS.beats.settlement_evolution_bad_segment, { segment = segment })
    end
    if segments[key] ~= nil then
      return nil, TEXTS.beats.settlement_evolution_duplicate_segment
    end
    if value == "" then
      if key == "c" then
        return nil, TEXTS.beats.review_evolution_bad_candidates
      end
      return nil, TEXTS.beats.review_evolution_bad_classes
    end
    segments[key] = value
  end
  return { artifact = artifact, summary = summary, segments = segments }, nil
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
local EXECUTOR_STAGES = { "analysis", "plan", "development" }

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

-- 值形：`<id>` 或 `<id>|<原因>`；`""` = 显式解绑。
local function parse_executor_value(value)
  local raw = trim(value or "")
  if raw == "" then
    return "", "", nil
  end
  local id, reason = cut(raw, "|")
  id = trim(id or "")
  reason = trim(reason or "")
  if id == "" or #id > 64 or id:find("%s") ~= nil then
    return nil, nil, TEXTS.beats.executor_bad_id
  end
  return id, reason, nil
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

function M.install(deps)
  TEXTS = deps.texts
end

M.EVOLUTION_CLASSES = EVOLUTION_CLASSES
M.EXECUTOR_STAGES = EXECUTOR_STAGES
M.executor_id_of = executor_id_of
M.executor_stage_ok = executor_stage_ok
M.format_executor = format_executor
M.parse_executor_key = parse_executor_key
M.parse_executor_value = parse_executor_value
M.parse_goal_auth = parse_goal_auth
M.EVOLUTION_KEPT = EVOLUTION_KEPT
M.evolution_path_ok = evolution_path_ok
M.evolution_settings = evolution_settings
M.format_classes = format_classes
M.format_review_note = format_review_note
M.format_settlement_evolution = format_settlement_evolution
M.kept_total = kept_total
M.normalize_evolution = normalize_evolution
M.parse_candidates = parse_candidates
M.parse_classes = parse_classes
M.parse_review_note = parse_review_note
M.parse_settlement_evolution = parse_settlement_evolution

M.RESOLVE_MODES = RESOLVE_MODES
M.inbox_mark = inbox_mark
M.inbox_replay = inbox_replay
M.parse_amend = parse_amend
M.parse_conflict = parse_conflict
M.parse_deps = parse_deps
M.parse_intake = parse_intake
M.parse_merge = parse_merge
M.parse_proposal = parse_proposal
M.parse_requirement = parse_requirement
M.parse_resolve = parse_resolve
M.parse_split = parse_split
M.probe_payload_ok = probe_payload_ok
M.strip_ext = strip_ext
M.valid_ext = valid_ext

return M
