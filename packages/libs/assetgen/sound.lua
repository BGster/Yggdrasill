-- `packages/libs/assetgen/sound.lua` —— 音频产物：s16 采样数组 + 包络 + 归一。
--
-- 契约：`docs/design/packages/assetgen.md` §2（c）/ §3 I8。
--
-- # 三层：原始样本 → 包络 → 归一
--
-- 每个算法只产 `raw[i]`（有符号整数，未归一）。包络在**生成时**逐样本乘进去
-- （`gain` 千分比），归一在最后**统一**做一次：找 `max|raw|` 再线性缩放到
-- `amp*32767//1000`。这样「峰值」是产物里可断言的字段（`peak`），而不是算法各自
-- 记住的一个数——`sound-peak-cap` 就是靠它逐样本复核的。
--
-- # 为什么样本是整数数组而不是字节串
--
-- 归一化要回看**所有**样本的最大值，边生成边装箱就没有回头路。数组留在 Lua 侧，
-- 装箱（`s16le` 交织）在归一之后一次性做完，按 256 样本分块走 `join.concat`。

local bytes = require("pkg.assetgen.bytes")
local join = require("pkg.assetgen.join")
local spec = require("pkg.assetgen.spec")
local wave = require("pkg.assetgen.wave")

local M = {}

local fail = spec.fail
local E = spec.E

--- 采样率白名单（契约 §2(c)：非白名单值一律 `-PARAM`，不做重采样）。
local SAMPLE_RATES = {
  [8000] = true,
  [11025] = true,
  [16000] = true,
  [22050] = true,
  [32000] = true,
  [44100] = true,
  [48000] = true,
}

local MAX_FRAMES = 400000

--- 包络种类：`none` / `lin` / `exp`（一阶反比，整数、单调、端点精确）。
local ENVELOPES = { exp = true, lin = true, none = true }

--- 公共参数：采样率 / 时长 / 声道 / 振幅 / 包络。
local function common(params)
  local sr = spec.range(params, "sr", 22050, 1, 192000)
  if SAMPLE_RATES[sr] == nil then
    fail(E.PARAM, "params.sr 必须是 8000/11025/16000/22050/32000/44100/48000 之一，得到 " .. sr)
  end
  local ms = spec.range(params, "ms", 200, 1, 60000)
  local ch = spec.range(params, "ch", 1, 1, 2)
  local amp = spec.range(params, "amp", 800, 0, 1000)
  local frames = (ms * sr) // 1000
  if frames < 1 then
    frames = 1
  end
  if frames > MAX_FRAMES then
    fail(E.CAP, "帧数 " .. frames .. " 超过 " .. MAX_FRAMES .. "（ms × sr 太大）")
  end
  local attack = (spec.range(params, "attack", 0, 0, 60000) * sr) // 1000
  if attack > frames then
    attack = frames
  end
  return {
    sr = sr,
    ch = ch,
    amp = amp,
    frames = frames,
    env = spec.one_of(params, "env", "exp", ENVELOPES),
    attack = attack,
  }
end

--- 第 `i`（0 基）个样本的包络增益（千分比）。
local function gain_at(ctx, i)
  if ctx.env == "none" then
    return 1000
  end
  if i < ctx.attack then
    return (i * 1000) // ctx.attack
  end
  local span = ctx.frames - ctx.attack
  if span <= 0 then
    return 1000
  end
  local t = i - ctx.attack
  if ctx.env == "lin" then
    return 1000 - (t * 1000) // span
  end
  return (1000 * (span - t)) // (span + 2 * t)
end

-- 前向声明：`finalize` 用它，实现在下方（交织与归一互相独立，顺序不必颠倒）。
local interleave

--- 归一 + 交织 + 装箱。`amp` 为 0 时产物是全静音（`peak = 0`）。
local function finalize(ctx, samples)
  local max = 0
  for i = 1, ctx.frames do
    local v = samples[i]
    local size = v < 0 and -v or v
    if size > max then
      max = size
    end
  end
  local target = (ctx.amp * 32767) // 1000
  if max > 0 and target > 0 then
    for i = 1, ctx.frames do
      local v = (samples[i] * target) // max
      if v > target then
        v = target
      elseif v < -target then
        v = -target
      end
      samples[i] = v
    end
  else
    for i = 1, ctx.frames do
      samples[i] = 0
    end
  end
  return {
    sr = ctx.sr,
    ch = ctx.ch,
    frames = ctx.frames,
    peak = ctx.amp,
    pcm = interleave(ctx, samples),
  }
