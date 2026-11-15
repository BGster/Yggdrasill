-- src/guide.lua —— 阶段提示壳（playbook 已于 2026-11-15 退役，规则正文在 docs/）。
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

-- playbook 退役后不再有章节索引；保留空表与函数壳，render/views 的调用点无需改动。
local STAGE_GUIDE = {}

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
  if section == nil then
    return
  end
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
