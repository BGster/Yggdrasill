-- `src/ewd998.lua` —— EWD998（Safra 环上终止检测；分布式计数 + 令牌有界）。
--
-- 契约：openwiki/contracts/mod/tla-models.md §映射表。源：tlaplus/Examples
-- `specifications/ewd998/EWD998.tla` + `EWD998.cfg`（N=4 档 TLC 实测
-- 4097/53271/14 系源仓 manifest 对 AsyncTerminationDetection 的记录，引用如实）。
-- 本移植取 N=2 小轨道（KAT 手算口径）。
--
-- 映射：`active/color/counter/pending`（每节点函数）→ `tla.node` 实体
-- schema `{id, active, color, counter, pending}`（每身份态一实体）；
-- `token`（记录）→ KV 标量三键 `tla.ewd.token.pos/q/color`；`Init`
-- 非确定域（active/color 全布尔/全色域）→ 钉规范初态：全 active / 全 white /
-- token(0,0,black)（契约 §语义子集声明；P4 分支使 Inv 在钉初态成立）；
-- `Next` 析取 → `step("send:i:j" / "recv:i" / "deact:i" / "probe" / "pass:i" /
-- "stutter")`，守卫不满足报 `YGG-MOD-TLA-GUARD`；
-- `TypeOK` / `Inv`（Safra 归纳不变量 P0 ∧ (P1∨P2∨P3∨P4)）/
-- `TerminationDetection`（terminationDetected ⇒ Termination 安全蕴含）→
-- inv 注册表；`terminationDetected` 谓词另暴露 `M.termination_detected()`
-- （终态 KAT 断言面）。WF_vars(System) 公平性属活性——裁剪（契约声明）。

local entity = require("pkg.entity")
local inv = require("src.inv")

local M = { VERSION = "0.1.0", name = "ewd998" }

local N = 2
M.N = N

local SCHEMA = "tla.node"

--- reset 期刷新的节点 id 缓存（ids_by_node[i]；热路径免 KV 全扫——旧版
--- id_of 每调用一次 entity.ids() 的 O(seq) 扫描，是 PERF 首测 1ms/拍的主因）。
local ids_by_node = nil

--- 沙箱方言：模块顶层纯；ygg 经 host() 单点取。
local function host()
  return ygg
end

local function all_ids()
  return entity.ids(nil, SCHEMA)
end

--- i 号节点的实体 id（reset 期缓存；O(1) 热路径）。
local function id_of(i)
  if ids_by_node ~= nil then
    return ids_by_node[i]
  end
  local ids = all_ids()
  for k = 1, #ids do
    if entity.get(ids[k], "id") == i then
      return ids[k]
    end
  end
  return nil
end

-- ── token 的 KV 三键 ──────────────────────────────────────────────────────────

local function token_get(key)
  return host().kv.get("tla.ewd.token." .. key)
end

local function token_set(key, value)
  host().kv.set("tla.ewd.token." .. key, value)
end

-- ── 规范工具（R4 整数纪律：手写 is_integer）─────────────────────────────────

