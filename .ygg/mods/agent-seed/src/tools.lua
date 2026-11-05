-- src/tools.lua —— 工具桥的 KV 面与回执面（`tool.*` 唯一写方）+ 两个事件 handler + 一条回执 effect。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；资源由入口 install(deps) 注入，
-- 注册面全部在 register() 内（与 src/render.lua 同构：render 也是 src/ 下的纯模块）。
--
-- 模型：`tool.call` 只**登记**请求（`tool.<id>.*` 行 + `meta.tool_calls` 队列），真正执行在宿主；
-- 结果以普通事件 `tool.result` 回来。Mod 永不做 IO（无 process / 网络 / 文件系统）。
--
-- 反向禁止：`src/render.lua` 不得 require 本模块（立刻成环）——摘要字段走 KV 读 `tool.pending`。
--
-- 本模块写的 KV 前缀：
--   tool.count / tool.pending / tool.last / tool.seq.* / tool.<id>.* / tool.notified.*

local M = {}

local config = require("src.config")
local flow = require("src.flow")
local hooks = require("src.hooks")
local render_mod = require("src.render")
local reqs = require("src.requirements")
local util = require("src.util")
local wire = require("src.wire")

local TEXTS

local effective =
  config.effective
local reject, reject_payload =
  flow.reject, flow.reject_payload
local render =
  render_mod.receipt
local STAGE, stage =
  reqs.STAGE, reqs.stage
local contains, cut, fill, join, split, trim =
  util.contains, util.cut, util.fill, util.join, util.split, util.trim
local strip_ext, valid_ext =
  wire.strip_ext, wire.valid_ext

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
-- （allowedTools ⊇ tools）由 `agent_seed_semantics` 的声明一致性用例钉住，不在运行期判。
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
  if #output > limit then
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
  }, nil
end

-- ───────────────────────────── 登记（分配 + 落盘；不叙事、不 render） ─────────────────────────────

-- 请求幂等键 → 已分配的 call_id（`@ext=` 重放）。扫 `tool.seq.*`，上界 cap()。
local function by_ext(ext)
  if ext == "" then
    return nil
  end
  local total = count_of()
  local limit = cap()
  for i = 1, total do
    if i > limit then
      break
    end
    local id = call_id_at(i)
    if id ~= nil and ext_of(id, "ext") == ext then
      return id
    end
  end
  return nil
end

local function request(tool_id, args_text, call_id, ext)
  -- ① allowlist 先于形状：未放行的工具连「形状对不对」都不该告诉调用方。
  if not contains(allowed(), tool_id) then
    return nil, fill(TEXTS.beats.tool_not_allowed, {
      tool_id = tostring(tool_id),
      allowed = allowed_text(),
    })
  end
  -- ② tool_id 形状（放行表自带形状校验，这里是纵深防御）。
  if #tool_id > MAX_TOOL_ID or tool_id:match(TOOL_ID_RE) == nil then
    return nil, fill(TEXTS.beats.tool_bad_tool_id, { tool_id = tostring(tool_id) })
  end
  -- ③ 参数。
  local pairs_out, problem = parse_args(args_text)
  if pairs_out == nil then
    return nil, fill(TEXTS.beats.tool_bad_args, { args = tostring(args_text), problem = problem })
  end
  -- ④ call_id。
  local chosen = trim(call_id or "")
  local total = count_of()
  local limit = cap()
  if chosen ~= "" then
    if #chosen > MAX_CALL_ID or chosen:match(CALL_ID_RE) == nil or RESERVED[chosen] == true then
      return nil, fill(TEXTS.beats.tool_bad_call_id, {
        call_id = chosen,
        max = tostring(MAX_CALL_ID),
      })
    end
    if exists_of(chosen) then
      return nil, fill(TEXTS.beats.tool_call_dup, {
        call_id = chosen,
        status = status_of(chosen),
      })
    end
  else
    -- 自动 id 从 count + 1 起探测空位（防「先自定义 t7、后自动分配到 t7」）。
    for step = 1, limit do
      local candidate = "t" .. tostring(total + step)
      if not exists_of(candidate) then
        chosen = candidate
        break
      end
    end
    if chosen == "" then
      return nil, fill(TEXTS.beats.tool_capacity, { cap = tostring(limit) })
    end
  end
  -- ⑤ 容量（生命周期计数上界）。
  if total >= limit then
    return nil, fill(TEXTS.beats.tool_capacity, { cap = tostring(limit) })
  end
  -- ⑥ 落盘（顺序固定，便于崩溃排查）。
  local index = total + 1
  ygg.kv.set("tool.count", index)
  ygg.kv.set("tool.seq." .. tostring(index), chosen)
  ygg.kv.set("tool." .. chosen .. ".id", chosen)
  ygg.kv.set("tool." .. chosen .. ".tool", tool_id)
  ygg.kv.set("tool." .. chosen .. ".args", args_text_of(pairs_out))
  ygg.kv.set("tool." .. chosen .. ".status", "pending")
  ygg.kv.set("tool." .. chosen .. ".code", -1)
  ygg.kv.set("tool." .. chosen .. ".output", "")
  ygg.kv.set("tool." .. chosen .. ".ext", tostring(ext or ""))
  ygg.kv.set("tool." .. chosen .. ".res_ext", "")
  ygg.kv.set("tool.pending", pending_count() + 1)
  return chosen, nil
