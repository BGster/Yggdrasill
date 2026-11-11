-- src/view/render.lua —— 回执渲染 + 双层图（local/full）+ 结构化摘要（`agent.summary` 的注册面）。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 资源由入口 install(deps) 注入。本模块写的 KV 前缀：`settlement.*`（仅 reset_settlement；其余只读）。

local M = {}

local config = require("src.core.config")
local gates = require("src.domain.gates")
local guide = require("src.view.guide")
local hooks = require("src.core.hooks")
local lanes = require("src.domain.lanes")
local menus = require("src.view.menus")
local props = require("src.domain.proposals")
local reqs = require("src.domain.requirements")
local util = require("src.core.util")
local wire = require("src.domain.wire")

local TEXTS
local CONFIG

local max_per_run, mode, num, play_mode, strict_gates, task_desc, task_title, text_of, executor_raw, goal_authorization =
  config.max_per_run, config.mode, config.num, config.play_mode, config.strict_gates, config.task_desc, config.task_title, config.text_of, config.executor_raw, config.goal_authorization
local gate_total =
  gates.gate_total
local narrate_stage_guide =
  guide.narrate_stage_guide
local lane_text, narrate_lane_ledger =
  lanes.lane_text, lanes.narrate_lane_ledger
local CONFIG_UI, MENU, menu, option_texts =
  menus.CONFIG_UI, menus.MENU, menus.menu, menus.option_texts
local P_PENDING, prop_count, prop_id, prop_key, prop_pending_total, prop_req, prop_rev, prop_stage, prop_status =
  props.P_PENDING, props.prop_count, props.prop_id, props.prop_key, props.prop_pending_total, props.prop_req, props.prop_rev, props.prop_stage, props.prop_status
local PHASE, R_MERGED, R_SPLIT, STAGE, job_capacity, job_total, req_closed, req_conflict, req_conflict_peer, req_conflict_total, req_count, req_deps, req_display, req_gate, req_id, req_job, req_key, req_lane, req_needs_amend, req_open_total, req_phase_name, req_rev, req_stale, req_status, req_tally, req_title, stage =
  reqs.PHASE, reqs.R_MERGED, reqs.R_SPLIT, reqs.STAGE, reqs.job_capacity, reqs.job_total, reqs.req_closed, reqs.req_conflict, reqs.req_conflict_peer, reqs.req_conflict_total, reqs.req_count, reqs.req_deps, reqs.req_display, reqs.req_gate, reqs.req_id, reqs.req_job, reqs.req_key, reqs.req_lane, reqs.req_needs_amend, reqs.req_open_total, reqs.req_phase_name, reqs.req_rev, reqs.req_stale, reqs.req_status, reqs.req_tally, reqs.req_title, reqs.stage
local fill, join, spaces, trim =
  util.fill, util.join, util.spaces, util.trim

