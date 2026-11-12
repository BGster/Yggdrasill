-- src/guide.lua —— 幕后规则书索引与阶段提示（`data/playbook.json`）。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 资源由入口 install(deps) 注入。本模块唯一写的 KV 前缀：无（只读）。

local M = {}

local util = require("src.core.util")

local TEXTS
local PLAYBOOK

local fill =
  util.fill

local SECTIONS = {}

-- 相位→规则章节索引（微状态口径）：立局看仓库形态；协调看机器/协议/驱动/异步/派发/泳道/集成；
-- 盘点看演进/门禁/运行产物/证据/Mod 工程/宿主契约；发布看集成章（develop→integrate→merge→deploy）；退役看指引。
-- r5（D-MA 规则大迁移）：harness 盲区章废除，81 条按域迁入 run_machine/drive/actions/inbox/
-- run_artifacts/mod_engineering/host_contract 七新章；每个映射章存在且非空由语义用例钉死。
local STAGE_GUIDE = {
  charter = { "repository" },
  coordinate = { "run_machine", "actions", "drive", "inbox", "dispatch", "lanes", "integration" },
  settle = { "evolution", "gates", "run_artifacts", "evidence", "mod_engineering", "host_contract" },
  release = { "integration" },
  retired = { "pointers" },
}

local function guide_section(st)
  local candidates = STAGE_GUIDE[st]
  if candidates == nil or #candidates == 0 then
    return PLAYBOOK.sections[1]
  end
  local section = SECTIONS[candidates[1]]
  if section == nil then
    return PLAYBOOK.sections[1]
  end
  return section
end

local function narrate_stage_guide(st)
  local section = guide_section(st)
  ygg.narrate(fill(TEXTS.beats.guide_hint, {
    title = section.title,
    summary = section.summary,
    count = tostring(#section.rules),
  }))
end

function M.install(deps)
  TEXTS = deps.texts
  PLAYBOOK = deps.playbook
  for i = 1, #PLAYBOOK.sections do
    local entry = PLAYBOOK.sections[i]
    SECTIONS[entry.id] = entry
  end
end

M.guide_section = guide_section
M.narrate_stage_guide = narrate_stage_guide

return M
