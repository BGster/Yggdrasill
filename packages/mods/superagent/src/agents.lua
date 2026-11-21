-- `src/agents.lua` —— 适配器：入向信封校验/幂等/路由 + 出向 agent_call + 回执落账。
--
-- # 职责（计划 §5 C5；信封契约见 data/envelope.json 与设计文档 §2）
--
--   * **信封解析**：线载荷（宿主 drain 注入的 pipe 段串）→ 字段表（按 kind 的
--     wire 序对位；宿主身份段 `|actor=` / `|actor_session=` 剥离——宿主是唯一
--     盖章方）。
--   * **幂等**：`sg.seen.<externalId> = kind`。同键重入 = 重放：叙述
--     `【重放】` 行（宿主 replayPrefix 语义：算成功消费、零状态变更）。
--   * **路由**：六类 kind 的处理体（chat/action/delivery/ping/error 在本文件，
--     custom 在 mod 层接 custom.lua——翻译器是任务系统的能力出口）。
--   * **出向**：`outbound_call(text, agent, session, call_ext, context)` ——
--     `ygg.meta("tool_calls", …)` 声明（宿主在飞工具桥，inlineWaitMs>0 同进程
--     就地回投）+ 调用登记 `sg.call.<call_id> = ext|context`。
--   * **回执落账**：工具线形 `<call_id>|<tool_id>|<status>|<code>|<output>|<ext>`；
--     解析（两端切分，output 可含竖线）→ 失败 = agent.error 记账（不自动重试，
--     R2）；成功 = 按登记 context 走交付落账（quests.deliver）。
--
-- # 确定性
--
-- 出向走 inline（同进程就地回投）：信封序列相同 ⇒ 工具输出相同 ⇒ sim 步数与
-- KV 状态逐字节相同 ⇒ 「同信封序列重放 sim.hash 逐字节相等」可复算。

local cast = require("src.cast")
local quests = require("src.quests")
local clock = require("src.clock")
local pkg_quest = require("pkg.quest")

local M = { VERSION = "1.0.0" }

local K_SEEN = "sg.seen."   -- sg.seen.<externalId> = kind
local K_CALL = "sg.call."   -- sg.call.<call_id> = "<ext>|<context>"

local function host()
  return ygg
end

