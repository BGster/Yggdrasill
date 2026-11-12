-- src/domain/requirements.lua —— 协调器阶段 + 需求图（`req.*`）：状态常量、访问器、写入、依赖校验、摘牌唯一出口。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 资源由入口 install(deps) 注入。本模块唯一写的 KV 前缀：`req.*`（唯一写方）。

local M = {}

local config = require("src.core.config")
local util = require("src.core.util")

local TEXTS
local LANES
local PROPS

local num, text_of =
  config.num, config.text_of
local fill, join, split, trim =
  util.fill, util.join, util.split, util.trim

local STAGE = {
  live = "live",
  retired = "retired",
}

local function stage()
  local value = ygg.kv.get("fsm.stage")
  if type(value) ~= "string" or value == "" then
    return STAGE.live
  end
  return value
end

-- D-MA23 双层剧情：需求真状态只有 `open` / `done` 两个；`split / merged / dropped`
-- 降为 `closed_reason` 字段（R11），终态判定 = `status=="done"`。
local R_OPEN = "open"

local R_DONE = "done"

-- 微状态英文标识符（唯一来源；menus / render / beats 图例全部读它）：
-- draft / planning / plan_gate / ready / developing / dev_review / accept_gate /
-- integrating / done / closed（closed 由 closed_reason 区分旁路）。
local PHASE = {
  draft = "draft",
  planning = "planning",
  plan_gate = "plan_gate",
  ready = "ready",
  developing = "developing",
  dev_review = "dev_review",
  accept_gate = "accept_gate",
  integrating = "integrating",
  done = "done",
  closed = "closed",
}

local CLOSED_SPLIT = "split"

local CLOSED_MERGED = "merged"

local CLOSED_DROPPED = "dropped"

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

-- plan_ok 逻辑判定（推进路径用）：显式值优先；字段缺席（旧存档兼容）视为 false——
-- 双层剧情下 status 不再区分进行中各态，缺席推断的旧链路（developing/done 视为已过门）
-- 随旧存档不兼容声明（r9 升级纪律：旧局不开、不迁移）一并退役。
local function req_plan_ok(index)
  return ygg.kv.get(req_key(index, "plan_ok")) == true
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

-- 关闭原因（R11：split / merged / dropped 降为字段，不再是状态）。
local function req_closed_reason(index)
  return text_of(req_key(index, "closed_reason"), "")
end

-- 需求级微状态判定（单一来源，DESIGN §2.5 判据顺序即优先级）：
--   closed ← status ~= open（closed_reason=="" → done（第9幕），否则 closed（旁路））；
--   integrating → 集成；gate==accept → 验收门禁；pending development → 交付受理；
--   job==development → 开发中；plan_ok → 待开发；gate==plan → 计划门禁；
--   pending plan 或 job==plan → 计划中；否则立项。
-- 关键点：待开发先于计划门禁与计划中（plan_ok=true 时遗留 pending 也判待开发，
-- 保证三条打回路径落点一致）；closed 不是幕名，由 closed_reason 区分。
local function req_phase(index)
  if req_status(index) ~= R_OPEN then
    if req_closed_reason(index) == "" then
      return PHASE.done
    end
    return PHASE.closed
  end
  if ygg.kv.get(req_key(index, "integrating")) == true then
    return PHASE.integrating
  end
  if req_gate(index) == "accept" then
    return PHASE.accept_gate
  end
  if req_has_pending(index, "development") then
    return PHASE.dev_review
  end
  if req_job(index) == "development" then
    return PHASE.developing
  end
  if req_plan_ok(index) then
    return PHASE.ready
  end
  if req_gate(index) == "plan" then
    return PHASE.plan_gate
  end
  if req_has_pending(index, "plan") or req_job(index) == "plan" then
    return PHASE.planning
  end
  return PHASE.draft
end

-- 运行级微状态标识（单一来源；menus / render / choice / beats 全部读它）：
-- charter 立局 / coordinate 协调 / settle 盘点 / release 发布 / retired 退役。
-- KV 上的 `run.phase` 取值 "charter"/""/"settle"/"release"（"" = 常驻主态协调，R2）；
-- 标识层把 "" 规范化为 coordinate——空串不能当菜单与渲染的表键。
local RUN_PHASE = {
  charter = "charter",
  coordinate = "coordinate",
  settle = "settle",
  release = "release",
  retired = "retired",
}

-- 微状态中文名（文案面专用：拒绝文案 / 图例里的「微状态用中文名」）：
-- 代码标识符 → 展示名，仅作 fill 文案用，不参与任何判定。
local RUN_PHASE_LABEL = {
  [RUN_PHASE.charter] = "立局",
  [RUN_PHASE.coordinate] = "协调",
  [RUN_PHASE.settle] = "盘点",
  [RUN_PHASE.release] = "发布",
  [RUN_PHASE.retired] = "退役",
}

