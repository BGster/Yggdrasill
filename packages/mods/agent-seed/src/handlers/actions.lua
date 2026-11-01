-- src/handlers/actions.lua —— action 入口：task.intake / requirement.add|list|dispatch。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 注册面全部在 install() 内（派发序 = 入口 install 调用序）。

local M = {}

local flow = require("src.flow")
local lanes = require("src.lanes")
local menus = require("src.menus")
local render_mod = require("src.render")
local reqs = require("src.requirements")
local util = require("src.util")
local wire = require("src.wire")

local TEXTS

local advance, reject, reject_payload =
  flow.advance, flow.reject, flow.reject_payload
local LANES, free_lanes, lane_text =
  lanes.LANES, lanes.free_lanes, lanes.lane_text
local dispatch_candidates =
  menus.dispatch_candidates
local narrate_req_list, render, req_panel_text =
  render_mod.narrate_req_list, render_mod.receipt, render_mod.req_panel_text
local R_ANALYZING, R_DEVELOPING, STAGE, job_capacity, job_total, req_append, req_capacity, req_count, req_deps_text, req_dev_blocked_text, req_dev_ready, req_display, req_id, req_job_stage, req_key, req_open_total, req_validate_deps, stage =
  reqs.R_ANALYZING, reqs.R_DEVELOPING, reqs.STAGE, reqs.job_capacity, reqs.job_total, reqs.req_append, reqs.req_capacity, reqs.req_count, reqs.req_deps_text, reqs.req_dev_blocked_text, reqs.req_dev_ready, reqs.req_display, reqs.req_id, reqs.req_job_stage, reqs.req_key, reqs.req_open_total, reqs.req_validate_deps, reqs.stage
local fill, join =
  util.fill, util.join
local parse_intake, parse_requirement, probe_payload_ok =
  wire.parse_intake, wire.parse_requirement, wire.probe_payload_ok



function M.install(deps)
  TEXTS = deps.texts
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
    if not probe_payload_ok(payload) then
      return reject_payload("requirement.dispatch", fill(TEXTS.beats.probe_rejected, {
        action = "requirement.dispatch",
        got = tostring(payload),
        usage = TEXTS.beats.req_dispatch_usage,
      }))
    end
    -- 「按就绪序派活」是幂等语义：**没东西可派是正常结果**（需求都在 job
    -- 中 / 在门禁 / 在待验收 / 已完成，或 job 槽位 / 泳道已满）——自动化的
    -- 连续 dispatch 不得因此记失败步（伪失败会把正常等待当成故障）。
    -- 只留两个真拒绝：阶段已收口（done）与载荷非法。
    local candidates = dispatch_candidates()
    if #candidates == 0 then
      ygg.narrate(fill(TEXTS.beats.req_dispatch_noop_none, {
        jobs = tostring(job_total()),
        cap = tostring(job_capacity()),
      }))
      return render()
    end
    local lanes = free_lanes()
    local filled = {}
    for i = 1, #candidates do
      if job_total() < job_capacity() then
        local index = candidates[i]
        local stage_name = req_job_stage(index)
        local lane = 0
        local claimed = 0
        local ok = true
        if stage_name == "development" then
          -- 主 Agent 已用 lane.claim 预挂牌 → 直接沿用那条（不重复分配、不覆盖已记的
          -- branch/base/clean）；否则取第一条空闲泳道，同事务挂牌（branch/base/clean 留空 = 未断言）。
          claimed = req_dev_ready(index) and LANES.bound(index) or 0
          if not req_dev_ready(index) then
            ok = false
          elseif claimed > 0 then
            lane = claimed
          elseif #lanes > 0 then
            lane = lanes[1]
            local kept = {}
            for j = 2, #lanes do
              kept[#kept + 1] = lanes[j]
            end
            lanes = kept
          else
            ok = false
          end
        end
        if ok then
          ygg.kv.set(req_key(index, "job"), stage_name)
          if stage_name == "analysis" then
            ygg.kv.set(req_key(index, "status"), R_ANALYZING)
          elseif stage_name == "development" then
            ygg.kv.set(req_key(index, "status"), R_DEVELOPING)
            if claimed == 0 then
              LANES.bind(lane, index)
            end
          end
          filled[#filled + 1] = fill(TEXTS.beats.req_dispatch_item, {
            id = req_id(index),
            stage = stage_name,
            lane = lane > 0 and ("lane-" .. tostring(lane)) or "-",
          })
        end
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
      items = join(filled, "；"),
      count = tostring(#filled),
      jobs = tostring(job_total()),
      cap = tostring(job_capacity()),
    }))
    render()
  end)
end


return M