end

--- s16 小端交织成字节串（每 256 个样本一块，最后 `join.concat`）。
---
--- **local 而不是 `M.*`**：`register` 把模块表的每个键都当算法装进算法表，一个
--- 辅助函数漏在 `M` 上就会变成 `algo_list("sound")` 里的第七个「算法」。
interleave = function(ctx, samples)
  local parts = {}
  local chunk = {}
  local at = 0
  for i = 1, #samples do
    local text = bytes.s16le(samples[i])
    for _ = 1, ctx.ch do
      at = at + 1
      chunk[at] = text
      if at == 256 then
        parts[#parts + 1] = join.concat(chunk)
        chunk = {}
        at = 0
      end
    end
  end
  if at > 0 then
    parts[#parts + 1] = join.concat(chunk)
  end
  return join.concat(parts)
end

-- ── 算法 ────────────────────────────────────────────────────────────────────

--- `sound.osc`：单振荡器（可扫频）。相位累加器是整数，`f` 随样本线性变化。
function M.osc(params, _)
  local ctx = common(params)
  local kind = spec.one_of(params, "wave", "sine", wave.KINDS)
  local freq = spec.range(params, "freq", 440, 1, 20000)
  local sweep = spec.range(params, "sweep", 0, -20000, 20000)
  local samples = {}
  local phase = 0
  for i = 0, ctx.frames - 1 do
    local f = freq + (sweep * i) // ctx.frames
    if f < 1 then
      f = 1
    end
    samples[i + 1] = (wave.value(kind, phase) * gain_at(ctx, i)) // 1000
    phase = (phase + (f * 65536) // ctx.sr) & 0xFFFF
  end
  return finalize(ctx, samples)
end

--- `sound.noise`：白噪 + 一阶低通（`lp` 千分比，1000 = 直通）+ 可选一阶高通。
---
--- 滤波系数是整数一阶递归：`y += (x - y) // k`。它不追求频响精确，追求**同样输入
--- 同样输出**——用浮点设计滤波器再整数化是另一个形状的分叉。
function M.noise(params, rng)
  local ctx = common(params)
  local lp = spec.range(params, "lp", 0, 0, 1000)
  local hp = spec.range(params, "hp", 0, 0, 1000)
  local k = 1 + ((1000 - lp) * 63) // 1000
  local kh = 1 + ((1000 - hp) * 63) // 1000
  local samples = {}
  local low = 0
  local high = 0
  for i = 0, ctx.frames - 1 do
    local raw = rng:int(-32768, 32767)
    low = low + (raw - low) // k
    high = high + (low - high) // kh
    local value = low
    if hp > 0 then
      value = low - high
    end
    samples[i + 1] = (value * gain_at(ctx, i)) // 1000
  end
  return finalize(ctx, samples)
end

--- `sound.silence`：全零（`peak = 0`）。存在的理由：`mix` 需要一条「静音底」。
function M.silence(params, _)
  local ctx = common(params)
  local samples = {}
  for i = 1, ctx.frames do
    samples[i] = 0
  end
  return finalize(ctx, samples)
end


--- `sound.partials`：模态叠加（若干正弦按 `mult` 千分比倍频、`amp` 千分比加权）。
---
--- 默认三个部分音（基频 + 八度 + 十二度）是「金属 / 木石」听感的骨架；参数化的是
--- **哪几个**部分音，而不是哪个游戏——所以它留在包里（契约 §1 的 I5）。
function M.partials(params, _)
  local ctx = common(params)
  local freq = spec.range(params, "freq", 220, 1, 20000)
  local list = spec.array(params, "partials", {
    { mult = 1000, amp = 1000 },
    { mult = 2000, amp = 500 },
    { mult = 3000, amp = 250 },
  })
  if #list == 0 then
    fail(E.PARAM, "params.partials 不能为空")
  end
  local steps = {}
  local phases = {}
  for j = 1, #list do
    local partial = list[j]
    local mult = spec.range(partial, "mult", 1000, 1, 64000)
    local amp = spec.range(partial, "amp", 1000, 0, 1000)
    steps[j] = ((freq * mult // 1000) * 65536) // ctx.sr
    phases[j] = 0
    list[j] = { step = steps[j], amp = amp }
  end
  local samples = {}
  for i = 0, ctx.frames - 1 do
    local sum = 0
    for j = 1, #list do
      sum = sum + (wave.value("sine", phases[j]) * list[j].amp) // 1000
      phases[j] = (phases[j] + list[j].step) & 0xFFFF
    end
    samples[i + 1] = (sum * gain_at(ctx, i)) // 1000
  end
  return finalize(ctx, samples)
end

--- `sound.steps`：阶梯噪声（每 `1/rate` 秒换一个电平），是脚步 / 碎石音的最小骨架。
function M.steps(params, rng)
  local ctx = common(params)
  local rate = spec.range(params, "rate", 20, 1, 4000)
  local hold = ctx.sr // rate
  if hold < 1 then
    hold = 1
  end
  local samples = {}
  local level = rng:int(-32768, 32767)
  for i = 0, ctx.frames - 1 do
    if i > 0 and (i % hold) == 0 then
      level = rng:int(-32768, 32767)
    end
    samples[i + 1] = (level * gain_at(ctx, i)) // 1000
  end
  return finalize(ctx, samples)
end

--- `sound.mix`：分层叠加（`at` 是毫秒偏移）。
---
--- **不做二次归一**：`sum` 的采样必须等于各层采样的整数和（`sound-mix-sum` 钉这条）。
--- 所以峰值是**导出量** `ceil(max·1000/32767)` 而不是目标值；和为 0 时 `peak = 0`。
function M.mix(params, _)
  local layers = spec.array(params, "layers", nil)
  if layers == nil or #layers == 0 then
    fail(E.PARAM, "sound.mix 需要非空的 params.layers")
  end
  local sr, ch = nil, nil
  local frames = 0
  local starts = {}
  for i = 1, #layers do
    local layer = layers[i]
    if type(layer) ~= "table" then
      fail(E.PARAM, "params.layers[" .. i .. "] 必须是表")
    end
    local product = layer.product
    if type(product) ~= "table" or type(product.pcm) ~= "string" then
      fail(E.PARAM, "params.layers[" .. i .. "].product 必须是 sound 产物")
    end
    if sr == nil then
      sr = product.sr
      ch = product.ch
    elseif product.sr ~= sr or product.ch ~= ch then
      fail(E.PARAM, "mix 的各层必须同 sr / ch（" .. sr .. "/" .. ch .. " vs " .. product.sr .. "/" .. product.ch .. "）")
    end
    local at = (spec.range(layer, "at", 0, 0, 60000) * sr) // 1000
    starts[i] = at
    if at + product.frames > frames then
      frames = at + product.frames
    end
  end
  if frames > MAX_FRAMES then
    fail(E.CAP, "mix 的总帧数 " .. frames .. " 超过 " .. MAX_FRAMES)
  end
  local samples = {}
  for i = 1, frames do
    samples[i] = 0
  end
  local max = 0
  for i = 1, #layers do
    local product = layers[i].product
    local pcm = product.pcm
    local base = starts[i]
    local stride = product.ch * 2
    for j = 0, product.frames - 1 do
      local lo = pcm:byte(j * stride + 1)
      local hi = pcm:byte(j * stride + 2)
      local v = lo | (hi << 8)
      if v >= 32768 then
        v = v - 65536
      end
      local index = base + j + 1
      local sum = samples[index] + v
      if sum > 32767 then
        sum = 32767
      elseif sum < -32768 then
        sum = -32768
      end
      samples[index] = sum
      local size = sum < 0 and -sum or sum
      if size > max then
        max = size
      end
    end
  end
  local peak = 0
  if max > 0 then
    peak = (max * 1000 + 32766) // 32767
  end
  return {
    sr = sr,
    ch = ch,
    frames = frames,
    peak = peak,
    pcm = interleave({ sr = sr, ch = ch, frames = frames }, samples),
  }
end

return M
