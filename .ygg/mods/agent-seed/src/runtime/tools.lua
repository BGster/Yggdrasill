-- src/runtime/tools.lua —— 工具桥的 KV 面与回执面（`tool.*` 唯一写方）+ 三个事件 handler + 两条 effect。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；资源由入口 install(deps) 注入，
-- 注册面全部在 register() 内（与 src/view/render.lua 同构：render 也是 src/ 下的纯模块）。
--
-- 模型：`tool.call` 只**登记**请求（`tool.<id>.*` 行 + `meta.tool_calls` 队列），真正执行在宿主；
-- 结果以普通事件 `tool.result` 回来。Mod 永不做 IO（无 process / 网络 / 文件系统）；唯一文件面是
-- 宿主中介的 `ygg.fs` VFS（`vfs.write` + artifact 核验），缺席时整体降级为「只记账本」。
--
-- 反向禁止：`src/view/render.lua` 不得 require 本模块（立刻成环）——摘要字段走 KV 读 `tool.pending`。
--
-- 本模块写的 KV 前缀：
--   tool.count / tool.pending / tool.last / tool.seq.* / tool.<id>.* / tool.notified.*
-- 响应式子系统（本模块的响应式分区；与既有写方共存，逐键登记）：
--   tool.reacted.*（本模块唯一）
--   lane.<k>.{req,branch,base,since,clean}（= `lane.claim` 的等价写面，共用 src.lanes 的 LANES.bind）
--   lane.<k>.probe.*（本模块唯一）
--   req.<i>.conflict{,_peer,_note,_source}（经 src/proposals.req_do_conflict 这唯一函数写）
--   artifact.*（本模块唯一；`record_artifact` 是唯一写点）
--   verify.passed / verify.report / verify.source（src/handlers/operate/review.lua 写 verify.last / verify.failures）
--   settlement.sessions.*（src/handlers/operate/review.lua 的 apply_sessions 是另一条写路径）

local M = {}

local config = require("src.core.config")
local flow = require("src.runtime.flow")
local gates = require("src.domain.gates")
local hooks = require("src.core.hooks")
local lanes = require("src.domain.lanes")
local props = require("src.domain.proposals")
local render_mod = require("src.view.render")
local reqs = require("src.domain.requirements")
local util = require("src.core.util")
local wire = require("src.domain.wire")

local TEXTS

local effective, num, text_of =
  config.effective, config.num, config.text_of
local reject, reject_payload =
  flow.reject, flow.reject_payload
local gate_problem =
  gates.gate_problem
local LANES =
  lanes.LANES
local req_do_conflict =
  props.req_do_conflict
local render =
  render_mod.receipt
local STAGE, job_capacity, req_closed, req_conflict, req_id, req_index_of, req_lane, req_status, req_sync_conflicts, stage =
  reqs.STAGE, reqs.job_capacity, reqs.req_closed, reqs.req_conflict, reqs.req_id, reqs.req_index_of, reqs.req_lane, reqs.req_status, reqs.req_sync_conflicts, reqs.stage
local contains, cut, fill, join, split, trim =
  util.contains, util.cut, util.fill, util.join, util.split, util.trim
local artifact_path_ok, strip_ext, valid_ext =
  wire.artifact_path_ok, wire.strip_ext, wire.valid_ext

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
local SESSIONS_TOOL_SOURCE = "tool:harness.archive_sessions"

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

local function short_sha(text)
  local raw = trim(text)
  if raw == "" then
    return TEXTS.beats.lane_field_none
  end
  if #raw <= SHA_DISPLAY then
    return raw
  end
  return raw:sub(1, SHA_DISPLAY)
end

local function artifact_capacity()
  local value = TEXTS.tuning.artifact_capacity
  if type(value) == "number" and value > 0 then
    return math.floor(value)
  end
  return 64
end

