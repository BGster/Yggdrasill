-- src/view/render/screen.lua —— S32 纯搬移自 render.lua（零逻辑变更；接线见交付回执）。

local M = {}
local clock = require("src.core.clock")
local hooks = require("src.core.hooks")
local readset = require("src.domain.readset")
local wire = require("src.domain.wire")
local config = require("src.core.config")
local gates = require("src.domain.gates")
local lanes = require("src.domain.lanes")
local menus = require("src.view.menus")
local props = require("src.domain.proposals")
local reqs = require("src.domain.requirements")
local util = require("src.core.util")
local state = require("src.view.render.state")
local TEXTS = setmetatable({}, { __index = function(_, k) return state.TEXTS[k] end })
local mod_mapview = require("src.view.render.mapview")
local mod_reqlist = require("src.view.render.reqlist")
local num = config.num
local play_mode = config.play_mode
local run_retire_armed = config.run_retire_armed
local text_of = config.text_of
local executor_raw = config.executor_raw
local goal_authorization = config.goal_authorization
local gate_problem = gates.gate_problem
local LANES = lanes.LANES
local req_lane_pool = reqs.req_lane_pool
local CONFIG_UI = menus.CONFIG_UI
local MENU = menus.MENU
local prop_pending_total = props.prop_pending_total
local RUN_PHASE = reqs.RUN_PHASE
local STAGE = reqs.STAGE
local req_conflict = reqs.req_conflict
local req_conflict_peer = reqs.req_conflict_peer
local req_count = reqs.req_count
local req_gate = reqs.req_gate
local req_id = reqs.req_id
local req_job = reqs.req_job
local req_key = reqs.req_key
local req_lane = reqs.req_lane
local req_phase = reqs.req_phase
local req_rev = reqs.req_rev
local req_status = reqs.req_status
local req_title = reqs.req_title
local run_phase = reqs.run_phase
local stage = reqs.stage
local fill = util.fill
local RENDER = mod_mapview.RENDER
local evolution_receipt, narrate_play_mode = mod_mapview.evolution_receipt, mod_mapview.narrate_play_mode
local is_resumable_agent_id, req_live_status, summary_profiles = mod_reqlist.is_resumable_agent_id, mod_reqlist.req_live_status, mod_reqlist.summary_profiles


local flushing = false
local summary_jobs

local function render()
  -- 响应式收敛（唯一入口）：所有 useEffect（唯一后继自动推进）必须在产出 choice **之前**
  -- 同步跑完——已产生 choice 的 handler 里再 emit 会被内核静默丢弃（transition.rs 排空规则）。
  -- 重入守卫：effect 自身不 render，这层只防未来新增 effect 误调。
  -- r4：这里也是**派发序号与纪元边沿的唯一推进点**——每次 Mod 派发恰好一次，且在
  -- flush_effects 之前（同一派发内 effect 看到的 now() 已是本派发值，派生一致）。
  if not flushing then
    flushing = true
    clock.bump()
    readset.observe()
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
  local current = run_phase()
  -- 二级页不跨相位存活：相位一变就清空（子页是当前处境的入口，不是全局抽屉）。
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
    renderer = RENDER[RUN_PHASE.coordinate]
  end
  renderer()
end

local SUMMARY_JOB_STAGES = { plan = true, development = true }

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

-- 在跑作业的结构化明细（plan / development 两个活跃阶段各占一条；
-- analysis 槽位已随阶段合并注销，req_job_stage 永不返回 analysis，此处不再列出）。
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
      local lane_pool = lane > 0 and req_lane_pool(i) or ""
      local branch = lane > 0 and text_of(LANES.key(lane_pool, lane, "branch"), "") or ""
      local base = lane > 0 and text_of(LANES.key(lane_pool, lane, "base"), "") or ""
      local worktree_path = lane > 0 and text_of(LANES.key(lane_pool, lane, "path"), "") or ""
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
        phase = req_phase(i),
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
  -- 只看相位菜单（MENU 本体）：二级页打开时 pendingChoice 是子页（首项 config.set），
  -- 不能因此把自己报成「不结局」，收口判定不能被展示层带偏。
  local builder = MENU[run_phase()]
  if builder == nil then
    return false
  end
  local items = builder()
  if items == nil or #items == 0 then
    return false
  end
  -- r9 常驻语义：首项是 review.note 只说明「发布已声明、批收口就绪」，不等于本局将终——
  -- 只有 run.retire 武装时首项才是**终局变体**（宿主据此预 closing 协议）。
  -- 未武装时恒 false（哑泵在批收口后继续循环，绝不预收口）。
  return items[1].action == "review.note" and run_retire_armed()
end

local function summary_fields()
  local fields = {
    stage = stage(),
    ending = ygg.kv.get("ending") == STAGE.retired,
    verifyPassed = ygg.kv.get("verify.passed") == true,
    nextEndsRun = summary_next_ends_run(),
    -- r9 常驻生命周期三字段（加性；旧宿主容忍式解析，缺字段取保守默认）：
    -- lifecycle = mod 自报模式（宿主所有常驻特化行为以它为闸，缺失 = 旧 mod 行为不变）；
    -- batch = run.batch 镜像（批轮转可观测计数，缺省 1）；
    -- batchDrained = 泵暂停信号 = 协调相位 ∧ gate_problem()==nil ∧ 未武装
    --   （「本批队列已清空且没有已武装的终局流程，等新 --requirement 进料」）。
    lifecycle = "resident",
    batch = summary_int(num("run.batch", 1)),
    batchDrained = run_phase() == RUN_PHASE.coordinate and gate_problem() == nil and not run_retire_armed(),
    -- r1 加性一键：当前 play.mode（goal/guide）投影到摘要。旧宿主容忍式解析
    -- （缺字段取保守缺省），在场唤醒（YGG_AUTO_WAKE=on）只认 "goal"。
    play = play_mode(),
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
    -- profile 注册表（r3，自报身份卡）：编排层按 harness 选 multi-agent 机制、按
    -- assignments 找跟进需求；assignments 是协调快照，权责仍以 executor 账本为准。
    profiles = summary_profiles(),
  }
  local awaiting = summary_awaiting()
  if awaiting ~= nil then
    fields.awaiting = awaiting
  end
  return fields
end

M.SUMMARY_JOB_STAGES = SUMMARY_JOB_STAGES
M.receipt = render
M.summary_awaiting = summary_awaiting
M.summary_conflicts = summary_conflicts
M.summary_fields = summary_fields
M.summary_gates = summary_gates
M.summary_int = summary_int
M.summary_jobs = summary_jobs
M.summary_next_ends_run = summary_next_ends_run
M.render = render

return M
