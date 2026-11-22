-- src/domain/wire/intake.lua —— S32 纯搬移自 wire.lua（零逻辑变更；接线见交付回执）。

local M = {}
local util = require("src.core.util")
local state = require("src.domain.wire.state")
local TEXTS = setmetatable({}, { __index = function(_, k) return state.TEXTS[k] end })
local cut = util.cut
local fill = util.fill
local join = util.join
local split = util.split
local trim = util.trim


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

-- 章程解析（剧情双层重写 三段）：`<title>|<goal>|<success>`。
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

-- ── stamp-only 路由的盖章段剥离（r9，按人读签）───────────────────────────
-- `config.set` / `gate.approve` / `coordinate.submit` 在 `pack.json` 里声明了 `actor: true`
-- 但**不带 write**（stamp-only）：宿主为直接 `--event` 通道在载荷末尾注入 `|actor=<id>`，
-- 这两段**不是业务载荷**——不剥掉会污染 `key=value` 的 value（config.set）与被 `cut` 切开的
-- note 段（门禁）。
--
-- 剥离面严格限定：只认**末尾连续**的 `actor_session` → `actor`（相对顺序与注入面一致），
-- 值走 identity_segment_value 同一形状尺（非法 = 可读拒绝，不让一份“身份”蒙进存档）。
-- 余下段里再出现保留段 = 错位（宿主只从末尾盖章）——与 parse_proposal 的 misplaced 同纪律，
-- 不静默重排（AGENTS §9.1 的 req_prop_*_misplaced 教训）。
--
-- 返回 payload, actor, actor_session, problem：无盖章段时原样返回且两个身份为 nil
--（旧路径 / 测试直派 / 票据路径逐字节不变）。
local function strip_identity_tail(payload)
  if type(payload) ~= "string" then
    return payload, nil, nil, nil
  end
  local parts = split(trim(payload), "|")
  local last = #parts
  if last < 2 then
    return payload, nil, nil, nil
  end
  local actor, actor_session = nil, nil
  local cut_from = last + 1
  local item = tail_segment_of(trim(parts[last]))
  if item ~= nil and item.name == "actor_session" then
    local value, problem = identity_segment_value(trim(parts[last]), 128, "executor_bad_actor_session")
    if problem ~= nil then
      return nil, nil, nil, problem
    end
    actor_session = value
    cut_from = last
    last = last - 1
    item = last >= 1 and tail_segment_of(trim(parts[last])) or nil
  end
  if item ~= nil and item.name == "actor" then
    local value, problem = identity_segment_value(trim(parts[last]), 64, "executor_bad_actor")
    if problem ~= nil then
      return nil, nil, nil, problem
    end
    actor = value
    cut_from = last
    last = last - 1
  end
  if actor == nil and actor_session == nil then
    return payload, nil, nil, nil
  end
  for i = 1, last do
    local stray = tail_segment_of(trim(parts[i]))
    if stray ~= nil then
      return nil, nil, nil, TEXTS.beats[stray.beat]
    end
  end
  local kept = {}
  for i = 1, cut_from - 1 do
    kept[#kept + 1] = parts[i]
  end
  return join(kept, "|"), actor, actor_session, nil
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

-- ── 幕前必读签名（r3 销票制）：`read.<act>/<doc-id>=<ver>` ───────────────
-- 键形 `read.<scene>/<doc-id>`：scene 声明「这份文档属于哪一幕」，doc-id 是被读文档的
-- 稳定短名（小写短横线）。**禁点号**：`src/core/config.lua` 的 effective() 用 `.` 逐段
-- 下钻，带点的键会把「读 read.<doc>」落到错位段上，所以在入口判死而不是在读取侧写特例。
-- scene 段不做枚举：把五幕写成白名单会 fail-closed 掉未来的需求级幕（与 harness 字段
-- 同一取舍），非法幕段的代价只是「不计入任何一幕」，本身不危险。
-- 值 `ver|scene|seq` 由 src/domain/readsig.lua 组装与本模块解析；这里只做纯形状判定。

-- `read.<scene>/<doc-id>` → "<scene>/<doc-id>"；任何形状偏差 → nil。
local function parse_read_key(key)
  if type(key) ~= "string" then
    return nil
  end
  local scene, doc = key:match("^read%.([a-z][a-z0-9%-]*)/([a-z][a-z0-9%-]*)$")
  if scene == nil or #scene > 32 or #doc > 48 then
    return nil
  end
  return scene .. "/" .. doc
end

-- 签名值里的 ver 段：4-40 位小写十六进制（r1 的 `git hash-object` 前缀，允许更长的
-- 摘要方案）。空串 = 显式清除（与 tools.root / run.retire 同构）。
local function read_ver_ok(ver)
  if type(ver) ~= "string" then
    return false
  end
  local raw = trim(ver)
  if raw == "" then
    return true
  end
  if #raw < 4 or #raw > 40 then
    return false
  end
  return raw:match("^[0-9a-f]+$") ~= nil
end

M.parse_profile_key = parse_profile_key
M.parse_read_key = parse_read_key
M.read_ver_ok = read_ver_ok
M.parse_deps = parse_deps
M.parse_dispatch = parse_dispatch
M.parse_charter = parse_charter
M.parse_intake = parse_intake
M.parse_requirement = parse_requirement
M.probe_payload_ok = probe_payload_ok
M.strip_ext = strip_ext
M.strip_identity_tail = strip_identity_tail
M.valid_ext = valid_ext
M.identity_segment_value = identity_segment_value
M.tail_segment_of = tail_segment_of

return M
