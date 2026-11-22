-- src/view/render/reqlist.lua —— S32 纯搬移自 render.lua（零逻辑变更；接线见交付回执）。

local M = {}
local hooks = require("src.core.hooks")
local props = require("src.domain.proposals")
local reqs = require("src.domain.requirements")
local wire = require("src.domain.wire")
local config = require("src.core.config")
local gates = require("src.domain.gates")
local lanes = require("src.domain.lanes")
local util = require("src.core.util")
local state = require("src.view.render.state")
local TEXTS = setmetatable({}, { __index = function(_, k) return state.TEXTS[k] end })
local text_of = config.text_of
local executor_raw = config.executor_raw
local gate_total = gates.gate_total
local LANES = lanes.LANES
local lane_text = lanes.lane_text
local req_lane_pool = reqs.req_lane_pool
local P_PENDING = props.P_PENDING
local prop_count = props.prop_count
local prop_id = props.prop_id
local prop_key = props.prop_key
local prop_pending_total = props.prop_pending_total
local prop_req = props.prop_req
local prop_rev = props.prop_rev
local prop_stage = props.prop_stage
local prop_status = props.prop_status
local PHASE = reqs.PHASE
local CLOSED_MERGED = reqs.CLOSED_MERGED
local CLOSED_SPLIT = reqs.CLOSED_SPLIT
local job_capacity = reqs.job_capacity
local job_total = reqs.job_total
local req_closed = reqs.req_closed
local req_closed_reason = reqs.req_closed_reason
local req_conflict = reqs.req_conflict
local req_conflict_peer = reqs.req_conflict_peer
local req_conflict_total = reqs.req_conflict_total
local req_count = reqs.req_count
local req_deps = reqs.req_deps
local req_display = reqs.req_display
local req_id = reqs.req_id
local req_job = reqs.req_job
local req_key = reqs.req_key
local req_lane = reqs.req_lane
local req_needs_amend = reqs.req_needs_amend
local req_phase = reqs.req_phase
local req_rev = reqs.req_rev
local req_stale = reqs.req_stale
local req_tally = reqs.req_tally
local req_title = reqs.req_title
local run_phase = reqs.run_phase
local fill = util.fill
local join = util.join
local split = util.split
local trim = util.trim


-- 面板汇总走响应式记忆化（useMemo）：依赖签名 = 逐需求的微状态 + 待验收提案数 +
-- 需求总数（均由 KV 状态派生，读档/重启后签名一变即重算，不靠内存推断）。
local function panel_tally()
  return hooks.useMemo("render.panel_tally", function()
    return {
      draft = req_tally(PHASE.draft),
      planning = req_tally(PHASE.planning),
      gate = gate_total(),
      ready = req_tally(PHASE.ready),
      developing = req_tally(PHASE.developing),
      review = req_tally(PHASE.dev_review) + req_tally(PHASE.accept_gate),
      integrating = req_tally(PHASE.integrating),
      done = req_tally(PHASE.done) + req_tally(PHASE.closed),
      total = req_count(),
    }
  end, function()
    local deps_sig = { req_count(), gate_total(), prop_pending_total() }
    for i = 1, req_count() do
      deps_sig[#deps_sig + 1] = req_phase(i)
    end
    return deps_sig
  end)
end

local function req_panel_text()
  local tally = panel_tally()
  local parts = {}
  parts[#parts + 1] = fill(TEXTS.beats.req_panel_counts, {
    draft = tostring(tally.draft),
    planning = tostring(tally.planning),
    gate = tostring(tally.gate),
    ready = tostring(tally.ready),
    developing = tostring(tally.developing),
    review = tostring(tally.review),
    integrating = tostring(tally.integrating),
    done = tostring(tally.done),
    total = tostring(tally.total),
  })
  parts[#parts + 1] = fill(TEXTS.beats.req_panel_jobs, {
    jobs = tostring(job_total()),
    cap = tostring(job_capacity()),
    pending = tostring(prop_pending_total()),
    conflicts = tostring(req_conflict_total()),
  })
  return join(parts, " | ")
