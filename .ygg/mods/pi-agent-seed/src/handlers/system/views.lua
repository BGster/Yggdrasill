-- src/handlers/system/views.lua —— 只读视图：guide.show / map.show。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 注册面全部在 install() 内（派发序 = 入口 install 调用序）。

local M = {}

local guide = require("src.view.guide")
local menus = require("src.view.menus")
local render_mod = require("src.view.render")
local reqs = require("src.domain.requirements")
local util = require("src.core.util")
local wire = require("src.domain.wire")

local TEXTS

local guide_section, narrate_guide =
  guide.guide_section, guide.narrate_guide
local narrate_map, render, stage_prompt, prompt_vars =
  render_mod.narrate_map, render_mod.receipt, render_mod.stage_prompt, render_mod.prompt_vars
local menu =
  menus.menu
local stage =
  reqs.stage
local fill, trim =
  util.fill, util.trim
local probe_payload_ok =
  wire.probe_payload_ok



function M.install(deps)
  TEXTS = deps.texts
  ygg.on("guide.show", function(payload)
    local current = stage()
    local id = trim(payload)
    if id == "" then
      id = guide_section(current).id
    end
    narrate_guide(id)
    render()
  end)
  ygg.on("map.show", function(payload)
    if probe_payload_ok(payload) then
      -- 默认出「当前节点 + 出边」；`map.show full` 才出设计全图。
      local current = stage()
      narrate_map(current, trim(payload) == "full", menu(current),
        fill(stage_prompt(current), prompt_vars()))
    else
      ygg.narrate(fill(TEXTS.beats.map_rejected, { got = tostring(payload), usage = TEXTS.beats.map_usage }))
    end
    render()
  end)
end


return M
