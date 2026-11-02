-- src/handlers/review.lua —— 协调与复盘：coordinate.submit / verify.submit / settlement.evolution / review.note。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 注册面全部在 install() 内（派发序 = 入口 install 调用序）。

local M = {}

local config = require("src.config")
local flow = require("src.flow")
local gates = require("src.gates")
local choice = require("src.handlers.choice")
local render_mod = require("src.render")
local reqs = require("src.requirements")
local util = require("src.util")
local wire = require("src.wire")

local TEXTS

local goal_authorization, num, strict_gates, text_of =
  config.goal_authorization, config.num, config.strict_gates, config.text_of
local advance, refuse, reject, reject_payload =
  flow.advance, flow.refuse, flow.reject, flow.reject_payload
local gate_problem, settlement_problem =
  gates.gate_problem, gates.settlement_problem
local HUMAN =
  choice.HUMAN
local render, reset_settlement =
  render_mod.receipt, render_mod.reset_settlement
local STAGE, req_count, stage =
  reqs.STAGE, reqs.req_count, reqs.stage
local cut, fill, split, trim =
  util.cut, util.fill, util.split, util.trim
local probe_payload_ok =
  wire.probe_payload_ok



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
    reset_settlement()
    ygg.kv.set("verify.passed", false)
    ygg.narrate(fill(TEXTS.beats.coordinate_ok, { count = tostring(req_count()) }))
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
      ygg.narrate(fill(TEXTS.beats.verify_ok, { count = tostring(req_count()) }))
      render()
      return
    end
    ygg.kv.set("verify.passed", false)
    local failures = num("verify.failures", 0) + 1
    ygg.kv.set("verify.failures", failures)
    ygg.kv.set("verify.last", report ~= "" and report or TEXTS.beats.verify_failed_record)
    if strict_gates() then
      reset_settlement()
      ygg.narrate(fill(TEXTS.beats.verify_fail, { failures = tostring(failures) }))
      advance(STAGE.coordinate)
    else
      ygg.narrate(fill(TEXTS.beats.gate_bypass, { failures = tostring(failures) }))
      ygg.kv.set("verify.passed", true)
      render()
    end
  end)
  ygg.on("settlement.evolution", function(payload)
    local current = stage()
    if current ~= STAGE.review then
      return reject("settlement.evolution", fill(TEXTS.beats.stage_expected, { expected = STAGE.review }))
    end
    if ygg.kv.get("verify.passed") ~= true then
      return refuse(TEXTS.beats.req_wait_rejected, "settlement.evolution", TEXTS.beats.settlement_evolution_verify_first)
    end
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
    ygg.kv.set("settlement.evolution.artifact", record.artifact)
    ygg.kv.set("settlement.evolution.summary", record.summary)
    ygg.kv.set("settlement.evolution.candidates", candidates)
    ygg.kv.set("settlement.evolution.classes", wire.format_classes(record.classes))
    ygg.narrate(fill(TEXTS.beats.settlement_evolution_ok, {
      summary = record.summary,
      candidates = candidates,
    }))
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
    local settlement_issue = settlement_problem()
    if settlement_issue ~= nil then
      ygg.narrate(fill(TEXTS.beats.req_gate_block, { action = "review.note", detail = settlement_issue }))
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
    ygg.kv.set("review.lesson", lesson)
    ygg.kv.set("review.score", score)
    -- 进化记录只从本轮回合的 settlement 登记单源归档，不再读载荷或 settings。
    ygg.kv.set("review.evolution.artifact", text_of("settlement.evolution.artifact", ""))
    ygg.kv.set("review.evolution.summary", text_of("settlement.evolution.summary", ""))
    ygg.kv.set("review.evolution.candidates", text_of("settlement.evolution.candidates", TEXTS.beats.evolution_none))
    ygg.kv.set("review.evolution.classes", text_of("settlement.evolution.classes", ""))
    ygg.kv.set("ending", STAGE.done)
    ygg.narrate(fill(TEXTS.beats.review_ok, { score = tostring(score), lesson = lesson }))
    advance(STAGE.done)
  end)
end


return M
