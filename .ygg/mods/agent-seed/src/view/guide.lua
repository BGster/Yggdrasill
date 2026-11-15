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

-- 相位→规则章节索引（微状态口径）：每相位播全部候选章，不再让首章遮蔽后续章节。
-- 未映射章同样是死资产：21 章必须至少在一条 phase-entry 路由上出现；map.show 另播全局索引。
local STAGE_GUIDE = {
  charter = { "repository", "credentials", "hard_constraints", "output" },
  coordinate = { "run_machine", "actions", "drive", "inbox", "lanes", "branches" },
  settle = { "evolution", "gates", "run_artifacts", "evidence", "live_evidence", "mod_engineering", "host_contract", "tools" },
  release = { "branches" },
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

local function narrate_guide_entry(section)
  ygg.narrate(fill(TEXTS.beats.guide_hint, {
    title = section.title,
    summary = section.summary,
    count = tostring(#section.rules),
  }))
end

local function narrate_stage_guide(st)
  local candidates = STAGE_GUIDE[st]
  if candidates == nil or #candidates == 0 then
    narrate_guide_entry(PLAYBOOK.sections[1])
    return
  end
  for i = 1, #candidates do
    local section = SECTIONS[candidates[i]]
    if section ~= nil then
      narrate_guide_entry(section)
    end
  end
end

local function narrate_guide_index()
  for i = 1, #PLAYBOOK.sections do
    narrate_guide_entry(PLAYBOOK.sections[i])
  end
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
M.narrate_guide_index = narrate_guide_index

return M
