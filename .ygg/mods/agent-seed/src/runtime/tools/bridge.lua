-- src/runtime/tools/bridge.lua —— S32 纯搬移自 tools.lua（零逻辑变更；接线见交付回执）。

local M = {}
local util = require("src.core.util")
local state = require("src.runtime.tools.state")
local TEXTS = setmetatable({}, { __index = function(_, k) return state.TEXTS[k] end })
local mod_parse = require("src.runtime.tools.parse")
local contains = util.contains
local cut = util.cut
local fill = util.fill
local split = util.split
local trim = util.trim
local CALL_ID_RE, MAX_CALL_ID, MAX_TOOL_ID, RESERVED, TOOL_ID_RE = mod_parse.CALL_ID_RE, mod_parse.MAX_CALL_ID, mod_parse.MAX_TOOL_ID, mod_parse.RESERVED, mod_parse.TOOL_ID_RE
local allowed, allowed_text, args_table, args_text_of, call_id_at, cap, count_of, exists_of, ext_of, parse_args, pending_count, status_of, tool_of, whole = mod_parse.allowed, mod_parse.allowed_text, mod_parse.args_table, mod_parse.args_text_of, mod_parse.call_id_at, mod_parse.cap, mod_parse.count_of, mod_parse.exists_of, mod_parse.ext_of, mod_parse.parse_args, mod_parse.pending_count, mod_parse.status_of, mod_parse.tool_of, mod_parse.whole


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

local function validate_call_request(tool_id, args_text, call_id)
  -- ① allowlist 先于形状：未放行的工具连「形状对不对」都不该告诉调用方。
  if not contains(allowed(), tool_id) then
    return nil, nil, nil, nil, fill(TEXTS.beats.tool_not_allowed, {
      tool_id = tostring(tool_id),
      allowed = allowed_text(),
    })
  end
  -- ② tool_id 形状（放行表自带形状校验，这里是纵深防御）。
  if #tool_id > MAX_TOOL_ID or tool_id:match(TOOL_ID_RE) == nil then
    return nil, nil, nil, nil, fill(TEXTS.beats.tool_bad_tool_id, { tool_id = tostring(tool_id) })
  end
  -- ③ 参数。
  local pairs_out, problem = parse_args(args_text)
  if pairs_out == nil then
    return nil, nil, nil, nil, fill(TEXTS.beats.tool_bad_args, { args = tostring(args_text), problem = problem })
  end
  -- ④ call_id。
  local chosen = trim(call_id or "")
  local total = count_of()
  local limit = cap()
  if chosen ~= "" then
    if #chosen > MAX_CALL_ID or chosen:match(CALL_ID_RE) == nil or RESERVED[chosen] == true then
      return nil, nil, nil, nil, fill(TEXTS.beats.tool_bad_call_id, {
        call_id = chosen,
        max = tostring(MAX_CALL_ID),
      })
    end
    if exists_of(chosen) then
      return nil, nil, nil, nil, fill(TEXTS.beats.tool_call_dup, {
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
      return nil, nil, nil, nil, fill(TEXTS.beats.tool_capacity, { cap = tostring(limit) })
    end
  end
  -- ⑤ 容量（生命周期计数上界）。
  if total >= limit then
    return nil, nil, nil, nil, fill(TEXTS.beats.tool_capacity, { cap = tostring(limit) })
  end
  return chosen, pairs_out, total, limit, nil
end

local function request(tool_id, args_text, call_id, ext)
  local chosen, pairs_out, total, _, problem = validate_call_request(tool_id, args_text, call_id)
  if chosen == nil then
    return nil, problem
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
  ygg.kv.set("tool." .. chosen .. ".truncated", false)
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
  ygg.kv.set("tool." .. call_id .. ".truncated", rec.truncated == true)
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

-- ───────────────────────────── 响应式后置效应（主动子系统） ─────────────────────────────
--
-- 触发口径：`tool.result` 落库为 ok + code 0 时，把 `output`（`k=v;k=v` 平坦映射）折叠成
-- 领域状态。6 个反应器共用一次闩锁 `tool.reacted.<call_id>`（与 `tool.notified.<id>` 分开：
-- 一个说「已说过 [TOOL SETTLED]」，一个说「已把结果折叠进领域状态」）。
-- 纪律（hooks.lua 头注释）：只写 KV + narrate；绝不 render() / ygg.choice / flow.refuse。
-- 拒绝行会带 rejection 前缀而被宿主记成失败步——探针失败是**事实报告**，不是 Mod 拒绝。

-- `k=v;k=v` → `{ k = v }`。半截段（无 `=`）静默丢弃：截断的输出只会留下不完整的尾段。
local function output_pairs(text)
  local out = {}
  if type(text) ~= "string" or text == "" then
    return out
  end
  local segments = split(text, ";")
  for i = 1, #segments do
    local key, value = cut(segments[i], "=")
    if value ~= nil then
      local name = trim(key)
      if name ~= "" then
        out[name] = trim(value)
      end
    end
  end
  return out
end

-- 整数字段（nil = 缺失 / 非整数）：误报类判据只能用「缺失」表达，不能默认成 0。
local function whole_at(map, name)
  return whole(map[name])
end

M.apply_result = apply_result
M.flush_meta = flush_meta
M.flush_outbox = flush_meta
M.output_pairs = output_pairs
M.request = request
M.validate_call_request = validate_call_request
M.by_ext = by_ext
M.whole_at = whole_at

return M
