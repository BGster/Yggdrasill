-- .ygg/mods/agent-seed/mod.lua —— 装配入口（不是 Mod 本体）
--
-- 本文件只做装配：读资源快照 → require 模块 → 纯模块 install → render.register →
-- handlers install → node:render → fsm.define。业务实现都在 src/ 下。
--
-- 派发序 = 下面 install() 的调用序（不是 require 序、也不是 pack.json 的 code.modules 声明序）。

-- 资源快照（片段装配；片段 ↔ entry 一一对应，见 pack.json entries）。
-- 契约：beats 键跨片段不相交；playbook 章节片段按文件名序（=order 序）排列，
--       各 section id 全局唯一（r5 起 harness 七片段同 id 拼接模式退役），装配不排序。

local TEXTS = ygg.data("ygg:scene/texts")          -- core：scenes/endings/prompts/options/tuning
TEXTS.beats = {}
for _, id in ipairs({ "flow", "requirements", "lanes", "map-guide", "settlement", "harness" }) do
  local fragment = ygg.data("ygg:scene/beats/" .. id)
  for key, value in pairs(fragment.beats) do
    TEXTS.beats[key] = value
  end
end

local CONFIG = ygg.data("ygg:agent/config")
local PLAYBOOK = ygg.data("ygg:agent/playbook")    -- head：schemaVersion/title/summary/source
PLAYBOOK.sections = {}
for _, id in ipairs({
  "ch1-foundation", "ch2-delivery", "ch3-evidence",
  "ch4-run-machine", "ch4-protocol", "ch4-async", "ch4-mod-host",
  "ch5-evolution-tools",
}) do
  local fragment = ygg.data("ygg:agent/playbook/" .. id)
  for _, section in ipairs(fragment.sections) do
    local last = PLAYBOOK.sections[#PLAYBOOK.sections]
    if last ~= nil and last.id == section.id then
      for _, rule in ipairs(section.rules) do
        last.rules[#last.rules + 1] = rule
      end
    else
      PLAYBOOK.sections[#PLAYBOOK.sections + 1] = section
    end
  end
end

local util = require("src.core.util")
local config = require("src.core.config")
local reqs = require("src.domain.requirements")
local lanes = require("src.domain.lanes")
local props = require("src.domain.proposals")
local gates = require("src.domain.gates")
local guide = require("src.view.guide")
local menus = require("src.view.menus")
local render_mod = require("src.view.render")
local wire = require("src.domain.wire")
local flow = require("src.runtime.flow")
local executor = require("src.domain.executor")
local actions = require("src.handlers.intake.actions") -- [deprecated-ref] module path pinned by pack.json code.modules
local lane_actions = require("src.handlers.operate.lanes")
local prop_actions = require("src.handlers.requirements.proposals")
local review = require("src.handlers.operate.review")
local req_ops = require("src.handlers.requirements.requirement_ops")
local gate_actions = require("src.handlers.requirements.gate_actions")
local config_actions = require("src.handlers.system.config")
local views = require("src.handlers.system.views")
local choice = require("src.handlers.operate.choice")
local lifecycle = require("src.handlers.intake.lifecycle") -- [deprecated-ref] module path pinned by pack.json code.modules
local tools = require("src.runtime.tools")
local charter = require("src.handlers.intake.charter")
local integration = require("src.handlers.requirements.integration")
local release = require("src.handlers.operate.release")

-- 注册序（40 点；handler 内的注册顺序 = 其在 install() 里的语句序）：
--   1     agent.summary（render.register）
--   2-4   actions: requirement.add / requirement.list / requirement.dispatch
--   5-6   lanes: lane.claim / lane.release
--   7-9   proposals: proposal.submit / accept / reject
--   10-15 review: coordinate.submit / verify.submit / settlement.evolution / settlement.sessions / settlement.todo / review.note
--   16-20 requirement_ops: split / merge / amend / conflict / resolve
--   21-22 gate_actions: gate.approve / gate.reject
--   23-24 charter: charter.set / run.begin
--   25-26 integration: integration.confirm / integration.reject
--   27-28 release: release.confirm / release.skip
--   29-31 config: config.set / open / close
--   32    views: map.show（完整视图 = 地图 + 当前剧情提示，payload 一律忽略）
--   33    choice: ygg.choice_selected
--   34-35 lifecycle: event.start / node:render
--   36-38 tools: tool.call / tool.result / vfs.write
--   39    fsm.define（本文件末尾）
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
charter.install({ texts = TEXTS })
integration.install({ texts = TEXTS })
release.install({ texts = TEXTS })
tools.register()

local STAGE =
  reqs.STAGE

-- D-MA23（R1）：内核 FSM 唯一转场 = live → retired（review.note 武装路径经 flow.advance 盖章）。
-- 原坐标 / review / done 三值废除；协调 / 盘点 / 发布是 run.phase 微状态（不占转场预算）。
ygg.fsm.define({
  name = "stage",
  initial = STAGE.live,
  event = "stage.advance",
  transitions = {
    {
      from = STAGE.live,
      to = STAGE.retired,
      when = function(target)
        return flow.allowed(target, STAGE.retired)
      end,
    },
  },
})