-- `artifact.*` 的唯一写方（路径形状由调用方先校）：同一个 path 重写只更新值，不增计数。
-- 满额 → 叙事 `[ARTIFACT LEDGER FULL]` 且不写（fail-soft：账本满不得炸回合）。
local function record_artifact(path, bytes, sha, source)
  if type(path) ~= "string" then
    return false
  end
  local key = trim(path)
  if key == "" then
    return false
  end
  local total = num("artifact.count", 0)
  if type(ygg.kv.get("artifact." .. key .. ".bytes")) ~= "number" then
    local limit = artifact_capacity()
    if total >= limit then
      ygg.narrate(fill(TEXTS.beats.artifact_ledger_full, {
        path = key,
        cap = tostring(limit),
      }))
      return false
    end
    total = total + 1
    ygg.kv.set("artifact.count", total)
    ygg.kv.set("artifact.seq." .. tostring(total), key)
  end
  ygg.kv.set("artifact." .. key .. ".bytes", bytes)
  ygg.kv.set("artifact." .. key .. ".sha256", sha or "")
  ygg.kv.set("artifact." .. key .. ".source", source or "")
  return true
end

-- 跳泳道冲突雷达：两条泳道的探针文件列表有交集 → 走 req_do_conflict（与人类路径同一函数），
-- 因此自动进 req_conflict_total → 阻塞派活与收口。一次探针最多抬一条冲突（防一条探针刷屏）。
local function radar_raise(k, files_text)
  local mine = split(trim(files_text), ",")
  for i = 1, #mine do
    mine[i] = trim(mine[i])
  end
  if #mine == 0 or #mine > MAX_RADAR_FILES then
    return false
  end
  local self_id = LANES.req_of(k)
  local self_index = req_index_of(self_id)
  if self_index == nil or req_closed(self_index) then
    return false
  end
  for other = 1, job_capacity() do
    if other ~= k then
      local peer_id = LANES.req_of(other)
      local peer_index = req_index_of(peer_id)
      if peer_id ~= "" and peer_id ~= self_id and peer_index ~= nil
        and not req_closed(peer_index)
        and req_conflict(self_index) == "" and req_conflict(peer_index) == "" then
        local theirs = split(text_of(LANES.key(other, "probe.files"), ""), ",")
        for i = 1, #theirs do
          theirs[i] = trim(theirs[i])
        end
        local hit = nil
        for i = 1, #mine do
          if mine[i] ~= "" and contains(theirs, mine[i]) then
            hit = mine[i]
            break
          end
        end
        if hit ~= nil then
          local note = fill(TEXTS.beats.conflict_radar_note, { file = hit })
          req_do_conflict(self_index, peer_index, note, "radar")
          req_sync_conflicts()
          ygg.narrate(fill(TEXTS.beats.conflict_radar, {
            first = self_id,
            k = tostring(k),
            second = peer_id,
            other = tostring(other),
            file = hit,
          }))
          return true
        end
      end
    end
  end
  return false
end

-- 反应器逐个规格（`tool_id -> function(pairs, call_id) -> changed`）。
-- 原因文案复用既有 beat（`lane_*` / `req_unknown`），不新增第二套措辞。
local REACTORS = {}

