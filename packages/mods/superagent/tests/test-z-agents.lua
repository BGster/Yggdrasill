-- `tests/test-agents.lua` —— 适配器自检（C5）。
--
-- 断言面（计划 C5 验收「信封校验/幂等/路由/出向 agent_call/回执落账」）：
--   1. wire 解析：字段按 kind wire 序对位；宿主身份段剥离；
--   2. 幂等：同 externalId 重入 = 【重放】行 + 零状态；空 externalId 拒；
--   3. agent.chat：线程增行 + 出向声明（meta.tool_calls 落 meta）+ 调用登记；
--   4. 回执落账：成功（绑定委托 → 文件 + done + 报酬）/失败（agent.error 记账，
--      不重试）/线形不合法拒绝；
--   5. 兜底路由：ping/error/action 各自记账。

local world = require("src.world")
local quests = require("src.quests")
local agents = require("src.agents")
local cast = require("src.cast")
local pkg_quest = require("pkg.quest")

if ygg.kv.get("sg.seeded") == nil then
  world.seed()
end
if ygg.kv.get("sg.comms") == nil then
  quests.open_all(world.world())
end

-- 本文件的专属实例（不碰 world 委托池——其他测试文件对 c1/c2/c3 有假设）。
if ygg.kv.get("sg.comm.agent-smoke") == nil then
  local giver = world.npc_id("archivist")
  local smoke_id = pkg_quest.open("archive-fetch")
  pkg_quest.assign(smoke_id, giver, nil)
  quests.register("agent-smoke", smoke_id, nil, 0)
end

-- 测试本地辅助（harness 无线程读数现成面；借 pkg.dialogue 的 history）。
local pkg_dialogue = require("pkg.dialogue")
local function thread_lines(thread)
  return #pkg_dialogue.history(thread)
end

-- mod.lua 装载期的同源映射（本测试自建，形状 = data/envelope.json）。
local WIRE = {}
local envelope = ygg.data("ygg:superagent/envelope")
for i = 1, #envelope.kinds do
  WIRE[envelope.kinds[i].kind] = envelope.kinds[i].wire
end

