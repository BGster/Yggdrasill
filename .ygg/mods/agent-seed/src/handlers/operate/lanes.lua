-- src/handlers/operate/lanes.lua —— 泳道动作：lane.claim / lane.release / lane.pool / 迁移解析。
--
-- r1 分池（D-MA24）：载荷尾部新增可选池段——`lane.claim` 第 6 段、`lane.release` 第 2 段
-- （缺省 = 默认池，键形与叙事字节沿用旧版）；新动作 `lane.pool "label|path"` 幂等注册
-- 池（label 形状见 domain.lanes.pool_ok，path 仅形状校验、不验是否 git 仓——首次 claim
-- 时由宿主可读回传 error=repo）。
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
local STAGE, RUN_PHASE, job_capacity, req_closed, req_id, req_index_of, req_lane, req_status, req_title, stage =
  reqs.STAGE, reqs.RUN_PHASE, reqs.job_capacity, reqs.req_closed, reqs.req_id, reqs.req_index_of, reqs.req_lane, reqs.req_status, reqs.req_title, reqs.stage
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
  local pool = trim(parts[6] or "")
  if pool ~= "" and not LANES.pool_ok(pool) then
    return nil, fill(TEXTS.beats.lane_pool_bad_label, { pool = pool })
  end
  if pool ~= "" and not LANES.pool_registered(pool) then
    return nil, fill(TEXTS.beats.lane_pool_unknown, { pool = pool })
  end
  return {
    k = k,
    id = id,
    branch = trim(parts[3] or ""),
    base = trim(parts[4] or ""),
    clean = trim(parts[5] or ""),
    pool = pool,
  }, nil
end

function LANES.parse_ref(payload)
  local max = job_capacity()
  if type(payload) ~= "string" then
    return nil, nil, fill(TEXTS.beats.bad_payload, { action = "lane.release" })
  end
  local parts = split(trim(payload), "|")
  local k = tonumber(trim(parts[1] or ""))
  if k == nil then
    return nil, nil, fill(TEXTS.beats.lane_release_usage, { max = tostring(max) })
  end
  k = math.floor(k)
  if k < 1 or k > max then
    return nil, nil, fill(TEXTS.beats.lane_bad_k, { k = tostring(k), max = tostring(max) })
  end
  local pool = trim(parts[2] or "")
  if pool ~= "" and not LANES.pool_ok(pool) then
    return nil, nil, fill(TEXTS.beats.lane_pool_bad_label, { pool = pool })
  end
  if pool ~= "" and not LANES.pool_registered(pool) then
    return nil, nil, fill(TEXTS.beats.lane_pool_unknown, { pool = pool })
  end
  return k, pool, nil
end

-- lane.pool "label|path" 解析：label 形状守门 + path 形状校验（禁 | ; 换行与首尾空白）。
-- 不校验 path 是否 git 仓（Mod 无 IO；首次 claim 由宿主可读回传）。
function LANES.parse_pool(payload)
  if type(payload) ~= "string" then
    return nil, nil, fill(TEXTS.beats.bad_payload, { action = "lane.pool" })
  end
  local parts = split(trim(payload), "|")
  local label = trim(parts[1] or "")
  local raw_path = parts[2] or ""
  local path = trim(raw_path)
  if not LANES.pool_ok(label) then
    return nil, nil, fill(TEXTS.beats.lane_pool_bad_label, { pool = label })
  end
  if path == "" or raw_path ~= path or path:find("[|;\n\r]") ~= nil then
    return nil, nil, fill(TEXTS.beats.lane_pool_bad_segment, { pool = label })
  end
  return label, path, nil
end

function M.install(deps)
  TEXTS = deps.texts
  ygg.on("lane.pool", function(payload)
    -- 池注册与认领同幕（协调幕）：盘点/发布冻结 lane.*，退役全拒。
    if stage() == STAGE.retired then
      return reject("lane.pool", TEXTS.beats.req_after_done)
    end
    if not flow.phase_guard("lane.pool", { RUN_PHASE.coordinate }) then
      return
    end
    local label, path, problem = LANES.parse_pool(payload)
    if problem ~= nil then
      return reject_payload("lane.pool", problem)
    end
    local fresh = LANES.register_pool(label, path)
    ygg.narrate(fill(TEXTS.beats.lane_pool_registered, {
      pool = label,
      path = path,
      count = tostring(#LANES.pools()),
      state = fresh and "registered" or "updated",
    }))
    render()
  end)
  ygg.on("lane.claim", function(payload)
    -- 泳道认领只在协调幕（§2.7：立局未开工；盘点/发布冻结 lane.*；退役全拒）。
    if stage() == STAGE.retired then
      return reject("lane.claim", TEXTS.beats.req_after_done)
    end
    if not flow.phase_guard("lane.claim", { RUN_PHASE.coordinate }) then
      return
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
        lane = LANES.label(spec.pool, spec.k),
      }))
    end
    -- 占位判据按 (池, k) 精确作用：跨池同 k 互不干扰（r1 分池核心语义）。
    local holder = LANES.req_of(spec.pool, spec.k)
    if holder ~= "" and holder ~= spec.id then
      return refuse(TEXTS.beats.req_wait_rejected, "lane.claim", fill(TEXTS.beats.lane_claim_occupied, {
        lane = LANES.label(spec.pool, spec.k),
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
    if bound > 0 and (bound ~= spec.k or LANES.req_pool(index) ~= spec.pool) then
      return refuse(TEXTS.beats.req_wait_rejected, "lane.claim", fill(TEXTS.beats.lane_claim_bound, {
        id = req_id(index),
        lane = LANES.label(LANES.req_pool(index), bound),
      }))
    end
    LANES.bind(spec.pool, spec.k, index, spec.branch, spec.base, spec.clean)
    ygg.narrate(fill(TEXTS.beats.lane_claimed, {
      lane = LANES.label(spec.pool, spec.k),
      id = req_id(index),
      title = req_title(index),
      branch = LANES.text_or(spec.branch, TEXTS.beats.lane_field_none),
      base = LANES.text_or(spec.base, TEXTS.beats.lane_field_none),
      since = tostring(num(LANES.key(spec.pool, spec.k, "since"), 0)),
    }))
    render()
  end)
  ygg.on("lane.release", function(payload)
    -- 泳道释放在协调幕（与认领同口径；集成/门禁打回的自动释放不走这里）。
    if stage() == STAGE.retired then
      return reject("lane.release", TEXTS.beats.req_after_done)
    end
    if not flow.phase_guard("lane.release", { RUN_PHASE.coordinate }) then
      return
    end
    local k, pool, problem = LANES.parse_ref(payload)
    if problem ~= nil then
      return reject_payload("lane.release", problem)
    end
    local freed = LANES.release_lane(pool, k)
    ygg.narrate(fill(TEXTS.beats.lane_released, {
      lane = LANES.label(pool, k),
      id = freed,
    }))
    render()
  end)
end


return M
