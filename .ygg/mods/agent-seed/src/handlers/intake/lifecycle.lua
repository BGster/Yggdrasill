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
local stage =
  reqs.stage
local fill, trim =
  util.fill, util.trim
local parse_intake =
  wire.parse_intake -- [deprecated-ref] name kept: r2 rev2 spec reuses this parser for the event.start charter



function M.install(deps)
  TEXTS = deps.texts
  ygg.on("event.start", function(payload)
    ygg.narrate(fill(TEXTS.scenes.prologue, {
      engine = tostring(ygg.engine_version),
      stage = stage(),
      mode = mode(),
    }))
    ygg.narrate(fill(stage_prompt("prologue"), prompt_vars()))
    ygg.narrate(TEXTS.beats.secret_redline)
    -- r2 rev2（B 方案①）：启动载荷即章程（title|description，复用 parse_intake 校验）。
    -- "" 与 "0" 都视为无载荷：照常 boot，章程由渲染回退显示 (untitled task)；
    -- 有载荷：同拍写 task.* 并落 coordinate（initial 即 coordinate，无段可过）；
    -- 已在 coordinate 再带载荷 = 幂等补写，不过段、不转场。
    local raw = trim(tostring(payload or ""))
    if raw ~= "" and raw ~= "0" then
      local title, description, problem = parse_intake(payload)
      if problem ~= nil then
        return reject_payload("event.start", problem)
      end
      ygg.kv.set("task.title", title)
      ygg.kv.set("task.description", description)
      ygg.narrate(fill(TEXTS.beats.charter_ok, { title = title, description = description }))
    end
    render()
  end)
  ygg.narrative.node("node:render", function()
    render()
  end)
end


return M
