-- src/handlers/actions.lua —— action 入口：task.intake / requirement.add|list|dispatch。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 注册面全部在 install() 内（派发序 = 入口 install 调用序）。

local M = {}

local flow = require("src.flow")
local gates = require("src.gates")
local hooks = require("src.hooks")
local lanes = require("src.lanes")
local menus = require("src.menus")
local props = require("src.proposals")
local render_mod = require("src.render")
local reqs = require("src.requirements")
local util = require("src.util")
local wire = require("src.wire")

local TEXTS

local advance, reject, reject_payload =
  flow.advance, flow.reject, flow.reject_payload
local LANES, free_lanes, lane_text =
  lanes.LANES, lanes.free_lanes, lanes.lane_text
local gate_total =
  gates.gate_total
local prop_pending_total =
  props.prop_pending_total
local dispatch_candidates =
  menus.dispatch_candidates
local narrate_req_list, render, req_panel_text =
  render_mod.narrate_req_list, render_mod.receipt, render_mod.req_panel_text
local R_DEVELOPING, R_PLANNING, STAGE, job_capacity, job_total, req_append, req_capacity, req_conflict_total, req_count, req_deps_text, req_dev_blocked_text, req_dev_ready, req_display, req_id, req_index_of, req_job, req_job_stage, req_key, req_open_total, req_validate_deps, stage =
  reqs.R_DEVELOPING, reqs.R_PLANNING, reqs.STAGE, reqs.job_capacity, reqs.job_total, reqs.req_append, reqs.req_capacity, reqs.req_conflict_total, reqs.req_count, reqs.req_deps_text, reqs.req_dev_blocked_text, reqs.req_dev_ready, reqs.req_display, reqs.req_id, reqs.req_index_of, reqs.req_job, reqs.req_job_stage, reqs.req_key, reqs.req_open_total, reqs.req_validate_deps, reqs.stage
local contains, fill, join =
  util.contains, util.fill, util.join
local parse_dispatch, parse_intake, parse_requirement, probe_payload_ok =
  wire.parse_dispatch, wire.parse_intake, wire.parse_requirement, wire.probe_payload_ok



-- 单需求派活（handler 的批量循环与 auto_dispatch effect 共用）：成功返回 true, stage, lane。
-- 「按就绪序派活」是幂等语义：不可派就返回 false（不报错）——job 槽满 / 泳道满 / 未就绪
-- 都视为正常等待，不得当成失败步。
local function dispatch_one(index)
  local stage_name = req_job_stage(index)
  if stage_name == nil or req_job(index) ~= "" then
    return false
  end
  if job_total() >= job_capacity() then
    return false
  end
  local lane, claimed = 0, 0
  if stage_name == "development" then
    -- 主 Agent 已用 lane.claim 预挂牌 → 直接沿用那条（不重复分配、不覆盖已记的
    -- branch/base/clean）；否则取第一条空闲泳道，同事务挂牌（branch/base/clean 留空 = 未断言）。
    if not req_dev_ready(index) then
      return false
    end
    claimed = LANES.bound(index)
    if claimed > 0 then
      lane = claimed
    else
      local free = free_lanes()
      if #free == 0 then
        return false
      end
      lane = free[1]
    end
  end
  ygg.kv.set(req_key(index, "job"), stage_name)
  if stage_name == "plan" then
    -- draft 直接派 plan（analysis + plan 合并）：派活就把存储态推进到 planning，
    -- 不再落 analyzing（R_ANALYZING 仅保留给 legacy 存档与显式 analysis 提案）。
    ygg.kv.set(req_key(index, "status"), R_PLANNING)
  elseif stage_name == "development" then
    ygg.kv.set(req_key(index, "status"), R_DEVELOPING)
    if claimed == 0 then
      LANES.bind(lane, index)
    end
  end
  return true, stage_name, lane
end

