-- src/runtime/tools/sync.lua —— S32 纯搬移自 tools.lua（零逻辑变更；接线见交付回执）。

local M = {}
local util = require("src.core.util")
local state = require("src.runtime.tools.state")
local TEXTS = setmetatable({}, { __index = function(_, k) return state.TEXTS[k] end })
local mod_bridge = require("src.runtime.tools.bridge")
local mod_parse = require("src.runtime.tools.parse")
local mod_reactive = require("src.runtime.tools.reactive")
local fill = util.fill
local join = util.join
local trim = util.trim
local REACTORS = mod_reactive.REACTORS
local output_pairs = mod_bridge.output_pairs
local cap, count_of, exists_of, output_max, whole = mod_parse.cap, mod_parse.count_of, mod_parse.exists_of, mod_parse.output_max, mod_parse.whole


-- ───────────────────────────── 同步工具调用（挂载 HostToolRunner 时单回合收口） ─────────────────────────────

local function settled_count()
  local value = whole(ygg.kv.get("tool.settled_count"))
  if value == nil or value < 0 then
    return 0
  end
  return value
end

local function serialize_data_map(data)
  if type(data) ~= "table" then
    return "", {}
  end
  local keys = {}
  local map = {}
  for k, v in next, data do
    local key = trim(tostring(k))
    if key ~= "" then
      keys[#keys + 1] = key
      map[key] = trim(tostring(v or ""))
    end
  end
  for i = 2, #keys do
    local cur = keys[i]
    local j = i - 1
    while j >= 1 and keys[j] > cur do
      keys[j + 1] = keys[j]
      j = j - 1
    end
    keys[j + 1] = cur
  end
  if #keys == 1 and keys[1] == "output" then
    local raw = map["output"] or ""
    local parsed = output_pairs(raw)
    for k, v in next, parsed do
      map[k] = v
    end
    return raw, map
  end
  local parts = {}
  for i = 1, #keys do
    local k = keys[i]
    parts[#parts + 1] = k .. "=" .. map[k]
  end
  return join(parts, ";"), map
end

local function allocate_call_id(call_id, total, limit)
  local chosen = trim(call_id or "")
  if chosen ~= "" then
    return chosen
  end
  for step = 1, limit do
    local candidate = "t" .. tostring(total + step)
    if not exists_of(candidate) then
      return candidate
    end
  end
  return ""
end

local function try_sync_call(tool_id, args_map, raw_args_text, call_id, ext)
  if type(ygg.tool) ~= "table" or type(ygg.tool.call) ~= "function" then
    return nil
  end
  local total = count_of()
  local limit = cap()
  if total >= limit then
    return nil
  end
  local chosen = allocate_call_id(call_id, total, limit)
  if chosen == "" then
    return nil
  end
  local ok_call, res = pcall(ygg.tool.call, tool_id, args_map or {})
  if not ok_call or type(res) ~= "table" or res.unmounted == true then
    return nil
  end
  local is_ok = res.ok == true
  local status = is_ok and "ok" or "error"
  local code_str = is_ok and "0" or "1"
  local output_raw, pairs_map = "", {}
  if is_ok and type(res.data) == "table" then
    output_raw, pairs_map = serialize_data_map(res.data)
  else
    output_raw = tostring(res.error or "")
  end
  output_raw = output_raw:gsub("[\r\n]", " / ")
  local out_limit = output_max()
  local truncated = false
  if #output_raw > out_limit then
    truncated = true
    local dropped = #output_raw - (out_limit - 3)
    output_raw = output_raw:sub(1, out_limit - 3) .. "..." .. fill(TEXTS.beats.tool_output_truncated, { dropped = tostring(dropped) })
  end
  local index = total + 1
  ygg.kv.set("tool.count", index)
  ygg.kv.set("tool.seq." .. tostring(index), chosen)
  ygg.kv.set("tool." .. chosen .. ".id", chosen)
  ygg.kv.set("tool." .. chosen .. ".tool", tool_id)
  ygg.kv.set("tool." .. chosen .. ".args", raw_args_text or "")
  ygg.kv.set("tool." .. chosen .. ".status", status)
  ygg.kv.set("tool." .. chosen .. ".code", code_str)
  ygg.kv.set("tool." .. chosen .. ".output", output_raw)
  ygg.kv.set("tool." .. chosen .. ".ext", tostring(ext or ""))
  ygg.kv.set("tool." .. chosen .. ".res_ext", "")
  ygg.kv.set("tool." .. chosen .. ".truncated", truncated)
  ygg.kv.set("tool." .. chosen .. ".notified", "1")
  ygg.kv.set("tool." .. chosen .. ".reacted", "1")
  ygg.kv.set("tool.notified." .. chosen, true)
  ygg.kv.set("tool.reacted." .. chosen, true)
  ygg.kv.set("tool.last", chosen)
  ygg.kv.set("tool.settled_count", settled_count() + 1)

  ygg.narrate(fill(TEXTS.beats.tool_settled, {
    call_id = chosen,
    status = status,
    code = code_str,
  }))
  if is_ok and type(res.data) == "table" then
    local reactor = REACTORS[tool_id]
    if reactor ~= nil then
      reactor(pairs_map, chosen)
    end
  end
  return chosen
end

M.settled_count = settled_count
M.try_sync_call = try_sync_call

return M
