-- src/runtime/tools/parse.lua —— S32 纯搬移自 tools.lua（零逻辑变更；接线见交付回执）。

local M = {}
local config = require("src.core.config")
local util = require("src.core.util")
local wire = require("src.domain.wire")
local state = require("src.runtime.tools.state")
local TEXTS = setmetatable({}, { __index = function(_, k) return state.TEXTS[k] end })
local effective = config.effective
local cut = util.cut
local fill = util.fill
local join = util.join
local split = util.split
local trim = util.trim
local strip_ext = wire.strip_ext
local valid_ext = wire.valid_ext


-- 形状常量（不是可调项）：可调项只有 TEXTS.tuning 的 tool_capacity / tool_output_max。
local MAX_CALL_ID = 32
local MAX_ARG_PAIRS = 8
local MAX_ARG_KEY = 32
local MAX_ARG_VALUE = 96
local MAX_ARGS_TEXT = 192
local MAX_TOOL_ID = 64
local CALL_ID_RE = "^[a-z][a-z0-9_%-]*$"
local TOOL_ID_RE = "^[a-z][a-z0-9_%-]*%.[a-z][a-z0-9_%-]*$"
local ARG_KEY_RE = "^[a-z][a-z0-9_]*$"

-- 响应式子系统的形状常量（不是可调项）：可调项只有 TEXTS.tuning 的 artifact_capacity。
local MAX_ARTIFACT_PATH = 128        -- 与 wire.evolution_path_ok 的上界同源
local MAX_VFS_CONTENT = 65536        -- `vfs.write` 的内容上限（64 KiB）
local MAX_ARTIFACT_CHECKS = 4        -- 一次 `proposal.submit` 最多同步核验几条
local MAX_RADAR_FILES = 32           -- 一次雷达最多比几个文件（防 O(n·m) 爆炸）
local SHA_DISPLAY = 12               -- 叙事里显示 sha256 的前多少位
local VERIFY_TOOL_SOURCE = "tool:gate.verify_suite"
-- r5（子会话清理）：harness.archive_sessions 从 Mod 工具桥整体退役——子会话清理改为
-- 提示词驱动（beats 清理文案 + settlement.sessions 声明制），宿主 builtin 的物理
-- 退役归 N2 宿主决策。SESSIONS_TOOL_SOURCE 与 REACTORS 条目随工具一并摘除。

-- `tool.<保留字>.*` 与计数键同名会撞车（`tool.count` 既是计数前缀又是行前缀）。
local RESERVED = { count = true, pending = true, last = true, notified = true }

-- ───────────────────────────── 标量读面（KV 派生，全部 O(1)） ─────────────────────────────

local function cap()
  local value = TEXTS.tuning.tool_capacity
  if type(value) == "number" and value > 0 then
    return math.floor(value)
  end
  return 64
end

local function output_max()
  local value = TEXTS.tuning.tool_output_max
  if type(value) == "number" and value > 0 then
    return math.floor(value)
  end
  return 1024
end

local function whole(value)
  local parsed = tonumber(value)
  if parsed == nil or parsed ~= math.floor(parsed) then
    return nil
  end
  return math.floor(parsed)
end

-- 生命周期计数（单调增；自定义 call_id 也增）。
local function count_of()
  local value = whole(ygg.kv.get("tool.count"))
  if value == nil or value < 0 then
    return 0
  end
  return value
end

-- 未终态计数（O(1)；`render.lua` 的摘要字段读它，绝不扫行）。
local function pending_count()
  local value = whole(ygg.kv.get("tool.pending"))
  if value == nil or value < 0 then
    return 0
  end
  return value
end

local function status_of(id)
  local value = ygg.kv.get("tool." .. tostring(id) .. ".status")
  if type(value) == "string" then
    return value
  end
  return ""
end

local function code_of(id)
  local value = whole(ygg.kv.get("tool." .. tostring(id) .. ".code"))
  if value == nil then
    return -1
  end
  return value
end

local function output_of(id)
  local value = ygg.kv.get("tool." .. tostring(id) .. ".output")
  if type(value) == "string" then
    return value
  end
  return ""
end

local function tool_of(id)
  local value = ygg.kv.get("tool." .. tostring(id) .. ".tool")
  if type(value) == "string" then
    return value
  end
  return ""
end

-- `field` ∈ { ext, res_ext }：请求幂等键 / 结果幂等键。
local function ext_of(id, field)
  local value = ygg.kv.get("tool." .. tostring(id) .. "." .. field)
  if type(value) == "string" then
    return value
  end
  return ""
end