REACTORS["git.worktree_claim"] = function(pairs)
  local k = whole_at(pairs, "lane")
  local index = req_index_of(pairs.req)
  local branch = trim(pairs.branch or "")
  local branch_problem = LANES.branch_problem(branch)
  local reason = nil
  if k == nil or k < 1 or k > job_capacity() then
    reason = fill(TEXTS.beats.lane_bad_k, { k = tostring(pairs.lane or "?"), max = tostring(job_capacity()) })
  elseif index == nil then
    reason = fill(TEXTS.beats.req_unknown, { id = tostring(pairs.req or "") })
  elseif req_closed(index) then
    reason = fill(TEXTS.beats.lane_claim_closed, { id = req_id(index), status = req_status(index) })
  elseif pairs.clean ~= "1" then
    reason = fill(TEXTS.beats.lane_claim_dirty, { k = tostring(k) })
  elseif branch == "" then
    reason = fill(TEXTS.beats.lane_branch_invalid, { branch = "(missing)" })  -- 挂载层缺省拒绝（不再自动生成默认名）
  elseif branch_problem ~= nil then
    reason = branch_problem
  elseif LANES.req_of(k) ~= "" and LANES.req_of(k) ~= req_id(index) then
    reason = fill(TEXTS.beats.lane_claim_occupied, { k = tostring(k), id = LANES.req_of(k) })
  elseif req_lane(index) > 0 and req_lane(index) ~= k then
    reason = fill(TEXTS.beats.lane_claim_bound, { id = req_id(index), k = tostring(req_lane(index)) })
  end
  if reason ~= nil then
    ygg.narrate(fill(TEXTS.beats.worktree_claim_refused, {
      k = tostring(pairs.lane or "?"),
      reason = reason,
    }))
    return false
  end
  -- 与 `lane.claim` 同函数同键（含 `since` 的 tick 分配与 `req.<i>.lane` 回写）。
  LANES.bind(k, index, pairs.branch or "", pairs.base or "", "1")
  local wt_path = trim(pairs.path or "")
  if wt_path ~= "" then
    ygg.kv.set(LANES.key(k, "path"), wt_path)
  end
  ygg.narrate(fill(TEXTS.beats.worktree_claimed, {
    k = tostring(k),
    req = req_id(index),
    branch = LANES.text_or(pairs.branch, TEXTS.beats.lane_field_none),
    base = LANES.text_or(pairs.base, TEXTS.beats.lane_field_none),
  }))
  return true
end

REACTORS["git.worktree_release"] = function(pairs)
  local k = whole_at(pairs, "lane")
  if k == nil or k < 1 or k > job_capacity() then
    ygg.narrate(fill(TEXTS.beats.worktree_release_refused, {
      reason = fill(TEXTS.beats.lane_bad_k, { k = tostring(pairs.lane or "?"), max = tostring(job_capacity()) }),
    }))
    return false
  end
  if pairs.released ~= "1" then
    return false
  end
  ygg.kv.set(LANES.key(k, "path"), "")
  if LANES.req_of(k) == "" then
    ygg.narrate(fill(TEXTS.beats.worktree_release_idle, { k = tostring(k) }))
    return false
  end
  -- **不能**写 `LANES.release(k)`：那个函数的入参是 req index，传 k 会释放错需求或空转。
  LANES.release_lane(k)
  ygg.narrate(fill(TEXTS.beats.worktree_released, { k = tostring(k) }))
  return true
end

REACTORS["git.lane_probe"] = function(pairs, call_id)
  local k = whole_at(pairs, "lane")
  if k == nil or k < 1 or k > job_capacity() then
    ygg.narrate(fill(TEXTS.beats.lane_probe_bad_lane, {
      lane = tostring(pairs.lane or "?"),
      max = tostring(job_capacity()),
    }))
    return false
  end
  local commits = whole_at(pairs, "commits")
  local clean = pairs.clean or ""
  ygg.kv.set(LANES.key(k, "probe.commits"), commits or -1)
  ygg.kv.set(LANES.key(k, "probe.clean"), clean)
  ygg.kv.set(LANES.key(k, "probe.head"), pairs.head or "")
  ygg.kv.set(LANES.key(k, "probe.files"), pairs.files or "")
  ygg.kv.set(LANES.key(k, "probe.call"), call_id)
  if commits == nil or commits == 0 or clean ~= "1" then
    ygg.narrate(fill(TEXTS.beats.lane_probe_failed, {
      k = tostring(k),
      commits = tostring(commits or -1),
      clean = (clean ~= "" and clean) or TEXTS.beats.lane_field_none,
    }))
  else
    ygg.narrate(fill(TEXTS.beats.lane_probe_recorded, {
      k = tostring(k),
      commits = tostring(commits),
      clean = clean,
      head = short_sha(pairs.head),
      files = (pairs.files ~= nil and pairs.files ~= "") and pairs.files or TEXTS.beats.lane_field_none,
    }))
  end
  if ygg.kv.get("tool." .. call_id .. ".truncated") == true then
    ygg.narrate(fill(TEXTS.beats.lane_probe_truncated, { call_id = call_id }))
    return true
  end
  radar_raise(k, pairs.files or "")
  return true
