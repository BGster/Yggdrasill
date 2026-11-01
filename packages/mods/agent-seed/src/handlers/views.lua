-- src/handlers/views.lua —— 只读视图：guide.show / map.show。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 注册面全部在 install() 内（派发序 = 入口 install 调用序）。

local M = {}

local guide = require("src.guide")
local render_mod = require("src.render")
local reqs = require("src.requirements")
local util = require("src.util")
local wire = require("src.wire")

local TEXTS

local guide_section, narrate_guide =
  guide.guide_section, guide.narrate_guide
local narrate_map, render =
  render_mod.narrate_map, render_mod.receipt
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
      narrate_map(stage())
    else
      ygg.narrate(fill(TEXTS.beats.map_rejected, { got = tostring(payload), usage = TEXTS.beats.map_usage }))
    end
    render()
  end)
end


return M
