-- src/view/render.lua —— 回执渲染 + 双层图（local/full）+ 结构化摘要（`agent.summary` 的注册面）。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 资源由入口 install(deps) 注入。本模块写的 KV 前缀：`settlement.*`（仅 reset_settlement；其余只读）。

local M = {}



local state = require("src.view.render.state")
local mod_reqlist = require("src.view.render.reqlist")
local mod_mapview = require("src.view.render.mapview")
local mod_screen = require("src.view.render.screen")
local render, summary_fields = mod_screen.render, mod_screen.summary_fields


-- 三项结算登记的统一重置：coordinate.submit 重入与 verify.submit=false 都调这一处。
-- 只清本局登记，不动 settings.review.evolution.* 预登记值。
function M.reset_settlement()
  ygg.kv.set("settlement.evolution.artifact", "")
  ygg.kv.set("settlement.evolution.summary", "")
  ygg.kv.set("settlement.evolution.candidates", "")
  ygg.kv.set("settlement.evolution.classes", "")
  ygg.kv.set("settlement.sessions.remaining", -1)
  ygg.kv.set("settlement.sessions.moved", 0)
  ygg.kv.set("settlement.sessions.destination", "")
  ygg.kv.set("settlement.todo.remaining", -1)
  ygg.kv.set("settlement.todo.verified_by", "")
end

function M.install(deps)
  state.TEXTS = deps.texts
  state.CONFIG = deps.config
end

function M.register()
  ygg.on("agent.summary", function()
    ygg.meta("summary", summary_fields())
    render()
  end)
end



for _, part in ipairs({ mod_reqlist, mod_mapview, mod_screen }) do
  for k, v in pairs(part) do
    M[k] = v
  end
end

return M
