-- src/domain/wire/proposal.lua —— S32 纯搬移自 wire.lua（零逻辑变更；接线见交付回执）。

local M = {}
local config = require("src.core.config")
local util = require("src.core.util")
local state = require("src.domain.wire.state")
local TEXTS = setmetatable({}, { __index = function(_, k) return state.TEXTS[k] end })
local mod_intake = require("src.domain.wire.intake")
local text_of = config.text_of
local contains = util.contains
local cut = util.cut
local fill = util.fill
local join = util.join
local split = util.split
local trim = util.trim
local identity_segment_value, tail_segment_of = mod_intake.identity_segment_value, mod_intake.tail_segment_of
-- 前向声明：`proposal.submit` 的末段 `exec=` 复用执行者值的形状校验（实现在 §E2 登记面）。
local parse_executor_value, parse_exec_segment


-- ============================================================================
-- harness tools 注册面（r3 / harness 机证）：config.set tools.root / tools.<name>
-- 只校形状，不读文件（Mod 无 IO；存在性由首次调用时宿主可读回传）。
-- ============================================================================

-- tools.root 值：目录路径。非空、≤240、无 `|`/换行/控制字符、无 `..` 段；空值 = 显式清除。
-- 允许绝对与相对路径（相对 = 相对 checkout 根，与 pack.json program 同约定）。
local function tools_root_ok(path)
  if type(path) ~= "string" then
    return false
  end
  local raw = trim(path)
  if raw == "" then
    return true -- 空 = 清除
  end
  if #raw > 240 then
    return false
  end
  if raw:find("|", 1, true) ~= nil or raw:find("[%c]") ~= nil then
    return false
  end
  -- 禁 `..` 路径段（目录穿越）
  for segment in raw:gmatch("[^/]+") do
    if segment == ".." then
      return false
    end
  end
  return true
end

-- tools.<name> 覆盖值：相对 root 的脚本路径，必须 `.sh` 结尾、无 `..`、无 `|`/控制字符。
local function tools_relpath_ok(path)
  if type(path) ~= "string" then
    return false
  end
  local raw = trim(path)
  if raw == "" then
    return true -- 空 = 清除（回落缺省 <name>.sh）
  end
  if #raw > 240 or raw:sub(-3) ~= ".sh" then
    return false
  end
  if raw:find("|", 1, true) ~= nil or raw:find("[%c]") ~= nil then
    return false
  end
  if raw:find("..", 1, true) ~= nil then
    return false
  end
  return true
end

-- tools.<name> 的 name 段形状：`^[a-z][a-z0-9-]{0,63}$`（与工具 id 尾段同规则）。
local function tools_name_ok(name)
  if type(name) ~= "string" or #name < 1 or #name > 64 then
    return false
  end
  return name:match("^[a-z][a-z0-9%-]*$") ~= nil
end

-- 解析 `tools.<name>` 键（root 之外的覆盖键）：返回 name 或 nil。
local function parse_tools_key(key)
  if type(key) ~= "string" then
    return nil
  end
  local name = key:match("^tools%.([a-z][a-z0-9%-]*)$")
  if name == nil or name == "root" then
    return nil
  end
  if #name > 64 then
    return nil
  end
  return name
end

