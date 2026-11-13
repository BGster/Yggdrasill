-- src/domain/wire.lua —— payload wire format 解析 + inbox 幂等键（`@ext=`）+ settlement.evolution 进化登记。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 资源由入口 install(deps) 注入。本模块唯一写的 KV 前缀：无（只读）。

local M = {}

local config = require("src.core.config")
local reqs = require("src.domain.requirements")
local util = require("src.core.util")

local TEXTS

local effective, text_of =
  config.effective, config.text_of
local contains, cut, fill, join, split, trim =
  util.contains, util.cut, util.fill, util.join, util.split, util.trim

-- 前向声明：`proposal.submit` 的末段 `exec=` 复用执行者值的形状校验（实现在 §E2 登记面）。
local parse_executor_value, parse_exec_segment

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

-- `requirement.dispatch` 的载荷：`""` / `"0"` = 按就绪序派活；`"rN"` = 只派这一条；
-- `"full"` 保留为合法 no-op（派活从来只按就绪序，没有「更多输出」这回事）。
local function parse_dispatch(payload)
  if payload == nil then
    return nil, nil
  end
  local raw = payload
  if type(payload) == "number" then
    if payload == 0 then
      return nil, nil
    end
    raw = tostring(payload)
  elseif type(payload) ~= "string" then
    raw = tostring(payload)
  else
    raw = trim(payload)
  end
  if raw == "" or raw == "0" or raw == "full" then
    return nil, nil
  end
  local target = raw:match("^r%d+$")
  if target == nil then
    return nil, fill(TEXTS.beats.probe_rejected, {
      got = tostring(payload),
      usage = TEXTS.beats.req_dispatch_usage,
    })
  end
  return target, nil
end

local function valid_ext(ext)
  if type(ext) ~= "string" then
    return false
  end
  local raw = trim(ext)
  if raw == "" or #raw > 128 then
    return false
  end
  return raw:find("%s") == nil
end

-- 章程解析（D-MA23 三段）：`<title>|<goal>|<success>`。
-- `parse_intake`（宽松，event.start 启动载荷用）：空段回退默认文案，不拒绝——
-- 保留旧「照常 boot」语义；`parse_charter`（严格，charter.set 用）：三段必须全部非空，
-- 否则可读拒绝（章程是可判定的验收面，不允许占位符冒充）。
local function parse_intake(payload)
  if type(payload) ~= "string" then
    return nil, nil, nil, fill(TEXTS.beats.bad_payload, { action = "event.start" })
  end
  local raw = trim(payload)
  if raw == "" then
    return TEXTS.beats.default_task_title, TEXTS.beats.default_task_goal, TEXTS.beats.default_task_success, nil
  end
  local title, rest = cut(raw, "|")
  local goal, success = cut(trim(rest or ""), "|")
  title = trim(title)
  goal = trim(goal or "")
  success = trim(success or "")
  if title == "" then
    title = TEXTS.beats.default_task_title
  end
  if goal == "" then
    goal = TEXTS.beats.default_task_goal
  end
  if success == "" then
    success = TEXTS.beats.default_task_success
  end
  return title, goal, success, nil
end

local function parse_charter(payload)
  if type(payload) ~= "string" then
    return nil, nil, nil, fill(TEXTS.beats.bad_payload, { action = "charter.set" })
  end
  local raw = trim(payload)
  if raw == "" then
    return nil, nil, nil, TEXTS.beats.charter_usage
  end
  local title, rest = cut(raw, "|")
  local goal, success = cut(trim(rest or ""), "|")
  title = trim(title)
  goal = trim(goal or "")
  success = trim(success or "")
  if title == "" or goal == "" or success == "" then
    return nil, nil, nil, TEXTS.beats.charter_usage
  end
  return title, goal, success, nil
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

-- ── 保留尾段链（r3 提交者身份）─────────────────────────────────────────────────────────
-- 宿主是唯一盖章方：声明路由上宿主在载荷末尾注入 `|actor=<id>` / `|actor_session=<id>`；
-- 调用方自带的同名段在宿主侧已被拒（ACTOR_FORGED），到达这里还带保留段 = 错位（旧宿主回滚面）。
-- 固定相对顺序：…|exec=<id>[:<reason>]|actor=<actor>|actor_session=<session>（各自可独立缺席）。
local TAIL_SEGMENTS = {
  { name = "exec", rank = 1, beat = "req_prop_exec_misplaced" },
  { name = "actor", rank = 2, beat = "req_prop_actor_misplaced" },
  { name = "actor_session", rank = 3, beat = "req_prop_actor_session_misplaced" },
}

