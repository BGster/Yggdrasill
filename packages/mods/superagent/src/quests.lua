-- `src/quests.lua` —— 任务系统：委托流转 + 任务板读数（模块顶层纯导出）。
--
-- # 职责（计划 §5 C4）
--
--   * **委托流转**：开局 `open_all`（每条委托 quest.open + giver=NPC 实体 ref、
--     taker 待定）→ `take`（assign taker 实体 + 限期）→ `work`（业务文件落
--     挂载点 + advance）→ `deliver`（回读校验 + advance 至 done + 报酬）。
--     限期 = 开单日 + 类型 deadlineDays；逾期自动 `quest.abandon`（R4 收口）。
--   * **任务板读数**：`board_phase()` 按在场委托状态给出动作集合（take/work/
--     deliver/收工），mod.lua 的选择路由消费。
--
-- # 与包契约的对齐
--
-- `pkg.quest.assign` 的 giver/taker 是**实体 ref**（正整数/nil）——外部 agent
-- 首触由 cast 层 spawn 实体（`sg.agentent.<id>` 缓存），字符串身份留在
-- `sg.agent.<id>` 台账；终态唯一（I3）由包侧钉死，本层不重置不重开（R3）。
--
-- # KV（前缀 sg.，本文件单写者）
--
--   sg.comms                换行委托 key 名表（升序）
--   sg.comm.<key>           quest 实例 id
--   sg.comm.<key>.taker     实体 ref（0 = 待定）
--   sg.comm.<key>.deadline  业务日 int
--   sg.stats                "taken|delivered|abandoned"

local cast = require("src.cast")
local pkg_role = require("pkg.role_skill")
local pkg_quest = require("pkg.quest")
local pkg_entity = require("pkg.entity")
local simframe = require("src.simframe")

local M = { VERSION = "1.0.0" }

local K = {
  comms = "sg.comms",
  comm = "sg.comm.",     -- sg.comm.<key> = quest id
  taker = "sg.comm.%s.taker",
  deadline = "sg.comm.%s.deadline",
  stats = "sg.stats",
  workfile = "sg.comm.%s.workfile",
}

local function host()
  return ygg
end

--- 委托 key 的格式化键（taker/deadline/workfile 三键共用）。
local function ckey(template, key)
  return (template:gsub("%%s", tostring(key)))
end

--- 连接（沙箱方言手写 join——`table.concat` 在真沙箱是 nil，selftest 看不出来）。
local function join_newline(list)
  local out = ""
  for i = 1, #list do
    if i > 1 then
      out = out .. "\n"
    end
    out = out .. tostring(list[i])
  end
  return out
end

