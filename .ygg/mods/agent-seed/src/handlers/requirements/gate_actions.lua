-- src/handlers/requirements/gate_actions.lua —— 两个强制门禁：gate.approve|reject。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 注册面全部在 install() 内（派发序 = 入口 install 调用序）。
-- D-MA23：approve(plan) → 待开发（plan_ok=true + 响应式自动派发）；
-- approve(accept) → 集成（integrating=true，不再直接 done——done 只经 integration.confirm）。

local M = {}

local config = require("src.core.config")
local flow = require("src.runtime.flow")
local gates = require("src.domain.gates")
local hooks = require("src.core.hooks")
local choice = require("src.handlers.operate.choice")
local render_mod = require("src.view.render")
local reqs = require("src.domain.requirements")
local tools = require("src.runtime.tools")
local util = require("src.core.util")

local TEXTS

local num, text_of =
  config.num, config.text_of
local phase_guard, refuse, reject_payload =
  flow.phase_guard, flow.refuse, flow.reject_payload
local gate_evidence_problem, gate_kind, gate_next, gate_problem_for, gate_rev_ok, gate_total, req_artifacts =
  gates.gate_evidence_problem, gates.gate_kind, gates.gate_next, gates.gate_problem_for, gates.gate_rev_ok, gates.gate_total, gates.req_artifacts
local HUMAN =
  choice.HUMAN
local render =
  render_mod.receipt
local release_lane_with_worktree =
  tools.release_lane_with_worktree
local RUN_PHASE, req_gate, req_id, req_index_of, req_key, req_phase, req_plan_ok, req_push_history, req_rev, req_sync_conflicts =
  reqs.RUN_PHASE, reqs.req_gate, reqs.req_id, reqs.req_index_of, reqs.req_key, reqs.req_phase, reqs.req_plan_ok, reqs.req_push_history, reqs.req_rev, reqs.req_sync_conflicts
local props = require("src.domain.proposals")
local P_PENDING, P_ACCEPTED, P_REJECTED, prop_count, prop_key, prop_req, prop_stage, prop_status =
  props.P_PENDING, props.P_ACCEPTED, props.P_REJECTED, props.prop_count, props.prop_key, props.prop_req, props.prop_stage, props.prop_status
local cut, fill, trim =
  util.cut, util.fill, util.trim

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

-- 升起本门禁的挂起提案（plan 门禁由 plan 提案提交即升起）：裁决时一并结算其状态，
-- 否则遗留 pending 会挡住后续提案（第二个 proposal.submit 拒绝面）。
local function settle_prop(index, stage_name, status)
  for i = 1, prop_count() do
    if prop_req(i) == index and prop_stage(i) == stage_name and prop_status(i) == P_PENDING then
      ygg.kv.set(prop_key(i, "status"), status)
      return i
    end
  end
  return nil
end

function M.install(deps)
  TEXTS = deps.texts
  ygg.on("gate.approve", function(payload)
    if not HUMAN.gated("gate.approve") then
      return
    end
    if not phase_guard("gate.approve", { RUN_PHASE.coordinate }) then
      return
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
    -- 单需求门禁判据（批准即开工的前半）：rN 的门禁只被 rN 自身事态阻塞——其它需求的
    -- pending 提案 / 在跑 job / 冲突不进本判定。gate.reject 不加判据（纠错通道保持畅通）。
    local problem = gate_problem_for(index)
    if problem ~= nil then
      return refuse(TEXTS.beats.req_wait_rejected, "gate.approve", problem)
    end
    -- 升起本门禁的 plan 提案随裁决落定（approved）；accept 门禁的 dev 提案在受理时已 accepted。
    settle_prop(index, "plan", P_ACCEPTED)
    ygg.kv.set(req_key(index, "gate"), "")
    if kind == "plan" then
      -- 计划门禁批准 → 待开发：plan_ok 置真（ready）；job 槽已空（提交时已清）。
      ygg.kv.set(req_key(index, "plan_ok"), true)
    else
      -- 验收门禁批准 → 集成：integrating=true；**不落 done、不释放泳道**（R7：
      -- 执行者仍需工作树完成 feature→develop 集成，done 只经 integration.confirm）。
      ygg.kv.set(req_key(index, "integrating"), true)
    end
    req_push_history(index, fill(TEXTS.beats.req_gate_hist, { kind = kind }))
    req_sync_conflicts()
    ygg.narrate(fill(TEXTS.beats.req_gate_ok, {
      id = req_id(index),
      kind = kind,
      status = req_phase(index),
      note = trim(note or "") ~= "" and trim(note) or TEXTS.beats.req_gate_no_note,
      artifacts = req_artifacts(index) ~= "" and req_artifacts(index) or TEXTS.beats.req_gate_no_artifacts,
      gates = tostring(gate_total()),
    }))
    -- 唯一后继自动推进走响应式：plan 批准 = 待开发，开发槽位已打开——打标后由 render() 入口的
    -- hooks.flush_effects 在产出 choice 之前同步完成单需求派活（省一回合）。
    if kind == "plan" then
      hooks.mark_auto_dispatch(index)
    end
    render()
  end)
  ygg.on("gate.reject", function(payload)
    if not phase_guard("gate.reject", { RUN_PHASE.coordinate }) then
      return
    end
    local target, feedback = cut(trim(payload), "|")
    local index = pick_gate(target)
    if index == nil then
      return reject_payload("gate.reject", TEXTS.beats.req_gate_none)
    end
    local kind = gate_kind(index)
    -- 打回落点（§2.4 转场表；两条都 rev+1）：
    --   plan 打回   → 计划中：plan_ok 置假、job 重新挂回 plan（计划任务回到执行者手上重做）、
    --                 升起门禁的 plan 提案落 rejected（腾出提交位）；
    --   accept 打回 → 待开发（R6 三条打回之一）：plan_ok 保持（keep 在 bump 前取——
    --                 req_bump_rev 会把 plan_ok 置假，验收打回不重置计划）、
    --                 释放泳道（连工作树）、dev 提案已在受理时落 accepted。
    local keep = req_plan_ok(index)
    settle_prop(index, "plan", P_REJECTED)
    ygg.kv.set(req_key(index, "gate"), "")
    if kind == "plan" then
      req_bump_rev(index)
      ygg.kv.set(req_key(index, "job"), "plan")
    else
      req_bump_rev(index)
      ygg.kv.set(req_key(index, "plan_ok"), keep)
      release_lane_with_worktree(index)
    end
    -- 打回记录反馈供下轮参考（rev 已 bump，下一轮提案用新 rev）。
    ygg.kv.set(req_key(index, "feedback"), trim(feedback or "") ~= "" and trim(feedback) or TEXTS.beats.req_reject_default)
    req_push_history(index, fill(TEXTS.beats.req_gate_reject_hist, { kind = kind }))
    req_sync_conflicts()
    ygg.narrate(fill(TEXTS.beats.req_gate_rejected, {
      id = req_id(index),
      kind = kind,
      status = req_phase(index),
      feedback = text_of(req_key(index, "feedback"), TEXTS.beats.req_reject_default),
      gates = tostring(gate_total()),
    }))
    render()
  end)
end

M.pick_gate = pick_gate

return M
