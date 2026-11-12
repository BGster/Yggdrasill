-- src/handlers/requirements/integration.lua —— 集成幕两事件：integration.confirm / integration.reject。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 注册面全部在 install() 内（派发序 = 入口 install 调用序）。
-- R5：两事件都不进票据门（由执行者发——执行者结构上无菜单）。
-- R7：泳道绑定窗口保持到集成 confirm/reject 才释放（交付受理→验收门禁→集成三幕不释放）。

local M = {}

local flow = require("src.runtime.flow")
local render_mod = require("src.view.render")
local reqs = require("src.domain.requirements")
local tools = require("src.runtime.tools")
local util = require("src.core.util")

local TEXTS

local phase_guard, reject_payload =
  flow.phase_guard, flow.reject_payload
local render =
  render_mod.receipt
local release_lane_with_worktree =
  tools.release_lane_with_worktree
local RUN_PHASE, req_id, req_index_of, req_key, req_phase, req_push_history, req_sync_conflicts =
  reqs.RUN_PHASE, reqs.req_id, reqs.req_index_of, reqs.req_key, reqs.req_phase, reqs.req_push_history, reqs.req_sync_conflicts
local cut, fill, trim =
  util.cut, util.fill, util.trim

-- 集成证据形状：`develop@<sha>`，sha 为 7-40 位十六进制（短 sha 与全 sha 都合法）。
local function develop_sha_ok(evidence)
  if type(evidence) ~= "string" then
    return false
  end
  local sha = trim(evidence)
  if sha:sub(1, 1) == "@" then
    return false
  end
  return #sha >= 7 and #sha <= 40 and sha:match("^%x+$") ~= nil
end

-- 解析 `<rN>|<evidence>`：返回 index, rest（evidence 或 reason）；形状错返回 nil, problem。
local function parse_integration(payload)
  if type(payload) ~= "string" then
    return nil, fill(TEXTS.beats.bad_payload, { action = "integration" })
  end
  local target, rest = cut(trim(payload), "|")
  local index = req_index_of(trim(target or ""))
  if index == nil then
    return nil, fill(TEXTS.beats.req_unknown, { id = tostring(target) })
  end
  return index, trim(rest or "")
end

function M.install(deps)
  TEXTS = deps.texts
  -- 集成幕只认 integration.confirm / integration.reject（§2.7：进入集成的唯一入口是
  -- gate.approve(accept)；confirm 校验 develop@<sha> 后落 done 终态）。
  ygg.on("integration.confirm", function(payload)
    if not phase_guard("integration.confirm", { RUN_PHASE.coordinate }) then
      return
    end
    local index, evidence = parse_integration(payload)
    if index == nil then
      return reject_payload("integration.confirm", evidence)
    end
    if req_phase(index) ~= reqs.PHASE.integrating then
      return reject_payload("integration.confirm", fill(TEXTS.beats.integration_not_integrating, {
        id = req_id(index),
        status = req_phase(index),
      }))
    end
    local sha = evidence
    if sha:sub(1, 8) == "develop@" then
      sha = trim(sha:sub(9))
    end
    if not develop_sha_ok(sha) then
      return reject_payload("integration.confirm", fill(TEXTS.beats.integration_bad_sha, {
        id = req_id(index),
        got = evidence,
      }))
    end
    ygg.kv.set(req_key(index, "develop_sha"), sha)
    ygg.kv.set(req_key(index, "integrating"), false)
    -- done = 需求级真状态（第 9 幕）；closed_reason 留空 = 正常交付闭环。
    ygg.kv.set(req_key(index, "status"), reqs.R_DONE)
    release_lane_with_worktree(index)
    req_push_history(index, fill(TEXTS.beats.integration_hist, { id = req_id(index), sha = sha }))
    req_sync_conflicts()
    ygg.narrate(fill(TEXTS.beats.integration_confirmed, {
      id = req_id(index),
      sha = sha,
      done = tostring(reqs.req_tally(reqs.PHASE.done) + reqs.req_tally(reqs.PHASE.closed)),
    }))
    render()
  end)
  -- 集成打回（R6 三条打回之一）：统一落待开发——plan_ok 保持、释放泳道、写 feedback。
  ygg.on("integration.reject", function(payload)
    if not phase_guard("integration.reject", { RUN_PHASE.coordinate }) then
      return
    end
    local index, reason = parse_integration(payload)
    if index == nil then
      return reject_payload("integration.reject", reason)
    end
    if req_phase(index) ~= reqs.PHASE.integrating then
      return reject_payload("integration.reject", fill(TEXTS.beats.integration_not_integrating, {
        id = req_id(index),
        status = req_phase(index),
      }))
    end
    ygg.kv.set(req_key(index, "integrating"), false)
    local note = trim(reason) ~= "" and trim(reason) or TEXTS.beats.req_reject_default
    ygg.kv.set(req_key(index, "feedback"), note)
    release_lane_with_worktree(index)
    req_push_history(index, fill(TEXTS.beats.integration_reject_hist, { id = req_id(index) }))
    req_sync_conflicts()
    ygg.narrate(fill(TEXTS.beats.integration_rejected, {
      id = req_id(index),
      reason = note,
    }))
    render()
  end)
end

return M
