-- sound：用例段自 pkg-assetgen.lua 原样搬移（S-25 批十三测试件连拆）。
return function(C)
  local S, run, eq, is_true, expect_error, fresh, pkg_sources, source_text, strip, samples_of, OSC, BUNDLE_ENTRIES =
    C.S, C.run, C.eq, C.is_true, C.expect_error, C.fresh, C.pkg_sources, C.source_text, C.strip, C.samples_of, C.OSC, C.BUNDLE_ENTRIES
run("pkg.assetgen.sound-same-seed-same-bytes", function()
  local assetgen = fresh()
  local first = assetgen.sound(OSC, "sound-seed")
  local second = assetgen.sound(OSC, "sound-seed")
  eq(#first.bytes, #second.bytes, "同 seed 两次 WAV 长度")
  is_true(first.bytes == second.bytes, "同 seed 两次的字节必须逐字节相等")
  eq(first.frames, 400, "frames = ms*sr//1000")
  eq(#first.pcm, 400 * 2, "#pcm = frames*ch*2")
  eq(#first.bytes, 44 + #first.pcm, "WAV = 44 字节头 + pcm")
  -- 纯振荡器不消耗随机数（参数完全决定波形），所以 seed 差异必须用**消耗流**的
  -- 算法来验：白噪。
  local noisy = { algo = "sound.noise", params = { ms = 50, sr = 8000, amp = 800 } }
  local third = assetgen.sound(noisy, "sound-seed-2")
  local fourth = assetgen.sound(noisy, "sound-seed")
  is_true(third.bytes ~= fourth.bytes, "异 seed 的噪声字节必须不同")
  eq(#third.bytes, #fourth.bytes, "异 seed 的噪声长度必须相同")
end)

run("pkg.assetgen.sound-peak-cap", function()
  local assetgen = fresh()
  local product = assetgen.sound({ algo = "sound.noise", params = { ms = 40, sr = 8000, amp = 700 } }, "peak")
  eq(product.peak, 700, "peak 是入参 amp 的千分比")
  local limit = (product.peak * 32767) // 1000
  local samples = samples_of(product)
  local low = 0
  for i = 1, #samples do
    local v = samples[i]
    local size = v < 0 and -v or v
    is_true(size <= limit, "第 " .. i .. " 个样本 " .. v .. " 超过 peak 上限 " .. limit)
    if size > low then
      low = size
    end
  end
  -- 归一必须真的用满幅：只断「不超上限」对一个全静音的实现恒真。
  is_true(low >= limit - 64, "最大样本 " .. low .. " 必须接近上限 " .. limit)
  local quiet = assetgen.sound({ algo = "sound.silence", params = { ms = 10, sr = 8000 } }, "peak")
  eq(quiet.peak, 800, "silence 的 peak 仍是 amp（全零时约定如此）")
  for i = 1, #samples_of(quiet) do
    eq(samples_of(quiet)[i], 0, "silence 必须逐样本为 0")
  end
end)

run("pkg.assetgen.sound-waveforms", function()
  local assetgen = fresh()
  local flat = { ms = 100, sr = 8000, amp = 1000, env = "none" }

  -- 方波：只出现 ±amp 两类值。
  local square = assetgen.sound({
    algo = "sound.osc",
    params = { wave = "square", freq = 100, ms = flat.ms, sr = flat.sr, amp = flat.amp, env = flat.env },
  }, "wave")
  local seen = {}
  local distinct = 0
  for _, v in ipairs(samples_of(square)) do
    if seen[v] == nil then
      seen[v] = true
      distinct = distinct + 1
    end
  end
  eq(distinct, 2, "方波只允许两类取值")

  -- 锯齿：相邻差为正，除了回绕点（每周期一次，容许 ±1）。
  local saw = samples_of(assetgen.sound({
    algo = "sound.osc",
    params = { wave = "saw", freq = 100, ms = flat.ms, sr = flat.sr, amp = flat.amp, env = flat.env },
  }, "wave"))
  local wraps = 0
  for i = 2, #saw do
    if saw[i] <= saw[i - 1] then
      wraps = wraps + 1
    end
  end
  local cycles = (100 * flat.ms) // 1000
  is_true(wraps >= cycles - 1 and wraps <= cycles + 1, "锯齿回绕次数应约为 " .. cycles .. "，实得 " .. wraps)

  -- 正弦：零交叉数约为 2 × 周期数。
  local sine = samples_of(assetgen.sound({
    algo = "sound.osc",
    params = { wave = "sine", freq = 100, ms = flat.ms, sr = flat.sr, amp = flat.amp, env = flat.env },
  }, "wave"))
  local crossings = 0
  for i = 2, #sine do
    if (sine[i - 1] < 0 and sine[i] >= 0) or (sine[i - 1] >= 0 and sine[i] < 0) then
      crossings = crossings + 1
    end
  end
  is_true(crossings >= 2 * cycles - 2 and crossings <= 2 * cycles + 2,
    "正弦零交叉应约为 " .. (2 * cycles) .. "，实得 " .. crossings)
  is_true(assetgen.sound(OSC, "wave").bytes ~= assetgen.sound({ algo = "sound.osc", params = { wave = "square", freq = 100, ms = 50, sr = 8000, amp = 800 } }, "wave").bytes,
    "不同波形的字节必须不同")
end)

-- ── 用例：WAV 头 / 包络 / 分层 / 音频硬帽 ──────────────────────────────────

local function u16le(text, at)
  return text:byte(at) | (text:byte(at + 1) << 8)
end

local function u32le(text, at)
  return text:byte(at)
    | (text:byte(at + 1) << 8)
    | (text:byte(at + 2) << 16)
    | (text:byte(at + 3) << 24)
end

run("pkg.assetgen.sound-wav-header", function()
  local assetgen = fresh()
  local product = assetgen.sound({ algo = "sound.osc", params = { wave = "square", freq = 440, ms = 25, sr = 44100, ch = 2, amp = 900 } }, "wav")
  local text = product.bytes
  eq(text:sub(1, 4), "RIFF", "RIFF 魔数")
  eq(text:sub(9, 12), "WAVE", "WAVE 魔数")
  eq(text:sub(13, 16), "fmt ", "fmt 块")
  eq(text:sub(37, 40), "data", "data 块")
  eq(u32le(text, 17), 16, "fmt 块长度")
  eq(u16le(text, 21), 1, "formatTag 必须是 PCM(1)")
  eq(u16le(text, 23), 2, "声道数")
  eq(u32le(text, 25), 44100, "采样率")
  eq(u32le(text, 29), 44100 * 2 * 2, "byteRate = sr*ch*2")
  eq(u16le(text, 33), 2 * 2, "blockAlign = ch*2")
  eq(u16le(text, 35), 16, "bitsPerSample")
  eq(u32le(text, 41), #product.pcm, "data 块长度 = #pcm")
  eq(u32le(text, 5), 36 + #product.pcm, "riffSize = 36 + dataSize")
  eq(#text, 44 + #product.pcm, "文件总长 = 44 + pcm")

  -- s16 装箱的三个锚点：`>>` 是逻辑右移，`//` 会算错负数的字节。
  local bytes = require("pkg.assetgen.bytes")
  eq(bytes.s16le(-1), "\255\255", "s16(-1) 低/高字节都是 0xFF")
  eq(bytes.s16le(-32768), "\0\128", "s16(-32768) = 0x00 0x80")
  eq(bytes.s16le(32767), "\255\127", "s16(32767) = 0xFF 0x7F")
end)

--- 一段样本的均方根。测试跑在普通 Lua 上，`math` 表可用，直接用 `math.sqrt` 取整。
local function segment_rms(samples, from, to)
  local sum = 0
  for i = from, to do
    local v = samples[i]
    sum = sum + v * v
  end
  local count = to - from + 1
  if count <= 0 then
    return 0
  end
  return math.floor(math.sqrt(sum // count))
end

run("pkg.assetgen.sound-envelope-decay", function()
  local assetgen = fresh()
  local product = assetgen.sound({
    algo = "sound.noise",
    params = { ms = 200, sr = 8000, amp = 900, attack = 10, env = "exp" },
  }, "env")
  local samples = samples_of(product)
  local n = #samples
  local quarter = n // 4
  local head = segment_rms(samples, 1, quarter)
  local tail = segment_rms(samples, n - quarter + 1, n)
  is_true(head > tail, "前 1/4 RMS（" .. head .. "）必须大于后 1/4（" .. tail .. "）")

  -- 最后一个 64 样本窗口的能量必须远低于最响的窗口（1/8 以上算没衰减）。
  local best = 0
  for start = 1, n - 63, 64 do
    local energy = segment_rms(samples, start, start + 63)
    if energy > best then
      best = energy
    end
  end
  local last = segment_rms(samples, n - 63, n)
  is_true(best > 0, "必须存在非零窗口")
  is_true(last * 8 < best, "末窗口能量 " .. last .. " 必须小于峰值窗口 " .. best .. " 的 1/8")

  -- `env = "none"` 是恒等增益：全程都不衰减。
  local flat = assetgen.sound({
    algo = "sound.osc",
    params = { wave = "sine", freq = 100, ms = 100, sr = 8000, amp = 1000, env = "none" },
  }, "env")
  local flat_samples = samples_of(flat)
  local flat_head = segment_rms(flat_samples, 1, 100)
  local flat_tail = segment_rms(flat_samples, #flat_samples - 99, #flat_samples)
  is_true(flat_head * 10 > flat_tail * 9 and flat_tail * 10 > flat_head * 9, "env=none 的 RMS 应基本持平")
end)

run("pkg.assetgen.sound-mix-sum", function()
  local assetgen = fresh()
  local function tone(freq)
    return assetgen.sound({
      algo = "sound.osc",
      params = { wave = "sine", freq = freq, ms = 50, sr = 8000, amp = 400, env = "none" },
    }, "mix")
  end
  local low = tone(100)
  local high = tone(200)
  local mixed = assetgen.sound({
    algo = "sound.mix",
    params = {
      layers = {
        { product = low, at = 0 },
        { product = high, at = 25 },
      },
    },
  }, "mix")
  eq(mixed.frames, 400 + 200, "mix 的帧数 = 最长层的末端")
  eq(#mixed.pcm, mixed.frames * 2, "#pcm = frames*ch*2")

  local a = samples_of(low)
  local b = samples_of(high)
  local out = samples_of(mixed)
  for i = 1, 200 do
    eq(out[i], a[i], "偏移之前只应有第一层（第 " .. i .. " 个样本）")
  end
  for i = 201, 400 do
    eq(out[i], a[i] + b[i - 200], "重叠区必须是整数和（第 " .. i .. " 个样本）")
  end
  for i = 401, 600 do
    eq(out[i], b[i - 200], "第一层结束后只剩第二层（第 " .. i .. " 个样本）")
  end
  is_true(mixed.peak > 0, "mix 的 peak 必须非零")
end)

run("pkg.assetgen.sound-caps", function()
  local assetgen = fresh()
  expect_error("YGG-PKG-ASSETGEN-CAP", function()
    assetgen.sound({ algo = "sound.osc", params = { wave = "sine", freq = 100, ms = 9000, sr = 48000 } }, "caps")
  end, "帧数超帽")
  expect_error("YGG-PKG-ASSETGEN-PARAM", function()
    assetgen.sound({ algo = "sound.osc", params = { wave = "sine", freq = 100, ms = 50, sr = 44101 } }, "caps")
  end, "sr 不在白名单")
  expect_error("YGG-PKG-ASSETGEN-PARAM", function()
    assetgen.sound({ algo = "sound.osc", params = { wave = "sine", freq = 100, ms = 0, sr = 8000 } }, "caps")
  end, "ms=0")
  expect_error("YGG-PKG-ASSETGEN-PARAM", function()
    assetgen.sound({ algo = "sound.osc", params = { wave = "nope", freq = 100, ms = 50, sr = 8000 } }, "caps")
  end, "未知波形")
  expect_error("YGG-PKG-ASSETGEN-PARAM", function()
    assetgen.sound({ algo = "sound.mix", params = { layers = {} } }, "caps")
  end, "空 layers")
end)

-- ── 用例：bundle 顺序与键 / 流隔离 ──────────────────────────────────────────


end