end

-- 字形表以 PHASE（10 微状态）为键 + 旁路关闭原因（R11：closed 不是幕名，由
-- closed_reason 区分）+ blocked 覆盖层：存储只有 open/done 两个真状态，展示全由 req_phase 派生。
local R_MARK = {
  [PHASE.draft] = "·",
  [PHASE.planning] = "◇",
  [PHASE.plan_gate] = "⊟",
  [PHASE.ready] = "○",
  [PHASE.developing] = "▶",
  [PHASE.dev_review] = "◀",
  [PHASE.accept_gate] = "⊞",
  [PHASE.integrating] = "◉",
  [PHASE.done] = "✓",
  [PHASE.closed] = "✗",
  [reqs.CLOSED_SPLIT] = "⑂",
  [reqs.CLOSED_MERGED] = "⇄",
  [reqs.CLOSED_DROPPED] = "⊘",
  blocked = "⚠",
}

local function is_resumable_agent_id(id)
  if type(id) ~= "string" then
    return false
  end
  local clean = trim(id)
  if clean == "" or clean == "unregistered" or clean == "pi-ygg-harness" or clean == "main" or clean == "default" then
    return false
  end
  return true
end

local function pending_prop_index_of(index)
  for i = 1, prop_count() do
    if prop_status(i) == P_PENDING and prop_req(i) == index then
      return i
    end
  end
  return nil
end

local function req_live_status(index)
  local id = req_id(index)
  local lane = req_lane(index)
  local pool = lane > 0 and req_lane_pool(index) or ""
  local job = req_job(index)
  local phase = req_phase(index)
  local branch = lane > 0 and text_of(LANES.key(pool, lane, "branch"), "") or ""
  local worktree_path = lane > 0 and text_of(LANES.key(pool, lane, "path"), "") or ""
  local lane_req = lane > 0 and text_of(LANES.key(pool, lane, "req"), "") or ""

  local worktree_status = "n/a"
  -- R7：泳道绑定窗口覆盖 开发中→交付受理→验收门禁→集成 四幕（执行者要工作树做集成）。
  if job == "development" or phase == PHASE.developing or phase == PHASE.dev_review
    or phase == PHASE.accept_gate or phase == PHASE.integrating then
    if lane > 0 and lane_req == id then
      -- 未断言分支显示 `-`（lane_field_none，与账本房规一致）：展示层不再虚构 `feature/<id>`。
      local br = branch ~= "" and branch or TEXTS.beats.lane_field_none
      if worktree_path ~= "" then
        worktree_status = "ready:" .. worktree_path .. "@" .. br
      else
        worktree_status = "bound:" .. LANES.label(pool, lane) .. "@" .. br
      end
    end
  elseif phase == PHASE.ready then
    worktree_status = "pending"
  end

  local prop_idx = pending_prop_index_of(index)
  local active_stage = job
  if active_stage == "" and prop_idx ~= nil then
    active_stage = prop_stage(prop_idx)
  end
  local exec_id = active_stage ~= "" and wire.executor_id_of(executor_raw(id, active_stage)) or ""

  local subagent_status = "idle"
  if prop_idx ~= nil then
    subagent_status = "delivered:p" .. tostring(prop_idx) .. (exec_id ~= "" and ("(" .. exec_id .. ")") or "")
  elseif job ~= "" then
    if exec_id ~= "" and exec_id ~= "unregistered" then
      subagent_status = "dispatched:" .. exec_id
    else
      subagent_status = "pending"
    end
  end

  local probe_head = lane > 0 and text_of(LANES.key(pool, lane, "probe.head"), "") or ""
  local probe_status = "n/a"
  if lane > 0 and probe_head ~= "" then
    local probe_commits = tostring(ygg.kv.get(LANES.key(pool, lane, "probe.commits")) or "0")
    local probe_clean = text_of(LANES.key(pool, lane, "probe.clean"), "0")
    probe_status = "verified:commits=" .. probe_commits .. ",clean=" .. probe_clean
  elseif job == "development" then
    probe_status = "pending"
  end

  return worktree_status, subagent_status, probe_status
