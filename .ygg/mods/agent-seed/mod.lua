-- packages/mods/agent-seed/mod.lua
-- M-Agent 泳道 A：冷启动 seed mod —— **双层状态机 + 提案验收**的 Agent 元工作流。
--
-- 一、协调器（`fsm.stage`，单机单事件 `stage.advance`，payload = 目标态）：
--
--   intake ──task.intake──▶ coordinate ──coordinate.submit──▶ review ──review.note──▶ done
--                              ▲                                 │
--                              └────── verify.submit=false ───────┘（打回重做）
--
--   协调器只管「派活 / 收提案 / 串行验收 / 最终复盘」；**不承载开发本身**。
--
-- 二、需求级流水线（每条需求独立生命周期，落 KV `req.<i>.status`）：
--
--   draft ──requirement.dispatch──▶ analyzing ──proposal──▶ analysis_review
--     ──accept──▶ planning ──dispatch──▶ planning(job) ──proposal──▶ plan_review
--     ──accept──▶ ready ──dispatch──▶ developing(lane) ──proposal──▶ dev_review
--     ──accept──▶ done
--
--   另有可追溯终态 split（已拆分）/ merged（已并入）/ dropped（已弃）与派生标记
--   blocked（= 有未解决冲突，展示态）。**多条需求的分析 / 规划 / 开发都可异步并行**
--   （全局最多 `tuning.job_capacity` 个 job），主 Agent 的**提案验收、共享需求图修改、
--   集成与最终 review 一律串行**（同一份需求图、同一个结论）。
--
-- 三、提案协议（subagent 不直接改共享状态）：
--
--   proposal.submit  "req_id|revision|stage|summary|artifacts[|changes]"
--   proposal.accept  "pN" / "rN" / ""（空 = 按 requirement_id canonical 序取一条）
--   proposal.reject  "pN[|reason]" / "rN[|reason]"
--
--   - stage ∈ analysis | plan | development；changes 可选（subagent 只**建议**）：
--     `split=a,b` / `merge=r5` / `amend=title:desc` / `conflict=r5:note`
--   - 提案绑定 revision：验收时 `revision ≠ req.<i>.rev` → 判 **stale**（可读拒绝，
--     旧提案不得覆盖新决定，需 rebase 后重提）。
--   - split / merge / amend / conflict 由**主 Agent 验收后**才能落地（提案里的 changes
--     在 proposal.accept 内应用）；subagent 无法直接改需求图。
--   - 冲突仍需**用户确认**：goal 遇到未解决冲突会停下等你选（不自动选默认路径）。
--
-- 四、泳道：分析 / 规划 / 开发都是异步 job，全局上限 `tuning.job_capacity`（默认 4）；
--   开发额外占一条 lane（`lane.<k>`，账本 `lane.<k>.{req,branch,base,since,clean}`），
--   只有「依赖就绪 + 无冲突」的需求能进开发泳道，相互声明冲突的需求不可能同时
--   developing（冲突标记让两者都不可调度）。
--   内核仍**单线程 dispatch + 单 autosave**：泳道只是宿主侧的并行工作单元，结果一律
--   通过 inbox 回到这里，宿主按 requirement_id / external_id canonical 序合流。
--
-- 五、收口语义：所有需求 `done`（或已 split/merged/dropped 且其后继全部 done）
--   ∧ 无待验收提案 ∧ 无未解决冲突 → `coordinate.submit` 才允许进 review；
--   review 里 `verify.submit=true` 进复盘，`review.note` 收口到 done（【结局）。
--
-- 六、展示与防自批两层（不影响状态机语义）：
--   - **人类动作门禁**（§人类动作门禁）：guide 下「会产生通过效果」的动作（验收 / 门禁 /
--     收口 / 复盘 / 切运行模式）只能经 `ygg.choice_selected` 路由进入，单发直派被可读拒绝；
--     goal 也走同一条路由，由外部决策命令决定是否派发（不会默认批准）。
--   - **二级菜单页**（§二级菜单页）：`config` 收敛为顶层单一入口，三项设置进子页；
--     纯展示层（KV `ui.page` / `ui.page.stage`），阶段一变即清空，不占主前进位置。

-- ============================================================================
-- §资源 装载期快照（资源只读、不入档）
-- ============================================================================
local TEXTS = ygg.data("ygg:scene/texts")
local CONFIG = ygg.data("ygg:agent/config")
local PLAYBOOK = ygg.data("ygg:agent/playbook")

-- ============================================================================
-- §工具 沙箱无 table 库，自备纯函数小工具
-- ============================================================================
local function fill(template, vars)
  if type(template) ~= "string" then
    return ""
  end
  return (template:gsub("{{([%w_]+)}}", function(key)
    local value = vars[key]
    if value == nil then
      return "{{" .. key .. "}}"
    end
    return tostring(value)
  end))
end

