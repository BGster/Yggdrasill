-- src/handlers/intake/charter.lua —— 立局幕两事件：charter.set（可反复修订）/ run.begin（开工）。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 注册面全部在 install() 内（派发序 = 入口 install 调用序）。
-- R5：两事件都不进票据门（charter 是局主自宣，run.begin 是开工声明，非人类里程碑动作）。

local M = {}

local flow = require("src.runtime.flow")
local render_mod = require("src.view.render")
local reqs = require("src.domain.requirements")
local util = require("src.core.util")
local wire = require("src.domain.wire")

local TEXTS

local reject_payload =
  flow.reject_payload
local render =
  render_mod.receipt
local RUN_PHASE, req_count, run_phase, run_phase_label =
  reqs.RUN_PHASE, reqs.req_count, reqs.run_phase, reqs.run_phase_label
local fill, trim =
  util.fill, util.trim
local parse_charter =
  wire.parse_charter

-- 章程三件套（RAW KV 读取——不带默认值；齐备判定不能用 config.task_* 的兜底访问器）。
local function charter_piece(name)
  return trim(tostring(ygg.kv.get("task." .. name) or ""))
end

local function charter_complete()
  return charter_piece("title") ~= ""
    and charter_piece("goal") ~= ""
    and charter_piece("success") ~= ""
end

function M.install(deps)
  TEXTS = deps.texts
  -- R3/R4：charter.set 仅立局可用；立局内可反复修订（最后一次为准）。
  ygg.on("charter.set", function(payload)
    local current = run_phase()
    if current ~= RUN_PHASE.charter then
      return reject_payload("charter.set", fill(TEXTS.beats.charter_locked, {
        phase = run_phase_label(current),
      }))
    end
    local title, goal, success, problem = parse_charter(payload)
    if problem ~= nil then
      return reject_payload("charter.set", problem)
    end
    ygg.kv.set("task.title", title)
    ygg.kv.set("task.goal", goal)
    ygg.kv.set("task.success", success)
    ygg.narrate(fill(TEXTS.beats.charter_ok, {
      title = title,
      goal = goal,
      success = success,
    }))
    render()
  end)
  -- R3：run.begin 前置 = 章程三件套齐备；落点 run.phase=""（协调幕）并叙 [RUN BEGUN]。
  ygg.on("run.begin", function()
    local current = run_phase()
    if current ~= RUN_PHASE.charter then
      return reject_payload("run.begin", fill(TEXTS.beats.charter_begun, {
        phase = run_phase_label(current),
      }))
    end
    if not charter_complete() then
      return reject_payload("run.begin", TEXTS.beats.charter_missing)
    end
    ygg.kv.set("run.phase", "")
    ygg.narrate(fill(TEXTS.beats.run_begun, {
      title = charter_piece("title"),
      count = tostring(req_count()),
    }))
    render()
  end)
end

M.charter_complete = charter_complete

return M