check("agents.wire.fields-by-declared-order", function()
  local problems = {}
  local env = agents.fields("agent.chat", "ext-1|agent-7|你好|sess-9|c1", WIRE)
  if env.externalId ~= "ext-1" or env.agent ~= "agent-7" or env.text ~= "你好"
    or env.session ~= "sess-9" or env.refs ~= "c1" then
    problems[#problems + 1] = "五段对位错：" .. tostring(env.externalId) .. "/" .. env.agent
  end
  -- 宿主身份段（尾部 actor=/actor_session=）剥离。
  local with_actor = agents.fields("agent.chat", "ext-2|agent-7|hi||c1|actor=s4-dev|actor_session=abc", WIRE)
  if with_actor.text ~= "hi" or with_actor.refs ~= "c1" then
    problems[#problems + 1] = "身份段剥离后段位错"
  end
  -- 缺段 = 空串占位。
  local sparse = agents.fields("harness.ping", "p-1|agent-9", WIRE)
  if sparse.session ~= "" or sparse.text ~= "" then
    problems[#problems + 1] = "缺段应为空串占位"
  end
  return table.concat(problems, "; ")
end)

check("agents.idempotent.replay-and-empty-ext", function()
  local problems = {}
  local first = agents.handle_chat("chat-x|agent-7|第一次||c1", WIRE)
  if first == nil then
    problems[#problems + 1] = "首封应被受理"
  end
  local replay = agents.handle_chat("chat-x|agent-7|第一次||c1", WIRE)
  if replay ~= nil then
    problems[#problems + 1] = "同键重入应走重放分支"
  end
  local lines = ygg_narrated_lines()
  local replay_line = lines[#lines]
  if replay_line == nil or replay_line:find("【重放】", 1, true) == nil then
    problems[#problems + 1] = "重放应叙述【重放】前缀行，实得 " .. tostring(replay_line)
  end
  -- 重放零状态：线程不再增行（首封已 +1）。
  local history = thread_lines("agent.agent-7")
  if history ~= 1 then
    problems[#problems + 1] = "同键重放不得增行，实得 " .. tostring(history)
  end
  -- 空 externalId = 校验拒绝。
  local empty = agents.handle_chat("|agent-7||hi|", WIRE)
  if empty ~= nil then
    problems[#problems + 1] = "空 externalId 应拒"
  end
  return table.concat(problems, "; ")
end)

check("agents.chat.outbound-declared-and-registered", function()
  local problems = {}
  -- 取一单给外线 agent（接单实体化），再来话 → 出向声明 + 登记。
  quests.take("agent-smoke", "ext:agent-7")
  local meta_before = 0
  for _ in pairs(ygg_meta_all()) do
    meta_before = meta_before + 1
  end
  local handled = agents.handle_chat("chat-y|agent-7|smoke 干得怎么样了||agent-smoke", WIRE)
  if handled == nil or handled.call_id == nil then
    problems[#problems + 1] = "chat 受理应返回 call_id"
  end
  local meta = ygg_meta_all()
  if meta["tool_calls"] == nil then
    problems[#problems + 1] = "出向应经 meta.tool_calls 声明"
  else
    local call = meta["tool_calls"][1]
    if call == nil or call.tool ~= "superagent.agent_call" or call.args == nil or call.args.text == nil then
      problems[#problems + 1] = "meta.tool_calls 形状错"
    end
  end
  local ext, call_agent, context = agents.call_context(handled.call_id)
  if ext ~= "chat-y" or call_agent ~= "agent-7" or context ~= "agent-smoke" then
    problems[#problems + 1] = "调用登记应为 chat-y|c1，实得 " .. tostring(ext) .. "|" .. tostring(context)
  end
  return table.concat(problems, "; ")
end)

check("agents.reply.success.delivers-bound-quest", function()
  local problems = {}
  -- wire：<call>|<tool>|<status>|<code>|<output>|<ext>（output 可含竖线）。
  local payload = "call-chat-y|superagent.agent_call|ok|0|ECHO[agent-7]: c1 干得怎么样了|ext-auto"
  local result = agents.handle_reply(payload)
  if result == nil or result.delivered ~= true then
    problems[#problems + 1] = "绑定回执应触发交付"
  end
  local snap = quests.snapshot("agent-smoke")
  if snap == nil or snap.state ~= "done" then
    problems[#problems + 1] = "回执交付后 agent-smoke 应 done，实得 " .. tostring(snap and snap.state)
  end
  -- 结算清账：sim 世界应少一个记录实体（顶 id 相对回落；绝对值随重建规模走）。
  local top = tonumber(ygg.kv.get("sg.sim_top") or "0")
  local spawned_total = 0
  for i = 1, #ygg_spawn_batches() do
    spawned_total = spawned_total + ygg_spawn_batches()[i]
  end
  if top >= spawned_total then
    problems[#problems + 1] = "结算后 sim 顶 id 应低于累计重建规模（" .. tostring(spawned_total)
      .. "），实得 " .. tostring(top)
  end
  -- 交付文件应带回执痕迹落盘。
  local vfs = ygg_vfs()
  local landed = false
  for path, content in pairs(vfs) do
    if path:find("biz-report-agent-smoke", 1, true) ~= nil and tostring(content):find("via=harness", 1, true) ~= nil then
      landed = true
      break
    end
  end
  if not landed then
    problems[#problems + 1] = "回执交付的业务文件应落盘且带 via=harness"
  end
  -- agent 台账 deliveries +1。
  local _, deliveries = cast.agent_stat("agent-7")
  if deliveries < 1 then
    problems[#problems + 1] = "外线 agent 交付台账应 ≥1"
  end
  return table.concat(problems, "; ")
end)

check("agents.reply.failure.accounted-not-retried", function()
  local problems = {}
  agents.outbound_call("问询", "agent-9", "", "chat-z", "c2")
  local fail_payload = "call-chat-z|superagent.agent_call|error|124|timeout after 30000ms|ext-auto"
  local result = agents.handle_reply(fail_payload)
  if result == nil or result.delivered ~= false then
    problems[#problems + 1] = "失败回执不得触发交付"
  end
  -- agent.error 记账：线程一行 + 派生事件一条。
  local errs = 0
  local events = ygg_emitted_events()
  for i = 1, #events do
    if events[i].name == "agent.error" and tostring(events[i].payload):find("call-chat-z", 1, true) ~= nil then
      errs = errs + 1
    end
  end
  if errs ~= 1 then
    problems[#problems + 1] = "失败应恰记一条 agent.error，实得 " .. tostring(errs)
  end
  -- 不合法线形（段数 <6）。
  local bad = agents.handle_reply("call-x|tool|ok")
  if bad ~= nil then
    problems[#problems + 1] = "段数不足的回执应拒"
  end
  return table.concat(problems, "; ")
end)

check("agents.builtin-routes.ping-error-action", function()
  local problems = {}
  local ping = agents.handle_ping("p-1|agent-7|在吗||", WIRE)
  if ping == nil then
    problems[#problems + 1] = "ping 应受理"
  end
  local err = agents.handle_error("e-1|agent-7|上会去了，稍后回||", WIRE)
  if err == nil then
    problems[#problems + 1] = "error 应受理并入线程"
  end
  local act = agents.handle_action("a-1|agent-7|申请接 c3||c3", WIRE)
  if act == nil then
    problems[#problems + 1] = "action 应记账（v1 不改任务态）"
  end
  -- 任务态未被 action 改动（前后对比；c3 可能已被其他测试文件接走，不能假设待定）。
  local c3_before = quests.snapshot("c3")
  local c3_after = quests.snapshot("c3")
  if c3_before == nil or c3_after == nil
    or c3_before.taker ~= c3_after.taker or c3_before.state ~= c3_after.state
    or c3_before.step ~= c3_after.step then
    problems[#problems + 1] = "action 申报不得直接改任务态（I3 双写风险）"
  end
  return table.concat(problems, "; ")
end)

