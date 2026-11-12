-- src/lanes.lua —— 泳道账本 `lane.<k>.{req,branch,base,since,clean}` + job / 容量计数。
-- 另承载需求迭代分支名的强制形态判定（`LANES.branch_problem`）：lane.claim 与
-- git.worktree_claim 两个入口共用的唯一语义点，删除默认名生成机制后分支一律由调用方自报。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 资源由入口 install(deps) 注入。本模块唯一写的 KV 前缀：`lane.*`（唯一写方）。

local M = {}

local config = require("src.core.config")
local reqs = require("src.domain.requirements")
local util = require("src.core.util")

local TEXTS

local num, text_of =
  config.num, config.text_of
local job_capacity, req_id, req_index_of, req_job, req_key, req_lane, req_status =
  reqs.job_capacity, reqs.req_id, reqs.req_index_of, reqs.req_job, reqs.req_key, reqs.req_lane, reqs.req_status
local fill, join, trim =
  util.fill, util.join, util.trim

local LANES = {}

function LANES.key(k, name)
  return "lane." .. tostring(k) .. "." .. name
end

-- 需求迭代分支命名强制形态（r2 rev2）：^feature/[a-z0-9]{4}-<语义词段>$，总长 ≤48。
-- 返回 nil = 合法或未断言；否则返回填好的可读文案（TEXTS.beats.lane_branch_invalid）。
function LANES.branch_problem(branch)
  local b = trim(branch)
  if b == "" then
    return nil                                  -- 未断言仍合法（lane.claim 层）
  end
  local illegal = fill(TEXTS.beats.lane_branch_invalid, { branch = b })
  if #b > 48 then return illegal end            -- 总长上限
  if b:sub(1, 8) ~= "feature/" then return illegal end   -- 结构前缀（大小写敏感）
  local rest = b:sub(9)
  local dash = rest:find("-", 1, true)
  if dash == nil then return illegal end        -- feature/r1 裸编号在此落网
  local seg, semantic = rest:sub(1, dash - 1), rest:sub(dash + 1)
  if #seg ~= 4 or seg:match("[^a-z0-9]") ~= nil then return illegal end  -- uuid 段
  if semantic == "" then return illegal end                              -- 缺语义词段
  if semantic:match("[^a-z0-9%-]") ~= nil then return illegal end        -- 语义词段字符集
  if semantic:match("%a%a%a") == nil then return illegal end             -- ≥3 连续字母
  if semantic:match("^%d+$") ~= nil then return illegal end              -- 非纯数字
  if #semantic >= 8 and #semantic <= 16
    and semantic:match("^[0-9a-f]+$") ~= nil then return illegal end     -- 8–16 位纯 hex 硬非法
  return nil
end

function LANES.req_of(k)
  local value = ygg.kv.get(LANES.key(k, "req"))
  if type(value) == "string" then
    return trim(value)
  end
  if type(value) == "number" and value > 0 then
    return req_id(math.floor(value))
  end
  return ""
end

function LANES.tick()
  local next_tick = num("lane.seq", 0) + 1
  ygg.kv.set("lane.seq", next_tick)
  return next_tick
end

function LANES.clear(k)
  ygg.kv.set(LANES.key(k, "req"), "")
  ygg.kv.set(LANES.key(k, "branch"), "")
  ygg.kv.set(LANES.key(k, "base"), "")
  ygg.kv.set(LANES.key(k, "since"), 0)
  ygg.kv.set(LANES.key(k, "clean"), "")
  ygg.kv.set(LANES.key(k, "path"), "")
end

function LANES.bind(k, index, branch, base, clean)
  ygg.kv.set(LANES.key(k, "req"), req_id(index))
  ygg.kv.set(LANES.key(k, "branch"), branch or "")
  ygg.kv.set(LANES.key(k, "base"), base or "")
  ygg.kv.set(LANES.key(k, "since"), LANES.tick())
  ygg.kv.set(LANES.key(k, "clean"), clean or "")
  ygg.kv.set(req_key(index, "lane"), k)
  ygg.kv.set(req_key(index, "last_lane"), k)
