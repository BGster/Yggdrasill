-- src/handlers/requirements/proposals.lua —— 提案入口与串行验收：proposal.submit|accept|reject。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 注册面全部在 install() 内（派发序 = 入口 install 调用序）。
-- D-MA23：plan 提案**交付即门禁**（落库后 gate="plan"，裁决权在人）；
-- development 提案落交付受理（主代理串行复核幕）；泳道保持到集成才释放（R7）。

local M = {}

local config = require("src.core.config")
local executor = require("src.domain.executor")
local flow = require("src.runtime.flow")
local choice = require("src.handlers.operate.choice")
local props = require("src.domain.proposals")
local render_mod = require("src.view.render")
local reqs = require("src.domain.requirements")
local tools = require("src.runtime.tools")
local util = require("src.core.util")
local wire = require("src.domain.wire")

local TEXTS

local text_of =
  config.text_of
local phase_guard, refuse, reject_payload =
  flow.phase_guard, flow.refuse, flow.reject_payload
local HUMAN =
  choice.HUMAN
local P_ACCEPTED, P_PENDING, P_REJECTED, P_STALE, apply_changes, prop_append, prop_by_ext, prop_count, prop_id, prop_key, prop_next, prop_pending_total, prop_req, prop_rev, prop_stage, prop_status =
  props.P_ACCEPTED, props.P_PENDING, props.P_REJECTED, props.P_STALE, props.apply_changes, props.prop_append, props.prop_by_ext, props.prop_count, props.prop_id, props.prop_key, props.prop_next, props.prop_pending_total, props.prop_req, props.prop_rev, props.prop_stage, props.prop_status
local render =
  render_mod.receipt
local release_lane_with_worktree =
  tools.release_lane_with_worktree
local RUN_PHASE, req_gate, req_id, req_index_of, req_job, req_key, req_lane, req_phase, req_push_history, req_rev, req_sync_conflicts =
  reqs.RUN_PHASE, reqs.req_gate, reqs.req_id, reqs.req_index_of, reqs.req_job, reqs.req_key, reqs.req_lane, reqs.req_phase, reqs.req_push_history, reqs.req_rev, reqs.req_sync_conflicts
local cut, fill, trim =
  util.cut, util.fill, util.trim
local parse_proposal =
  wire.parse_proposal

