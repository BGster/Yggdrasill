-- `src/twophase.lua` —— TwoPhase 握手（交替同步 + 抽象参数实例化）。
--
-- 契约：openwiki/contracts/mod/tla-models.md §映射表。源：tlaplus/Examples
-- `specifications/TwoPhase/{TwoPhase,Alternate,MCTwoPhase}.tla`（TLC 实测
-- MCTwoPhase 4/5/4，引自源仓 manifest.json）。
--
-- 抽象参数实例化（MCTwoPhase）：`XInit(v) == v = 0`、`XAct(i, xInit, xNext)
-- == xNext = xInit`——x 是被抽象的内部状态，实例化后 x 恒等（写不变）。
-- 本移植取 MCTwoPhase 实例（x≡0），Alternate 的抽象变量 vBar = (p+c)%2
-- 作为 KAT 附带断言（refinement 映射的数值面）。
--
-- 映射：`p, c, x` 全局小态 → KV 标量 `tla.tp.p / tla.tp.c / tla.tp.x`；
-- `Init`（p=0, c=0, XInit(x)）→ 钉 (0,0,0)；`ProducerStep`（p=c 守卫）/
-- `ConsumerStep`（p≠c 守卫）→ `step("producer"/"consumer")`，守卫不满足报
-- `YGG-MOD-TLA-GUARD`（fail-closed；TLA 里守卫假 = 动作不使能，轨道输入
-- 选了不使能的动作即轨道错误）；`%2` 域 {0,1} 无负输入，与 Lua % 同值
-- （KAT 钉死）；`Inv`（p∈{0,1} ∧ c∈{0,1}）→ inv 注册表。

local kv = nil
local inv = require("src.inv")

local M = { VERSION = "0.1.0", name = "twophase" }

--- 沙箱方言：模块顶层纯；ygg 经 host() 单点取。
local function host()
  if kv == nil then
    kv = ygg.kv
  end
  return kv
end

local function get(key)
  return host().get("tla.tp." .. key)
end

local function set(key, value)
  host().set("tla.tp." .. key, value)
end

-- ── 不变量（源 Inv 逐字对齐）────────────────────────────────────────────────

--- `Inv == (p \in {0,1}) /\ (c \in {0,1})`。
local function check_inv()
  local violations = {}
  local p = get("p")
  local c = get("c")
  if p ~= 0 and p ~= 1 then
    violations[#violations + 1] = "p ∉ {0,1}：" .. tostring(p)
  end
  if c ~= 0 and c ~= 1 then
    violations[#violations + 1] = "c ∉ {0,1}：" .. tostring(c)
  end
  return violations
end

-- ── 装配 ─────────────────────────────────────────────────────────────────────

function M.install()
  inv.register(M.name, "Inv", check_inv)
  return true
end

--- `reset()`：钉规范初态（p=0, c=0, XInit(x) ⇒ x=0）。
function M.reset()
  set("p", 0)
  set("c", 0)
  set("x", 0)
end

--- `step(action)`：`"producer"`（p=c ∧ x'=x ∧ p'=(p+1)%2）/ `"consumer"`
--- （p≠c ∧ x'=x ∧ c'=(c+1)%2）/ `"stutter"`。
function M.step(action)
  if action == nil or action == "stutter" then
    return
  end
  local p = get("p")
  local c = get("c")
  if action == "producer" then
    if p ~= c then
      error("YGG-MOD-TLA-GUARD: producer 守卫 p=c 不满足（p=" .. p .. " c=" .. c .. "）", 0)
    end
    set("p", (p + 1) % 2)
  elseif action == "consumer" then
    if p == c then
      error("YGG-MOD-TLA-GUARD: consumer 守卫 p≠c 不满足（p=" .. p .. " c=" .. c .. "）", 0)
    end
    set("c", (c + 1) % 2)
  else
    error("YGG-MOD-TLA-ACTION: twophase 未知动作（producer|consumer|stutter）：" .. tostring(action), 0)
  end
  set("x", get("x")) -- XAct：x 恒等（MCTwoPhase 实例化的写不变）
end

--- 启用动作（状态决定：p=c 时 producer、p≠c 时 consumer；stutter 恒在）。
function M.auto_actions()
  if get("p") == get("c") then
    return { "producer", "stutter" }
  end
  return { "consumer", "stutter" }
end

--- `snapshot()`：规范快照（p/c/x 三标量）。
function M.snapshot()
  return { c = get("c"), p = get("p"), x = get("x") }
end

--- Alternate 抽象投影 `vBar == (p + c) % 2`（refinement 映射数值面，KAT 用）。
function M.vbar()
  return (get("p") + get("c")) % 2
end

return M
