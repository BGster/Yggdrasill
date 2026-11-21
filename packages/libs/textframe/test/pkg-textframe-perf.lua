-- `pkg.textframe.perf` —— 契约 `docs/design/packages/textframe.md` §5 的五指标 / 九键。
--
-- # 基准集（固定，不是可调旋钮）
--
-- `80×24` 世界区 + `6` 行 HUD = 30 行帧，即 Saltline 的 `TUNING.world.view_w/view_h`
-- 与 HUD 六行。尺寸是定义的输入：想让数字好看就去调小它，等于把契约的口径改掉。
--
-- # 两个数字面
--
-- 同一个用例在 **debug**（门禁命令 `cargo run -q --example selftest`）与 **release**
-- 两面差约 3–4 倍。绝对帽按慢的那面（debug）标定，契约 §5 两面都记；真正的回归信号是
-- `vs_naive`——同面、同数据集、与构建档位无关的比值。
--
-- # 手搓对照 `naive_compose`
--
-- `out = out .. ch` 的朴素写法，正确性上与本包等价（`-compose` 文件里已用 r4 适配器
-- 逐字对照过），这里只当**性能标尺**。本包不追求在 w=80 时赢过它（此宽度下二次项的
-- 常数还没长出来），追求的是随 w 增长不塌：两两合并是 O(n·log n)，它是 O(n²)。

local S = rawget(_G, "__SELFTEST")
if type(S) ~= "table" then
  S = { pass = 0, fail = 0 }
  _G.__SELFTEST = S
end

local function run(name, body)
  local ok, err = pcall(body)
  if ok then
    S.pass = S.pass + 1
  else
    S.fail = S.fail + 1
    print("FAIL " .. name .. ": " .. tostring(err))
  end
end

local function is_true(value, label)
  if not value then
    error((label or "is_true") .. ": expected truthy", 2)
  end
end

local function eq(actual, expected, label)
  if actual ~= expected then
    error(("%s: expected %s, got %s"):format(tostring(label or "eq"), tostring(expected), tostring(actual)), 2)
  end
end

local function fresh()
  for name in pairs(package.loaded) do
    if type(name) == "string" and (name == "pkg.textframe" or name:find("^pkg%.textframe%.")) then
      package.loaded[name] = nil
    end
  end
  local module = require("pkg.textframe")
  module.install()
  return module
end

local function naive_compose(buf, hud_rows)
  local lines = {}
  local w = buf.w
  for row = 1, buf.h do
    local text = ""
    local base = (row - 1) * w
    for col = 1, w do
      text = text .. (buf.cells[base + col] or " ")
    end
    lines[row] = text
  end
  for i = 1, #hud_rows do
    lines[buf.h + i] = hud_rows[i]
  end
  return lines
end

local tf = fresh()

