-- src/handlers/operate/lanes.lua —— 泳道动作：lane.claim / lane.release / 迁移解析。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 注册面全部在 install() 内（派发序 = 入口 install 调用序）。

local M = {}

local config = require("src.core.config")
local flow = require("src.runtime.flow")
local lanes = require("src.domain.lanes")
local render_mod = require("src.view.render")
local reqs = require("src.domain.requirements")
local util = require("src.core.util")

local TEXTS

local num =
  config.num
local refuse, reject, reject_payload =
  flow.refuse, flow.reject, flow.reject_payload
local LANES =
  lanes.LANES
local render =
  render_mod.receipt
local STAGE, job_capacity, req_closed, req_id, req_index_of, req_lane, req_status, req_title, stage =
  reqs.STAGE, reqs.job_capacity, reqs.req_closed, reqs.req_id, reqs.req_index_of, reqs.req_lane, reqs.req_status, reqs.req_title, reqs.stage
local fill, split, trim =
  util.fill, util.split, util.trim

function LANES.parse_claim(payload)
  local max = job_capacity()
  if type(payload) ~= "string" then
    return nil, fill(TEXTS.beats.bad_payload, { action = "lane.claim" })
  end
  local parts = split(trim(payload), "|")
  local k = tonumber(trim(parts[1] or ""))
  local id = trim(parts[2] or "")
  if k == nil or id == "" then
    return nil, fill(TEXTS.beats.lane_claim_usage, { max = tostring(max) })
  end
  k = math.floor(k)
  if k < 1 or k > max then
    return nil, fill(TEXTS.beats.lane_bad_k, { k = tostring(k), max = tostring(max) })
  end
  return {
    k = k,
    id = id,
    branch = trim(parts[3] or ""),
    base = trim(parts[4] or ""),
    clean = trim(parts[5] or ""),
  }, nil
end

function LANES.parse_ref(payload)
  local max = job_capacity()
  if type(payload) ~= "string" then
    return nil, fill(TEXTS.beats.bad_payload, { action = "lane.release" })
  end
  local parts = split(trim(payload), "|")
  local k = tonumber(trim(parts[1] or ""))
  if k == nil then
    return nil, fill(TEXTS.beats.lane_release_usage, { max = tostring(max) })
  end
  k = math.floor(k)
  if k < 1 or k > max then
    return nil, fill(TEXTS.beats.lane_bad_k, { k = tostring(k), max = tostring(max) })
  end
  return k, nil
end

function M.install(deps)
  TEXTS = deps.texts
  ygg.on("lane.claim", function(payload)
    if stage() == STAGE.done then
      return reject("lane.claim", TEXTS.beats.req_after_done)
    end
    local spec, problem = LANES.parse_claim(payload)
    if problem ~= nil then
      return reject_payload("lane.claim", problem)
    end
    -- 分支名强制形态（r2 rev2）：非法名可读拒绝并留痕；空分支 = 未断言仍合法。
    -- 位置在载荷自洽组（parse 之后的自洽检查靠前，世界态检查靠后）。
    local branch_problem = LANES.branch_problem(spec.branch)
    if branch_problem ~= nil then
      return refuse(TEXTS.beats.req_wait_rejected, "lane.claim", branch_problem)
    end
    -- 脏树禁挂牌：主 Agent 已确认工作区干净才允许挂牌（这里只验它的断言）。
    if spec.clean ~= "1" then
      return refuse(TEXTS.beats.req_wait_rejected, "lane.claim", fill(TEXTS.beats.lane_claim_dirty, {
        k = tostring(spec.k),
      }))
    end
    local holder = LANES.req_of(spec.k)
    if holder ~= "" and holder ~= spec.id then
      return refuse(TEXTS.beats.req_wait_rejected, "lane.claim", fill(TEXTS.beats.lane_claim_occupied, {
        k = tostring(spec.k),
        id = holder,
      }))
    end
    local index = req_index_of(spec.id)
    if index == nil then
      return reject_payload("lane.claim", fill(TEXTS.beats.req_unknown, { id = tostring(spec.id) }))
    end
    if req_closed(index) then
      return refuse(TEXTS.beats.req_wait_rejected, "lane.claim", fill(TEXTS.beats.lane_claim_closed, {
        id = req_id(index),
        status = req_status(index),
      }))
    end
    local bound = req_lane(index)
    if bound > 0 and bound ~= spec.k then
      return refuse(TEXTS.beats.req_wait_rejected, "lane.claim", fill(TEXTS.beats.lane_claim_bound, {
        id = req_id(index),
        k = tostring(bound),
      }))
    end
    LANES.bind(spec.k, index, spec.branch, spec.base, spec.clean)
    ygg.narrate(fill(TEXTS.beats.lane_claimed, {
      k = tostring(spec.k),
      id = req_id(index),
      title = req_title(index),
      branch = LANES.text_or(spec.branch, TEXTS.beats.lane_field_none),
      base = LANES.text_or(spec.base, TEXTS.beats.lane_field_none),
      since = tostring(num(LANES.key(spec.k, "since"), 0)),
    }))
    render()
  end)
  ygg.on("lane.release", function(payload)
    if stage() == STAGE.done then
      return reject("lane.release", TEXTS.beats.req_after_done)
    end
    local k, problem = LANES.parse_ref(payload)
    if problem ~= nil then
      return reject_payload("lane.release", problem)
    end
    local freed = LANES.release_lane(k)
    ygg.narrate(fill(TEXTS.beats.lane_released, { k = tostring(k), id = freed }))
    render()
  end)
end


return M
