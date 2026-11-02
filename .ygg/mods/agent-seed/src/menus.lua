-- src/menus.lua —— 阶段菜单 + 配置二级页（选项文本 ↔ 动作载荷的唯一来源；`ui.page` / `ui.page.stage`）。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 资源由入口 install(deps) 注入。本模块唯一写的 KV 前缀：`ui.*`（唯一写方）。

local M = {}

local config = require("src.config")
local gates = require("src.gates")
local guide = require("src.guide")
local props = require("src.proposals")
local reqs = require("src.requirements")
local util = require("src.util")
local wire = require("src.wire")

local TEXTS

local mode, next_mode, next_play_mode, play_mode, strict_gates, text_of =
  config.mode, config.next_mode, config.next_play_mode, config.play_mode, config.strict_gates, config.text_of
local gate_kind, gate_next, gate_total =
  gates.gate_kind, gates.gate_next, gates.gate_total
local guide_section =
  guide.guide_section
local prop_id, prop_next, prop_pending_total, prop_req, prop_stage =
  props.prop_id, props.prop_next, props.prop_pending_total, props.prop_req, props.prop_stage
local STAGE, job_capacity, job_total, req_conflict, req_conflict_peer, req_count, req_dev_ready, req_id, req_job, req_job_stage, req_open_total, stage =
  reqs.STAGE, reqs.job_capacity, reqs.job_total, reqs.req_conflict, reqs.req_conflict_peer, reqs.req_count, reqs.req_dev_ready, reqs.req_id, reqs.req_job, reqs.req_job_stage, reqs.req_open_total, reqs.stage
local fill, trim =
  util.fill, util.trim

-- settlement.evolution 菜单载荷读取预登记面；缺省零候选不需伪 artifact。
local function settlement_evolution_payload()
  local artifact, summary, candidates, classes = wire.evolution_settings()
  summary = trim(summary)
  if summary == "" then
    summary = TEXTS.beats.settlement_evolution_default_summary
  end
  candidates = trim(candidates)
  if candidates == "" then
    candidates = TEXTS.beats.evolution_none
  end
  return wire.format_settlement_evolution(artifact, summary, candidates, classes)
end

local function settlement_registered()
  return trim(text_of("settlement.evolution.summary", "")) ~= ""
end

local function review_note_payload()
  return wire.format_review_note(TEXTS.beats.default_lesson, tostring(TEXTS.tuning.default_score))
end

