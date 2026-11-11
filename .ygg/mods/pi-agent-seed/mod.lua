-- .ygg/mods/pi-agent-seed/mod.lua —— 装配入口（不是 Mod 本体）
--
-- 本文件只做装配：读资源快照 → require 模块 → 纯模块 install → render.register →
-- handlers install → node:render → fsm.define。业务实现都在 src/ 下。
--
-- 派发序 = 下面 install() 的调用序（不是 require 序、也不是 pack.json 的 code.modules 声明序）。

-- 资源快照（片段装配；片段 ↔ entry 一一对应，见 pack.json entries）。
-- 契约：beats 键跨片段不相交；playbook 章节片段按文件名序（=order 序）排列，
--       相邻同 id（harness 7 片段）按序拼接 rules；装配不排序。

local TEXTS = ygg.data("ygg:scene/texts")          -- core：scenes/endings/prompts/options/tuning
TEXTS.beats = {}
for _, id in ipairs({ "flow", "requirements", "lanes", "map-guide", "settlement", "harness" }) do
  local fragment = ygg.data("ygg:scene/beats/" .. id)
  for key, value in pairs(fragment.beats) do
    TEXTS.beats[key] = value
  end
end

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
local executor = require("src.executor")
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
local tools = require("src.tools")

-- 注册序（35 点；handler 内的注册顺序 = 其在 install() 里的语句序）：
--   1     agent.summary（render.register）
--   2-5   actions: task.intake / requirement.add / requirement.list / requirement.dispatch
--   6-7   lanes: lane.claim / lane.release
--   8-10  proposals: proposal.submit / accept / reject
--   11-16 review: coordinate.submit / verify.submit / settlement.evolution / settlement.sessions / settlement.todo / review.note
--   17-21 requirement_ops: split / merge / amend / conflict / resolve
--   22-23 gate_actions: gate.approve / gate.reject
--   24-26 config: config.set / open / close
--   27-28 views: guide.show / map.show
--   29    choice: ygg.choice_selected
--   30-31 lifecycle: event.start / node:render
--   32-34 tools: tool.call / tool.result / vfs.write
--   35    fsm.define（本文件末尾）
-- 两条 effect（hooks.useEffect("tools.receipt") 与 hooks.useEffect("tools.reactive")）由
-- `tools.register()` 注册，与 `actions` 的 `auto_dispatch` 同性质：不是 handler，不占注册序号，
-- 也不参与「同名事件多 handler」的排序。两者同处 render() 入口的同一个 flush_effects。

config.install({ texts = TEXTS, config = CONFIG })
reqs.install({ texts = TEXTS, lanes = lanes.LANES, props = props })
lanes.install({ texts = TEXTS })
props.install({ texts = TEXTS })
gates.install({ texts = TEXTS })
guide.install({ texts = TEXTS, playbook = PLAYBOOK })
menus.install({ texts = TEXTS })
render_mod.install({ texts = TEXTS, config = CONFIG })
wire.install({ texts = TEXTS })
flow.install({ texts = TEXTS })
executor.install({ texts = TEXTS })
tools.install({ texts = TEXTS })

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
tools.register()

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
