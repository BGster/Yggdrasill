-- src/handlers/review.lua —— 协调与复盘：coordinate.submit / verify.submit / review.note。
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

local num, strict_gates =
  config.num, config.strict_gates
local advance, refuse, reject, reject_payload =
  flow.advance, flow.refuse, flow.reject, flow.reject_payload
local gate_problem =
  gates.gate_problem
local HUMAN =
  choice.HUMAN
local render =
  render_mod.receipt
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
      ygg.narrate(fill(TEXTS.beats.verify_fail, { failures = tostring(failures) }))
      advance(STAGE.coordinate)
    else
      ygg.narrate(fill(TEXTS.beats.gate_bypass, { failures = tostring(failures) }))
      ygg.kv.set("verify.passed", true)
      render()
    end
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
    -- 进化声明：段 > 已登记设置 > 缺省；形状与计数自洽均在写状态前校验。
    local registered_artifact, registered_candidates, registered_classes = wire.evolution_settings()
    local record, record_err = wire.normalize_evolution(
      note.segments.ev or registered_artifact,
      note.segments.c or registered_candidates,
      note.segments.cls or registered_classes
    )
    if record == nil then
      return reject_payload("review.note", record_err)
    end
    ygg.kv.set("review.lesson", lesson)
    ygg.kv.set("review.score", score)
    ygg.kv.set("review.evolution.artifact", record.artifact)
    ygg.kv.set("review.evolution.candidates", tostring(record.kept) .. "/" .. tostring(record.screened))
    ygg.kv.set("review.evolution.classes", wire.format_classes(record.classes))
    ygg.kv.set("ending", STAGE.done)
    ygg.narrate(fill(TEXTS.beats.review_ok, { score = tostring(score), lesson = lesson }))
    advance(STAGE.done)
  end)
end


return M
