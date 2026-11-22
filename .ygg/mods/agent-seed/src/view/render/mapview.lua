-- src/view/render/mapview.lua —— S32 纯搬移自 render.lua（零逻辑变更；接线见交付回执）。

local M = {}
local config = require("src.core.config")
local guide = require("src.view.guide")
local lanes = require("src.domain.lanes")
local menus = require("src.view.menus")
local props = require("src.domain.proposals")
local reqs = require("src.domain.requirements")
local util = require("src.core.util")
local state = require("src.view.render.state")
local TEXTS = setmetatable({}, { __index = function(_, k) return state.TEXTS[k] end })
local CONFIG = setmetatable({}, { __index = function(_, k) return state.CONFIG[k] end })
local mod_reqlist = require("src.view.render.reqlist")
local max_per_run = config.max_per_run
local mode = config.mode
local num = config.num
local play_mode = config.play_mode
local run_retire_armed = config.run_retire_armed
local strict_gates = config.strict_gates
local task_goal = config.task_goal
local task_success = config.task_success
local task_title = config.task_title
local text_of = config.text_of
local narrate_stage_guide = guide.narrate_stage_guide
local lane_text = lanes.lane_text
local narrate_lane_ledger = lanes.narrate_lane_ledger
local menu = menus.menu
local option_texts = menus.option_texts
local prop_pending_total = props.prop_pending_total
local PHASE = reqs.PHASE
local RUN_PHASE = reqs.RUN_PHASE
local job_capacity = reqs.job_capacity
local job_total = reqs.job_total
local req_count = reqs.req_count
local req_open_total = reqs.req_open_total
local req_tally = reqs.req_tally
local run_phase = reqs.run_phase
local fill = util.fill
local join = util.join
local spaces = util.spaces
local trim = util.trim
local narrate_profiles, narrate_req_list, req_panel_text = mod_reqlist.narrate_profiles, mod_reqlist.narrate_req_list, mod_reqlist.req_panel_text


local function req_graph_lines()
  narrate_req_list()
end

local MAP_NODES = { RUN_PHASE.charter, RUN_PHASE.coordinate, RUN_PHASE.settle, RUN_PHASE.release, RUN_PHASE.retired }

-- 剧情双层重写 常驻批循环图（§2.3）：live 内四幕自循环（立局→协调⇄盘点→发布→批结算）；
-- live→retired 是唯一真转场（review.note 武装路径经 flow.advance 盖章）。
-- 注：本表是**展示面**（不是 FSM 转移表，mod.lua 的 ygg.fsm.define 一字未动）；
-- to_stage 对 review.note 按武装状态二选一，让操作视图的目标相位与真实落点一致。
local MAP_EDGES = {
  { from = RUN_PHASE.charter, to = RUN_PHASE.coordinate, action = "run.begin", note = "map_note_run_begin" },
  { from = RUN_PHASE.coordinate, to = RUN_PHASE.settle, action = "coordinate.submit", note = "map_note_batch_boundary" },
  {
    from = RUN_PHASE.settle,
    to = RUN_PHASE.coordinate,
    action = "verify.submit=false",
    note = "map_note_back",
  },
  -- 盘点四项齐备自动转发布（R9 效应驱动，不是事件边）。
  { from = RUN_PHASE.settle, to = RUN_PHASE.release, action = "(auto) settle complete", note = "map_note_settle_ready" },
  { from = RUN_PHASE.release, to = RUN_PHASE.coordinate, action = "review.note", note = "map_note_batch_settle" },
  { from = RUN_PHASE.release, to = RUN_PHASE.retired, action = "review.note", note = "map_note_retire" },
}

-- 需求级流水线（设计视图）= 九幕（§2.4）：立项 → 计划中 → 计划门禁 → 待开发 →
-- 开发中 → 交付受理 → 验收门禁 → 集成 → done；三条打回边（R6）统一落待开发。
local PIPELINE = {
  { node = "[draft]", action = "requirement.dispatch", target = "[planning]", note = "map_note_job" },
  { node = "[planning]", action = "proposal.submit(plan)", target = "[plan_gate]", note = "map_note_plan_submit" },
  {
    node = "[plan_gate]",
    action = "gate.approve / gate.reject",
    target = "[ready] / [planning]",
    note = "map_note_gate_plan",
  },
  {
    node = "[ready]",
    action = "requirement.dispatch",
    target = "[developing·lane]",
    note = "map_note_lane",
  },
  { node = "[developing]", action = "proposal.submit(development)", target = "[dev_review]", note = "map_note_deliver" },
  {
    node = "[dev_review]",
    action = "proposal.accept / proposal.reject",
    target = "[accept_gate] / [ready]",
    note = "map_note_accept",
  },
  {
    node = "[accept_gate]",
    action = "gate.approve / gate.reject",
    target = "[integrating] / [ready]",
    note = "map_note_gate_accept",
  },
  {
    node = "[integrating]",
    action = "integration.confirm / integration.reject",
    target = "[done] / [ready]",
    note = "map_note_integration",
  },
}