-- 面板汇总走响应式记忆化（useMemo）：依赖签名 = 逐需求的派生展示名 + 待验收提案数 +
-- 需求总数（均由 KV 状态派生，读档/重启后签名一变即重算，不靠内存推断）。
local function panel_tally()
  return hooks.useMemo("render.panel_tally", function()
    return {
      draft = req_tally(PHASE.draft),
      analyzing = req_tally(PHASE.analyzing) + req_tally(PHASE.analysis_review),
      planning = req_tally(PHASE.planning) + req_tally(PHASE.plan_review),
      gate = gate_total(),
      ready = req_tally(PHASE.ready),
      developing = req_tally(PHASE.developing) + req_tally(PHASE.dev_review),
      done = req_tally(PHASE.done),
      total = req_count(),
    }
  end, function()
    local deps_sig = { req_count(), gate_total(), prop_pending_total() }
    for i = 1, req_count() do
      deps_sig[#deps_sig + 1] = req_phase_name(i)
    end
    return deps_sig
  end)
end

local function req_panel_text()
  local tally = panel_tally()
  local parts = {}
  parts[#parts + 1] = fill(TEXTS.beats.req_panel_counts, {
    draft = tostring(tally.draft),
    analyzing = tostring(tally.analyzing),
    planning = tostring(tally.planning),
    gate = tostring(tally.gate),
    ready = tostring(tally.ready),
    developing = tostring(tally.developing),
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

-- 字形表以 PHASE（派生展示名）为键：存储只有 5 态，展示仍有 14 个旧名。
local R_MARK = {
  [PHASE.draft] = "·",
  [PHASE.analyzing] = "◐",
  [PHASE.analysis_review] = "◑",
  [PHASE.planning] = "◇",
  [PHASE.plan_review] = "◈",
  [PHASE.plan_gate] = "⊟",
  [PHASE.ready] = "○",
  [PHASE.developing] = "▶",
  [PHASE.dev_review] = "◀",
  [PHASE.accept] = "⊞",
  [PHASE.done] = "✓",
  [PHASE.split] = "⑂",
  [PHASE.merged] = "⇄",
  [PHASE.dropped] = "⊘",
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
  local job = req_job(index)
  local phase = req_phase_name(index)
  local branch = lane > 0 and text_of("lane." .. tostring(lane) .. ".branch", "") or ""
  local worktree_path = lane > 0 and text_of("lane." .. tostring(lane) .. ".path", "") or ""
  local lane_req = lane > 0 and text_of("lane." .. tostring(lane) .. ".req", "") or ""

  local worktree_status = "n/a"
  if job == "development" or phase == PHASE.developing or phase == PHASE.dev_review or phase == PHASE.accept then
    if lane > 0 and lane_req == id then
      -- 未断言分支显示 `-`（lane_field_none，与账本房规一致）：展示层不再虚构 `feature/<id>`。
      local br = branch ~= "" and branch or TEXTS.beats.lane_field_none
      if worktree_path ~= "" then
        worktree_status = "ready:" .. worktree_path .. "@" .. br
      else
        worktree_status = "bound:lane-" .. tostring(lane) .. "@" .. br
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

  local probe_head = lane > 0 and text_of("lane." .. tostring(lane) .. ".probe.head", "") or ""
  local probe_status = "n/a"
  if lane > 0 and probe_head ~= "" then
    local probe_commits = tostring(ygg.kv.get("lane." .. tostring(lane) .. ".probe.commits") or "0")
    local probe_clean = text_of("lane." .. tostring(lane) .. ".probe.clean", "0")
    probe_status = "verified:commits=" .. probe_commits .. ",clean=" .. probe_clean
  elseif job == "development" then
    probe_status = "pending"
  end

  return worktree_status, subagent_status, probe_status
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
  local status = req_status(index)
  if status == R_SPLIT then
    parts[#parts + 1] = fill(TEXTS.beats.req_trace_split, { ids = text_of(req_key(index, "children"), "") })
  elseif status == R_MERGED then
    parts[#parts + 1] = fill(TEXTS.beats.req_trace_merged, { id = text_of(req_key(index, "into"), "") })
  end
  local lane = req_lane(index)
  if lane > 0 then
    parts[#parts + 1] = fill(TEXTS.beats.req_trace_lane, { lane = "lane-" .. tostring(lane) })
  end
  local job = req_job(index)
  if job ~= "" then
    parts[#parts + 1] = fill(TEXTS.beats.req_trace_job, { job = job })
  end
  -- E2 执行者账本（自声明）：已登记就列出来；当前 job 对应阶段未登记就明说。
  local exec_ids = {}
  local exec_list = {}
  for i = 1, #wire.EXECUTOR_STAGES do
    local name = wire.EXECUTOR_STAGES[i]
    local agent = wire.executor_id_of(executor_raw(req_id(index), name))
    if agent ~= "" then
      exec_ids[name] = agent
      exec_list[#exec_list + 1] = name .. "=" .. agent
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
    stage = stage(),
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

local function req_graph_lines()
  narrate_req_list()
end

local MAP_NODES = { STAGE.intake, STAGE.coordinate, STAGE.review, STAGE.done }

local MAP_EDGES = {
  { from = STAGE.intake, to = STAGE.coordinate, action = "task.intake" },
  { from = STAGE.coordinate, to = STAGE.review, action = "coordinate.submit", note = "map_note_all_done" },
  {
    from = STAGE.review,
    to = STAGE.coordinate,
    action = "verify.submit=false",
    note = "map_note_back",
  },
  { from = STAGE.review, to = STAGE.done, action = "review.note", note = "map_note_ending" },
}

-- 需求级流水线（设计视图）= 5 个存储态；每行中列是「态内的三跳」（提案 → 验收 → 门禁）。
-- 上一轮这里漏了中间那跳 proposal.accept，读起来像「提案直接进门禁」，方向是反的。
-- analysis + plan 已合并：draft 派活直接进 planning（prop 仍走「提案 → 验收」两跳）。
local PIPELINE = {
  { node = "[draft]", action = "requirement.dispatch", target = "[planning]", note = "map_note_job" },
  {
    node = "[planning]",
    action = "proposal → proposal.accept → gate.approve",
    target = "[planning·plan_ok]",
    note = "map_note_gate_plan",
  },
  {
    node = "[planning·plan_ok]",
    action = "requirement.dispatch",
    target = "[developing·lane]",
    note = "map_note_lane",
  },
  {
    node = "[developing]",
    action = "proposal → proposal.accept → gate.approve",
    target = "[done]",
    note = "map_note_gate_accept",
  },
}

-- 旁路（不进主流水线）：左列是**可变前件**（不是状态，是可做该操作的条件），右列是结果终态。
-- 旧版的左右是反的——把 split / merged / dropped 这些终态摆在左列，读起来像「终态还能再拆
-- 一次」。方向修正：前件 = draft / analyzing / planning（无在途工作，见 req_graph_mutable）；
-- keep_first 的前件是「draft / ready 的冲突对」（见 requirement.resolve 的守卫）。
local PIPELINE_BYPASS = {
  {
    node = "[draft/analyzing/planning]",
    action = "requirement.split",
    target = "[split] + [children·draft]",
    note = "map_note_split",
  },
  {
    node = "[draft/analyzing/planning]",
    action = "requirement.merge",
    target = "[merged] + [new·draft]",
    note = "map_note_merge",
  },
  {
    node = "[draft/ready conflict pair]",
    action = "requirement.resolve keep_first",
    target = "[dropped]",
    note = "map_note_dropped",
  },
}

local MAP_LABEL_WIDTH = 0

for i = 1, #MAP_NODES do
  local width = #MAP_NODES[i] + 2
  if width > MAP_LABEL_WIDTH then
    MAP_LABEL_WIDTH = width
  end
end

local PIPE_LABEL_WIDTH = 0

for i = 1, #PIPELINE do
  local width = #PIPELINE[i].node
  if width > PIPE_LABEL_WIDTH then
    PIPE_LABEL_WIDTH = width
  end
end

for i = 1, #PIPELINE_BYPASS do
  local width = #PIPELINE_BYPASS[i].node
  if width > PIPE_LABEL_WIDTH then
    PIPE_LABEL_WIDTH = width
  end
end

local function pad(label, width)
  local out = label
  for _ = #label + 1, width do
    out = out .. " "
  end
  return out
end

local function out_edges(node)
  local out = {}
  for i = 1, #MAP_EDGES do
    if MAP_EDGES[i].from == node then
      out[#out + 1] = MAP_EDGES[i]
    end
  end
  return out
end

-- 出边的目标阶段：`MAP_EDGES` 是唯一来源（不另建第二份边表）。
-- 不在表里的动作（map.show / requirement.* / config.* / lane.*）= 不换阶段（自环）。
-- 特例：review → coordinate 的打回边在表里记作 `verify.submit=false`，而菜单里的
-- 失败项载荷是 `all_green=false;report=…`，所以要按载荷前缀补一次查找。
local function to_stage(action, payload)
  for i = 1, #MAP_EDGES do
    if MAP_EDGES[i].action == action then
      return MAP_EDGES[i].to
    end
  end
  if action == "verify.submit" and type(payload) == "string" then
    if trim(payload):sub(1, 15) == "all_green=false" then
      for i = 1, #MAP_EDGES do
        if MAP_EDGES[i].action == "verify.submit=false" then
          return MAP_EDGES[i].to
        end
      end
    end
  end
  return nil
end

local function narrate_pipeline(rows)
  for i = 1, #rows do
    local row = rows[i]
    ygg.narrate(fill(TEXTS.beats.map_row, {
      mark = spaces(2),
      node = pad(row.node, PIPE_LABEL_WIDTH),
      branch = TEXTS.beats.map_branch_single,
      action = row.action,
      target = row.target,
      note = TEXTS.beats[row.note],
    }))
  end
end

local function narrate_map(current, full, items, prompt)
  if not full then
    -- 操作视图：剧情是当前节点，选择是它的有向出边（与 pendingChoice 同源）。
    ygg.narrate(fill(TEXTS.beats.map_local_head, { stage = current, panel = req_panel_text() }))
    if type(prompt) == "string" and prompt ~= "" then
      ygg.narrate(prompt)
    end
    if items == nil or #items == 0 then
      ygg.narrate(TEXTS.beats.map_terminal)
    else
      for i = 1, #items do
        local item = items[i]
        -- 当前 MENU 没有 enabled=false 的项，所以 state 恒为「可选择」；但当 `item.when`
        -- 缺失时打印的是「条件未展示」而不是「无额外条件」——渲染层看不见 choice 侧的真实
        -- 前置（那在候选构造里），不得替它担保「没有条件」。
        -- 引入条件项时必须同时补 map_edge_blocked 与 choice 侧的拦截（不能只改渲染）。
        ygg.narrate(fill(TEXTS.beats.map_local_edge, {
          index = tostring(i),
          text = item.text,
          target = to_stage(item.action, item.payload) or current,
          state = TEXTS.beats.map_edge_ready,
          when = item.when or TEXTS.beats.map_edge_no_condition,
        }))
      end
    end
    ygg.narrate(TEXTS.beats.map_local_foot)
    return
  end

  -- 设计视图：仅 `map.show full` 输出；不是日常操作视图。
  ygg.narrate(fill(TEXTS.beats.map_head, { stage = current, panel = req_panel_text() }))
  local blank = spaces(MAP_LABEL_WIDTH)
  for i = 1, #MAP_NODES do
    local node = MAP_NODES[i]
    local mark = spaces(2)
    if node == current then
      mark = TEXTS.beats.map_mark
    end
    local label = pad("[" .. node .. "]", MAP_LABEL_WIDTH)
    local edges = out_edges(node)
    if #edges == 0 then
      ygg.narrate(fill(TEXTS.beats.map_row_leaf, {
        mark = mark,
        node = label,
        branch = TEXTS.beats.map_branch_single,
        text = TEXTS.beats.map_terminal,
      }))
    end
    for j = 1, #edges do
      local edge = edges[j]
      local branch = TEXTS.beats.map_branch_single
      if #edges > 1 then
        branch = j == #edges and TEXTS.beats.map_branch_last or TEXTS.beats.map_branch_first
      end
      local lead = j == 1 and mark or spaces(2)
      local head = j == 1 and label or blank
      ygg.narrate(fill(TEXTS.beats.map_row, {
        mark = lead,
        node = head,
        branch = branch,
        action = edge.action,
        target = "[" .. edge.to .. "]",
        note = edge.note ~= nil and TEXTS.beats[edge.note] or "",
      }))
    end
  end
  ygg.narrate(TEXTS.beats.map_pipeline_head)
  narrate_pipeline(PIPELINE)
  ygg.narrate(TEXTS.beats.map_pipeline_bypass_head)
  narrate_pipeline(PIPELINE_BYPASS)
  ygg.narrate(fill(TEXTS.beats.map_jobs, {
    lanes = lane_text(),
    jobs = tostring(job_total()),
    cap = tostring(job_capacity()),
    pending = tostring(prop_pending_total()),
  }))
  narrate_lane_ledger()
  ygg.narrate(TEXTS.beats.map_foot)
end

local RENDER = {}

local prompt_vars

-- 剧情是点：一个节点只有一段文本（`<节点>_common`）。
-- goal / guide 的差异只在「谁派发」，由 narrate_play_mode 每条回执报一次，不在每个节点重复。
local function stage_prompt(st)
  return TEXTS.beats[st .. "_common"] or TEXTS.beats.stage_prompt_missing
end

local function narrate_play_mode()
  local hint = TEXTS.beats.play_hint_guide
  if play_mode() == "goal" then
    hint = TEXTS.beats.play_hint_goal
  end
  ygg.narrate(fill(TEXTS.beats.play_receipt, { play = play_mode(), hint = hint }))
end

-- 演进偏好如实体现在文案：它只是建议口径的标记，本 Mod 不自动修订、不登记修订请求。
local function evolution_receipt()
  local current = mode()
  local paths = CONFIG.evolution and CONFIG.evolution.allow_paths
  local paths_text = type(paths) == "table" and join(paths, ", ") or TEXTS.beats.paths_none
  if current == "auto" then
    return fill(TEXTS.beats.evolution_auto, { paths = paths_text, cap = tostring(max_per_run()) })
  elseif current == "suggest" then
    return fill(TEXTS.beats.evolution_suggest, { paths = paths_text })
  end
  return TEXTS.beats.evolution_off
end

local function narrate_req_panel()
  ygg.narrate(req_panel_text())
  ygg.narrate(fill(TEXTS.beats.req_lane_panel, {
    lanes = lane_text(),
    jobs = tostring(job_total()),
    cap = tostring(job_capacity()),
  }))
end

RENDER[STAGE.intake] = function()
  local current = stage()
  ygg.narrate(fill(TEXTS.scenes.intake, { stage = current, gates = tostring(strict_gates()) }))
  ygg.narrate(fill(stage_prompt(STAGE.intake), prompt_vars()))
  if req_count() > 0 then
    narrate_req_panel()
  end
  narrate_stage_guide(current)
  ygg.choice(TEXTS.prompts.intake, option_texts(menu(current)))
end

RENDER[STAGE.coordinate] = function()
  local current = stage()
  ygg.narrate(fill(TEXTS.scenes.coordinate, {
    title = task_title(),
    description = task_desc(),
    count = tostring(req_count()),
    open = tostring(req_open_total()),
    pending = tostring(prop_pending_total()),
  }))
  ygg.narrate(fill(stage_prompt(STAGE.coordinate), prompt_vars()))
  narrate_req_panel()
  narrate_stage_guide(current)
  ygg.choice(TEXTS.prompts.coordinate, option_texts(menu(current)))
end

RENDER[STAGE.review] = function()
  local current = stage()
  ygg.narrate(fill(TEXTS.scenes.review, {
    title = task_title(),
    count = tostring(req_count()),
    done = tostring(req_tally(PHASE.done)),
    gates = tostring(strict_gates()),
  }))
  -- 数据红线只在开局（event.start）与规则章里出现；常规步不重复。
  narrate_req_panel()
  narrate_stage_guide(current)
  ygg.choice(TEXTS.prompts.review, option_texts(menu(current)))
end

RENDER[STAGE.done] = function()
  ygg.narrate(fill(TEXTS.endings.done, {
    title = task_title(),
    count = tostring(req_count()),
    done = tostring(req_tally(PHASE.done)),
    score = tostring(num("review.score", TEXTS.tuning.default_score)),
    lesson = text_of("review.lesson", TEXTS.beats.default_lesson),
    candidates = text_of("review.evolution.candidates", TEXTS.beats.evolution_none),
  }))
  if play_mode() == "goal" then
    ygg.narrate(TEXTS.beats.play_done_goal)
  end
end

local flushing = false
local summary_jobs

local function render()
  -- 响应式收敛（唯一入口）：所有 useEffect（唯一后继自动推进）必须在产出 choice **之前**
  -- 同步跑完——已产生 choice 的 handler 里再 emit 会被内核静默丢弃（transition.rs 排空规则）。
  -- 重入守卫：effect 自身不 render，这层只防未来新增 effect 误调。
  if not flushing then
    flushing = true
    hooks.flush_effects()
    flushing = false
  end
  local jobs_now = summary_jobs()
  if #jobs_now > 0 then
    local actions_now = {}
    for i = 1, #jobs_now do
      if jobs_now[i].subagentAction ~= nil then
        actions_now[#actions_now + 1] = jobs_now[i].subagentAction
      end
    end
    if #actions_now > 0 then
      ygg.meta("harness_actions", actions_now)
    end
  end
  local current = stage()
  -- 二级页不跨阶段存活：阶段一变就清空（子页是当前处境的入口，不是全局抽屉）。
  if CONFIG_UI.page() ~= "" and CONFIG_UI.page_stage() ~= current then
    CONFIG_UI.set_page("")
  end
  narrate_play_mode()
  -- 演进建议只在配置二级页输出：它是「要你决定的事」而不是每步都要读的处境。
  if CONFIG_UI.page() == "config" then
    ygg.narrate(evolution_receipt())
    local auth = goal_authorization()
    if auth == "" then
      ygg.narrate(TEXTS.beats.config_goal_auth_none)
    else
      ygg.narrate(fill(TEXTS.beats.config_goal_auth, { auth = auth }))
    end
  end
  local renderer = RENDER[current]
  if renderer == nil then
    ygg.narrate(fill(TEXTS.beats.unknown_stage, { stage = tostring(current) }))
    renderer = RENDER[STAGE.intake]
  end
  renderer()
end

local SUMMARY_JOB_STAGES = { analysis = true, plan = true, development = true }

local function summary_int(value)
  local number = tonumber(value) or 0
  if number < 0 then
    number = 0
  end
  return math.floor(number)
end

local function summary_conflicts()
  local out = {}
  for i = 1, req_count() do
    local note = req_conflict(i)
    if note ~= "" then
      out[#out + 1] = { id = req_id(i), peer = req_conflict_peer(i), note = note }
    end
  end
  return out
end

local function summary_awaiting()
  for i = 1, req_count() do
    local job = req_job(i)
    if SUMMARY_JOB_STAGES[job] == true then
      return { req = req_id(i), rev = summary_int(req_rev(i)), stage = job }
    end
  end
  return nil
end

-- 在跑作业的结构化明细（analysis / plan / development 三阶段各占一条）。
-- 只读 KV 派生的展示面：与 summary_awaiting 同源（后者只返回第一条），
-- 这里返回全量，供守护模式的调度端一次性取到 id / rev / lane / branch / base / 标题 / 描述，
-- 以及**账本里绑定的执行者**：`executor` = 当前阶段的 agent id，`planExecutor` = plan 阶段的
-- agent id（未绑定 = ""）。两者都剥掉 `|<原因>` 后缀——调度端只认 id，不解析原因；
-- 本字段只是自声明账本的镜像，不构成身份凭据。
summary_jobs = function()
  local out = {}
  for i = 1, req_count() do
    local job = req_job(i)
    if SUMMARY_JOB_STAGES[job] == true then
      local lane = req_lane(i)
      local id = req_id(i)
      local rev = summary_int(req_rev(i))
      local branch = lane > 0 and text_of("lane." .. tostring(lane) .. ".branch", "") or ""
      local base = lane > 0 and text_of("lane." .. tostring(lane) .. ".base", "") or ""
      local worktree_path = lane > 0 and text_of("lane." .. tostring(lane) .. ".path", "") or ""
      local exec_id = wire.executor_id_of(executor_raw(id, job))
      local plan_exec_id = wire.executor_id_of(executor_raw(id, "plan"))
      local resume_id = ""
      if is_resumable_agent_id(exec_id) then
        resume_id = exec_id
      elseif job == "development" and is_resumable_agent_id(plan_exec_id) then
        resume_id = plan_exec_id
      end
      local worktree_status, subagent_status, probe_status = req_live_status(i)
      local title = req_title(i)
      local desc = text_of(req_key(i, "description"), "")
      local artifact_path = "artifacts/" .. id .. "-" .. job .. ".md"
      local wt_display = worktree_path ~= "" and worktree_path or "."
      local branch_display = branch ~= "" and branch or "-"
      local base_display = base ~= "" and base or "-"
      local submit_command = fill(TEXTS.beats.subagent_submit_cmd, {
        id = id,
        rev = tostring(rev),
        stage = job,
        artifact = artifact_path,
      })
      local brief = fill(TEXTS.beats.subagent_brief, {
        id = id,
        rev = tostring(rev),
        stage = job,
        title = title,
        desc = desc,
        worktree = wt_display,
        branch = branch_display,
        base = base_display,
        artifact = artifact_path,
        submit = submit_command,
      })
      local delta_prompt = ""
      if resume_id ~= "" then
        delta_prompt = fill(TEXTS.beats.subagent_delta, {
          id = id,
          rev = tostring(rev),
          stage = job,
          worktree = wt_display,
          artifact = artifact_path,
        })
      end
      local full_fallback_brief = brief
      if resume_id ~= "" then
        full_fallback_brief = brief .. fill(TEXTS.beats.subagent_fallback_note, {
          resume_id = resume_id,
        })
      end
      local subagent_action = {
        action = (resume_id ~= "") and "subagent.resume" or "subagent.spawn",
        reqId = id,
        rev = rev,
        stage = job,
        lane = summary_int(lane),
        resumeAgentId = resume_id,
        worktreePath = worktree_path,
        artifactPath = artifact_path,
        brief = brief,
        deltaPrompt = delta_prompt,
        fullFallbackBrief = full_fallback_brief,
        submitCommand = submit_command,
      }
      out[#out + 1] = {
        id = id,
        rev = rev,
        stage = job,
        status = req_status(i),
        phase = req_phase_name(i),
        lane = summary_int(lane),
        branch = branch,
        base = base,
        worktreePath = worktree_path,
        worktreeStatus = worktree_status,
        subagentStatus = subagent_status,
        probeStatus = probe_status,
        executor = exec_id,
        planExecutor = plan_exec_id,
        title = title,
        description = desc,
        subagentAction = subagent_action,
      }
    end
  end
  return out
end

-- 待运行门禁的结构化明细（plan / accept 两类）——同上，展示面只读 KV。
local function summary_gates()
  local out = {}
  for i = 1, req_count() do
    local gate = req_gate(i)
    if gate ~= "" then
      out[#out + 1] = {
        id = req_id(i),
        gate = gate,
        rev = summary_int(req_rev(i)),
        title = req_title(i),
      }
    end
  end
  return out
end

local function summary_next_ends_run()
  -- 只看阶段菜单（MENU 本体）：二级页打开时 pendingChoice 是子页（首项 config.set），
  -- 不能因此把自己报成「不结局」——收口判定不能被展示层带偏。
  local builder = MENU[stage()]
  if builder == nil then
    return false
  end
  local items = builder()
  if items == nil or #items == 0 then
    return false
  end
  return items[1].action == "review.note"
end

local function summary_fields()
  local fields = {
    stage = stage(),
    ending = ygg.kv.get("ending") == STAGE.done,
    verifyPassed = ygg.kv.get("verify.passed") == true,
    nextEndsRun = summary_next_ends_run(),
    pendingProposals = summary_int(prop_pending_total()),
    requirementCount = summary_int(req_count()),
    conflicts = summary_conflicts(),
    -- 结构化作业 / 门禁明细：空表在宿主侧序列化为 `[]`（不是 null），调度端无需区分缺失与空。
    jobs = summary_jobs(),
    gates = summary_gates(),
    -- 工具桥的在飞计数（只读 KV：`src/view/render.lua` 不得 require("src.runtime.tools")，那会立刻成环）。
    pendingTools = summary_int(ygg.kv.get("tool.pending")),
    -- 进化声明（只声称「声明了什么」）：宿主与只读查询可直接核对，不涉及内容质量。
    evolution = {
      artifact = text_of("review.evolution.artifact", ""),
      summary = text_of("review.evolution.summary", ""),
      candidates = text_of("review.evolution.candidates", TEXTS.beats.evolution_none),
      classes = text_of("review.evolution.classes", ""),
    },
  }
  local awaiting = summary_awaiting()
  if awaiting ~= nil then
    fields.awaiting = awaiting
  end
  return fields
end

prompt_vars = function()
  return {
    engine = tostring(ygg.engine_version),
    stage = stage(),
    play = play_mode(),
    mode = mode(),
    gates = tostring(strict_gates()),
    count = tostring(req_count()),
    open = tostring(req_open_total()),
    pending = tostring(prop_pending_total()),
    jobs = tostring(job_total()),
    cap = tostring(job_capacity()),
    score = tostring(TEXTS.tuning.default_score),
  }
end

-- 三项结算登记的统一重置：coordinate.submit 重入与 verify.submit=false 都调这一处。
-- 只清本局登记，不动 settings.review.evolution.* 预登记值。
function M.reset_settlement()
  ygg.kv.set("settlement.evolution.artifact", "")
  ygg.kv.set("settlement.evolution.summary", "")
  ygg.kv.set("settlement.evolution.candidates", "")
  ygg.kv.set("settlement.evolution.classes", "")
  ygg.kv.set("settlement.sessions.remaining", -1)
  ygg.kv.set("settlement.sessions.moved", 0)
  ygg.kv.set("settlement.sessions.destination", "")
  ygg.kv.set("settlement.todo.remaining", -1)
  ygg.kv.set("settlement.todo.verified_by", "")
end

function M.install(deps)
  TEXTS = deps.texts
  CONFIG = deps.config
end

function M.register()
  ygg.on("agent.summary", function()
    ygg.meta("summary", summary_fields())
    render()
  end)
end

M.MAP_EDGES = MAP_EDGES
M.MAP_LABEL_WIDTH = MAP_LABEL_WIDTH
M.MAP_NODES = MAP_NODES
M.PIPELINE = PIPELINE
M.PIPELINE_BYPASS = PIPELINE_BYPASS
M.PIPE_LABEL_WIDTH = PIPE_LABEL_WIDTH
M.RENDER = RENDER
M.R_MARK = R_MARK
M.SUMMARY_JOB_STAGES = SUMMARY_JOB_STAGES
M.evolution_receipt = evolution_receipt
M.is_resumable_agent_id = is_resumable_agent_id
M.narrate_map = narrate_map
M.narrate_play_mode = narrate_play_mode
M.narrate_req_list = narrate_req_list
M.narrate_req_panel = narrate_req_panel
M.out_edges = out_edges
M.pad = pad
M.prompt_vars = prompt_vars
M.receipt = render
M.req_graph_lines = req_graph_lines
M.req_live_status = req_live_status
M.req_panel_text = req_panel_text
M.req_trace = req_trace
M.stage_prompt = stage_prompt
M.summary_awaiting = summary_awaiting
M.summary_conflicts = summary_conflicts
M.summary_fields = summary_fields
M.summary_gates = summary_gates
M.summary_int = summary_int
M.summary_jobs = summary_jobs
M.summary_next_ends_run = summary_next_ends_run

return M
