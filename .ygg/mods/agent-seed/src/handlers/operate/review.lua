-- src/handlers/operate/review.lua —— 批循环四幕：coordinate.submit / verify.submit / settlement.* / review.note。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 注册面全部在 install() 内（派发序 = 入口 install 调用序）。
-- D-MA23：协调/盘点/发布是 run.phase 微状态——coordinate.submit 不再转场（R8）；
-- 盘点四项齐备由响应式效应自动转发布幕（R9）；review.note 前置=发布已声明（R10），
-- 未武装=批结算（batch+1 回协调），run.retire 武装=退役终局（唯一真转场 live→retired）。

local M = {}

local config = require("src.core.config")
local flow = require("src.runtime.flow")
local gates = require("src.domain.gates")
local hooks = require("src.core.hooks")
local choice = require("src.handlers.operate.choice")
local release_mod = require("src.handlers.operate.release")
local render_mod = require("src.view.render")
local reqs = require("src.domain.requirements")
local tools = require("src.runtime.tools")
local util = require("src.core.util")
local wire = require("src.domain.wire")

local TEXTS

local goal_authorization, num, strict_gates, run_retire_armed, text_of =
  config.goal_authorization, config.num, config.strict_gates, config.run_retire_armed, config.text_of
local advance, phase_guard, refuse, reject, reject_payload =
  flow.advance, flow.phase_guard, flow.refuse, flow.reject, flow.reject_payload
local gate_problem, settlement_problem =
  gates.gate_problem, gates.settlement_problem
local HUMAN =
  choice.HUMAN
local render =
  render_mod.receipt
local reset_settlement =
  render_mod.reset_settlement
local reset_release =
  release_mod.reset_release
local RUN_PHASE, req_append, req_capacity, req_closed_reason, req_count, req_id, req_key, req_open_total, req_status, req_title, reqs_R_OPEN, run_phase, stage =
  reqs.RUN_PHASE, reqs.req_append, reqs.req_capacity, reqs.req_closed_reason, reqs.req_count, reqs.req_id, reqs.req_key, reqs.req_open_total, reqs.req_status, reqs.req_title, reqs.R_OPEN, reqs.run_phase, reqs.stage
local cut, fill, split, trim =
  util.cut, util.fill, util.split, util.trim
local probe_payload_ok =
  wire.probe_payload_ok

-- ============================================================================
-- harness 机证 cross-check（r3 / D-MA26）
-- 与 settlement.evolution 的 artifact-probe 同构：机证与人证同级不越权（§9.2c）。
-- 未注册 tools.root / 参数缺 / 脚本不可执行 → 人证旧路径 + 可读提示，绝不拒绝结算。
-- 机证与人证矛盾 → reject_payload（零状态写入）。mod 沙箱读不到进程环境，
-- 参数只能来自局 KV（config.set tools.session / tools.sessions_root）。
-- ============================================================================

-- tools.root 是否已注册（机证启用开关）。
local function tools_registered()
  return trim(text_of("settings.tools.root", "")) ~= ""
end

-- settle 相位进入时的一次性提醒（coordinate.submit 尾部调用）。
local function tools_unregistered_hint()
  if not tools_registered() then
    ygg.narrate(TEXTS.beats.tools_unregistered_hint)
  end
end

-- 会话参数三元组：session（主会话 id = todo 的 session / sessions 的 parent）、sessions_root。
local function tools_session_params()
  local session = trim(text_of("settings.tools.session", ""))
  local sessions_root = trim(text_of("settings.tools.sessions_root", ""))
  return session, sessions_root
end

-- 读取同步调用的输出 k=v 对；工具未挂载/失败返回 nil + 问题文案键。
local function harness_output(call_id)
  if call_id == nil or call_id == "" then
    return nil, "unmounted"
  end
  if tools.status_of(call_id) ~= "ok" then
    return nil, "failed"
  end
  return tools.output_pairs(tools.output_of(call_id)), nil
end

