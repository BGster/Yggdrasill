-- src/domain/lanes.lua —— 泳道账本（r1 分池）：默认池 `lane.<k>.*`（键形沿用旧版，
-- = 运行仓 CWD toplevel 自己的池）+ 具名池 `lane.<label>.<k>.*`（D-MA24）+ 池注册表
-- `lane.pools` / `lane.pool.<label>.path` + job / 容量计数。
-- 另承载需求迭代分支名的强制形态判定（`LANES.branch_problem`）：lane.claim 与
-- git.worktree_claim 两个入口共用的唯一语义点，删除默认名生成机制后分支一律由调用方自报。
--
-- 池键形约定（r1 D-MA24）：pool == ""（默认池）→ `lane.<k>.<name>`（字节级沿用旧键）；
-- 具名池 → `lane.<label>.<k>.<name>`，label 形状 `^[a-z][a-z0-9-]{0,23}$`（首字符字母，
-- 与旧版纯数字 k 键空间零冲突），保留字 `seq` / `pools` 不许用。需求侧绑定双写
-- `req.<i>.lane`（k）与 `req.<i>.lane_pool`（label，""=默认池）。
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
local fill, join, split, trim =
  util.fill, util.join, util.split, util.trim

local LANES = {}

local POOL_RESERVED = { seq = true, pools = true }

-- ── 池模型（r1 D-MA24）─────────────────────────────────────────────────────────────

-- label 形状守门：^[a-z][a-z0-9-]{0,23}$ 且 ∉ {seq, pools}。Lua 模式无 {n,m} 有界量词
-- （{ } 是字面量），总长上限用 #label 判（首字符字母 + 无界类 + ≤24 = 同一形状）。
-- 返回 true = 合法。
function LANES.pool_ok(label)
  if type(label) ~= "string" then
    return false
  end
  if POOL_RESERVED[label] then
    return false
  end
  if #label > 24 then
    return false
  end
  return label:match("^[a-z][a-z0-9%-]*$") ~= nil
end

