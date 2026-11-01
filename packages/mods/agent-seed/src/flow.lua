-- src/flow.lua —— 转场与拒绝的唯二出口（`advance` / `refuse`）+ 状态机票据。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 资源由入口 install(deps) 注入。本模块唯一写的 KV 前缀：无（转场由 fsm 写 `fsm.stage`）。

local M = {}

local menus = require("src.menus")
local render_mod = require("src.render")
local reqs = require("src.requirements")
local util = require("src.util")

local TEXTS

local CONFIG_UI, MENU =
  menus.CONFIG_UI, menus.MENU
local render =
  render_mod.receipt
local stage =
  reqs.stage
local contains, fill, join =
  util.contains, util.fill, util.join

local function advance(target)
  -- 转场即离开二级页（子页跟着阶段走，不跨阶段）。
  CONFIG_UI.set_page("")
  if target ~= stage() then
    ygg.emit("stage.advance", target)
  end
  ygg.narrative["goto"]("node:render")
end

local function allowed_list(current)
  -- 只看阶段菜单本体：拒绝文案里不能列出 config 子页的项（子页是展示层，不是阶段能力）。
  local builder = MENU[current]
  local items = nil
  if builder ~= nil then
    items = builder()
  end
  if items == nil then
    return TEXTS.beats.allowed_none
  end
  local names = {}
  for i = 1, #items do
    if not contains(names, items[i].action) then
      names[#names + 1] = items[i].action
    end
  end
  return join(names, "、")
end

local function refuse(template, action, detail)
  local current = stage()
  ygg.narrate(fill(template, {
    action = tostring(action),
    stage = current,
    allowed = allowed_list(current),
    detail = detail or TEXTS.beats.rejected_default,
  }))
  render()
end

local function reject(action, detail)
  return refuse(TEXTS.beats.rejected, action, detail)
end

local function reject_payload(action, detail)
  return refuse(TEXTS.beats.rejected_payload, action, detail)
end

function M.install(deps)
  TEXTS = deps.texts
end

M.advance = advance
M.allowed_list = allowed_list
M.refuse = refuse
M.reject = reject
M.reject_payload = reject_payload

return M