-- todo 机证：与人证 remaining=0 比对。返回 problem（nil = 通过或回退）。
-- 通过时写 source=machine 并叙事；回退时写 source=human 由调用方决定。
local function cross_check_todo()
  if not tools_registered() then
    ygg.narrate(fill(TEXTS.beats.tools_root_missing, { item = "todo" }))
    return nil, "human"
  end
  local session, sessions_root = tools_session_params()
  if session == "" then
    ygg.narrate(TEXTS.beats.todo_session_missing)
    return nil, "human"
  end
  local call_id = tools.try_sync_call(
    "harness.todo-check",
    { session = session, sessions_root = sessions_root },
    "session=" .. session,
    "",
    ""
  )
  local pairs_map, err = harness_output(call_id)
  if pairs_map == nil then
    ygg.narrate(fill(TEXTS.beats.tools_script_missing, { item = "todo", detail = tostring(err) }))
    return nil, "human"
  end
  local remaining = tonumber(pairs_map.remaining)
  if remaining == nil then
    ygg.narrate(fill(TEXTS.beats.tools_script_missing, { item = "todo", detail = "no remaining key" }))
    return nil, "human"
  end
  if remaining > 0 then
    return fill(TEXTS.beats.tools_check_refused, {
      item = "todo",
      detail = "machine remaining=" .. tostring(remaining) .. " but declared 0",
    }), nil
  end
  ygg.narrate(fill(TEXTS.beats.tools_check_passed, { item = "todo", detail = "remaining=0" }))
  return nil, "machine"
end

-- sessions 机证：与人证 remaining=0（即全部归档，matching=0）比对。
local function cross_check_sessions()
  if not tools_registered() then
    ygg.narrate(fill(TEXTS.beats.tools_root_missing, { item = "sessions" }))
    return nil, "human"
  end
  local session, sessions_root = tools_session_params()
  if session == "" then
    ygg.narrate(TEXTS.beats.sessions_parent_missing)
    return nil, "human"
  end
  local call_id = tools.try_sync_call(
    "harness.sessions-count",
    { parent = session, sessions_root = sessions_root },
    "parent=" .. session,
    "",
    ""
  )
  local pairs_map, err = harness_output(call_id)
  if pairs_map == nil then
    ygg.narrate(fill(TEXTS.beats.tools_script_missing, { item = "sessions", detail = tostring(err) }))
    return nil, "human"
  end
  local matching = tonumber(pairs_map.matching)
  if matching == nil then
    ygg.narrate(fill(TEXTS.beats.tools_script_missing, { item = "sessions", detail = "no matching key" }))
    return nil, "human"
  end
  if matching > 0 then
    return fill(TEXTS.beats.tools_check_refused, {
      item = "sessions",
      detail = "machine matching=" .. tostring(matching) .. " but declared 0",
    }), nil
  end
  ygg.narrate(fill(TEXTS.beats.tools_check_passed, { item = "sessions", detail = "matching=0" }))
  return nil, "machine"
end

-- 局末两项清理的落库（独立动作与 settlement.evolution 可选段共用）：写 KV + 叙事，不 render。
local function apply_sessions(moved, remaining, destination)
  ygg.kv.set("settlement.sessions.moved", moved)
  ygg.kv.set("settlement.sessions.remaining", remaining)
  ygg.kv.set("settlement.sessions.destination", destination)
  ygg.narrate(fill(TEXTS.beats.settlement_sessions_ok, {
    moved = tostring(moved),
    destination = destination,
  }))
end

local function apply_todo(remaining, verified_by)
  ygg.kv.set("settlement.todo.remaining", remaining)
  ygg.kv.set("settlement.todo.verified_by", verified_by)
  ygg.narrate(fill(TEXTS.beats.settlement_todo_ok, { verified_by = verified_by }))
end

-- 盘点四项齐备（验证 / 演进登记 / 会话清理 / 待办清理）。
local function settle_four_ready()
  if run_phase() ~= RUN_PHASE.settle then
    return false
  end
  if ygg.kv.get("verify.passed") ~= true then
    return false
  end
  if trim(text_of("settlement.evolution.summary", "")) == "" then
    return false
  end
  if text_of("settlement.sessions.destination", "") == "" then
    return false
  end
  if num("settlement.todo.remaining", -1) ~= 0 then
    return false
  end
  return true
end

