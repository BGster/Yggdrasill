-- src/runtime/tools/reactive.lua —— S32 纯搬移自 tools.lua（零逻辑变更；接线见交付回执）。

local M = {}
local config = require("src.core.config")
local gates = require("src.domain.gates")
local lanes = require("src.domain.lanes")
local props = require("src.domain.proposals")
local reqs = require("src.domain.requirements")
local util = require("src.core.util")
local wire = require("src.domain.wire")
local state = require("src.runtime.tools.state")
local TEXTS = setmetatable({}, { __index = function(_, k) return state.TEXTS[k] end })
local mod_bridge = require("src.runtime.tools.bridge")
local mod_parse = require("src.runtime.tools.parse")
local num = config.num
local text_of = config.text_of
local gate_problem = gates.gate_problem
local LANES = lanes.LANES
local req_do_conflict = props.req_do_conflict
local RUN_PHASE = reqs.RUN_PHASE
local job_capacity = reqs.job_capacity
local req_closed = reqs.req_closed
local req_conflict = reqs.req_conflict
local req_id = reqs.req_id
local req_index_of = reqs.req_index_of
local req_lane = reqs.req_lane
local req_status = reqs.req_status
local req_sync_conflicts = reqs.req_sync_conflicts
local run_phase = reqs.run_phase
local contains = util.contains
local fill = util.fill
local split = util.split
local trim = util.trim
local artifact_path_ok = wire.artifact_path_ok
local MAX_ARTIFACT_CHECKS, MAX_ARTIFACT_PATH, MAX_RADAR_FILES, SHA_DISPLAY, VERIFY_TOOL_SOURCE = mod_parse.MAX_ARTIFACT_CHECKS, mod_parse.MAX_ARTIFACT_PATH, mod_parse.MAX_RADAR_FILES, mod_parse.SHA_DISPLAY, mod_parse.VERIFY_TOOL_SOURCE
local output_pairs, whole_at = mod_bridge.output_pairs, mod_bridge.whole_at
local args_table, code_of, output_of, pending_count, status_of, tool_of, whole = mod_parse.args_table, mod_parse.code_of, mod_parse.output_of, mod_parse.pending_count, mod_parse.status_of, mod_parse.tool_of, mod_parse.whole


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

-- 跳泳道冲突雷达：**同池**两条泳道的探针文件列表有交集 → 走 req_do_conflict（与人类路径
-- 同一函数），因此自动进 req_conflict_total → 阻塞派活与收口。一次探针最多抬一条冲突
--（防一条探针刷屏）。r1 分池：跨池文件树无关，雷达只比同池对（跨池同名文件不再误报）。
local function radar_raise(pool, k, files_text)
  local mine = split(trim(files_text), ",")
  for i = 1, #mine do
    mine[i] = trim(mine[i])
  end
  if #mine == 0 or #mine > MAX_RADAR_FILES then
    return false
  end
  local self_id = LANES.req_of(pool, k)
  local self_index = req_index_of(self_id)
  if self_index == nil or req_closed(self_index) then
    return false
  end
  for other = 1, job_capacity() do
    if other ~= k then
      local peer_id = LANES.req_of(pool, other)
      local peer_index = req_index_of(peer_id)
      if peer_id ~= "" and peer_id ~= self_id and peer_index ~= nil
        and not req_closed(peer_index)
        and req_conflict(self_index) == "" and req_conflict(peer_index) == "" then
        local theirs = split(text_of(LANES.key(pool, other, "probe.files"), ""), ",")
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
            lane = LANES.label(pool, k),
            second = peer_id,
            peer = LANES.label(pool, other),
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

-- r1 分池：池归属从**请求快照**（`tool.<call_id>.args` 的 repo= 段）反查——宿主输出行
-- 不携带 --repo（claim/release/probe 的输出键序是冻结的宿主契约），请求行才有；
-- 同步路径 try_sync_call 也写同一 args KV，两路同源。默认工具（无 repo 段）= 默认池
-- ("")；带 repo 段但反查不到已注册池 = fail-closed 拒绝（返回 nil 池 + 可读原因）。
local function pool_of_call(pairs, call_id)
  local repo = trim(pairs.repo or "")
  if repo == "" then
    local args = args_table(call_id)
    if args ~= nil then
      repo = trim(args.repo or "")
    end
  end
  if repo == "" then
    return "", nil
  end
  local pool = LANES.pool_of_path(repo)
  if pool == nil then
    return nil, fill(TEXTS.beats.lane_pool_unknown, { pool = repo })
  end
  return pool, nil
end