local function tail_segment_of(segment)
  for _, item in ipairs(TAIL_SEGMENTS) do
    -- `actor_session=x` 不匹配 `^actor=`（第 6 字节是 `_` 不是 `=`），三段可独立判定。
    if segment:match("^" .. item.name .. "=") ~= nil then
      return item
    end
  end
  return nil
end

-- 身份段值形状（与宿主 validate 同规则）：非空、≤max、无空白、无引号/反斜杠
--（`|` 不可能：段已按 `|` 切开）。非法 → 可读拒绝（防张冠李戴的“身份”进存档）。
local function identity_segment_value(segment, max, bad_beat)
  local value = trim(segment:match("^[^=]+=(.*)$") or "")
  if value == "" or #value > max or value:find("%s") ~= nil
    or value:find('["\\]') ~= nil then
    return nil, TEXTS.beats[bad_beat]
  end
  return value, nil
end

-- ── profile 注册表（r3，自报身份卡：声明面不是凭据）─────────────────────────
-- 键形 `profile.<handle>.<key>`：handle 形状复用 exec=/YGG_ACTOR 规则（账本 /
-- profile / 宿主章三层同形同值，字符串相等贯通）；保留键 harness / session_id /
-- handle / role / assignments，其余自由键；空值 = 清除该字段（整卡随 run 终结）。
-- 写面单源在 handlers/config.lua 的 profile_set；解析/校验纯函数单源在这里。

-- `profile.<handle>.<key>` → handle, field；任何形状偏差 → nil（调用方回 profile_bad_handle）。
local function parse_profile_key(key)
  if type(key) ~= "string" then
    return nil, nil
  end
  local handle, field = key:match("^profile%.([^.]+)%.(.+)$")
  if handle == nil or trim(field or "") == "" then
    return nil, nil
  end
  -- handle 形状复用 executor id 规则：非空、≤64、无空白、无 `|`（键里可能带 `|`，显式拦）。
  if #handle > 64 or handle:find("%s") ~= nil or handle:find("|", 1, true) ~= nil then
    return nil, nil
  end
  return handle, field
end

-- ============================================================================
-- harness tools 注册面（r3 / D-MA26）：config.set tools.root / tools.<name>
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

local function evolution_path_ok(path)
  if type(path) ~= "string" then
    return false
  end
  local raw = trim(path)
  if raw == "" or #raw > 128 then
    return false
  end
  if raw:sub(1, 10) ~= "artifacts/" or raw:sub(-3) ~= ".md" then
    return false
  end
  if #raw <= 13 or raw:find("..", 1, true) ~= nil or raw:find("%s") ~= nil then
    return false
  end
  return true
end

-- artifact 路径形状 = 同一个判据（`artifacts/<name>.md`，≤128 B，无 `..`，无空白）。
-- `vfs.write`、`fs.artifact_probe` 与 `proposal.submit` 的同步核验共用这一处事实源，不复制正则。

local function whole_number(value)
  local parsed = tonumber(value)
  if parsed == nil or parsed ~= math.floor(parsed) then
    return nil
  end
  return math.floor(parsed)
end

local function parse_candidates(text)
  local kept_text, screened_text = cut(trim(text), "/")
  local kept = whole_number(trim(kept_text or ""))
  local screened = whole_number(trim(screened_text or ""))
  if kept == nil or screened == nil then
    return nil
  end
  if kept < 0 or screened < 0 or kept > screened then
    return nil
  end
  return kept, screened
end

local function parse_classes(text)
  local out = {}
  local total = 0
  for _, part in ipairs(split(trim(text), ",")) do
    local piece = trim(part)
    if piece ~= "" then
      local kind, count_text = cut(piece, ":")
      kind = trim(kind or "")
      local count = whole_number(trim(count_text or ""))
      if not contains(EVOLUTION_CLASSES, kind) or count == nil or count < 0 or out[kind] ~= nil then
        return nil
      end
      out[kind] = count
      total = total + count
    end
  end
  return out, total
end