-- 响应式：唯一后继自动派活（声明式注册，不在各 handler 末尾手写 emit）。
-- 触发面 = 业务 handler 打的「刚就绪」标记：proposal.accept(analysis) → plan 槽打开；
-- gate.approve(plan) → 开发槽打开。判据（写进 AGENTS.md，可判定）：同轮无待验收提案、
-- 无待批门禁、无未解决冲突、job 槽未满；且目标需求确实可派（dev 还需 plan_ok / 无冲突 /
-- 依赖就绪 + 有空闲泳道）。effect 只推进状态与叙事，绝不 render / ygg.choice ——
-- 连锁反应在 render() 入口的 flush_effects 里收敛，唯一的 choice 由 render() 产出。
local AUTO_DISPATCH = "auto_dispatch"

local function auto_dispatch_effect()
  local changed = false
  local items = {}
  for i = 1, req_count() do
    if hooks.has_mark(i) then
      local ok, stage_name, lane = dispatch_one(i)
      if ok then
        items[#items + 1] = fill(TEXTS.beats.req_dispatch_item, {
          id = req_id(i),
          stage = stage_name,
          lane = lane > 0 and ("lane-" .. tostring(lane)) or "-",
        })
        changed = true
      end
    end
  end
  if changed then
    ygg.narrate(fill(TEXTS.beats.req_dispatch_auto, {
      items = join(items, "; "),
      count = tostring(#items),
      jobs = tostring(job_total()),
      cap = tostring(job_capacity()),
    }))
  end
  return changed
end

function M.install(deps)
  TEXTS = deps.texts
  -- 装配期注册（mod.lua 调用 install 时一次）：运行期由 render() 入口的 flush_effects 驱动。
  hooks.useEffect(AUTO_DISPATCH, function()
    if stage() ~= STAGE.coordinate then
      return nil
    end
    if prop_pending_total() > 0 or gate_total() > 0 or req_conflict_total() > 0 then
      return nil
    end
    if job_total() >= job_capacity() then
      return nil
    end
    local deps_sig = { job_total(), job_capacity() }
    local any = false
    for i = 1, req_count() do
      if hooks.has_mark(i) then
        deps_sig[#deps_sig + 1] = i
        deps_sig[#deps_sig + 1] = tostring(req_job_stage(i))
        deps_sig[#deps_sig + 1] = req_job(i)
        any = true
      end
    end
    if not any then
      return nil
    end
    return deps_sig
  end, auto_dispatch_effect)
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
    local spec_target, dispatch_problem = parse_dispatch(payload)
    if dispatch_problem ~= nil then
      return reject_payload("requirement.dispatch", dispatch_problem)
    end
    -- 「按就绪序派活」是幂等语义：**没东西可派是正常结果**（需求都在 job
    -- 中 / 在门禁 / 在待验收 / 已完成，或 job 槽位 / 泳道已满）——自动化的
    -- 连续 dispatch 不得因此记失败步（伪失败会把正常等待当成故障）。
    -- 只留两个真拒绝：阶段已收口（done）与载荷非法。
    local candidates = dispatch_candidates()
    -- `rN` 载荷只派这一条（声明式派生事件的载荷）：其余候选派活是副作用，不是「唯一后继」。
    if spec_target ~= nil then
      local wanted = req_index_of(spec_target)
      if wanted ~= nil and contains(candidates, wanted) then
        candidates = { wanted }
      else
        candidates = {}
      end
    end
    if #candidates == 0 then
      ygg.narrate(fill(TEXTS.beats.req_dispatch_noop_none, {
        jobs = tostring(job_total()),
        cap = tostring(job_capacity()),
      }))
      return render()
    end
    local filled = {}
    for i = 1, #candidates do
      local index = candidates[i]
      local ok, stage_name, lane = dispatch_one(index)
      if ok then
        filled[#filled + 1] = fill(TEXTS.beats.req_dispatch_item, {
          id = req_id(index),
          stage = stage_name,
          lane = lane > 0 and ("lane-" .. tostring(lane)) or "-",
        })
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
      items = join(filled, "; "),
      count = tostring(#filled),
      jobs = tostring(job_total()),
      cap = tostring(job_capacity()),
    }))
    render()
  end)
end


return M
