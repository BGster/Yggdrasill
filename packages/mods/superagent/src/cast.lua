-- `src/cast.lua` —— 人物系统：对话线程绑定 + 报酬成长 + 承接登记（模块顶层纯导出）。
--
-- # 职责（计划 §5 C2）
--
--   * **对话线程绑定**：每个 NPC 一个 `pkg.dialogue` 线程（键 `cast.<npc key>`），
--     每回合 `talk` 落一行（`who=0` 系统行，文本带说话人前缀）。外线 agent 的
--     对话线程键 `agent.<agent-id>`（C5 入向 handler 复用）。
--   * **报酬成长**：交付后 `reward(taker_id, skill)`——未持有则授予（
--     `role.skill_gained`），已持有则回满格（`recharge`）；成长进 `effective`
--     合成面（包侧只读合成，从不写回）。
--   * **承接登记**：外部 agent 首次接触记 `sg.agent.<id> = chats|deliveries`
--     计数账（C5 幂等/落账共用）。
--
-- # 线程键与 KV
--
-- 线程内容在 pkg.dialogue 的 KV（`dlg.*` 前缀，包侧单写者）；本文件只写
-- `sg.agent.*` 与读 `cast.*` 键约定。观察面：线程行数经 agent.summary meta
-- 带出（`dlgThreads` / `dlgLines`），事件页投影归入向 handler（D1 语义）。

local pkg_dialogue = require("pkg.dialogue")
local pkg_role = require("pkg.role_skill")

local M = { VERSION = "1.0.0" }

local K_AGENT = "sg.agent." -- sg.agent.<agent-id> = "chats|deliveries"

local function host()
  return ygg
end

--- NPC 的对话线程键。
function M.thread_of(npc_key)
  return "cast." .. tostring(npc_key)
end

--- 外线 agent 的对话线程键。
function M.agent_thread(agent_id)
  return "agent." .. tostring(agent_id)
end

--- `talk(npc_key, who_label, line)` → 线程行数（落一行，行文 `<who>: <line>`）。
---
--- 事件直发与选择路由共用；一行一回合，`pkg.dialogue` 的窗口帽照常生效。
function M.talk(npc_key, who_label, line)
  local thread = M.thread_of(npc_key)
  pkg_dialogue.say(thread, who_label .. ": " .. tostring(line))
  return #pkg_dialogue.history(thread)
end

--- `agent_say(agent_id, who_label, line)` → 线程行数（外线 agent 回合）。
function M.agent_say(agent_id, who_label, line)
  local thread = M.agent_thread(agent_id)
  pkg_dialogue.say(thread, who_label .. ": " .. tostring(line))
  return #pkg_dialogue.history(thread)
end

--- `history(npc_key)` → 线程行副本（冒烟断言面）。
function M.history(npc_key)
  return pkg_dialogue.history(M.thread_of(npc_key))
end

--- `threads_stat()` → `threads, lines`（线程数 / 总行数；summary meta 用）。
function M.threads_stat()
  local keys = pkg_dialogue.threads()
  local lines = 0
  for i = 1, #keys do
    lines = lines + #pkg_dialogue.history(keys[i])
  end
  return #keys, lines
end

-- ── 报酬成长（交付 → role-skill 成长）────────────────────────────────────────

--- `reward(taker_id, skill)` → 描述串；未持有=授予，已持有=回满格。
---
--- 计费纪律：一次交付恰一次调用（quests.lua 交付路径单点），I3 终态唯一在
--- 任务侧已钉，报酬侧不重复发（grant 第二次是零写，recharge 有界）。
function M.reward(taker_id, skill)
  local state = pkg_role.skill_state(taker_id, skill)
  if state ~= nil then
    local cap = state.capacity or 3
    local restored = pkg_role.recharge(taker_id, skill, cap)
    return "recharged:" .. tostring(skill) .. "=" .. tostring(restored)
  end
  pkg_role.grant_skill(taker_id, skill)
  return "granted:" .. tostring(skill)
end

-- ── 承接登记（外部 agent 台账；C5 幂等落账共用）────────────────────────────

--- `touch_agent(agent_id, field)` → 计数（field ∈ chats|deliveries）。
function M.touch_agent(agent_id, field)
  local h = host()
  local raw = h.kv.get(K_AGENT .. agent_id) or "0|0"
  local chats = tonumber((raw:gsub("([^|]*)|.*", "%1"))) or 0
  local deliveries = tonumber((raw:gsub("[^|]*|", ""))) or 0
  if field == "chats" then
    chats = chats + 1
  elseif field == "deliveries" then
    deliveries = deliveries + 1
  end
  h.kv.set(K_AGENT .. agent_id, chats .. "|" .. deliveries)
  return field == "deliveries" and deliveries or chats
end

--- `agent_stat(agent_id)` → `chats, deliveries`（无记录 = 0, 0）。
function M.agent_stat(agent_id)
  local raw = host().kv.get(K_AGENT .. agent_id)
  if raw == nil then
    return 0, 0
  end
  local chats = tonumber((raw:gsub("([^|]*)|.*", "%1"))) or 0
  local deliveries = tonumber((raw:gsub("[^|]*|", ""))) or 0
  return chats, deliveries
end

return M
