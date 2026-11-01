-- src/handlers/proposals.lua —— 提案入口与串行验收：proposal.submit|accept|reject。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 注册面全部在 install() 内（派发序 = 入口 install 调用序）。

local M = {}

local config = require("src.config")
local flow = require("src.flow")
local choice = require("src.handlers.choice")
local props = require("src.proposals")
local render_mod = require("src.render")
local reqs = require("src.requirements")
local util = require("src.util")
local wire = require("src.wire")

local TEXTS

local text_of =
  config.text_of
local refuse, reject, reject_payload =
  flow.refuse, flow.reject, flow.reject_payload
local HUMAN =
  choice.HUMAN
local P_ACCEPTED, P_PENDING, P_REJECTED, P_STALE, apply_changes, prop_append, prop_by_ext, prop_count, prop_id, prop_key, prop_next, prop_pending_total, prop_req, prop_rev, prop_stage, prop_status =
  props.P_ACCEPTED, props.P_PENDING, props.P_REJECTED, props.P_STALE, props.apply_changes, props.prop_append, props.prop_by_ext, props.prop_count, props.prop_id, props.prop_key, props.prop_next, props.prop_pending_total, props.prop_req, props.prop_rev, props.prop_stage, props.prop_status
local render =
  render_mod.receipt
local R_ACCEPT, R_ANALYSIS_REVIEW, R_ANALYZING, R_DEVELOPING, R_DEV_REVIEW, R_DRAFT, R_PLANNING, R_PLAN_GATE, R_PLAN_REVIEW, R_READY, STAGE, req_advance_after_accept, req_id, req_index_of, req_key, req_push_history, req_release, req_rev, req_status, req_sync_conflicts, stage =
  reqs.R_ACCEPT, reqs.R_ANALYSIS_REVIEW, reqs.R_ANALYZING, reqs.R_DEVELOPING, reqs.R_DEV_REVIEW, reqs.R_DRAFT, reqs.R_PLANNING, reqs.R_PLAN_GATE, reqs.R_PLAN_REVIEW, reqs.R_READY, reqs.STAGE, reqs.req_advance_after_accept, reqs.req_id, reqs.req_index_of, reqs.req_key, reqs.req_push_history, reqs.req_release, reqs.req_rev, reqs.req_status, reqs.req_sync_conflicts, reqs.stage
local cut, fill, trim =
  util.cut, util.fill, util.trim
local parse_proposal =
  wire.parse_proposal

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

function M.install(deps)
  TEXTS = deps.texts
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
end

M.pick_prop = pick_prop

return M