run("pkg.textframe.perf", function()
  -- ① 包加载（5 次取中位）
  local load_samples = {}
  for round = 1, 5 do
    for name in pairs(package.loaded) do
      if type(name) == "string" and (name == "pkg.textframe" or name:find("^pkg%.textframe%.")) then
        package.loaded[name] = nil
      end
    end
    local began = os.clock()
    local module = require("pkg.textframe")
    module.install()
    load_samples[round] = (os.clock() - began) * 1000
  end
  table.sort(load_samples)
  local load_ms = load_samples[3]
  print(("perf.textframe.load_ms=%.4f"):format(load_ms))
  is_true(load_ms < 10, ("包加载 < 10ms，实得 %.4fms"):format(load_ms))
  tf = require("pkg.textframe") -- 上面的循环清了缓存，这里拿回本轮实例

  -- 基准集
  local w, h = 80, 24
  local buf = tf.frame(w, h)
  local glyphs = ".:-=+*#oO0@%"
  for i = 1, w * h do
    buf.cells[i] = glyphs:sub((i % 11) + 1, (i % 11) + 1)
  end
  local hud = { "生命 88", "弹匣 12/30", "目标 2/3", "雨 300", "字幕", "进度 60%" }

  -- ② 单格写（10 万次）
  local probe = tf.frame(w, h)
  local t_put = os.clock()
  for i = 1, 100000 do
    tf.put(probe, (i - 1) % w + 1, (i - 1) // w % h + 1, "x")
  end
  local put_ns = (os.clock() - t_put) * 1e9 / 100000
  print(("perf.textframe.put_ns=%.1f"):format(put_ns))
  is_true(put_ns < 5000, ("单格写 < 5µs，实得 %.1fns"):format(put_ns))

  -- ③ 整帧 blit（一层 80×24 精灵覆盖层叠到世界区上，2000 次 × 3 中位）
  -- 形态取 Saltline `src/view/sprites.lua` 的实况：稀疏写入，约 1/20 的格子有字。
  local base = tf.frame(w, h)
  local layer = tf.frame(w, h)
  for i = 1, w * h do
    if i % 20 == 0 then
      layer.cells[i] = "*"
    end
  end
  local blit_us = 0
  do
    local runs = {}
    for r = 1, 3 do
      local began = os.clock()
      for _ = 1, 2000 do
        tf.blit(base, layer, 1, 1)
      end
      runs[r] = (os.clock() - began) * 1000 * 1000 / 2000
    end
    table.sort(runs)
    blit_us = runs[2]
  end
  print(("perf.textframe.blit_us=%.1f"):format(blit_us))
  is_true(blit_us <= 500, ("整帧 blit ≤ 500µs，实得 %.1fµs"):format(blit_us))

  -- ④⑦ 帧合成吞吐 / vs 手搓（3 次取中位，每次 2000 帧）
  local function bench(fn)
    local runs = {}
    for r = 1, 3 do
      local began = os.clock()
      for _ = 1, 2000 do
        fn(buf, hud)
      end
      runs[r] = (os.clock() - began) * 1000 / 2000
    end
    table.sort(runs)
    return runs[2]
  end
  local pkg_ms = bench(function(b, rows)
    return tf.stack({ tf.to_lines(b), rows })
  end)
  local naive_ms = bench(naive_compose)
  local compose_us = pkg_ms * 1000
  local vs_naive = pkg_ms / naive_ms
  print(("perf.textframe.compose_us=%.1f"):format(compose_us))
  print(("perf.textframe.throughput_fps=%.1f"):format(1000 / pkg_ms))
  print(("perf.textframe.vs_naive=%.2f"):format(vs_naive))
  is_true(compose_us <= 1000, ("帧合成 ≤ 1000µs（debug 面实测的约 2×），实得 %.1fµs"):format(compose_us))
  -- 回归信号：同面 vs 手搓（与构建档位无关）。实测 debug 1.6–1.7× / release 1.3×；
  -- 阈值 2× 抓的是「拼接路径退化」，不是这台机器的抖动。
  is_true(vs_naive <= 2.0, ("本包 ≤ 2.0× 手搓（pkg %.4fms vs naive %.4fms = %.2f×）"):format(pkg_ms, naive_ms, vs_naive))

  -- ⑤ 每帧常驻内存（100 帧）
  collectgarbage()
  collectgarbage()
  local m0 = collectgarbage("count")
  local frame_kib = 0
  do
    -- 块作用域：100 帧在块内被持有（量的是「留下 100 帧的成本」），出块即释放
    local kept = {}
    for i = 1, 100 do
      kept[i] = tf.stack({ tf.to_lines(buf), hud })
    end
    eq(#kept, 100, "常驻测量持有 100 帧")
    -- **先整收一次再读**：`collectgarbage("count")` 的差值会把「这 100 轮循环里
    -- 还没被回收的瞬时垃圾」一并算进来，而 GC 什么时候跑取决于进入本用例时的堆大小
    -- （跨文件耦合：实测同一份代码在前序包多分配约 1 MB 时从 4.6 涨到 13.7 KiB/帧）。
    -- 本条要量的是「留下 100 帧」的**活**内存，所以差值必须在一次整收之后取。
    collectgarbage()
    frame_kib = (collectgarbage("count") - m0) / 100
  end
  print(("perf.textframe.frame_kib=%.2f"):format(frame_kib))
  is_true(frame_kib <= 8, ("每帧 ≤ 8KiB（实测 4.65 的约 1.7×），实得 %.2fKiB"):format(frame_kib))

  -- ⑥ 泄漏：100 轮后内存差 + 逐字符相等
  collectgarbage()
  collectgarbage()
  local mem_base = collectgarbage("count")
  local first = table.concat(tf.to_lines(buf), "\n")
  local equal = 1
  for _ = 1, 100 do
    if table.concat(tf.to_lines(buf), "\n") ~= first then
      equal = 0
    end
  end
  collectgarbage()
  local after = collectgarbage("count")
  local leak_pct = 0
  if mem_base > 0 then
    leak_pct = (after - mem_base) / mem_base * 100
  end
  print(("perf.textframe.leak_gc_pct=%.3f"):format(leak_pct))
  print(("perf.textframe.leak_100_equal=%d"):format(equal))
  is_true(leak_pct <= 1, ("100 轮内存增长 ≤ 1%%，实得 %.3f%%"):format(leak_pct))
  eq(equal, 1, "100 轮输出逐字符相等")
end)

return S