-- 已注册池的 label 列表（`lane.pools` 逗号串，插入序；默认池 "" 不在其列）。
function LANES.pools()
  local raw = text_of("lane.pools", "")
  local out = {}
  if raw == "" then
    return out
  end
  for _, label in ipairs(split(raw, ",")) do
    label = trim(label)
    if label ~= "" then
      out[#out + 1] = label
    end
  end
  return out
end

-- 池的本地检出路径（`lane.pool.<label>.path`，工具桥 --repo 参数的取值源）。
function LANES.pool_path(label)
  return text_of("lane.pool." .. label .. ".path", "")
end

function LANES.pool_registered(label)
  for _, name in ipairs(LANES.pools()) do
    if name == label then
      return true
    end
  end
  return false
end

-- 注册表唯一写方：幂等 upsert。新 label 追加进 `lane.pools` 尾部；path 覆盖写。
-- 返回是否为新池（叙事用）。path 形状由调用方（lane.pool handler）先校验。
function LANES.register_pool(label, path)
  local fresh = not LANES.pool_registered(label)
  if fresh then
    local all = LANES.pools()
    all[#all + 1] = label
    ygg.kv.set("lane.pools", join(all, ","))
  end
  ygg.kv.set("lane.pool." .. label .. ".path", path)
  return fresh
end

-- 按 --repo 路径反查池 label（工具反应器折叠用）；找不到返回 nil。
function LANES.pool_of_path(path)
  local want = trim(path or "")
  if want == "" then
    return nil
  end
  for _, label in ipairs(LANES.pools()) do
    if LANES.pool_path(label) == want then
      return label
    end
  end
  return nil
end

-- 泳道显示标签：默认池 `lane-<k>`（字节沿用旧行）；具名池 `lane-<label>-<k>`。
function LANES.label(pool, k)
  if pool == nil or pool == "" then
    return "lane-" .. tostring(k)
  end
  return "lane-" .. pool .. "-" .. tostring(k)
end

-- ── 键助手与分支形态 ────────────────────────────────────────────────────────────────

function LANES.key(pool, k, name)
  if pool == nil or pool == "" then
    return "lane." .. tostring(k) .. "." .. name
  end
  return "lane." .. pool .. "." .. tostring(k) .. "." .. name
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

-- ── 账本读写（全部池化；pool == "" = 默认池）───────────────────────────────────────

function LANES.req_of(pool, k)
  local value = ygg.kv.get(LANES.key(pool, k, "req"))
  if type(value) == "string" then
    return trim(value)
  end
  if type(value) == "number" and value > 0 then
    return req_id(math.floor(value))
  end
  return ""
end

-- 需求侧池绑定（""= 默认池；`bind` 写、`release` 清零）。
function LANES.req_pool(index)
  return text_of(req_key(index, "lane_pool"), "")
end

function LANES.tick()
  local next_tick = num("lane.seq", 0) + 1
  ygg.kv.set("lane.seq", next_tick)
  return next_tick
end

function LANES.clear(pool, k)
  ygg.kv.set(LANES.key(pool, k, "req"), "")
  ygg.kv.set(LANES.key(pool, k, "branch"), "")
  ygg.kv.set(LANES.key(pool, k, "base"), "")
  ygg.kv.set(LANES.key(pool, k, "since"), 0)
  ygg.kv.set(LANES.key(pool, k, "clean"), "")
  ygg.kv.set(LANES.key(pool, k, "path"), "")
end

function LANES.bind(pool, k, index, branch, base, clean)
  ygg.kv.set(LANES.key(pool, k, "req"), req_id(index))
  ygg.kv.set(LANES.key(pool, k, "branch"), branch or "")
  ygg.kv.set(LANES.key(pool, k, "base"), base or "")
  ygg.kv.set(LANES.key(pool, k, "since"), LANES.tick())
  ygg.kv.set(LANES.key(pool, k, "clean"), clean or "")
  ygg.kv.set(req_key(index, "lane"), k)
  ygg.kv.set(req_key(index, "lane_pool"), pool or "")
  ygg.kv.set(req_key(index, "last_lane"), k)
  ygg.kv.set(req_key(index, "last_lane_pool"), pool or "")
end

function LANES.bound(index)
  local k = req_lane(index)
  if k > 0 and LANES.req_of(LANES.req_pool(index), k) == req_id(index) then
    return k
  end
  return 0
end

-- 泳道当前自报分支（r2 rev2 E4：派发引导用）。
function LANES.branch_of(pool, k)
  return text_of(LANES.key(pool, k, "branch"), "")
end

function LANES.release(index)
  local k = req_lane(index)
  local pool = LANES.req_pool(index)
  if k > 0 and LANES.req_of(pool, k) == req_id(index) then
    LANES.clear(pool, k)
  end
  ygg.kv.set(req_key(index, "lane"), 0)
  ygg.kv.set(req_key(index, "lane_pool"), "")
end

-- 按**泳道坐标**释放（入参是 (pool, k)，不是 req index）——反应器与 `lane.release`
-- 共用这一处写方。返回上一持有人 id 文本；空泳道返回 `lane_release_idle`
--（叙事取值与内联实现逐字相同）。
function LANES.release_lane(pool, k)
  local holder = LANES.req_of(pool, k)
  local index = nil
  if holder ~= "" then
    index = req_index_of(holder)
  end
  LANES.clear(pool, k)
  if index ~= nil and req_lane(index) == k and LANES.req_pool(index) == (pool or "") then
    ygg.kv.set(req_key(index, "lane"), 0)
    ygg.kv.set(req_key(index, "lane_pool"), "")
    return req_id(index)
  end
  if holder ~= "" then
    return holder
  end
  return TEXTS.beats.lane_release_idle
end

-- 池内首个空道（并行度按池计：每池 k ∈ 1..job_capacity）。
function LANES.free(pool)
  for k = 1, job_capacity() do
    if LANES.req_of(pool, k) == "" then
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

-- ── 枚举与叙事（默认池先列、字节沿用旧行；具名池按注册序追加）─────────────────────

local function free_lanes(pool)
  local out = {}
  for number = 1, job_capacity() do
    if LANES.req_of(pool, number) == "" then
      out[#out + 1] = number
    end
  end
  return out
end

local function lane_text()
  local parts = {}
  local function emit(pool)
    for number = 1, job_capacity() do
      local label = LANES.label(pool, number)
      local holder = LANES.req_of(pool, number)
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
  end
  emit("")
  for _, pool in ipairs(LANES.pools()) do
    emit(pool)
  end
  return join(parts, TEXTS.beats.req_lane_sep)
end

local function narrate_lane_ledger()
  local function emit(pool)
    local total = job_capacity()
    local used = 0
    local free = {}
    local rows = {}
    for number = 1, total do
      local holder = LANES.req_of(pool, number)
      if holder == "" then
        free[#free + 1] = LANES.label(pool, number)
      else
        used = used + 1
        local none = TEXTS.beats.lane_field_none
        rows[#rows + 1] = fill(TEXTS.beats.lane_ledger_used, {
          k = tostring(number),
          lane = LANES.label(pool, number),
          id = holder,
          stage = LANES.stage_of(req_index_of(holder)),
          branch = LANES.text_or(text_of(LANES.key(pool, number, "branch"), ""), none),
          base = LANES.text_or(text_of(LANES.key(pool, number, "base"), ""), none),
          since = tostring(num(LANES.key(pool, number, "since"), 0)),
          clean = LANES.text_or(text_of(LANES.key(pool, number, "clean"), ""), none),
        })
      end
    end
    if pool == "" then
      ygg.narrate(fill(TEXTS.beats.lane_ledger_head, {
        used = tostring(used),
        total = tostring(total),
      }))
    else
      ygg.narrate(fill(TEXTS.beats.lane_ledger_head_pool, {
        pool = pool,
        used = tostring(used),
        total = tostring(total),
      }))
    end
    for i = 1, #rows do
      ygg.narrate(rows[i])
    end
    if #free == 0 then
      if pool == "" then
        ygg.narrate(fill(TEXTS.beats.lane_ledger_all_free, { total = tostring(total) }))
      else
        ygg.narrate(fill(TEXTS.beats.lane_ledger_all_free_pool, { pool = pool, total = tostring(total) }))
      end
    else
      ygg.narrate(fill(TEXTS.beats.lane_ledger_none, { lanes = join(free, ", ") }))
    end
  end
  emit("")
  for _, pool in ipairs(LANES.pools()) do
    emit(pool)
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
