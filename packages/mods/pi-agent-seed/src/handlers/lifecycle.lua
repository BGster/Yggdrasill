-- src/handlers/lifecycle.lua —— 局的开场与渲染节点：event.start + node:render。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 注册面全部在 install() 内（派发序 = 入口 install 调用序）。

local M = {}

local config = require("src.config")
local render_mod = require("src.render")
local reqs = require("src.requirements")
local util = require("src.util")

local TEXTS

local mode =
  config.mode
local prompt_vars, render, stage_prompt =
  render_mod.prompt_vars, render_mod.receipt, render_mod.stage_prompt
local stage =
  reqs.stage
local fill =
  util.fill



function M.install(deps)
  TEXTS = deps.texts
  ygg.on("event.start", function()
    ygg.narrate(fill(TEXTS.scenes.prologue, {
      engine = tostring(ygg.engine_version),
      stage = stage(),
      mode = mode(),
    }))
    ygg.narrate(fill(stage_prompt("prologue"), prompt_vars()))
    ygg.narrate(TEXTS.beats.secret_redline)
    render()
  end)
  ygg.narrative.node("node:render", function()
    render()
  end)
end


return M