end

-- 宿主通道三态后缀（r3，显示 join 不改权责）：(req, stage) 最新提案落库的宿主
-- actor 章对照账本 id——相等 = [verified]；不等 = [mismatch:actor=…]（识别不拒绝）；
-- 无章（旧提案 / config.set 直接登记 / 旧宿主回滚面）= [unverified]。
local function prop_verdict(index, stage_name, exec_id)
  local actor = props.prop_actor_for(index, stage_name)
  if actor == nil then
    return TEXTS.beats.req_trace_actor_unverified
  end
  if actor == exec_id then
    return TEXTS.beats.req_trace_actor_verified
  end
  return fill(TEXTS.beats.req_trace_actor_mismatch, { actor = actor })
end

-- ── profile 注册表投影（r3，自报身份卡：声明面不是凭据）─────────────────

-- 沙箱无 table 库：插入排序（handle 量小，线性插入足够；投影口径 = handle 升序）。
local function sorted_strings(list)
  local out = {}
  for _, value in ipairs(list) do
    local pos = #out + 1
    for i = 1, #out do
      if out[i] > value then
        pos = i
        break
      end
    end
    for i = #out, pos, -1 do
      out[i + 1] = out[i]
    end
    out[pos] = value
  end
  return out
end

-- 注册句柄清单（settings.profile_index 维护序 → handle 升序；索引键由
-- handlers/config.lua 的 profile_set 单源维护，用户面写不进去）。
local function profile_handles()
  local out = {}
  for _, part in ipairs(split(text_of("settings.profile_index", ""), ",")) do
    local piece = trim(part)
    if piece ~= "" then
      out[#out + 1] = piece
    end
  end
  return sorted_strings(out)
end

local function profile_field(handle, name)
  return text_of("settings.profile." .. handle .. "." .. name, "")
end

-- 设计视图 PROFILES 行：≤4 张全列（harness[,role] 短注）+ 截断计数；无注册整行
-- 省略（不占默认视图——操作视图不加，保持最小）。
local function narrate_profiles()
  local handles = profile_handles()
  if #handles == 0 then
    return
  end
  local items = {}
  local shown = #handles < 4 and #handles or 4
  for i = 1, shown do
    local handle = handles[i]
    local harness = profile_field(handle, "harness")
    local role = profile_field(handle, "role")
    local piece = handle .. "(" .. harness
    if role ~= "" then
      piece = piece .. "," .. role
    end
    items[#items + 1] = piece .. ")"
  end
  if #handles > shown then
    items[#items + 1] = fill(TEXTS.beats.map_note_profiles_more, { count = tostring(#handles - shown) })
  end
  ygg.narrate(fill(TEXTS.beats.map_note_profiles, { list = join(items, " ") }))
end

-- summary 结构化面（编排层的机器可读面）：≤8 张，handle 升序；assignments 解析成
-- token 数组（编排层直接消费）。半张卡按缺键降级——自报注册可分多次 config.set 完成。
local function summary_profiles()
  local out = {}
  for i, handle in ipairs(profile_handles()) do
    if i > 8 then
      break
    end
    local assignments = {}
    for _, token in ipairs(split(profile_field(handle, "assignments"), ",")) do
      local piece = trim(token)
      if piece ~= "" then
        assignments[#assignments + 1] = piece
      end
    end
    out[#out + 1] = {
      handle = handle,
      harness = profile_field(handle, "harness"),
      session_id = profile_field(handle, "session_id"),
      role = profile_field(handle, "role"),
      assignments = assignments,
    }
  end
  return out
end

local function req_trace(index)
  local parts = {}
  local deps = req_deps(index)
  if #deps > 0 then
    parts[#parts + 1] = fill(TEXTS.beats.req_trace_deps, { ids = join(deps, ",") })
  end
  local origin = text_of(req_key(index, "origin"), "")
  if origin ~= "" then
    parts[#parts + 1] = fill(TEXTS.beats.req_trace_origin, { id = origin })
  end
  -- R11：split / merged 是 closed_reason，不再是状态值。
  local closed_reason = req_closed_reason(index)
  if closed_reason == CLOSED_SPLIT then
    parts[#parts + 1] = fill(TEXTS.beats.req_trace_split, { ids = text_of(req_key(index, "children"), "") })
  elseif closed_reason == CLOSED_MERGED then
    parts[#parts + 1] = fill(TEXTS.beats.req_trace_merged, { id = text_of(req_key(index, "into"), "") })
  end
  local lane = req_lane(index)
  if lane > 0 then
    parts[#parts + 1] = fill(TEXTS.beats.req_trace_lane, { lane = LANES.label(req_lane_pool(index), lane) })
  end
  local job = req_job(index)
  if job ~= "" then
    parts[#parts + 1] = fill(TEXTS.beats.req_trace_job, { job = job })
  end
  -- E2 执行者账本（自声明）：已登记就列出来；当前 job 对应阶段未登记就明说。
  -- r3：每项并列两组短注记——profile harness 短注记（声明面）+ 宿主通道三态
  --（最新提案的宿主 actor 章对照账本 id）；session id 不进 trace（grep 链走事件行）。
  local exec_ids = {}
  local exec_list = {}
  for i = 1, #wire.EXECUTOR_STAGES do
    local name = wire.EXECUTOR_STAGES[i]
    local agent = wire.executor_id_of(executor_raw(req_id(index), name))
    if agent ~= "" then
      exec_ids[name] = agent
      local shown = agent
      local harness = text_of("settings.profile." .. agent .. ".harness", "")
      if harness ~= "" then
        shown = shown .. fill(TEXTS.beats.req_trace_profile_tag, { harness = harness })
      end
      exec_list[#exec_list + 1] = name .. "=" .. shown .. prop_verdict(index, name, agent)
    end
  end
  if #exec_list > 0 then
    parts[#parts + 1] = fill(TEXTS.beats.req_trace_exec, { list = join(exec_list, " ") })
  end
  if wire.executor_stage_ok(job) and exec_ids[job] == nil then
    parts[#parts + 1] = fill(TEXTS.beats.req_trace_exec_missing, { stage = job })
  end
  if not reqs.req_closed(index) then
    local wt_status, sub_status = req_live_status(index)
    parts[#parts + 1] = "(wt=" .. wt_status .. ")(subagent=" .. sub_status .. ")"
  end
  if req_rev(index) > 1 then
    parts[#parts + 1] = fill(TEXTS.beats.req_trace_rev, { rev = tostring(req_rev(index)) })
  end
  if req_conflict(index) ~= "" then
    parts[#parts + 1] = fill(TEXTS.beats.req_trace_conflict, {
      note = req_conflict(index),
      peer = req_conflict_peer(index),
    })
  elseif req_needs_amend(index) then
    parts[#parts + 1] = TEXTS.beats.req_trace_needs_amend
  end
  if req_stale(index) then
    parts[#parts + 1] = TEXTS.beats.req_trace_stale
  end
  return join(parts, "")
end

-- r13 分页调参：默认页行数（与 req_capacity 同处 TEXTS.tuning；12 > 单批
-- 操作域、显著小于渲染撞墙行数 —— §0.2 实测 kv_len≈292 时全量 list 在
-- 100k 地板下必死）。
local function req_list_page()
  local value = TEXTS.tuning.req_list_page
  if type(value) == "number" and value > 0 then
    return value
  end
  return 12
end

-- 默认页行序（r13）：open 需求优先（canonical 序），不足页宽用最近闭环行
-- （新→旧）补足。这是**展示序**，不改任何 KV/状态语义（行数据同源同值，
-- 漏出行只付一次 req_closed 分类读）。长局 canonical 前缀恰是历史闭环需求
-- （req.count 单调不减），字面前缀页会出现「整页 done、活跃需求全被略」
-- 的反转 —— open 优先是操作语义下的正确页。
local function req_page_order()
  local open_rows, closed_rows = {}, {}
  for i = 1, req_count() do
    if req_closed(i) then
      closed_rows[#closed_rows + 1] = i
    else
      open_rows[#open_rows + 1] = i
    end
  end
  local ordered = {}
  for k = 1, #open_rows do
    ordered[#ordered + 1] = open_rows[k]
  end
  for k = #closed_rows, 1, -1 do
    ordered[#ordered + 1] = closed_rows[k]
  end
  return ordered
end

-- `full`（`requirement.list full`，wire 已预留的 opt-in，今天由 no-op 转真）
-- = 全量渲染（低频运维动作）；默认 = 分页视图：需求行与待验收提案行都截到
-- 页宽 + 提示行（「其余 K 条略（共 M 条）」），渲染行数 O(M) → O(页宽常数)。
local function narrate_req_list(full)
  ygg.narrate(fill(TEXTS.beats.req_list_head, {
    panel = req_panel_text(),
    stage = run_phase(),
    lanes = lane_text(),
  }))
  local total = req_count()
  if total == 0 then
    ygg.narrate(TEXTS.beats.req_list_empty)
  else
    local ordered = req_page_order()
    local page = full and total or req_list_page()
    local shown = page < #ordered and page or #ordered
    for k = 1, shown do
      local i = ordered[k]
      ygg.narrate(fill(TEXTS.beats.req_list_row, {
        mark = R_MARK[req_display(i)] or "?",
        id = req_id(i),
        status = req_display(i),
        title = req_title(i),
        trace = req_trace(i),
      }))
    end
    local omitted = #ordered - shown
    if omitted > 0 then
      ygg.narrate(fill(TEXTS.beats.req_list_more, {
        omitted = tostring(omitted),
        total = tostring(total),
      }))
    end
  end
  -- 待验收提案（canonical 序；主 Agent 一次只验一条）—— 同页约束：pending
  -- 行是第二成本源（每行带完整 summary 长文本），同样截到页宽 + 提示行。
  local pending = prop_pending_total()
  if pending > 0 then
    ygg.narrate(fill(TEXTS.beats.req_list_props, { count = tostring(pending) }))
    local pending_rows = {}
    for i = 1, prop_count() do
      if prop_status(i) == P_PENDING then
        pending_rows[#pending_rows + 1] = i
      end
    end
    local page = full and #pending_rows or req_list_page()
    local shown = page < #pending_rows and page or #pending_rows
    for k = 1, shown do
      local i = pending_rows[k]
      ygg.narrate(fill(TEXTS.beats.req_prop_row, {
        id = prop_id(i),
        req = req_id(prop_req(i)),
        rev = tostring(prop_rev(i)),
        stage = prop_stage(i),
        summary = text_of(prop_key(i, "summary"), ""),
      }))
    end
    local omitted = #pending_rows - shown
    if omitted > 0 then
      ygg.narrate(fill(TEXTS.beats.req_list_props_more, {
        omitted = tostring(omitted),
        total = tostring(pending),
      }))
    end
  end
  ygg.narrate(TEXTS.beats.req_list_legend)
end

M.R_MARK = R_MARK
M.is_resumable_agent_id = is_resumable_agent_id
M.narrate_profiles = narrate_profiles
M.narrate_req_list = narrate_req_list
M.req_live_status = req_live_status
M.req_panel_text = req_panel_text
M.req_trace = req_trace
M.summary_profiles = summary_profiles

return M