local function split(text, sep)
  local out = {}
  if type(text) ~= "string" or text == "" then
    return out
  end
  local start = 1
  while true do
    local index, last = text:find(sep, start, true)
    if index == nil then
      out[#out + 1] = text:sub(start)
      return out
    end
    out[#out + 1] = text:sub(start, index - 1)
    start = last + 1
  end
end

local function cut(text, sep)
  local index, last = text:find(sep, 1, true)
  if index == nil then
    return text, nil
  end
  return text:sub(1, index - 1), text:sub(last + 1)
end

local function trim(text)
  if type(text) ~= "string" then
    return ""
  end
  return (text:gsub("^%s+", ""):gsub("%s+$", ""))
end

local function join(items, sep)
  local out = ""
  for i = 1, #items do
    if i > 1 then
      out = out .. sep
    end
    out = out .. tostring(items[i])
  end
  return out
end

local function contains(list, value)
  for i = 1, #list do
    if list[i] == value then
      return true
    end
  end
  return false
end

local function to_bool(text)
  if text == "true" then
    return true
  end
  if text == "false" then
    return false
  end
  return nil
end

-- 沙箱不暴露 string 库静态面：自备填充。
local function spaces(count)
  local out = ""
  for _ = 1, count do
    out = out .. " "
  end
  return out
end

-- ============================================================================
-- §状态 协调器阶段（KV `fsm.stage`）
-- ============================================================================
local STAGE = {
  intake = "intake",
  coordinate = "coordinate",
  review = "review",
  done = "done",
}

local function stage()
  local value = ygg.kv.get("fsm.stage")
  if type(value) ~= "string" then
    return STAGE.intake
  end
  return value
end

-- ============================================================================
-- §配置 两层配置：动态覆盖（KV `settings.<path>`）> 静态默认（config.json）
-- ============================================================================
local function static(path)
  local node = CONFIG
  for _, part in ipairs(split(path, ".")) do
    if type(node) ~= "table" then
      return nil
    end
    node = node[part]
  end
  return node
end

local function effective(path)
  local override = ygg.kv.get("settings." .. path)
  if override ~= nil then
    return override
  end
  return static(path)
end

local MODES = { "off", "suggest", "auto" }

local function mode()
  local value = effective("evolution.mode")
  if type(value) == "string" then
    return value
  end
  return "off"
end

local function next_mode()
  local current = mode()
  for i = 1, #MODES do
    if MODES[i] == current then
      return MODES[(i % #MODES) + 1]
    end
  end
  return MODES[1]
end

local PLAY_MODES = { "goal", "guide" }
local DEFAULT_PLAY_MODE = "guide"

local function play_mode()
  local value = effective("play.mode")
  if type(value) == "string" and contains(PLAY_MODES, value) then
    return value
  end
  return DEFAULT_PLAY_MODE
end

local function next_play_mode()
  local current = play_mode()
  for i = 1, #PLAY_MODES do
    if PLAY_MODES[i] == current then
      return PLAY_MODES[(i % #PLAY_MODES) + 1]
    end
  end
  return PLAY_MODES[1]
end

local function strict_gates()
  return effective("gates.require_all_green") == true
end

local function max_per_run()
  local value = effective("evolution.max_per_run")
  if type(value) == "number" then
    return value
  end
  return 0
end

local function num(key, fallback)
  local value = ygg.kv.get(key)
  if type(value) == "number" then
    return value
  end
  return fallback
end

local function text_of(key, fallback)
  local value = ygg.kv.get(key)
  if type(value) == "string" and value ~= "" then
    return value
  end
  return fallback
end

local function task_title()
  return text_of("task.title", TEXTS.beats.default_task_title)
end

local function task_desc()
  return text_of("task.description", TEXTS.beats.default_task_desc)
end

-- ============================================================================
-- §需求 需求图（KV `req.*`）——状态、依赖、修订、冲突、拆并追溯
--
-- status ∈ draft / analyzing / analysis_review / planning / plan_review /
--          ready / developing / dev_review / done / split / merged / dropped
-- 另有：conflict（未解决冲突说明 → 展示为 blocked）、needs_amend（待修订）、
--       stale（提案 rev 过期提示）、job（在跑的 job 阶段）、lane（开发泳道）。
-- ============================================================================
local R_DRAFT = "draft"
local R_ANALYZING = "analyzing"
local R_ANALYSIS_REVIEW = "analysis_review"
local R_PLANNING = "planning"
local R_PLAN_REVIEW = "plan_review"
local R_PLAN_GATE = "plan_gate"
local R_READY = "ready"
local R_DEVELOPING = "developing"
local R_DEV_REVIEW = "dev_review"
local R_ACCEPT = "accept"
local R_DONE = "done"
local R_SPLIT = "split"
local R_MERGED = "merged"
local R_DROPPED = "dropped"

-- 终态：不再需要调度，也不再阻塞收口（done 或已转化 / 已弃）。
local R_TERMINAL = {
  [R_DONE] = true,
  [R_SPLIT] = true,
  [R_MERGED] = true,
  [R_DROPPED] = true,
}

local function req_count()
  return num("req.count", 0)
end

local function req_key(index, name)
  return "req." .. tostring(index) .. "." .. name
end

local function req_id(index)
  return "r" .. tostring(index)
end

local function req_status(index)
  local value = ygg.kv.get(req_key(index, "status"))
  if type(value) == "string" then
    return value
  end
  return ""
end

local function req_title(index)
  return text_of(req_key(index, "title"), TEXTS.beats.default_req_title)
end

local function req_rev(index)
  return num(req_key(index, "rev"), 1)
end

local function req_deps(index)
  local out = {}
  for _, part in ipairs(split(text_of(req_key(index, "deps"), ""), ",")) do
    local piece = trim(part)
    if piece ~= "" then
      out[#out + 1] = piece
    end
  end
  return out
end

local function req_deps_text(index)
  local deps = req_deps(index)
  if #deps == 0 then
    return TEXTS.beats.req_deps_none
  end
  return fill(TEXTS.beats.req_deps_have, { ids = join(deps, ",") })
end

local function req_index_of(id)
  if type(id) ~= "string" then
    return nil
  end
  local raw = trim(id)
  if raw:sub(1, 1) ~= "r" then
    return nil
  end
  local number = tonumber(raw:sub(2))
  if number == nil then
    return nil
  end
  number = math.floor(number)
  if number < 1 or number > req_count() or req_status(number) == "" then
    return nil
  end
  return number
end

local function req_conflict(index)
  return text_of(req_key(index, "conflict"), "")
end

local function req_conflict_peer(index)
  return text_of(req_key(index, "conflict_peer"), "")
end

local function req_needs_amend(index)
  return ygg.kv.get(req_key(index, "needs_amend")) == true
end

local function req_stale(index)
  return ygg.kv.get(req_key(index, "stale")) == true
end

local function req_successors(index)
  local status = req_status(index)
  local raw = ""
  if status == R_SPLIT then
    raw = text_of(req_key(index, "children"), "")
  elseif status == R_MERGED then
    raw = text_of(req_key(index, "into"), "")
  end
  local out = {}
  for _, part in ipairs(split(raw, ",")) do
    local piece = trim(part)
    if piece ~= "" then
      out[#out + 1] = piece
    end
  end
  return out
end

-- 依赖（以及收口判定）用的「已满足」递归：done / dropped 满足；split / merged
-- 看其后继是否全部满足；其余一律不满足。visiting 走路径防环（写路径已拦环）。
local req_satisfied

req_satisfied = function(index, visiting)
  visiting = visiting or {}
  if visiting[index] then
    return false
  end
  local status = req_status(index)
  if status == R_DONE or status == R_DROPPED then
    return true
  end
  if status == R_SPLIT or status == R_MERGED then
    local successors = req_successors(index)
    if #successors == 0 then
      return false
    end
    visiting[index] = true
    local ok = true
    for _, id in ipairs(successors) do
      local child = req_index_of(id)
      if child == nil or not req_satisfied(child, visiting) then
        ok = false
        break
      end
    end
    visiting[index] = nil
    return ok
  end
  return false
end

local function req_deps_ok(index)
  for _, id in ipairs(req_deps(index)) do
    local dep = req_index_of(id)
    if dep == nil or not req_satisfied(dep, nil) then
      return false
    end
  end
  return true
end

local function req_unmet_deps(index)
  local out = {}
  for _, id in ipairs(req_deps(index)) do
    local dep = req_index_of(id)
    if dep == nil or not req_satisfied(dep, nil) then
      out[#out + 1] = id
    end
  end
  return join(out, ",")
end

-- 开发候选未就绪的原因（派活被拒时给人看）。
local function req_dev_blocked_text()
  local parts = {}
  for i = 1, req_count() do
    if req_status(i) == R_READY and not req_dev_ready(i) then
      if req_conflict(i) ~= "" then
        parts[#parts + 1] = fill(TEXTS.beats.req_why_conflict, { id = req_id(i) })
      elseif req_needs_amend(i) then
        parts[#parts + 1] = fill(TEXTS.beats.req_why_amend, { id = req_id(i) })
      else
        parts[#parts + 1] = fill(TEXTS.beats.req_why_deps, { id = req_id(i), ids = req_unmet_deps(i) })
      end
      if #parts >= 3 then
        break
      end
    end
  end
  if #parts == 0 then
    return TEXTS.beats.req_why_none
  end
  return join(parts, TEXTS.beats.req_why_sep)
end

-- 可进「开发」的前提：ready + 依赖已满足 + 无冲突 + 不需要修订。
local function req_dev_ready(index)
  return req_status(index) == R_READY
    and req_conflict(index) == ""
    and not req_needs_amend(index)
    and req_deps_ok(index)
end

-- 展示态：blocked = 有未解决冲突或待修订（需人工）；stale 单独提示。
local function req_display(index)
  local status = req_status(index)
  if status == "" then
    return "?"
  end
  if req_conflict(index) ~= "" or req_needs_amend(index) then
    if status ~= R_DONE and status ~= R_SPLIT and status ~= R_MERGED and status ~= R_DROPPED then
      return "blocked"
    end
  end
  return status
end

local function req_closed(index)
  return R_TERMINAL[req_status(index)] == true
end

local function req_tally(status)
  local total = 0
  for i = 1, req_count() do
    if req_status(i) == status then
      total = total + 1
    end
  end
  return total
end

local function req_conflict_total()
  local total = 0
  for i = 1, req_count() do
    if (req_conflict(i) ~= "" or req_needs_amend(i)) and not req_closed(i) then
      total = total + 1
    end
  end
  return total
end

-- 冲突计数镜像落盘（宿主 goal 的暂停判据；不解析 KV 键、不推断）。
local function req_sync_conflicts()
  local total = req_conflict_total()
  ygg.kv.set("req.conflicts", total)
  return total
end

-- 未收口需求（non-terminal）数量。
local function req_open_total()
  local total = 0
  for i = 1, req_count() do
    if not req_closed(i) then
      total = total + 1
    end
  end
  return total
end

-- 全部需求都已滿足（-done / dropped / 拆并后继全 done）。
local function req_all_done()
  for i = 1, req_count() do
    if not req_satisfied(i, nil) then
      return false
    end
  end
  return true
end

-- 派活候选：状态 → 该跑的 job 阶段（nil = 现在不派）。
local function req_job_stage(index)
  local status = req_status(index)
  if status == R_DRAFT then
    return "analysis"
  elseif status == R_PLANNING then
    return "plan"
  elseif status == R_READY then
    return "development"
  end
  return nil
end

local function req_job(index)
  return text_of(req_key(index, "job"), "")
end

local function req_lane(index)
  return num(req_key(index, "lane"), 0)
end

-- 在跑 job 数量（全局并行上限的判据）。
local function job_total()
  local total = 0
  for i = 1, req_count() do
    if req_job(i) ~= "" then
      total = total + 1
    end
  end
  return total
end

local function job_capacity()
  local value = TEXTS.tuning.job_capacity
  if type(value) == "number" and value > 0 then
    return value
  end
  return 4
end

-- 需求总数上限（防需求图无限膨胀）
local function req_capacity()
  local value = TEXTS.tuning.req_capacity
  if type(value) == "number" and value > 0 then
    return value
  end
  return 32
end

-- ============================================================================
-- §泳道 泳道账本（`lane.<k>.{req,branch,base,since,clean}`）
--
-- 与多仓参照实现（`lane.<repo>.<k>.*`）的差异：本 Mod 单仓、无 `repo` 层，泳道号就是
-- 全局 job 槽位号（1..`tuning.job_capacity`），账本与旧编号**同一批键**——不新开命名空间，
-- 旧存档的 `lane.<k>.req`（数字 index）照样读得懂（见 LANES.req_of），因此**不需要一次性迁移**。
--
-- 物理 worktree 由主 Agent 操作（建 / 删 / `git status` 检查都是它的事），mod 只记事实：
-- 谁占了哪条泳道、分支、base、挂牌序号，以及 clean 断言的结论（**mod 不能跑 git**）。
-- `since` 记的是单调挂牌序号（沙箱没有墙钟）；真正的墙钟时间由主 Agent 写在 worktree 根的
-- `.lane`。`branch` / `base` / `clean` 由 `lane.claim` 填；`requirement.dispatch` 自动挂牌时
-- 三者留空 = **未断言**（不是「干净」，也不是「没有分支」）。
-- ============================================================================
local LANES = {}

function LANES.key(k, name)
  return "lane." .. tostring(k) .. "." .. name
end

-- 读账本占用者（需求 id 字符串；"" = 空闲）。容忍旧式数字写法（旧存档写的是 index）。
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

-- 挂牌序号（单调递增）：沙箱没有墙钟，`since` 只表达「第几次挂牌」，不表达时间点。
function LANES.tick()
  local next_tick = num("lane.seq", 0) + 1
  ygg.kv.set("lane.seq", next_tick)
  return next_tick
end

-- 摘牌：五键一起清（漏清一个就会留下幽灵 branch/base）。
function LANES.clear(k)
  ygg.kv.set(LANES.key(k, "req"), "")
  ygg.kv.set(LANES.key(k, "branch"), "")
  ygg.kv.set(LANES.key(k, "base"), "")
  ygg.kv.set(LANES.key(k, "since"), 0)
  ygg.kv.set(LANES.key(k, "clean"), "")
end

-- 挂牌（同一次事务写五键 + 需求的泳道号）。branch/base/clean 缺省 = 未断言。
function LANES.bind(k, index, branch, base, clean)
  ygg.kv.set(LANES.key(k, "req"), req_id(index))
  ygg.kv.set(LANES.key(k, "branch"), branch or "")
  ygg.kv.set(LANES.key(k, "base"), base or "")
  ygg.kv.set(LANES.key(k, "since"), LANES.tick())
  ygg.kv.set(LANES.key(k, "clean"), clean or "")
  ygg.kv.set(req_key(index, "lane"), k)
end

-- 该需求的预挂牌泳道（0 = 没有）：dispatch 复用它，不覆盖已记的 branch/base/clean。
function LANES.bound(index)
  local k = req_lane(index)
  if k > 0 and LANES.req_of(k) == req_id(index) then
    return k
  end
  return 0
end

-- 摘掉该需求持有的泳道（幽灵防护：只在账本确实记着它时才清，不误伤别人）。
function LANES.release(index)
  local k = req_lane(index)
  if k > 0 and LANES.req_of(k) == req_id(index) then
    LANES.clear(k)
  end
  ygg.kv.set(req_key(index, "lane"), 0)
end

-- 第一条空闲泳道号；0 = 全占。
function LANES.free()
  for k = 1, job_capacity() do
    if LANES.req_of(k) == "" then
      return k
    end
  end
  return 0
end

-- 展示用：空字段读成「—」（mod 不知道的部分不假装知道）。
function LANES.text_or(value, fallback)
  local text = trim(value)
  if text == "" then
    return fallback
  end
  return text
end

-- 展示用阶段：在跑 job 优先，否则用需求状态（空括号会让人误读成「没状态」）。
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

-- 账本明细（map.show 专用：紧凑的 lane_text 塞不下 branch/base/since/clean）。
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
    ygg.narrate(fill(TEXTS.beats.lane_ledger_none, { lanes = join(free, "、") }))
  end
end

-- 入队 / 派生 / 拆并共用的唯一写入点。
local function req_append(title, description, ext, deps, origin)
  local index = req_count() + 1
  ygg.kv.set("req.count", index)
  ygg.kv.set(req_key(index, "title"), title)
  ygg.kv.set(req_key(index, "description"), description)
  ygg.kv.set(req_key(index, "status"), R_DRAFT)
  ygg.kv.set(req_key(index, "ext"), ext or "")
  ygg.kv.set(req_key(index, "deps"), join(deps or {}, ","))
  ygg.kv.set(req_key(index, "rev"), 1)
  ygg.kv.set(req_key(index, "hist"), "")
  ygg.kv.set(req_key(index, "conflict"), "")
  ygg.kv.set(req_key(index, "conflict_peer"), "")
  ygg.kv.set(req_key(index, "needs_amend"), false)
  ygg.kv.set(req_key(index, "stale"), false)
  ygg.kv.set(req_key(index, "job"), "")
  ygg.kv.set(req_key(index, "lane"), 0)
  ygg.kv.set(req_key(index, "artifacts"), "")
  ygg.kv.set(req_key(index, "summary"), "")
  ygg.kv.set(req_key(index, "children"), "")
  ygg.kv.set(req_key(index, "into"), "")
  ygg.kv.set(req_key(index, "origin"), origin or "")
  ygg.kv.set(req_key(index, "proposal"), 0)
  ygg.kv.set("req.last", req_id(index))
  return index
end

local function req_push_history(index, note)
  local entries = {}
  for _, part in ipairs(split(text_of(req_key(index, "hist"), ""), ";")) do
    local piece = trim(part)
    if piece ~= "" then
      entries[#entries + 1] = piece
    end
  end
  entries[#entries + 1] = note
  while #entries > 4 do
    local kept = {}
    for i = 2, #entries do
      kept[#kept + 1] = entries[i]
    end
    entries = kept
  end
  ygg.kv.set(req_key(index, "hist"), join(entries, ";"))
end

local function req_bump_rev(index)
  local rev = req_rev(index) + 1
  ygg.kv.set(req_key(index, "rev"), rev)
  ygg.kv.set(req_key(index, "stale"), false)
  return rev
end

-- 依赖防环：沿 from 的依赖链能否回到 target。
local function req_reaches(from, target, visiting)
  if from == target then
    return true
  end
  visiting = visiting or {}
  if visiting[from] then
    return false
  end
  visiting[from] = true
  for _, id in ipairs(req_deps(from)) do
    local dep = req_index_of(id)
    if dep ~= nil and req_reaches(dep, target, visiting) then
      return true
    end
  end
  return false
end

local function req_validate_deps(index, ids)
  local seen = {}
  for _, id in ipairs(ids) do
    local dep = req_index_of(id)
    if dep == nil then
      return fill(TEXTS.beats.req_bad_dep, { id = tostring(id) })
    end
    if dep == index then
      return fill(TEXTS.beats.req_self_dep, { id = req_id(index) })
    end
    if seen[dep] then
      return fill(TEXTS.beats.req_dup_dep, { id = req_id(dep) })
    end
    seen[dep] = true
    if req_reaches(dep, index, nil) then
      return fill(TEXTS.beats.req_cycle_dep, { id = req_id(index), dep = req_id(dep) })
    end
  end
  return nil
end

-- ============================================================================
-- §提案 subagent 只提议、不改共享状态：提案落 KV `prop.*`，主 Agent 串行验收。
-- 状态：pending / accepted / rejected / stale（rev 不匹配 → stale）。
-- ============================================================================
local P_PENDING = "pending"
local P_ACCEPTED = "accepted"
local P_REJECTED = "rejected"
local P_STALE = "stale"

local function prop_count()
  return num("prop.count", 0)
end

local function prop_key(index, name)
  return "prop." .. tostring(index) .. "." .. name
end

local function prop_id(index)
  return "p" .. tostring(index)
end

local function prop_status(index)
  local value = ygg.kv.get(prop_key(index, "status"))
  if type(value) == "string" then
    return value
  end
  return ""
end

local function prop_req(index)
  return num(prop_key(index, "req"), 0)
end

local function prop_rev(index)
  return num(prop_key(index, "rev"), 0)
end

local function prop_stage(index)
  return text_of(prop_key(index, "stage"), "")
end

local function prop_append(req_index, rev, stage_name, summary, artifacts, ext, changes)
  local index = prop_count() + 1
  ygg.kv.set("prop.count", index)
  ygg.kv.set(prop_key(index, "req"), req_index)
  ygg.kv.set(prop_key(index, "rev"), rev)
  ygg.kv.set(prop_key(index, "stage"), stage_name)
  ygg.kv.set(prop_key(index, "summary"), summary)
  ygg.kv.set(prop_key(index, "artifacts"), artifacts)
  ygg.kv.set(prop_key(index, "status"), P_PENDING)
  ygg.kv.set(prop_key(index, "ext"), ext or "")
  ygg.kv.set(prop_key(index, "changes"), changes or "")
  return index
end

local function prop_pending_total()
  local total = 0
  for i = 1, prop_count() do
    if prop_status(i) == P_PENDING then
      total = total + 1
    end
  end
  return total
end

-- canonical 验收序：requirement_id 升序，同一需求内提案 id 升序。
local function prop_next()
  local best = nil
  for i = 1, prop_count() do
    if prop_status(i) == P_PENDING then
      if best == nil then
        best = i
      else
        local a = prop_req(i)
        local b = prop_req(best)
        if a < b or (a == b and i < best) then
          best = i
        end
      end
    end
  end
  return best
end

-- 外部幂等键：同一 external_id 的提案不重复入账。
local function prop_by_ext(ext)
  if type(ext) ~= "string" or ext == "" then
    return nil
  end
  for i = 1, prop_count() do
    if ygg.kv.get(prop_key(i, "ext")) == ext then
      return i
    end
  end
  return nil
end

-- ---- 共享需求图操作（主 Agent 直改与提案验收应用共用同一套规则）----------

local function req_do_split(parent, children)
  local deps = req_deps(parent)
  local ids = {}
  for i = 1, #children do
    local child = req_append(children[i], fill(TEXTS.beats.req_split_desc, { id = req_id(parent) }), "", deps, req_id(parent))
    ids[#ids + 1] = req_id(child)
  end
  ygg.kv.set(req_key(parent, "status"), R_SPLIT)
  ygg.kv.set(req_key(parent, "children"), join(ids, ","))
  req_push_history(parent, fill(TEXTS.beats.req_split_hist, { ids = join(ids, ",") }))
  return ids
end

local function req_do_merge(sources, title, description)
  local seen = {}
  for i = 1, #sources do
    seen[sources[i]] = true
  end
  local dep_seen = {}
  local dep_order = {}
  for i = 1, #sources do
    for _, dep in ipairs(req_deps(sources[i])) do
      local dep_index = req_index_of(dep)
      if dep_index ~= nil then
        if not seen[dep_index] and not dep_seen[dep_index] then
          dep_seen[dep_index] = true
          dep_order[#dep_order + 1] = req_id(dep_index)
        end
      elseif not dep_seen[dep] then
        dep_seen[dep] = true
        dep_order[#dep_order + 1] = dep
      end
    end
  end
  local ids = {}
  for i = 1, #sources do
    ids[#ids + 1] = req_id(sources[i])
  end
  local next_title = title
  if next_title == "" then
    next_title = fill(TEXTS.beats.req_merge_title, { ids = join(ids, ",") })
  end
  local next_desc = description
  if next_desc == "" then
    next_desc = fill(TEXTS.beats.req_merge_desc, { ids = join(ids, ",") })
  end
  local target = req_append(next_title, next_desc, "", dep_order, "")
  for i = 1, #sources do
    local index = sources[i]
    ygg.kv.set(req_key(index, "status"), R_MERGED)
    ygg.kv.set(req_key(index, "into"), req_id(target))
    ygg.kv.set(req_key(index, "conflict"), "")
    ygg.kv.set(req_key(index, "conflict_peer"), "")
    ygg.kv.set(req_key(index, "job"), "")
    req_push_history(index, fill(TEXTS.beats.req_merge_hist, { id = req_id(target) }))
  end
  return target, next_title, dep_order
end

local function req_do_conflict(first, second, note)
  ygg.kv.set(req_key(first, "conflict"), note)
  ygg.kv.set(req_key(first, "conflict_peer"), req_id(second))
  ygg.kv.set(req_key(second, "conflict"), note)
  ygg.kv.set(req_key(second, "conflict_peer"), req_id(first))
  req_push_history(first, TEXTS.beats.req_conflict_hist)
  req_push_history(second, TEXTS.beats.req_conflict_hist)
end

-- 验收提案时应用的拟议变更（subagent 只能建议，落不落地看验收）。
local function apply_changes(index, changes)
  if type(changes) ~= "string" or trim(changes) == "" then
    return ""
  end
  local kind, body = cut(trim(changes), "=")
  kind = trim(kind)
  body = trim(body or "")
  if kind == "split" then
    local children = {}
    for _, part in ipairs(split(body, ",")) do
      local piece = trim(part)
      if piece ~= "" then
        children[#children + 1] = piece
      end
    end
    if #children < 2 then
      return ""
    end
    local ids = req_do_split(index, children)
    return fill(TEXTS.beats.req_split_ok, { id = req_id(index), ids = join(ids, ","), deps = join(req_deps(index), ",") })
  elseif kind == "merge" then
    local other = req_index_of(body)
    if other == nil or other == index then
      return ""
    end
    local sources = { index, other }
    local target, title = req_do_merge(sources, "", "")
    return fill(TEXTS.beats.req_merge_ok, {
      ids = req_id(index) .. "," .. req_id(other),
      id = req_id(target),
      title = title,
    })
  elseif kind == "amend" then
    local title, description = cut(body, ":")
    title = trim(title)
    description = trim(description or "")
    if title ~= "" then
      ygg.kv.set(req_key(index, "title"), title)
    end
    if description ~= "" then
      ygg.kv.set(req_key(index, "description"), description)
    end
    return fill(TEXTS.beats.req_amend_ok_short, { id = req_id(index) })
  elseif kind == "conflict" then
    local peer_id, note = cut(body, ":")
    local peer = req_index_of(trim(peer_id or ""))
    if peer == nil or peer == index then
      return ""
    end
    req_do_conflict(index, peer, trim(note or "") ~= "" and trim(note) or TEXTS.beats.req_conflict_default)
    return fill(TEXTS.beats.req_conflict_ok_short, { first = req_id(index), second = req_id(peer) })
  end
  return ""
end

-- 验收后推进需求自己的状态机（analysis → planning → ready → done）。
local function req_advance_after_accept(index, stage_name)
  local next_status
  if stage_name == "analysis" then
    next_status = R_PLANNING
  elseif stage_name == "plan" then
    -- plan 完成后进**plan_gate**：主 Agent 批了才能 develop（不能自动跳过）。
    next_status = R_PLAN_GATE
  elseif stage_name == "development" then
    -- 执行方交付（accept）≠ 主 Agent 已验收：先落 accept，等 accept_gate。
    next_status = R_ACCEPT
  else
    return nil
  end
  ygg.kv.set(req_key(index, "status"), next_status)
  ygg.kv.set(req_key(index, "job"), "")
  if next_status == R_ACCEPT then
    -- 交付后释放泳道（连同 branch/base/since/clean 一起清）；重做时重新分配（不长期占位）。
    LANES.release(index)
  end
  return next_status
end

-- 两个**强制主 Agent 门禁**：plan 批后才 develop，accept（执行方交付）核验后才 done。
-- 门禁是真实状态 + 动作约束（不是提示词）：status ∈ {plan_gate, accept} 时不能派活、
-- 不能跳过，必须由 gate.approve / gate.reject 显式推进（goal 也不能绕过）。
local GATE_STATES = { [R_PLAN_GATE] = "plan", [R_ACCEPT] = "accept" }

local function gate_kind(index)
  return GATE_STATES[req_status(index)]
end

-- 待门禁需求（canonical 序：需求 id 升序）；主 Agent 串行处理，不阻塞其他需求。
local function gate_next()
  for i = 1, req_count() do
    if gate_kind(i) ~= nil then
      return i
    end
  end
  return nil
end

local function gate_total()
  local total = 0
  for i = 1, req_count() do
    if gate_kind(i) ~= nil then
      total = total + 1
    end
  end
  return total
end

local function req_artifacts(index)
  return text_of(req_key(index, "artifacts"), "")
end

-- accept_gate 的证据硬约束：不接受「只写 summary 的完成自称」，必须有产物路径。
local function gate_evidence_problem(index, kind)
  if kind == "accept" and req_artifacts(index) == "" then
    return fill(TEXTS.beats.req_gate_no_evidence, { id = req_id(index) })
  end
  return nil
end

-- 门禁绑定 revision：待批期间需求被 amend → 旧门禁失效（stale），不得批准。
local function gate_rev_ok(index)
  return num(req_key(index, "gate_rev"), 0) == req_rev(index)
end

-- 流程面板：job 槽位 + 待验收提案 + 需求流水线分布（只读）。
local function req_panel_text()
  local parts = {}
  parts[#parts + 1] = fill(TEXTS.beats.req_panel_counts, {
    draft = tostring(req_tally(R_DRAFT)),
    analyzing = tostring(req_tally(R_ANALYZING) + req_tally(R_ANALYSIS_REVIEW)),
    planning = tostring(req_tally(R_PLANNING) + req_tally(R_PLAN_REVIEW)),
    gate = tostring(gate_total()),
    ready = tostring(req_tally(R_READY)),
    developing = tostring(req_tally(R_DEVELOPING) + req_tally(R_DEV_REVIEW)),
    done = tostring(req_tally(R_DONE)),
    total = tostring(req_count()),
  })
  parts[#parts + 1] = fill(TEXTS.beats.req_panel_jobs, {
    jobs = tostring(job_total()),
    cap = tostring(job_capacity()),
    pending = tostring(prop_pending_total()),
    conflicts = tostring(req_conflict_total()),
  })
  return join(parts, "｜")
end

-- 展示态标记（list / map 共用）。
local R_MARK = {
  [R_DRAFT] = "·",
  [R_ANALYZING] = "◐",
  [R_ANALYSIS_REVIEW] = "◑",
  [R_PLANNING] = "◇",
  [R_PLAN_REVIEW] = "◈",
  [R_PLAN_GATE] = "⊟",
  [R_READY] = "○",
  [R_DEVELOPING] = "▶",
  [R_DEV_REVIEW] = "◀",
  [R_ACCEPT] = "⊞",
  [R_DONE] = "✓",
  [R_SPLIT] = "⑂",
  [R_MERGED] = "⇄",
  [R_DROPPED] = "⊘",
  blocked = "⚠",
}

local function req_trace(index)
  local parts = {}
  local deps = req_deps(index)
  if #deps > 0 then
    parts[#parts + 1] = fill(TEXTS.beats.req_trace_deps, { ids = join(deps, ",") })
  end
  local origin = text_of(req_key(index, "origin"), "")
  if origin ~= "" then
    parts[#parts + 1] = fill(TEXTS.beats.req_trace_origin, { id = origin })
  end
  local status = req_status(index)
  if status == R_SPLIT then
    parts[#parts + 1] = fill(TEXTS.beats.req_trace_split, { ids = text_of(req_key(index, "children"), "") })
  elseif status == R_MERGED then
    parts[#parts + 1] = fill(TEXTS.beats.req_trace_merged, { id = text_of(req_key(index, "into"), "") })
  end
  local lane = req_lane(index)
  if lane > 0 then
    parts[#parts + 1] = fill(TEXTS.beats.req_trace_lane, { lane = "lane-" .. tostring(lane) })
  end
  local job = req_job(index)
  if job ~= "" then
    parts[#parts + 1] = fill(TEXTS.beats.req_trace_job, { job = job })
  end
  if req_rev(index) > 1 then
    parts[#parts + 1] = fill(TEXTS.beats.req_trace_rev, { rev = tostring(req_rev(index)) })
  end
  if req_conflict(index) ~= "" then
    parts[#parts + 1] = fill(TEXTS.beats.req_trace_conflict, {
      note = req_conflict(index),
      peer = req_conflict_peer(index),
    })
  elseif req_needs_amend(index) then
    parts[#parts + 1] = TEXTS.beats.req_trace_needs_amend
  end
  if req_stale(index) then
    parts[#parts + 1] = TEXTS.beats.req_trace_stale
  end
  return join(parts, "")
end

local function narrate_req_list()
  ygg.narrate(fill(TEXTS.beats.req_list_head, {
    panel = req_panel_text(),
    stage = stage(),
    lanes = lane_text(),
  }))
  if req_count() == 0 then
    ygg.narrate(TEXTS.beats.req_list_empty)
  else
    for i = 1, req_count() do
      local shown = req_display(i)
      ygg.narrate(fill(TEXTS.beats.req_list_row, {
        mark = R_MARK[shown] or "?",
        id = req_id(i),
        status = shown,
        title = req_title(i),
        trace = req_trace(i),
      }))
    end
  end
  -- 待验收提案（canonical 序；主 Agent 一次只验一条）
  local pending = prop_pending_total()
  if pending > 0 then
    ygg.narrate(fill(TEXTS.beats.req_list_props, { count = tostring(pending) }))
    for i = 1, prop_count() do
      if prop_status(i) == P_PENDING then
        ygg.narrate(fill(TEXTS.beats.req_prop_row, {
          id = prop_id(i),
          req = req_id(prop_req(i)),
          rev = tostring(prop_rev(i)),
          stage = prop_stage(i),
          summary = text_of(prop_key(i, "summary"), ""),
        }))
      end
    end
  end
  ygg.narrate(TEXTS.beats.req_list_legend)
end

-- ============================================================================
-- §地图 双层节点图：协调器（4 态）+ 需求流水线（8 态）+ job/泳道台账
-- 单源 = 下面两张节点/边表；不另加资源/Schema。
-- ============================================================================
local MAP_NODES = { STAGE.intake, STAGE.coordinate, STAGE.review, STAGE.done }

local MAP_EDGES = {
  { from = STAGE.intake, to = STAGE.coordinate, action = "task.intake" },
  { from = STAGE.coordinate, to = STAGE.review, action = "coordinate.submit", note = "map_note_all_done" },
  {
    from = STAGE.review,
    to = STAGE.coordinate,
    action = "verify.submit=false",
    note = "map_note_back",
  },
  { from = STAGE.review, to = STAGE.done, action = "review.note", note = "map_note_ending" },
}

local PIPELINE = {
  { node = "[draft]", action = "requirement.dispatch", target = "[analyzing]", note = "map_note_job" },
  { node = "[analyzing]", action = "proposal", target = "[analysis_review]", note = "map_note_proposal" },
  { node = "[analysis_review]", action = "proposal.accept", target = "[planning]", note = "map_note_accept" },
  { node = "[planning]", action = "requirement.dispatch", target = "[plan(job)]", note = "map_note_job" },
  { node = "[plan(job)]", action = "proposal", target = "[plan_review]", note = "map_note_proposal" },
  {
    node = "[plan_review]",
    action = "proposal.accept",
    target = "[plan_gate]",
    note = "map_note_gate",
  },
  {
    node = "[plan_gate]",
    action = "gate.approve",
    target = "[ready]",
    note = "map_note_gate_plan",
  },
  { node = "[ready]", action = "requirement.dispatch", target = "[developing·lane]", note = "map_note_lane" },
  { node = "[developing]", action = "proposal", target = "[dev_review]", note = "map_note_proposal" },
  {
    node = "[dev_review]",
    action = "proposal.accept",
    target = "[accept·执行方交付]",
    note = "map_note_deliver",
  },
  {
    node = "[accept]",
    action = "gate.approve",
    target = "[done]",
    note = "map_note_gate_accept",
  },
}

local MAP_LABEL_WIDTH = 0
for i = 1, #MAP_NODES do
  local width = #MAP_NODES[i] + 2
  if width > MAP_LABEL_WIDTH then
    MAP_LABEL_WIDTH = width
  end
end

local PIPE_LABEL_WIDTH = 0
for i = 1, #PIPELINE do
  local width = #PIPELINE[i].node
  if width > PIPE_LABEL_WIDTH then
    PIPE_LABEL_WIDTH = width
  end
end

local function pad(label, width)
  local out = label
  for _ = #label + 1, width do
    out = out .. " "
  end
  return out
end

local function out_edges(node)
  local out = {}
  for i = 1, #MAP_EDGES do
    if MAP_EDGES[i].from == node then
      out[#out + 1] = MAP_EDGES[i]
    end
  end
  return out
end

local function narrate_map(current)
  ygg.narrate(fill(TEXTS.beats.map_head, { stage = current, panel = req_panel_text() }))
  local blank = spaces(MAP_LABEL_WIDTH)
  for i = 1, #MAP_NODES do
    local node = MAP_NODES[i]
    local mark = spaces(2)
    if node == current then
      mark = TEXTS.beats.map_mark
    end
    local label = pad("[" .. node .. "]", MAP_LABEL_WIDTH)
    local edges = out_edges(node)
    if #edges == 0 then
      ygg.narrate(fill(TEXTS.beats.map_row_leaf, {
        mark = mark,
        node = label,
        branch = TEXTS.beats.map_branch_single,
        text = TEXTS.beats.map_terminal,
      }))
    end
    for j = 1, #edges do
      local edge = edges[j]
      local branch = TEXTS.beats.map_branch_single
      if #edges > 1 then
        branch = j == #edges and TEXTS.beats.map_branch_last or TEXTS.beats.map_branch_first
      end
      local lead = j == 1 and mark or spaces(2)
      local head = j == 1 and label or blank
      ygg.narrate(fill(TEXTS.beats.map_row, {
        mark = lead,
        node = head,
        branch = branch,
        action = edge.action,
        target = "[" .. edge.to .. "]",
        note = edge.note ~= nil and TEXTS.beats[edge.note] or "",
      }))
    end
  end
  ygg.narrate(TEXTS.beats.map_pipeline_head)
  for i = 1, #PIPELINE do
    local row = PIPELINE[i]
    ygg.narrate(fill(TEXTS.beats.map_row, {
      mark = spaces(2),
      node = pad(row.node, PIPE_LABEL_WIDTH),
      branch = TEXTS.beats.map_branch_single,
      action = row.action,
      target = row.target,
      note = TEXTS.beats[row.note],
    }))
  end
  ygg.narrate(fill(TEXTS.beats.map_jobs, {
    lanes = lane_text(),
    jobs = tostring(job_total()),
    cap = tostring(job_capacity()),
    pending = tostring(prop_pending_total()),
  }))
  narrate_lane_ledger()
  ygg.narrate(TEXTS.beats.map_foot)
end

-- ============================================================================
-- §指南 现场规则资源（guide.show 按 section id 取用；每阶段只提示一行）
-- ============================================================================
local SECTIONS = {}
local SECTION_IDS = {}
for i = 1, #PLAYBOOK.sections do
  local entry = PLAYBOOK.sections[i]
  SECTIONS[entry.id] = entry
  SECTION_IDS[#SECTION_IDS + 1] = entry.id
end

local STAGE_GUIDE = {
  intake = { "output" },
  coordinate = { "dispatch", "lanes", "integration" },
  review = { "gates", "evidence" },
  done = { "pointers" },
}

local function guide_section(st)
  local candidates = STAGE_GUIDE[st]
  if candidates == nil or #candidates == 0 then
    return PLAYBOOK.sections[1]
  end
  local section = SECTIONS[candidates[1]]
  if section == nil then
    return PLAYBOOK.sections[1]
  end
  return section
end

local MODALITY_MARK = {
  ["must"] = "［必须］",
  ["must-not"] = "［禁止］",
  ["should"] = "［应当］",
}

local function narrate_section(section)
  ygg.narrate(fill(TEXTS.beats.guide_head, {
    title = section.title,
    summary = section.summary,
    count = tostring(#section.rules),
  }))
  for i = 1, #section.rules do
    local rule = section.rules[i]
    ygg.narrate(fill(TEXTS.beats.guide_rule, {
      mark = MODALITY_MARK[rule.modality] or "",
      text = rule.text,
    }))
  end
  ygg.narrate(fill(TEXTS.beats.guide_tail, { id = section.id, total = tostring(#SECTION_IDS) }))
end

local function narrate_guide(id)
  local section = SECTIONS[id]
  if section == nil then
    ygg.narrate(fill(TEXTS.beats.guide_unknown, {
      id = tostring(id),
      ids = join(SECTION_IDS, "、"),
    }))
    return
  end
  narrate_section(section)
end

local function narrate_stage_guide(st)
  local section = guide_section(st)
  ygg.narrate(fill(TEXTS.beats.guide_hint, {
    title = section.title,
    summary = section.summary,
    count = tostring(#section.rules),
  }))
end

local function guide_item(st)
  local section = guide_section(st)
  return {
    text = fill(TEXTS.options.guide, { title = section.title, count = tostring(#section.rules) }),
    action = "guide.show",
    payload = section.id,
  }
end

local function map_item(st)
  return {
    text = fill(TEXTS.options.map, { stage = st }),
    action = "map.show",
    payload = "full",
  }
end

-- ============================================================================
-- §二级菜单页 纯展示层：不参与状态机，阶段一变即清空。
-- 顶层菜单只留「config」一个入口，三项设置进子页（收敛入口，不让三条 config.set
-- 平铺占住阶段菜单）。状态落 KV `ui.page` / `ui.page.stage`（纯展示态：不进摘要、
-- 不影响派活，也不改任何业务动作语义）。
-- ============================================================================
local CONFIG_UI = {}

function CONFIG_UI.page()
  local value = ygg.kv.get("ui.page")
  if type(value) ~= "string" then
    return ""
  end
  return value
end

function CONFIG_UI.page_stage()
  local value = ygg.kv.get("ui.page.stage")
  if type(value) ~= "string" then
    return ""
  end
  return value
end

function CONFIG_UI.set_page(page, st)
  local next_page = page or ""
  ygg.kv.set("ui.page", next_page)
  if next_page == "" then
    ygg.kv.set("ui.page.stage", "")
  else
    ygg.kv.set("ui.page.stage", st or stage())
  end
end

-- 子页是否正占着「当前阶段」的菜单
function CONFIG_UI.open(st)
  return CONFIG_UI.page() == "config" and CONFIG_UI.page_stage() == (st or stage())
end

-- 顶层唯一入口
function CONFIG_UI.item()
  return {
    text = TEXTS.options.config,
    action = "config.open",
    payload = "",
  }
end

function CONFIG_UI.close_item()
  return {
    text = TEXTS.options.config_close,
    action = "config.close",
    payload = "",
  }
end

function CONFIG_UI.play_item()
  local next_value = next_play_mode()
  return {
    text = fill(TEXTS.options.config_play, { mode = play_mode(), next = next_value }),
    action = "config.set",
    payload = "play.mode=" .. next_value,
  }
end

function CONFIG_UI.evolution_item()
  local next_value = next_mode()
  return {
    text = fill(TEXTS.options.config_evolution, { mode = mode(), next = next_value }),
    action = "config.set",
    payload = "evolution.mode=" .. next_value,
  }
end

function CONFIG_UI.gates_item()
  local current = strict_gates()
  return {
    text = fill(TEXTS.options.config_gates, { state = tostring(current), next = tostring(not current) }),
    action = "config.set",
    payload = "gates.require_all_green=" .. tostring(not current),
  }
end

function CONFIG_UI.page_items()
  return {
    CONFIG_UI.play_item(),
    CONFIG_UI.evolution_item(),
    CONFIG_UI.gates_item(),
    CONFIG_UI.close_item(),
  }
end

-- ============================================================================
-- §菜单 选项文本 ↔ 动作载荷的单一来源；每阶段第一条 = 主前进动作。
-- 需求动作（split / merge / amend / conflict / resolve）是主 Agent 的显式命令，
-- 不进常驻菜单（避免误触）；冲突时三条解决路径会出现在最前。
-- ============================================================================
local MENU = {}

local function req_list_item()
  return {
    text = fill(TEXTS.options.req_list, {
      pending = tostring(prop_pending_total()),
      open = tostring(req_open_total()),
    }),
    action = "requirement.list",
    payload = "full",
  }
end

local function req_add_item()
  return {
    text = TEXTS.options.req_add,
    action = "requirement.add",
    payload = TEXTS.beats.default_req_payload,
  }
end

-- 冲突解决三选项（只有真冲突对时出现；goal 会停下来等你选）
local function resolve_items()
  local first = nil
  local peer = ""
  for i = 1, req_count() do
    if req_conflict(i) ~= "" then
      first = i
      peer = req_conflict_peer(i)
      break
    end
  end
  if first == nil then
    return {}
  end
  local ids = req_id(first)
  if peer ~= "" then
    ids = ids .. "," .. peer
  end
  return {
    {
      text = fill(TEXTS.options.req_resolve_keep, { ids = ids }),
      action = "requirement.resolve",
      payload = ids .. "|keep_both",
    },
    {
      text = fill(TEXTS.options.req_resolve_first, { ids = ids }),
      action = "requirement.resolve",
      payload = ids .. "|keep_first",
    },
    {
      text = fill(TEXTS.options.req_resolve_revise, { ids = ids }),
      action = "requirement.resolve",
      payload = ids .. "|revise",
    },
  }
end

local function append_all(items, extra)
  for i = 1, #extra do
    items[#items + 1] = extra[i]
  end
  return items
end

-- 派活候选（按需求 id 升序，canonical）。
local function dispatch_candidates()
  local out = {}
  for i = 1, req_count() do
    if req_job_stage(i) ~= nil and req_job(i) == "" then
      if req_job_stage(i) ~= "development" or req_dev_ready(i) then
        out[#out + 1] = i
      end
    end
  end
  return out
end

local function dispatch_item()
  local candidates = dispatch_candidates()
  if #candidates == 0 then
    return nil
  end
  return {
    text = fill(TEXTS.options.dispatch, {
      count = tostring(#candidates),
      jobs = tostring(job_total()),
      cap = tostring(job_capacity()),
      id = req_id(candidates[1]),
    }),
    action = "requirement.dispatch",
    payload = "",
  }
end

local function accept_item()
  local next_prop = prop_next()
  if next_prop == nil then
    return nil
  end
  return {
    text = fill(TEXTS.options.accept, {
      id = prop_id(next_prop),
      req = req_id(prop_req(next_prop)),
      stage = prop_stage(next_prop),
      pending = tostring(prop_pending_total()),
    }),
    action = "proposal.accept",
    payload = prop_id(next_prop),
  }
end

-- 两个强制门禁的菜单入口（canonical 序；approve 是主前进动作之一）
local function gate_items()
  local index = gate_next()
  if index == nil then
    return {}
  end
  local kind = gate_kind(index)
  return {
    {
      text = fill(TEXTS.options.gate_approve, {
        id = req_id(index),
        kind = kind,
        gates = tostring(gate_total()),
      }),
      action = "gate.approve",
      payload = req_id(index),
    },
    {
      text = fill(TEXTS.options.gate_reject, { id = req_id(index), kind = kind }),
      action = "gate.reject",
      payload = req_id(index) .. "|" .. TEXTS.beats.req_gate_reject_default,
    },
  }
end

MENU[STAGE.intake] = function()
  local items = append_all({}, resolve_items())
  items[#items + 1] = {
    text = TEXTS.options.intake,
    action = "task.intake",
    payload = TEXTS.beats.default_task_title .. "|" .. TEXTS.beats.default_task_desc,
  }
  items[#items + 1] = guide_item(STAGE.intake)
  items[#items + 1] = map_item(STAGE.intake)
  items[#items + 1] = req_list_item()
  items[#items + 1] = req_add_item()
  items[#items + 1] = CONFIG_UI.item()
  return items
end

MENU[STAGE.coordinate] = function()
  local items = append_all({}, resolve_items())
  -- 主前进动作优先级：验收提案 > 过门禁 > 派活 > 收口进 review（canonical 串行）。
  local accept = accept_item()
  local gates = gate_items()
  local dispatch = dispatch_item()
  if accept ~= nil then
    items[#items + 1] = accept
  elseif #gates > 0 then
    items = append_all(items, gates)
  elseif dispatch ~= nil then
    items[#items + 1] = dispatch
  else
    items[#items + 1] = {
      text = TEXTS.options.coordinate_submit,
      action = "coordinate.submit",
      payload = "",
    }
  end
  items[#items + 1] = guide_item(STAGE.coordinate)
  items[#items + 1] = map_item(STAGE.coordinate)
  items[#items + 1] = req_list_item()
  items[#items + 1] = req_add_item()
  items[#items + 1] = CONFIG_UI.item()
  return items
end

MENU[STAGE.review] = function()
  local items = append_all({}, resolve_items())
  items[#items + 1] = { text = TEXTS.options.verify_green, action = "verify.submit", payload = "true" }
  items[#items + 1] = {
    text = TEXTS.options.verify_fail,
    action = "verify.submit",
    payload = "all_green=false;report=" .. TEXTS.beats.default_gate_report,
  }
  items[#items + 1] = guide_item(STAGE.review)
  items[#items + 1] = map_item(STAGE.review)
  items[#items + 1] = req_list_item()
  items[#items + 1] = req_add_item()
  items[#items + 1] = CONFIG_UI.item()
  return items
end

local function menu(st)
  -- 二级页生效时只返回子页项（渲染与选择都走这里，顶层项暂时让位）。
  if CONFIG_UI.open(st) then
    return CONFIG_UI.page_items()
  end
  local builder = MENU[st]
  if builder == nil then
    return nil
  end
  return builder()
end

local function option_texts(items)
  local out = {}
  for i = 1, #items do
    out[i] = items[i].text
  end
  return out
end

-- ============================================================================
-- §渲染 回执 = 处境 + 双层进度（协调器阶段 + 需求流水线）+ 指导 + pendingChoice
-- 渲染只读 KV，不写状态（同一步重渲染不漂移）。
-- ============================================================================
local RENDER = {}
-- 前向声明：RENDER 里要用 prompt_vars（定义在它之后）。
local prompt_vars

local function stage_prompt(st)
  if play_mode() == "goal" then
    return TEXTS.beats[st .. "_goal"] or TEXTS.beats[st .. "_guide"]
  end
  return TEXTS.beats[st .. "_guide"]
end

local function narrate_play_mode()
  local hint = TEXTS.beats.play_hint_guide
  if play_mode() == "goal" then
    hint = TEXTS.beats.play_hint_goal
  end
  ygg.narrate(fill(TEXTS.beats.play_receipt, { play = play_mode(), hint = hint }))
end

local function evolution_receipt()
  local current = mode()
  local used = num("evolution.used", 0)
  local cap = max_per_run()
  local paths = CONFIG.evolution and CONFIG.evolution.allow_paths
  local paths_text = type(paths) == "table" and join(paths, "、") or TEXTS.beats.paths_none
  if current == "auto" then
    if used >= cap then
      return fill(TEXTS.beats.evolution_capped, { paths = paths_text, cap = tostring(cap) })
    end
    return TEXTS.beats.evolution_auto_pending
  elseif current == "suggest" then
    return fill(TEXTS.beats.evolution_suggest, { paths = paths_text })
  end
  return TEXTS.beats.evolution_off
end

local function narrate_req_panel()
  ygg.narrate(req_panel_text())
  ygg.narrate(fill(TEXTS.beats.req_lane_panel, {
    lanes = lane_text(),
    jobs = tostring(job_total()),
    cap = tostring(job_capacity()),
  }))
end

RENDER[STAGE.intake] = function()
  local current = stage()
  ygg.narrate(fill(TEXTS.scenes.intake, { stage = current, gates = tostring(strict_gates()) }))
  ygg.narrate(fill(stage_prompt(STAGE.intake), prompt_vars()))
  if req_count() > 0 then
    narrate_req_panel()
  end
  narrate_stage_guide(current)
  ygg.choice(TEXTS.prompts.intake, option_texts(menu(current)))
end

RENDER[STAGE.coordinate] = function()
  local current = stage()
  ygg.narrate(fill(TEXTS.scenes.coordinate, {
    title = task_title(),
    description = task_desc(),
    count = tostring(req_count()),
    open = tostring(req_open_total()),
    pending = tostring(prop_pending_total()),
  }))
  ygg.narrate(fill(stage_prompt(STAGE.coordinate), prompt_vars()))
  narrate_req_panel()
  ygg.narrate(evolution_receipt())
  narrate_stage_guide(current)
  ygg.choice(TEXTS.prompts.coordinate, option_texts(menu(current)))
end

RENDER[STAGE.review] = function()
  local current = stage()
  ygg.narrate(fill(TEXTS.scenes.review, {
    title = task_title(),
    count = tostring(req_count()),
    done = tostring(req_tally(R_DONE)),
    gates = tostring(strict_gates()),
  }))
  ygg.narrate(TEXTS.beats.secret_redline)
  ygg.narrate(evolution_receipt())
  narrate_req_panel()
  narrate_stage_guide(current)
  ygg.choice(TEXTS.prompts.review, option_texts(menu(current)))
end

RENDER[STAGE.done] = function()
  ygg.narrate(fill(TEXTS.endings.done, {
    title = task_title(),
    count = tostring(req_count()),
    done = tostring(req_tally(R_DONE)),
    score = tostring(num("review.score", TEXTS.tuning.default_score)),
    lesson = text_of("review.lesson", TEXTS.beats.default_lesson),
  }))
  if play_mode() == "goal" then
    ygg.narrate(TEXTS.beats.play_done_goal)
  end
end

local function render()
  local current = stage()
  -- 二级页不跨阶段存活：阶段一变就清空（子页是当前处境的入口，不是全局抽屉）。
  if CONFIG_UI.page() ~= "" and CONFIG_UI.page_stage() ~= current then
    CONFIG_UI.set_page("")
  end
  narrate_play_mode()
  local renderer = RENDER[current]
  if renderer == nil then
    ygg.narrate(fill(TEXTS.beats.unknown_stage, { stage = tostring(current) }))
    renderer = RENDER[STAGE.intake]
  end
  renderer()
end

-- ============================================================================
-- §摘要 回执标准字段（宿主唯一的语义判定面）
--
-- 只读动作 `agent.summary` 走内核的**结构化回执通道**：`ygg.meta("summary", {...})`
-- 把结构化数据投进回执信封的 `meta` 键，与人读叙事完全分离——不再拼
-- `【摘要】<JSON>` 这类「前缀 + 字符串」契约（Mod 在叙事里写同样前缀不会被误判，
-- 宿主也不再从人读文本里剖数据）。宿主据此判定「何时收口 / 有无冲突 /
-- 有无待验收提案 / 是否有 job 空等」，**不读任何 Mod 私有 KV 键**
-- （ending、fsm.stage、verify.passed、prop.*、req.*）——换 Mod 或改内部键名
-- 不再需要改宿主。
--
-- 字段：stage（展示用）/ ending / verifyPassed / nextEndsRun /
--       pendingProposals / requirementCount / conflicts[{id,peer,note}] /
--       awaiting{req,rev,stage}（无在跑 job 时该键缺失）
--
-- 值面纪律（内核 `ygg.meta` 把关）：标量 / 稠密序列 / 字符串键表，非有限值拒绝。
-- 读的都是本文档已有的只读访问器：不改业务逻辑、不改 KV 键、不改动作语义。
-- 旧宿主不认得该动作 → 只会当一条普通叙事看（本动作不 narrate 摘要行）。
-- ============================================================================
-- 在跑的 job 阶段（与 req_job 写入的三个值同源）。
local SUMMARY_JOB_STAGES = { analysis = true, plan = true, development = true }

-- 计数归一：进 meta 的计数必须是整数（宿主按整型读；字符串/浮点都会读成 0）。
local function summary_int(value)
  local number = tonumber(value) or 0
  if number < 0 then
    number = 0
  end
  return math.floor(number)
end

local function summary_conflicts()
  local out = {}
  for i = 1, req_count() do
    local note = req_conflict(i)
    if note ~= "" then
      out[#out + 1] = { id = req_id(i), peer = req_conflict_peer(i), note = note }
    end
  end
  return out
end

-- 首个在跑 job（无则 nil → meta 里不出现该键，宿主读成「无空等」）。
local function summary_awaiting()
  for i = 1, req_count() do
    local job = req_job(i)
    if SUMMARY_JOB_STAGES[job] == true then
      return { req = req_id(i), rev = summary_int(req_rev(i)), stage = job }
    end
  end
  return nil
end

-- 「下一步就是收口动作」= 当前阶段的菜单首项是 review.note（阶段名不进宿主）。
local function summary_next_ends_run()
  -- 只看阶段菜单（MENU 本体）：二级页打开时 pendingChoice 是子页（首项 config.set），
  -- 不能因此把自己报成「不结局」——收口判定不能被展示层带偏。
  local builder = MENU[stage()]
  if builder == nil then
    return false
  end
  local items = builder()
  if items == nil or #items == 0 then
    return false
  end
  return items[1].action == "review.note"
end

local function summary_fields()
  local fields = {
    stage = stage(),
    ending = ygg.kv.get("ending") == STAGE.done,
    verifyPassed = ygg.kv.get("verify.passed") == true,
    nextEndsRun = summary_next_ends_run(),
    pendingProposals = summary_int(prop_pending_total()),
    requirementCount = summary_int(req_count()),
    conflicts = summary_conflicts(),
  }
  local awaiting = summary_awaiting()
  if awaiting ~= nil then
    fields.awaiting = awaiting
  end
  return fields
end

-- 只读动作：只投结构化 meta（不 narrate 摘要、不推进状态、不改 KV）+ 原地重渲染
-- （pendingChoice 与上一步一致）。
ygg.on("agent.summary", function()
  ygg.meta("summary", summary_fields())
  render()
end)

-- 提示词占位符（各阶段共用超集；fill 只替换模板里出现的键）。
prompt_vars = function()
  return {
    engine = tostring(ygg.engine_version),
    stage = stage(),
    play = play_mode(),
    mode = mode(),
    gates = tostring(strict_gates()),
    count = tostring(req_count()),
    open = tostring(req_open_total()),
    pending = tostring(prop_pending_total()),
    jobs = tostring(job_total()),
    cap = tostring(job_capacity()),
    score = tostring(TEXTS.tuning.default_score),
  }
end

-- 提示词占位符的赋值已完成（前向声明见 §渲染 上方）。

-- ============================================================================
-- §推进 转场 + 渲染的唯二出口；拒绝路径 = narrate + 原地重渲染。
-- ============================================================================
local function advance(target)
  -- 转场即离开二级页（子页跟着阶段走，不跨阶段）。
  CONFIG_UI.set_page("")
  if target ~= stage() then
    ygg.emit("stage.advance", target)
  end
  ygg.narrative["goto"]("node:render")
end

local function allowed_list(current)
  -- 只看阶段菜单本体：拒绝文案里不能列出 config 子页的项（子页是展示层，不是阶段能力）。
  local builder = MENU[current]
  local items = nil
  if builder ~= nil then
    items = builder()
  end
  if items == nil then
    return TEXTS.beats.allowed_none
  end
  local names = {}
  for i = 1, #items do
    if not contains(names, items[i].action) then
      names[#names + 1] = items[i].action
    end
  end
  return join(names, "、")
end

local function refuse(template, action, detail)
  local current = stage()
  ygg.narrate(fill(template, {
    action = tostring(action),
    stage = current,
    allowed = allowed_list(current),
    detail = detail or TEXTS.beats.rejected_default,
  }))
  render()
end

local function reject(action, detail)
  return refuse(TEXTS.beats.rejected, action, detail)
end

local function reject_payload(action, detail)
  return refuse(TEXTS.beats.rejected_payload, action, detail)
end

-- ============================================================================
-- §人类动作门禁：guide 模式下「会产生通过效果」的动作只认菜单选择，单发直派一律拒绝
--
-- 引擎不区分「谁派发的」（`--event 动作` 与 TTY 按键走同一套通道），所以本层把这件事
-- 从隐式变成显式：guide 下这些动作必须经 ygg.choice_selected 路由进入；goal 也走
-- choice_selected，但是否派发由外部决策命令决定，不会默认批准。
--
-- 人类动作 = 会产生「通过」效果的动作（自批风险）。打回类（gate.reject /
-- proposal.reject）故意不在内：打回不产生任何通过效果，自打回只会让自己返工，
-- 不构成伪造验收；而它是唯一的纠错通道，堵死它等于堵死返工路径（菜单本就没有带
-- 理由的 reject 项）。与多仓参照实现的差异：那边的 integration.* / release.* /
-- lane.bootstrap 在本 Mod 没有对应动作，集合只留五条；`config.set play.mode`
-- 单独门禁（切模式改变「谁在派发」，是自批链条的第一环）。
-- ============================================================================
local HUMAN = {}

HUMAN.ACTIONS = {
  ["proposal.accept"] = true,
  ["gate.approve"] = true,
  ["coordinate.submit"] = true,
  ["verify.submit"] = true,
  ["review.note"] = true,
}

-- 一次性票据：`ygg.emit` 是**入队**而非同步派发，派生事件到达时菜单 handler 早已返回，
-- 所以不能用调用栈深度表达「这一步来自菜单」。菜单路由在 emit 前盖章，
-- 对应 handler 取证即消费；非人类动作不盖章，票据不会泄到下一个直派。
HUMAN.ticket = nil

function HUMAN.needs(label)
  if play_mode() ~= "guide" then
    return true
  end
  if HUMAN.ticket == label then
    HUMAN.ticket = nil
    return true
  end
  return refuse(TEXTS.beats.guide_human_blocked, label)
end

function HUMAN.gated(action)
  if HUMAN.ACTIONS[action] ~= true then
    return true
  end
  return HUMAN.needs(action)
end

-- 菜单项的 action 与「人类动作」的标签不一定同名（`config.set play.mode=…`），
-- 盖章与取证必须用同一套标签，否则人从菜单选了也会被拒。
function HUMAN.label(action, payload)
  if HUMAN.ACTIONS[action] == true then
    return action
  end
  if action == "config.set" and type(payload) == "string" then
    local head = trim(payload)
    if head:sub(1, 9) == "play.mode" then
      return "config.set play.mode"
    end
  end
  return nil
end

-- ============================================================================
-- §动作解析 wire format：内核 payload 只有四标量，复合载荷压成单字符串。
-- ============================================================================
local function probe_payload_ok(payload)
  if payload == nil then
    return true
  end
  if type(payload) == "number" then
    return payload == 0
  end
  if type(payload) == "string" then
    local raw = trim(payload)
    return raw == "" or raw == "0" or raw == "full"
  end
  return false
end

local function valid_ext(ext)
  if type(ext) ~= "string" then
    return false
  end
  local raw = trim(ext)
  if raw == "" or #raw > 64 then
    return false
  end
  return raw:find("%s") == nil
end

local function parse_intake(payload)
  if type(payload) ~= "string" then
    return nil, nil, fill(TEXTS.beats.bad_payload, { action = "task.intake" })
  end
  local raw = trim(payload)
  if raw == "" then
    return TEXTS.beats.default_task_title, TEXTS.beats.default_task_desc, nil
  end
  local title, description = cut(raw, "|")
  title = trim(title)
  description = trim(description or "")
  if title == "" then
    title = TEXTS.beats.default_task_title
  end
  if description == "" then
    description = TEXTS.beats.default_task_desc
  end
  return title, description, nil
end

local function parse_deps(raw)
  local out = {}
  if type(raw) ~= "string" then
    return out
  end
  local raw_text = trim(raw)
  if raw_text == "" or raw_text == "-" then
    return out
  end
  for _, part in ipairs(split(raw_text, ",")) do
    local piece = trim(part)
    if piece ~= "" then
      out[#out + 1] = piece
    end
  end
  return out
end

-- requirement.add：title|description[|external_id[|depends_on]]
local function parse_requirement(payload)
  if type(payload) ~= "string" then
    return nil, nil, nil, nil, fill(TEXTS.beats.bad_payload, { action = "requirement.add" })
  end
  local raw = trim(payload)
  if raw == "" then
    return TEXTS.beats.default_req_title, TEXTS.beats.default_req_desc, "", {}, nil
  end
  local parts = split(raw, "|")
  local title = trim(parts[1] or "")
  local description = trim(parts[2] or "")
  local ext = ""
  local dep_raw = ""
  if #parts >= 3 then
    if valid_ext(parts[3]) then
      ext = trim(parts[3])
      dep_raw = trim(parts[4] or "")
    else
      description = trim(description .. "|" .. trim(parts[3]))
    end
  end
  if title == "" then
    title = TEXTS.beats.default_req_title
  end
  if description == "" then
    description = TEXTS.beats.default_req_desc
  end
  return title, description, ext, parse_deps(dep_raw), nil
end

-- 载荷末尾可选 `|@ext=<external_id>`：CLI inbox 注入的幂等键（手写单发可省略）。
local function strip_ext(payload)
  if type(payload) ~= "string" then
    return payload, ""
  end
  local marker = "|@ext="
  local index = payload:find(marker, 1, true)
  if index == nil then
    return payload, ""
  end
  return payload:sub(1, index - 1), trim(payload:sub(index + #marker))
end

-- 异步重放：同一个 external_id 已经成功落地 → 幂等返回（不重复 amend / split / …）。
-- cursor 崩溃安全的最后一道兵底：重启后重放同一行不会把状态推两次。
local function inbox_replay(ext)
  if type(ext) ~= "string" or ext == "" then
    return false
  end
  if text_of("inbox.last", "") ~= ext then
    return false
  end
  ygg.narrate(fill(TEXTS.beats.req_replayed_op, { ext = ext }))
  return true
end

local function inbox_mark(ext)
  if type(ext) == "string" and ext ~= "" then
    ygg.kv.set("inbox.last", ext)
  end
end

-- proposal.submit：req_id|revision|stage|summary|artifacts[|changes[|external_id]]
local function parse_proposal(payload)
  if type(payload) ~= "string" then
    return nil, fill(TEXTS.beats.bad_payload, { action = "proposal.submit" })
  end
  local parts = split(trim(payload), "|")
  if #parts < 4 then
    return nil, TEXTS.beats.req_prop_usage
  end
  local req_id_text = trim(parts[1])
  local rev = tonumber(trim(parts[2] or ""))
  local stage_name = trim(parts[3] or "")
  if rev == nil or not contains({ "analysis", "plan", "development" }, stage_name) then
    return nil, TEXTS.beats.req_prop_usage
  end
  return {
    req = req_id_text,
    rev = math.floor(rev),
    stage = stage_name,
    summary = trim(parts[4] or ""),
    artifacts = trim(parts[5] or ""),
    changes = trim(parts[6] or ""),
    ext = trim(parts[7] or ""),
  }, nil
end

local function parse_split(payload)
  if type(payload) ~= "string" then
    return nil, nil, fill(TEXTS.beats.bad_payload, { action = "requirement.split" })
  end
  local parts = split(trim(payload), "|")
  local parent = trim(parts[1] or "")
  local children = {}
  for i = 2, #parts do
    local title = trim(parts[i])
    if title ~= "" then
      children[#children + 1] = title
    end
  end
  if parent == "" or #children < 2 then
    return nil, nil, TEXTS.beats.req_split_usage
  end
  return parent, children, nil
end

local function parse_merge(payload)
  if type(payload) ~= "string" then
    return nil, nil, nil, fill(TEXTS.beats.bad_payload, { action = "requirement.merge" })
  end
  local parts = split(trim(payload), "|")
  local ids = {}
  for _, part in ipairs(split(trim(parts[1] or ""), ",")) do
    local piece = trim(part)
    if piece ~= "" then
      ids[#ids + 1] = piece
    end
  end
  if #ids < 2 then
    return nil, nil, nil, TEXTS.beats.req_merge_usage
  end
  return ids, trim(parts[2] or ""), trim(parts[3] or ""), nil
end

local function parse_amend(payload)
  if type(payload) ~= "string" then
    return nil, nil, nil, nil, fill(TEXTS.beats.bad_payload, { action = "requirement.amend" })
  end
  local parts = split(trim(payload), "|")
  local id = trim(parts[1] or "")
  if id == "" then
    return nil, nil, nil, nil, TEXTS.beats.req_amend_usage
  end
  return id, trim(parts[2] or ""), trim(parts[3] or ""), trim(parts[4] or ""), nil
end

local function parse_conflict(payload)
  if type(payload) ~= "string" then
    return nil, nil, fill(TEXTS.beats.bad_payload, { action = "requirement.conflict" })
  end
  local parts = split(trim(payload), "|")
  local ids = {}
  for _, part in ipairs(split(trim(parts[1] or ""), ",")) do
    local piece = trim(part)
    if piece ~= "" then
      ids[#ids + 1] = piece
    end
  end
  if #ids ~= 2 then
    return nil, nil, TEXTS.beats.req_conflict_usage
  end
  local note = trim(parts[2] or "")
  if note == "" then
    note = TEXTS.beats.req_conflict_default
  end
  return ids, note, nil
end

local RESOLVE_MODES = { "keep_both", "keep_first", "revise" }

local function parse_resolve(payload)
  if type(payload) ~= "string" then
    return nil, nil, nil, fill(TEXTS.beats.bad_payload, { action = "requirement.resolve" })
  end
  local parts = split(trim(payload), "|")
  local ids = {}
  for _, part in ipairs(split(trim(parts[1] or ""), ",")) do
    local piece = trim(part)
    if piece ~= "" then
      ids[#ids + 1] = piece
    end
  end
  local mode = trim(parts[2] or "")
  if #ids ~= 2 or not contains(RESOLVE_MODES, mode) then
    return nil, nil, nil, fill(TEXTS.beats.req_resolve_usage, { modes = join(RESOLVE_MODES, " / ") })
  end
  return ids, mode, trim(parts[3] or ""), nil
end

-- ============================================================================
-- §门禁 收口判据（所有验收/收口动作共用同一套）
-- ============================================================================
local function gate_problem()
  if prop_pending_total() > 0 then
    return fill(TEXTS.beats.req_gate_props, { pending = tostring(prop_pending_total()) })
  end
  if job_total() > 0 then
    return fill(TEXTS.beats.req_gate_jobs, { jobs = tostring(job_total()), cap = tostring(job_capacity()) })
  end
  if req_conflict_total() > 0 then
    return fill(TEXTS.beats.req_gate_conflicts, { conflicts = tostring(req_conflict_total()) })
  end
  if not req_all_done() then
    return fill(TEXTS.beats.req_gate_open, { open = tostring(req_open_total()) })
  end
  return nil
end

-- review 阶段的菜单（verify.passed 后主前进动作换成 review.note）
MENU[STAGE.review] = function()
  local items = append_all({}, resolve_items())
  if ygg.kv.get("verify.passed") == true then
    items[#items + 1] = {
      text = TEXTS.options.review_note,
      action = "review.note",
      payload = TEXTS.beats.default_lesson .. "|" .. tostring(TEXTS.tuning.default_score),
    }
  else
    items[#items + 1] = { text = TEXTS.options.verify_green, action = "verify.submit", payload = "true" }
  end
  items[#items + 1] = {
    text = TEXTS.options.verify_fail,
    action = "verify.submit",
    payload = "all_green=false;report=" .. TEXTS.beats.default_gate_report,
  }
  items[#items + 1] = guide_item(STAGE.review)
  items[#items + 1] = map_item(STAGE.review)
  items[#items + 1] = req_list_item()
  items[#items + 1] = req_add_item()
  items[#items + 1] = CONFIG_UI.item()
  return items
end

-- ============================================================================
-- §动作 handler
-- ============================================================================

-- 释放 job / 泳道（验收、拒绝、拆并都用这一处，避免漏释放）。
local function req_release(index)
  ygg.kv.set(req_key(index, "job"), "")
  LANES.release(index)
end

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

-- ============================================================================
-- §泳道动作 泳道账本的人工入口
--
-- 两个都不进 HUMAN.ACTIONS：不产生「通过」效果（只记账本），也没有自批风险。
-- 常规路径不用调它们：挂牌由 requirement.dispatch 同事务完成（branch/base/clean
-- 留空 = 未断言），摘牌走 req_release / 交付晋升。它们的用途是「主 Agent 已建好
-- worktree 并验过脏树」时把 branch / base / clean 记进账本，让 map.show 的账本
-- 对得上物理事实。
--
-- **mod 不能跑 git**：`clean` 只能是主 Agent 的判断结果——它在 lane.claim 前
-- 已跑过 `git status --porcelain`（除 `.lane` 外必须为空）并把结论写成 "1"。
-- ============================================================================

-- 解析（挂在 LANES 上而不新增顶层 local：本 chunk 的 local 名额已接近 Lua 上限）
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

-- lane.claim：泳道号|需求id|分支|base|clean
-- 预挂牌（需求还没进开发）与补记（已开发中，补充 branch/base）都走这一个入口。
ygg.on("lane.claim", function(payload)
  if stage() == STAGE.done then
    return reject("lane.claim", TEXTS.beats.req_after_done)
  end
  local spec, problem = LANES.parse_claim(payload)
  if problem ~= nil then
    return reject_payload("lane.claim", problem)
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

-- lane.release：泳道号
ygg.on("lane.release", function(payload)
  if stage() == STAGE.done then
    return reject("lane.release", TEXTS.beats.req_after_done)
  end
  local k, problem = LANES.parse_ref(payload)
  if problem ~= nil then
    return reject_payload("lane.release", problem)
  end
  local holder = LANES.req_of(k)
  LANES.clear(k)
  local freed = TEXTS.beats.lane_release_idle
  local index = req_index_of(holder)
  if index ~= nil and req_lane(index) == k then
    ygg.kv.set(req_key(index, "lane"), 0)
    freed = req_id(index)
  elseif holder ~= "" then
    freed = holder
  end
  ygg.narrate(fill(TEXTS.beats.lane_released, { k = tostring(k), id = freed }))
  render()
end)

-- 提案提交（subagent 的唯一入口；不直接改需求图）
ygg.on("proposal.submit", function(payload)
  if stage() == STAGE.done then
    return reject("proposal.submit", TEXTS.beats.req_after_done)
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
  local status = req_status(index)
  -- 每个阶段两态可收提案：「工作中」（首次）与该阶段的 review（**重定基**）。
  -- review 态只在「本需求没有待验收提案」时收（旧提案已 stale / 被拒），
  -- 有正常待验收提案时绝不覆盖（必须先 proposal.accept / reject）。
  local slots = {
    analysis = { working = R_ANALYZING, review = R_ANALYSIS_REVIEW },
    plan = { working = R_PLANNING, review = R_PLAN_REVIEW },
    development = { working = R_DEVELOPING, review = R_DEV_REVIEW },
  }
  local slot = slots[spec.stage]
  if status ~= slot.working and status ~= slot.review then
    return reject_payload("proposal.submit", fill(TEXTS.beats.req_prop_stage_mismatch, {
      id = req_id(index),
      stage = spec.stage,
      status = status,
    }))
  end
  if status == slot.review then
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
    -- 重定基：旧提案已作废，清掉需求的 stale 标记（本次提案重新起算）。
    ygg.kv.set(req_key(index, "stale"), false)
  end
  local summarize = spec.summary
  if summarize == "" then
    summarize = TEXTS.beats.req_prop_no_summary
  end
  local prop_index = prop_append(index, spec.rev, spec.stage, summarize, spec.artifacts, spec.ext, spec.changes)
  local review_status = {
    analysis = R_ANALYSIS_REVIEW,
    plan = R_PLAN_REVIEW,
    development = R_DEV_REVIEW,
  }
  ygg.kv.set(req_key(index, "status"), review_status[spec.stage])
  ygg.kv.set(req_key(index, "job"), "")
  ygg.narrate(fill(TEXTS.beats.req_prop_ok, {
    id = prop_id(prop_index),
    req = req_id(index),
    stage = spec.stage,
    rev = tostring(spec.rev),
    summary = summarize,
    pending = tostring(prop_pending_total()),
  }))
  render()
end)

-- 提案验收（主 Agent 串行；一次只处理一条需求）
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

ygg.on("proposal.accept", function(payload)
  if not HUMAN.gated("proposal.accept") then
    return
  end
  if stage() == STAGE.done then
    return reject("proposal.accept", TEXTS.beats.req_after_done)
  end
  local prop_index = pick_prop(payload, true)
  if prop_index == nil then
    return reject_payload("proposal.accept", TEXTS.beats.req_prop_none)
  end
  local index = prop_req(prop_index)
  local stage_name = prop_stage(prop_index)
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
  ygg.kv.set(req_key(index, "summary"), text_of(prop_key(prop_index, "summary"), ""))
  ygg.kv.set(req_key(index, "artifacts"), text_of(prop_key(prop_index, "artifacts"), ""))
  ygg.kv.set(prop_key(prop_index, "status"), P_ACCEPTED)
  local change_note = apply_changes(index, text_of(prop_key(prop_index, "changes"), ""))
  req_release(index)
  local next_status = req_advance_after_accept(index, stage_name)
  -- 门禁绑定 revision（待批期间 amend → stale，不得批准）
  if next_status == R_PLAN_GATE or next_status == R_ACCEPT then
    ygg.kv.set(req_key(index, "gate_rev"), req_rev(index))
  end
  req_push_history(index, fill(TEXTS.beats.req_accept_hist, { id = prop_id(prop_index), stage = stage_name }))
  req_sync_conflicts()
  ygg.narrate(fill(TEXTS.beats.req_accept_ok, {
    id = prop_id(prop_index),
    req = req_id(index),
    stage = stage_name,
    status = tostring(next_status or req_status(index)),
    change = change_note,
    pending = tostring(prop_pending_total()),
  }))
  render()
end)

ygg.on("proposal.reject", function(payload)
  if stage() == STAGE.done then
    return reject("proposal.reject", TEXTS.beats.req_after_done)
  end
  local target, reason = cut(trim(payload), "|")
  local prop_index = pick_prop(target, true)
  if prop_index == nil then
    return reject_payload("proposal.reject", TEXTS.beats.req_prop_none)
  end
  local index = prop_req(prop_index)
  local stage_name = prop_stage(prop_index)
  ygg.kv.set(prop_key(prop_index, "status"), P_REJECTED)
  local back = { analysis = R_DRAFT, plan = R_PLANNING, development = R_READY }
  req_release(index)
  ygg.kv.set(req_key(index, "status"), back[stage_name] or R_DRAFT)
  req_push_history(index, fill(TEXTS.beats.req_reject_hist, { id = prop_id(prop_index) }))
  ygg.narrate(fill(TEXTS.beats.req_reject_ok, {
    id = prop_id(prop_index),
    req = req_id(index),
    status = req_status(index),
    reason = trim(reason or "") ~= "" and trim(reason) or TEXTS.beats.req_reject_default,
  }))
  render()
end)

ygg.on("coordinate.submit", function(payload)
  if not HUMAN.gated("coordinate.submit") then
    return
  end
  local current = stage()
  if current ~= STAGE.coordinate then
    return reject("coordinate.submit", fill(TEXTS.beats.stage_expected, { expected = STAGE.coordinate }))
  end
  if not probe_payload_ok(payload) then
    return reject_payload("coordinate.submit", fill(TEXTS.beats.probe_rejected, {
      action = "coordinate.submit",
      got = tostring(payload),
      usage = TEXTS.beats.coordinate_usage,
    }))
  end
  local problem = gate_problem()
  if problem ~= nil then
    return refuse(TEXTS.beats.req_wait_rejected, "coordinate.submit", problem)
  end
  ygg.kv.set("verify.passed", false)
  ygg.narrate(fill(TEXTS.beats.coordinate_ok, { count = tostring(req_count()) }))
  advance(STAGE.review)
end)

ygg.on("verify.submit", function(payload)
  if not HUMAN.gated("verify.submit") then
    return
  end
  local current = stage()
  if current ~= STAGE.review then
    return reject("verify.submit", fill(TEXTS.beats.stage_expected, { expected = STAGE.review }))
  end
  local all_green = true
  local report = ""
  if type(payload) == "boolean" then
    all_green = payload
  elseif type(payload) == "string" then
    local raw = trim(payload)
    if raw == "false" then
      all_green = false
    elseif raw ~= "true" then
      local parsed = nil
      for _, segment in ipairs(split(raw, ";")) do
        local key, value = cut(trim(segment), "=")
        key = trim(key)
        value = trim(value or "")
        if key == "all_green" then
          parsed = value == "true"
        elseif key == "report" then
          report = value
        end
      end
      if parsed == nil then
        return reject_payload("verify.submit", fill(TEXTS.beats.bad_payload, { action = "verify.submit" }))
      end
      all_green = parsed
    end
  else
    return reject_payload("verify.submit", fill(TEXTS.beats.bad_payload, { action = "verify.submit" }))
  end
  if all_green then
    local problem = gate_problem()
    if problem ~= nil then
      ygg.narrate(fill(TEXTS.beats.req_gate_block, { action = "verify.submit=true", detail = problem }))
      render()
      return
    end
    ygg.kv.set("verify.passed", true)
    ygg.kv.set("verify.last", TEXTS.beats.verify_green_record)
    ygg.narrate(fill(TEXTS.beats.verify_ok, { count = tostring(req_count()) }))
    render()
    return
  end
  ygg.kv.set("verify.passed", false)
  local failures = num("verify.failures", 0) + 1
  ygg.kv.set("verify.failures", failures)
  ygg.kv.set("verify.last", report ~= "" and report or TEXTS.beats.verify_failed_record)
  if strict_gates() then
    ygg.narrate(fill(TEXTS.beats.verify_fail, { failures = tostring(failures) }))
    advance(STAGE.coordinate)
  else
    ygg.narrate(fill(TEXTS.beats.gate_bypass, { failures = tostring(failures) }))
    ygg.kv.set("verify.passed", true)
    render()
  end
end)

ygg.on("review.note", function(payload)
  if not HUMAN.gated("review.note") then
    return
  end
  local current = stage()
  if current ~= STAGE.review then
    return reject("review.note", fill(TEXTS.beats.stage_expected, { expected = STAGE.review }))
  end
  local problem = gate_problem()
  if problem ~= nil then
    ygg.narrate(fill(TEXTS.beats.req_gate_block, { action = "review.note", detail = problem }))
    render()
    return
  end
  if strict_gates() and ygg.kv.get("verify.passed") ~= true then
    ygg.narrate(TEXTS.beats.req_gate_verify)
    render()
    return
  end
  local lesson, score_text = cut(trim(payload), "|")
  lesson = trim(lesson)
  if lesson == "" then
    lesson = TEXTS.beats.default_lesson
  end
  local score = TEXTS.tuning.default_score
  local candidate = trim(score_text or "")
  if candidate ~= "" then
    local parsed = tonumber(candidate)
    if parsed == nil or parsed < 1 or parsed > 5 then
      return reject_payload("review.note", fill(TEXTS.beats.bad_score, { got = candidate }))
    end
    score = math.floor(parsed)
  end
  ygg.kv.set("review.lesson", lesson)
  ygg.kv.set("review.score", score)
  ygg.kv.set("ending", STAGE.done)
  ygg.narrate(fill(TEXTS.beats.review_ok, { score = tostring(score), lesson = lesson }))
  advance(STAGE.done)
end)

-- ---- 主 Agent 的需求图命令（串行；subagent 只能用 proposal 建议）--------

ygg.on("requirement.split", function(payload)
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
  local status = req_status(parent)
  if status ~= R_DRAFT and status ~= R_READY then
    return refuse(TEXTS.beats.req_wait_rejected, "requirement.split", fill(TEXTS.beats.req_split_status, {
      id = req_id(parent),
      status = status,
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
    local status = req_status(index)
    if status ~= R_DRAFT and status ~= R_READY then
      return refuse(TEXTS.beats.req_wait_rejected, "requirement.merge", fill(TEXTS.beats.req_merge_status, {
        id = req_id(index),
        status = status,
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
  if status == R_SPLIT or status == R_MERGED or status == R_DROPPED then
    return refuse(TEXTS.beats.req_wait_rejected, "requirement.amend", fill(TEXTS.beats.req_amend_closed, {
      id = req_id(index),
      status = status,
    }))
  end
  if status == R_DONE then
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
  if status == R_DEVELOPING then
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
        status = req_status(index),
      }))
    end
    if req_status(index) == R_DEVELOPING then
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
  if mode == "keep_first" and req_status(second) ~= R_DRAFT and req_status(second) ~= R_READY then
    return refuse(TEXTS.beats.req_wait_rejected, "requirement.resolve", fill(TEXTS.beats.req_resolve_drop_busy, {
      id = req_id(second),
      status = req_status(second),
    }))
  end
  if mode == "revise" then
    for _, index in ipairs({ first, second }) do
      if req_status(index) ~= R_DRAFT and req_status(index) ~= R_READY then
        return refuse(TEXTS.beats.req_wait_rejected, "requirement.resolve", fill(TEXTS.beats.req_resolve_revise_busy, {
          id = req_id(index),
          status = req_status(index),
        }))
      end
    end
  end
  ygg.kv.set(req_key(first, "conflict"), "")
  ygg.kv.set(req_key(second, "conflict"), "")
  ygg.kv.set(req_key(first, "conflict_peer"), "")
  ygg.kv.set(req_key(second, "conflict_peer"), "")
  local effect
  if mode == "keep_first" then
    ygg.kv.set(req_key(second, "status"), R_DROPPED)
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

-- 门禁动作（两个强制主 Agent 节点：plan_gate / accept_gate）。
-- 空载荷 = canonical 序取下一条待批需求；也可显式 `rN[|说明]`。
local function pick_gate(target)
  local raw = trim(target or "")
  if raw == "" then
    return gate_next()
  end
  local index = req_index_of(raw)
  if index == nil or gate_kind(index) == nil then
    return nil
  end
  return index
end

ygg.on("gate.approve", function(payload)
  if not HUMAN.gated("gate.approve") then
    return
  end
  if stage() == STAGE.done then
    return reject("gate.approve", TEXTS.beats.req_after_done)
  end
  local target, note = cut(trim(payload), "|")
  local index = pick_gate(target)
  if index == nil then
    return reject_payload("gate.approve", TEXTS.beats.req_gate_none)
  end
  local kind = gate_kind(index)
  -- stale：待批期间被 amend 过 → 旧门禁不得批准。
  if not gate_rev_ok(index) then
    return refuse(TEXTS.beats.req_wait_rejected, "gate.approve", fill(TEXTS.beats.req_gate_stale, {
      id = req_id(index),
      got = tostring(num(req_key(index, "gate_rev"), 0)),
      want = tostring(req_rev(index)),
    }))
  end
  -- 证据硬约束：accept_gate 不接受只有 summary 的完成自称。
  local evidence = gate_evidence_problem(index, kind)
  if evidence ~= nil then
    return refuse(TEXTS.beats.req_wait_rejected, "gate.approve", evidence)
  end
  local next_status = kind == "plan" and R_READY or R_DONE
  ygg.kv.set(req_key(index, "status"), next_status)
  req_release(index)
  req_push_history(index, fill(TEXTS.beats.req_gate_hist, { kind = kind }))
  req_sync_conflicts()
  ygg.narrate(fill(TEXTS.beats.req_gate_ok, {
    id = req_id(index),
    kind = kind,
    status = next_status,
    note = trim(note or "") ~= "" and trim(note) or TEXTS.beats.req_gate_no_note,
    artifacts = req_artifacts(index) ~= "" and req_artifacts(index) or TEXTS.beats.req_gate_no_artifacts,
    gates = tostring(gate_total()),
  }))
  render()
end)

ygg.on("gate.reject", function(payload)
  if stage() == STAGE.done then
    return reject("gate.reject", TEXTS.beats.req_after_done)
  end
  local target, feedback = cut(trim(payload), "|")
  local index = pick_gate(target)
  if index == nil then
    return reject_payload("gate.reject", TEXTS.beats.req_gate_none)
  end
  local kind = gate_kind(index)
  local back = kind == "plan" and R_PLANNING or R_READY
  ygg.kv.set(req_key(index, "status"), back)
  req_release(index)
  -- 打回不改 rev（执行方下一轮提案仍用当前 rev），但记录反馈供下轮参考。
  ygg.kv.set(req_key(index, "feedback"), trim(feedback or "") ~= "" and trim(feedback) or TEXTS.beats.req_reject_default)
  req_push_history(index, fill(TEXTS.beats.req_gate_reject_hist, { kind = kind }))
  req_sync_conflicts()
  ygg.narrate(fill(TEXTS.beats.req_gate_rejected, {
    id = req_id(index),
    kind = kind,
    status = back,
    feedback = text_of(req_key(index, "feedback"), TEXTS.beats.req_reject_default),
    gates = tostring(gate_total()),
  }))
  render()
end)

ygg.on("config.set", function(payload)
  local current = stage()
  if type(payload) ~= "string" then
    return reject("config.set", fill(TEXTS.beats.bad_payload, { action = "config.set" }))
  end
  local key, value = cut(trim(payload), "=")
  key = trim(key)
  value = trim(value or "")
  if key:sub(1, 9) == "settings." then
    key = key:sub(10)
  end
  local problem = nil
  if key == "play.mode" then
    -- 切模式会改变「谁在派发」：guide→goal 等于给后续直派开闸，故单重门禁。
    if not HUMAN.needs("config.set play.mode") then
      return
    end
    if not contains(PLAY_MODES, value) then
      problem = fill(TEXTS.beats.bad_config_value, { key = key, value = value, allowed = join(PLAY_MODES, " / ") })
    else
      ygg.kv.set("settings.play.mode", value)
      -- 驱动者换了人，二级页的「当前处境」已失效：退出子页回阶段菜单。
      CONFIG_UI.set_page("")
    end
  elseif key == "evolution.mode" then
    if not contains(MODES, value) then
      problem = fill(TEXTS.beats.bad_config_value, { key = key, value = value, allowed = join(MODES, " / ") })
    else
      ygg.kv.set("settings.evolution.mode", value)
    end
  elseif key == "evolution.max_per_run" then
    local parsed = tonumber(value)
    if parsed == nil or parsed < 0 or parsed > 8 then
      problem = fill(TEXTS.beats.bad_config_value, { key = key, value = value, allowed = "0–8 的整数" })
    else
      ygg.kv.set("settings.evolution.max_per_run", math.floor(parsed))
    end
  elseif key == "gates.require_all_green" or key == "autosave" or key == "hot_reload" then
    local parsed = to_bool(value)
    if parsed == nil then
      problem = fill(TEXTS.beats.bad_config_value, { key = key, value = value, allowed = "true / false" })
    else
      ygg.kv.set("settings." .. key, parsed)
    end
  else
    problem = fill(TEXTS.beats.bad_config_key, { key = key, allowed = TEXTS.beats.config_keys })
  end
  if problem ~= nil then
    return reject_payload("config.set", problem)
  end
  ygg.narrate(fill(TEXTS.beats.config_changed, {
    key = key,
    value = tostring(effective(key)),
    stage = current,
  }))
  render()
end)

-- ---- 二级菜单页的两个入口（纯展示层：只改 ui.page / ui.page.stage）----
-- 子页不占顶层首项：只有人主动选 config.open 才进，进了也只换菜单不换阶段。

ygg.on("config.open", function(payload)
  if not probe_payload_ok(payload) then
    return reject_payload("config.open", fill(TEXTS.beats.probe_rejected, {
      action = "config.open",
      got = tostring(payload),
      usage = TEXTS.beats.config_open_usage,
    }))
  end
  CONFIG_UI.set_page("config", stage())
  ygg.narrate(TEXTS.beats.config_opened)
  render()
end)

ygg.on("config.close", function(payload)
  if not probe_payload_ok(payload) then
    return reject_payload("config.close", fill(TEXTS.beats.probe_rejected, {
      action = "config.close",
      got = tostring(payload),
      usage = TEXTS.beats.config_open_usage,
    }))
  end
  CONFIG_UI.set_page("")
  ygg.narrate(fill(TEXTS.beats.config_closed, { stage = stage() }))
  render()
end)

ygg.on("guide.show", function(payload)
  local current = stage()
  local id = trim(payload)
  if id == "" then
    id = guide_section(current).id
  end
  narrate_guide(id)
  render()
end)

ygg.on("map.show", function(payload)
  if probe_payload_ok(payload) then
    narrate_map(stage())
  else
    ygg.narrate(fill(TEXTS.beats.map_rejected, { got = tostring(payload), usage = TEXTS.beats.map_usage }))
  end
  render()
end)

-- 选择路由：序号 → MENU 对应项 → 同名动作事件（与直派事件同一套守卫）。
ygg.on("ygg.choice_selected", function(index)
  local current = stage()
  local items = menu(current)
  if items == nil then
    ygg.narrate(fill(TEXTS.beats.unknown_choice, { stage = current }))
    render()
    return
  end
  local picked = tonumber(index)
  if picked == nil or items[picked] == nil then
    ygg.narrate(fill(TEXTS.beats.bad_index, {
      stage = current,
      index = tostring(index),
      count = tostring(#items),
    }))
    render()
    return
  end
  -- 盖章：只能在这里盖章（菜单路由），单发直派没有盖章机会。
  local label = HUMAN.label(items[picked].action, items[picked].payload)
  if label ~= nil then
    HUMAN.ticket = label
  end
  -- 二级页只跟 config.* 走：选了任何阶段动作就离开子页（否则子页会占住阶段菜单）。
  if items[picked].action:sub(1, 7) ~= "config." then
    CONFIG_UI.set_page("")
  end
  ygg.emit(items[picked].action, items[picked].payload)
end)

ygg.on("event.start", function()
  ygg.narrate(fill(TEXTS.scenes.prologue, {
    engine = tostring(ygg.engine_version),
    stage = stage(),
    mode = mode(),
  }))
  ygg.narrate(fill(stage_prompt("prologue"), prompt_vars()))
  ygg.narrate(TEXTS.beats.secret_redline)
  render()
end)

ygg.narrative.node("node:render", function()
  render()
end)

-- ============================================================================
-- §状态机 单机单事件：协调器四态；非法目标 = 静默 no-op（真正的门在 handler）。
-- ============================================================================
ygg.fsm.define({
  name = "stage",
  initial = STAGE.intake,
  event = "stage.advance",
  transitions = {
    {
      from = STAGE.intake,
      to = STAGE.coordinate,
      when = function(target)
        return target == STAGE.coordinate
      end,
    },
    {
      from = STAGE.coordinate,
      to = STAGE.review,
      when = function(target)
        return target == STAGE.review
      end,
    },
    {
      from = STAGE.review,
      to = STAGE.coordinate,
      when = function(target)
        return target == STAGE.coordinate
      end,
    },
    {
      from = STAGE.review,
      to = STAGE.done,
      when = function(target)
        return target == STAGE.done
      end,
    },
  },
})
