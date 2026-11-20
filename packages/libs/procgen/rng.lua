-- `packages/libs/procgen/rng.lua` —— 命名流派生 + xoshiro128** 取数原语。
--
-- 契约：`docs/design/packages/procgen.md`（I1 确定性 / I2 流隔离 / I3 无外部熵 /
-- I7 顶层纯度）。本文件是包的**最底层叶子**：其它模块都 require 它，它谁也不
-- require——所以错误码表与 `fail` 住在这里（为一个常数表多开第七个模块，不如
-- 让它跟着唯一被所有人依赖的叶子走）。
--
-- # 三条语言约束（不满足就静默改产物，见契约 §8）
--
-- 1. 64 位常量只写十六进制字面量：十进制 `14695981039346656037` 超 i64，Lua 会
--    把它解析成浮点，随后 `~` / `*` 报「integer representation」。
-- 2. 32×32 直接相乘会溢出到 64 位，必须先拆半（`mul32`）；`mul32` 与直接相乘
--    在低 32 位一致，这是拆半正确的判据。
-- 3. `>>` 是**逻辑**右移（`(-1) >> 1` = 2^63-1），复位与旋转都靠它。
--
-- # 沙箱面
--
-- 宿主沙箱的 globals 是白名单（`crates/ygg-sandbox/src/sandbox.rs` 的
-- `BASE_WHITELIST`）：`math` / `string` / `table` 三张表**都不可达**。所以本包
-- 不用它们（连 `string` 表的 `format` 都不用），只用算术、位运算与 `tostring`。

local M = {}

-- ── 错误码面（包内唯一一份；`pkg.procgen` 的 `ERRORS` 从这里复制）──────────────

M.E = {
  SEED = "YGG-PKG-PROCGEN-SEED",
  RANGE = "YGG-PKG-PROCGEN-RANGE",
  PARAM = "YGG-PKG-PROCGEN-PARAM",
  ALGO = "YGG-PKG-PROCGEN-ALGO",
  DEF = "YGG-PKG-PROCGEN-DEF",
  DEPS = "YGG-PKG-PROCGEN-DEPS",
}

--- 报错：`<码>: <细节>`（level 0 = 不夹带调用点，用例按码做子串断言）。
function M.fail(code, detail)
  error(code .. ": " .. detail, 0)
end

-- ── 常量（K5：64 位一律十六进制）──────────────────────────────────────────────

local FNV_OFFSET = 0xCBF29CE484222325
local FNV_PRIME = 0x100000001B3
local MIX_A = 0xBF58476D1CE4E5B9
local MIX_B = 0x94D049BB133111EB
local M32 = 0xFFFFFFFF
local SPAN32 = 0x100000000
local GOLDEN = 0x9E3779B9
local I64_MAX = 0x7FFFFFFFFFFFFFFF
local FLOAT_SCALE = 1 / 16777216 -- 2^-24：`float` 的 24 位粒度

-- ── 32 位原语 ────────────────────────────────────────────────────────────────

--- 32 位无符号乘法（先拆高低半，再各自回绕；直接相乘会溢出到 2^64）。
local function mul32(a, b)
  return (((a & 0xFFFF) * b) + ((((a >> 16) * b) & 0xFFFF) << 16)) & M32
end

--- 32 位循环左移（`k` 取 1..31）。
local function rotl32(x, k)
  return ((x << k) | (x >> (32 - k))) & M32
end

--- FNV-1a 64：`h0` 与质数都是十六进制字面量（K5），乘法按 2^64 回绕。
function M.fnv64(text)
  local h = FNV_OFFSET
  for i = 1, #text do
    h = h ~ text:byte(i)
    h = h * FNV_PRIME
  end
  return h
end

--- splitmix64 终结器：把一个 64 位值打散成雪崩过的 64 位值。
function M.mix64(z)
  z = z ~ (z >> 30)
  z = z * MIX_A
  z = z ~ (z >> 27)
  z = z * MIX_B
  z = z ~ (z >> 31)
  return z
