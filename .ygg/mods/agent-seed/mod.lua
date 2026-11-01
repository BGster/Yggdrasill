-- .ygg/mods/agent-seed/mod.lua —— 装配入口（不是 Mod 本体）
--
-- 本文件只做装配：读资源快照 → require 模块 → 纯模块 install → render.register →
-- handlers install → node:render → fsm.define。业务实现都在 src/ 下。
--
-- 派发序 = 下面 install() 的调用序（不是 require 序、也不是 pack.json 的 code.modules 声明序）。

local TEXTS = ygg.data("ygg:scene/texts")
local CONFIG = ygg.data("ygg:agent/config")
local PLAYBOOK = ygg.data("ygg:agent/playbook")

local util = require("src.util")
local config = require("src.config")
local reqs = require("src.requirements")
local lanes = require("src.lanes")
local props = require("src.proposals")
local gates = require("src.gates")
local guide = require("src.guide")
local menus = require("src.menus")
local render_mod = require("src.render")
local wire = require("src.wire")
local flow = require("src.flow")

config.install({ texts = TEXTS, config = CONFIG })
reqs.install({ texts = TEXTS, lanes = lanes.LANES })
lanes.install({ texts = TEXTS })
props.install({ texts = TEXTS })
gates.install({ texts = TEXTS })
guide.install({ texts = TEXTS, playbook = PLAYBOOK })
menus.install({ texts = TEXTS })
render_mod.install({ texts = TEXTS, config = CONFIG })
wire.install({ texts = TEXTS })
flow.install({ texts = TEXTS })

render_mod.register()

local MODES, PLAY_MODES, effective, mode, num, play_mode, strict_gates, text_of =
  config.MODES, config.PLAY_MODES, config.effective, config.mode, config.num, config.play_mode, config.strict_gates, config.text_of
local advance, refuse, reject, reject_payload =
  flow.advance, flow.refuse, flow.reject, flow.reject_payload
local gate_evidence_problem, gate_kind, gate_next, gate_problem, gate_rev_ok, gate_total, req_artifacts =
  gates.gate_evidence_problem, gates.gate_kind, gates.gate_next, gates.gate_problem, gates.gate_rev_ok, gates.gate_total, gates.req_artifacts
local guide_section, narrate_guide =
  guide.guide_section, guide.narrate_guide
local LANES, free_lanes, lane_text =
  lanes.LANES, lanes.free_lanes, lanes.lane_text
local CONFIG_UI, dispatch_candidates, menu =
  menus.CONFIG_UI, menus.dispatch_candidates, menus.menu
local P_ACCEPTED, P_PENDING, P_REJECTED, P_STALE, apply_changes, prop_append, prop_by_ext, prop_count, prop_id, prop_key, prop_next, prop_pending_total, prop_req, prop_rev, prop_stage, prop_status, req_do_conflict, req_do_merge, req_do_split =
  props.P_ACCEPTED, props.P_PENDING, props.P_REJECTED, props.P_STALE, props.apply_changes, props.prop_append, props.prop_by_ext, props.prop_count, props.prop_id, props.prop_key, props.prop_next, props.prop_pending_total, props.prop_req, props.prop_rev, props.prop_stage, props.prop_status, props.req_do_conflict, props.req_do_merge, props.req_do_split
local narrate_map, narrate_req_list, prompt_vars, render, req_panel_text, stage_prompt =
  render_mod.narrate_map, render_mod.narrate_req_list, render_mod.prompt_vars, render_mod.receipt, render_mod.req_panel_text, render_mod.stage_prompt
local R_ACCEPT, R_ANALYSIS_REVIEW, R_ANALYZING, R_DEVELOPING, R_DEV_REVIEW, R_DONE, R_DRAFT, R_DROPPED, R_MERGED, R_PLANNING, R_PLAN_GATE, R_PLAN_REVIEW, R_READY, R_SPLIT, STAGE, job_capacity, job_total, req_advance_after_accept, req_append, req_bump_rev, req_capacity, req_closed, req_conflict, req_conflict_peer, req_count, req_deps, req_deps_text, req_dev_blocked_text, req_dev_ready, req_display, req_id, req_index_of, req_job_stage, req_key, req_lane, req_open_total, req_push_history, req_release, req_rev, req_status, req_sync_conflicts, req_title, req_validate_deps, stage =
  reqs.R_ACCEPT, reqs.R_ANALYSIS_REVIEW, reqs.R_ANALYZING, reqs.R_DEVELOPING, reqs.R_DEV_REVIEW, reqs.R_DONE, reqs.R_DRAFT, reqs.R_DROPPED, reqs.R_MERGED, reqs.R_PLANNING, reqs.R_PLAN_GATE, reqs.R_PLAN_REVIEW, reqs.R_READY, reqs.R_SPLIT, reqs.STAGE, reqs.job_capacity, reqs.job_total, reqs.req_advance_after_accept, reqs.req_append, reqs.req_bump_rev, reqs.req_capacity, reqs.req_closed, reqs.req_conflict, reqs.req_conflict_peer, reqs.req_count, reqs.req_deps, reqs.req_deps_text, reqs.req_dev_blocked_text, reqs.req_dev_ready, reqs.req_display, reqs.req_id, reqs.req_index_of, reqs.req_job_stage, reqs.req_key, reqs.req_lane, reqs.req_open_total, reqs.req_push_history, reqs.req_release, reqs.req_rev, reqs.req_status, reqs.req_sync_conflicts, reqs.req_title, reqs.req_validate_deps, reqs.stage
local contains, cut, fill, join, split, to_bool, trim =
  util.contains, util.cut, util.fill, util.join, util.split, util.to_bool, util.trim
local inbox_mark, inbox_replay, parse_amend, parse_conflict, parse_deps, parse_intake, parse_merge, parse_proposal, parse_requirement, parse_resolve, parse_split, probe_payload_ok, strip_ext =
  wire.inbox_mark, wire.inbox_replay, wire.parse_amend, wire.parse_conflict, wire.parse_deps, wire.parse_intake, wire.parse_merge, wire.parse_proposal, wire.parse_requirement, wire.parse_resolve, wire.parse_split, wire.probe_payload_ok, wire.strip_ext