local function is_integer(v)
  return type(v) == "number" and v == (v // 1)
end

--- `Rng(a,b)`（源定义逐字：Node 内的闭区间）。
local function rng(a, b)
  local out = {}
  for i = 0, N - 1 do
    if a <= i and i <= b then
      out[#out + 1] = i
    end
  end
  return out
end

--- `Sum(f, S)`（源 Sum 逐字：定义域受限求和）。
local function sum_counter(range_list)
  local total = 0
  for k = 1, #range_list do
    total = total + entity.get(id_of(range_list[k]), "counter")
  end
  return total
end

--- `B == Sum(pending, Node)`（在途消息数）。
local function big_b()
  local total = 0
  for i = 0, N - 1 do
    total = total + entity.get(id_of(i), "pending")
  end
  return total
end

-- ── 不变量 ───────────────────────────────────────────────────────────────────

--- `TypeOK`：active∈BOOL / color∈Color / counter∈Int / pending∈Nat /
--- token∈Token（pos∈Node, q∈Int, color∈Color）。
local function check_type_ok()
  local violations = {}
  for i = 0, N - 1 do
    local id = id_of(i)
    if id == nil then
      violations[#violations + 1] = "缺节点 " .. i
    else
      if type(entity.get(id, "active")) ~= "boolean" then
        violations[#violations + 1] = "active[" .. i .. "] 非布尔"
      end
      local color = entity.get(id, "color")
      if color ~= "white" and color ~= "black" then
        violations[#violations + 1] = "color[" .. i .. "] ∉ Color：" .. tostring(color)
      end
      if not is_integer(entity.get(id, "counter")) then
        violations[#violations + 1] = "counter[" .. i .. "] 非整：" .. tostring(entity.get(id, "counter"))
      end
      local pending = entity.get(id, "pending")
      if not is_integer(pending) or pending < 0 then
        violations[#violations + 1] = "pending[" .. i .. "] ∉ Nat：" .. tostring(pending)
      end
    end
  end
  local pos = token_get("pos")
  if not is_integer(pos) or pos < 0 or pos > N - 1 then
    violations[#violations + 1] = "token.pos ∉ Node：" .. tostring(pos)
  end
  if not is_integer(token_get("q")) then
    violations[#violations + 1] = "token.q 非整：" .. tostring(token_get("q"))
  end
  local tcolor = token_get("color")
  if tcolor ~= "white" and tcolor ~= "black" then
    violations[#violations + 1] = "token.color ∉ Color：" .. tostring(tcolor)
  end
  return violations
end

--- `Inv`（Safra 归纳不变量，源逐字）：P0 ∧ (P1 ∨ P2 ∨ P3 ∨ P4)。
local function check_inv()
  local violations = {}
  if big_b() ~= sum_counter(rng(0, N - 1)) then
    violations[#violations + 1] = "P0: B ≠ Sum(counter)"
    return violations
  end
  local pos = token_get("pos")
  local q = token_get("q")
  -- P1：token 后方全被动且 q 与计数一致。
  local p1 = true
  for _, i in ipairs(rng(pos + 1, N - 1)) do
    if entity.get(id_of(i), "active") then
      p1 = false
      break
    end
  end
  if p1 then
    local q_exp = 0
    if pos ~= N - 1 then
      q_exp = sum_counter(rng(pos + 1, N - 1))
    end
    if q ~= q_exp then
      p1 = false
    end
  end
  if p1 then
    return violations
  end
  -- P2：token 前方计数和 + q > 0。
  if sum_counter(rng(0, pos)) + q > 0 then
    return violations
  end
  -- P3：token 前方有黑节点。
  for _, i in ipairs(rng(0, pos)) do
    if entity.get(id_of(i), "color") == "black" then
      return violations
    end
  end
  -- P4：令牌黑。
  if token_get("color") == "black" then
    return violations
  end
  violations[#violations + 1] = "P1..P4 全不成立"
  return violations
end

--- `terminationDetected`（源状态谓词逐字）。
local function termination_detected()
  local pos = token_get("pos")
  local q = token_get("q")
  local tcolor = token_get("color")
  return pos == 0 and tcolor == "white"
    and q + entity.get(id_of(0), "counter") == 0
    and entity.get(id_of(0), "color") == "white"
    and entity.get(id_of(0), "active") == false
end

--- `Termination`：全被动 ∧ B=0。
local function termination()
  if big_b() ~= 0 then
    return false
  end
  for i = 0, N - 1 do
    if entity.get(id_of(i), "active") then
      return false
    end
  end
  return true
end

--- `TerminationDetection`（cfg INVARIANT：安全蕴含逐字）。
local function check_termination_detection()
  if termination_detected() and not termination() then
    return { "terminationDetected 成立但 Termination 不成立" }
  end
  return {}
end

-- ── 装配 ─────────────────────────────────────────────────────────────────────

function M.install()
  entity.define(SCHEMA, {
    attrs = {
      id = { type = "number", required = true },
      active = { type = "boolean", required = true },
      color = { type = "string", required = true },
      counter = { type = "number", required = true },
      pending = { type = "number", required = true },
    },
  })
  inv.register(M.name, "TerminationDetection", check_termination_detection)
  inv.register(M.name, "TypeOK", check_type_ok)
  inv.register(M.name, "Inv", check_inv)
  return true
end

--- `reset()`：钉规范初态（全 active / 全 white / 计数与在途全 0 /
--- token(0,0,black)；契约 §语义子集声明裁剪域）。
function M.reset()
  ids_by_node = nil
  local ids = all_ids()
  for i = 1, #ids do
    entity.destroy(ids[i])
  end
  ids_by_node = {}
  for i = 0, N - 1 do
    ids_by_node[i] = entity.spawn(SCHEMA, {
      id = i, active = true, color = "white", counter = 0, pending = 0,
    })
  end
  token_set("pos", 0)
  token_set("q", 0)
  token_set("color", "black")
end

--- `step(action)`：源 Next 析取的六个显式动作。守卫不满足报
--- `YGG-MOD-TLA-GUARD`（TLA 守卫假 = 动作不使能，轨道选了即轨道错误）。
function M.step(action)
  if action == nil or action == "stutter" then
    return
  end
  local act = tostring(action)
  local send_i, send_j = act:match("^send:(%d+):(%d+)$")
  local recv_i = act:match("^recv:(%d+)$")
  local deact_i = act:match("^deact:(%d+)$")
  if send_i ~= nil then
    local i, j = tonumber(send_i), tonumber(send_j)
    if not entity.get(id_of(i), "active") or i == j then
      error("YGG-MOD-TLA-GUARD: send 守卫（active[i] ∧ j≠i）不满足：" .. act, 0)
    end
    entity.set(id_of(i), "counter", entity.get(id_of(i), "counter") + 1)
    entity.set(id_of(j), "pending", entity.get(id_of(j), "pending") + 1)
  elseif recv_i ~= nil then
    local i = tonumber(recv_i)
    local id = id_of(i)
    if entity.get(id, "pending") <= 0 then
      error("YGG-MOD-TLA-GUARD: recv 守卫 pending[i]>0 不满足：" .. act, 0)
    end
    entity.set(id, "pending", entity.get(id, "pending") - 1)
    entity.set(id, "counter", entity.get(id, "counter") - 1)
    entity.set(id, "color", "black")
    entity.set(id, "active", true)
  elseif deact_i ~= nil then
    local i = tonumber(deact_i)
    local id = id_of(i)
    if not entity.get(id, "active") then
      error("YGG-MOD-TLA-GUARD: deact 守卫 active[i] 不满足：" .. act, 0)
    end
    entity.set(id, "active", false)
  elseif act == "probe" then
    -- InitiateProbe：pos=0 ∧ 前轮不结论（黑令牌 ∨ 黑 0 号 ∨ 计数和 > 0）。
    local inconclusive = token_get("color") == "black"
      or entity.get(id_of(0), "color") == "black"
      or entity.get(id_of(0), "counter") + token_get("q") > 0
    if token_get("pos") ~= 0 or not inconclusive then
      error("YGG-MOD-TLA-GUARD: probe 守卫（pos=0 ∧ 前轮不结论）不满足", 0)
    end
    token_set("pos", N - 1)
    token_set("q", 0)
    token_set("color", "white")
    entity.set(id_of(0), "color", "white")
  elseif act:match("^pass:(%d+)$") ~= nil then
    local i = tonumber(act:match("^pass:(%d+)$"))
    local id = id_of(i)
    if entity.get(id, "active") or token_get("pos") ~= i then
      error("YGG-MOD-TLA-GUARD: pass 守卫（¬active[i] ∧ pos=i）不满足：" .. act, 0)
    end
    token_set("pos", token_get("pos") - 1)
    token_set("q", token_get("q") + entity.get(id, "counter"))
    token_set("color", entity.get(id, "color") == "black" and "black" or token_get("color"))
    entity.set(id, "color", "white")
  else
    error("YGG-MOD-TLA-ACTION: ewd998 未知动作：" .. act, 0)
  end
end

--- 启用动作枚举（升序；stutter 恒在）。
function M.auto_actions()
  local out = {}
  for i = 0, N - 1 do
    local id = id_of(i)
    if entity.get(id, "active") then
      for j = 0, N - 1 do
        if j ~= i then
          out[#out + 1] = "send:" .. i .. ":" .. j
        end
      end
      out[#out + 1] = "deact:" .. i
    end
    if entity.get(id, "pending") > 0 then
      out[#out + 1] = "recv:" .. i
    end
  end
  if token_get("pos") == 0 then
    local inconclusive = token_get("color") == "black"
      or entity.get(id_of(0), "color") == "black"
      or entity.get(id_of(0), "counter") + token_get("q") > 0
    if inconclusive then
      out[#out + 1] = "probe"
    end
  else
    local i = token_get("pos")
    if i ~= 0 and not entity.get(id_of(i), "active") then
      out[#out + 1] = "pass:" .. i
    end
  end
  out[#out + 1] = "stutter"
  for k = 1, #out do
    local min = k
    for j = k + 1, #out do
      if out[j] < out[min] then
        min = j
      end
    end
    if min ~= k then
      out[k], out[min] = out[min], out[k]
    end
  end
  return out
end

--- `snapshot()`：规范快照（nodes 按 id 升序 + token 三键）。
function M.snapshot()
  local nodes = {}
  for i = 0, N - 1 do
    local id = id_of(i)
    nodes[#nodes + 1] = {
      id = i,
      active = entity.get(id, "active") == true,
      color = entity.get(id, "color"),
      counter = entity.get(id, "counter"),
      pending = entity.get(id, "pending"),
    }
  end
  return {
    nodes = nodes,
    token = { pos = token_get("pos"), q = token_get("q"), color = token_get("color") },
  }
end

--- `terminationDetected` 读（终态 KAT 断言面）。
function M.termination_detected()
  return termination_detected()
end

return M