-- 旁路（不进主流水线，R11：旁路关闭 = closed_reason 字段）：左列是**可变前件**
-- （结构化判据 = open ∧ 无门禁 ∧ 无 pending ∧ 无 job ∧ 无泳道，覆盖立项/计划中/待开发），
-- 右列是结果（status=done + closed_reason）。
local PIPELINE_BYPASS = {
  {
    node = "[draft/planning/ready]",
    action = "requirement.split",
    target = "[closed·split] + [children·draft]",
    note = "map_note_split",
  },
  {
    node = "[draft/planning/ready]",
    action = "requirement.merge",
    target = "[closed·merged] + [new·draft]",
    note = "map_note_merge",
  },
  {
    node = "[draft/ready conflict pair]",
    action = "requirement.resolve keep_first",
    target = "[closed·dropped]",
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

-- 出边的目标相位：`MAP_EDGES` 是唯一来源（不另建第二份边表）。
-- 不在表里的动作（map.show / requirement.* / config.* / lane.* / release.*）= 不换相位（自环）。
-- 特例：settle → coordinate 的打回边在表里记作 `verify.submit=false`，而菜单里的
-- 失败项载荷是 `all_green=false;report=…`，所以要按载荷前缀补一次查找。
-- 特例二（r9/剧情双层重写）：review.note 有两条语义边（批收口 → 协调 / 退役 → 退役），
-- 按 run.retire 武装状态二选一，让操作视图的目标相位与真实落点一致；
-- run.begin 是立局幕的唯一出口（→ 协调）。
local function to_stage(action, payload)
  if action == "review.note" then
    if run_retire_armed() then
      return RUN_PHASE.retired
    end
    return RUN_PHASE.coordinate
  end
  if action == "run.begin" then
    return RUN_PHASE.coordinate
  end
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
  narrate_profiles()
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

RENDER[RUN_PHASE.charter] = function()
  local current = run_phase()
  ygg.narrate(fill(TEXTS.scenes.charter, {
    title = task_title(),
    goal = task_goal(),
    success = task_success(),
    batch = tostring(num("run.batch", 1)),
  }))
  ygg.narrate(fill(stage_prompt(RUN_PHASE.charter), prompt_vars()))
  narrate_stage_guide(current)
  ygg.choice(TEXTS.prompts.charter, option_texts(menu(current)))
end

RENDER[RUN_PHASE.coordinate] = function()
  local current = run_phase()
  ygg.narrate(fill(TEXTS.scenes.coordinate, {
    title = task_title(),
    goal = task_goal(),
    count = tostring(req_count()),
    open = tostring(req_open_total()),
    pending = tostring(prop_pending_total()),
  }))
  ygg.narrate(fill(stage_prompt(RUN_PHASE.coordinate), prompt_vars()))
  narrate_req_panel()
  narrate_stage_guide(current)
  ygg.choice(TEXTS.prompts.coordinate, option_texts(menu(current)))
end

RENDER[RUN_PHASE.settle] = function()
  local current = run_phase()
  ygg.narrate(fill(TEXTS.scenes.settle, {
    title = task_title(),
    count = tostring(req_count()),
    done = tostring(req_tally(PHASE.done) + req_tally(PHASE.closed)),
    gates = tostring(strict_gates()),
  }))
  -- 数据红线只在开局（event.start）与规则章里出现；常规步不重复。
  narrate_req_panel()
  narrate_stage_guide(current)
  ygg.choice(TEXTS.prompts.settle, option_texts(menu(current)))
end

RENDER[RUN_PHASE.release] = function()
  local current = run_phase()
  -- 发布已声明的展示串：confirm → master@<sha> + tag；skip → 跳过理由；未声明 → 空串。
  local declared = ""
  local master_sha = text_of("run.release.master_sha", "")
  if master_sha ~= "" then
    declared = "｜已声明合并 master@" .. master_sha .. " tag " .. text_of("run.release.tag", "")
  elseif text_of("run.release.skipped_reason", "") ~= "" then
    declared = "｜已声明跳过：" .. text_of("run.release.skipped_reason", "")
  end
  ygg.narrate(fill(TEXTS.scenes.release, {
    title = task_title(),
    batch = tostring(num("run.batch", 1)),
    declared = declared,
  }))
  ygg.narrate(fill(stage_prompt(RUN_PHASE.release), prompt_vars()))
  narrate_stage_guide(current)
  ygg.choice(TEXTS.prompts.release, option_texts(menu(current)))
end

RENDER[RUN_PHASE.retired] = function()
  local current = run_phase()
  -- r9/剧情双层重写：retired = 退役终态（仅 run.retire 武装后的 review.note 可达）。
  ygg.narrate(fill(TEXTS.endings.done, {
    title = task_title(),
    count = tostring(req_count()),
    done = tostring(req_tally(PHASE.done) + req_tally(PHASE.closed)),
    batch = tostring(num("run.batch", 1)),
    score = tostring(num("review.score", TEXTS.tuning.default_score)),
    lesson = text_of("review.lesson", TEXTS.beats.default_lesson),
    candidates = text_of("review.evolution.candidates", TEXTS.beats.evolution_none),
  }))
  if play_mode() == "goal" then
    ygg.narrate(TEXTS.beats.play_done_goal)
  end
  narrate_stage_guide(current)
  ygg.choice(TEXTS.prompts.retired, option_texts(menu(RUN_PHASE.retired)))
end

prompt_vars = function()
  return {
    engine = tostring(ygg.engine_version),
    stage = run_phase(),
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

M.MAP_EDGES = MAP_EDGES
M.MAP_LABEL_WIDTH = MAP_LABEL_WIDTH
M.MAP_NODES = MAP_NODES
M.PIPELINE = PIPELINE
M.PIPELINE_BYPASS = PIPELINE_BYPASS
M.PIPE_LABEL_WIDTH = PIPE_LABEL_WIDTH
M.RENDER = RENDER
M.evolution_receipt = evolution_receipt
M.narrate_map = narrate_map
M.narrate_play_mode = narrate_play_mode
M.narrate_req_panel = narrate_req_panel
M.out_edges = out_edges
M.pad = pad
M.prompt_vars = prompt_vars
M.req_graph_lines = req_graph_lines
M.stage_prompt = stage_prompt

return M
