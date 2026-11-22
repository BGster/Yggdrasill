-- src/domain/wire.lua —— payload wire format 解析 + inbox 幂等键（`@ext=`）+ settlement.evolution 进化登记。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 资源由入口 install(deps) 注入。本模块唯一写的 KV 前缀：无（只读）。

local M = {}




local state = require("src.domain.wire.state")
local mod_intake = require("src.domain.wire.intake")
local mod_proposal = require("src.domain.wire.proposal")
local mod_evolution = require("src.domain.wire.evolution")
local mod_settlement = require("src.domain.wire.settlement")


function M.install(deps)
  state.TEXTS = deps.texts
end




for _, part in ipairs({ mod_intake, mod_proposal, mod_evolution, mod_settlement }) do
  for k, v in pairs(part) do
    M[k] = v
  end
end

return M
