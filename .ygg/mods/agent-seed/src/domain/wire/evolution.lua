-- src/domain/wire/evolution.lua —— S32 纯搬移自 wire.lua（零逻辑变更；接线见交付回执）。

local M = {}
local config = require("src.core.config")
local util = require("src.core.util")
local state = require("src.domain.wire.state")
local TEXTS = setmetatable({}, { __index = function(_, k) return state.TEXTS[k] end })
local mod_proposal = require("src.domain.wire.proposal")
local effective = config.effective
local contains = util.contains
local cut = util.cut
local join = util.join
local split = util.split
local trim = util.trim
local EVOLUTION_CLASSES, EVOLUTION_KEPT = mod_proposal.EVOLUTION_CLASSES, mod_proposal.EVOLUTION_KEPT


local function evolution_path_ok(path)
  if type(path) ~= "string" then
    return false
  end
  local raw = trim(path)
  if raw == "" or #raw > 128 then
    return false
  end
  if raw:sub(1, 10) ~= "artifacts/" or raw:sub(-3) ~= ".md" then
    return false
  end
  if #raw <= 13 or raw:find("..", 1, true) ~= nil or raw:find("%s") ~= nil then
    return false
  end
  return true
end

-- artifact 路径形状 = 同一个判据（`artifacts/<name>.md`，≤128 B，无 `..`，无空白）。
-- `vfs.write`、`fs.artifact_probe` 与 `proposal.submit` 的同步核验共用这一处事实源，不复制正则。

local function whole_number(value)
  local parsed = tonumber(value)
  if parsed == nil or parsed ~= math.floor(parsed) then
    return nil
  end
  return math.floor(parsed)
end

local function parse_candidates(text)
  local kept_text, screened_text = cut(trim(text), "/")
  local kept = whole_number(trim(kept_text or ""))
  local screened = whole_number(trim(screened_text or ""))
  if kept == nil or screened == nil then
    return nil
  end
  if kept < 0 or screened < 0 or kept > screened then
    return nil
  end
  return kept, screened
end

local function parse_classes(text)
  local out = {}
  local total = 0
  for _, part in ipairs(split(trim(text), ",")) do
    local piece = trim(part)
    if piece ~= "" then
      local kind, count_text = cut(piece, ":")
      kind = trim(kind or "")
      local count = whole_number(trim(count_text or ""))
      if not contains(EVOLUTION_CLASSES, kind) or count == nil or count < 0 or out[kind] ~= nil then
        return nil
      end
      out[kind] = count
      total = total + count
    end
  end
  return out, total
end

local function format_classes(classes)
  local out = {}
  for i = 1, #EVOLUTION_CLASSES do
    local kind = EVOLUTION_CLASSES[i]
    local count = classes[kind]
    if type(count) == "number" then
      out[#out + 1] = kind .. ":" .. tostring(count)
    end
  end
  return join(out, ",")
end

local function kept_total(classes)
  local total = 0
  for i = 1, #EVOLUTION_CLASSES do
    local kind = EVOLUTION_CLASSES[i]
    if EVOLUTION_KEPT[kind] == true then
      total = total + (classes[kind] or 0)
    end
  end
  return total
end

-- 四路输入一律是字符串（缺省 = ""）：artifact 可空；summary 去空白后必填；
-- candidates 空 = 0/0；classes 空 = 未分类。config.set 只逐键校形状，
-- 跨字段完整性集中在这里校（预登记阶段允许分次登记）。
-- 完整性是结算登记硬条件：screened > 0 就必须同时有复核 artifact 与逐类计数——
-- 「筛了几条、留了几分」而把复核记录与根因分类留在散文里，等于没登记。
local function normalize_evolution(artifact, summary, candidates, classes)
  local record = { artifact = "", summary = "", kept = 0, screened = 0, classes = {} }
  local summary_text = trim(summary or "")
  if summary_text == "" then
    return nil, TEXTS.beats.settlement_evolution_missing
  end
  record.summary = summary_text
  local artifact_text = trim(artifact or "")
  if artifact_text ~= "" then
    if not evolution_path_ok(artifact_text) then
      return nil, TEXTS.beats.review_evolution_bad_path
    end
    record.artifact = artifact_text
  end
  local candidates_text = trim(candidates or "")
  if candidates_text ~= "" then
    local kept, screened = parse_candidates(candidates_text)
    if kept == nil then
      return nil, TEXTS.beats.review_evolution_bad_candidates
    end
    record.kept, record.screened = kept, screened
  end
  local classes_text = trim(classes or "")
  local parsed, total = nil, 0
  if classes_text ~= "" then
    parsed, total = parse_classes(classes_text)
    if parsed == nil or total == 0 then
      return nil, TEXTS.beats.review_evolution_bad_classes
    end
  end
  if record.screened > 0 then
    if record.artifact == "" then
      return nil, TEXTS.beats.review_evolution_need_artifact
    end
    if parsed == nil then
      return nil, TEXTS.beats.review_evolution_need_classes
    end
  end
  if parsed ~= nil then
    if total ~= record.screened or kept_total(parsed) ~= record.kept then
      return nil, TEXTS.beats.review_evolution_mismatch
    end
    record.classes = parsed
  end
  return record, nil
end

-- 预登记面（`config.set review.evolution.*` → `settings.review.evolution.*`）与菜单载荷读同一份口径。
local function evolution_settings()
  local function read(name)
    local value = effective(name)
    if type(value) == "string" then
      return value
    end
    return ""
  end
  return read("review.evolution.artifact"), read("review.evolution.summary"),
    read("review.evolution.candidates"), read("review.evolution.classes")
end

-- 菜单构造结算载荷：零候选也有中性 summary 与显式 c=0/0，不需要伪 artifact。
local function format_settlement_evolution(artifact, summary, candidates, classes)
  local summary_text = trim(summary or "")
  if summary_text == "" then
    summary_text = TEXTS.beats.settlement_evolution_default_summary
  end
  local candidates_text = trim(candidates or "")
  local parts = {
    trim(artifact or ""),
    summary_text,
    "c=" .. (candidates_text ~= "" and candidates_text or "0/0"),
  }
  local classes_text = trim(classes or "")
  if classes_text ~= "" then
    parts[#parts + 1] = "cls=" .. classes_text
  end
  return join(parts, "|")
end

-- ============================================================================
-- §局末两项清理（subagent 会话 / 主会话 Todo）的形状单一事实源
--
-- 两个入口共用同一套校验：独立动作 `settlement.sessions|todo` 与
-- `settlement.evolution` 的可选段（`sessions=` / `todo=`，`/` 分隔，外层是 `|`）。
-- 自声明：Mod 读不到会话与 Todo API，这里只校「登记断言」的形状。
-- ============================================================================
local TODO_VERIFIED_MAX = 120

M.artifact_path_ok = evolution_path_ok
M.evolution_path_ok = evolution_path_ok
M.evolution_settings = evolution_settings
M.format_classes = format_classes
M.format_settlement_evolution = format_settlement_evolution
M.kept_total = kept_total
M.normalize_evolution = normalize_evolution
M.parse_candidates = parse_candidates
M.parse_classes = parse_classes
M.TODO_VERIFIED_MAX = TODO_VERIFIED_MAX

return M
