-- src/menus.lua —— 阶段菜单 + 配置二级页（选项文本 ↔ 动作载荷的唯一来源；`ui.page` / `ui.page.stage`）。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 资源由入口 install(deps) 注入。本模块唯一写的 KV 前缀：`ui.*`（唯一写方）。

local M = {}

local config = require("src.core.config")
local gates = require("src.domain.gates")
local hooks = require("src.core.hooks")
local props = require("src.domain.proposals")
local reqs = require("src.domain.requirements")
local util = require("src.core.util")
local wire = require("src.domain.wire")

local TEXTS

local mode, next_mode, next_play_mode, play_mode, run_retire_armed, strict_gates, text_of =
  config.mode, config.next_mode, config.next_play_mode, config.play_mode, config.run_retire_armed, config.strict_gates, config.text_of
local gate_kind, gate_next, gate_total =
  gates.gate_kind, gates.gate_next, gates.gate_total
local prop_id, prop_next, prop_pending_total, prop_req, prop_stage =
  props.prop_id, props.prop_next, props.prop_pending_total, props.prop_req, props.prop_stage
local RUN_PHASE, job_capacity, job_total, req_conflict, req_conflict_peer, req_count, req_dev_ready, req_has_pending, req_id, req_job, req_job_stage, req_open_total, run_phase =
  reqs.RUN_PHASE, reqs.job_capacity, reqs.job_total, reqs.req_conflict, reqs.req_conflict_peer, reqs.req_count, reqs.req_dev_ready, reqs.req_has_pending, reqs.req_id, reqs.req_job, reqs.req_job_stage, reqs.req_open_total, reqs.run_phase
local fill =
  util.fill
local trim =
  util.trim

-- settlement.evolution 载荷读预登记面（零候选由 format_settlement_evolution 补中性默认值）。
-- 局末两项清理也折进同一事务（可选段 sessions= / todo=）：未登记时补默认段，
-- 让 review 阶段一步做完三项登记（省回合），独立动作 settlement.sessions|todo 仍可用。
local function settlement_evolution_payload()
  local artifact, summary, candidates, classes = wire.evolution_settings()
  local payload = wire.format_settlement_evolution(artifact, summary, candidates, classes)
  if text_of("settlement.sessions.destination", "") == "" then
    payload = payload .. "|" .. TEXTS.beats.settlement_sessions_segment
  end
  if config.num("settlement.todo.remaining", -1) ~= 0 then
    payload = payload .. "|" .. TEXTS.beats.settlement_todo_segment
  end
  return payload
end

-- 「本轮进化登记过了吗」：只看本轮回合 settlement 四键里的 summary，不看预登记面。
local function settlement_registered()
  return trim(text_of("settlement.evolution.summary", "")) ~= ""
end

-- review.note 载荷固定 `lesson|score` 两段；进化声明不从这里走（旧 ev/c/cls 段会被拒）。
-- r9：未武装 = 批收口（回 coordinate）；run.retire 武装 = 终局退役（[ENDING · run retired]）。
local function review_note_payload()
  return wire.format_review_note(TEXTS.beats.default_lesson, tostring(TEXTS.tuning.default_score))
end

