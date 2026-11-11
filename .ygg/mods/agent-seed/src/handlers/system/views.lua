-- src/handlers/views.lua —— 只读视图：map.show（完整视图 = 地图 + 当前剧情提示）。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 注册面全部在 install() 内（派发序 = 入口 install 调用序）。

local M = {}

local menus = require("src.view.menus")
local render_mod = require("src.view.render")
local reqs = require("src.domain.requirements")
local util = require("src.core.util")


local narrate_map, render, stage_prompt, prompt_vars =
  render_mod.narrate_map, render_mod.receipt, render_mod.stage_prompt, render_mod.prompt_vars
local menu =
  menus.menu
local stage =
  reqs.stage
local fill =
  util.fill



function M.install(_)
  ygg.on("map.show", function()
    -- rev1 r3：单一行为，任何 payload 一律忽略（无 ""/0/full/section_id 分支）。
    local current = stage()
    narrate_map(current, false, menu(current),
      fill(stage_prompt(current), prompt_vars()))
    render()
  end)
end


return M