local function guide_item(st)
  local section = guide_section(st)
  return {
    text = fill(TEXTS.options.guide, { title = section.title, count = tostring(#section.rules) }),
    action = "guide.show",
    payload = section.id,
  }
end

local function map_item(st)
  return {
    text = fill(TEXTS.options.map, { stage = st }),
    action = "map.show",
    payload = "full",
  }
end

local CONFIG_UI = {}

function CONFIG_UI.page()
  local value = ygg.kv.get("ui.page")
  if type(value) ~= "string" then
    return ""
  end
  return value
end

function CONFIG_UI.page_stage()
  local value = ygg.kv.get("ui.page.stage")
  if type(value) ~= "string" then
    return ""
  end
  return value
end

function CONFIG_UI.set_page(page, st)
  local next_page = page or ""
  ygg.kv.set("ui.page", next_page)
  if next_page == "" then
    ygg.kv.set("ui.page.stage", "")
  else
    ygg.kv.set("ui.page.stage", st or stage())
  end
end

function CONFIG_UI.open(st)
  return CONFIG_UI.page() == "config" and CONFIG_UI.page_stage() == (st or stage())
end

function CONFIG_UI.item()
  return {
    text = TEXTS.options.config,
    action = "config.open",
    payload = "",
  }
end

function CONFIG_UI.close_item()
  return {
    text = TEXTS.options.config_close,
    action = "config.close",
    payload = "",
  }
end

function CONFIG_UI.play_item()
  local next_value = next_play_mode()
  return {
    text = fill(TEXTS.options.config_play, { mode = play_mode(), next = next_value }),
    action = "config.set",
    payload = "play.mode=" .. next_value,
  }
end

function CONFIG_UI.evolution_item()
  local next_value = next_mode()
  return {
    text = fill(TEXTS.options.config_evolution, { mode = mode(), next = next_value }),
    action = "config.set",
    payload = "evolution.mode=" .. next_value,
  }
end

function CONFIG_UI.gates_item()
  local current = strict_gates()
  return {
    text = fill(TEXTS.options.config_gates, { state = tostring(current), next = tostring(not current) }),
    action = "config.set",
    payload = "gates.require_all_green=" .. tostring(not current),
  }
end

function CONFIG_UI.page_items()
  return {
    CONFIG_UI.play_item(),
    CONFIG_UI.evolution_item(),
    CONFIG_UI.gates_item(),
    CONFIG_UI.close_item(),
  }
end

local MENU = {}

local function req_list_item()
  return {
    text = fill(TEXTS.options.req_list, {
      pending = tostring(prop_pending_total()),
      open = tostring(req_open_total()),
    }),
    action = "requirement.list",
    payload = "full",
  }
end

local function req_add_item()
  return {
    text = TEXTS.options.req_add,
    action = "requirement.add",
    payload = TEXTS.beats.default_req_payload,
  }
end

local function resolve_items()
  local first = nil
  local peer = ""
  for i = 1, req_count() do
    if req_conflict(i) ~= "" then
      first = i
      peer = req_conflict_peer(i)
      break
    end
  end
  if first == nil then
    return {}
  end
  local ids = req_id(first)
  if peer ~= "" then
    ids = ids .. "," .. peer
  end
  return {
    {
      text = fill(TEXTS.options.req_resolve_keep, { ids = ids }),
      action = "requirement.resolve",
      payload = ids .. "|keep_both",
    },
    {
      text = fill(TEXTS.options.req_resolve_first, { ids = ids }),
      action = "requirement.resolve",
      payload = ids .. "|keep_first",
    },
    {
      text = fill(TEXTS.options.req_resolve_revise, { ids = ids }),
      action = "requirement.resolve",
      payload = ids .. "|revise",
    },
  }
end

local function append_all(items, extra)
  for i = 1, #extra do
    items[#items + 1] = extra[i]
  end
  return items
end

local function dispatch_candidates()
  local out = {}
  for i = 1, req_count() do
    if req_job_stage(i) ~= nil and req_job(i) == "" then
      if req_job_stage(i) ~= "development" or req_dev_ready(i) then
        out[#out + 1] = i
      end
    end
  end
  return out
end

local function dispatch_item()
  local candidates = dispatch_candidates()
  if #candidates == 0 then
    return nil
  end
  return {
    text = fill(TEXTS.options.dispatch, {
      count = tostring(#candidates),
      jobs = tostring(job_total()),
      cap = tostring(job_capacity()),
      id = req_id(candidates[1]),
    }),
    action = "requirement.dispatch",
    payload = "",
  }
end

local function accept_item()
  local next_prop = prop_next()
  if next_prop == nil then
    return nil
  end
  return {
    text = fill(TEXTS.options.accept, {
      id = prop_id(next_prop),
      req = req_id(prop_req(next_prop)),
      stage = prop_stage(next_prop),
      pending = tostring(prop_pending_total()),
    }),
    action = "proposal.accept",
    payload = prop_id(next_prop),
  }
end

local function gate_items()
  local index = gate_next()
  if index == nil then
    return {}
  end
  local kind = gate_kind(index)
  return {
    {
      text = fill(TEXTS.options.gate_approve, {
        id = req_id(index),
        kind = kind,
        gates = tostring(gate_total()),
      }),
      action = "gate.approve",
      payload = req_id(index),
    },
    {
      text = fill(TEXTS.options.gate_reject, { id = req_id(index), kind = kind }),
      action = "gate.reject",
      payload = req_id(index) .. "|" .. TEXTS.beats.req_gate_reject_default,
    },
  }
end

MENU[STAGE.intake] = function()
  local items = append_all({}, resolve_items())
  items[#items + 1] = {
    text = TEXTS.options.intake,
    action = "task.intake",
    payload = TEXTS.beats.default_task_title .. "|" .. TEXTS.beats.default_task_desc,
  }
  items[#items + 1] = guide_item(STAGE.intake)
  items[#items + 1] = map_item(STAGE.intake)
  items[#items + 1] = req_list_item()
  items[#items + 1] = req_add_item()
  items[#items + 1] = CONFIG_UI.item()
  return items
end

MENU[STAGE.coordinate] = function()
  local items = append_all({}, resolve_items())
  -- 主前进动作优先级：验收提案 > 过门禁 > 派活 > 收口进 review（canonical 串行）。
  local accept = accept_item()
  local gates = gate_items()
  local dispatch = dispatch_item()
  if accept ~= nil then
    items[#items + 1] = accept
  elseif #gates > 0 then
    items = append_all(items, gates)
  elseif dispatch ~= nil then
    items[#items + 1] = dispatch
  else
    items[#items + 1] = {
      text = TEXTS.options.coordinate_submit,
      action = "coordinate.submit",
      payload = "",
    }
  end
  items[#items + 1] = guide_item(STAGE.coordinate)
  items[#items + 1] = map_item(STAGE.coordinate)
  items[#items + 1] = req_list_item()
  items[#items + 1] = req_add_item()
  items[#items + 1] = CONFIG_UI.item()
  return items
end

local function menu(st)
  -- 二级页生效时只返回子页项（渲染与选择都走这里，顶层项暂时让位）。
  if CONFIG_UI.open(st) then
    return CONFIG_UI.page_items()
  end
  local builder = MENU[st]
  if builder == nil then
    return nil
  end
  return builder()
end

local function option_texts(items)
  local out = {}
  for i = 1, #items do
    out[i] = items[i].text
  end
  return out
end

MENU[STAGE.review] = function()
  local items = append_all({}, resolve_items())
  if ygg.kv.get("verify.passed") == true then
    if settlement_registered() then
      items[#items + 1] = {
        text = TEXTS.options.review_note,
        action = "review.note",
        payload = review_note_payload(),
      }
    else
      items[#items + 1] = {
        text = TEXTS.options.settlement_evolution,
        action = "settlement.evolution",
        payload = settlement_evolution_payload(),
      }
    end
  else
    items[#items + 1] = { text = TEXTS.options.verify_green, action = "verify.submit", payload = "true" }
  end
  items[#items + 1] = {
    text = TEXTS.options.verify_fail,
    action = "verify.submit",
    payload = "all_green=false;report=" .. TEXTS.beats.default_gate_report,
  }
  items[#items + 1] = guide_item(STAGE.review)
  items[#items + 1] = map_item(STAGE.review)
  items[#items + 1] = req_list_item()
  items[#items + 1] = req_add_item()
  items[#items + 1] = CONFIG_UI.item()
  return items
end

function M.install(deps)
  TEXTS = deps.texts
end

M.CONFIG_UI = CONFIG_UI
M.MENU = MENU
M.accept_item = accept_item
M.append_all = append_all
M.dispatch_candidates = dispatch_candidates
M.dispatch_item = dispatch_item
M.gate_items = gate_items
M.guide_item = guide_item
M.map_item = map_item
M.menu = menu
M.option_texts = option_texts
M.req_add_item = req_add_item
M.req_list_item = req_list_item
M.resolve_items = resolve_items

return M
