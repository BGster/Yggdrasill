-- src/domain/gates.lua —— 门禁与收口判据（只读选择器）：plan_gate / accept_gate / coordinate.submit 共用同一套。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 资源由入口 install(deps) 注入。本模块唯一写的 KV 前缀：无（只读）。

local M = {}

local config = require("src.core.config")
local props = require("src.domain.proposals")
local reqs = require("src.domain.requirements")
local util = require("src.core.util")

local TEXTS

local num, text_of =
  config.num, config.text_of
local P_PENDING, prop_count, prop_pending_total, prop_req, prop_status =
  props.P_PENDING, props.prop_count, props.prop_pending_total, props.prop_req, props.prop_status
local job_capacity, job_total, req_all_done, req_conflict, req_conflict_total, req_count, req_gate, req_id, req_job, req_key, req_needs_amend, req_open_total, req_rev, req_status =
  reqs.job_capacity, reqs.job_total, reqs.req_all_done, reqs.req_conflict, reqs.req_conflict_total, reqs.req_count, reqs.req_gate, reqs.req_id, reqs.req_job, reqs.req_key, reqs.req_needs_amend, reqs.req_open_total, reqs.req_rev, reqs.req_status
local fill, join, trim =
  util.fill, util.join, util.trim

-- 门禁种类现在由 `req.gate`（""/"plan"/"accept"）承载，不再是一个状态。
local function gate_kind(index)
  local kind = req_gate(index)
  if kind == "" then
    return nil
  end
  return kind
end

local function gate_next()
  for i = 1, req_count() do
    if gate_kind(i) ~= nil then
      return i
    end
  end
  return nil
end

local function gate_total()
  local total = 0
  for i = 1, req_count() do
    if gate_kind(i) ~= nil then
      total = total + 1
    end
  end
  return total
end

local function req_artifacts(index)
  return text_of(req_key(index, "artifacts"), "")
end

local function gate_evidence_problem(index, kind)
  if kind == "accept" and req_artifacts(index) == "" then
    return fill(TEXTS.beats.req_gate_no_evidence, { id = req_id(index) })
  end
  return nil
end

local function gate_rev_ok(index)
  -- gate_rev 从不清零，所以「仍在门禁上」是硬前提（否则已批准的需求会被重复批准）。
  return gate_kind(index) ~= nil and num(req_key(index, "gate_rev"), 0) == req_rev(index)
end

local function gate_problem()
  -- r2 rev2（B 方案⑥）空局保护两条款：章程非空 ∧ 需求数 ≥1，否则三个收口动作
  -- （coordinate.submit / verify.submit / review.note）共用同一条可读拒绝。
  if trim(text_of("task.title", "")) == "" then
    return TEXTS.beats.gate_charter_first
  end
  if req_count() == 0 then
    return TEXTS.beats.gate_charter_first
  end
  if prop_pending_total() > 0 then
    return fill(TEXTS.beats.req_gate_props, { pending = tostring(prop_pending_total()) })
  end
  if job_total() > 0 then
    return fill(TEXTS.beats.req_gate_jobs, { jobs = tostring(job_total()), cap = tostring(job_capacity()) })
  end
  if req_conflict_total() > 0 then
    return fill(TEXTS.beats.req_gate_conflicts, { conflicts = tostring(req_conflict_total()) })
  end
  if not req_all_done() then
    return fill(TEXTS.beats.req_gate_open, { open = tostring(req_open_total()) })
  end
  return nil
end

-- 需求自身的 pending 提案数（gate_problem_for 的计数口径；与 requirements.req_has_pending
-- 同读一张 prop.* 表、同一个 pending 状态，布尔版只给真值不给数量，这里要有 {{count}} 文案）。
local function req_pending_total(index)
  local total = 0
  for i = 1, prop_count() do
    if prop_status(i) == P_PENDING and prop_req(i) == index then
      total = total + 1
    end
  end
  return total
end

-- 单需求门禁判据（推进类专用，与 gate_problem 的 run 收口/证据全局屏障分层）：
-- rN 的门禁只被 rN 自身事态阻塞——其它需求的 pending 提案 / 在跑 job / 冲突不进本判定。
-- 依序返回第一个命中（人类可读细节），否则 nil：
--   1) 自身 pending 提案（防御性：正常流被 submit_slot_ok 的 gate 守卫 + req_prop_busy 挡住）
--   2) 自身在跑 job（防御性：提案提交时已清 job——plan 交付即门禁，dev 落受理幕）
--   3) 自身未解决冲突 / needs_amend（唯一真实可共存：冲突雷达 / resolve revise 可在待批期落）
-- 显式不含：stale / gate_rev 失配（gate.approve 的 gate_rev_ok 已查，避免双报）、
-- req_all_done / req_open_total（收口专用）与一切全局计数。
local function gate_problem_for(index)
  local own = req_pending_total(index)
  if own > 0 then
    return fill(TEXTS.beats.req_gate_own_props, { id = req_id(index), count = tostring(own) })
  end
  local job = req_job(index)
  if job ~= "" then
    return fill(TEXTS.beats.req_gate_own_job, { id = req_id(index), job = job })
  end
  if req_conflict(index) ~= "" or req_needs_amend(index) then
    return fill(TEXTS.beats.req_gate_own_conflict, { id = req_id(index) })
  end
  return nil
end

-- 局末清理前件（收口专用，与 gate_problem 分层）：
-- gate_problem 管需求侧（待验收提案 / 在跑 job / 未解决冲突 / 未 done）；
-- 这里管 run 级结算（进化复核 / subagent 会话 / 主会话 Todo）。全部通过才允许 review.note。
-- Lua 沙箱没有文件、会话与 Todo API：本函数只校登记断言，不证明实际复核或清理发生过。
local function settlement_problem()
  if ygg.kv.get("verify.passed") ~= true then
    return TEXTS.beats.settlement_verify_first
  end
  local missing = {}
  if trim(text_of("settlement.evolution.summary", "")) == "" then
    missing[#missing + 1] = TEXTS.beats.settlement_evolution_missing
  end
  if text_of("settlement.sessions.destination", "") == "" then
    missing[#missing + 1] = TEXTS.beats.settlement_sessions_missing
  end
  if num("settlement.todo.remaining", -1) ~= 0 then
    missing[#missing + 1] = TEXTS.beats.settlement_todo_missing
  end
  if #missing > 0 then
    return join(missing, "; ")
  end
  return nil
end

function M.install(deps)
  TEXTS = deps.texts
end

M.gate_evidence_problem = gate_evidence_problem
M.gate_kind = gate_kind
M.gate_next = gate_next
M.gate_problem = gate_problem
M.gate_problem_for = gate_problem_for
M.gate_rev_ok = gate_rev_ok
M.gate_total = gate_total
M.req_artifacts = req_artifacts
M.settlement_problem = settlement_problem

return M