local function run_phase()
  if stage() == STAGE.retired then
    return RUN_PHASE.retired
  end
  local p = text_of("run.phase", RUN_PHASE.charter)
  if p == RUN_PHASE.settle then
    return RUN_PHASE.settle
  end
  if p == RUN_PHASE.release then
    return RUN_PHASE.release
  end
  if p == RUN_PHASE.charter then
    return RUN_PHASE.charter
  end
  -- "" 与一切未知取值 → 协调（常驻主态；fail-visible 但不崩溃）。
  return RUN_PHASE.coordinate
end

local function run_phase_label(phase_id)
  return RUN_PHASE_LABEL[phase_id] or phase_id
end

local function req_successors(index)
  local reason = req_closed_reason(index)
  local raw = ""
  if reason == CLOSED_SPLIT then
    raw = text_of(req_key(index, "children"), "")
  elseif reason == CLOSED_MERGED then
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
  if status ~= R_DONE then
    return false
  end
  -- R11：req_satisfied 按 closed_reason 走 successor 闭包——done（交付）与 dropped 直接满足；
  -- split / merged 必须全部后继需求满足才满足（无后继 = 不满足）。
  local reason = req_closed_reason(index)
  if reason == CLOSED_SPLIT or reason == CLOSED_MERGED then
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
  return true
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

-- 开发就绪 = 待开发微状态 ∧ 无冲突 ∧ 无 needs_amend ∧ 依赖已满足。
local function req_dev_ready(index)
  return req_status(index) == R_OPEN
    and req_plan_ok(index)
    and req_gate(index) == ""
    and req_job(index) == ""
    and not req_has_pending(index)
    and req_conflict(index) == ""
    and not req_needs_amend(index)
    and req_deps_ok(index)
end

local function req_display(index)
  local status = req_status(index)
  if status == "" then
    return "?"
  end
  -- blocked（冲突 / needs_amend）是叠加在 req_phase 之上的覆盖层，不是微状态。
  if req_conflict(index) ~= "" or req_needs_amend(index) then
    if status == R_OPEN then
      return "blocked"
    end
  end
  -- 旁路关闭的展示名 = 关闭原因（split / merged / dropped，R11）；正常交付 = done。
  local phase = req_phase(index)
  if phase == PHASE.closed then
    return req_closed_reason(index)
  end
  return phase
end

-- 终态判定 = status=="done"（真状态，不再数 split/merged/dropped——它们都是 done）。
local function req_closed(index)
  return req_status(index) == R_DONE
end

-- 计数口径 = 微状态（10 相位）：panel_tally 直接按 req_phase 计。
local function req_tally(phase_name)
  local total = 0
  for i = 1, req_count() do
    if req_phase(i) == phase_name then
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
  if req_count() == 0 then
    return false
  end
  for i = 1, req_count() do
    if not req_satisfied(i, nil) then
      return false
    end
  end
  return true
end

-- 可派活的阶段（微状态口径）；nil = 不可派：立项 → plan；待开发 → development。
-- 门禁待批 / 有 pending 提案 / integrating / 已闭环的需求一律不返（结构上不可派）。
local function req_job_stage(index)
  local phase = req_phase(index)
  if phase == PHASE.draft then
    return "plan"
  end
  if phase == PHASE.ready then
    return "development"
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
  ygg.kv.set(req_key(index, "status"), R_OPEN)
  ygg.kv.set(req_key(index, "closed_reason"), "")
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
  ygg.kv.set(req_key(index, "integrating"), false)
  ygg.kv.set(req_key(index, "develop_sha"), "")
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

-- D-MA23：`req_advance_after_accept` 已删除——真状态不再因受理改变（受理只升门禁，
-- 落点改写全部在 handlers 层直接写 gate / gate_rev / job；见 proposals.lua / gate_actions.lua）。

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
M.RUN_PHASE = RUN_PHASE
M.RUN_PHASE_LABEL = RUN_PHASE_LABEL
M.CLOSED_DROPPED = CLOSED_DROPPED
M.CLOSED_MERGED = CLOSED_MERGED
M.CLOSED_SPLIT = CLOSED_SPLIT
M.R_DONE = R_DONE
M.R_OPEN = R_OPEN
M.STAGE = STAGE
M.job_capacity = job_capacity
M.job_total = job_total
M.req_all_done = req_all_done
M.req_append = req_append
M.req_bump_rev = req_bump_rev
M.req_capacity = req_capacity
M.req_closed = req_closed
M.req_closed_reason = req_closed_reason
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
M.req_phase = req_phase
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
M.run_phase = run_phase
M.run_phase_label = run_phase_label
M.stage = stage

return M
