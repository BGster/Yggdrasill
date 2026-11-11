-- src/core/hooks.lua —— 轻量响应式（类 React）：useMemo / useEffect / flush_effects。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；无 install 依赖。
--
-- 纪律（写进 AGENTS.md，可判定）：
--   · effect 只做「状态推进 + 叙事」，绝不调 render() / ygg.choice —— 连锁反应必须在
--     render() 产出唯一一次 choice **之前**同步收敛（内核排空规则：已产生 choice 的 handler
--     里再 emit 会被静默丢弃，见 crates/ygg-core/src/transition.rs）。
--   · 确定性：依赖签名一律由 KV 状态派生（不依赖跨 dispatch 的 Lua 内存推断）；读档或
--     重启后签名变化即自动重算，同一份存档在任何进程里得到同一份派生结果。

local M = {}

local util = require("src.core.util")

local join =
  util.join

-- 连锁反应最多收敛轮数：超过即判死循环并拒绝（fail-closed，不静默降级）。
local MAX_PASSES = 4

local memos = {}   -- key -> { deps = { scalar, ... }, value = <任意> }
local effects = {} -- 有序数组：{ name, deps_fn, effect_fn }（注册序 = 执行序）
local marks = {}   -- 本轮转换内的「刚就绪」标记：req_index -> true（由业务 handler 打标）

local function same_deps(a, b)
  if a == nil or b == nil then
    return false
  end
  if #a ~= #b then
    return false
  end
  for i = 1, #a do
    if a[i] ~= b[i] then
      return false
    end
  end
  return true
end

local function signature(deps)
  local parts = {}
  for i = 1, #deps do
    parts[i] = tostring(deps[i])
  end
  return join(parts, "|")
end

-- 记忆化派生值：`deps_fn()` 返回标量数组，浅比较相等即复用上次结果。
-- key 必须稳定（同一 key = 同一个记忆槽）；回调都在调用线程内同步执行，不跨 dispatch 缓存。
local function useMemo(key, compute, deps_fn)
  local deps = deps_fn()
  local slot = memos[key]
  if slot ~= nil and same_deps(slot.deps, deps) then
    return slot.value
  end
  local value = compute()
  memos[key] = { deps = deps, value = value }
  return value
end

-- 状态一变就整体失效（flush_effects 在任一 effect 产生变更后自动调用）。
function M.invalidate_memos()
  memos = {}
end

-- 声明式连锁反应：装配期（handler install / mod.lua）注册一次，运行期只遍历。
local function useEffect(name, deps_fn, effect_fn)
  for i = 1, #effects do
    if effects[i].name == name then
      effects[i].deps_fn = deps_fn
      effects[i].effect_fn = effect_fn
      return
    end
  end
  effects[#effects + 1] = { name = name, deps_fn = deps_fn, effect_fn = effect_fn }
end

-- 「刚就绪」标记：业务 handler 完成状态推进后打标（proposal.accept(analysis) /
-- gate.approve(plan)）。只有带标记的需求才会被自动推进——requirement.add 的 draft 不会。
function M.mark_auto_dispatch(index)
  marks[index] = true
end

function M.has_mark(index)
  return marks[index] == true
end

function M.clear_marks()
  marks = {}
end

-- 连锁反应收敛：最多 MAX_PASSES 轮；某一轮全体 effect 都没有产生变更即收敛。
-- 返回 { passes, changed, ran } 供测试与叙事定位；不收敛直接报错（拒绝收口，不静默跳过）。
function M.flush_effects()
  local ran = {}
  for pass = 1, MAX_PASSES do
    local changed = false
    local seen = {}
    for i = 1, #effects do
      local effect = effects[i]
      local deps = effect.deps_fn()
      -- deps_fn 返回 nil / false = 本 effect 此刻不适用（不是「依赖为空」）。
      if deps ~= nil and deps ~= false then
        local sign = signature(deps)
        -- 本轮转换内已用同一签名跑过就不再重跑（同签名 = 同一处境的重复求值）。
        if seen[effect.name] ~= sign then
          seen[effect.name] = sign
          if effect.effect_fn() == true then
            changed = true
            ran[#ran + 1] = effect.name
            M.invalidate_memos()
          end
        end
      end
    end
    if not changed then
      M.clear_marks()
      return { passes = pass, changed = #ran > 0, ran = ran }
    end
  end
  M.clear_marks()
  error("hooks.flush_effects: effects did not converge within " .. tostring(MAX_PASSES) .. " passes")
end

M.MAX_PASSES = MAX_PASSES
M.useEffect = useEffect
M.useMemo = useMemo

return M
