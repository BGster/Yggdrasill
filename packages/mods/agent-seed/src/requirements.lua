-- src/requirements.lua —— 协调器阶段 + 需求图（`req.*`）：状态常量、访问器、写入、依赖校验、摘牌唯一出口。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 资源由入口 install(deps) 注入。本模块唯一写的 KV 前缀：`req.*`（唯一写方）。

local M = {}

local config = require("src.config")
local util = require("src.util")

local TEXTS
local LANES

local num, text_of =
  config.num, config.text_of
local fill, join, split, trim =
  util.fill, util.join, util.split, util.trim

local STAGE = {
  intake = "intake",
  coordinate = "coordinate",
  review = "review",
  done = "done",
}

local function stage()
  local value = ygg.kv.get("fsm.stage")
  if type(value) ~= "string" then
    return STAGE.intake
  end
  return value
end

local R_DRAFT = "draft"

local R_ANALYZING = "analyzing"

local R_ANALYSIS_REVIEW = "analysis_review"

local R_PLANNING = "planning"

local R_PLAN_REVIEW = "plan_review"

local R_PLAN_GATE = "plan_gate"

local R_READY = "ready"

local R_DEVELOPING = "developing"

local R_DEV_REVIEW = "dev_review"

local R_ACCEPT = "accept"

local R_DONE = "done"

local R_SPLIT = "split"

local R_MERGED = "merged"

local R_DROPPED = "dropped"

local R_TERMINAL = {
  [R_DONE] = true,
  [R_SPLIT] = true,
  [R_MERGED] = true,
  [R_DROPPED] = true,
}

local function req_count()
  return num("req.count", 0)
end

local function req_key(index, name)
  return "req." .. tostring(index) .. "." .. name
end

local function req_id(index)
  return "r" .. tostring(index)
end

local function req_status(index)
  local value = ygg.kv.get(req_key(index, "status"))
  if type(value) == "string" then
    return value
  end
  return ""
end

local function req_title(index)
  return text_of(req_key(index, "title"), TEXTS.beats.default_req_title)
end

local function req_rev(index)
  return num(req_key(index, "rev"), 1)
end

