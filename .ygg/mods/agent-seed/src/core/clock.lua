-- src/core/clock.lua —— 派发序号与纪元边沿（r4）：**整份状态都在 KV 里**，没有 Lua 内存。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）。
-- 本模块写的 KV 前缀（都不在 config.set 用户面：无分支 → 直写被 bad_config_key 拒）：
--   settings.clock_seq                  —— 全局派发计数（本模块是唯一自增者）
--   settings.epoch.<id>.value/.seq      —— 纪元边沿：值变化时封存「变的那一刻的 seq」
--   settings.sign_seq.<doc>             —— 签名时刻快照（r4 的权威序号）
--
-- **刻意不写 r3 预留的 `settings.event_seq`**：readsig 的签名值形状是 `ver|scene|seq`，
-- 该字段在那里恒为 0；改它会当场把 4 条既有用例的 `...|0` 断言打红，并让「读签」两环
-- 抢同一个键。权威序号只存 `settings.sign_seq.<doc>`（单写者 = clock.mark_sign），
-- 消费侧（gates.read_problem）先读值段、为 0 时回落读快照——两处都不缺信息。
--
-- 为什么不用宿主时钟：`kvRevision` 与事件 seq 都没有 Lua 读取面，`ygg.ticks()` 的生产
-- 推进路径不存在（r3 已实证，见 src/domain/readsig.lua 头注）。所以「序号」只能自建：
-- 一个单调计数器 + 一处封存点，全部落 KV ⇒ 读档 / 重放 / 换代都从同一份状态复算
-- （§9.1「派生依赖必须是 KV 标量」的同一纪律）。
--
-- 唯一自增点 = `src/view/render.lua` 的重入守卫块（每次 Mod 派发恰好一次）。
-- 本模块**零业务依赖**（只 require util）：gates 要 require 它，而 render 已经 require
-- gates —— 任何反向依赖都成环（render → gates → flow → render），叶子位是本模块的硬约束。

local M = {}

local util = require("src.core.util")

local trim = util.trim

local SEQ = "settings.clock_seq"
local EPOCH = "settings.epoch."
local SIGN = "settings.sign_seq."

-- 配置值可能来自 JSON（number）或 KV 覆盖（number / 字符串），统一收敛成非负整数。
local function as_int(value)
  local number = tonumber(value)
  if number == nil then
    return 0
  end
  number = math.floor(number)
  if number < 0 then
    return 0
  end
  return number
end

-- 当前派发序号（本派发**结束时**的值；handler 体内读到的是上一次派发的收尾值）。
function M.now()
  return as_int(ygg.kv.get(SEQ))
end

-- 派发计数 +1（唯一写口；返回自增后的值）。
function M.bump()
  local next_seq = M.now() + 1
  ygg.kv.set(SEQ, next_seq)
  return next_seq
end

-- 纪元封存：`value` 与上次观察值不同才写（值未变 = 同一纪元 = 不产生新 seq）。
-- 返回该纪元**当前**的进入 seq（0 = 从未封存 = 未知，调用方须降级而不是误判）。
function M.seal(id, value)
  local value_key = EPOCH .. id .. ".value"
  local seq_key = EPOCH .. id .. ".seq"
  local clean = trim(tostring(value))
  if ygg.kv.get(value_key) == clean then
    return as_int(ygg.kv.get(seq_key))
  end
  ygg.kv.set(value_key, clean)
  -- now() 在此刻是「本派发的序号」：边沿发生的派发本身即进入点（不是下一次派发）。
  local seq = M.now()
  if seq < 1 then
    seq = 1
  end
  ygg.kv.set(seq_key, seq)
  return seq
end

-- 纪元进入 seq（0 = 未知；消费者必须把 0 当「不判」而不是「刚进入」）。
function M.epoch_seq(id)
  return as_int(ygg.kv.get(EPOCH .. id .. ".seq"))
end

-- 纪元当前值（可读性 / 叙事用；判定不依赖它）。
function M.epoch_value(id)
  local value = ygg.kv.get(EPOCH .. id .. ".value")
  if type(value) == "string" then
    return value
  end
  return ""
end

-- 签名时刻快照（r3 的值段 `ver|scene|seq` 也带 seq；两者互为印证，任一处缺值即降级）。
function M.mark_sign(doc)
  local seq = M.now()
  ygg.kv.set(SIGN .. doc, seq)
  return seq
end

function M.sign_seq(doc)
  return as_int(ygg.kv.get(SIGN .. doc))
end

-- 与 readsig 的清除同步（签名作废时快照一并作废，避免「值空了、序号还在」的半状态）。
function M.clear_sign(doc)
  local key = SIGN .. doc
  if ygg.kv.get(key) ~= nil then
    ygg.kv.set(key, 0)
  end
end

M.DEFAULT_UNKNOWN = 0

return M
