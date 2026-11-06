-- src/requirements.lua —— 协调器阶段 + 需求图（`req.*`）：状态常量、访问器、写入、依赖校验、摘牌唯一出口。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 资源由入口 install(deps) 注入。本模块唯一写的 KV 前缀：`req.*`（唯一写方）。

local M = {}

local config = require("src.config")
local util = require("src.util")

local TEXTS
local LANES
local PROPS

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

-- 存储态只有 5 个 + 3 个终态标记（split / merged / dropped 不并入 done）。
-- 旧的 14 名（analysis_review / plan_review / plan_gate / ready / dev_review / accept）
-- 不再落存储，改为 `req_phase_name()` 由「存储态 + job / pending 提案 / gate / plan_ok」派生，
-- 让回执、图例与面板数字逐字保持原口径。
local R_DRAFT = "draft"

local R_ANALYZING = "analyzing"

local R_PLANNING = "planning"

local R_DEVELOPING = "developing"

local R_DONE = "done"

-- 展示名（14 个旧名，只出现在渲染面）：PHASE 是唯一来源，render 的字形表以它为键。
local PHASE = {
  draft = "draft",
  analyzing = "analyzing",
  analysis_review = "analysis_review",
  planning = "planning",
  plan_review = "plan_review",
  plan_gate = "plan_gate",
  ready = "ready",
  developing = "developing",
  dev_review = "dev_review",
  accept = "accept",
  done = "done",
  split = "split",
  merged = "merged",
  dropped = "dropped",
}

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

-- 冲突的来源（"human" / "radar"；空 = 无冲突或旧存档）。纯记录面：不参与任何判定。
local function req_conflict_source(index)
  return text_of(req_key(index, "conflict_source"), "")
end

local function req_needs_amend(index)
  return ygg.kv.get(req_key(index, "needs_amend")) == true
end

local function req_stale(index)
  return ygg.kv.get(req_key(index, "stale")) == true
end

-- 门禁种类（"" = 不在门禁上）。5 态下它替代旧的 plan_gate / accept 两个状态。
local function req_gate(index)
  local value = ygg.kv.get(req_key(index, "gate"))
  if type(value) == "string" then
    return value
  end
  return ""
end

-- plan_ok 三态读取：`true` / `false` = 显式写入（新模型），`nil` = **字段缺席**（本模型之前创建的
-- 在途行，旧版从不写这个键）。升级时两者的区别是判别性的：把缺席当 false 会让升级前就在
-- developing 的需求永久卡住（提交 development 提案被拒）。
local function req_plan_flag(index)
  local value = ygg.kv.get(req_key(index, "plan_ok"))
  if value == true or value == false then
    return value
  end
  return nil
end

-- 逻辑判定（推进路径用）：显式值优先；缺席时按状态推断——旧模型下能进 developing 或
-- 已经 done 的行，必定已过 plan 门禁。draft / analyzing / planning 视为未批准。
local function req_plan_ok(index)
  local flag = req_plan_flag(index)
  if flag ~= nil then
    return flag
  end
  local status = req_status(index)
  return status == R_DEVELOPING or status == R_DONE
end

local function req_job(index)
  return text_of(req_key(index, "job"), "")
end

-- 待验收提案（读 prop.*；由 install 注入，避免 requirements ← proposals 的反向 require）。
-- stage_name 为 nil 表示任一阶段。
local function req_has_pending(index, stage_name)
  if PROPS == nil then
    return false
  end
  for i = 1, PROPS.prop_count() do
    if PROPS.prop_status(i) == PROPS.P_PENDING and PROPS.prop_req(i) == index then
      if stage_name == nil or PROPS.prop_stage(i) == stage_name then
        return true
      end
    end
  end
  return false
end

-- 派生展示名（14 个旧名之一）；不含 blocked 覆盖层（那是 req_display 的事）。
local function req_phase_name(index)
  local status = req_status(index)
  if status == R_DRAFT then
    return PHASE.draft
  end
  if status == R_ANALYZING then
    if req_has_pending(index, "analysis") then
      return PHASE.analysis_review
    end
    return PHASE.analyzing
  end
  if status == R_PLANNING then
    if req_gate(index) == "plan" then
      return PHASE.plan_gate
    end
    -- 兼容槽位：planning 态下仍然可以收 legacy 的 analysis 提案（门禁与 plan_ok 都未落），
    -- 它得先在面板上显示成「待验收分析」而不是「规划中」。
    if req_has_pending(index, "analysis") then
      return PHASE.analysis_review
    end
    if req_plan_ok(index) then
      return PHASE.ready
    end
    if req_has_pending(index, "plan") then
      return PHASE.plan_review
    end
    return PHASE.planning
  end
  if status == R_DEVELOPING then
    if req_gate(index) == "accept" then
      return PHASE.accept
    end
    if req_has_pending(index, "development") then
      return PHASE.dev_review
    end
    return PHASE.developing
  end
  return status
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
    if req_plan_ok(i) and not req_dev_ready(i) then
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
  return req_status(index) == R_PLANNING
    and req_plan_ok(index)
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
  return req_phase_name(index)