local function exists_of(id)
  local value = ygg.kv.get("tool." .. tostring(id) .. ".id")
  return type(value) == "string" and value ~= ""
end

-- 分配序索引（`flush_meta` 的遍历面：`tool.count` 是生命周期计数，**不是**行号上界）。
local function call_id_at(index)
  local value = ygg.kv.get("tool.seq." .. tostring(index))
  if type(value) == "string" and value ~= "" then
    return value
  end
  return nil
end

-- 归一化参数文本 `k=v;k=v` → `{ k = v, … }`；空 = nil（**省略** `args` 键：
-- Lua 空表只能序列化成 `[]`，`args = []` 与 `args = {}` 在宿主侧是不同的形状）。
local function args_table(id)
  local raw = ygg.kv.get("tool." .. tostring(id) .. ".args")
  if type(raw) ~= "string" or raw == "" then
    return nil
  end
  local out = {}
  local segments = split(raw, ";")
  for i = 1, #segments do
    local key, value = cut(segments[i], "=")
    if value ~= nil then
      out[trim(key)] = trim(value)
    end
  end
  return out
end

-- ───────────────────────────── allowlist（Layer 2，fail-closed） ─────────────────────────────

-- `data/config.json.allowedTools`：缺失 / 非表 / 元素非法 → 空数组（一个也不放行）。
-- Mod **读不到** `pack.json`（`ygg.data` 只读 entries 声明的资源），所以跨层一致性
-- （allowedTools ⊇ tools）由 `tests/tools.lua` 的声明一致性用例钉住，不在运行期判。
local function allowed()
  local raw = effective("allowedTools")
  local out = {}
  if type(raw) ~= "table" then
    return out
  end
  for i = 1, #raw do
    local id = trim(raw[i])
    if id ~= "" and #id <= MAX_TOOL_ID and id:match(TOOL_ID_RE) ~= nil then
      out[#out + 1] = id
    end
  end
  return out
end

local function allowed_text()
  local list = allowed()
  if #list == 0 then
    return TEXTS.beats.tool_allowed_none
  end
  return join(list, ", ")
end

-- ───────────────────────────── 解析器（纯函数；返回「值…, 问题串」，绝不 error()） ─────────────────────────────

-- 参数：`k=v;k=v`（保输入序；归一化文本即 KV 里的 `.args`）。
local function parse_args(text)
  local out = {}
  local raw = trim(text or "")
  if raw == "" then
    return out, nil
  end
  if #raw > MAX_ARGS_TEXT then
    return nil, "the whole argument string must be at most " .. tostring(MAX_ARGS_TEXT) .. " bytes"
  end
  local segments = split(raw, ";")
  if #segments > MAX_ARG_PAIRS then
    return nil, "at most " .. tostring(MAX_ARG_PAIRS) .. " pairs are allowed (got " .. tostring(#segments) .. ")"
  end
  local seen = {}
  for i = 1, #segments do
    local segment = trim(segments[i])
    local key, value = cut(segment, "=")
    key = trim(key or "")
    if value == nil then
      return nil, "segment `" .. segment .. "` is not `k=v`"
    end
    value = trim(value)
    if #key > MAX_ARG_KEY or key:match(ARG_KEY_RE) == nil then
      return nil, "key `" .. key .. "` must match ^[a-z][a-z0-9_]*$ and be at most " .. tostring(MAX_ARG_KEY) .. " bytes"
    end
    if #value > MAX_ARG_VALUE then
      return nil, "the value of `" .. key .. "` must be at most " .. tostring(MAX_ARG_VALUE) .. " bytes"
    end
    if value:find("|", 1, true) ~= nil or value:find("[\r\n\0]") ~= nil then
      return nil, "the value of `" .. key .. "` must not contain a vertical bar, CR, LF, or NUL"
    end
    if seen[key] then
      return nil, "key `" .. key .. "` appears twice"
    end
    seen[key] = true
    out[#out + 1] = { k = key, v = value }
  end
  return out, nil
end

local function args_text_of(pairs_out)
  if pairs_out == nil or #pairs_out == 0 then
    return ""
  end
  local parts = {}
  for i = 1, #pairs_out do
    parts[#parts + 1] = pairs_out[i].k .. "=" .. pairs_out[i].v
  end
  return join(parts, ";")
end

-- tool.call：`<tool_id>[|<k=v;...>][|<call_id>]`（位次固定：第 2 段恒为参数，第 3 段恒为 call_id；
-- 只要 call_id 就写成 `git.status||t2`）。
local function parse_call(payload)
  if type(payload) ~= "string" then
    return nil, nil, nil, fill(TEXTS.beats.bad_payload, { action = "tool.call" })
  end
  local parts = split(trim(payload), "|")
  local count = #parts
  if count == 0 or count > 3 then
    return nil, nil, nil, TEXTS.beats.tool_usage
  end
  local tool_id = trim(parts[1])
  if tool_id == "" then
    return nil, nil, nil, TEXTS.beats.tool_usage
  end
  local args_text = ""
  if count >= 2 then
    args_text = trim(parts[2])
  end
  local call_id = ""
  if count == 3 then
    call_id = trim(parts[3])
  end
  return tool_id, args_text, call_id, nil
end

-- tool.result：`<call_id>|<tool_id>|<status>|<code>|<output>[|<external_id>]`。
-- 段数判定确定性：先剥 `|@ext=`，再按竖线切；5 段取标记 ext，6 段取第 6 段，其余拒绝。
local function parse_result(payload)
  if type(payload) ~= "string" then
    return nil, fill(TEXTS.beats.bad_payload, { action = "tool.result" })
  end
  local body, marked_ext = strip_ext(payload)
  local parts = split(trim(body), "|")
  local count = #parts
  if count < 5 or count > 6 then
    return nil, TEXTS.beats.tool_result_usage
  end
  local call_id = trim(parts[1])
  local tool_id = trim(parts[2])
  if call_id == "" or tool_id == "" then
    return nil, TEXTS.beats.tool_result_usage
  end
  local status = trim(parts[3])
  if status ~= "ok" and status ~= "err" then
    return nil, fill(TEXTS.beats.tool_bad_status, { status = status })
  end
  local code = -1
  local code_text = trim(parts[4])
  if code_text ~= "" then
    code = whole(code_text)
    if code == nil then
      return nil, fill(TEXTS.beats.tool_bad_code, { code = code_text })
    end
  end
  local output = parts[5] or ""
  -- 竖线切分已保证第 5 段不含竖线（含竖线只会多出段 → 上面已拒）；这里再判一次是
  -- 纵深防御，杜绝未来改成「末段合并」时静默把 bar 放进 output。
  if output:find("|", 1, true) ~= nil then
    return nil, TEXTS.beats.tool_result_usage
  end
  output = output:gsub("[\r\n]", " / ")
  local limit = output_max()
  -- `truncated` 是**显式判据**（不是字符串嗅探）：超限输出会追加含数字的截断提示，
  -- 反应器靠这个布尔值决定「不得用这份 output 做集合比对」（宁可漏报不可误报）。
  local truncated = false
  if #output > limit then
    truncated = true
    local dropped = #output - (limit - 3)
    output = output:sub(1, limit - 3) .. "..." .. fill(TEXTS.beats.tool_output_truncated, { dropped = tostring(dropped) })
  end
  local ext = marked_ext
  if count == 6 then
    local sixth = trim(parts[6])
    if sixth ~= "" then
      ext = sixth
    end
  end
  if ext ~= "" and not valid_ext(ext) then
    return nil, TEXTS.beats.tool_result_usage
  end
  return {
    call_id = call_id,
    tool_id = tool_id,
    status = status,
    code = code,
    output = output,
    ext = trim(ext or ""),
    truncated = truncated,
  }, nil
end

M.CALL_ID_RE = CALL_ID_RE
M.MAX_ARTIFACT_PATH = MAX_ARTIFACT_PATH
M.MAX_CALL_ID = MAX_CALL_ID
M.MAX_VFS_CONTENT = MAX_VFS_CONTENT
M.TOOL_ID_RE = TOOL_ID_RE
M.allowed = allowed
M.allowed_text = allowed_text
M.args_table = args_table
M.call_id_at = call_id_at
M.code_of = code_of
M.output_of = output_of
M.parse_args = parse_args
M.parse_call = parse_call
M.parse_result = parse_result
M.pending_count = pending_count
M.status_of = status_of
M.MAX_ARTIFACT_CHECKS = MAX_ARTIFACT_CHECKS
M.MAX_RADAR_FILES = MAX_RADAR_FILES
M.MAX_TOOL_ID = MAX_TOOL_ID
M.RESERVED = RESERVED
M.SHA_DISPLAY = SHA_DISPLAY
M.VERIFY_TOOL_SOURCE = VERIFY_TOOL_SOURCE
M.args_text_of = args_text_of
M.cap = cap
M.count_of = count_of
M.exists_of = exists_of
M.ext_of = ext_of
M.output_max = output_max
M.tool_of = tool_of
M.whole = whole

return M
