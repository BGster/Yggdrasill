-- src/render.lua —— 回执渲染 + 双层图（local/full）+ 结构化摘要（`agent.summary` 的注册面）。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 资源由入口 install(deps) 注入。本模块唯一写的 KV 前缀：无（只读；渲染不改状态）。

local M = {}

local config = require("src.config")
local gates = require("src.gates")
local guide = require("src.guide")
local lanes = require("src.lanes")
local menus = require("src.menus")
local props = require("src.proposals")
local reqs = require("src.requirements")
local util = require("src.util")

local TEXTS
local CONFIG

local max_per_run, mode, num, play_mode, strict_gates, task_desc, task_title, text_of =
  config.max_per_run, config.mode, config.num, config.play_mode, config.strict_gates, config.task_desc, config.task_title, config.text_of
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
local R_ACCEPT, R_ANALYSIS_REVIEW, R_ANALYZING, R_DEVELOPING, R_DEV_REVIEW, R_DONE, R_DRAFT, R_DROPPED, R_MERGED, R_PLANNING, R_PLAN_GATE, R_PLAN_REVIEW, R_READY, R_SPLIT, STAGE, job_capacity, job_total, req_conflict, req_conflict_peer, req_conflict_total, req_count, req_deps, req_display, req_id, req_job, req_key, req_lane, req_needs_amend, req_open_total, req_rev, req_stale, req_status, req_tally, req_title, stage =
  reqs.R_ACCEPT, reqs.R_ANALYSIS_REVIEW, reqs.R_ANALYZING, reqs.R_DEVELOPING, reqs.R_DEV_REVIEW, reqs.R_DONE, reqs.R_DRAFT, reqs.R_DROPPED, reqs.R_MERGED, reqs.R_PLANNING, reqs.R_PLAN_GATE, reqs.R_PLAN_REVIEW, reqs.R_READY, reqs.R_SPLIT, reqs.STAGE, reqs.job_capacity, reqs.job_total, reqs.req_conflict, reqs.req_conflict_peer, reqs.req_conflict_total, reqs.req_count, reqs.req_deps, reqs.req_display, reqs.req_id, reqs.req_job, reqs.req_key, reqs.req_lane, reqs.req_needs_amend, reqs.req_open_total, reqs.req_rev, reqs.req_stale, reqs.req_status, reqs.req_tally, reqs.req_title, reqs.stage
local fill, join, spaces =
  util.fill, util.join, util.spaces

local function req_panel_text()
  local parts = {}
  parts[#parts + 1] = fill(TEXTS.beats.req_panel_counts, {
    draft = tostring(req_tally(R_DRAFT)),
    analyzing = tostring(req_tally(R_ANALYZING) + req_tally(R_ANALYSIS_REVIEW)),
    planning = tostring(req_tally(R_PLANNING) + req_tally(R_PLAN_REVIEW)),
    gate = tostring(gate_total()),
    ready = tostring(req_tally(R_READY)),
    developing = tostring(req_tally(R_DEVELOPING) + req_tally(R_DEV_REVIEW)),
    done = tostring(req_tally(R_DONE)),
    total = tostring(req_count()),
  })
  parts[#parts + 1] = fill(TEXTS.beats.req_panel_jobs, {
    jobs = tostring(job_total()),
    cap = tostring(job_capacity()),
    pending = tostring(prop_pending_total()),
    conflicts = tostring(req_conflict_total()),
  })
  return join(parts, "｜")
end

local R_MARK = {
  [R_DRAFT] = "·",
  [R_ANALYZING] = "◐",
  [R_ANALYSIS_REVIEW] = "◑",
  [R_PLANNING] = "◇",
  [R_PLAN_REVIEW] = "◈",
  [R_PLAN_GATE] = "⊟",
  [R_READY] = "○",
  [R_DEVELOPING] = "▶",
  [R_DEV_REVIEW] = "◀",
  [R_ACCEPT] = "⊞",
  [R_DONE] = "✓",
  [R_SPLIT] = "⑂",
  [R_MERGED] = "⇄",
  [R_DROPPED] = "⊘",
  blocked = "⚠",
}

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

local function narrate_req_list()
  ygg.narrate(fill(TEXTS.beats.req_list_head, {
    panel = req_panel_text(),
    stage = stage(),
    lanes = lane_text(),
  }))
  if req_count() == 0 then
    ygg.narrate(TEXTS.beats.req_list_empty)
  else
    for i = 1, req_count() do
      local shown = req_display(i)
      ygg.narrate(fill(TEXTS.beats.req_list_row, {
        mark = R_MARK[shown] or "?",
        id = req_id(i),
        status = shown,
        title = req_title(i),
        trace = req_trace(i),
      }))
    end
  end
  -- 待验收提案（canonical 序；主 Agent 一次只验一条）
  local pending = prop_pending_total()
  if pending > 0 then
    ygg.narrate(fill(TEXTS.beats.req_list_props, { count = tostring(pending) }))
    for i = 1, prop_count() do
      if prop_status(i) == P_PENDING then
        ygg.narrate(fill(TEXTS.beats.req_prop_row, {
          id = prop_id(i),
          req = req_id(prop_req(i)),
          rev = tostring(prop_rev(i)),
          stage = prop_stage(i),
          summary = text_of(prop_key(i, "summary"), ""),
        }))
      end
    end
  end
  ygg.narrate(TEXTS.beats.req_list_legend)
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

