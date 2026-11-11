-- src/domain/executor.lua —— 执行者账本的单点写入（`settings.executor.<rN>.<stage>`）。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 资源由入口 install(deps) 注入。本模块写的 KV 前缀：`settings.executor.*`（唯一写方）。
-- 两个入口共用同一份校验：`config.set executor.<rN>.<stage>` 与 `proposal.submit` 末段 `exec=`。
-- 自声明：只校形状与「换人必给原因」，不验证 id 真伪。

local M = {}

local config = require("src.core.config")
local reqs = require("src.domain.requirements")
local util = require("src.core.util")
local wire = require("src.domain.wire")

local TEXTS

local executor_raw =
  config.executor_raw
local req_id, req_index_of, req_push_history =
  reqs.req_id, reqs.req_index_of, reqs.req_push_history
local fill, join, trim =
  util.fill, util.join, util.trim

-- 失败 = 可读拒绝描述（由调用方决定用哪种拒绝口径渲染）：
--   kind = "payload" → `flow.reject_payload`（载荷形状不对）
--   kind = "blocked" → `flow.refuse(beats.req_wait_rejected, …)`（前件未满足）
-- 成功 = nil（叙事已在此输出；调用方负责 render）。
local function failure(kind, detail)
  return { kind = kind, detail = detail }
end

local function bind(key, value)
  local req_text, stage_name = wire.parse_executor_key(key)
  if req_text == nil or not wire.executor_stage_ok(stage_name) then
    return failure("payload", fill(TEXTS.beats.executor_bad_key, {
      key = tostring(key),
      stages = join(wire.EXECUTOR_STAGES, " / "),
    }))
  end
  local index = req_index_of(req_text)
  if index == nil then
    return failure("payload", fill(TEXTS.beats.executor_unknown_req, { id = req_text }))
  end
  local agent, reason, id_problem = wire.parse_executor_value(value)
  if id_problem ~= nil then
    return failure("payload", id_problem)
  end
  local slot = "settings.executor." .. req_text .. "." .. stage_name
  local current = wire.executor_id_of(executor_raw(req_text, stage_name))
  if agent == "" then
    ygg.kv.set(slot, "")
    ygg.narrate(fill(TEXTS.beats.executor_cleared, { id = req_id(index), stage = stage_name }))
    return nil
  end
  if current == agent then
    ygg.narrate(fill(TEXTS.beats.executor_same, {
      id = req_id(index),
      stage = stage_name,
      agent = agent,
    }))
    return nil
  end
  -- 跨阶段：plan 看 analysis；development 看 plan（plan 缺失时回看 analysis）。
  local prev_stage = nil
  if stage_name == "plan" then
    prev_stage = "analysis"
  elseif stage_name == "development" then
    prev_stage = "plan"
  end
  local prev = ""
  if prev_stage ~= nil then
    prev = wire.executor_id_of(executor_raw(req_text, prev_stage))
    if prev == "" and stage_name == "development" then
      prev = wire.executor_id_of(executor_raw(req_text, "analysis"))
    end
  end
  local handoff_from = current ~= "" and current or prev
  local needs_reason = handoff_from ~= "" and handoff_from ~= agent
  if needs_reason and reason == "" then
    return failure("blocked", fill(TEXTS.beats.executor_reason_required, {
      id = req_id(index),
      stage = stage_name,
      prev = handoff_from,
      agent = agent,
    }))
  end
  ygg.kv.set(slot, wire.format_executor(agent, reason))
  -- 只有「真的换了人」才留痕；跨阶段同 id 是正常连续（首次绑定该 stage），不写 hist。
  if handoff_from ~= "" and handoff_from ~= agent then
    req_push_history(index, fill(TEXTS.beats.executor_hist, {
      stage = stage_name,
      from = handoff_from,
      to = agent,
      reason = trim(reason) ~= "" and trim(reason) or TEXTS.beats.executor_no_reason,
    }))
  end
  ygg.narrate(fill(TEXTS.beats.executor_bound, {
    id = req_id(index),
    stage = stage_name,
    agent = agent,
  }))
  return nil
end

function M.install(deps)
  TEXTS = deps.texts
end

M.bind = bind

return M