-- assignments token 化：逗号分隔 `r<n>[:<stage>]`，≤16 个、总长 ≤512；空串 = 空
-- 清单（清除字段）。编排层要机器解析，形状必须够硬。
local function parse_assignments(raw)
  local tokens = {}
  local total = 0
  for _, part in ipairs(split(trim(raw or ""), ",")) do
    local piece = trim(part)
    if piece ~= "" then
      -- Lua 模式不支持对捕获组加 `?` 量词（`?` 会变字面问号）：两种形态分别判定。
      if piece:match("^r%d+$") == nil and piece:match("^r%d+:[a-z_]+$") == nil then
        return nil, TEXTS.beats.profile_bad_assignments
      end
      total = total + #piece + 1
      tokens[#tokens + 1] = piece
      if #tokens > 16 then
        return nil, TEXTS.beats.profile_bad_assignments
      end
    end
  end
  if total - 1 > 512 then
    return nil, TEXTS.beats.profile_bad_assignments
  end
  return tokens, nil
end

-- 保留键/自由键校验矩阵（§3.2.2）：返回清洗后的值 + 错误。空值 = 清除（除
-- handle：显式写了就必须与键名一致，空 ≠ handle → 拒）。
local function validate_profile_value(handle, field, value)
  local raw = trim(value or "")
  if field == "handle" then
    if raw ~= handle then
      return nil, TEXTS.beats.profile_bad_handle
    end
    return raw, nil
  end
  if raw == "" then
    return "", nil -- 保留键/自由键空值 = 清除该字段，不做形状校验
  end
  if field == "harness" then
    -- 不做枚举（enum 会 fail-closed 掉未来 harness）；文档推荐词表 {pi, claude-code, codex, generic}。
    if #raw > 32 or raw:find("%s") ~= nil or raw:find("|", 1, true) ~= nil then
      return nil, TEXTS.beats.profile_bad_value
    end
    return raw, nil
  elseif field == "session_id" then
    -- 不透明审计字段：原生 session id，不做形状强校验（不同 harness 格式不同）。
    if #raw > 128 or raw:find("%s") ~= nil or raw:find("|", 1, true) ~= nil then
      return nil, TEXTS.beats.profile_bad_value
    end
    return raw, nil
  elseif field == "role" then
    if #raw > 64 or raw:find("[\r\n]") ~= nil then
      return nil, TEXTS.beats.profile_bad_value
    end
    return raw, nil
  elseif field == "assignments" then
    -- token 化只做校验（存储仍是逗号串；summary 投影时再解析成数组）。
    local _, problem = parse_assignments(raw)
    if problem ~= nil then
      return nil, problem
    end
    return raw, nil
  end
  -- 自由键：键名 ^[a-z0-9_]{1,32}$，值 ≤256 单行（`|` 允许——自由值不进叙事不进 wire）。
  if field:match("^[a-z0-9_]+$") == nil or #field > 32 then
    return nil, TEXTS.beats.profile_bad_value
  end
  if #raw > 256 or raw:find("[\r\n]") ~= nil then
    return nil, TEXTS.beats.profile_bad_value
  end
  return raw, nil
end

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

local function parse_proposal(payload)
  if type(payload) ~= "string" then
    return nil, fill(TEXTS.beats.bad_payload, { action = "proposal.submit" })
  end
  local parts = split(trim(payload), "|")
  -- 保留尾段只认**尾链固定相对序**（至多 3 段）：先从末尾连弹 → 校验相对序 →
  -- 余下段再带保留前缀 = 错位（宿主对 <7 段的载荷会补段重排，中间段会被静默挪进别的槽位）。
  -- 沙箱无 `table` 库：tail 按弹出序（载荷逆序）追加，校验/取值时反向遍历即得载荷原序。
  local tail = {} -- { {name, segment, meta}, … }（弹出序 = 载荷逆序）
  for _ = 1, 3 do
    local last = #parts
    if last == 0 then
      break
    end
    local item = tail_segment_of(trim(parts[last]))
    if item == nil then
      break
    end
    tail[#tail + 1] = { name = item.name, segment = trim(parts[last]), meta = item }
    parts[last] = nil
  end
  for i = 1, #parts do
    local item = tail_segment_of(trim(parts[i]))
    if item ~= nil then
      return nil, TEXTS.beats[item.beat]
    end
  end
  -- 相对序：弹出序是载荷逆序 → rank 必须严格递减；违规拒绝并点名错位段
  --（如 …|actor_session=s|actor=a：actor_session 没有落在最后）。
  local last_rank = nil
  for i = 1, #tail do
    local rank = tail[i].meta.rank
    if last_rank ~= nil and rank >= last_rank then
      return nil, TEXTS.beats[tail[i].meta.beat]
    end
    last_rank = rank
  end
  -- 逐段取值（反向遍历 = 载荷原序）：exec= 复用执行者形状校验；actor/actor_session
  -- 用身份形状（互不混用）。
  local exec_id, exec_reason, host_actor, host_actor_session = nil, nil, nil, nil
  for i = #tail, 1, -1 do
    local item = tail[i]
    if item.name == "exec" then
      local id, reason, problem = parse_exec_segment(item.segment)
      if problem ~= nil then
        return nil, problem
      end
      if id == "" then
        return nil, TEXTS.beats.executor_bad_id
      end
      exec_id, exec_reason = id, reason
    elseif item.name == "actor" then
      local value, problem = identity_segment_value(item.segment, 64, "executor_bad_actor")
      if problem ~= nil then
        return nil, problem
      end
      host_actor = value
    else
      local value, problem = identity_segment_value(item.segment, 128, "executor_bad_actor_session")
      if problem ~= nil then
        return nil, problem
      end
      host_actor_session = value
    end
  end
  if #parts < 4 then
    return nil, TEXTS.beats.req_prop_usage
  end
  local req_id_text = trim(parts[1])
  local rev = tonumber(trim(parts[2] or ""))
  local stage_name = trim(parts[3] or "")
  if rev == nil or not contains({ "plan", "development" }, stage_name) then
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
    exec = exec_id,
    exec_reason = exec_reason,
    host_actor = host_actor,
    host_actor_session = host_actor_session,
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
-- §settlement.evolution 进化登记 + review.note 复盘载荷
-- （artifact|summary[|c=<kept>/<screened>[|cls=<kind>:<n>,...]]）
--
-- 声明 ≠ 证明：这里只校验**形状与完整性**（summary、路径形状、两个整数、六类计数自洽、
-- screened > 0 时 artifact 与 cls 都不得缺），沙箱读不到文件，也不判断建议质量或复核是否真的独立执行过。
-- 分两层：parse_settlement_evolution 只认载荷形状，normalize_evolution 校跨字段完整性。
-- ============================================================================
local EVOLUTION_CLASSES = { "long_term", "unused", "one_off", "temp_tool", "redundant", "no_scene" }

-- 允许进入 backlog 的三类：长期机制 / 已有未用（需根因）/ 冗余退役。
local EVOLUTION_KEPT = { long_term = true, unused = true, redundant = true }


-- 值形：`<id>` 或 `<id>|<原因>`（也接受 `<id>:<原因>`，供 `proposal.submit` 的末段使用）；
-- `""` = 显式解绑。
parse_executor_value = function(value)
  local raw = trim(value or "")
  if raw == "" then
    return "", "", nil
  end
  local id, reason = cut(raw, "|")
  if reason == nil then
    id, reason = cut(raw, ":")
  end
  id = trim(id or "")
  reason = trim(reason or "")
  if id == "" or #id > 64 or id:find("%s") ~= nil then
    return nil, nil, TEXTS.beats.executor_bad_id
  end
  return id, reason, nil
end

-- `proposal.submit` 末段 `exec=<id>[:<原因>]`：登记并入提案提交（同事务），不自成回合。
parse_exec_segment = function(segment)
  local key, value = cut(trim(segment or ""), "=")
  if trim(key or "") ~= "exec" then
    return nil, nil, TEXTS.beats.executor_bad_id
  end
  return parse_executor_value(value)
end

M.EVOLUTION_CLASSES = EVOLUTION_CLASSES
M.parse_exec_segment = parse_exec_segment
M.parse_executor_value = parse_executor_value
M.parse_assignments = parse_assignments
M.validate_profile_value = validate_profile_value
M.EVOLUTION_KEPT = EVOLUTION_KEPT
M.parse_tools_key = parse_tools_key
M.tools_name_ok = tools_name_ok
M.tools_relpath_ok = tools_relpath_ok
M.tools_root_ok = tools_root_ok
M.RESOLVE_MODES = RESOLVE_MODES
M.inbox_mark = inbox_mark
M.inbox_replay = inbox_replay
M.parse_amend = parse_amend
M.parse_conflict = parse_conflict
M.parse_merge = parse_merge
M.parse_proposal = parse_proposal
M.parse_resolve = parse_resolve
M.parse_split = parse_split

return M