--- 名表切分（world.name_table 复用会引循环依赖，这里手写一份同语义）。
local function name_table(text)
  local out = {}
  if type(text) ~= "string" or #text == 0 then
    return out
  end
  for key in (text .. "\n"):gmatch("([^\n]*)\n") do
    if #key > 0 then
      out[#out + 1] = key
    end
  end
  return out
end

--- 升序排序（选择排序；沙箱方言无 table.sort）。
local function sort(list)
  for i = 1, #list do
    local min = i
    for j = i + 1, #list do
      if list[j] < list[min] then
        min = j
      end
    end
    if min ~= i then
      list[i], list[min] = list[min], list[i]
    end
  end
  return list
end

--- taker 规格 → 实体 ref：`clerk:<key>`（在局文员）| `ext:<agent-id>`（外线，
--- 首触即 spawn 实体）| ""（待定）。
function M.taker_ref(spec)
  local h = host()
  if spec == nil or spec == "" then
    return 0
  end
  local kind, name = spec:match("^([a-z]+):(.+)$")
  if kind == "clerk" then
    return h.kv.get("sg.npc." .. name) or 0
  end
  if kind == "ext" then
    local cached = h.kv.get("sg.agentent." .. name)
    if cached ~= nil then
      return cached
    end
    -- 外线 agent 占位实体走包面 spawn（宿主 ygg.entity.spawn 不收属性——
    -- 属性袋是 pkg.entity 叠在宿主实体之上的层）。
    local id = pkg_entity.spawn("bureau.npc", {
      name = name,
      title = "外线 agent",
      kind = "clerk",
      briefing = "",
    })
    pkg_role.grant(id, "bureau.clerk")
    h.kv.set("sg.agentent." .. name, id)
    return id
  end
  error("YGG-MOD-QUEST-TAKER: taker 规格必须是 clerk:<key> 或 ext:<agent-id>，得到 " .. tostring(spec), 0)
end

-- ── 开局：类型注册 + 委托开单 ────────────────────────────────────────────────

--- `open_all(world)`：注册全部 questTypes（幂等）+ 逐条开单。
---
--- giver = 委托人 NPC 实体 ref；taker 待定（0）。返回 {opened=n, ids={…}}。
function M.open_all(world)
  local h = host()
  local define_type = pkg_quest.define
  for i = 1, #world.questTypes do
    local t = world.questTypes[i]
    local steps = {}
    for s = 1, #t.steps do
      steps[s] = { id = t.steps[s], guide = t.steps[s] }
    end
    define_type(t.name, { steps = steps })
  end
  local keys = {}
  local ids = {}
  for i = 1, #world.commissions do
    local c = world.commissions[i]
    local existing = h.kv.get(K.comm .. c.key)
    if existing == nil then
      local id = pkg_quest.open(c.type)
      local giver = h.kv.get("sg.npc." .. c.giver) or 0
      pkg_quest.assign(id, giver, nil)
      local day = h.kv.get("sg.day") or 0
      local deadline = day + 0
      for j = 1, #world.questTypes do
        if world.questTypes[j].name == c.type then
          deadline = day + world.questTypes[j].deadlineDays
        end
      end
      h.kv.set(K.comm .. c.key, id)
      h.kv.set(ckey(K.taker, c.key), 0)
      h.kv.set(ckey(K.deadline, c.key), deadline)
      keys[#keys + 1] = c.key
      ids[c.key] = id
    end
  end
  local merged = {}
  local existing_text = h.kv.get(K.comms)
  if type(existing_text) == "string" and #existing_text > 0 then
    for key in (existing_text .. "\n"):gmatch("([^\n]*)\n") do
      if #key > 0 then
        merged[#merged + 1] = key
      end
    end
  end
  for i = 1, #keys do
    merged[#merged + 1] = keys[i]
  end
  sort(merged)
  h.kv.set(K.comms, join_newline(merged))
  h.kv.set(K.stats, h.kv.get(K.stats) or "0|0|0")
  return { opened = #keys, ids = ids }
end

--- `register(key, id, taker_spec, deadline_days)` → 快照。
---
--- 把一个**已开**的 quest 实例（如自定义任务的直开实例）纳进委托账（键名
--- 空间仍归本文件单写者）。deadline_days 缺省 = 0（不限期）。
function M.register(key, id, taker_spec, deadline_days)
  local h = host()
  if h.kv.get(K.comm .. key) ~= nil then
    error("YGG-MOD-QUEST-DUP: 委托 key 重复注册：" .. tostring(key), 0)
  end
  local ref = 0
  if taker_spec ~= nil and taker_spec ~= "" then
    ref = M.taker_ref(taker_spec)
    pkg_quest.assign(id, nil, ref)
  end
  local day = h.kv.get("sg.day") or 0
  h.kv.set(K.comm .. key, id)
  h.kv.set(ckey(K.taker, key), ref)
  h.kv.set(ckey(K.deadline, key), day + (deadline_days or 0))
  local merged = {}
  local existing_text = h.kv.get(K.comms)
  if type(existing_text) == "string" and #existing_text > 0 then
    for existing in (existing_text .. "\n"):gmatch("([^\n]*)\n") do
      if #existing > 0 then
        merged[#merged + 1] = existing
      end
    end
  end
  merged[#merged + 1] = key
  sort(merged)
  h.kv.set(K.comms, join_newline(merged))
  return M.snapshot(key)
end

--- 委托 key 名表（升序）。
function M.keys()
  return name_table(host().kv.get(K.comms))
end

--- 委托快照：`{key, id, state, step, taker, deadline, open}`。
function M.snapshot(key)
  local h = host()
  local id = h.kv.get(K.comm .. key)
  if id == nil then
    return nil
  end
  local inst = pkg_quest.get(id)
  if inst == nil then
    return nil
  end
  return {
    key = key,
    id = id,
    state = inst.state,
    step = inst.step,
    taker = h.kv.get(ckey(K.taker, key)) or 0,
    deadline = h.kv.get(ckey(K.deadline, key)) or 0,
    open = inst.state == "open",
  }
end

--- 结算清账：从 sim 世界注销一个记录实体（世界规模随游戏进度走——sim.hash
--- 因此携带结算数：同信封序列同哈希、异序列异哈希，重放断言非常数）。
--- 降级（宿主未挂 sim）= 诚实账一行，顶 id 照走（KV 真源不回退）。
local function settle_sim_one(h)
  local top = tonumber(h.kv.get("sg.sim_top") or "0") or 0
  if top < 1 then
    return false
  end
  local ok, err = pcall(function()
    h.sim.despawn_batch(simframe.despawn_one(top))
  end)
  h.kv.set("sg.sim_top", tostring(top - 1))
  if not ok then
    h.narrate("[SIM-UNMOUNTED] 结算清账未进模拟内核（宿主未挂 sim），KV 顶 id 照走 "
      .. tostring(top - 1) .. "；宿主面细节：" .. tostring(err))
    return false
  end
  return true
end

--- 统计读改（taken|delivered|abandoned）。
local function bump(field)
  local h = host()
  local raw = h.kv.get(K.stats) or "0|0|0"
  local taken = tonumber((raw:gsub("([^|]*)|.*", "%1"))) or 0
  local delivered = tonumber(raw:match("|([^|]*)|")) or 0
  local abandoned = tonumber((raw:gsub(".*|", ""))) or 0
  if field == "taken" then
    taken = taken + 1
  elseif field == "delivered" then
    delivered = delivered + 1
  elseif field == "abandoned" then
    abandoned = abandoned + 1
  end
  h.kv.set(K.stats, taken .. "|" .. delivered .. "|" .. abandoned)
  return taken, delivered, abandoned
end

--- 统计读数。
function M.stats()
  local raw = host().kv.get(K.stats) or "0|0|0"
  return tonumber((raw:gsub("([^|]*)|.*", "%1"))) or 0,
    tonumber(raw:match("|([^|]*)|")) or 0,
    tonumber((raw:gsub(".*|", ""))) or 0
end

-- ── 限期（R4：clock 驱动的自动弃单）──────────────────────────────────────────

--- `enforce_deadlines(today)` → 被弃 key 数组；today 缺省 = 当前业务日。
---
--- 只弃「在办」（open 且 taker 已定）的逾期单；待定单没有逾期问题（没人接）。
function M.enforce_deadlines(today)
  local h = host()
  local day = today or (h.kv.get("sg.day") or 0)
  local keys = M.keys()
  local dropped = {}
  for i = 1, #keys do
    local snap = M.snapshot(keys[i])
    if snap ~= nil and snap.open and snap.taker ~= 0 and snap.deadline > 0 and day > snap.deadline then
      pkg_quest.abandon(snap.id, "deadline")
      h.kv.set(ckey(K.taker, keys[i]), 0)
      bump("abandoned")
      settle_sim_one(h)
      dropped[#dropped + 1] = keys[i]
    end
  end
  return dropped
end

-- ── 流转 ────────────────────────────────────────────────────────────────────

--- `commission_for(phase)` → 第一个处于该可动作阶段的委托快照（升序确定性）。
---
--- phase ∈ take（open 且待定）/ work（open 且 taker 已定、step=accept 后）/
--- deliver（open 且 taker 已定、已干活）。
local function first_with(pred)
  local keys = M.keys()
  for i = 1, #keys do
    local snap = M.snapshot(keys[i])
    if snap ~= nil and pred(snap) then
      return snap
    end
  end
  return nil
end

function M.takeable()
  return first_with(function(snap)
    return snap.open and snap.taker == 0
  end)
end

function M.workable()
  return first_with(function(snap)
    return snap.open and snap.taker ~= 0 and snap.step == 1
  end)
end

function M.deliverable()
  return first_with(function(snap)
    return snap.open and snap.taker ~= 0 and snap.step == 2
  end)
end

--- `take(key, taker_spec)` → 快照；key 缺省 = 第一个待定单。
function M.take(key, taker_spec)
  local h = host()
  local snap = key ~= nil and key ~= "" and M.snapshot(key) or M.takeable()
  if snap == nil then
    return nil
  end
  local ref = M.taker_ref(taker_spec or "clerk:clerk-mo")
  -- giver 必须保留（assign 是双写：nil 会把 giver 抹成 0）。
  local inst = pkg_quest.get(snap.id)
  pkg_quest.assign(snap.id, inst ~= nil and inst.giver or nil, ref)
  h.kv.set(ckey(K.taker, snap.key), ref)
  bump("taken")
  return M.snapshot(snap.key)
end

--- `work_file_path(key, prefix, degraded)` → 挂载点/降级 VFS 的业务文件路径。
function M.work_file_path(key, prefix, degraded)
  if degraded then
    return "artifacts/biz/" .. tostring(prefix) .. "-" .. tostring(key) .. ".txt"
  end
  return "mounts/biz/" .. tostring(prefix) .. "-" .. tostring(key) .. ".txt"
end

--- `record_work(key, path, content)` → true；写 + 回读逐字节校验（一致性门）。
---
--- 回读不等 = 写穿失败，报错不推进（诚实账：绝不在没落盘的单上 advance）。
function M.record_work(key, path, content)
  local h = host()
  local ok, written = pcall(function()
    return h.fs.write(path, content)
  end)
  if not ok then
    error("YGG-MOD-QUEST-FS: 业务文件写入失败（" .. tostring(written) .. "）", 0)
  end
  local back = h.fs.read(path)
  if back ~= content then
    error("YGG-MOD-QUEST-FS: 回读与写入不逐字节相等（" .. path .. "）", 0)
  end
  h.kv.set(ckey(K.workfile, key), path)
  return true
end

--- `advance_step(key)` → 推进后的 state（take 后 work/deliver 各推一格）。
function M.advance_step(key)
  local snap = M.snapshot(key)
  if snap == nil or not snap.open then
    error("YGG-MOD-QUEST-STATE: 委托 " .. tostring(key) .. " 不在可推进状态", 0)
  end
  return pkg_quest.advance(snap.id)
end

--- `deliver(key)` → 结果表（state=done + 报酬描述）；key 缺省 = 第一个可交付单。
---
--- 推进口径：从**任意**未-done 状态循环推进到 done（I3 终态由包面钉死：
--- done 上再 advance 会被包面拒绝）。报酬与投影归 finalize，本函数只推一步职责。
function M.deliver(key)
  local snap = key ~= nil and key ~= "" and M.snapshot(key) or M.deliverable()
  if snap == nil then
    return nil
  end
  if not snap.open then
    error("YGG-MOD-QUEST-STATE: 委托 " .. tostring(snap.key) .. " 不在 open（终态唯一）", 0)
  end
  local state = pkg_quest.advance(snap.id)
  while state ~= "done" do
    state = pkg_quest.advance(snap.id)
  end
  return M.finalize(snap.key)
end

--- `finalize(key)` → 结果表；对已 done 的单做报酬 + 交付落账（不推进）。
---
--- deliver 与 harness 回执两条交付路径共用这一个落账点（计费纪律：一次交付
--- 恰一次调用；grant 第二次是零写、recharge 有界、投影是幂等名表）。
function M.finalize(key)
  local h = host()
  local snap = M.snapshot(key)
  if snap == nil then
    return nil
  end
  local world = h.data("ygg:superagent/world")
  local skill = ""
  for i = 1, #world.commissions do
    if world.commissions[i].key == snap.key then
      skill = world.commissions[i].rewardSkill
    end
  end
  local reward = ""
  if snap.taker ~= 0 and skill ~= "" then
    reward = cast.reward(snap.taker, skill)
  end
  bump("delivered")
  settle_sim_one(h)
  -- 持久投影：完成单名表（quest.completed 派生事件不落盘——sqlite/json 两后端
  -- 只记顶层步——这条名表是「已交付」的机器可查事实，随存档往返）。
  local completed = {}
  local raw = h.kv.get("sg.completed")
  if type(raw) == "string" and #raw > 0 then
    for key in (raw .. "\n"):gmatch("([^\n]*)\n") do
      if #key > 0 then
        completed[#completed + 1] = key
      end
    end
  end
  local present = false
  for i = 1, #completed do
    if completed[i] == snap.key then
      present = true
      break
    end
  end
  if not present then
    completed[#completed + 1] = snap.key
  end
  sort(completed)
  h.kv.set("sg.completed", join_newline(completed))
  return { key = snap.key, id = snap.id, state = "done", reward = reward, skill = skill }
end

-- ── 任务板 ──────────────────────────────────────────────────────────────────

--- `completed()` → 已交付（state=done）委托 key 名表（升序）。
function M.completed()
  local out = {}
  local raw = host().kv.get("sg.completed")
  if type(raw) == "string" and #raw > 0 then
    for key in (raw .. "\n"):gmatch("([^\n]*)\n") do
      if #key > 0 then
        out[#out + 1] = key
      end
    end
  end
  return out
end

--- `find_by_taker(entity_id)` → 该承接者名下第一个在办委托快照 | nil。
---
--- 回执绑定回退用：信封 refs 为空时，「这个 agent 来话」的交付上下文 =
--- 它自己接的单（一个 agent 同时至多背一单是本局约定，v1 取第一个在办）。
function M.find_by_taker(entity_id)
  if entity_id == nil or entity_id == 0 then
    return nil
  end
  local keys = M.keys()
  for i = 1, #keys do
    local snap = M.snapshot(keys[i])
    if snap ~= nil and snap.open and snap.taker == entity_id then
      return snap
    end
  end
  return nil
end

--- `open_count()` → open 状态委托数（任务板读数）。
function M.open_count()
  local keys = M.keys()
  local n = 0
  for i = 1, #keys do
    local snap = M.snapshot(keys[i])
    if snap ~= nil and snap.open then
      n = n + 1
    end
  end
  return n
end

--- `board_phase()` → "take"|"work"|"deliver"|"idle"（第一个可动作阶段，升序）。
function M.board_phase()
  if M.deliverable() ~= nil then
    return "deliver"
  end
  if M.workable() ~= nil then
    return "work"
  end
  if M.takeable() ~= nil then
    return "take"
  end
  return "idle"
end

return M