local function req_deps(index)
  local out = {}
  for _, part in ipairs(split(text_of(req_key(index, "deps"), ""), ",")) do
    local piece = trim(part)
    if piece ~= "" then
      out[#out + 1] = piece
    end
  end
  return out
end

local function req_deps_text(index)
  local deps = req_deps(index)
  if #deps == 0 then
    return TEXTS.beats.req_deps_none
  end
  return fill(TEXTS.beats.req_deps_have, { ids = join(deps, ",") })
end

local function req_index_of(id)
  if type(id) ~= "string" then
    return nil
  end
  local raw = trim(id)
  if raw:sub(1, 1) ~= "r" then
    return nil
  end
  local number = tonumber(raw:sub(2))
  if number == nil then
    return nil
  end
  number = math.floor(number)
  if number < 1 or number > req_count() or req_status(number) == "" then
    return nil
  end
  return number
end

local function req_conflict(index)
  return text_of(req_key(index, "conflict"), "")
end

local function req_conflict_peer(index)
  return text_of(req_key(index, "conflict_peer"), "")
end

local function req_needs_amend(index)
  return ygg.kv.get(req_key(index, "needs_amend")) == true
end

local function req_stale(index)
  return ygg.kv.get(req_key(index, "stale")) == true
end

local function req_successors(index)
  local status = req_status(index)
  local raw = ""
  if status == R_SPLIT then
    raw = text_of(req_key(index, "children"), "")
  elseif status == R_MERGED then
    raw = text_of(req_key(index, "into"), "")
  end
  local out = {}
  for _, part in ipairs(split(raw, ",")) do
    local piece = trim(part)
    if piece ~= "" then
      out[#out + 1] = piece
    end
  end
  return out
end

local req_satisfied

req_satisfied = function(index, visiting)
  visiting = visiting or {}
  if visiting[index] then
    return false
  end
  local status = req_status(index)
  if status == R_DONE or status == R_DROPPED then
    return true
  end
  if status == R_SPLIT or status == R_MERGED then
    local successors = req_successors(index)
    if #successors == 0 then
      return false
    end
    visiting[index] = true
    local ok = true
    for _, id in ipairs(successors) do
      local child = req_index_of(id)
      if child == nil or not req_satisfied(child, visiting) then
        ok = false
        break
      end
    end
    visiting[index] = nil
    return ok
  end
  return false
end

local function req_deps_ok(index)
  for _, id in ipairs(req_deps(index)) do
    local dep = req_index_of(id)
    if dep == nil or not req_satisfied(dep, nil) then
      return false
    end
  end
  return true
end

local function req_unmet_deps(index)
  local out = {}
  for _, id in ipairs(req_deps(index)) do
    local dep = req_index_of(id)
    if dep == nil or not req_satisfied(dep, nil) then
      out[#out + 1] = id
    end
  end
  return join(out, ",")
end

local function req_dev_blocked_text()
  local parts = {}
  for i = 1, req_count() do
    if req_status(i) == R_READY and not req_dev_ready(i) then
      if req_conflict(i) ~= "" then
        parts[#parts + 1] = fill(TEXTS.beats.req_why_conflict, { id = req_id(i) })
      elseif req_needs_amend(i) then
        parts[#parts + 1] = fill(TEXTS.beats.req_why_amend, { id = req_id(i) })
      else
        parts[#parts + 1] = fill(TEXTS.beats.req_why_deps, { id = req_id(i), ids = req_unmet_deps(i) })
      end
      if #parts >= 3 then
        break
      end
    end
  end
  if #parts == 0 then
    return TEXTS.beats.req_why_none
  end
  return join(parts, TEXTS.beats.req_why_sep)
end

local function req_dev_ready(index)
  return req_status(index) == R_READY
    and req_conflict(index) == ""
    and not req_needs_amend(index)
    and req_deps_ok(index)
end

local function req_display(index)
  local status = req_status(index)
  if status == "" then
    return "?"
  end
  if req_conflict(index) ~= "" or req_needs_amend(index) then
    if status ~= R_DONE and status ~= R_SPLIT and status ~= R_MERGED and status ~= R_DROPPED then
      return "blocked"
    end
  end
  return status
end

local function req_closed(index)
  return R_TERMINAL[req_status(index)] == true
end

local function req_tally(status)
  local total = 0
  for i = 1, req_count() do
    if req_status(i) == status then
      total = total + 1
    end
  end
  return total
end

local function req_conflict_total()
  local total = 0
  for i = 1, req_count() do
    if (req_conflict(i) ~= "" or req_needs_amend(i)) and not req_closed(i) then
      total = total + 1
    end
  end
  return total
end

local function req_sync_conflicts()
  local total = req_conflict_total()
  ygg.kv.set("req.conflicts", total)
  return total
end

local function req_open_total()
  local total = 0
  for i = 1, req_count() do
    if not req_closed(i) then
      total = total + 1
    end
  end
  return total
end

local function req_all_done()
  for i = 1, req_count() do
    if not req_satisfied(i, nil) then
      return false
    end
  end
  return true
end

local function req_job_stage(index)
  local status = req_status(index)
  if status == R_DRAFT then
    return "analysis"
  elseif status == R_PLANNING then
    return "plan"
  elseif status == R_READY then
    return "development"
  end
  return nil
end

local function req_job(index)
  return text_of(req_key(index, "job"), "")
end

local function req_lane(index)
  return num(req_key(index, "lane"), 0)
end

local function job_total()
  local total = 0
  for i = 1, req_count() do
    if req_job(i) ~= "" then
      total = total + 1
    end
  end
  return total
end

local function job_capacity()
  local value = TEXTS.tuning.job_capacity
  if type(value) == "number" and value > 0 then
    return value
  end
  return 4
end

local function req_capacity()
  local value = TEXTS.tuning.req_capacity
  if type(value) == "number" and value > 0 then
    return value
  end
  return 32
end

local function req_append(title, description, ext, deps, origin)
  local index = req_count() + 1
  ygg.kv.set("req.count", index)
  ygg.kv.set(req_key(index, "title"), title)
  ygg.kv.set(req_key(index, "description"), description)
  ygg.kv.set(req_key(index, "status"), R_DRAFT)
  ygg.kv.set(req_key(index, "ext"), ext or "")
  ygg.kv.set(req_key(index, "deps"), join(deps or {}, ","))
  ygg.kv.set(req_key(index, "rev"), 1)
  ygg.kv.set(req_key(index, "hist"), "")
  ygg.kv.set(req_key(index, "conflict"), "")
  ygg.kv.set(req_key(index, "conflict_peer"), "")
  ygg.kv.set(req_key(index, "needs_amend"), false)
  ygg.kv.set(req_key(index, "stale"), false)
  ygg.kv.set(req_key(index, "job"), "")
  ygg.kv.set(req_key(index, "lane"), 0)
  ygg.kv.set(req_key(index, "artifacts"), "")
  ygg.kv.set(req_key(index, "summary"), "")
  ygg.kv.set(req_key(index, "children"), "")
  ygg.kv.set(req_key(index, "into"), "")
  ygg.kv.set(req_key(index, "origin"), origin or "")
  ygg.kv.set(req_key(index, "proposal"), 0)
  ygg.kv.set("req.last", req_id(index))
  return index
end

local function req_push_history(index, note)
  local entries = {}
  for _, part in ipairs(split(text_of(req_key(index, "hist"), ""), ";")) do
    local piece = trim(part)
    if piece ~= "" then
      entries[#entries + 1] = piece
    end
  end
  entries[#entries + 1] = note
  while #entries > 4 do
    local kept = {}
    for i = 2, #entries do
      kept[#kept + 1] = entries[i]
    end
    entries = kept
  end
  ygg.kv.set(req_key(index, "hist"), join(entries, ";"))
end

local function req_bump_rev(index)
  local rev = req_rev(index) + 1
  ygg.kv.set(req_key(index, "rev"), rev)
  ygg.kv.set(req_key(index, "stale"), false)
  return rev
end

local function req_reaches(from, target, visiting)
  if from == target then
    return true
  end
  visiting = visiting or {}
  if visiting[from] then
    return false
  end
  visiting[from] = true
  for _, id in ipairs(req_deps(from)) do
    local dep = req_index_of(id)
    if dep ~= nil and req_reaches(dep, target, visiting) then
      return true
    end
  end
  return false
end

local function req_validate_deps(index, ids)
  local seen = {}
  for _, id in ipairs(ids) do
    local dep = req_index_of(id)
    if dep == nil then
      return fill(TEXTS.beats.req_bad_dep, { id = tostring(id) })
    end
    if dep == index then
      return fill(TEXTS.beats.req_self_dep, { id = req_id(index) })
    end
    if seen[dep] then
      return fill(TEXTS.beats.req_dup_dep, { id = req_id(dep) })
    end
    seen[dep] = true
    if req_reaches(dep, index, nil) then
      return fill(TEXTS.beats.req_cycle_dep, { id = req_id(index), dep = req_id(dep) })
    end
  end
  return nil
end

local function req_advance_after_accept(index, stage_name)
  local next_status
  if stage_name == "analysis" then
    next_status = R_PLANNING
  elseif stage_name == "plan" then
    -- plan 完成后进**plan_gate**：主 Agent 批了才能 develop（不能自动跳过）。
    next_status = R_PLAN_GATE
  elseif stage_name == "development" then
    -- 执行方交付（accept）≠ 主 Agent 已验收：先落 accept，等 accept_gate。
    next_status = R_ACCEPT
  else
    return nil
  end
  ygg.kv.set(req_key(index, "status"), next_status)
  ygg.kv.set(req_key(index, "job"), "")
  if next_status == R_ACCEPT then
    -- 交付后释放泳道（连同 branch/base/since/clean 一起清）；重做时重新分配（不长期占位）。
    LANES.release(index)
  end
  return next_status
end

local function req_release(index)
  ygg.kv.set(req_key(index, "job"), "")
  LANES.release(index)
end

function M.install(deps)
  TEXTS = deps.texts
  LANES = deps.lanes
end

M.R_ACCEPT = R_ACCEPT
M.R_ANALYSIS_REVIEW = R_ANALYSIS_REVIEW
M.R_ANALYZING = R_ANALYZING
M.R_DEVELOPING = R_DEVELOPING
M.R_DEV_REVIEW = R_DEV_REVIEW
M.R_DONE = R_DONE
M.R_DRAFT = R_DRAFT
M.R_DROPPED = R_DROPPED
M.R_MERGED = R_MERGED
M.R_PLANNING = R_PLANNING
M.R_PLAN_GATE = R_PLAN_GATE
M.R_PLAN_REVIEW = R_PLAN_REVIEW
M.R_READY = R_READY
M.R_SPLIT = R_SPLIT
M.R_TERMINAL = R_TERMINAL
M.STAGE = STAGE
M.job_capacity = job_capacity
M.job_total = job_total
M.req_advance_after_accept = req_advance_after_accept
M.req_all_done = req_all_done
M.req_append = req_append
M.req_bump_rev = req_bump_rev
M.req_capacity = req_capacity
M.req_closed = req_closed
M.req_conflict = req_conflict
M.req_conflict_peer = req_conflict_peer
M.req_conflict_total = req_conflict_total
M.req_count = req_count
M.req_deps = req_deps
M.req_deps_ok = req_deps_ok
M.req_deps_text = req_deps_text
M.req_dev_blocked_text = req_dev_blocked_text
M.req_dev_ready = req_dev_ready
M.req_display = req_display
M.req_id = req_id
M.req_index_of = req_index_of
M.req_job = req_job
M.req_job_stage = req_job_stage
M.req_key = req_key
M.req_lane = req_lane
M.req_needs_amend = req_needs_amend
M.req_open_total = req_open_total
M.req_push_history = req_push_history
M.req_reaches = req_reaches
M.req_release = req_release
M.req_rev = req_rev
M.req_stale = req_stale
M.req_status = req_status
M.req_successors = req_successors
M.req_sync_conflicts = req_sync_conflicts
M.req_tally = req_tally
M.req_title = req_title
M.req_unmet_deps = req_unmet_deps
M.req_validate_deps = req_validate_deps
M.satisfied = satisfied
M.stage = stage

return M