-- r3 / D-MA25：批结算尾部自动追加下一批的「维护更新 docs wiki」需求（draft 态，进
-- 入派发队列，由主代理在下一批协调幕 requirement.dispatch——draft 行永不带 mark，
-- auto_dispatch 不碰它）。防重复：注册表里存在 open ∧ 无 closed_reason ∧（maintenance
-- 标记 ∨ 标题精确等于规范标题）的行则跳过——closed_reason 过滤保证 done / split /
-- merged / dropped 的历史维护行不压制新批追加。尽力而为：去重命中或容量满只发可读
-- 叙事（前缀非拒绝前缀，不计失败步），绝不使批结算本身失败。退役路径不调用（本函数
-- 只挂在未武装批结算尾部）。
local function append_maintenance_req(batch)
  for i = 1, req_count() do
    if req_status(i) == reqs_R_OPEN
      and req_closed_reason(i) == ""
      and (
        ygg.kv.get(req_key(i, "maintenance"))
        or req_title(i) == TEXTS.beats.maintenance_req_title
      )
    then
      ygg.narrate(fill(TEXTS.beats.maintenance_req_skipped_dedup, { id = req_id(i) }))
      return
    end
  end
  if req_open_total() >= req_capacity() then
    ygg.narrate(fill(TEXTS.beats.maintenance_req_skipped_full, { cap = tostring(req_capacity()) }))
    return
  end
  local index = req_append(
    TEXTS.beats.maintenance_req_title,
    TEXTS.beats.maintenance_req_desc,
    "",
    {},
    ""
  )
  ygg.kv.set(req_key(index, "maintenance"), "1")
  ygg.kv.set("run.maintenance.last", req_id(index))
  ygg.narrate(fill(TEXTS.beats.maintenance_req_added, {
    id = req_id(index),
    title = TEXTS.beats.maintenance_req_title,
    batch = tostring(batch),
  }))
end