local TEXTS = ygg.data("ygg:scene/texts")

local CONFIG = ygg.data("ygg:agent/config")

local PLAYBOOK = ygg.data("ygg:agent/playbook")

local HUMAN = {}

HUMAN.ACTIONS = {
  ["proposal.accept"] = true,
  ["gate.approve"] = true,
  ["coordinate.submit"] = true,
  ["verify.submit"] = true,
  ["review.note"] = true,
}

HUMAN.ticket = nil

function HUMAN.needs(label)
  if play_mode() ~= "guide" then
    return true
  end
  if HUMAN.ticket == label then
    HUMAN.ticket = nil
    return true
  end
  return refuse(TEXTS.beats.guide_human_blocked, label)
end

function HUMAN.gated(action)
  if HUMAN.ACTIONS[action] ~= true then
    return true
  end
  return HUMAN.needs(action)
end

function HUMAN.label(action, payload)
  if HUMAN.ACTIONS[action] == true then
    return action
  end
  if action == "config.set" and type(payload) == "string" then
    local head = trim(payload)
    if head:sub(1, 9) == "play.mode" then
      return "config.set play.mode"
    end
  end
  return nil
end

ygg.on("task.intake", function(payload)
  local current = stage()
  if current ~= STAGE.intake then
    return reject("task.intake", fill(TEXTS.beats.stage_expected, { expected = STAGE.intake }))
  end
  local title, description, problem = parse_intake(payload)
  if problem ~= nil then
    return reject_payload("task.intake", problem)
  end
  ygg.kv.set("task.title", title)
  ygg.kv.set("task.description", description)
  ygg.narrate(fill(TEXTS.beats.intake_ok, { title = title }))
  advance(STAGE.coordinate)
end)

ygg.on("requirement.add", function(payload)
  if stage() == STAGE.done then
    return reject("requirement.add", TEXTS.beats.req_after_done)
  end
  local title, description, ext, deps, problem = parse_requirement(payload)
  if problem ~= nil then
    return reject_payload("requirement.add", problem)
  end
  -- 幂等：同 external_id 重放不重复入队（CLI 崩溃重放安全面）。
  if type(ext) == "string" and ext ~= "" then
    for i = 1, req_count() do
      if ygg.kv.get(req_key(i, "ext")) == ext then
        ygg.narrate(fill(TEXTS.beats.req_replayed, {
          ext = ext,
          id = req_id(i),
          status = req_display(i),
        }))
        render()
        return
      end
    end
  end
  local dep_problem = req_validate_deps(req_count() + 1, deps)
  if dep_problem ~= nil then
    return reject_payload("requirement.add", dep_problem)
  end
  if req_open_total() >= req_capacity() then
    return reject_payload("requirement.add", fill(TEXTS.beats.req_full, { cap = tostring(req_capacity()) }))
  end
  local index = req_append(title, description, ext, deps, "")
  ygg.narrate(fill(TEXTS.beats.req_added, {
    id = req_id(index),
    title = title,
    deps = req_deps_text(index),
    counts = req_panel_text(),
  }))
  render()
end)

ygg.on("requirement.list", function(payload)
  if not probe_payload_ok(payload) then
    return reject_payload("requirement.list", fill(TEXTS.beats.probe_rejected, {
      action = "requirement.list",
      got = tostring(payload),
      usage = TEXTS.beats.req_list_usage,
    }))
  end
  narrate_req_list()
  render()
end)

