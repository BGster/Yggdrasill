-- src/handlers/gate_actions.lua —— 两个强制门禁：gate.approve|reject。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 注册面全部在 install() 内（派发序 = 入口 install 调用序）。

local M = {}

local config = require("src.config")
local flow = require("src.flow")
local gates = require("src.gates")
local choice = require("src.handlers.choice")
local render_mod = require("src.render")
local reqs = require("src.requirements")
local util = require("src.util")

local TEXTS

local num, text_of =
  config.num, config.text_of
local refuse, reject, reject_payload =
  flow.refuse, flow.reject, flow.reject_payload
local gate_evidence_problem, gate_kind, gate_next, gate_rev_ok, gate_total, req_artifacts =
  gates.gate_evidence_problem, gates.gate_kind, gates.gate_next, gates.gate_rev_ok, gates.gate_total, gates.req_artifacts
local HUMAN =
  choice.HUMAN
local render =
  render_mod.receipt
local R_DONE, R_PLANNING, R_READY, STAGE, req_id, req_index_of, req_key, req_push_history, req_release, req_rev, req_sync_conflicts, stage =
  reqs.R_DONE, reqs.R_PLANNING, reqs.R_READY, reqs.STAGE, reqs.req_id, reqs.req_index_of, reqs.req_key, reqs.req_push_history, reqs.req_release, reqs.req_rev, reqs.req_sync_conflicts, reqs.stage
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

function M.install(deps)
  TEXTS = deps.texts
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
end

M.pick_gate = pick_gate

return M
