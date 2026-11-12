-- src/handlers/operate/review.lua —— 协调与复盘：coordinate.submit / verify.submit / settlement.* / review.note。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 注册面全部在 install() 内（派发序 = 入口 install 调用序）。

local M = {}

local config = require("src.core.config")
local flow = require("src.runtime.flow")
local gates = require("src.domain.gates")
local choice = require("src.handlers.operate.choice")
local render_mod = require("src.view.render")
local reqs = require("src.domain.requirements")
local tools = require("src.runtime.tools")
local util = require("src.core.util")
local wire = require("src.domain.wire")

local TEXTS

local goal_authorization, num, strict_gates, run_retire_armed, text_of =
  config.goal_authorization, config.num, config.strict_gates, config.run_retire_armed, config.text_of
local advance, refuse, reject, reject_payload =
  flow.advance, flow.refuse, flow.reject, flow.reject_payload
local gate_problem, settlement_problem =
  gates.gate_problem, gates.settlement_problem
local HUMAN =
  choice.HUMAN
local render =
  render_mod.receipt
local reset_settlement =
  render_mod.reset_settlement
local STAGE, req_count, stage =
  reqs.STAGE, reqs.req_count, reqs.stage
local cut, fill, split, trim =
  util.cut, util.fill, util.split, util.trim
local probe_payload_ok =
  wire.probe_payload_ok



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

function M.install(deps)
  TEXTS = deps.texts
  ygg.on("coordinate.submit", function(payload)
    if not HUMAN.gated("coordinate.submit") then
      return
    end
    local current = stage()
    if current ~= STAGE.coordinate then
      return reject("coordinate.submit", fill(TEXTS.beats.stage_expected, { expected = STAGE.coordinate }))
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
    -- r9 批语义：coordinate.submit 是**批边界**（不是「走向终局的第一步」）。
    ygg.narrate(fill(TEXTS.beats.coordinate_ok, {
      count = tostring(req_count()),
      batch = tostring(num("run.batch", 1)),
    }))
    advance(STAGE.review)
  end)
  ygg.on("verify.submit", function(payload)
    if not HUMAN.gated("verify.submit") then
      return
    end
    local current = stage()
    if current ~= STAGE.review then
      return reject("verify.submit", fill(TEXTS.beats.stage_expected, { expected = STAGE.review }))
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
    ygg.kv.set("verify.passed", false)
    reset_settlement()
    local failures = num("verify.failures", 0) + 1
    ygg.kv.set("verify.failures", failures)
    ygg.kv.set("verify.last", report ~= "" and report or TEXTS.beats.verify_failed_record)
    if strict_gates() then
      ygg.narrate(fill(TEXTS.beats.verify_fail, { failures = tostring(failures) }))
      advance(STAGE.coordinate)
    else
      ygg.narrate(fill(TEXTS.beats.gate_bypass, { failures = tostring(failures) }))
      ygg.kv.set("verify.passed", true)
      render()
    end
  end)
  -- 三项结算登记都是自声明：进化复核只校形状与计数，会话 / Todo 只校形状与 remaining=0。
  ygg.on("settlement.evolution", function(payload)
    local current = stage()
    if current ~= STAGE.review then
      return reject("settlement.evolution", fill(TEXTS.beats.stage_expected, { expected = STAGE.review }))
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
    local current = stage()
    if current ~= STAGE.review then
      return reject("settlement.sessions", fill(TEXTS.beats.stage_expected, { expected = STAGE.review }))
    end
    local sessions, sessions_problem = wire.parse_sessions_payload(payload)
    if sessions == nil then
      return reject_payload("settlement.sessions", sessions_problem)
    end
    apply_sessions(sessions.moved, sessions.remaining, sessions.destination)
    render()
  end)
  ygg.on("settlement.todo", function(payload)
    local current = stage()
    if current ~= STAGE.review then
      return reject("settlement.todo", fill(TEXTS.beats.stage_expected, { expected = STAGE.review }))
    end
    -- 载荷 "remaining|verified_by"：恰好两段；remaining 必须恰为 0；verified_by 非空。
    -- Lua 沙箱没有 Todo API：这里只记「主会话复核到 0 项」这条断言，不声称清理条数。
    local todo, todo_problem = wire.parse_todo_payload(payload)
    if todo == nil then
      return reject_payload("settlement.todo", todo_problem)
    end
    apply_todo(todo.remaining, todo.verified_by)
    render()
  end)
  ygg.on("review.note", function(payload)
    if not HUMAN.gated("review.note") then
      return
    end
    local current = stage()
    if current ~= STAGE.review then
      return reject("review.note", fill(TEXTS.beats.stage_expected, { expected = STAGE.review }))
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
      -- 退役路径（r9 唯一终局，仅 run.retire 武装可达）：写 ending + done。
      -- [ENDING 行由 RENDER[done] 在 node:render 上叙事，本 handler 只留归档叙事。
      ygg.kv.set("ending", STAGE.done)
      ygg.narrate(fill(TEXTS.beats.review_ok, { score = tostring(score), lesson = lesson }))
      advance(STAGE.done)
      return
    end
    -- 批收口路径（常驻默认）：滚动单槽已归档 → run.batch+1 → 复位 verify/settlement → 回 coordinate。
    -- 红线：绝不写 `ending`、叙事行首绝不含 [ENDING 前缀（宿主按行首前缀判终局，测试钉死）。
    local batch = num("run.batch", 1)
    ygg.kv.set("run.batch", batch + 1)
    ygg.kv.set("verify.passed", false)
    reset_settlement()
    ygg.narrate(fill(TEXTS.beats.review_batch_ok, {
      batch = tostring(batch),
      next = tostring(batch + 1),
      score = tostring(score),
      lesson = lesson,
    }))
    advance(STAGE.coordinate)
  end)
end


return M
