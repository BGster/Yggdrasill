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
--   lane.<pool>.<k>.{req,branch,base,since,clean}（= `lane.claim` 的等价写面，共用 src.domain.lanes 的
--     LANES.bind；pool == "" = 默认池，键形沿用旧版 `lane.<k>.*`）
--   lane.<pool>.<k>.probe.*（本模块唯一；探针与雷达均池内作用）
--   req.<i>.conflict{,_peer,_note,_source}（经 src/proposals.req_do_conflict 这唯一函数写）
--   artifact.*（本模块唯一；`record_artifact` 是唯一写点）
--   verify.passed / verify.report / verify.source（src/handlers/operate/review.lua 写 verify.last / verify.failures）
--   settlement.sessions.*（src/handlers/operate/review.lua 的 apply_sessions 是另一条写路径）

local M = {}

local flow = require("src.runtime.flow")
local hooks = require("src.core.hooks")
local render_mod = require("src.view.render")
local reqs = require("src.domain.requirements")
local util = require("src.core.util")
local wire = require("src.domain.wire")

local TEXTS

local reject, reject_payload =
  flow.reject, flow.reject_payload
local render =
  render_mod.receipt
local STAGE, stage =
  reqs.STAGE, reqs.stage
local cut, fill, trim =
  util.cut, util.fill, util.trim
local artifact_path_ok, strip_ext, valid_ext =
  wire.artifact_path_ok, wire.strip_ext, wire.valid_ext
local state = require("src.runtime.tools.state")
local mod_parse = require("src.runtime.tools.parse")
local mod_bridge = require("src.runtime.tools.bridge")
local mod_reactive = require("src.runtime.tools.reactive")
local mod_sync = require("src.runtime.tools.sync")
local mod_auto = require("src.runtime.tools.auto")
local MAX_ARTIFACT_PATH, MAX_VFS_CONTENT = mod_parse.MAX_ARTIFACT_PATH, mod_parse.MAX_VFS_CONTENT
local apply_result, by_ext, flush_meta, request, validate_call_request = mod_bridge.apply_result, mod_bridge.by_ext, mod_bridge.flush_meta, mod_bridge.request, mod_bridge.validate_call_request
local args_text_of, parse_call, parse_result, pending_count, whole = mod_parse.args_text_of, mod_parse.parse_call, mod_parse.parse_result, mod_parse.pending_count, mod_parse.whole
local reactive_deps, reactive_effect, receipt_deps, receipt_effect, record_artifact, short_sha = mod_reactive.reactive_deps, mod_reactive.reactive_effect, mod_reactive.receipt_deps, mod_reactive.receipt_effect, mod_reactive.record_artifact, mod_reactive.short_sha
local try_sync_call = mod_sync.try_sync_call


-- ───────────────────────────── 装配面 ─────────────────────────────

function M.install(deps)
  state.TEXTS = deps.texts
  TEXTS = deps.texts
end

local function handle_call(payload)
  if stage() == STAGE.retired then
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
  -- 机器动作（外部派发），不是人类里程碑动作：不进菜单、不占票据（openwiki/operating/acts/coordinate.md 协调幕）。
  ygg.on("tool.call", handle_call)
  ygg.on("tool.result", function(payload)
    if stage() == STAGE.retired then
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
    if stage() == STAGE.retired then
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

M.artifact_path_ok = artifact_path_ok
M.handle_call = handle_call


for _, part in ipairs({ mod_parse, mod_bridge, mod_reactive, mod_sync, mod_auto }) do
  for k, v in pairs(part) do
    M[k] = v
  end
end

return M
