-- src/handlers/choice.lua —— 选择路由 + 人类动作门禁（一次性票据与它的盖章/取证必须在同一模块）。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 注册面全部在 install() 内（派发序 = 入口 install 调用序）。

local M = {}

local config = require("src.config")
local flow = require("src.flow")
local menus = require("src.menus")
local render_mod = require("src.render")
local reqs = require("src.requirements")
local util = require("src.util")

local TEXTS

local play_mode =
  config.play_mode
local refuse =
  flow.refuse
local CONFIG_UI, menu =
  menus.CONFIG_UI, menus.menu
local render =
  render_mod.receipt
local stage =
  reqs.stage
local fill, trim =
  util.fill, util.trim

local HUMAN = {}

HUMAN.ACTIONS = {
  ["proposal.accept"] = true,
  ["gate.approve"] = true,
  ["coordinate.submit"] = true,
  ["verify.submit"] = true,
  ["review.note"] = true,
}

HUMAN.ticket = nil

function HUMAN.needs(label)
  if play_mode() ~= "guide" then
    return true
  end
  if HUMAN.ticket == label then
    HUMAN.ticket = nil
    return true
  end
  return refuse(TEXTS.beats.guide_human_blocked, label)
end

function HUMAN.gated(action)
  if HUMAN.ACTIONS[action] ~= true then
    return true
  end
  return HUMAN.needs(action)
end

function HUMAN.label(action, payload)
  if HUMAN.ACTIONS[action] == true then
    return action
  end
  if action == "config.set" and type(payload) == "string" then
    local head = trim(payload)
    if head:sub(1, 9) == "play.mode" then
      return "config.set play.mode"
    end
  end
  return nil
end

function M.install(deps)
  TEXTS = deps.texts
  ygg.on("ygg.choice_selected", function(index)
    local current = stage()
    local items = menu(current)
    if items == nil then
      ygg.narrate(fill(TEXTS.beats.unknown_choice, { stage = current }))
      render()
      return
    end
    local picked = tonumber(index)
    if picked == nil or items[picked] == nil then
      ygg.narrate(fill(TEXTS.beats.bad_index, {
        stage = current,
        index = tostring(index),
        count = tostring(#items),
      }))
      render()
      return
    end
    -- 盖章：只能在这里盖章（菜单路由），单发直派没有盖章机会。
    local label = HUMAN.label(items[picked].action, items[picked].payload)
    if label ~= nil then
      HUMAN.ticket = label
    end
    -- 二级页只跟 config.* 走：选了任何阶段动作就离开子页（否则子页会占住阶段菜单）。
    if items[picked].action:sub(1, 7) ~= "config." then
      CONFIG_UI.set_page("")
    end
    ygg.emit(items[picked].action, items[picked].payload)
  end)
end

M.HUMAN = HUMAN

return M
