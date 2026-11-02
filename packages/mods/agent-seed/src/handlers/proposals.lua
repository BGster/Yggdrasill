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
local R_ANALYZING, R_DEVELOPING, R_DRAFT, R_PLANNING, STAGE, req_advance_after_accept, req_gate, req_id, req_index_of, req_key, req_phase_name, req_plan_flag, req_plan_ok, req_push_history, req_release, req_rev, req_status, req_sync_conflicts, stage =
  reqs.R_ANALYZING, reqs.R_DEVELOPING, reqs.R_DRAFT, reqs.R_PLANNING, reqs.STAGE, reqs.req_advance_after_accept, reqs.req_gate, reqs.req_id, reqs.req_index_of, reqs.req_key, reqs.req_phase_name, reqs.req_plan_flag, reqs.req_plan_ok, reqs.req_push_history, reqs.req_release, reqs.req_rev, reqs.req_status, reqs.req_sync_conflicts, reqs.stage
local cut, fill, trim =
  util.cut, util.fill, util.trim
local parse_proposal =
  wire.parse_proposal

-- 阶段槽位（5 态）：旧的 working / review 两态已合并成同一个存储态，
-- 槽位只判「这个阶段现在能不能收提案」；「重定基 vs 被占用」由 pending 提案表判。
local WORKING_STATUS = {
  analysis = R_ANALYZING,
  plan = R_PLANNING,
  development = R_DEVELOPING,
}

-- 返回 (ok, reason)：reason ∈ "status" / "gate" / "plan_on" / "plan_off"（ok 时为 nil）。
-- reason 必须区分开：把「计划未批准」报成「阶段与状态不匹配」会把排错人引到错方向。
local function submit_slot_ok(index, stage_name)
  if req_status(index) ~= WORKING_STATUS[stage_name] then
    return false, "status"
  end
  -- 门禁待批期间不得再收提案（plan 门禁落在 planning、accept 门禁落在 developing）。
  if req_gate(index) ~= "" then
    return false, "gate"
  end
  -- plan 槽位：**只认显式 true**。字段缺席（升级前的在途行）不得当成已批准，
  -- 否则升级瞬间卡在 planning 的行会连 plan 提案都收不进来。
  if stage_name == "plan" then
    if req_plan_flag(index) == true then
      return false, "plan_on"
    end
    return true, nil
  end
  -- development 槽位：逻辑判定（req_plan_ok 对缺席行按状态推断：已处于 developing
  -- 就说明旧模型下已过 plan 门禁 → 放行；显式 false 则仍拒收）。
  if stage_name == "development" and not req_plan_ok(index) then
    return false, "plan_off"
  end
  return true, nil
end

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
    -- 阶段槽位（5 态）：旧的 working / review 两态合并成同一个存储态，槽位只判阶段可达性。
    local slot_ok, slot_reason = submit_slot_ok(index, spec.stage)
    if not slot_ok then
      local detail
      if slot_reason == "plan_off" then
        detail = fill(TEXTS.beats.req_prop_plan_missing, {
          id = req_id(index),
          status = req_phase_name(index),
        })
      else
        detail = fill(TEXTS.beats.req_prop_stage_mismatch, {
          id = req_id(index),
          stage = spec.stage,
          status = req_phase_name(index),
        })
      end
      return reject_payload("proposal.submit", detail)
    end
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
    -- 重定基：旧提案已作废（stale / 被拒），清掉需求的 stale 标记（本次提案重新起算）。
    ygg.kv.set(req_key(index, "stale"), false)
    local summarize = spec.summary
    if summarize == "" then
      summarize = TEXTS.beats.req_prop_no_summary
    end
    local prop_index = prop_append(index, spec.rev, spec.stage, summarize, spec.artifacts, spec.ext, spec.changes)
    -- 状态不变（5 态下「等验收」由 pending 提案表表达）；只释放 job 槽位。
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
    req_push_history(index, fill(TEXTS.beats.req_accept_hist, { id = prop_id(prop_index), stage = stage_name }))
    req_sync_conflicts()
    ygg.narrate(fill(TEXTS.beats.req_accept_ok, {
      id = prop_id(prop_index),
      req = req_id(index),
      stage = stage_name,
      status = next_status ~= nil and req_phase_name(index) or req_status(index),
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
    -- 5 态下三路回退只有两个落点：analysis → draft；plan / development → planning（展示名由
    -- plan_ok 派生而不落存储）：development 回退沿用原值——未 amend 即「旧 ready」，amend 过
    -- 则 req_bump_rev 早已把它置假，落回 planning；plan 回退时 plan_ok 仍为假。
    local back = { analysis = R_DRAFT, plan = R_PLANNING, development = R_PLANNING }
    req_release(index)
    ygg.kv.set(req_key(index, "status"), back[stage_name] or R_DRAFT)
    req_push_history(index, fill(TEXTS.beats.req_reject_hist, { id = prop_id(prop_index) }))
    ygg.narrate(fill(TEXTS.beats.req_reject_ok, {
      id = prop_id(prop_index),
      req = req_id(index),
      status = req_phase_name(index),
      reason = trim(reason or "") ~= "" and trim(reason) or TEXTS.beats.req_reject_default,
    }))
    render()
  end)
end

M.pick_prop = pick_prop

return M