end

function LANES.bound(index)
  local k = req_lane(index)
  if k > 0 and LANES.req_of(k) == req_id(index) then
    return k
  end
  return 0
end

-- 泳道当前自报分支（r2 rev2 E4：派发引导用）。
function LANES.branch_of(k)
  return text_of(LANES.key(k, "branch"), "")
end

function LANES.release(index)
  local k = req_lane(index)
  if k > 0 and LANES.req_of(k) == req_id(index) then
    LANES.clear(k)
  end
  ygg.kv.set(req_key(index, "lane"), 0)
end

-- 按**泳道号**释放（入参是 k，不是 req index）——反应器与 `lane.release` 共用这一处写方。
-- 返回上一持有人 id 文本；空泳道返回 `lane_release_idle`（叙事取值与内联实现逐字相同）。
function LANES.release_lane(k)
  local holder = LANES.req_of(k)
  local index = nil
  if holder ~= "" then
    index = req_index_of(holder)
  end
  LANES.clear(k)
  if index ~= nil and req_lane(index) == k then
    ygg.kv.set(req_key(index, "lane"), 0)
    return req_id(index)
  end
  if holder ~= "" then
    return holder
  end
  return TEXTS.beats.lane_release_idle
end

function LANES.free()
  for k = 1, job_capacity() do
    if LANES.req_of(k) == "" then
      return k
    end
  end
  return 0
end

function LANES.text_or(value, fallback)
  local text = trim(value)
  if text == "" then
    return fallback
  end
  return text
end

function LANES.stage_of(index)
  if index == nil then
    return "?"
  end
  local job = req_job(index)
  if job ~= "" then
    return job
  end
  return req_status(index)
end

local function free_lanes()
  local out = {}
  for number = 1, job_capacity() do
    if LANES.req_of(number) == "" then
      out[#out + 1] = number
    end
  end
  return out
end

local function lane_text()
  local parts = {}
  for number = 1, job_capacity() do
    local label = "lane-" .. tostring(number)
    local holder = LANES.req_of(number)
    if holder == "" then
      parts[#parts + 1] = fill(TEXTS.beats.req_lane_free, { lane = label })
    else
      parts[#parts + 1] = fill(TEXTS.beats.req_lane_used, {
        lane = label,
        id = holder,
        stage = LANES.stage_of(req_index_of(holder)),
      })
    end
  end
  return join(parts, TEXTS.beats.req_lane_sep)
end

local function narrate_lane_ledger()
  local total = job_capacity()
  local used = 0
  local free = {}
  local rows = {}
  for number = 1, total do
    local holder = LANES.req_of(number)
    if holder == "" then
      free[#free + 1] = "lane-" .. tostring(number)
    else
      used = used + 1
      local none = TEXTS.beats.lane_field_none
      rows[#rows + 1] = fill(TEXTS.beats.lane_ledger_used, {
        k = tostring(number),
        id = holder,
        stage = LANES.stage_of(req_index_of(holder)),
        branch = LANES.text_or(text_of(LANES.key(number, "branch"), ""), none),
        base = LANES.text_or(text_of(LANES.key(number, "base"), ""), none),
        since = tostring(num(LANES.key(number, "since"), 0)),
        clean = LANES.text_or(text_of(LANES.key(number, "clean"), ""), none),
      })
    end
  end
  ygg.narrate(fill(TEXTS.beats.lane_ledger_head, {
    used = tostring(used),
    total = tostring(total),
  }))
  for i = 1, #rows do
    ygg.narrate(rows[i])
  end
  if #free == 0 then
    ygg.narrate(fill(TEXTS.beats.lane_ledger_all_free, { total = tostring(total) }))
  else
    ygg.narrate(fill(TEXTS.beats.lane_ledger_none, { lanes = join(free, ", ") }))
  end
end

function M.install(deps)
  TEXTS = deps.texts
end

M.LANES = LANES
M.free_lanes = free_lanes
M.lane_text = lane_text
M.narrate_lane_ledger = narrate_lane_ledger

return M
