-- src/handlers/requirements/requirement_ops.lua —— 需求图命令：split|merge|amend|conflict|resolve。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 注册面全部在 install() 内（派发序 = 入口 install 调用序）。

local M = {}

local config = require("src.core.config")
local flow = require("src.runtime.flow")
local props = require("src.domain.proposals")
local render_mod = require("src.view.render")
local reqs = require("src.domain.requirements")
local util = require("src.core.util")
local wire = require("src.domain.wire")

local TEXTS

local text_of =
  config.text_of
local refuse, reject, reject_payload =
  flow.refuse, flow.reject, flow.reject_payload
local req_do_conflict, req_do_merge, req_do_split =
  props.req_do_conflict, props.req_do_merge, props.req_do_split
local render =
  render_mod.receipt
local PHASE, R_DONE, R_OPEN, STAGE, req_append, req_bump_rev, req_capacity, req_closed, req_closed_reason, req_conflict, req_conflict_peer, req_count, req_deps, req_deps_text, req_gate, req_has_pending, req_id, req_index_of, req_job, req_key, req_lane, req_open_total, req_phase, req_push_history, req_release, req_status, req_sync_conflicts, req_title, req_validate_deps =
  reqs.PHASE, reqs.R_DONE, reqs.R_OPEN, reqs.STAGE, reqs.req_append, reqs.req_bump_rev, reqs.req_capacity, reqs.req_closed, reqs.req_closed_reason, reqs.req_conflict, reqs.req_conflict_peer, reqs.req_count, reqs.req_deps, reqs.req_deps_text, reqs.req_gate, reqs.req_has_pending, reqs.req_id, reqs.req_index_of, reqs.req_job, reqs.req_key, reqs.req_lane, reqs.req_open_total, reqs.req_phase, reqs.req_push_history, reqs.req_release, reqs.req_status, reqs.req_sync_conflicts, reqs.req_title, reqs.req_validate_deps
local fill, join =
  util.fill, util.join
local inbox_mark, inbox_replay, parse_amend, parse_conflict, parse_deps, parse_merge, parse_resolve, parse_split, strip_ext =
  wire.inbox_mark, wire.inbox_replay, wire.parse_amend, wire.parse_conflict, wire.parse_deps, wire.parse_merge, wire.parse_resolve, wire.parse_split, wire.strip_ext

-- 退役局全拒（§2.7 live·retired：全部业务事件拒绝——需求图命令也不例外）。
local function run_retired(action)
  if STAGE.retired ~= reqs.stage() then
    return false
  end
  return reject(action, TEXTS.beats.req_after_done)
end

-- 需求图可变判据（微状态口径，§2.4 末段）：split / merge 仅对**开发前**且无活动任务、
-- 门禁、泳道、冲突的需求有效。结构化判据 = open ∧ 无门禁 ∧ 无 pending ∧ 无 job ∧
-- 无泳道 ∧ 不在集成——恰好覆盖立项/计划中/待开发三个微状态（计划门禁被 gate 项排除，
-- 交付受理被 pending 项排除，集成被 integrating 项排除）。
local function req_graph_mutable(index)
  if req_status(index) ~= R_OPEN then
    return false
  end
  if req_gate(index) ~= "" then
    return false
  end
  if req_has_pending(index) then
    return false
  end
  if req_job(index) ~= "" or req_lane(index) > 0 then
    return false
  end
  if ygg.kv.get(req_key(index, "integrating")) == true then
    return false
  end
  return true
end

-- 旧的 ready 语义 → 微状态口径：待开发 = plan_ok 已批 ∧ 槽位空闲。
local function req_plan_ready(index)
  return req_phase(index) == PHASE.ready
end



function M.install(deps)
  TEXTS = deps.texts
  ygg.on("requirement.split", function(payload)
    if run_retired("requirement.split") then
      return
    end
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
    if not req_graph_mutable(parent) then
      return refuse(TEXTS.beats.req_wait_rejected, "requirement.split", fill(TEXTS.beats.req_split_status, {
        id = req_id(parent),
        status = req_phase(parent),
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
    if run_retired("requirement.merge") then
      return
    end
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
      if not req_graph_mutable(index) then
        return refuse(TEXTS.beats.req_wait_rejected, "requirement.merge", fill(TEXTS.beats.req_merge_status, {
          id = req_id(index),
          status = req_phase(index),
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
    if run_retired("requirement.amend") then
      return
    end
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
    if status ~= R_OPEN then
      -- R11：旁路关闭（split/merged/dropped）不可修订；done（正常交付）可派生新需求。
      if req_closed_reason(index) ~= "" then
        return refuse(TEXTS.beats.req_wait_rejected, "requirement.amend", fill(TEXTS.beats.req_amend_closed, {
          id = req_id(index),
          status = req_closed_reason(index),
        }))
      end
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
    -- 在途开发流（开发中/交付受理/验收门禁/集成）：只许改标题描述，不许换依赖。
    local phase = req_phase(index)
    if phase == PHASE.developing or phase == PHASE.dev_review
      or phase == PHASE.accept_gate or phase == PHASE.integrating then
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
    if run_retired("requirement.conflict") then
      return
    end
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
          status = req_closed_reason(index) ~= "" and req_closed_reason(index) or PHASE.done,
        }))
      end
      -- 在途开发流（开发中/交付受理/验收门禁/集成）不可升冲突。
      local phase = req_phase(index)
      if phase == PHASE.developing or phase == PHASE.dev_review
        or phase == PHASE.accept_gate or phase == PHASE.integrating then
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
    if run_retired("requirement.resolve") then
      return
    end
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
    if mode == "keep_first" and req_phase(second) ~= PHASE.draft and not req_plan_ready(second) then
      return refuse(TEXTS.beats.req_wait_rejected, "requirement.resolve", fill(TEXTS.beats.req_resolve_drop_busy, {
        id = req_id(second),
        status = req_phase(second),
      }))
    end
    if mode == "revise" then
      for _, index in ipairs({ first, second }) do
        if req_phase(index) ~= PHASE.draft and not req_plan_ready(index) then
          return refuse(TEXTS.beats.req_wait_rejected, "requirement.resolve", fill(TEXTS.beats.req_resolve_revise_busy, {
            id = req_id(index),
            status = req_phase(index),
          }))
        end
      end
    end
    ygg.kv.set(req_key(first, "conflict"), "")
    ygg.kv.set(req_key(second, "conflict"), "")
    ygg.kv.set(req_key(first, "conflict_peer"), "")
    ygg.kv.set(req_key(second, "conflict_peer"), "")
    ygg.kv.set(req_key(first, "conflict_note"), "")
    ygg.kv.set(req_key(second, "conflict_note"), "")
    ygg.kv.set(req_key(first, "conflict_source"), "")
    ygg.kv.set(req_key(second, "conflict_source"), "")
    local effect
    if mode == "keep_first" then
      -- R11：dropped 是关闭原因（closed_reason），不再是状态；status 同步写 done。
      ygg.kv.set(req_key(second, "status"), R_DONE)
      ygg.kv.set(req_key(second, "closed_reason"), reqs.CLOSED_DROPPED)
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
end


return M
