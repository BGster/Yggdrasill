-- src/handlers/intake/lifecycle.lua —— 局的开场与渲染节点：event.start + node:render。 -- [deprecated-ref] module path pinned by pack.json code.modules
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 注册面全部在 install() 内（派发序 = 入口 install 调用序）。

local M = {}

local config = require("src.core.config")
local flow = require("src.runtime.flow")
local render_mod = require("src.view.render")
local reqs = require("src.domain.requirements")
local util = require("src.core.util")
local wire = require("src.domain.wire")

local TEXTS

local mode =
  config.mode
local reject_payload =
  flow.reject_payload
local prompt_vars, render, stage_prompt =
  render_mod.prompt_vars, render_mod.receipt, render_mod.stage_prompt
local run_phase, run_phase_label =
  reqs.run_phase, reqs.run_phase_label
local fill, trim =
  util.fill, util.trim
local parse_intake =
  wire.parse_intake

function M.install(deps)
  TEXTS = deps.texts
  ygg.on("event.start", function(payload)
    -- R2：run.phase 初值 charter（只在真缺席时写入——重放不把已开工的局拉回立局）。
    -- 判缺席用 nil：合法的 ""（= 协调，run.begin 写入）不能被重放拉回 charter（T4 缺陷 2）。
    if ygg.kv.get("run.phase") == nil then
      ygg.kv.set("run.phase", "charter")
    end
    -- boot 三则：引擎 / 模式 / 红线（R3：只叙述，不转场）。
    ygg.narrate(fill(TEXTS.scenes.prologue, {
      engine = tostring(ygg.engine_version),
      stage = run_phase_label(run_phase()),
      mode = mode(),
    }))
    ygg.narrate(fill(stage_prompt("prologue"), prompt_vars()))
    ygg.narrate(TEXTS.beats.secret_redline)
    -- R3：启动载荷即章程（title|goal|success，宽松解析——空段回退默认文案，不拒绝；
    -- 章程的严格校验在 charter.set）。"" 与 "0" 都视为无载荷：照常 boot，
    -- 章程由后续 charter.set 或渲染回退显示。
    local raw = trim(tostring(payload or ""))
    if raw ~= "" and raw ~= "0" then
      local title, goal, success, problem = parse_intake(payload)
      if problem ~= nil then
        return reject_payload("event.start", problem)
      end
      ygg.kv.set("task.title", title)
      ygg.kv.set("task.goal", goal)
      ygg.kv.set("task.success", success)
      ygg.narrate(fill(TEXTS.beats.charter_ok, {
        title = title,
        goal = goal,
        success = success,
      }))
    end
    render()
  end)
  ygg.narrative.node("node:render", function()
    render()
  end)
end


return M
