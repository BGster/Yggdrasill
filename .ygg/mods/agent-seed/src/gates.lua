-- src/gates.lua —— 门禁与收口判据（只读选择器）：plan_gate / accept_gate / coordinate.submit 共用同一套。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 资源由入口 install(deps) 注入。本模块唯一写的 KV 前缀：无（只读）。

local M = {}

local config = require("src.config")
local props = require("src.proposals")
local reqs = require("src.requirements")
local util = require("src.util")

local TEXTS

local num, text_of =
  config.num, config.text_of
local prop_pending_total =
  props.prop_pending_total
local R_ACCEPT, R_PLAN_GATE, job_capacity, job_total, req_all_done, req_conflict_total, req_count, req_id, req_key, req_open_total, req_rev, req_status =
  reqs.R_ACCEPT, reqs.R_PLAN_GATE, reqs.job_capacity, reqs.job_total, reqs.req_all_done, reqs.req_conflict_total, reqs.req_count, reqs.req_id, reqs.req_key, reqs.req_open_total, reqs.req_rev, reqs.req_status
local fill =
  util.fill

local GATE_STATES = { [R_PLAN_GATE] = "plan", [R_ACCEPT] = "accept" }

local function gate_kind(index)
  return GATE_STATES[req_status(index)]
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
  return num(req_key(index, "gate_rev"), 0) == req_rev(index)
end

local function gate_problem()
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

function M.install(deps)
  TEXTS = deps.texts
end

M.GATE_STATES = GATE_STATES
M.gate_evidence_problem = gate_evidence_problem
M.gate_kind = gate_kind
M.gate_next = gate_next
M.gate_problem = gate_problem
M.gate_rev_ok = gate_rev_ok
M.gate_total = gate_total
M.req_artifacts = req_artifacts

return M
