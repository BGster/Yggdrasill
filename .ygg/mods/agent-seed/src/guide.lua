-- src/guide.lua —— 现场规则章节索引与渲染（`data/playbook.json`；`guide.show <section_id>`）。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 资源由入口 install(deps) 注入。本模块唯一写的 KV 前缀：无（只读）。

local M = {}

local util = require("src.util")

local TEXTS
local PLAYBOOK

local fill, join =
  util.fill, util.join

local SECTIONS = {}

local SECTION_IDS = {}

local STAGE_GUIDE = {
  intake = { "output" },
  coordinate = { "dispatch", "lanes", "integration" },
  review = { "gates", "evidence" },
  done = { "pointers" },
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

local MODALITY_MARK = {
  ["must"] = "［必须］",
  ["must-not"] = "［禁止］",
  ["should"] = "［应当］",
}

local function narrate_section(section)
  ygg.narrate(fill(TEXTS.beats.guide_head, {
    title = section.title,
    summary = section.summary,
    count = tostring(#section.rules),
  }))
  for i = 1, #section.rules do
    local rule = section.rules[i]
    ygg.narrate(fill(TEXTS.beats.guide_rule, {
      mark = MODALITY_MARK[rule.modality] or "",
      text = rule.text,
    }))
  end
  ygg.narrate(fill(TEXTS.beats.guide_tail, { id = section.id, total = tostring(#SECTION_IDS) }))
end

local function narrate_guide(id)
  local section = SECTIONS[id]
  if section == nil then
    ygg.narrate(fill(TEXTS.beats.guide_unknown, {
      id = tostring(id),
      ids = join(SECTION_IDS, "、"),
    }))
    return
  end
  narrate_section(section)
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
    SECTION_IDS[#SECTION_IDS + 1] = entry.id
  end
end

M.MODALITY_MARK = MODALITY_MARK
M.SECTIONS = SECTIONS
M.SECTION_IDS = SECTION_IDS
M.STAGE_GUIDE = STAGE_GUIDE
M.guide_section = guide_section
M.narrate_guide = narrate_guide
M.narrate_section = narrate_section
M.narrate_stage_guide = narrate_stage_guide

return M