end

-- ── 种子派生（宿主种子 → 命名流 → 四字状态）──────────────────────────────────

--- 流名：缺省 `"main"`；显式给就必须是非空字符串。
function M.stream_name(stream)
  if stream == nil then
    return "main"
  end
  if type(stream) ~= "string" or stream == "" then
    M.fail(M.E.PARAM, "stream 必须是非空字符串，得到 " .. type(stream))
  end
  return stream
end

--- 种子 → 文本。整数 `"123"`、浮点走 Lua 的默认数字文本口径。
---
--- 不实现 `%.17g`：沙箱里 `string` 表不可达，而 `tostring` 是白名单面。代价是
--- 浮点 seed 只有默认精度可辨——要精确复现请用整数或字符串 seed（契约 §2 已写）。
local function seed_text(seed)
  local kind = type(seed)
  if kind == "string" then
    if seed == "" then
      M.fail(M.E.SEED, "seed 不得为空串")
    end
    return seed
  end
  if kind == "number" then
    return tostring(seed)
  end
  M.fail(M.E.SEED, "seed 必须是 string 或 number，得到 " .. kind)
end

--- `(seed, stream)` → 四字状态：`w[i] = mix64(fnv64(seed .. "/" .. stream .. "/" .. i))`。
---
--- 每条流的状态**完全**由 `(seed, stream)` 决定，不共享计数器——插入新流不改既有流
--- 的序列（I2）。四字全 0 是 xoshiro 的退化态，用黄金比例常数顶掉。
local function derived_state(seed, stream)
  local text = seed_text(seed)
  local state = {}
  for i = 1, 4 do
    state[i] = M.mix64(M.fnv64(text .. "/" .. stream .. "/" .. (i - 1))) & M32
  end
  if state[1] == 0 and state[2] == 0 and state[3] == 0 and state[4] == 0 then
    state[1] = GOLDEN
  end
  return state
end

-- ── 句柄 ─────────────────────────────────────────────────────────────────────

local RNG = {}
RNG.__index = RNG

--- 取句柄的状态表（句柄形状校验的唯一处）。
local function state_of(rng)
  if type(rng) ~= "table" or type(rawget(rng, "s")) ~= "table" then
    M.fail(M.E.PARAM, "rng 句柄必须是 procgen.rng(...) 的返回值")
  end
  return rawget(rng, "s")
end