function M.install(deps)
  TEXTS = deps.texts
  -- 盘点收口效应（R9）：四项齐备 → run.phase="release"（自动转发布幕，不新增事件、
  -- 不渲染、非门禁动作——「唯一声明后继」的又一实例，§2.9 四不变量全满足）。
  hooks.useEffect("settle.close", function()
    if not settle_four_ready() then
      return nil
    end
    return { "settle_ready" }
  end, function()
    if not settle_four_ready() then
      return false
    end
    ygg.kv.set("run.phase", "release")
    ygg.narrate(TEXTS.beats.settle_ready)
    return true
  end)
  ygg.on("coordinate.submit", function(payload)
    if not HUMAN.gated("coordinate.submit") then
      return
    end
    if not phase_guard("coordinate.submit", { RUN_PHASE.coordinate }) then
      return
    end
    if not probe_payload_ok(payload) then
      return reject_payload("coordinate.submit", fill(TEXTS.beats.probe_rejected, {
        action = "coordinate.submit",
        got = tostring(payload),
        usage = TEXTS.beats.coordinate_usage,
      }))
    end
    local problem = gate_problem()
    if problem ~= nil then
      return refuse(TEXTS.beats.req_wait_rejected, "coordinate.submit", problem)
    end
    ygg.kv.set("verify.passed", false)
    -- 每次进入结算都重新登记三项任务，避免沿用上一轮过期证据。
    reset_settlement()
    -- r9 / R8 批语义：coordinate.submit 是**批边界**（不是「走向终局的第一步」）；
    -- settle 是 run.phase 微状态，内核 FSM 不转场。
    ygg.kv.set("run.phase", "settle")
    ygg.narrate(fill(TEXTS.beats.coordinate_ok, {
      count = tostring(req_count()),
      batch = tostring(num("run.batch", 1)),
    }))
    -- r3 / D-MA26：进入结算时若未注册 tools.root，给一次性机证提醒（非门禁、不阻塞）。
    tools_unregistered_hint()
    render()
  end)
  ygg.on("verify.submit", function(payload)
    if not HUMAN.gated("verify.submit") then
      return
    end
    if not phase_guard("verify.submit", { RUN_PHASE.settle }) then
      return
    end
    local all_green = true
    local report = ""
    if type(payload) == "boolean" then
      all_green = payload
    elseif type(payload) == "string" then
      local raw = trim(payload)
      if raw == "false" then
        all_green = false
      elseif raw ~= "true" then
        local parsed = nil
        for _, segment in ipairs(split(raw, ";")) do
          local key, value = cut(trim(segment), "=")
          key = trim(key)
          value = trim(value or "")
          if key == "all_green" then
            parsed = value == "true"
          elseif key == "report" then
            report = value
          end
        end
        if parsed == nil then
          return reject_payload("verify.submit", fill(TEXTS.beats.bad_payload, { action = "verify.submit" }))
        end
        all_green = parsed
      end
    else
      return reject_payload("verify.submit", fill(TEXTS.beats.bad_payload, { action = "verify.submit" }))
    end
    if all_green then
      local problem = gate_problem()
      if problem ~= nil then
        ygg.narrate(fill(TEXTS.beats.req_gate_block, { action = "verify.submit=true", detail = problem }))
        render()
        return
      end
      ygg.kv.set("verify.passed", true)
      ygg.kv.set("verify.last", TEXTS.beats.verify_green_record)
      ygg.narrate(fill(TEXTS.beats.verify_ok, {
        count = tostring(req_count()),
        batch = tostring(num("run.batch", 1)),
      }))
      render()
      return
    end
    -- R9：整批退回协调——run.phase=""（不转场）并清 settlement 与 release 键。
    ygg.kv.set("verify.passed", false)
    reset_settlement()
    reset_release()
    ygg.kv.set("run.phase", "")
    local failures = num("verify.failures", 0) + 1
    ygg.kv.set("verify.failures", failures)
    ygg.kv.set("verify.last", report ~= "" and report or TEXTS.beats.verify_failed_record)
    if strict_gates() then
      ygg.narrate(fill(TEXTS.beats.verify_fail, { failures = tostring(failures) }))
    else
      ygg.narrate(fill(TEXTS.beats.gate_bypass, { failures = tostring(failures) }))
      ygg.kv.set("verify.passed", true)
    end
    render()
  end)
  -- 三项结算登记都是自声明：进化复核只校形状与计数，会话 / Todo 只校形状与 remaining=0。
  ygg.on("settlement.evolution", function(payload)
    if not phase_guard("settlement.evolution", { RUN_PHASE.settle }) then
      return
    end
    if ygg.kv.get("verify.passed") ~= true then
      return refuse(TEXTS.beats.req_wait_rejected, "settlement.evolution", TEXTS.beats.settlement_verify_first)
    end
    -- 分层：parse 只认载荷形状（project 的三条精确诊断在这里），normalize 校跨字段完整性。
    local registration, registration_err = wire.parse_settlement_evolution(payload)
    if registration == nil then
      return reject_payload("settlement.evolution", registration_err)
    end
    local _, _, registered_candidates, registered_classes = wire.evolution_settings()
    local record, record_err = wire.normalize_evolution(
      registration.artifact,
      registration.summary,
      registration.segments.c or registered_candidates,
      registration.segments.cls or registered_classes
    )
    if record == nil then
      return reject_payload("settlement.evolution", record_err)
    end
    -- E2（r2 rev2）：登记前对非空 artifact 做挂载探针核验（fs.artifact_probe 同步工具桥）。
    -- 空 artifact（c=0/0）保持合法；缺失 / 空 / 读错 / 工具未挂载 → 可读拒绝登记：
    -- 不写任何 settlement.* KV，也不触发归档副作用（auto_archive 只在登记成功后才可能触发）。
    if trim(record.artifact) ~= "" then
      local call_id = tools.try_sync_call(
        "fs.artifact_probe",
        { path = record.artifact },
        "path=" .. record.artifact,
        "",
        ""
      )
      local artifact_problem = nil
      if call_id == nil or call_id == "" then
        artifact_problem = fill(TEXTS.beats.settlement_artifact_no_vfs, { artifact = record.artifact })
      elseif tools.status_of(call_id) ~= "ok" then
        artifact_problem = fill(TEXTS.beats.settlement_artifact_read_error, { artifact = record.artifact })
      else
        local pairs_map = tools.output_pairs(tools.output_of(call_id))
        local bytes = tonumber(pairs_map.bytes) or 0
        if pairs_map.exists ~= "1" then
          artifact_problem = fill(TEXTS.beats.settlement_artifact_absent, { artifact = record.artifact })
        elseif bytes <= 0 then
          artifact_problem = fill(TEXTS.beats.settlement_artifact_empty, { artifact = record.artifact })
        end
      end
      if artifact_problem ~= nil then
        return reject_payload("settlement.evolution", artifact_problem)
      end
    end
    local candidates = tostring(record.kept) .. "/" .. tostring(record.screened)
    local classes_text = wire.format_classes(record.classes)
    ygg.kv.set("settlement.evolution.artifact", record.artifact)
    ygg.kv.set("settlement.evolution.summary", record.summary)
    ygg.kv.set("settlement.evolution.candidates", candidates)
    ygg.kv.set("settlement.evolution.classes", classes_text)
    ygg.narrate(fill(TEXTS.beats.settlement_evolution_ok, {
      artifact = record.artifact ~= "" and record.artifact or TEXTS.beats.evolution_artifact_none,
      summary = record.summary,
      candidates = candidates,
      classes = classes_text ~= "" and classes_text or TEXTS.beats.evolution_classes_none,
    }))
    -- 同一事务里把局末两项清理也登记完（可选段）：省两个回合，且不复活独立动作的旧文法。
    if registration.sessions ~= nil then
      apply_sessions(
        registration.sessions.moved,
        registration.sessions.remaining,
        registration.sessions.destination
      )
    elseif tostring(ygg.kv.get("settlement.sessions.remaining") or "") ~= "0" then
      -- r1 rev1：不再静默自动归档（会伪造主会话身份），改可读提示由用户决定安全清理路径。
      ygg.narrate(fill(TEXTS.beats.settlement_sessions_unsetted_hint, {
        remaining = tostring(ygg.kv.get("settlement.sessions.remaining") or "?"),
      }))
    end
    if registration.todo ~= nil then
      apply_todo(registration.todo.remaining, registration.todo.verified_by)
    end
    render()
  end)
  -- 局末两项清理：subagent 会话与主会话 Todo。Mod 读不到会话与 Todo API，真实清理由主会话做。
  ygg.on("settlement.sessions", function(payload)
    if not phase_guard("settlement.sessions", { RUN_PHASE.settle }) then
      return
    end
    local sessions, sessions_problem = wire.parse_sessions_payload(payload)
    if sessions == nil then
      return reject_payload("settlement.sessions", sessions_problem)
    end
    -- r3 / D-MA26：机证 cross-check（与人证同级）。矛盾 → 拒（零状态写入）；
    -- 未注册/参数缺/脚本不可执行 → 人证旧路径 + 提示（handler 内已叙事）。
    local cc_problem, cc_source = cross_check_sessions()
    if cc_problem ~= nil then
      return reject_payload("settlement.sessions", cc_problem)
    end
    apply_sessions(sessions.moved, sessions.remaining, sessions.destination)
    ygg.kv.set("settlement.sessions.source", cc_source == "machine" and "machine:harness.sessions-count" or "human")
    render()
  end)
  ygg.on("settlement.todo", function(payload)
    if not phase_guard("settlement.todo", { RUN_PHASE.settle }) then
      return
    end
    -- 载荷 "remaining|verified_by"：恰好两段；remaining 必须恰为 0；verified_by 非空。
    -- Lua 沙箱没有 Todo API：这里只记「主会话复核到 0 项」这条断言，不声称清理条数。
    local todo, todo_problem = wire.parse_todo_payload(payload)
    if todo == nil then
      return reject_payload("settlement.todo", todo_problem)
    end
    -- r3 / D-MA26：机证 cross-check（与人证同级）。矛盾 → 拒（零状态写入）；
    -- 未注册/参数缺/脚本不可执行 → 人证旧路径 + 提示（handler 内已叙事）。
    local cc_problem, cc_source = cross_check_todo()
    if cc_problem ~= nil then
      return reject_payload("settlement.todo", cc_problem)
    end
    apply_todo(todo.remaining, todo.verified_by)
    ygg.kv.set("settlement.todo.source", cc_source == "machine" and "machine:harness.todo-check" or "human")
    render()
  end)
  ygg.on("review.note", function(payload)
    if not HUMAN.gated("review.note") then
      return
    end
    if not phase_guard("review.note", { RUN_PHASE.release }) then
      return
    end
    local problem = gate_problem()
    if problem ~= nil then
      ygg.narrate(fill(TEXTS.beats.req_gate_block, { action = "review.note", detail = problem }))
      render()
      return
    end
    if strict_gates() and ygg.kv.get("verify.passed") ~= true then
      ygg.narrate(TEXTS.beats.req_gate_verify)
      render()
      return
    end
    if tostring(ygg.kv.get("settlement.sessions.remaining") or "") ~= "0" then
      -- r1 rev1：不再静默自动归档；保留未结算阻断，改可读提示由用户决定安全清理路径。
      ygg.narrate(fill(TEXTS.beats.settlement_sessions_unsetted_hint, {
        remaining = tostring(ygg.kv.get("settlement.sessions.remaining") or "?"),
      }))
    end
    -- 三项结算任务未登记（或 remaining ≠ 0）就不允许收口。
    local settlement = settlement_problem()
    if settlement ~= nil then
      ygg.narrate(fill(TEXTS.beats.req_gate_block, { action = "review.note", detail = settlement }))
      render()
      return
    end
    -- 运行期进入 goal 的局：收口前必须有 run 级授权声明（静态 data/config.json 预设 goal 不校验）。
    if ygg.kv.get("settings.play.mode") == "goal" and goal_authorization() == "" then
      ygg.narrate(fill(TEXTS.beats.req_gate_block, {
        action = "review.note",
        detail = TEXTS.beats.goal_auth_required,
      }))
      render()
      return
    end
    -- R10 新前置：发布已声明（master_sha ∨ skipped_reason 任一非空）。
    local master_sha = text_of("run.release.master_sha", "")
    local skipped_reason = text_of("run.release.skipped_reason", "")
    if master_sha == "" and skipped_reason == "" then
      ygg.narrate(fill(TEXTS.beats.req_gate_block, {
        action = "review.note",
        detail = TEXTS.beats.release_required,
      }))
      render()
      return
    end
    local note, note_err = wire.parse_review_note(payload)
    if note == nil then
      return reject_payload("review.note", note_err)
    end
    local lesson = note.lesson
    if lesson == "" then
      lesson = TEXTS.beats.default_lesson
    end
    local score = TEXTS.tuning.default_score
    local candidate = note.score_text
    if candidate ~= "" then
      local parsed = tonumber(candidate)
      if parsed == nil or parsed < 1 or parsed > 5 then
        return reject_payload("review.note", fill(TEXTS.beats.bad_score, { got = candidate }))
      end
      score = math.floor(parsed)
    end
    -- 进化声明只从已登记的 settlement 四键原样归档，不从 note/settings 重算。
    ygg.kv.set("review.lesson", lesson)
    ygg.kv.set("review.score", score)
    ygg.kv.set("review.evolution.artifact", text_of("settlement.evolution.artifact", ""))
    ygg.kv.set("review.evolution.summary", text_of("settlement.evolution.summary", ""))
    ygg.kv.set("review.evolution.candidates", text_of("settlement.evolution.candidates", "0/0"))
    ygg.kv.set("review.evolution.classes", text_of("settlement.evolution.classes", ""))
    if run_retire_armed() then
      -- 退役路径（r9 唯一终局，仅 run.retire 武装可达）：写 ending + advance(retired)。
      -- [ENDING 行由 RENDER[retired] 在 node:render 上叙事，本 handler 只留归档叙事。
      ygg.kv.set("ending", reqs.STAGE.retired)
      ygg.narrate(fill(TEXTS.beats.review_ok, { score = tostring(score), lesson = lesson }))
      advance(reqs.STAGE.retired)
      return
    end
    -- 批收口路径（常驻默认）：滚动单槽已归档 → 清 release 声明 → run.batch+1 → 复位
    -- verify/settlement → run.phase=""（回协调，不转场）。
    -- 红线：绝不写 `ending`、叙事行首绝不含 [ENDING 前缀（宿主按行首前缀判终局，测试钉死）。
    reset_release()
    local batch = num("run.batch", 1)
    ygg.kv.set("run.batch", batch + 1)
    ygg.kv.set("verify.passed", false)
    reset_settlement()
    ygg.kv.set("run.phase", "")
    ygg.narrate(fill(TEXTS.beats.review_batch_ok, {
      batch = tostring(batch),
      next = tostring(batch + 1),
      score = tostring(score),
      lesson = lesson,
    }))
    -- r3 / D-MA25：追加发生在全部结算检查之后——本批再无任何 gate_problem 读点，
    -- 不阻塞本批；新需求落在下一批协调幕的派发队列（§9.1 四条件：handler 事务内
    -- 普通派生写，非 effect）。
    append_maintenance_req(batch)
    render()
  end)
end

M.settle_four_ready = settle_four_ready

return M