local function format_classes(classes)
  local out = {}
  for i = 1, #EVOLUTION_CLASSES do
    local kind = EVOLUTION_CLASSES[i]
    local count = classes[kind]
    if type(count) == "number" then
      out[#out + 1] = kind .. ":" .. tostring(count)
    end
  end
  return join(out, ",")
end

local function kept_total(classes)
  local total = 0
  for i = 1, #EVOLUTION_CLASSES do
    local kind = EVOLUTION_CLASSES[i]
    if EVOLUTION_KEPT[kind] == true then
      total = total + (classes[kind] or 0)
    end
  end
  return total
end

-- 四路输入一律是字符串（缺省 = ""）：artifact 可空；summary 去空白后必填；
-- candidates 空 = 0/0；classes 空 = 未分类。config.set 只逐键校形状，
-- 跨字段完整性集中在这里校（预登记阶段允许分次登记）。
-- 完整性是结算登记硬条件：screened > 0 就必须同时有复核 artifact 与逐类计数——
-- 「筛了几条、留了几分」而把复核记录与根因分类留在散文里，等于没登记。
local function normalize_evolution(artifact, summary, candidates, classes)
  local record = { artifact = "", summary = "", kept = 0, screened = 0, classes = {} }
  local summary_text = trim(summary or "")
  if summary_text == "" then
    return nil, TEXTS.beats.settlement_evolution_missing
  end
  record.summary = summary_text
  local artifact_text = trim(artifact or "")
  if artifact_text ~= "" then
    if not evolution_path_ok(artifact_text) then
      return nil, TEXTS.beats.review_evolution_bad_path
    end
    record.artifact = artifact_text
  end
  local candidates_text = trim(candidates or "")
  if candidates_text ~= "" then
    local kept, screened = parse_candidates(candidates_text)
    if kept == nil then
      return nil, TEXTS.beats.review_evolution_bad_candidates
    end
    record.kept, record.screened = kept, screened
  end
  local classes_text = trim(classes or "")
  local parsed, total = nil, 0
  if classes_text ~= "" then
    parsed, total = parse_classes(classes_text)
    if parsed == nil or total == 0 then
      return nil, TEXTS.beats.review_evolution_bad_classes
    end
  end
  if record.screened > 0 then
    if record.artifact == "" then
      return nil, TEXTS.beats.review_evolution_need_artifact
    end
    if parsed == nil then
      return nil, TEXTS.beats.review_evolution_need_classes
    end
  end
  if parsed ~= nil then
    if total ~= record.screened or kept_total(parsed) ~= record.kept then
      return nil, TEXTS.beats.review_evolution_mismatch
    end
    record.classes = parsed
  end
  return record, nil
end

-- 预登记面（`config.set review.evolution.*` → `settings.review.evolution.*`）与菜单载荷读同一份口径。
local function evolution_settings()
  local function read(name)
    local value = effective(name)
    if type(value) == "string" then
      return value
    end
    return ""
  end
  return read("review.evolution.artifact"), read("review.evolution.summary"),
    read("review.evolution.candidates"), read("review.evolution.classes")
end

-- 菜单构造结算载荷：零候选也有中性 summary 与显式 c=0/0，不需要伪 artifact。
local function format_settlement_evolution(artifact, summary, candidates, classes)
  local summary_text = trim(summary or "")
  if summary_text == "" then
    summary_text = TEXTS.beats.settlement_evolution_default_summary
  end
  local candidates_text = trim(candidates or "")
  local parts = {
    trim(artifact or ""),
    summary_text,
    "c=" .. (candidates_text ~= "" and candidates_text or "0/0"),
  }
  local classes_text = trim(classes or "")
  if classes_text ~= "" then
    parts[#parts + 1] = "cls=" .. classes_text
  end
  return join(parts, "|")
end

-- ============================================================================
-- §局末两项清理（subagent 会话 / 主会话 Todo）的形状单一事实源
--
-- 两个入口共用同一套校验：独立动作 `settlement.sessions|todo` 与
-- `settlement.evolution` 的可选段（`sessions=` / `todo=`，`/` 分隔，外层是 `|`）。
-- 自声明：Mod 读不到会话与 Todo API，这里只校「登记断言」的形状。
-- ============================================================================
local TODO_VERIFIED_MAX = 120

local function sessions_shape_ok(moved, remaining, destination)
  if moved == nil or remaining == nil then
    return false
  end
  return moved >= 0 and remaining == 0 and destination ~= ""
end

local function todo_shape_ok(remaining, verified_by)
  if remaining == nil then
    return false
  end
  return remaining == 0 and verified_by ~= "" and #verified_by <= TODO_VERIFIED_MAX
end

-- 独立动作的载荷：`moved|remaining|destination`（`|` 分隔，恰三段）。
local function parse_sessions_payload(payload)
  if type(payload) ~= "string" then
    return nil, TEXTS.beats.settlement_sessions_usage
  end
  local parts = split(trim(payload), "|")
  if #parts ~= 3 then
    return nil, TEXTS.beats.settlement_sessions_usage
  end
  local moved = tonumber(trim(parts[1]))
  local remaining = tonumber(trim(parts[2]))
  local destination = trim(parts[3])
  if not sessions_shape_ok(moved, remaining, destination) then
    return nil, TEXTS.beats.settlement_sessions_usage
  end
  return { moved = math.floor(moved), remaining = 0, destination = destination }, nil
end

-- 独立动作的载荷：`remaining|verified_by`（`|` 分隔，恰两段）。
local function parse_todo_payload(payload)
  if type(payload) ~= "string" then
    return nil, TEXTS.beats.settlement_todo_usage
  end
  local parts = split(trim(payload), "|")
  if #parts ~= 2 then
    return nil, TEXTS.beats.settlement_todo_usage
  end
  local remaining = tonumber(trim(parts[1]))
  local verified_by = trim(parts[2])
  if not todo_shape_ok(remaining, verified_by) then
    return nil, TEXTS.beats.settlement_todo_usage
  end
  return { remaining = 0, verified_by = verified_by }, nil
end

-- 可选段的载荷：`moved/remaining/destination`（只切前两刀，destination 里的 `/` 原样保留）。
local function parse_sessions_segment(value)
  local moved_text, rest = cut(trim(value or ""), "/")
  local remaining_text, destination = cut(trim(rest or ""), "/")
  local moved = tonumber(trim(moved_text))
  local remaining = tonumber(trim(remaining_text))
  local text = trim(destination or "")
  if not sessions_shape_ok(moved, remaining, text) then
    return nil, TEXTS.beats.settlement_sessions_segment_bad
  end
  return { moved = math.floor(moved), remaining = 0, destination = text }, nil
end

-- 可选段的载荷：`remaining/verified_by`（只切第一刀，verified_by 里的 `/` 原样保留）。
local function parse_todo_segment(value)
  local remaining_text, verified_by = cut(trim(value or ""), "/")
  local remaining = tonumber(trim(remaining_text))
  local text = trim(verified_by or "")
  if not todo_shape_ok(remaining, text) then
    return nil, TEXTS.beats.settlement_todo_segment_bad
  end
  return { remaining = 0, verified_by = text }, nil
end

-- `artifact|summary[|c=<kept>/<screened>[|cls=<kind>:<n>,...]]
--          [|sessions=<moved>/<remaining>/<destination>][|todo=<remaining>/<verified_by>]`。
-- 本函数只解析载荷形状（{artifact, summary, segments, sessions, todo}），跨字段完整性交
-- normalize_evolution；c / cls 缺段才回落 settings。显式空段、ev=、重复段与未知段都在这里拒绝。
local function parse_settlement_evolution(payload)
  if type(payload) ~= "string" then
    return nil, fill(TEXTS.beats.bad_payload, { action = "settlement.evolution" })
  end
  local parts = split(trim(payload), "|")
  if #parts < 2 then
    return nil, TEXTS.beats.settlement_evolution_usage
  end
  local artifact = trim(parts[1] or "")
  local summary = trim(parts[2] or "")
  if summary == "" then
    return nil, TEXTS.beats.settlement_evolution_need_summary
  end
  local segments = {}
  local sessions, todo = nil, nil
  for i = 3, #parts do
    local segment = trim(parts[i])
    local key, value = cut(segment, "=")
    key = trim(key or "")
    value = trim(value or "")
    if key ~= "c" and key ~= "cls" and key ~= "sessions" and key ~= "todo" then
      return nil, fill(TEXTS.beats.settlement_evolution_bad_segment, { segment = segment })
    end
    if segments[key] ~= nil then
      return nil, TEXTS.beats.settlement_evolution_duplicate_segment
    end
    if value == "" then
      if key == "c" then
        return nil, TEXTS.beats.review_evolution_bad_candidates
      elseif key == "cls" then
        return nil, TEXTS.beats.review_evolution_bad_classes
      elseif key == "sessions" then
        return nil, TEXTS.beats.settlement_sessions_segment_bad
      end
      return nil, TEXTS.beats.settlement_todo_segment_bad
    end
    segments[key] = value
    if key == "sessions" then
      local parsed, segment_problem = parse_sessions_segment(value)
      if parsed == nil then
        return nil, segment_problem
      end
      sessions = parsed
    elseif key == "todo" then
      local parsed, segment_problem = parse_todo_segment(value)
      if parsed == nil then
        return nil, segment_problem
      end
      todo = parsed
    end
  end
  return { artifact = artifact, summary = summary, segments = segments, sessions = sessions, todo = todo }, nil
end

-- 菜单构造 review.note 载荷：`lesson|score` 两段，进化声明不从这里走。
local function format_review_note(lesson, score)
  return join({ lesson, score }, "|")
end

local function parse_review_note(payload)
  if type(payload) ~= "string" then
    return nil, fill(TEXTS.beats.bad_payload, { action = "review.note" })
  end
  local parts = split(trim(payload), "|")
  if #parts > 2 then
    -- 旧版把进化声明塞进 note（ev= / c= / cls=）：一律拒绝，进化只走 settlement.evolution。
    for i = 3, #parts do
      local key = cut(trim(parts[i]), "=")
      key = trim(key or "")
      if key == "ev" or key == "c" or key == "cls" then
        return nil, TEXTS.beats.review_note_legacy_evolution
      end
    end
    return nil, TEXTS.beats.review_note_usage
  end
  return { lesson = trim(parts[1] or ""), score_text = trim(parts[2] or "") }, nil
end

-- ============================================================================
-- §E2 登记面：goal 授权 + 执行者账本（`config.set` 的两个键族）
--
-- 两个都是**自声明**：状态机只能校形状与「换人必给原因」，读不到会话，也不验证 id 真伪。
-- 落点：`settings.goal.authorization` / `settings.executor.<rN>.<stage>`（`settings.*` 唯一写方 = config.set）。
-- ============================================================================
local EXECUTOR_STAGES = { "plan", "development" } -- R12：只剩两个活跃阶段，analysis 槽位删除

local GOAL_AUTH_MAX = 200

local function parse_goal_auth(value)
  local raw = trim(value or "")
  if raw == "" then
    return "", nil
  end
  if #raw > GOAL_AUTH_MAX or raw:find("[\r\n]") ~= nil then
    return nil, TEXTS.beats.goal_auth_bad
  end
  return raw, nil
end

local function executor_stage_ok(stage)
  return contains(EXECUTOR_STAGES, stage)
end

-- `executor.rN.<stage>` → req_id, stage；其它形状 → nil（由调用方报可读错误）。
local function parse_executor_key(key)
  if type(key) ~= "string" then
    return nil, nil
  end
  return key:match("^executor%.(r%d+)%.([a-z_]+)$")
end

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

local function format_executor(id, reason)
  if reason ~= nil and reason ~= "" then
    return id .. "|" .. reason
  end
  return id
end

-- 只取 id 部分（忽略 `|原因`）：列表展示与前序阶段比较都用它。
local function executor_id_of(raw)
  local id = cut(trim(raw or ""), "|")
  return trim(id or "")
end

function M.install(deps)
  TEXTS = deps.texts
end

M.EVOLUTION_CLASSES = EVOLUTION_CLASSES
M.EXECUTOR_STAGES = EXECUTOR_STAGES
M.executor_id_of = executor_id_of
M.executor_stage_ok = executor_stage_ok
M.format_executor = format_executor
M.parse_exec_segment = parse_exec_segment
M.parse_executor_key = parse_executor_key
M.parse_executor_value = parse_executor_value
M.parse_goal_auth = parse_goal_auth
M.parse_profile_key = parse_profile_key
M.parse_assignments = parse_assignments
M.validate_profile_value = validate_profile_value
M.EVOLUTION_KEPT = EVOLUTION_KEPT
M.artifact_path_ok = evolution_path_ok
M.evolution_path_ok = evolution_path_ok
M.evolution_settings = evolution_settings
M.format_classes = format_classes
M.format_review_note = format_review_note
M.format_settlement_evolution = format_settlement_evolution
M.kept_total = kept_total
M.normalize_evolution = normalize_evolution
M.parse_candidates = parse_candidates
M.parse_classes = parse_classes
M.parse_review_note = parse_review_note
M.parse_sessions_payload = parse_sessions_payload
M.parse_sessions_segment = parse_sessions_segment
M.parse_settlement_evolution = parse_settlement_evolution
M.parse_todo_payload = parse_todo_payload
M.parse_todo_segment = parse_todo_segment
M.sessions_shape_ok = sessions_shape_ok
M.todo_shape_ok = todo_shape_ok
M.parse_tools_key = parse_tools_key
M.tools_name_ok = tools_name_ok
M.tools_relpath_ok = tools_relpath_ok
M.tools_root_ok = tools_root_ok

M.RESOLVE_MODES = RESOLVE_MODES
M.inbox_mark = inbox_mark
M.inbox_replay = inbox_replay
M.parse_amend = parse_amend
M.parse_conflict = parse_conflict
M.parse_deps = parse_deps
M.parse_dispatch = parse_dispatch
M.parse_charter = parse_charter
M.parse_intake = parse_intake
M.parse_merge = parse_merge
M.parse_proposal = parse_proposal
M.parse_requirement = parse_requirement
M.parse_resolve = parse_resolve
M.parse_split = parse_split
M.probe_payload_ok = probe_payload_ok
M.strip_ext = strip_ext
M.valid_ext = valid_ext

return M