--- `procgen.rng(seed, stream?, state?)` → 新句柄。
---
--- 给了 `state` 就用它（四整数快照，`procgen.state` 的返回值），`seed` / `stream`
--- 一律忽略——存档续生成走这条。状态是**副本**，不是别的句柄的别名。
function M.new(seed, stream, state)
  local words
  if state ~= nil then
    if type(state) ~= "table" then
      M.fail(M.E.PARAM, "state 必须是四整数数组，得到 " .. type(state))
    end
    words = {}
    for i = 1, 4 do
      local v = state[i]
      if type(v) ~= "number" or v ~= (v // 1) or v < 0 or v > M32 then
        M.fail(M.E.PARAM, "state[" .. i .. "] 必须是 [0, 2^32) 的整数")
      end
      words[i] = v | 0
    end
    if words[1] == 0 and words[2] == 0 and words[3] == 0 and words[4] == 0 then
      M.fail(M.E.PARAM, "state 四字全 0（xoshiro 要求非零态）")
    end
  else
    words = derived_state(seed, M.stream_name(stream))
  end
  return setmetatable({ s = words }, RNG)
end

--- `rng.next32(rng)` → 一个 32 位无符号整数（xoshiro128** 的一步）。
---
--- 原语面：`int` 的拒绝采样与 KAT 都要它，所以它**必须**是导出面而不是内部实现
--- （KAT 是「同 seed 同产物」的证据锚，不能靠拼出来的等价式间接验）。
function M.next32(rng)
  local s = state_of(rng)
  local result = mul32(rotl32(mul32(s[2], 5), 7), 9)
  local t = (s[2] << 9) & M32
  s[3] = s[3] ~ s[1]
  s[4] = s[4] ~ s[2]
  s[2] = s[2] ~ s[3]
  s[1] = s[1] ~ s[4]
  s[3] = s[3] ~ t
  s[4] = rotl32(s[4], 11)
  return result
end

-- 句柄方法与模块函数**同一个函数**：`rng.int(rng, 1, 6)` 与 `rng:int(1, 6)` 都对。
RNG.next32 = function(self)
  return M.next32(self)
end

-- ── 取数面（闭区间整数 / [0,1) 浮点 / 抽取 / 加权抽取 / 置换 / 快照）─────────────────

--- 整数化：只收整值数字（整值浮点也收，值不变），带小数或越 i64 即 `-PARAM`。
local function to_int(value, label)
  if type(value) ~= "number" or value ~= (value // 1) then
    M.fail(M.E.PARAM, label .. " 必须是整数，得到 " .. tostring(value))
  end
  if value > I64_MAX or value < -I64_MAX then
    M.fail(M.E.PARAM, label .. " 超出 64 位整数域")
  end
  return value | 0
end

--- `rng.int(rng, lo, hi)` → `[lo, hi]` 闭区间整数。
---
--- **拒绝采样**，不是取模：`limit = 2^32 - (2^32 %% span)`，抽到 `r < limit` 为止。
--- 取模会让小的余数面偏多，而「同 seed 同产物」的验收里偏差是**静默**的（序列
--- 依然可复现，只是分布偏了）。`span == 1` 直接返回 `lo`，**不取数**——单面区间
--- 消耗一个随机数是纯浪费，且会让「插一个常量参数」改变后续序列。
function M.int(rng, lo, hi)
  local first = to_int(lo, "lo")
  local last = to_int(hi, "hi")
  if first > last then
    M.fail(M.E.RANGE, "int 的 lo > hi（" .. first .. " > " .. last .. "）")
  end
  local span = last - first + 1
  if span <= 0 then
    M.fail(M.E.RANGE, "int 的区间宽溢出 64 位")
  end
  if span > SPAN32 then
    M.fail(M.E.RANGE, "int 的区间宽 " .. span .. " 超过 2^32")
  end
  if span == 1 then
    return first
  end
  local limit = SPAN32 - (SPAN32 % span)
  while true do
    local draw = M.next32(rng)
    if draw < limit then
      return first + (draw % span)
    end
  end
end

--- `rng.float(rng)` → `[0, 1)`，24 位粒度（`(next32 >> 8) * 2^-24`，f64 精确可表）。
function M.float(rng)
  return (M.next32(rng) >> 8) * FLOAT_SCALE
end

--- `rng.pick(rng, list)` → 列表中的一个元素（空表或非数组 = `-PARAM`）。
function M.pick(rng, list)
  if type(list) ~= "table" or #list < 1 then
    M.fail(M.E.PARAM, "pick 需要非空数组")
  end
  return list[M.int(rng, 1, #list)]
end

--- `rng.weighted(rng, weights)` → `1..#weights` 中的一个**下标**（按权重抽，非均匀）。
---
--- **权重是非负整数的相对份数**：不做归一化（归一化必引浮点除法，跨机舍入破
--- I1），不收小数（要小数请调用方放大成整数）。零权项合法但不可达。
---
--- `Σw` 用整数累加（精确，无浮点结合律问题），再走 `int(1, Σw)` 的拒绝采样。
--- 因此**等权池上 `weighted` 与 `int` 逐值相等**：`pick` 是本函数的特例
--- （`list[rng:weighted(ones(#list))]`），不是另一个取数面。
---
--- 两轮遍历都用下标 `1..n`，不碰 `pairs` / `next`：池序 = 数组声明序，键序遍历
--- 会让「同 seed 同产物」取决于虚拟机实现。`Σw == 1` 时 `int` 短路、**不取数**。
function M.weighted(rng, weights)
  if type(weights) ~= "table" or #weights < 1 then
    M.fail(M.E.PARAM, "weighted 需要非空数组")
  end
  local n = #weights
  local total = 0
  for i = 1, n do
    -- 标签用**常量**而不用 `"weights[" .. i .. "]"`：后者每次调用要拼 20 个字符串，
    -- 实测把 1k 次抽样从 1.11 ms 推到 2.19 ms（2×）。失败路径的下标在下面单独拼。
    local weight = to_int(weights[i], "weights[i]")
    if weight < 0 then
      M.fail(M.E.PARAM, "weights[" .. i .. "] 必须 ≥ 0（权重是相对份数）")
    end
    total = total + weight
    if total > SPAN32 then
      -- 逐元素查：total 从未越过 2^32 再加大于 i64 的项，整数加法不会回绕
      M.fail(M.E.RANGE, "权重总和超过 2^32（int 的抽取面宽上限）")
    end
  end
  if total < 1 then
    M.fail(M.E.PARAM, "权重总和必须 > 0（全零权重没有可抽的项）")
  end
  local r = M.int(rng, 1, total)
  local acc = 0
  for i = 1, n do
    acc = acc + (weights[i] | 0) -- 用整数面累加（入参可能是整值浮点）
    if acc >= r then
      return i
    end
  end
  -- 不可达：total 是精确和且 r ∈ [1, total]，最后一次累加必 ≥ r
  M.fail(M.E.PARAM, "weighted 的权重总和与累计不一致")
end

--- `rng.shuffle(rng, list)` → **新表**（Fisher–Yates，从后往前换）。
---
--- 不原地改：调用方的表常常是 defs 里的参数表，原地洗牌会让「同 seed 同产物」
--- 取决于谁先跑（第一次调用改了共享状态）。
function M.shuffle(rng, list)
  if type(list) ~= "table" then
    M.fail(M.E.PARAM, "shuffle 需要数组，得到 " .. type(list))
  end
  local out = {}
  for i = 1, #list do
    out[i] = list[i]
  end
  for i = #out, 2, -1 do
    local j = M.int(rng, 1, i)
    out[i], out[j] = out[j], out[i]
  end
  return out
end

--- `rng.state(rng)` → 四整数**副本**（改它不影响句柄；存档续生成靠它）。
function M.state(rng)
  local s = state_of(rng)
  return { s[1], s[2], s[3], s[4] }
end

--- 取一个整数参数：`nil` → `default`；非整数或越 `[low, high]` 即 `-PARAM`。
---
--- 参数校验器与 `fail` 同住在叶子（同一个理由：十几行的公共工具不为它单开模块）。
--- 算法的 `params` 一律经它取——生成器里不接受任何未校验的外部数字。
function M.param_int(params, key, default, low, high)
  local value = params[key]
  if value == nil then
    value = default
  end
  if type(value) ~= "number" or value ~= (value // 1) then
    M.fail(M.E.PARAM, "params." .. key .. " 必须是整数，得到 " .. tostring(value))
  end
  if value < low or value > high then
    M.fail(M.E.PARAM, "params." .. key .. " 必须落在 [" .. low .. ", " .. high .. "]")
  end
  return value | 0
end

RNG.int = function(self, lo, hi)
  return M.int(self, lo, hi)
end

RNG.float = function(self)
  return M.float(self)
end

RNG.pick = function(self, list)
  return M.pick(self, list)
end

RNG.weighted = function(self, weights)
  return M.weighted(self, weights)
end

RNG.shuffle = function(self, list)
  return M.shuffle(self, list)
end

RNG.state = function(self)
  return M.state(self)
end

return M