ygg.on("requirement.dispatch", function(payload)
  if stage() == STAGE.done then
    return reject("requirement.dispatch", TEXTS.beats.req_after_done)
  end
  if not probe_payload_ok(payload) then
    return reject_payload("requirement.dispatch", fill(TEXTS.beats.probe_rejected, {
      action = "requirement.dispatch",
      got = tostring(payload),
      usage = TEXTS.beats.req_dispatch_usage,
    }))
  end
  -- 「按就绪序派活」是幂等语义：**没东西可派是正常结果**（需求都在 job
  -- 中 / 在门禁 / 在待验收 / 已完成，或 job 槽位 / 泳道已满）——自动化的
  -- 连续 dispatch 不得因此记失败步（伪失败会把正常等待当成故障）。
  -- 只留两个真拒绝：阶段已收口（done）与载荷非法。
  local candidates = dispatch_candidates()
  if #candidates == 0 then
    ygg.narrate(fill(TEXTS.beats.req_dispatch_noop_none, {
      jobs = tostring(job_total()),
      cap = tostring(job_capacity()),
    }))
    return render()
  end
  local lanes = free_lanes()
  local filled = {}
  for i = 1, #candidates do
    if job_total() < job_capacity() then
      local index = candidates[i]
      local stage_name = req_job_stage(index)
      local lane = 0
      local claimed = 0
      local ok = true
      if stage_name == "development" then
        -- 主 Agent 已用 lane.claim 预挂牌 → 直接沿用那条（不重复分配、不覆盖已记的
        -- branch/base/clean）；否则取第一条空闲泳道，同事务挂牌（branch/base/clean 留空 = 未断言）。
        claimed = req_dev_ready(index) and LANES.bound(index) or 0
        if not req_dev_ready(index) then
          ok = false
        elseif claimed > 0 then
          lane = claimed
        elseif #lanes > 0 then
          lane = lanes[1]
          local kept = {}
          for j = 2, #lanes do
            kept[#kept + 1] = lanes[j]
          end
          lanes = kept
        else
          ok = false
        end
      end
      if ok then
        ygg.kv.set(req_key(index, "job"), stage_name)
        if stage_name == "analysis" then
          ygg.kv.set(req_key(index, "status"), R_ANALYZING)
        elseif stage_name == "development" then
          ygg.kv.set(req_key(index, "status"), R_DEVELOPING)
          if claimed == 0 then
            LANES.bind(lane, index)
          end
        end
        filled[#filled + 1] = fill(TEXTS.beats.req_dispatch_item, {
          id = req_id(index),
          stage = stage_name,
          lane = lane > 0 and ("lane-" .. tostring(lane)) or "-",
        })
      end
    end
  end
  if #filled == 0 then
    ygg.narrate(fill(TEXTS.beats.req_dispatch_noop_blocked, {
      jobs = tostring(job_total()),
      cap = tostring(job_capacity()),
      lanes = lane_text(),
      reasons = req_dev_blocked_text(),
    }))
    return render()
  end
  ygg.narrate(fill(TEXTS.beats.req_dispatch_ok, {
    items = join(filled, "；"),
    count = tostring(#filled),
    jobs = tostring(job_total()),
    cap = tostring(job_capacity()),
  }))
  render()
end)

function LANES.parse_claim(payload)
  local max = job_capacity()
  if type(payload) ~= "string" then
    return nil, fill(TEXTS.beats.bad_payload, { action = "lane.claim" })
  end
  local parts = split(trim(payload), "|")
  local k = tonumber(trim(parts[1] or ""))
  local id = trim(parts[2] or "")
  if k == nil or id == "" then
    return nil, fill(TEXTS.beats.lane_claim_usage, { max = tostring(max) })
  end
  k = math.floor(k)
  if k < 1 or k > max then
    return nil, fill(TEXTS.beats.lane_bad_k, { k = tostring(k), max = tostring(max) })
  end
  return {
    k = k,
    id = id,
    branch = trim(parts[3] or ""),
    base = trim(parts[4] or ""),
    clean = trim(parts[5] or ""),
  }, nil
end

function LANES.parse_ref(payload)
  local max = job_capacity()
  if type(payload) ~= "string" then
    return nil, fill(TEXTS.beats.bad_payload, { action = "lane.release" })
  end
  local parts = split(trim(payload), "|")
  local k = tonumber(trim(parts[1] or ""))
  if k == nil then
    return nil, fill(TEXTS.beats.lane_release_usage, { max = tostring(max) })
  end
  k = math.floor(k)
  if k < 1 or k > max then
    return nil, fill(TEXTS.beats.lane_bad_k, { k = tostring(k), max = tostring(max) })
  end
  return k, nil
end

ygg.on("lane.claim", function(payload)
  if stage() == STAGE.done then
    return reject("lane.claim", TEXTS.beats.req_after_done)
  end
  local spec, problem = LANES.parse_claim(payload)
  if problem ~= nil then
    return reject_payload("lane.claim", problem)
  end
  -- 脏树禁挂牌：主 Agent 已确认工作区干净才允许挂牌（这里只验它的断言）。
  if spec.clean ~= "1" then
    return refuse(TEXTS.beats.req_wait_rejected, "lane.claim", fill(TEXTS.beats.lane_claim_dirty, {
      k = tostring(spec.k),
    }))
  end
  local holder = LANES.req_of(spec.k)
  if holder ~= "" and holder ~= spec.id then
    return refuse(TEXTS.beats.req_wait_rejected, "lane.claim", fill(TEXTS.beats.lane_claim_occupied, {
      k = tostring(spec.k),
      id = holder,
    }))
  end
  local index = req_index_of(spec.id)
  if index == nil then
    return reject_payload("lane.claim", fill(TEXTS.beats.req_unknown, { id = tostring(spec.id) }))
  end
  if req_closed(index) then
    return refuse(TEXTS.beats.req_wait_rejected, "lane.claim", fill(TEXTS.beats.lane_claim_closed, {
      id = req_id(index),
      status = req_status(index),
    }))
  end
  local bound = req_lane(index)
  if bound > 0 and bound ~= spec.k then
    return refuse(TEXTS.beats.req_wait_rejected, "lane.claim", fill(TEXTS.beats.lane_claim_bound, {
      id = req_id(index),
      k = tostring(bound),
    }))
  end
  LANES.bind(spec.k, index, spec.branch, spec.base, spec.clean)
  ygg.narrate(fill(TEXTS.beats.lane_claimed, {
    k = tostring(spec.k),
    id = req_id(index),
    title = req_title(index),
    branch = LANES.text_or(spec.branch, TEXTS.beats.lane_field_none),
    base = LANES.text_or(spec.base, TEXTS.beats.lane_field_none),
    since = tostring(num(LANES.key(spec.k, "since"), 0)),
  }))
  render()
end)

ygg.on("lane.release", function(payload)
  if stage() == STAGE.done then
    return reject("lane.release", TEXTS.beats.req_after_done)
  end
  local k, problem = LANES.parse_ref(payload)
  if problem ~= nil then
    return reject_payload("lane.release", problem)
  end
  local holder = LANES.req_of(k)
  LANES.clear(k)
  local freed = TEXTS.beats.lane_release_idle
  local index = req_index_of(holder)
  if index ~= nil and req_lane(index) == k then
    ygg.kv.set(req_key(index, "lane"), 0)
    freed = req_id(index)
  elseif holder ~= "" then
    freed = holder
  end
  ygg.narrate(fill(TEXTS.beats.lane_released, { k = tostring(k), id = freed }))
  render()
end)

ygg.on("proposal.submit", function(payload)
  if stage() == STAGE.done then
    return reject("proposal.submit", TEXTS.beats.req_after_done)
  end
  local spec, problem = parse_proposal(payload)
  if problem ~= nil then
    return reject_payload("proposal.submit", problem)
  end
  local existing = prop_by_ext(spec.ext)
  if existing ~= nil then
    ygg.narrate(fill(TEXTS.beats.req_prop_replayed, { ext = spec.ext, id = prop_id(existing) }))
    render()
    return
  end
  local index = req_index_of(spec.req)
  if index == nil then
    return reject_payload("proposal.submit", fill(TEXTS.beats.req_unknown, { id = tostring(spec.req) }))
  end
  local status = req_status(index)
  -- 每个阶段两态可收提案：「工作中」（首次）与该阶段的 review（**重定基**）。
  -- review 态只在「本需求没有待验收提案」时收（旧提案已 stale / 被拒），
  -- 有正常待验收提案时绝不覆盖（必须先 proposal.accept / reject）。
  local slots = {
    analysis = { working = R_ANALYZING, review = R_ANALYSIS_REVIEW },
    plan = { working = R_PLANNING, review = R_PLAN_REVIEW },
    development = { working = R_DEVELOPING, review = R_DEV_REVIEW },
  }
  local slot = slots[spec.stage]
  if status ~= slot.working and status ~= slot.review then
    return reject_payload("proposal.submit", fill(TEXTS.beats.req_prop_stage_mismatch, {
      id = req_id(index),
      stage = spec.stage,
      status = status,
    }))
  end
  if status == slot.review then
    local blocking = nil
    for i = 1, prop_count() do
      if prop_status(i) == P_PENDING and prop_req(i) == index then
        blocking = i
        break
      end
    end
    if blocking ~= nil then
      return refuse(TEXTS.beats.req_wait_rejected, "proposal.submit", fill(TEXTS.beats.req_prop_busy, {
        id = req_id(index),
        prop = prop_id(blocking),
      }))
    end
    -- 重定基：旧提案已作废，清掉需求的 stale 标记（本次提案重新起算）。
    ygg.kv.set(req_key(index, "stale"), false)
  end
  local summarize = spec.summary
  if summarize == "" then
    summarize = TEXTS.beats.req_prop_no_summary
  end
  local prop_index = prop_append(index, spec.rev, spec.stage, summarize, spec.artifacts, spec.ext, spec.changes)
  local review_status = {
    analysis = R_ANALYSIS_REVIEW,
    plan = R_PLAN_REVIEW,
    development = R_DEV_REVIEW,
  }
  ygg.kv.set(req_key(index, "status"), review_status[spec.stage])
  ygg.kv.set(req_key(index, "job"), "")
  ygg.narrate(fill(TEXTS.beats.req_prop_ok, {
    id = prop_id(prop_index),
    req = req_id(index),
    stage = spec.stage,
    rev = tostring(spec.rev),
    summary = summarize,
    pending = tostring(prop_pending_total()),
  }))
  render()
end)

local function pick_prop(target, allow_pending_only)
  local raw = trim(target or "")
  if raw == "" then
    return prop_next()
  end
  if raw:sub(1, 1) == "p" then
    local number = tonumber(raw:sub(2))
    if number == nil then
      return nil
    end
    number = math.floor(number)
    if number < 1 or number > prop_count() then
      return nil
    end
    if allow_pending_only and prop_status(number) ~= P_PENDING then
      return nil
    end
    return number
  end
  local req_index = req_index_of(raw)
  if req_index == nil then
    return nil
  end
  for i = 1, prop_count() do
    if prop_status(i) == P_PENDING and prop_req(i) == req_index then
      return i
    end
  end
  return nil
end

ygg.on("proposal.accept", function(payload)
  if not HUMAN.gated("proposal.accept") then
    return
  end
  if stage() == STAGE.done then
    return reject("proposal.accept", TEXTS.beats.req_after_done)
  end
  local prop_index = pick_prop(payload, true)
  if prop_index == nil then
    return reject_payload("proposal.accept", TEXTS.beats.req_prop_none)
  end
  local index = prop_req(prop_index)
  local stage_name = prop_stage(prop_index)
  -- 串行验收 + stale 判定：提案绑定 revision，旧 rev 不得覆盖新决定。
  if prop_rev(prop_index) ~= req_rev(index) then
    ygg.kv.set(prop_key(prop_index, "status"), P_STALE)
    ygg.kv.set(req_key(index, "stale"), true)
    return refuse(TEXTS.beats.req_wait_rejected, "proposal.accept", fill(TEXTS.beats.req_accept_stale, {
      id = prop_id(prop_index),
      req = req_id(index),
      got = tostring(prop_rev(prop_index)),
      want = tostring(req_rev(index)),
    }))
  end
  ygg.kv.set(req_key(index, "summary"), text_of(prop_key(prop_index, "summary"), ""))
  ygg.kv.set(req_key(index, "artifacts"), text_of(prop_key(prop_index, "artifacts"), ""))
  ygg.kv.set(prop_key(prop_index, "status"), P_ACCEPTED)
  local change_note = apply_changes(index, text_of(prop_key(prop_index, "changes"), ""))
  req_release(index)
  local next_status = req_advance_after_accept(index, stage_name)
  -- 门禁绑定 revision（待批期间 amend → stale，不得批准）
  if next_status == R_PLAN_GATE or next_status == R_ACCEPT then
    ygg.kv.set(req_key(index, "gate_rev"), req_rev(index))
  end
  req_push_history(index, fill(TEXTS.beats.req_accept_hist, { id = prop_id(prop_index), stage = stage_name }))
  req_sync_conflicts()
  ygg.narrate(fill(TEXTS.beats.req_accept_ok, {
    id = prop_id(prop_index),
    req = req_id(index),
    stage = stage_name,
    status = tostring(next_status or req_status(index)),
    change = change_note,
    pending = tostring(prop_pending_total()),
  }))
  render()
end)

ygg.on("proposal.reject", function(payload)
  if stage() == STAGE.done then
    return reject("proposal.reject", TEXTS.beats.req_after_done)
  end
  local target, reason = cut(trim(payload), "|")
  local prop_index = pick_prop(target, true)
  if prop_index == nil then
    return reject_payload("proposal.reject", TEXTS.beats.req_prop_none)
  end
  local index = prop_req(prop_index)
  local stage_name = prop_stage(prop_index)
  ygg.kv.set(prop_key(prop_index, "status"), P_REJECTED)
  local back = { analysis = R_DRAFT, plan = R_PLANNING, development = R_READY }
  req_release(index)
  ygg.kv.set(req_key(index, "status"), back[stage_name] or R_DRAFT)
  req_push_history(index, fill(TEXTS.beats.req_reject_hist, { id = prop_id(prop_index) }))
  ygg.narrate(fill(TEXTS.beats.req_reject_ok, {
    id = prop_id(prop_index),
    req = req_id(index),
    status = req_status(index),
    reason = trim(reason or "") ~= "" and trim(reason) or TEXTS.beats.req_reject_default,
  }))
  render()
end)

ygg.on("coordinate.submit", function(payload)
  if not HUMAN.gated("coordinate.submit") then
    return
  end
  local current = stage()
  if current ~= STAGE.coordinate then
    return reject("coordinate.submit", fill(TEXTS.beats.stage_expected, { expected = STAGE.coordinate }))
  end
  if not probe_payload_ok(payload) then
    return reject_payload("coordinate.submit", fill(TEXTS.beats.probe_rejected, {
      action = "coordinate.submit",
      got = tostring(payload),
      usage = TEXTS.beats.coordinate_usage,
    }))
  end
  local problem = gate_problem()
  if problem ~= nil then
    return refuse(TEXTS.beats.req_wait_rejected, "coordinate.submit", problem)
  end
  ygg.kv.set("verify.passed", false)
  ygg.narrate(fill(TEXTS.beats.coordinate_ok, { count = tostring(req_count()) }))
  advance(STAGE.review)
end)

ygg.on("verify.submit", function(payload)
  if not HUMAN.gated("verify.submit") then
    return
  end
  local current = stage()
  if current ~= STAGE.review then
    return reject("verify.submit", fill(TEXTS.beats.stage_expected, { expected = STAGE.review }))
  end
  local all_green = true
  local report = ""
  if type(payload) == "boolean" then
    all_green = payload
  elseif type(payload) == "string" then
    local raw = trim(payload)
    if raw == "false" then
      all_green = false
    elseif raw ~= "true" then
      local parsed = nil
      for _, segment in ipairs(split(raw, ";")) do
        local key, value = cut(trim(segment), "=")
        key = trim(key)
        value = trim(value or "")
        if key == "all_green" then
          parsed = value == "true"
        elseif key == "report" then
          report = value
        end
      end
      if parsed == nil then
        return reject_payload("verify.submit", fill(TEXTS.beats.bad_payload, { action = "verify.submit" }))
      end
      all_green = parsed
    end
  else
    return reject_payload("verify.submit", fill(TEXTS.beats.bad_payload, { action = "verify.submit" }))
  end
  if all_green then
    local problem = gate_problem()
    if problem ~= nil then
      ygg.narrate(fill(TEXTS.beats.req_gate_block, { action = "verify.submit=true", detail = problem }))
      render()
      return
    end
    ygg.kv.set("verify.passed", true)
    ygg.kv.set("verify.last", TEXTS.beats.verify_green_record)
    ygg.narrate(fill(TEXTS.beats.verify_ok, { count = tostring(req_count()) }))
    render()
    return
  end
  ygg.kv.set("verify.passed", false)
  local failures = num("verify.failures", 0) + 1
  ygg.kv.set("verify.failures", failures)
  ygg.kv.set("verify.last", report ~= "" and report or TEXTS.beats.verify_failed_record)
  if strict_gates() then
    ygg.narrate(fill(TEXTS.beats.verify_fail, { failures = tostring(failures) }))
    advance(STAGE.coordinate)
  else
    ygg.narrate(fill(TEXTS.beats.gate_bypass, { failures = tostring(failures) }))
    ygg.kv.set("verify.passed", true)
    render()
  end
end)

ygg.on("review.note", function(payload)
  if not HUMAN.gated("review.note") then
    return
  end
  local current = stage()
  if current ~= STAGE.review then
    return reject("review.note", fill(TEXTS.beats.stage_expected, { expected = STAGE.review }))
  end
  local problem = gate_problem()
  if problem ~= nil then
    ygg.narrate(fill(TEXTS.beats.req_gate_block, { action = "review.note", detail = problem }))
    render()
    return
  end
  if strict_gates() and ygg.kv.get("verify.passed") ~= true then
    ygg.narrate(TEXTS.beats.req_gate_verify)
    render()
    return
  end
  local lesson, score_text = cut(trim(payload), "|")
  lesson = trim(lesson)
  if lesson == "" then
    lesson = TEXTS.beats.default_lesson
  end
  local score = TEXTS.tuning.default_score
  local candidate = trim(score_text or "")
  if candidate ~= "" then
    local parsed = tonumber(candidate)
    if parsed == nil or parsed < 1 or parsed > 5 then
      return reject_payload("review.note", fill(TEXTS.beats.bad_score, { got = candidate }))
    end
    score = math.floor(parsed)
  end
  ygg.kv.set("review.lesson", lesson)
  ygg.kv.set("review.score", score)
  ygg.kv.set("ending", STAGE.done)
  ygg.narrate(fill(TEXTS.beats.review_ok, { score = tostring(score), lesson = lesson }))
  advance(STAGE.done)
end)

ygg.on("requirement.split", function(payload)
  local ext
  payload, ext = strip_ext(payload)
  if inbox_replay(ext) then
    render()
    return
  end
  local parent_id, children, problem = parse_split(payload)
  if problem ~= nil then
    return reject_payload("requirement.split", problem)
  end
  local parent = req_index_of(parent_id)
  if parent == nil then
    return reject_payload("requirement.split", fill(TEXTS.beats.req_unknown, { id = tostring(parent_id) }))
  end
  local status = req_status(parent)
  if status ~= R_DRAFT and status ~= R_READY then
    return refuse(TEXTS.beats.req_wait_rejected, "requirement.split", fill(TEXTS.beats.req_split_status, {
      id = req_id(parent),
      status = status,
    }))
  end
  if req_conflict(parent) ~= "" then
    return refuse(TEXTS.beats.req_wait_rejected, "requirement.split", fill(TEXTS.beats.req_split_blocked, { id = req_id(parent) }))
  end
  if req_open_total() + #children > req_capacity() then
    return reject_payload("requirement.split", fill(TEXTS.beats.req_full, { cap = tostring(req_capacity()) }))
  end
  local ids = req_do_split(parent, children)
  req_sync_conflicts()
  inbox_mark(ext)
  ygg.narrate(fill(TEXTS.beats.req_split_ok, {
    id = req_id(parent),
    ids = join(ids, ","),
    deps = join(req_deps(parent), ","),
  }))
  render()
end)

ygg.on("requirement.merge", function(payload)
  local ext
  payload, ext = strip_ext(payload)
  if inbox_replay(ext) then
    render()
    return
  end
  local ids, title, description, problem = parse_merge(payload)
  if problem ~= nil then
    return reject_payload("requirement.merge", problem)
  end
  local sources = {}
  local seen = {}
  for i = 1, #ids do
    local index = req_index_of(ids[i])
    if index == nil then
      return reject_payload("requirement.merge", fill(TEXTS.beats.req_unknown, { id = tostring(ids[i]) }))
    end
    if seen[index] then
      return reject_payload("requirement.merge", fill(TEXTS.beats.req_merge_dup, { id = req_id(index) }))
    end
    seen[index] = true
    local status = req_status(index)
    if status ~= R_DRAFT and status ~= R_READY then
      return refuse(TEXTS.beats.req_wait_rejected, "requirement.merge", fill(TEXTS.beats.req_merge_status, {
        id = req_id(index),
        status = status,
      }))
    end
    sources[#sources + 1] = index
  end
  local target, next_title, deps = req_do_merge(sources, title, description)
  req_sync_conflicts()
  inbox_mark(ext)
  ygg.narrate(fill(TEXTS.beats.req_merge_ok, {
    ids = join(ids, ","),
    id = req_id(target),
    title = next_title,
    deps = join(deps, ","),
  }))
  render()
end)

ygg.on("requirement.amend", function(payload)
  local ext
  payload, ext = strip_ext(payload)
  if inbox_replay(ext) then
    render()
    return
  end
  local id, title, description, dep_raw, problem = parse_amend(payload)
  if problem ~= nil then
    return reject_payload("requirement.amend", problem)
  end
  local index = req_index_of(id)
  if index == nil then
    return reject_payload("requirement.amend", fill(TEXTS.beats.req_unknown, { id = tostring(id) }))
  end
  local status = req_status(index)
  if status == R_SPLIT or status == R_MERGED or status == R_DROPPED then
    return refuse(TEXTS.beats.req_wait_rejected, "requirement.amend", fill(TEXTS.beats.req_amend_closed, {
      id = req_id(index),
      status = status,
    }))
  end
  if status == R_DONE then
    -- done 的修订**派生新需求**（不重开已验收的成果）。
    local next_title = title ~= "" and title or req_title(index)
    local next_desc = description ~= "" and description or text_of(req_key(index, "description"), TEXTS.beats.default_req_desc)
    local deps = dep_raw ~= "" and parse_deps(dep_raw) or {}
    local dep_problem = req_validate_deps(req_count() + 1, deps)
    if dep_problem ~= nil then
      return reject_payload("requirement.amend", dep_problem)
    end
    if req_open_total() >= req_capacity() then
      return reject_payload("requirement.amend", fill(TEXTS.beats.req_full, { cap = tostring(req_capacity()) }))
    end
    local created = req_append(next_title, next_desc, "", deps, req_id(index))
    local rev = req_bump_rev(index)
    req_push_history(index, fill(TEXTS.beats.req_amend_derive_hist, { rev = tostring(rev), id = req_id(created) }))
    inbox_mark(ext)
    ygg.narrate(fill(TEXTS.beats.req_amend_derived, {
      id = req_id(index),
      new = req_id(created),
      title = next_title,
      rev = tostring(rev),
    }))
    render()
    return
  end
  if status == R_DEVELOPING then
    if dep_raw ~= "" then
      return refuse(TEXTS.beats.req_wait_rejected, "requirement.amend", fill(TEXTS.beats.req_amend_deps_active, { id = req_id(index) }))
    end
    if title ~= "" then
      ygg.kv.set(req_key(index, "title"), title)
    end
    if description ~= "" then
      ygg.kv.set(req_key(index, "description"), description)
    end
    local rev = req_bump_rev(index)
    req_push_history(index, fill(TEXTS.beats.req_amend_hist_active, { rev = tostring(rev) }))
    inbox_mark(ext)
    ygg.narrate(fill(TEXTS.beats.req_amend_active, { id = req_id(index), rev = tostring(rev) }))
    render()
    return
  end
  local deps = nil
  if dep_raw ~= "" then
    deps = parse_deps(dep_raw)
    local dep_problem = req_validate_deps(index, deps)
    if dep_problem ~= nil then
      return reject_payload("requirement.amend", dep_problem)
    end
  end
  if title ~= "" then
    ygg.kv.set(req_key(index, "title"), title)
  end
  if description ~= "" then
    ygg.kv.set(req_key(index, "description"), description)
  end
  if deps ~= nil then
    ygg.kv.set(req_key(index, "deps"), join(deps, ","))
  end
  ygg.kv.set(req_key(index, "needs_amend"), false)
  local rev = req_bump_rev(index)
  req_push_history(index, fill(TEXTS.beats.req_amend_hist, { rev = tostring(rev) }))
  req_sync_conflicts()
  inbox_mark(ext)
  ygg.narrate(fill(TEXTS.beats.req_amend_ok, {
    id = req_id(index),
    rev = tostring(rev),
    title = req_title(index),
    deps = req_deps_text(index),
  }))
  render()
end)

ygg.on("requirement.conflict", function(payload)
  local ext
  payload, ext = strip_ext(payload)
  if inbox_replay(ext) then
    render()
    return
  end
  local ids, note, problem = parse_conflict(payload)
  if problem ~= nil then
    return reject_payload("requirement.conflict", problem)
  end
  local first = req_index_of(ids[1])
  local second = req_index_of(ids[2])
  if first == nil then
    return reject_payload("requirement.conflict", fill(TEXTS.beats.req_unknown, { id = tostring(ids[1]) }))
  end
  if second == nil then
    return reject_payload("requirement.conflict", fill(TEXTS.beats.req_unknown, { id = tostring(ids[2]) }))
  end
  if first == second then
    return reject_payload("requirement.conflict", TEXTS.beats.req_conflict_dup)
  end
  for _, index in ipairs({ first, second }) do
    if req_closed(index) then
      return reject_payload("requirement.conflict", fill(TEXTS.beats.req_conflict_status, {
        id = req_id(index),
        status = req_status(index),
      }))
    end
    if req_status(index) == R_DEVELOPING then
      return refuse(TEXTS.beats.req_wait_rejected, "requirement.conflict", fill(TEXTS.beats.req_conflict_busy, {
        id = req_id(index),
      }))
    end
    if req_conflict(index) ~= "" then
      return refuse(TEXTS.beats.req_wait_rejected, "requirement.conflict", fill(TEXTS.beats.req_conflict_exists, {
        id = req_id(index),
        peer = req_conflict_peer(index),
      }))
    end
  end
  req_do_conflict(first, second, note)
  req_sync_conflicts()
  inbox_mark(ext)
  ygg.narrate(fill(TEXTS.beats.req_conflict_ok, {
    first = req_id(first),
    second = req_id(second),
    note = note,
    modes = TEXTS.beats.req_resolve_modes,
  }))
  render()
end)

ygg.on("requirement.resolve", function(payload)
  local ext
  payload, ext = strip_ext(payload)
  if inbox_replay(ext) then
    render()
    return
  end
  local ids, mode, note, problem = parse_resolve(payload)
  if problem ~= nil then
    return reject_payload("requirement.resolve", problem)
  end
  local first = req_index_of(ids[1])
  local second = req_index_of(ids[2])
  if first == nil then
    return reject_payload("requirement.resolve", fill(TEXTS.beats.req_unknown, { id = tostring(ids[1]) }))
  end
  if second == nil then
    return reject_payload("requirement.resolve", fill(TEXTS.beats.req_unknown, { id = tostring(ids[2]) }))
  end
  if first == second then
    return reject_payload("requirement.resolve", TEXTS.beats.req_conflict_dup)
  end
  if mode == "keep_first" and req_status(second) ~= R_DRAFT and req_status(second) ~= R_READY then
    return refuse(TEXTS.beats.req_wait_rejected, "requirement.resolve", fill(TEXTS.beats.req_resolve_drop_busy, {
      id = req_id(second),
      status = req_status(second),
    }))
  end
  if mode == "revise" then
    for _, index in ipairs({ first, second }) do
      if req_status(index) ~= R_DRAFT and req_status(index) ~= R_READY then
        return refuse(TEXTS.beats.req_wait_rejected, "requirement.resolve", fill(TEXTS.beats.req_resolve_revise_busy, {
          id = req_id(index),
          status = req_status(index),
        }))
      end
    end
  end
  ygg.kv.set(req_key(first, "conflict"), "")
  ygg.kv.set(req_key(second, "conflict"), "")
  ygg.kv.set(req_key(first, "conflict_peer"), "")
  ygg.kv.set(req_key(second, "conflict_peer"), "")
  local effect
  if mode == "keep_first" then
    ygg.kv.set(req_key(second, "status"), R_DROPPED)
    req_release(second)
    req_push_history(second, fill(TEXTS.beats.req_resolve_drop_hist, { note = note }))
    effect = fill(TEXTS.beats.req_resolve_keep_first, { id = req_id(second) })
  elseif mode == "revise" then
    ygg.kv.set(req_key(first, "needs_amend"), true)
    ygg.kv.set(req_key(second, "needs_amend"), true)
    req_push_history(first, fill(TEXTS.beats.req_resolve_revise_hist, { note = note }))
    req_push_history(second, fill(TEXTS.beats.req_resolve_revise_hist, { note = note }))
    effect = TEXTS.beats.req_resolve_revised
  else
    req_push_history(first, fill(TEXTS.beats.req_resolve_keep_hist, { note = note }))
    req_push_history(second, fill(TEXTS.beats.req_resolve_keep_hist, { note = note }))
    effect = TEXTS.beats.req_resolve_kept
  end
  req_sync_conflicts()
  inbox_mark(ext)
  ygg.narrate(fill(TEXTS.beats.req_resolve_ok, {
    first = req_id(first),
    second = req_id(second),
    mode = mode,
    effect = effect,
  }))
  render()
end)

local function pick_gate(target)
  local raw = trim(target or "")
  if raw == "" then
    return gate_next()
  end
  local index = req_index_of(raw)
  if index == nil or gate_kind(index) == nil then
    return nil
  end
  return index
end

ygg.on("gate.approve", function(payload)
  if not HUMAN.gated("gate.approve") then
    return
  end
  if stage() == STAGE.done then
    return reject("gate.approve", TEXTS.beats.req_after_done)
  end
  local target, note = cut(trim(payload), "|")
  local index = pick_gate(target)
  if index == nil then
    return reject_payload("gate.approve", TEXTS.beats.req_gate_none)
  end
  local kind = gate_kind(index)
  -- stale：待批期间被 amend 过 → 旧门禁不得批准。
  if not gate_rev_ok(index) then
    return refuse(TEXTS.beats.req_wait_rejected, "gate.approve", fill(TEXTS.beats.req_gate_stale, {
      id = req_id(index),
      got = tostring(num(req_key(index, "gate_rev"), 0)),
      want = tostring(req_rev(index)),
    }))
  end
  -- 证据硬约束：accept_gate 不接受只有 summary 的完成自称。
  local evidence = gate_evidence_problem(index, kind)
  if evidence ~= nil then
    return refuse(TEXTS.beats.req_wait_rejected, "gate.approve", evidence)
  end
  local next_status = kind == "plan" and R_READY or R_DONE
  ygg.kv.set(req_key(index, "status"), next_status)
  req_release(index)
  req_push_history(index, fill(TEXTS.beats.req_gate_hist, { kind = kind }))
  req_sync_conflicts()
  ygg.narrate(fill(TEXTS.beats.req_gate_ok, {
    id = req_id(index),
    kind = kind,
    status = next_status,
    note = trim(note or "") ~= "" and trim(note) or TEXTS.beats.req_gate_no_note,
    artifacts = req_artifacts(index) ~= "" and req_artifacts(index) or TEXTS.beats.req_gate_no_artifacts,
    gates = tostring(gate_total()),
  }))
  render()
end)

ygg.on("gate.reject", function(payload)
  if stage() == STAGE.done then
    return reject("gate.reject", TEXTS.beats.req_after_done)
  end
  local target, feedback = cut(trim(payload), "|")
  local index = pick_gate(target)
  if index == nil then
    return reject_payload("gate.reject", TEXTS.beats.req_gate_none)
  end
  local kind = gate_kind(index)
  local back = kind == "plan" and R_PLANNING or R_READY
  ygg.kv.set(req_key(index, "status"), back)
  req_release(index)
  -- 打回不改 rev（执行方下一轮提案仍用当前 rev），但记录反馈供下轮参考。
  ygg.kv.set(req_key(index, "feedback"), trim(feedback or "") ~= "" and trim(feedback) or TEXTS.beats.req_reject_default)
  req_push_history(index, fill(TEXTS.beats.req_gate_reject_hist, { kind = kind }))
  req_sync_conflicts()
  ygg.narrate(fill(TEXTS.beats.req_gate_rejected, {
    id = req_id(index),
    kind = kind,
    status = back,
    feedback = text_of(req_key(index, "feedback"), TEXTS.beats.req_reject_default),
    gates = tostring(gate_total()),
  }))
  render()
end)

ygg.on("config.set", function(payload)
  local current = stage()
  if type(payload) ~= "string" then
    return reject("config.set", fill(TEXTS.beats.bad_payload, { action = "config.set" }))
  end
  local key, value = cut(trim(payload), "=")
  key = trim(key)
  value = trim(value or "")
  if key:sub(1, 9) == "settings." then
    key = key:sub(10)
  end
  local problem = nil
  if key == "play.mode" then
    -- 切模式会改变「谁在派发」：guide→goal 等于给后续直派开闸，故单重门禁。
    if not HUMAN.needs("config.set play.mode") then
      return
    end
    if not contains(PLAY_MODES, value) then
      problem = fill(TEXTS.beats.bad_config_value, { key = key, value = value, allowed = join(PLAY_MODES, " / ") })
    else
      ygg.kv.set("settings.play.mode", value)
      -- 驱动者换了人，二级页的「当前处境」已失效：退出子页回阶段菜单。
      CONFIG_UI.set_page("")
    end
  elseif key == "evolution.mode" then
    if not contains(MODES, value) then
      problem = fill(TEXTS.beats.bad_config_value, { key = key, value = value, allowed = join(MODES, " / ") })
    else
      ygg.kv.set("settings.evolution.mode", value)
    end
  elseif key == "evolution.max_per_run" then
    local parsed = tonumber(value)
    if parsed == nil or parsed < 0 or parsed > 8 then
      problem = fill(TEXTS.beats.bad_config_value, { key = key, value = value, allowed = "0–8 的整数" })
    else
      ygg.kv.set("settings.evolution.max_per_run", math.floor(parsed))
    end
  elseif key == "gates.require_all_green" or key == "autosave" or key == "hot_reload" then
    local parsed = to_bool(value)
    if parsed == nil then
      problem = fill(TEXTS.beats.bad_config_value, { key = key, value = value, allowed = "true / false" })
    else
      ygg.kv.set("settings." .. key, parsed)
    end
  else
    problem = fill(TEXTS.beats.bad_config_key, { key = key, allowed = TEXTS.beats.config_keys })
  end
  if problem ~= nil then
    return reject_payload("config.set", problem)
  end
  ygg.narrate(fill(TEXTS.beats.config_changed, {
    key = key,
    value = tostring(effective(key)),
    stage = current,
  }))
  render()
end)

ygg.on("config.open", function(payload)
  if not probe_payload_ok(payload) then
    return reject_payload("config.open", fill(TEXTS.beats.probe_rejected, {
      action = "config.open",
      got = tostring(payload),
      usage = TEXTS.beats.config_open_usage,
    }))
  end
  CONFIG_UI.set_page("config", stage())
  ygg.narrate(TEXTS.beats.config_opened)
  render()
end)

ygg.on("config.close", function(payload)
  if not probe_payload_ok(payload) then
    return reject_payload("config.close", fill(TEXTS.beats.probe_rejected, {
      action = "config.close",
      got = tostring(payload),
      usage = TEXTS.beats.config_open_usage,
    }))
  end
  CONFIG_UI.set_page("")
  ygg.narrate(fill(TEXTS.beats.config_closed, { stage = stage() }))
  render()
end)

ygg.on("guide.show", function(payload)
  local current = stage()
  local id = trim(payload)
  if id == "" then
    id = guide_section(current).id
  end
  narrate_guide(id)
  render()
end)

ygg.on("map.show", function(payload)
  if probe_payload_ok(payload) then
    narrate_map(stage())
  else
    ygg.narrate(fill(TEXTS.beats.map_rejected, { got = tostring(payload), usage = TEXTS.beats.map_usage }))
  end
  render()
end)

ygg.on("ygg.choice_selected", function(index)
  local current = stage()
  local items = menu(current)
  if items == nil then
    ygg.narrate(fill(TEXTS.beats.unknown_choice, { stage = current }))
    render()
    return
  end
  local picked = tonumber(index)
  if picked == nil or items[picked] == nil then
    ygg.narrate(fill(TEXTS.beats.bad_index, {
      stage = current,
      index = tostring(index),
      count = tostring(#items),
    }))
    render()
    return
  end
  -- 盖章：只能在这里盖章（菜单路由），单发直派没有盖章机会。
  local label = HUMAN.label(items[picked].action, items[picked].payload)
  if label ~= nil then
    HUMAN.ticket = label
  end
  -- 二级页只跟 config.* 走：选了任何阶段动作就离开子页（否则子页会占住阶段菜单）。
  if items[picked].action:sub(1, 7) ~= "config." then
    CONFIG_UI.set_page("")
  end
  ygg.emit(items[picked].action, items[picked].payload)
end)

ygg.on("event.start", function()
  ygg.narrate(fill(TEXTS.scenes.prologue, {
    engine = tostring(ygg.engine_version),
    stage = stage(),
    mode = mode(),
  }))
  ygg.narrate(fill(stage_prompt("prologue"), prompt_vars()))
  ygg.narrate(TEXTS.beats.secret_redline)
  render()
end)

ygg.narrative.node("node:render", function()
  render()
end)

ygg.fsm.define({
  name = "stage",
  initial = STAGE.intake,
  event = "stage.advance",
  transitions = {
    {
      from = STAGE.intake,
      to = STAGE.coordinate,
      when = function(target)
        return target == STAGE.coordinate
      end,
    },
    {
      from = STAGE.coordinate,
      to = STAGE.review,
      when = function(target)
        return target == STAGE.review
      end,
    },
    {
      from = STAGE.review,
      to = STAGE.coordinate,
      when = function(target)
        return target == STAGE.coordinate
      end,
    },
    {
      from = STAGE.review,
      to = STAGE.done,
      when = function(target)
        return target == STAGE.done
      end,
    },
  },
})