end

-- 结果应用（幂等；不叙事、不 render）。返回 `"applied"` / `"replay", ext` / `nil, 问题串`。
local function apply_result(rec)
  local call_id = rec.call_id
  if not exists_of(call_id) then
    return nil, fill(TEXTS.beats.tool_unknown_call, { call_id = call_id })
  end
  local expected = tool_of(call_id)
  if expected ~= rec.tool_id then
    return nil, fill(TEXTS.beats.tool_result_mismatch, {
      call_id = call_id,
      expected = expected,
      got = rec.tool_id,
    })
  end
  -- 结果重放：同一个 external_id 已经应用过 = 幂等返回，一个字节都不改。
  if rec.ext ~= "" and rec.ext == ext_of(call_id, "res_ext") then
    return "replay", rec.ext
  end
  local current = status_of(call_id)
  -- 一个 call 只有一个终态：终态之后换状态是冲突（不静默覆盖），同状态只补幂等键。
  if (current == "ok" or current == "err") and rec.status ~= current then
    return nil, fill(TEXTS.beats.tool_result_mismatch, {
      call_id = call_id,
      expected = current,
      got = rec.status,
    })
  end
  ygg.kv.set("tool." .. call_id .. ".status", rec.status)
  ygg.kv.set("tool." .. call_id .. ".code", rec.code)
  ygg.kv.set("tool." .. call_id .. ".output", rec.output)
  if rec.ext ~= "" then
    ygg.kv.set("tool." .. call_id .. ".res_ext", rec.ext)
  end
  ygg.kv.set("tool.last", call_id)
  -- 只有 pending → 终态才减：重放 / 同状态重申都不许把计数减成负数。
  if current == "pending" then
    local left = pending_count() - 1
    if left < 0 then
      left = 0
    end
    ygg.kv.set("tool.pending", left)
  end
  return "applied", nil
end

-- ───────────────────────────── 请求队列（唯一编码点） ─────────────────────────────