local PIPELINE = {
  { node = "[draft]", action = "requirement.dispatch", target = "[analyzing]", note = "map_note_job" },
  { node = "[analyzing]", action = "proposal", target = "[analysis_review]", note = "map_note_proposal" },
  { node = "[analysis_review]", action = "proposal.accept", target = "[planning]", note = "map_note_accept" },
  { node = "[planning]", action = "requirement.dispatch", target = "[plan(job)]", note = "map_note_job" },
  { node = "[plan(job)]", action = "proposal", target = "[plan_review]", note = "map_note_proposal" },
  {
    node = "[plan_review]",
    action = "proposal.accept",
    target = "[plan_gate]",
    note = "map_note_gate",
  },
  {
    node = "[plan_gate]",
    action = "gate.approve",
    target = "[ready]",
    note = "map_note_gate_plan",
  },
  { node = "[ready]", action = "requirement.dispatch", target = "[developing·lane]", note = "map_note_lane" },
  { node = "[developing]", action = "proposal", target = "[dev_review]", note = "map_note_proposal" },
  {
    node = "[dev_review]",
    action = "proposal.accept",
    target = "[accept·执行方交付]",
    note = "map_note_deliver",
  },
  {
    node = "[accept]",
    action = "gate.approve",
    target = "[done]",
    note = "map_note_gate_accept",
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

local function narrate_map(current)
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
  for i = 1, #PIPELINE do
    local row = PIPELINE[i]
    ygg.narrate(fill(TEXTS.beats.map_row, {
      mark = spaces(2),
      node = pad(row.node, PIPE_LABEL_WIDTH),
      branch = TEXTS.beats.map_branch_single,
      action = row.action,
      target = row.target,
      note = TEXTS.beats[row.note],
    }))
  end
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

local function stage_prompt(st)
  if play_mode() == "goal" then
    return TEXTS.beats[st .. "_goal"] or TEXTS.beats[st .. "_guide"]
  end
  return TEXTS.beats[st .. "_guide"]
end

local function narrate_play_mode()
  local hint = TEXTS.beats.play_hint_guide
  if play_mode() == "goal" then
    hint = TEXTS.beats.play_hint_goal
  end
  ygg.narrate(fill(TEXTS.beats.play_receipt, { play = play_mode(), hint = hint }))
end

local function evolution_receipt()
  local current = mode()
  local used = num("evolution.used", 0)
  local cap = max_per_run()
  local paths = CONFIG.evolution and CONFIG.evolution.allow_paths
  local paths_text = type(paths) == "table" and join(paths, "、") or TEXTS.beats.paths_none
  if current == "auto" then
    if used >= cap then
      return fill(TEXTS.beats.evolution_capped, { paths = paths_text, cap = tostring(cap) })
    end
    return TEXTS.beats.evolution_auto_pending
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
  ygg.narrate(evolution_receipt())
  narrate_stage_guide(current)
  ygg.choice(TEXTS.prompts.coordinate, option_texts(menu(current)))
end

RENDER[STAGE.review] = function()
  local current = stage()
  ygg.narrate(fill(TEXTS.scenes.review, {
    title = task_title(),
    count = tostring(req_count()),
    done = tostring(req_tally(R_DONE)),
    gates = tostring(strict_gates()),
  }))
  ygg.narrate(TEXTS.beats.secret_redline)
  ygg.narrate(evolution_receipt())
  narrate_req_panel()
  narrate_stage_guide(current)
  ygg.choice(TEXTS.prompts.review, option_texts(menu(current)))
end

RENDER[STAGE.done] = function()
  ygg.narrate(fill(TEXTS.endings.done, {
    title = task_title(),
    count = tostring(req_count()),
    done = tostring(req_tally(R_DONE)),
    score = tostring(num("review.score", TEXTS.tuning.default_score)),
    lesson = text_of("review.lesson", TEXTS.beats.default_lesson),
  }))
  if play_mode() == "goal" then
    ygg.narrate(TEXTS.beats.play_done_goal)
  end
end

local function render()
  local current = stage()
  -- 二级页不跨阶段存活：阶段一变就清空（子页是当前处境的入口，不是全局抽屉）。
  if CONFIG_UI.page() ~= "" and CONFIG_UI.page_stage() ~= current then
    CONFIG_UI.set_page("")
  end
  narrate_play_mode()
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
M.PIPE_LABEL_WIDTH = PIPE_LABEL_WIDTH
M.RENDER = RENDER
M.R_MARK = R_MARK
M.SUMMARY_JOB_STAGES = SUMMARY_JOB_STAGES
M.evolution_receipt = evolution_receipt
M.narrate_map = narrate_map
M.narrate_play_mode = narrate_play_mode
M.narrate_req_list = narrate_req_list
M.narrate_req_panel = narrate_req_panel
M.out_edges = out_edges
M.pad = pad
M.prompt_vars = prompt_vars
M.receipt = render
M.req_panel_text = req_panel_text
M.req_trace = req_trace
M.stage_prompt = stage_prompt
M.summary_awaiting = summary_awaiting
M.summary_conflicts = summary_conflicts
M.summary_fields = summary_fields
M.summary_int = summary_int
M.summary_next_ends_run = summary_next_ends_run

return M