end

REACTORS["fs.artifact_probe"] = function(pairs)
  local path = trim(pairs.path or "")
  if pairs.exists ~= "1" then
    ygg.narrate(fill(TEXTS.beats.artifact_missing, { path = path }))
    return false
  end
  local bytes = whole_at(pairs, "bytes")
  if bytes == nil or bytes <= 0 then
    ygg.narrate(fill(TEXTS.beats.artifact_empty, { path = path }))
    return false
  end
  if not artifact_path_ok(path) then
    ygg.narrate(fill(TEXTS.beats.artifact_path_rejected, { path = path, max = tostring(MAX_ARTIFACT_PATH) }))
    return false
  end
  if not record_artifact(path, bytes, pairs.sha256 or "", "probe") then
    return false
  end
  ygg.narrate(fill(TEXTS.beats.artifact_verified, {
    path = path,
    bytes = tostring(bytes),
    sha = short_sha(pairs.sha256),
  }))
  return true
end

REACTORS["gate.verify_suite"] = function(pairs)
  local summary = trim(pairs.summary or "")
  if stage() ~= STAGE.review then
    ygg.narrate(fill(TEXTS.beats.verify_suite_blocked, {
      reason = fill(TEXTS.beats.stage_expected, { expected = STAGE.review }),
    }))
    return false
  end
  if pairs.passed ~= "1" then
    ygg.narrate(fill(TEXTS.beats.verify_suite_failed, {
      summary = (summary ~= "" and summary) or TEXTS.beats.default_gate_report,
    }))
    return false
  end
  -- 与 `verify.submit all_green=true` 同一前件：机证不是「绕过门禁」的边。
  local blocked = gate_problem()
  if blocked ~= nil then
    ygg.narrate(fill(TEXTS.beats.verify_suite_blocked, { reason = blocked }))
    return false
  end
  ygg.kv.set("verify.passed", true)
  ygg.kv.set("verify.report", summary)
  -- 溯源键：机证与人证同级不等强，报告里不得声称机证更强。
  ygg.kv.set("verify.source", VERIFY_TOOL_SOURCE)
  ygg.narrate(fill(TEXTS.beats.verify_suite_passed, {
    summary = (summary ~= "" and summary) or TEXTS.beats.verify_green_record,
  }))
  return true
end

REACTORS["harness.archive_sessions"] = function(pairs)
  local dest = trim(pairs.dest or "")
  if stage() ~= STAGE.review then
    ygg.narrate(fill(TEXTS.beats.sessions_archive_refused, {
      remaining = (pairs.remaining ~= nil and pairs.remaining ~= "") and pairs.remaining or "?",
      dest = TEXTS.beats.lane_field_none,
    }))
    return false
  end
  if pairs.remaining ~= "0" or dest == "" then
    ygg.narrate(fill(TEXTS.beats.sessions_archive_refused, {
      remaining = (pairs.remaining ~= nil and pairs.remaining ~= "") and pairs.remaining or "?",
      dest = (dest ~= "" and dest) or TEXTS.beats.lane_field_none,
    }))
    return false
  end
  local moved = whole_at(pairs, "moved") or 0
  ygg.kv.set("settlement.sessions.moved", moved)
  ygg.kv.set("settlement.sessions.remaining", 0)
  ygg.kv.set("settlement.sessions.destination", dest)
  ygg.kv.set("settlement.sessions.source", SESSIONS_TOOL_SOURCE)
  ygg.narrate(fill(TEXTS.beats.sessions_archived, { moved = tostring(moved), dest = dest }))
  return true
