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
-- 不用它们（连 `string.format` 都不用），只用算术、位运算与 `tostring`。

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
local GOLDEN = 0x9E3779B9

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
  local t = (s[1] << 9) & M32
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

return M