-- `meta.tool_calls` = 稠密数组 `[{id, tool, args?}]`，序 = 分配序；**只列 pending 行**。
-- 无参数时**省略** `args` 键（Lua 空表只能读成 `[]`，与宿主的 `args: []` 容错口径不同形）。
local function flush_meta()
  local list = {}
  local total = count_of()
  local limit = cap()
  for i = 1, total do
    if i > limit then
      break
    end
    local id = call_id_at(i)
    if id ~= nil and status_of(id) == "pending" then
      local entry = { id = id, tool = tool_of(id) }
      local args = args_table(id)
      if args ~= nil then
        entry.args = args
      end
      list[#list + 1] = entry
    end
  end
  ygg.meta("tool_calls", list)
end

-- ───────────────────────────── 回执 effect（只推进闩锁 + 一行叙事） ─────────────────────────────

-- 依赖全是 KV 派生标量（AGENTS.md §9.1）：装载 / 读档 / 重启后签名一致，结果确定。
local function receipt_deps()
  local last = ygg.kv.get("tool.last")
  if type(last) ~= "string" or last == "" then
    return nil
  end
  local status = status_of(last)
  if status ~= "ok" and status ~= "err" then
    return nil
  end
  if ygg.kv.get("tool.notified." .. last) == true then
    return nil
  end
  return { last, status, code_of(last), pending_count() }
end

-- 一次性闩锁 → 天然幂等：重放结果不会第二次说出 `[TOOL SETTLED]`。
-- 绝不改 `req.*` / `prop.*` / gate / lane / stage，也不 render() / ygg.choice()（AGENTS.md §9.1）。
local function receipt_effect()
  local last = ygg.kv.get("tool.last")
  if type(last) ~= "string" or last == "" then
    return nil
  end
  ygg.kv.set("tool.notified." .. last, true)
  ygg.narrate(fill(TEXTS.beats.tool_settled, {
    call_id = last,
    status = status_of(last),
    code = tostring(code_of(last)),
  }))
  return true
end

-- ───────────────────────────── 装配面 ─────────────────────────────

function M.install(deps)
  TEXTS = deps.texts
end

function M.register()
  -- 机器动作（外部派发），不是人类里程碑动作：不进菜单、不占票据（AGENTS.md §8）。
  ygg.on("tool.call", function(payload)
    if stage() == STAGE.done then
      return reject("tool.call", TEXTS.beats.tool_after_done)
    end
    local body, marked_ext = strip_ext(payload)
    local tool_id, args_text, call_id, problem = parse_call(body)
    if problem ~= nil then
      return reject_payload("tool.call", problem)
    end
    local ext = trim(marked_ext or "")
    if ext ~= "" and not valid_ext(ext) then
      return reject_payload("tool.call", fill(TEXTS.beats.bad_payload, { action = "tool.call" }))
    end
    local replayed = by_ext(ext)
    if replayed ~= nil then
      ygg.narrate(fill(TEXTS.beats.tool_replayed, { call_id = replayed, ext = ext }))
      render()
      return
    end
    local chosen, request_problem = request(tool_id, args_text, call_id, ext)
    if chosen == nil then
      return reject_payload("tool.call", request_problem)
    end
    local stored = ygg.kv.get("tool." .. chosen .. ".args")
    ygg.narrate(fill(TEXTS.beats.tool_requested, {
      call_id = chosen,
      tool_id = tool_id,
      args = (type(stored) == "string" and stored ~= "" and stored) or "-",
      pending = tostring(pending_count()),
    }))
    flush_meta()
    render()
  end)
  ygg.on("tool.result", function(payload)
    if stage() == STAGE.done then
      return reject("tool.result", TEXTS.beats.tool_after_done)
    end
    local rec, problem = parse_result(payload)
    if rec == nil then
      return reject_payload("tool.result", problem)
    end
    local outcome, detail = apply_result(rec)
    if outcome == "replay" then
      ygg.narrate(fill(TEXTS.beats.tool_replayed, { call_id = rec.call_id, ext = detail }))
      flush_meta()
      render()
      return
    end
    if outcome == nil then
      return reject_payload("tool.result", detail)
    end
    ygg.narrate(fill(TEXTS.beats.tool_result, {
      call_id = rec.call_id,
      tool_id = rec.tool_id,
      status = rec.status,
      code = tostring(rec.code),
      output = rec.output ~= "" and rec.output or "-",
    }))
    -- render() 首行是 hooks.flush_effects()：回执 effect 在**同一回合**收敛。
    flush_meta()
    render()
  end)
  hooks.useEffect("tools.receipt", receipt_deps, receipt_effect)
end

M.CALL_ID_RE = CALL_ID_RE
M.MAX_CALL_ID = MAX_CALL_ID
M.TOOL_ID_RE = TOOL_ID_RE
M.allowed = allowed
M.allowed_text = allowed_text
M.apply_result = apply_result
M.args_table = args_table
M.call_id_at = call_id_at
M.code_of = code_of
M.flush_meta = flush_meta
M.output_of = output_of
M.parse_args = parse_args
M.parse_call = parse_call
M.parse_result = parse_result
M.pending_count = pending_count
M.request = request
M.status_of = status_of

return M