-- 槽位判定（微状态口径，§2.4/§2.7）：plan 提案只收「计划中」（job=="plan"）；
-- development 提案只收「开发中」（job=="development"）。门禁挂起期间一律不收
-- （一次一门禁）；「重定基 vs 被占用」由 pending 提案表判（第二个 proposal.submit 拒）。
-- 返回 (ok, reason)：reason ∈ "gate" / "status"（ok 时为 nil）。
local function submit_slot_ok(index, stage_name)
  if req_gate(index) ~= "" then
    return false, "gate"
  end
  local job = req_job(index)
  if stage_name == "plan" and job == "plan" then
    return true, nil
  end
  if stage_name == "development" and job == "development" then
    -- 施工中 requirement.amend 只更新标题/描述（进度不重置），必须允许按新 rev 提案。
    return true, nil
  end
  return false, "status"
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
    if not phase_guard("proposal.submit", { RUN_PHASE.coordinate }) then
      return
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
    -- 槽位：微状态口径（计划中收 plan、开发中收 development；门禁挂起不收）。
    local slot_ok, slot_reason = submit_slot_ok(index, spec.stage)
    if not slot_ok then
      local detail
      if slot_reason == "gate" then
        detail = fill(TEXTS.beats.req_prop_stage_mismatch, {
          id = req_id(index),
          stage = spec.stage,
          status = req_phase(index),
        })
      else
        detail = fill(TEXTS.beats.req_prop_stage_mismatch, {
          id = req_id(index),
          stage = spec.stage,
          status = req_phase(index),
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
    -- 执行者登记并入提案提交（省一回合）：校验失败则整体不生效（提案不落库）。
    if spec.exec ~= nil then
      local problem = executor.bind(
        "executor." .. spec.req .. "." .. spec.stage,
        wire.format_executor(spec.exec, spec.exec_reason)
      )
      if problem ~= nil then
        if problem.kind == "blocked" then
          return refuse(TEXTS.beats.req_wait_rejected, "proposal.submit", problem.detail)
        end
        return reject_payload("proposal.submit", problem.detail)
      end
    end
    -- 重定基：旧提案已作废（stale / 被拒），清掉需求的 stale 标记（本次提案重新起算）。
    ygg.kv.set(req_key(index, "stale"), false)
    if spec.stage == "development" then
      local lane_idx = req_lane(index)
      if lane_idx > 0 then
        tools.auto_probe_lane(lane_idx)
      end
    end
    local summarize = spec.summary
    if summarize == "" then
      summarize = TEXTS.beats.req_prop_no_summary
    end
    local prop_index = prop_append(index, spec.rev, spec.stage, summarize, spec.artifacts, spec.ext, spec.changes, spec.host_actor, spec.host_actor_session)
    -- 提交者身份三态互证（r3）：wire 尾段 exec= 与宿主注入的 actor= 章比对。
    -- 识别不拒绝：mismatch 只是记录 + 叙事（裁决权在人 / 门禁），不拦验收；
    -- actor_session 是纯审计字段，不参与任何判定。
    if spec.exec ~= nil then
      if spec.host_actor ~= nil then
        if spec.host_actor == spec.exec then
          ygg.narrate(fill(TEXTS.beats.req_prop_actor_verified, { id = prop_id(prop_index), exec = spec.exec }))
        else
          ygg.narrate(fill(TEXTS.beats.req_prop_actor_mismatch, {
            id = prop_id(prop_index),
            exec = spec.exec,
            actor = spec.host_actor,
          }))
        end
      else
        ygg.narrate(fill(TEXTS.beats.req_prop_actor_unverified, { id = prop_id(prop_index), exec = spec.exec }))
      end
    end
    -- 同步核验（0ms，受 `tuning.artifact_capacity` 与 `MAX_ARTIFACT_CHECKS` 限）：
    -- 宿主没装 `ygg.fs` 时整体 no-op；命中则同步落 `artifact.*` 并叙事（不 render，本 handler 末尾统一 render）。
    if spec.artifacts ~= "" then
      tools.verify_artifacts(spec.artifacts)
    end
    -- 只释放 job 槽位；状态保持 open（真状态不因交付改变）。
    ygg.kv.set(req_key(index, "job"), "")
    if spec.stage == "plan" then
      -- D-MA23：plan 提案交付即门禁——落库后立刻升起 plan 门禁（裁决权在人），
      -- gate_rev 绑定 rev（待批期间 amend → gate_rev ≠ rev → stale，不得批准）。
      ygg.kv.set(req_key(index, "gate"), "plan")
      ygg.kv.set(req_key(index, "gate_rev"), spec.rev)
      ygg.narrate(fill(TEXTS.beats.req_prop_ok, {
        id = prop_id(prop_index),
        req = req_id(index),
        stage = spec.stage,
        rev = tostring(spec.rev),
        summary = summarize,
        pending = tostring(prop_pending_total()),
      }))
      ygg.narrate(fill(TEXTS.beats.req_prop_plan_gate, {
        id = req_id(index),
        rev = tostring(spec.rev),
      }))
    else
      -- development 提案落交付受理（pending 表达幕）；**不释放泳道**（R7：执行者
      -- 仍需该工作树完成 feature→develop 集成）。
      ygg.narrate(fill(TEXTS.beats.req_prop_ok, {
        id = prop_id(prop_index),
        req = req_id(index),
        stage = spec.stage,
        rev = tostring(spec.rev),
        summary = summarize,
        pending = tostring(prop_pending_total()),
      }))
    end
    render()
  end)
  ygg.on("proposal.accept", function(payload)
    if not HUMAN.gated("proposal.accept") then
      return
    end
    if not phase_guard("proposal.accept", { RUN_PHASE.coordinate }) then
      return
    end
    local prop_index = pick_prop(payload, true)
    if prop_index == nil then
      return reject_payload("proposal.accept", TEXTS.beats.req_prop_none)
    end
    local index = prop_req(prop_index)
    local stage_name = prop_stage(prop_index)
    if stage_name == "plan" then
      -- plan 提案的裁决走 gate.approve / gate.reject（提交即门禁）；受理不是它的出口。
      return refuse(TEXTS.beats.req_wait_rejected, "proposal.accept", fill(TEXTS.beats.req_accept_plan, {
        id = prop_id(prop_index),
        req = req_id(index),
      }))
    end
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
    -- 交付受理 → 验收门禁：复制 summary/artifacts、应用 changes、升起 accept 门禁。
    -- 泳道保持绑定（R7）；真状态不变（open）。
    ygg.kv.set(req_key(index, "summary"), text_of(prop_key(prop_index, "summary"), ""))
    ygg.kv.set(req_key(index, "artifacts"), text_of(prop_key(prop_index, "artifacts"), ""))
    ygg.kv.set(prop_key(prop_index, "status"), P_ACCEPTED)
    local change_note = apply_changes(index, text_of(prop_key(prop_index, "changes"), ""))
    ygg.kv.set(req_key(index, "gate"), "accept")
    ygg.kv.set(req_key(index, "gate_rev"), req_rev(index))
    req_push_history(index, fill(TEXTS.beats.req_accept_hist, { id = prop_id(prop_index), stage = stage_name }))
    req_sync_conflicts()
    ygg.narrate(fill(TEXTS.beats.req_accept_ok, {
      id = prop_id(prop_index),
      req = req_id(index),
      stage = stage_name,
      status = req_phase(index),
      change = change_note,
      pending = tostring(prop_pending_total()),
    }))
    render()
  end)
  ygg.on("proposal.reject", function(payload)
    if not phase_guard("proposal.reject", { RUN_PHASE.coordinate }) then
      return
    end
    local target, reason = cut(trim(payload), "|")
    local prop_index = pick_prop(target, true)
    if prop_index == nil then
      return reject_payload("proposal.reject", TEXTS.beats.req_prop_none)
    end
    local index = prop_req(prop_index)
    local stage_name = prop_stage(prop_index)
    ygg.kv.set(prop_key(prop_index, "status"), P_REJECTED)
    if stage_name == "plan" then
      -- plan 提案打回：清门禁回计划中（job 重新挂回 plan——计划任务回到执行者手上重做；
      -- plan_ok 本就为假，rev 不动）。
      ygg.kv.set(req_key(index, "gate"), "")
      ygg.kv.set(req_key(index, "gate_rev"), 0)
      ygg.kv.set(req_key(index, "job"), "plan")
    else
      -- development 提案打回（R6 三条打回之一）：统一落待开发——plan_ok 保持、
      -- 释放泳道（连工作树）；rev 不动（执行方下一轮提案仍用当前 rev）。
      release_lane_with_worktree(index)
    end
    req_push_history(index, fill(TEXTS.beats.req_reject_hist, { id = prop_id(prop_index) }))
    ygg.narrate(fill(TEXTS.beats.req_reject_ok, {
      id = prop_id(prop_index),
      req = req_id(index),
      status = req_phase(index),
      reason = trim(reason or "") ~= "" and trim(reason) or TEXTS.beats.req_reject_default,
    }))
    render()
  end)
end

M.pick_prop = pick_prop

return M
