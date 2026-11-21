-- bundle-perf：用例段自 pkg-assetgen.lua 原样搬移（S-25 批十三测试件连拆）。
return function(C)
  local S, run, eq, is_true, expect_error, fresh, pkg_sources, source_text, strip, samples_of, OSC, BUNDLE_ENTRIES =
    C.S, C.run, C.eq, C.is_true, C.expect_error, C.fresh, C.pkg_sources, C.source_text, C.strip, C.samples_of, C.OSC, C.BUNDLE_ENTRIES
run("pkg.assetgen.bundle-order-and-keys", function()
  local assetgen = fresh()
  local bundle = assetgen.bundle(BUNDLE_ENTRIES, "bundle-seed")
  eq(#bundle.items, 3, "items 数量")
  for i = 1, #BUNDLE_ENTRIES do
    eq(bundle.items[i].key, BUNDLE_ENTRIES[i].key, "items 顺序必须等于入参顺序（第 " .. i .. " 项）")
    eq(bundle.items[i].kind, BUNDLE_ENTRIES[i].kind, "items kind（第 " .. i .. " 项）")
    eq(bundle.items[i].product.algo, BUNDLE_ENTRIES[i].algo, "items algo（第 " .. i .. " 项）")
  end
  eq(bundle.seed_text, "bundle-seed", "bundle.seed_text")
  eq(bundle.v, 1, "bundle.v 是单一常量")
  eq(bundle.canon, assetgen.canon(bundle), "bundle 的 canon 与 canon(bundle) 一致")
  is_true(bundle.items[1].product.stream == "assetgen/grass", "缺省流名 = assetgen/<key>")

  local function bundle_error(code, entries, label)
    expect_error(code, function()
      assetgen.bundle(entries, "bundle-seed")
    end, label)
  end
  bundle_error("YGG-PKG-ASSETGEN-PARAM", {
    BUNDLE_ENTRIES[1],
    { key = "grass", kind = "mesh", algo = "mesh.box", params = {} },
  }, "重复 key")
  bundle_error("YGG-PKG-ASSETGEN-PARAM", {
    { key = "", kind = "texture", algo = "texture.checker", params = { w = 4, h = 4 } },
  }, "空 key")
  bundle_error("YGG-PKG-ASSETGEN-KIND", {
    { key = "x", kind = "video", algo = "texture.checker", params = { w = 4, h = 4 } },
  }, "kind 不在三值")
  bundle_error("YGG-PKG-ASSETGEN-PARAM", {}, "空 entries")
end)

run("pkg.assetgen.stream-isolation", function()
  local assetgen = fresh()
  local texture_spec = { algo = "texture.fbm", params = { w = 16, h = 16, ch = 3, cell = 4, octaves = 2 } }
  local mesh_spec = { algo = "mesh.blob", params = { radius = 500, segs = 6, rings = 4, amp = 300 } }
  local sound_spec = { algo = "sound.noise", params = { ms = 30, sr = 8000 } }

  -- 先用后生成音频，再反过来：三条流各自的状态由 (seed, stream) 派生，顺序不得影响。
  local tex_a = assetgen.texture(texture_spec, "iso-seed")
  local mesh_a = assetgen.mesh(mesh_spec, "iso-seed")
  local snd_a = assetgen.sound(sound_spec, "iso-seed")
  local snd_b = assetgen.sound(sound_spec, "iso-seed")
  local mesh_b = assetgen.mesh(mesh_spec, "iso-seed")
  local tex_b = assetgen.texture(texture_spec, "iso-seed")

  is_true(tex_a.bytes == tex_b.bytes, "纹理字节不受生成顺序影响")
  is_true(mesh_a.bytes == mesh_b.bytes, "网格字节不受生成顺序影响")
  is_true(snd_a.bytes == snd_b.bytes, "音频字节不受生成顺序影响")

  -- 同一 (seed, stream) 显式共享时序列才共享；换类别的缺省流名互不相同。
  local streams = assetgen.streams("iso-seed")
  eq(#assetgen.algo_list("texture"), 7, "texture 算法数")
  eq(#assetgen.algo_list("mesh"), 6, "mesh 算法数")
  eq(#assetgen.algo_list("sound"), 6, "sound 算法数")
  local names = assetgen.algo_list("mesh")
  for i = 2, #names do
    is_true(names[i - 1] < names[i], "algo_list 必须字节序升序：" .. names[i - 1] .. " < " .. names[i])
  end
  eq(type(streams.texture), "table", "streams 返回三条句柄")
  eq(streams.texture:int(0, 0), 0, "句柄可用（单面区间不取数）")
  expect_error("YGG-PKG-ASSETGEN-KIND", function()
    assetgen.algo_list("video")
  end, "未知 kind")
end)

-- ── 用例：泄漏（X5）与性能（X6）────────────────────────────────────────────

run("pkg.assetgen.leak-100-rounds", function()
  local assetgen = fresh()
  local spec_small = { algo = "texture.fbm", params = { w = 16, h = 16, ch = 3, cell = 4, octaves = 2 } }
  local baseline = assetgen.texture(spec_small, "leak-seed")
  collectgarbage("collect")
  collectgarbage("collect")
  local before = collectgarbage("count")
  local canons_equal = true
  local bytes_equal = true
  local last = baseline
  for _ = 1, 100 do
    last = assetgen.texture(spec_small, "leak-seed")
    if last.canon ~= baseline.canon then
      canons_equal = false
    end
    if last.bytes ~= baseline.bytes then
      bytes_equal = false
    end
  end
  collectgarbage("collect")
  collectgarbage("collect")
  local after = collectgarbage("count")
  local growth = 0
  if before > 0 then
    growth = (after - before) * 100 / before
  end
  print("perf.assetgen.leak_100_equal=" .. tostring(canons_equal and bytes_equal and 1 or 0))
  print(("perf.assetgen.leak_gc_pct=%.3f"):format(growth))
  is_true(canons_equal, "100 轮 canon 必须全等")
  is_true(bytes_equal, "100 轮 bytes 必须全等")
  is_true(growth <= 1, "100 轮内存增长必须 ≤ 1%，实得 " .. ("%.3f"):format(growth) .. "%")
  -- 产物留在作用域里（`last` 带着最后一次的结果），所以上面的增长不是「东西被回收了」。
  is_true(#last.bytes == 16 * 16 * 3, "最后一次产物必须完整")
end)

--- 三次取中位（每项跑三次，取中间那个；与 procgen §5 同口径）。
local function median3(body)
  local a = body()
  local b = body()
  local c = body()
  if a > b then
    a, b = b, a
  end
  if b > c then
    b, c = c, b
  end
  if a > b then
    a, b = b, a
  end
  return b
end

local function ms_of(body)
  local started = os.clock()
  body()
  return (os.clock() - started) * 1000
end

run("pkg.assetgen.perf", function()
  local assetgen = fresh()
  local numbers = {}

  -- ① 包加载（清缓存后 require + install）
  numbers.load_ms = median3(function()
    for name in pairs(package.loaded) do
      if type(name) == "string" and (name == "pkg.assetgen" or name:find("^pkg%.assetgen%.")) then
        package.loaded[name] = nil
      end
    end
    local started = os.clock()
    local fresh_mod = require("pkg.assetgen")
    fresh_mod.install()
    return (os.clock() - started) * 1000
  end)

  -- ② 纹理 64×64 RGBA fbm 三倍频
  local tex_spec = { algo = "texture.fbm", params = { w = 64, h = 64, ch = 4, cell = 8, octaves = 3 } }
  numbers.tex64_ms = median3(function()
    return ms_of(function()
      local product = assetgen.texture(tex_spec, "perf-tex")
      numbers.tex_bytes = #product.bytes
    end)
  end)

  -- ③ 几何 torus 24×12（含 OBJ 文本）
  local torus_spec = { algo = "mesh.torus", params = { radius = 1000, tube = 300, segs = 24, rings = 12 } }
  numbers.mesh_torus_ms = median3(function()
    return ms_of(function()
      local product = assetgen.mesh(torus_spec, "perf-mesh")
      numbers.mesh_bytes = #product.bytes
    end)
  end)

  -- ④ 音频 1 s @22050 osc + 包络
  local sound_spec = { algo = "sound.osc", params = { wave = "sine", freq = 440, ms = 1000, sr = 22050, amp = 900, env = "exp", attack = 5 } }
  numbers.sound_1s_ms = median3(function()
    return ms_of(function()
      local product = assetgen.sound(sound_spec, "perf-sound")
      numbers.sound_bytes = #product.bytes
    end)
  end)

  -- ⑤ 16 件 bundle
  local entries = {}
  for i = 1, 16 do
    entries[i] = {
      key = "item" .. i,
      kind = "texture",
      algo = "texture.checker",
      params = { w = 32, h = 32, ch = 4, size = 4 },
    }
  end
  numbers.bundle16_ms = median3(function()
    return ms_of(function()
      local bundle = assetgen.bundle(entries, "perf-bundle")
      numbers.bundle_items = #bundle.items
    end)
  end)

  -- ⑥ PCM 装箱吞吐（样本/秒）：10 万帧单声道 wav16。装箱发生在**生成期**
  -- （`encode(p,"wav16")` 是恒等，见契约 §4），所以这里量整条生成 + 交织 + 装箱路径。
  local long_spec = {
    algo = "sound.osc",
    params = { wave = "sine", freq = 100, ms = 2100, sr = 48000, amp = 900, env = "none" },
  }
  local long = assetgen.sound(long_spec, "perf-pcm")
  local sink = long.bytes
  numbers.pcm_samples_per_s = median3(function()
    local started = os.clock()
    local last = nil
    for _ = 1, 3 do
      last = assetgen.sound(long_spec, "perf-pcm")
    end
    local elapsed = os.clock() - started
    if elapsed <= 0 then
      error("os.clock 没有推进（无法测吞吐）", 0)
    end
    sink = last.bytes
    return math.floor((long.frames * 3) / elapsed)
  end)

  -- ⑦ 常驻内存（GC 差，KiB）：**生成后再收一次**——否则量到的是临时串的垃圾。
  local measure = function(body)
    collectgarbage("collect")
    collectgarbage("collect")
    local before = collectgarbage("count")
    local kept = body()
    collectgarbage("collect")
    collectgarbage("collect")
    local after = collectgarbage("count")
    return math.floor((after - before) * 10 + 0.5) / 10, kept
  end
  local tex_kib, kept_texture = measure(function()
    return assetgen.texture(tex_spec, "perf-mem")
  end)
  numbers.tex64_mem_kib = tex_kib
  local sound_kib, kept_sound = measure(function()
    return assetgen.sound({ algo = "sound.noise", params = { ms = 1000, sr = 22050, amp = 800 } }, "perf-mem")
  end)
  numbers.sound1s_mem_kib = sound_kib

  -- 十个键分成两组：8 个由本用例产出，2 个由泄漏用例产出（同一次运行里已跑过）。
  local printed = 0
  local order = assetgen.perf_keys()
  for i = 1, #order do
    local key = order[i]
    local value = numbers[key]
    if value == nil then
      is_true(key == "leak_100_equal" or key == "leak_gc_pct", "未知的性能键 " .. key)
    else
      print(("perf.assetgen.%s=%s"):format(key, tostring(value)))
      printed = printed + 1
    end
  end
  eq(printed, 8, "本用例必须打印 8 个性能键（另 2 个由 leak-100-rounds 打印）")
  eq(#order, 10, "契约 §5 恰好十个性能键")

  is_true(numbers.load_ms <= 10, "load_ms 必须 ≤ 10，实得 " .. numbers.load_ms)
  is_true(numbers.tex64_ms <= 40, "tex64_ms 必须 ≤ 40（实测 17–18，留 2× 回归余量），实得 " .. numbers.tex64_ms)
  is_true(numbers.mesh_torus_ms <= 15, "mesh_torus_ms 必须 ≤ 15，实得 " .. numbers.mesh_torus_ms)
  is_true(numbers.sound_1s_ms <= 60, "sound_1s_ms 必须 ≤ 60（实测 27–29，留 2× 回归余量），实得 " .. numbers.sound_1s_ms)
  is_true(numbers.bundle16_ms <= 200, "bundle16_ms 必须 ≤ 200，实得 " .. numbers.bundle16_ms)
  is_true(numbers.pcm_samples_per_s >= 300000, "PCM 吞吐必须 ≥ 300k 样本/秒，实得 " .. numbers.pcm_samples_per_s)
  is_true(numbers.tex64_mem_kib <= 128, "64×64 RGBA 常驻必须 ≤ 128 KiB，实得 " .. numbers.tex64_mem_kib)
  is_true(numbers.sound1s_mem_kib <= 2560, "1s@22050 常驻必须 ≤ 2.5 MiB，实得 " .. numbers.sound1s_mem_kib)
  is_true(#kept_texture.bytes == 64 * 64 * 4, "纹理必须完整")
  is_true(#kept_sound.pcm == kept_sound.frames * 2, "音频必须完整")
  is_true(sink ~= nil and #sink == #long.bytes, "wav16 装箱必须完整")
  is_true(numbers.bundle_items == 16, "bundle 必须 16 件")
end)

end
