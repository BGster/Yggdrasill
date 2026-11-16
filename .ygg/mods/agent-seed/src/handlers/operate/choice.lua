-- src/handlers/operate/choice.lua —— 选择路由 + 人类动作门禁（一次性票据与它的盖章/取证必须在同一模块）。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 注册面全部在 install() 内（派发序 = 入口 install 调用序）。

local M = {}

local config = require("src.core.config")
local flow = require("src.runtime.flow")
local menus = require("src.view.menus")
local render_mod = require("src.view.render")
local reqs = require("src.domain.requirements")
local util = require("src.core.util")

local TEXTS

local play_mode =
  config.play_mode
local refuse =
  flow.refuse
local CONFIG_UI, menu =
  menus.CONFIG_UI, menus.menu
local render =
  render_mod.receipt
local run_phase =
  reqs.run_phase
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

-- 本步的入口（r9 按人读签需要），三态：
--   "ticket"    = 人类从菜单点选（`ygg.choice_selected` 盖章后 emit，载荷无宿主身份段）
--   "delegated" = guide 模式下主 Agent 在用户对话授权后代执行（载荷带 `|actor=` 章）
--   "direct"    = goal 模式自主派发，或非 guide 的直派（载荷带 `|actor=` 章）
-- 判定侧据此决定「没身份」的结局：票据路降级并告明，带身份的两条路正常记账
-- （见 gates.read_problem）。只会在 HUMAN.needs 里改写：人机门禁动作每步都过它，
-- 所以值总是本步的。
HUMAN.entry = "direct"

function HUMAN.needs(label)
  if play_mode() ~= "guide" then
    HUMAN.entry = "direct"
    return true
  end
  if HUMAN.ticket == label then
    HUMAN.ticket = nil
    HUMAN.entry = "ticket"
    return true
  end
  -- guide 模式的语义是「主 Agent 必须先请示用户」，不是「用户必须自己点菜单」。
  -- 用户只有对话窗口一个通道，主 Agent 就是他唯一的执行面；拒绝直派等于把 guide
  -- 锁死成「去终端点菜单」，那不符合产品形态。放行，但记为 delegated：读签判定仍
  -- 要求载荷带 `|actor=` 章（可审计，见 gates.read_problem 的直派路），且与 goal 的
  -- 自主 direct 在审计上可区分。
  HUMAN.entry = "delegated"
  return true
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
    local current = run_phase()
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