local function map_item(st)
  return {
    text = fill(TEXTS.options.map, { stage = st }),
    action = "map.show",
    -- rev1 r3：map.show 忽略一切 payload；菜单数据不说谎，故不再携带已死的 "full"。
    payload = "",
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
    ygg.kv.set("ui.page.stage", st or run_phase())
  end
end

function CONFIG_UI.open(st)
  return CONFIG_UI.page() == "config" and CONFIG_UI.page_stage() == (st or run_phase())
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

-- r9 退役切换项（纯展示层入口，写仍走 config.set，值域 true / ""）：
-- 武装后 review 菜单首项的 review.note 变终局变体（summary.nextEndsRun 免费翻转）。
function CONFIG_UI.retire_item()
  local armed = run_retire_armed()
  return {
    text = fill(TEXTS.options.config_retire, { state = armed and "armed" or "off", next = armed and '""' or "true" }),
    action = "config.set",
    payload = "run.retire=" .. (armed and '""' or "true"),
  }
end

function CONFIG_UI.page_items()
  return {
    CONFIG_UI.play_item(),
    CONFIG_UI.evolution_item(),
    CONFIG_UI.gates_item(),
    CONFIG_UI.retire_item(),
    CONFIG_UI.close_item(),
  }
end

local MENU = {}

-- 章程三件套齐备（RAW KV 读取，与 charter.lua 同口径；不带默认值）。
local function charter_complete()
  for _, name in ipairs({ "title", "goal", "success" }) do
    if trim(tostring(ygg.kv.get("task." .. name) or "")) == "" then
      return false
    end
  end
  return true
end

-- 发布已声明（R10）：master_sha ∨ skipped_reason 任一非空。
local function release_declared()
  return text_of("run.release.master_sha", "") ~= ""
    or text_of("run.release.skipped_reason", "") ~= ""
end

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
  -- 响应式记忆化（useMemo）：依赖签名就是本函数用到的四个谓词——需求数、逐需求的
  -- job 阶段 / 当前 job / 待验收提案 / 开发就绪；完全由 KV 派生，读档后签名变化即重算。
  local cached = hooks.useMemo("menus.dispatch_candidates", function()
    local out = {}
    for i = 1, req_count() do
      -- 微状态口径：可派微状态只有立项（→ plan）与待开发（→ development）；交付受理 /
      -- 门禁 / 集成 / 已闭环的结构性排除由 req_job_stage（nil）与 pending / gate 判据承担。
      if req_job_stage(i) ~= nil and req_job(i) == "" and not req_has_pending(i) then
        if req_job_stage(i) ~= "development" or req_dev_ready(i) then
          out[#out + 1] = i
        end
      end
    end
    return out
  end, function()
    local deps_sig = { req_count() }
    for i = 1, req_count() do
      deps_sig[#deps_sig + 1] = tostring(req_job_stage(i))
      deps_sig[#deps_sig + 1] = req_job(i)
      deps_sig[#deps_sig + 1] = req_has_pending(i) and "1" or "0"
      deps_sig[#deps_sig + 1] = req_dev_ready(i) and "1" or "0"
    end
    return deps_sig
  end)
  -- 返回拷贝：调用方可以改写自己的候选表（如 `rN` 载荷收窄），不得污染记忆化结果。
  local out = {}
  for i = 1, #cached do
    out[i] = cached[i]
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

MENU[RUN_PHASE.charter] = function()
  local items = {
    {
      text = TEXTS.options.charter_set,
      action = "charter.set",
      payload = TEXTS.beats.charter_payload,
    },
  }
  -- run.begin 只在章程三件套齐备时上桌（菜单数据不说谎：未齐备时派发只会可读拒绝）。
  if charter_complete() then
    items[#items + 1] = {
      text = TEXTS.options.run_begin,
      action = "run.begin",
      payload = "",
    }
  end
  items[#items + 1] = map_item(RUN_PHASE.charter)
  items[#items + 1] = CONFIG_UI.item()
  return items
end

MENU[RUN_PHASE.coordinate] = function()
  local items = append_all({}, resolve_items())
  -- 主前进动作并存（canonical 序不变）：验收提案第 1 → 门禁两项紧随 → 派活 / 收口兑底。
  -- 门禁不再被「任一需求有待验收提案」整体藏起（按需求局部化：gate.approve 只看 rN 自身，
  -- gate_problem_for 在 handler 里判；跨需求阻塞改由串行验收 / 收口前置承担）。单需求下
  -- accept 与 gate 结构上不同时出现（门禁只能由受理唯一 pending 提案升起），全 1 菜单
  -- 首项序号不变。（plan 门禁已随 D-MA23 改为提交即门禁——不再经由 proposal.accept。）
  local accept = accept_item()
  local gates = gate_items()
  local dispatch = dispatch_item()
  if accept ~= nil then
    items[#items + 1] = accept
  end
  items = append_all(items, gates)
  if accept == nil and #gates == 0 then
    if dispatch ~= nil then
      items[#items + 1] = dispatch
    else
      items[#items + 1] = {
        text = TEXTS.options.coordinate_submit,
        action = "coordinate.submit",
        payload = "",
      }
    end
  end
  items[#items + 1] = map_item(RUN_PHASE.coordinate)
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

MENU[RUN_PHASE.settle] = function()
  local items = append_all({}, resolve_items())
  -- 盘点内子微状态序（§2.3）：待验证 → 待登记演进 → 待结会话 → 待清待办；
  -- 四项齐备由 settle.close 效应自动转发布幕（不占菜单项，不新增事件）。
  if ygg.kv.get("verify.passed") == true then
    if not settlement_registered() then
      items[#items + 1] = {
        text = TEXTS.options.settlement_evolution,
        action = "settlement.evolution",
        payload = settlement_evolution_payload(),
      }
    end
    if text_of("settlement.sessions.destination", "") == "" then
      items[#items + 1] = {
        text = TEXTS.options.settlement_sessions,
        action = "settlement.sessions",
        payload = TEXTS.beats.settlement_sessions_payload,
      }
    end
    if config.num("settlement.todo.remaining", -1) ~= 0 then
      items[#items + 1] = {
        text = TEXTS.options.settlement_todo,
        action = "settlement.todo",
        payload = TEXTS.beats.settlement_todo_payload,
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
  items[#items + 1] = map_item(RUN_PHASE.settle)
  items[#items + 1] = req_list_item()
  items[#items + 1] = req_add_item()
  items[#items + 1] = CONFIG_UI.item()
  return items
end

MENU[RUN_PHASE.release] = function()
  local items = append_all({}, resolve_items())
  if not release_declared() then
    -- 发布未声明：先声明本批结果（confirm / skip 二选一）。
    items[#items + 1] = {
      text = TEXTS.options.release_confirm,
      action = "release.confirm",
      payload = TEXTS.beats.release_confirm_payload,
    }
    items[#items + 1] = {
      text = TEXTS.options.release_skip,
      action = "release.skip",
      payload = TEXTS.beats.release_skip_payload,
    }
  else
    -- 已声明：首项 = review.note（批结算；run.retire 武装时它是终局变体，
    -- summary.nextEndsRun 据首项判定，宿主预收口协议只认它）。
    items[#items + 1] = {
      text = TEXTS.options.review_note,
      action = "review.note",
      payload = review_note_payload(),
    }
  end
  items[#items + 1] = map_item(RUN_PHASE.release)
  items[#items + 1] = CONFIG_UI.item()
  return items
end

MENU[RUN_PHASE.retired] = function()
  return {
    map_item(RUN_PHASE.retired),
    req_list_item(),
  }
end

function M.install(deps)
  TEXTS = deps.texts
end

M.CONFIG_UI = CONFIG_UI
M.MENU = MENU
M.accept_item = accept_item
M.append_all = append_all
M.charter_complete = charter_complete
M.dispatch_candidates = dispatch_candidates
M.dispatch_item = dispatch_item
M.gate_items = gate_items
M.map_item = map_item
M.menu = menu
M.option_texts = option_texts
M.release_declared = release_declared
M.req_add_item = req_add_item
M.req_list_item = req_list_item
M.resolve_items = resolve_items

return M
