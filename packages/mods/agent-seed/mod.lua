-- .ygg/mods/agent-seed/mod.lua —— 装配入口（不是 Mod 本体）
--
-- 本文件只做装配：读资源快照 → require 模块 → 纯模块 install → render.register →
-- handlers install → node:render → fsm.define。业务实现都在 src/ 下。
--
-- 派发序 = 下面 install() 的调用序（不是 require 序、也不是 pack.json 的 code.modules 声明序）。

local TEXTS = ygg.data("ygg:scene/texts")
local CONFIG = ygg.data("ygg:agent/config")
local PLAYBOOK = ygg.data("ygg:agent/playbook")

local util = require("src.util")
local config = require("src.config")
local reqs = require("src.requirements")
local lanes = require("src.lanes")
local props = require("src.proposals")
local gates = require("src.gates")
local guide = require("src.guide")
local menus = require("src.menus")
local render_mod = require("src.render")
local wire = require("src.wire")
local flow = require("src.flow")
local actions = require("src.handlers.actions")
local lane_actions = require("src.handlers.lanes")
local prop_actions = require("src.handlers.proposals")
local review = require("src.handlers.review")
local req_ops = require("src.handlers.requirement_ops")
local gate_actions = require("src.handlers.gate_actions")
local config_actions = require("src.handlers.config")
local views = require("src.handlers.views")
local choice = require("src.handlers.choice")
local lifecycle = require("src.handlers.lifecycle")

-- 注册序（30 点，拆分前后逐点对照；handler 内的注册顺序 = 其在 install() 里的语句序）：
--   1     agent.summary（render.register）
--   2-5   actions: task.intake / requirement.add / requirement.list / requirement.dispatch
--   6-7   lanes: lane.claim / lane.release
--   8-10  proposals: proposal.submit / accept / reject
--   11-14 review: coordinate.submit / verify.submit / settlement.evolution / review.note
--   15-19 requirement_ops: split / merge / amend / conflict / resolve
--   20-21 gate_actions: gate.approve / gate.reject
--   22-24 config: config.set / open / close
--   25-26 views: guide.show / map.show
--   27    choice: ygg.choice_selected
--   28-29 lifecycle: event.start / node:render
--   30    fsm.define（本文件末尾）

config.install({ texts = TEXTS, config = CONFIG })
reqs.install({ texts = TEXTS, lanes = lanes.LANES })
lanes.install({ texts = TEXTS })
props.install({ texts = TEXTS })
gates.install({ texts = TEXTS })
guide.install({ texts = TEXTS, playbook = PLAYBOOK })
menus.install({ texts = TEXTS })
render_mod.install({ texts = TEXTS, config = CONFIG })
wire.install({ texts = TEXTS })
flow.install({ texts = TEXTS })

render_mod.register()

actions.install({ texts = TEXTS })
lane_actions.install({ texts = TEXTS })
prop_actions.install({ texts = TEXTS })
review.install({ texts = TEXTS })
req_ops.install({ texts = TEXTS })
gate_actions.install({ texts = TEXTS })
config_actions.install({ texts = TEXTS })
views.install({ texts = TEXTS })
choice.install({ texts = TEXTS })
lifecycle.install({ texts = TEXTS })

local STAGE =
  reqs.STAGE

ygg.fsm.define({
  name = "stage",
  initial = STAGE.intake,
  event = "stage.advance",
  transitions = {
    {
      from = STAGE.intake,
      to = STAGE.coordinate,
      when = function(target)
        return flow.allowed(target, STAGE.coordinate)
      end,
    },
    {
      from = STAGE.coordinate,
      to = STAGE.review,
      when = function(target)
        return flow.allowed(target, STAGE.review)
      end,
    },
    {
      from = STAGE.review,
      to = STAGE.coordinate,
      when = function(target)
        return flow.allowed(target, STAGE.coordinate)
      end,
    },
    {
      from = STAGE.review,
      to = STAGE.done,
      when = function(target)
        return flow.allowed(target, STAGE.done)
      end,
    },
  },
})
