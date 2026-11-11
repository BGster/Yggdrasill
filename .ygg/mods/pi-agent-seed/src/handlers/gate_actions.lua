-- src/handlers/gate_actions.lua —— 两个强制门禁：gate.approve|reject。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 注册面全部在 install() 内（派发序 = 入口 install 调用序）。

local M = {}

local config = require("src.config")
local flow = require("src.flow")
local gates = require("src.gates")
local hooks = require("src.hooks")
local choice = require("src.handlers.choice")
local render_mod = require("src.render")
local reqs = require("src.requirements")
local tools = require("src.tools")
local util = require("src.util")

local TEXTS

local num, text_of =
  config.num, config.text_of
local refuse, reject, reject_payload =
  flow.refuse, flow.reject, flow.reject_payload
local gate_evidence_problem, gate_kind, gate_next, gate_problem_for, gate_rev_ok, gate_total, req_artifacts =
  gates.gate_evidence_problem, gates.gate_kind, gates.gate_next, gates.gate_problem_for, gates.gate_rev_ok, gates.gate_total, gates.req_artifacts
local HUMAN =
  choice.HUMAN
local render =
  render_mod.receipt
local R_DONE, R_PLANNING, STAGE, req_id, req_index_of, req_key, req_lane, req_phase_name, req_plan_ok, req_push_history, req_release, req_rev, req_sync_conflicts, stage =
  reqs.R_DONE, reqs.R_PLANNING, reqs.STAGE, reqs.req_id, reqs.req_index_of, reqs.req_key, reqs.req_lane, reqs.req_phase_name, reqs.req_plan_ok, reqs.req_push_history, reqs.req_release, reqs.req_rev, reqs.req_sync_conflicts, reqs.stage
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
    -- 单需求门禁判据（批准即开工的前半）：rN 的门禁只被 rN 自身事态阻塞——其它需求的
    -- pending 提案 / 在跑 job / 冲突不进本判定；run 收口链（coordinate.submit / verify /
    -- review）的 gate_problem 全局屏障保持不动。gate.reject 不加判据（纠错通道保持畅通）。
    local problem = gate_problem_for(index)
    if problem ~= nil then
      return refuse(TEXTS.beats.req_wait_rejected, "gate.approve", problem)
    end
    local next_status = kind == "plan" and R_PLANNING or R_DONE
    ygg.kv.set(req_key(index, "status"), next_status)
    -- 门禁摘下：plan 批准 → plan_ok 置真（旧的 ready）；accept 批准 → done。
    ygg.kv.set(req_key(index, "gate"), "")
    if kind == "plan" then
      ygg.kv.set(req_key(index, "plan_ok"), true)
    else
      local lane_idx = req_lane(index)
      if lane_idx <= 0 then
        lane_idx = num(req_key(index, "last_lane"), 0)
      end
      if lane_idx > 0 then
        tools.auto_release_worktree(lane_idx)
        ygg.kv.set(req_key(index, "last_lane"), 0)
      end
    end
    req_release(index)
    req_push_history(index, fill(TEXTS.beats.req_gate_hist, { kind = kind }))
    req_sync_conflicts()
    ygg.narrate(fill(TEXTS.beats.req_gate_ok, {
      id = req_id(index),
      kind = kind,
      status = req_phase_name(index),
      note = trim(note or "") ~= "" and trim(note) or TEXTS.beats.req_gate_no_note,
      artifacts = req_artifacts(index) ~= "" and req_artifacts(index) or TEXTS.beats.req_gate_no_artifacts,
      gates = tostring(gate_total()),
    }))
    -- 唯一后继自动推进走响应式：plan 批准 = ready，开发槽位已打开——打标后由 render() 入口的
    -- hooks.flush_effects 在产出 choice 之前同步完成单需求派活（省一回合）。
    if kind == "plan" then
      hooks.mark_auto_dispatch(index)
    end
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
    -- 打回落点（两侧共用一条：存储态一律回 planning，展示名由 plan_ok 派生）：
    --   plan 打回   → plan_ok 置假 —— 计划没过门禁，必须重派 plan；
    --   accept 打回 → plan_ok 沿用 keep —— 未 amend 即旧 ready（重进开发队列，
    --                 再次 dispatch 派的是 development）；amend 过则 req_bump_rev 已把
    --                 plan_ok 置假，keep 为假 → 停在 planning 强制重规划。
    -- keep 必须在写 status **之前**取：legacy 行（本模型之前创建，plan_ok 字段缺席）
    -- 靠 req_plan_ok 按「当前状态」推断——developing / done 视为已过 plan 门禁；
    -- 先把 status 写成 planning 再推断只会拿到假值，把在途的开发需求误打回 planning。
    local keep = req_plan_ok(index)
    ygg.kv.set(req_key(index, "status"), R_PLANNING)
    ygg.kv.set(req_key(index, "gate"), "")
    if kind == "plan" then
      ygg.kv.set(req_key(index, "plan_ok"), false)
    else
      ygg.kv.set(req_key(index, "plan_ok"), keep)
    end
    req_release(index)
    -- 打回不改 rev（执行方下一轮提案仍用当前 rev），但记录反馈供下轮参考。
    ygg.kv.set(req_key(index, "feedback"), trim(feedback or "") ~= "" and trim(feedback) or TEXTS.beats.req_reject_default)
    req_push_history(index, fill(TEXTS.beats.req_gate_reject_hist, { kind = kind }))
    req_sync_conflicts()
    ygg.narrate(fill(TEXTS.beats.req_gate_rejected, {
      id = req_id(index),
      kind = kind,
      status = req_phase_name(index),
      feedback = text_of(req_key(index, "feedback"), TEXTS.beats.req_reject_default),
      gates = tostring(gate_total()),
    }))
    render()
  end)
end

M.pick_gate = pick_gate

return M