end

local function req_closed(index)
  return R_TERMINAL[req_status(index)] == true
end

-- 计数口径 = 派生展示名（旧 14 名）：analysis_review 归 analyzing、plan_review/plan_gate/ready
-- 各算各的，与合并前 `req_tally(A) + req_tally(B)` 的求和逐字一致。
local function req_tally(phase_name)
  local total = 0
  for i = 1, req_count() do
    if req_phase_name(i) == phase_name then
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

-- 可派活的阶段；nil = 不可派。两个「不可派」的来源必须都在这里表达：
--   ① 门禁待批（gate ~= ""）——5 态下 planning 同时覆盖 plan 阶段与 plan_gate，
--      缺这条守卫会把门禁待批的需求当成 plan 候选派出去；
--   ② analyzing 不返回 analysis——与旧版一致（分析 job 一旦提交提案就不再重派，
--      避免「重定基」被派活路径绕过）。
local function req_job_stage(index)
  local status = req_status(index)
  if status == R_DRAFT then
    -- analysis + plan 已合并：draft 直接派 plan（存储态同时推进到 planning）。
    -- `analyzing` 降级为兼容槽位（legacy 存档 / 显式 analysis 提案），不再由派活路径产生。
    return "plan"
  end
  if status == R_PLANNING then
    if req_gate(index) ~= "" then
      return nil
    end
    if req_plan_ok(index) then
      return "development"
    end
    return "plan"
  end
  return nil
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
  ygg.kv.set(req_key(index, "conflict_note"), "")
  ygg.kv.set(req_key(index, "conflict_source"), "")
  ygg.kv.set(req_key(index, "needs_amend"), false)
  ygg.kv.set(req_key(index, "stale"), false)
  ygg.kv.set(req_key(index, "gate"), "")
  ygg.kv.set(req_key(index, "gate_rev"), 0)
  ygg.kv.set(req_key(index, "plan_ok"), false)
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
  -- 计划随 rev 一起作废：旧的 ready 上 amend 仍能派开发（用陈旧计划），这里是收紧。
  ygg.kv.set(req_key(index, "plan_ok"), false)
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

-- 验收后推进：analysis → planning；plan → planning + gate="plan"；development → developing + gate="accept"。
-- 门禁绑定 revision 也在这里写（`req.*` 只有本模块写），待批期间 amend → gate_rev ≠ rev → 不得批准。
local function req_advance_after_accept(index, stage_name)
  local next_status, next_gate
  if stage_name == "analysis" then
    next_status, next_gate = R_PLANNING, ""
  elseif stage_name == "plan" then
    -- plan 完成 ≠ 可以开发：落 plan 门禁，主 Agent 批了才置 plan_ok。
    next_status, next_gate = R_PLANNING, "plan"
  elseif stage_name == "development" then
    -- 执行方交付 ≠ 已验收：落 accept 门禁。
    next_status, next_gate = R_DEVELOPING, "accept"
  else
    return nil
  end
  ygg.kv.set(req_key(index, "status"), next_status)
  ygg.kv.set(req_key(index, "gate"), next_gate)
  ygg.kv.set(req_key(index, "gate_rev"), next_gate ~= "" and req_rev(index) or 0)
  ygg.kv.set(req_key(index, "job"), "")
  if next_status == R_DEVELOPING then
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
  PROPS = deps.props
end

M.PHASE = PHASE
M.R_ANALYZING = R_ANALYZING
M.R_DEVELOPING = R_DEVELOPING
M.R_DONE = R_DONE
M.R_DRAFT = R_DRAFT
M.R_DROPPED = R_DROPPED
M.R_MERGED = R_MERGED
M.R_PLANNING = R_PLANNING
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
M.req_conflict_source = req_conflict_source
M.req_conflict_total = req_conflict_total
M.req_count = req_count
M.req_deps = req_deps
M.req_deps_ok = req_deps_ok
M.req_deps_text = req_deps_text
M.req_dev_blocked_text = req_dev_blocked_text
M.req_dev_ready = req_dev_ready
M.req_display = req_display
M.req_gate = req_gate
M.req_has_pending = req_has_pending
M.req_id = req_id
M.req_index_of = req_index_of
M.req_job = req_job
M.req_job_stage = req_job_stage
M.req_key = req_key
M.req_lane = req_lane
M.req_needs_amend = req_needs_amend
M.req_open_total = req_open_total
M.req_phase_name = req_phase_name
M.req_plan_flag = req_plan_flag
M.req_plan_ok = req_plan_ok
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
M.req_satisfied = req_satisfied
M.req_validate_deps = req_validate_deps
M.stage = stage

return M