--- 线载荷切段（宿主身份段剥离：尾部 actor= / actor_session= 前缀段）。
function M.wire_segments(payload)
  local out = {}
  for seg in (tostring(payload) .. "|"):gmatch("([^|]*)|") do
    out[#out + 1] = seg
  end
  while #out > 0 do
    local last = out[#out]
    if last:find("actor=", 1, true) == 1 then
      out[#out] = nil
    else
      break
    end
  end
  return out
end

--- 信封字段表：`fields(kind, payload, wire_map)` —— wire_map = kind → 段名数组
--- （mod.lua 装载期从 data/envelope.json 构建）。
function M.fields(kind, payload, wire_map)
  local segs = M.wire_segments(payload)
  local names = wire_map[kind] or {}
  local out = {}
  for i = 1, #names do
    out[names[i]] = segs[i] or ""
  end
  out._segments = segs
  return out
end

--- 幂等闸：返回 true = 新信封（可处理）；false = 重放（调用方只叙述）。
function M.first_touch(external_id, kind)
  local h = host()
  if external_id == nil or external_id == "" then
    return false, "externalId 为空"
  end
  local prior = h.kv.get(K_SEEN .. external_id)
  if prior ~= nil then
    return false, nil
  end
  h.kv.set(K_SEEN .. external_id, kind)
  return true, nil
end

--- 工具结果线形解析（两端切分：output 可含竖线）。
---
--- `<call_id>|<tool_id>|<status>|<code>|<output>|<external_id>` → 表；段数 <6 = nil。
function M.parse_result(payload)
  local segs = M.wire_segments(payload)
  if #segs < 6 then
    return nil
  end
  local call_id = segs[1]
  local tool_id = segs[2]
  local status = segs[3]
  local code = segs[4]
  local ext_id = segs[#segs]
  local output = segs[5]
  for i = 6, #segs - 1 do
    output = output .. "|" .. segs[i]
  end
  return { call_id = call_id, tool_id = tool_id, status = status, code = code, output = output, ext = ext_id }
end

-- ── 出向 ────────────────────────────────────────────────────────────────────

--- `outbound_call(text, agent, session, ext, context)` → call_id。
---
--- 声明即走宿主在飞工具桥（pack.json tools.superagent.agent_call）；登记
--- `sg.call.<id> = ext|context` 供回执落账对号。
function M.outbound_call(text, agent, session, ext, context)
  local h = host()
  local call_id = "call-" .. tostring(ext)
  h.kv.set(K_CALL .. call_id, tostring(ext) .. "|" .. tostring(agent) .. "|" .. tostring(context or ""))
  h.meta("tool_calls", {
    {
      id = call_id,
      tool = "superagent.agent_call",
      args = { text = tostring(text), agent = tostring(agent), session = tostring(session or "") },
    },
  })
  return call_id
end

--- 登记读回（回执落账用）→ `ext, agent, context`。
function M.call_context(call_id)
  local raw = host().kv.get(K_CALL .. call_id)
  if raw == nil then
    return nil, nil, nil
  end
  local segs = {}
  for seg in (raw .. "|"):gmatch("([^|]*)|") do
    segs[#segs + 1] = seg
  end
  return segs[1], segs[2] or "", segs[3] or ""
end

-- ── 入向路由体（mod.lua 的 kind handler 委托到这里）─────────────────────────

--- agent.chat：对话一回合 + 出向 agent_call（context = refs 首段，可空）。
function M.handle_chat(payload, wire_map)
  local h = host()
  local env = M.fields("agent.chat", payload, wire_map)
  local fresh, why = M.first_touch(env.externalId, "agent.chat")
  if not fresh then
    h.narrate("【重放】agent.chat 信封 " .. tostring(env.externalId) .. (why ~= nil and ("（" .. why .. "）") or "") .. " 已消费过，零状态变更。")
    return nil
  end
  if env.agent == "" then
    h.narrate("【拒绝】agent.chat 缺 agent 段（信封校验失败）。")
    h.emit("agent.error", (env.externalId ~= "" and env.externalId or "?") .. "|missing-agent")
    return nil
  end
  local lines = cast.agent_say(env.agent, env.agent, env.text ~= "" and env.text or "(空消息)")
  cast.touch_agent(env.agent, "chats")
  h.emit("agent.dialogue", cast.agent_thread(env.agent) .. "|" .. tostring(lines) .. "|" .. env.text)
  local context = env.refs
  local bar = context:find(",", 1, true)
  if bar ~= nil then
    context = context:sub(1, bar - 1)
  end
  local call_id = M.outbound_call(env.text, env.agent, env.session, env.externalId, context)
  h.narrate("已受理 agent " .. env.agent .. " 的来话（线程 " .. tostring(lines) .. " 行），外线征询中（" .. call_id .. "）。")
  clock.advance()
  return { env = env, call_id = call_id, context = context }
end

--- agent.action：动作申报按 refs/text 落台账（v1 不直接改任务态——任务态的
--- 变更只走 commission.* 游戏事件或交付回执，双写面是 I3 风险）。
function M.handle_action(payload, wire_map)
  local h = host()
  local env = M.fields("agent.action", payload, wire_map)
  local fresh, why = M.first_touch(env.externalId, "agent.action")
  if not fresh then
    h.narrate("【重放】agent.action 信封 " .. tostring(env.externalId) .. (why ~= nil and ("（" .. why .. "）") or "") .. " 已消费过。")
    return nil
  end
  cast.touch_agent(env.agent ~= "" and env.agent or "?", "chats")
  h.emit("agent.recorded", "action|" .. tostring(env.agent) .. "|" .. env.text)
  h.narrate("动作申报已记账：" .. (env.text ~= "" and env.text or "(空)") .. "（refs=" .. env.refs .. "）。")
  clock.advance()
  return env
end

--- quest.delivery：外线交付申报（refs 首段 = 委托 key）→ 走 quests 交付落账。
function M.handle_delivery(payload, wire_map)
  local h = host()
  local env = M.fields("quest.delivery", payload, wire_map)
  local fresh, why = M.first_touch(env.externalId, "quest.delivery")
  if not fresh then
    h.narrate("【重放】quest.delivery 信封 " .. tostring(env.externalId) .. (why ~= nil and ("（" .. why .. "）") or "") .. " 已消费过。")
    return nil
  end
  local key = env.refs
  local comma = key:find(",", 1, true)
  if comma ~= nil then
    key = key:sub(1, comma - 1)
  end
  local snap = key ~= "" and quests.snapshot(key) or quests.deliverable()
  if snap == nil or not snap.open or snap.taker == 0 then
    h.narrate("【拒绝】交付申报对不上在办委托（refs=" .. env.refs .. "）。")
    h.emit("agent.error", (env.agent ~= "" and env.agent or "?") .. "|delivery-no-target")
    return nil
  end
  local result = quests.deliver(snap.key)
  if result == nil then
    h.emit("agent.error", (env.agent ~= "" and env.agent or "?") .. "|delivery-failed")
    return nil
  end
  cast.touch_agent(env.agent ~= "" and env.agent or "?", "deliveries")
  h.emit("agent.recorded", "delivered|" .. result.key .. "|" .. result.state)
  h.narrate("外线交付入账：" .. result.key .. " → " .. result.state .. "（" .. result.reward .. "）。")
  clock.advance()
  return result
end

--- harness.ping：存活探测只记账（不改任务态）。
function M.handle_ping(payload, wire_map)
  local h = host()
  local env = M.fields("harness.ping", payload, wire_map)
  local fresh, why = M.first_touch(env.externalId, "harness.ping")
  if not fresh then
    h.narrate("【重放】harness.ping 信封 " .. tostring(env.externalId) .. (why ~= nil and ("（" .. why .. "）") or "") .. " 已消费过。")
    return nil
  end
  h.emit("agent.recorded", "ping|" .. tostring(env.agent))
  h.narrate("harness 存活：" .. (env.agent ~= "" and env.agent or "?") .. "（" .. env.text .. "）。")
  clock.advance()
  return env
end

--- agent.error：错误上报入对话线程（R2：失败记账，不自动重试）。
function M.handle_error(payload, wire_map)
  local h = host()
  local env = M.fields("agent.error", payload, wire_map)
  local fresh, why = M.first_touch(env.externalId, "agent.error")
  if not fresh then
    h.narrate("【重放】agent.error 信封 " .. tostring(env.externalId) .. (why ~= nil and ("（" .. why .. "）") or "") .. " 已消费过。")
    return nil
  end
  local agent = env.agent ~= "" and env.agent or "?"
  local lines = cast.agent_say(agent, "harness", "ERROR " .. env.text)
  h.emit("agent.error", agent .. "|" .. env.text)
  h.narrate("错误入账（线程 " .. tostring(lines) .. " 行）：" .. agent .. " — " .. env.text)
  clock.advance()
  return env
end

--- accept→work→deliver→done 的推进（委托在 work 步前接单时还差三步；
--- 只推到 done 为止，I3 终态恰一次由包面钉死）。
local function advance_to_done(key)
  local snap = quests.snapshot(key)
  if snap == nil then
    return nil
  end
  local state = pkg_quest.advance(snap.id)
  while state ~= "done" do
    state = pkg_quest.advance(snap.id)
  end
  return state
end

--- 工具回执落账（harness.replied；宿主 inline 回投的 resultEvent）。
---
--- 成功 = 按登记 context 交付（context 是委托 key 时）；失败 = agent.error 记账。
--- 两种终态都叙述，不自动重试（R2）。
function M.handle_reply(payload)
  local h = host()
  local result = M.parse_result(payload)
  if result == nil then
    h.narrate("【拒绝】工具回执线形不合法（段数 <6）：" .. tostring(payload))
    return nil
  end
  local _, agent, context = M.call_context(result.call_id)
  if result.status ~= "ok" or result.code ~= "0" then
    cast.agent_say("harness", "harness", "CALL-FAIL " .. result.call_id .. " status=" .. result.status .. " code=" .. result.code)
    h.emit("agent.error", "harness|" .. result.call_id .. "|status=" .. result.status)
    h.narrate("外线征询失败已记账（" .. result.call_id .. " status=" .. result.status .. " code=" .. result.code .. "），不自动重试。")
    return { call = result, delivered = false }
  end
  -- 绑定回退：refs 未指委托（--requirement 只有 agent/text 两段）→ 该 agent
  -- 自己接的在办单（taker 实体反查；一次只背一单是 v1 约定）。
  if (context == nil or context == "") and agent ~= nil and agent ~= "" then
    local agent_entity = host().kv.get("sg.agentent." .. agent)
    local mine = quests.find_by_taker(agent_entity)
    if mine ~= nil then
      context = mine.key
    end
  end
  if context ~= nil and context ~= "" then
    local snap = quests.snapshot(context)
    if snap ~= nil and snap.open and snap.taker ~= 0 then
      local config = h.data("ygg:superagent/config")
      local path = quests.work_file_path(snap.key, config.game.reportPrefix, false)
      local content = "bureau-report|" .. snap.key .. "|via=harness|reply=" .. result.output
      local ok = pcall(function()
        quests.record_work(snap.key, path, content)
      end)
      if not ok then
        h.narrate("【拒绝】回执触发交付的文件写穿失败（" .. path .. "），委托保持原状。")
        return { call = result, delivered = false }
      end
      -- 推进至 done（回执路径从 accept 步起，循环推进）+ 单点落账（finalize
      -- 不推进——双重推进必撞包面 I3）。
      advance_to_done(snap.key)
      local done = quests.finalize(snap.key)
      if agent ~= nil and agent ~= "" then
        cast.touch_agent(agent, "deliveries")
      end
      h.emit("agent.recorded", "delivered|" .. context .. "|via-harness")
      h.narrate("外线交付完成：" .. context .. "（回执：" .. result.output .. "）" .. (done ~= nil and ("，报酬 " .. done.reward) or ""))
      return { call = result, delivered = true }
    end
  end
  h.narrate("外线回执落账：" .. result.output .. "（无绑定委托，仅记账）。")
  return { call = result, delivered = false }
end

return M