REACTORS["git.worktree_claim"] = function(pairs, call_id)
  local k = whole_at(pairs, "lane")
  local index = req_index_of(pairs.req)
  local branch = trim(pairs.branch or "")
  local branch_problem = LANES.branch_problem(branch)
  local pool, pool_problem = pool_of_call(pairs, call_id)
  local reason = nil
  if k == nil or k < 1 or k > job_capacity() then
    reason = fill(TEXTS.beats.lane_bad_k, { k = tostring(pairs.lane or "?"), max = tostring(job_capacity()) })
  elseif pool_problem ~= nil then
    reason = pool_problem
  elseif index == nil then
    reason = fill(TEXTS.beats.req_unknown, { id = tostring(pairs.req or "") })
  elseif req_closed(index) then
    reason = fill(TEXTS.beats.lane_claim_closed, { id = req_id(index), status = req_status(index) })
  elseif pairs.clean ~= "1" then
    reason = fill(TEXTS.beats.lane_claim_dirty, { lane = LANES.label(pool, k) })
  elseif branch == "" then
    reason = fill(TEXTS.beats.lane_branch_invalid, { branch = "(missing)" })  -- 挂载层缺省拒绝（不再自动生成默认名）
  elseif branch_problem ~= nil then
    reason = branch_problem
  elseif LANES.req_of(pool, k) ~= "" and LANES.req_of(pool, k) ~= req_id(index) then
    reason = fill(TEXTS.beats.lane_claim_occupied, { lane = LANES.label(pool, k), id = LANES.req_of(pool, k) })
  elseif req_lane(index) > 0 and (req_lane(index) ~= k or LANES.req_pool(index) ~= pool) then
    reason = fill(TEXTS.beats.lane_claim_bound, { id = req_id(index), lane = LANES.label(LANES.req_pool(index), req_lane(index)) })
  end
  if reason ~= nil then
    ygg.narrate(fill(TEXTS.beats.worktree_claim_refused, {
      lane = tostring(pairs.lane or "?"),
      reason = reason,
    }))
    return false
  end
  -- 与 `lane.claim` 同函数同键（含 `since` 的 tick 分配与 `req.<i>.lane`/`lane_pool` 回写）。
  LANES.bind(pool, k, index, pairs.branch or "", pairs.base or "", "1")
  local wt_path = trim(pairs.path or "")
  if wt_path ~= "" then
    ygg.kv.set(LANES.key(pool, k, "path"), wt_path)
  end
  ygg.narrate(fill(TEXTS.beats.worktree_claimed, {
    lane = LANES.label(pool, k),
    req = req_id(index),
    branch = LANES.text_or(pairs.branch, TEXTS.beats.lane_field_none),
    base = LANES.text_or(pairs.base, TEXTS.beats.lane_field_none),
  }))
  return true
end

REACTORS["git.worktree_release"] = function(pairs, call_id)
  local k = whole_at(pairs, "lane")
  if k == nil or k < 1 or k > job_capacity() then
    ygg.narrate(fill(TEXTS.beats.worktree_release_refused, {
      reason = fill(TEXTS.beats.lane_bad_k, { k = tostring(pairs.lane or "?"), max = tostring(job_capacity()) }),
    }))
    return false
  end
  local pool, pool_problem = pool_of_call(pairs, call_id)
  if pool_problem ~= nil then
    ygg.narrate(fill(TEXTS.beats.worktree_release_refused, { reason = pool_problem }))
    return false
  end
  if pairs.released ~= "1" then
    return false
  end
  ygg.kv.set(LANES.key(pool, k, "path"), "")
  if LANES.req_of(pool, k) == "" then
    ygg.narrate(fill(TEXTS.beats.worktree_release_idle, { lane = LANES.label(pool, k) }))
    return false
  end
  -- **不能**写 `LANES.release(k)`：那个函数的入参是 req index，传 k 会释放错需求或空转。
  LANES.release_lane(pool, k)
  ygg.narrate(fill(TEXTS.beats.worktree_released, { lane = LANES.label(pool, k) }))
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
  local pool, pool_problem = pool_of_call(pairs, call_id)
  if pool_problem ~= nil then
    ygg.narrate(fill(TEXTS.beats.worktree_release_refused, { reason = pool_problem }))
    return false
  end
  local commits = whole_at(pairs, "commits")
  local clean = pairs.clean or ""
  ygg.kv.set(LANES.key(pool, k, "probe.commits"), commits or -1)
  ygg.kv.set(LANES.key(pool, k, "probe.clean"), clean)
  ygg.kv.set(LANES.key(pool, k, "probe.head"), pairs.head or "")
  ygg.kv.set(LANES.key(pool, k, "probe.files"), pairs.files or "")
  ygg.kv.set(LANES.key(pool, k, "probe.call"), call_id)
  if commits == nil or commits == 0 or clean ~= "1" then
    ygg.narrate(fill(TEXTS.beats.lane_probe_failed, {
      lane = LANES.label(pool, k),
      commits = tostring(commits or -1),
      clean = (clean ~= "" and clean) or TEXTS.beats.lane_field_none,
    }))
  else
    ygg.narrate(fill(TEXTS.beats.lane_probe_recorded, {
      lane = LANES.label(pool, k),
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
  radar_raise(pool, k, pairs.files or "")
  return true
end

-- r1 分池：池化工具与默认工具共用同一折叠语义（池归属由请求 repo 段反查，见
-- pool_of_call）——只是同函数别名注册，不写第二套实现。
REACTORS["git.worktree_claim_pool"] = REACTORS["git.worktree_claim"]
REACTORS["git.worktree_release_pool"] = REACTORS["git.worktree_release"]
REACTORS["git.lane_probe_pool"] = REACTORS["git.lane_probe"]

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
  -- 盘点幕（settle 微状态）是 verify 证据的唯一直落点（§9.2c：机证与人证同前件）。
  if run_phase() ~= RUN_PHASE.settle then
    ygg.narrate(fill(TEXTS.beats.verify_suite_blocked, {
      reason = fill(TEXTS.beats.stage_expected, { expected = "盘点" }),
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

-- 依赖全是 KV 派生标量（docs/design/kernel/state-machine.md §2.9）：装载 / 读档 / 重启后签名一致，结果确定。
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
-- 绝不改 `req.*` / `prop.*` / gate / lane / stage，也不 render() / ygg.choice()（docs/design/kernel/state-machine.md §2.9.2）。
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

M.reactive_deps = reactive_deps
M.reactive_effect = reactive_effect
M.reactive_tool_ids = reactive_tool_ids
M.receipt_deps = receipt_deps
M.receipt_effect = receipt_effect
M.record_artifact = record_artifact
M.verify_artifacts = verify_artifacts
M.REACTORS = REACTORS
M.short_sha = short_sha

return M