end

local function reactive_tool_ids()
  local ids = {}
  for id in next, REACTORS do
    ids[#ids + 1] = id
  end
  return ids
end

-- 依赖全是 KV 派生标量：装载 / 读档 / 重启后签名一致；不认识的 tool id 不占闩锁、不产生 deps。
local function reactive_deps()
  local last = ygg.kv.get("tool.last")
  if type(last) ~= "string" or last == "" then
    return nil
  end
  if status_of(last) ~= "ok" or code_of(last) ~= 0 then
    return nil
  end
  if ygg.kv.get("tool.reacted." .. last) == true then
    return nil
  end
  local tool = tool_of(last)
  if REACTORS[tool] == nil then
    return nil
  end
  return { last, tool }
end

-- 闩锁先落：反应器中途 return false 也不重入（一次性语义）。
local function reactive_effect()
  local last = ygg.kv.get("tool.last")
  if type(last) ~= "string" or last == "" then
    return false
  end
  local tool = tool_of(last)
  local reactor = REACTORS[tool]
  if reactor == nil then
    return false
  end
  ygg.kv.set("tool.reacted." .. last, true)
  local pairs_out = output_pairs(output_of(last))
  if next(pairs_out) == nil then
    ygg.narrate(fill(TEXTS.beats.reactive_malformed, { call_id = last, tool_id = tool }))
    return false
  end
  if reactor(pairs_out, last) == true then
    return true
  end
  return false
end

-- `proposal.submit` 与 `vfs.write` 共用的同步核验（0ms；`ygg.fs` 缺席则整体 no-op，返回 0）。
local function verify_artifacts(paths_text)
  local fs = ygg.fs
  if type(fs) ~= "table" or type(fs.stat) ~= "function" then
    return 0
  end
  local seen, count = 0, 0
  local list = split(trim(paths_text or ""), ",")
  for i = 1, #list do
    local path = trim(list[i])
    if path ~= "" and artifact_path_ok(path) and seen < MAX_ARTIFACT_CHECKS then
      seen = seen + 1
      local ok, stat = pcall(fs.stat, path)
      if ok and type(stat) == "table" and stat.exists == true then
        local bytes = whole(stat.bytes) or 0
        if bytes > 0 and record_artifact(path, bytes, tostring(stat.sha256 or ""), "probe") then
          ygg.narrate(fill(TEXTS.beats.artifact_verified, {
            path = path,
            bytes = tostring(bytes),
            sha = short_sha(stat.sha256),
          }))
          count = count + 1
        end
      end
    end
  end
  return count
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

local function auto_claim_worktree(lane, req_index)
  if not contains(allowed(), "git.worktree_claim") then
    return nil
  end
  local k = whole(lane)
  if k == nil or k <= 0 or req_index == nil then
    return nil
  end
  local id = req_id(req_index)
  -- r2 rev2：默认分支名生成机制已删除——分支由主 Agent 预挂牌自报（账本 `lane.<k>.branch`），
  -- 空 = 可读拒绝不发起宿主调用（宿主先执行、反应器后折叠，发空名调用会让宿主 git 报错，
  -- 叙事不可读——必须在 mod 层拦）。
  local branch = text_of(LANES.key(k, "branch"), "")
  if branch == "" then
    ygg.narrate(fill(TEXTS.beats.worktree_claim_refused, {
      k = tostring(k),
      reason = fill(TEXTS.beats.lane_branch_invalid, { branch = "(missing)" }),
    }))
    return nil
  end
  local args_map = {
    lane = tostring(k),
    req = id,
    branch = branch,
  }
  local raw_args = "lane=" .. tostring(k) .. ";req=" .. id .. ";branch=" .. branch
  return try_sync_call("git.worktree_claim", args_map, raw_args, "", "")
end

local function auto_probe_lane(lane)
  if not contains(allowed(), "git.lane_probe") then
    return nil
  end
  local k = whole(lane)
  if k == nil or k <= 0 then
    return nil
  end
  local base = text_of(LANES.key(k, "base"), "HEAD")
  if base == "" then
    base = "HEAD"
  end
  local branch = text_of(LANES.key(k, "branch"), "")
  if branch == "" then
    -- r2 rev2：不再用 `feature/<holder>` 兑底虚构名字；账本分支空 = 可读提示并跳过探针
    -- （探针是警告面，不阻塞——但也不该拿编造的分支名去探测）。
    ygg.narrate(fill(TEXTS.beats.lane_branch_invalid, { branch = "(missing)" }))
    return nil
  end
  local args_map = {
    lane = tostring(k),
    base = base,
    branch = branch,
  }
  local raw_args = "lane=" .. tostring(k) .. ";base=" .. base .. ";branch=" .. branch
  return try_sync_call("git.lane_probe", args_map, raw_args, "", "")
end

local function auto_release_worktree(lane)
  if not contains(allowed(), "git.worktree_release") then
    return nil
  end
  local k = whole(lane)
  if k == nil or k <= 0 then
    return nil
  end
  local args_map = {
    lane = tostring(k),
  }
  local raw_args = "lane=" .. tostring(k)
  return try_sync_call("git.worktree_release", args_map, raw_args, "", "")
end

local function auto_archive_sessions()
  if not contains(allowed(), "harness.archive_sessions") then
    return nil
  end
  if tostring(ygg.kv.get("settlement.sessions.remaining") or "") == "0" then
    return nil
  end
  -- r1 rev1 fail-closed：run.id / run.parent 未登记时不伪造主会话身份、不发归档请求。
  -- 宿主按子串匹配选文件，伪造 parent（如回落 "default"）可能把主会话一起 rename 进 trash。
  -- 这里只给可读叙事说明缺什么为何停，绝不写伪造的 remaining=0；安全路径由用户显式决定
  -- （带已核验 parent/batch 的显式 harness.archive_sessions，或登记 settlement.sessions）。
  local batch = text_of("run.id", "")
  local parent = text_of("run.parent", "")
  if batch == "" or parent == "" then
    ygg.narrate(fill(TEXTS.beats.sessions_archive_held, {
      batch = (batch ~= "" and batch) or TEXTS.beats.lane_field_none,
      parent = (parent ~= "" and parent) or TEXTS.beats.lane_field_none,
    }))
    return nil
  end
  local args_map = {
    parent = parent,
    batch = batch,
  }
  local raw_args = "parent=" .. parent .. ";batch=" .. batch
  return try_sync_call("harness.archive_sessions", args_map, raw_args, "", "")
end

-- ───────────────────────────── 装配面 ─────────────────────────────

function M.install(deps)
  TEXTS = deps.texts
end

local function handle_call(payload)
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
  -- 挂载同步执行前先过与 request() 同源的 validate_call_request 校验门（未放行/坏形状/重复 id/超容一律先拒绝）。
  local validated_id, pairs_out, _, _, req_problem = validate_call_request(tool_id, args_text, call_id)
  if validated_id == nil then
    return reject_payload("tool.call", req_problem)
  end
  local args_map = {}
  for i = 1, #pairs_out do
    args_map[pairs_out[i].k] = pairs_out[i].v
  end
  local norm_args = args_text_of(pairs_out)
  local synced = try_sync_call(tool_id, args_map, norm_args, validated_id, ext)
  if synced ~= nil then
    flush_meta()
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
end

function M.register()
  -- 机器动作（外部派发），不是人类里程碑动作：不进菜单、不占票据（AGENTS.md §8）。
  ygg.on("tool.call", handle_call)
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
  -- 机器动作（外部派发）：把 artifact 写盘委托给宿主 VFS（Mod 自身不做 IO）。
  -- 载荷 `artifacts/<name>.md|<content>`（首个 `|` 之前是路径，其后全部是内容）。
  -- 无论宿主是否提供 `ygg.fs`，都落 `artifact.*` 账本——否则同一份代码在「宿主管线」与
  -- 「无 ygg.fs 的宿主」下言行不一致；区别只在叙事行（`[VFS WRITTEN]` vs `[VFS LEDGER ONLY]`）。
  ygg.on("vfs.write", function(payload)
    if stage() == STAGE.done then
      return reject("vfs.write", TEXTS.beats.tool_after_done)
    end
    if type(payload) ~= "string" then
      return reject_payload("vfs.write", fill(TEXTS.beats.bad_payload, { action = "vfs.write" }))
    end
    local path, content = cut(payload, "|")
    path = trim(path or "")
    content = content or ""
    if path == "" or content == "" or #content > MAX_VFS_CONTENT then
      return reject_payload("vfs.write", fill(TEXTS.beats.vfs_usage, { max = tostring(MAX_VFS_CONTENT) }))
    end
    if not artifact_path_ok(path) then
      return reject_payload("vfs.write", fill(TEXTS.beats.vfs_path_rejected, {
        path = path,
        max = tostring(MAX_ARTIFACT_PATH),
      }))
    end
    local fs = ygg.fs
    local bytes, sha, wrote = #content, "", false
    if type(fs) == "table" and type(fs.write) == "function" then
      local ok, result = pcall(fs.write, path, content)
      if not ok or type(result) ~= "table" then
        return refuse(TEXTS.beats.req_wait_rejected, "vfs.write", fill(TEXTS.beats.vfs_write_failed, {
          path = path,
          reason = TEXTS.beats.rejected_default,
        }))
      end
      bytes = whole(result.bytes) or bytes
      sha = tostring(result.sha256 or "")
      wrote = true
    end
    record_artifact(path, bytes, sha, "vfs")
    if wrote then
      ygg.narrate(fill(TEXTS.beats.vfs_written, {
        path = path,
        bytes = tostring(bytes),
        sha = short_sha(sha),
      }))
    else
      ygg.narrate(fill(TEXTS.beats.vfs_ledger_only, { path = path, bytes = tostring(bytes) }))
    end
    render()
  end)
  -- 两条 effect 同处 render() 入口的同一个 flush_effects：回执行先说，领域折叠紧随其后。
  hooks.useEffect("tools.receipt", receipt_deps, receipt_effect)
  hooks.useEffect("tools.reactive", reactive_deps, reactive_effect)
end

M.CALL_ID_RE = CALL_ID_RE
M.MAX_ARTIFACT_PATH = MAX_ARTIFACT_PATH
M.MAX_CALL_ID = MAX_CALL_ID
M.MAX_VFS_CONTENT = MAX_VFS_CONTENT
M.TOOL_ID_RE = TOOL_ID_RE
M.allowed = allowed
M.allowed_text = allowed_text
M.apply_result = apply_result
M.args_table = args_table
M.artifact_path_ok = artifact_path_ok
M.auto_archive_sessions = auto_archive_sessions
M.auto_claim_worktree = auto_claim_worktree
M.auto_probe_lane = auto_probe_lane
M.auto_release_worktree = auto_release_worktree
M.call_id_at = call_id_at
M.code_of = code_of
M.flush_meta = flush_meta
M.flush_outbox = flush_meta
M.handle_call = handle_call
M.output_of = output_of
M.output_pairs = output_pairs
M.parse_args = parse_args
M.parse_call = parse_call
M.parse_result = parse_result
M.pending_count = pending_count
M.reactive_deps = reactive_deps
M.reactive_effect = reactive_effect
M.reactive_tool_ids = reactive_tool_ids
M.receipt_deps = receipt_deps
M.receipt_effect = receipt_effect
M.record_artifact = record_artifact
M.request = request
M.settled_count = settled_count
M.status_of = status_of
M.try_sync_call = try_sync_call
M.validate_call_request = validate_call_request
M.verify_artifacts = verify_artifacts

return M
