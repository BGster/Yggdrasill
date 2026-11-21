-- `tests/test-g-stress.lua` —— v3 长局压测（bot 台 + 五指标仪表盘 + 残留键口径）。
--
-- # 契约定位（docs/design/mod/shapez-game.md §2.7）
--
-- 档位（计划 §4）：`SHAPEZ_STRESS=full` → 全量档（10⁴ 拍 / 10⁴ 拆建轮 /
-- 100 轮存读档画像 / 10⁴ 档吞吐三读数，显式命令不进默认门禁）；缺省 fast
-- （2×10³ 拍 / 2×10² 拆建轮 / 3 次周期存读档 + 吞吐单发往返 / 3 桶耗时）。
-- 档位探测 pcall 守卫 os（缺席降 fast 并注记——沿 test-b 先例）。
--
-- 断言纪律：只断**不变量**（决策 KAT 恰等 / KV 键数恰 +spawn 数 / 墓碑恰
-- +destroy 数 / 锚·探针逐值相等 / 往返逐字节 / id 保真）；性能只产读数
-- （`PERF shapez.v3.*` 八行固定键 + INFO 注记行），**不阈断**——防机器抖动红门。
--
-- 仪表盘口径：selftest 普通 Lua 面。os.clock / collectgarbage 经 pcall 探测，
-- 生产沙箱 deny 二者（R-b：仪表盘只落 selftest 面；缺席读数以 -1 标缺）。
--
-- 闭环剧本（g4）：closed 模式首存拍（含）前 extend/demolish 空转 ⇒ 首存档
-- 零带零洞（id 致密，test-f「k 拍前零传送带」约束的 bot 版）。A 局与续跑局
-- B 同节奏周期存读档（各自落档）：save→reload 丢带内运输件是 v2 既定登记
-- （§4 简化 10），两侧同扰动 ⇒ [k..N] 全锚逐值相等 + 终态导出逐字节。
-- 零件流转压测在 g3（无存档非闭环）与 g5（吞吐台直生世界）。

local bot = require("src.bot")
local savegame = require("src.savegame")
local simtick = require("src.simtick")
local codec = require("pkg.codec")
local entity = require("pkg.entity")
local procgen = require("pkg.procgen")

local WORLD = ygg.data("ygg:shapez/world")

local SEED_A = "s26-v3-golden"
local SEED_B = "s26-v3-alt"
local ANCHOR_EVERY = WORLD.tuning.anchorEveryTicks
local THR_PATH = "s26-v3-thr.json"
local LEAK_PATH = "s26-v3-leak.json"
local CL_PATH = "s26-v3-cl"

-- ── 档位与规模常量（唯一调节面；full 超预算先降轮数再降规模，见契约 §2.7）──
local FULL = false
do
  local ok, v = pcall(function()
    return os.getenv("SHAPEZ_STRESS")
  end)
  if ok and v == "full" then
    FULL = true
  end
end

local S = FULL and {
  ticks = 10000, churn = 10000, save_every = 2000,
  g3_ticks = 3000, thr_n = 10000, leak_rounds = 100, buckets = 10,
  caps = { belts = 16, procs = 8, miners = 3 },
} or {
  ticks = 2000, churn = 200, save_every = 500,
  g3_ticks = 1000, thr_n = 2000, leak_rounds = 0, buckets = 3,
  caps = { belts = 12, procs = 6, miners = 2 },
}

print("INFO shapez.v3.tier=" .. (FULL and "full" or "fast")
  .. " ticks=" .. S.ticks .. " churn=" .. S.churn .. " save_every=" .. S.save_every
  .. " thr_n=" .. S.thr_n .. " leak_rounds=" .. S.leak_rounds)

-- ── 探针（生产沙箱 deny os/collectgarbage → pcall 探测，缺席 -1 标缺）───────
local HAVE_CLOCK, HAVE_GC = false, false
do
  local okc, clockv = pcall(function()
    return os.clock()
  end)
  HAVE_CLOCK = okc and type(clockv) == "number"
  HAVE_GC = pcall(function()
    collectgarbage("collect")
    return collectgarbage("count")
  end)
end
if not HAVE_CLOCK or not HAVE_GC then
  print("INFO shapez.v3.probe_missing clock=" .. tostring(HAVE_CLOCK) .. " gc=" .. tostring(HAVE_GC))
end

local function now_ms()
  if HAVE_CLOCK then
    return os.clock() * 1000
  end
  return 0
end

--- 双 collect 后读 count（entity.md 泄漏口径的采集同款）。
local function gc_kb()
  if HAVE_GC then
    collectgarbage("collect")
    collectgarbage("collect")
    return collectgarbage("count")
  end
  return -1
end

-- ── KV 键普查（仪表盘①口径；生产沙箱无遍历面 ⇒ 只在 selftest 镜像成立）────
--- 分桶：rec.*（spawn 落键；destroy 后成墓碑）/ sz.path.*（带路径态，含
--- 失效带 = startid 已不在世的键）/ sz.anchor.* / sz.chunk.* / 其余。
local function census()
  local kv = ygg_kv_all()
  local c = { all = 0, rec = 0, rec_bytes = 0, path = 0, path_dead = 0,
    anchor = 0, chunk = 0, other = 0 }
  for key in pairs(kv) do
    c.all = c.all + 1
    if key:find("^pkg%.entity%.rec%.") then -- spawn 落键；destroy 后成墓碑（entity.md S9）
      c.rec = c.rec + 1
      c.rec_bytes = c.rec_bytes + #key + #tostring(kv[key])
    elseif key:find("^sz%.path%.") then
      c.path = c.path + 1
      local startid = tonumber(key:match("^sz%.path%.(%d+)$"))
      if startid == nil or not entity.exists(startid) then
        c.path_dead = c.path_dead + 1
      end
    elseif key:find("^sz%.anchor%.") then
      c.anchor = c.anchor + 1
    elseif key:find("^sz%.chunk%.") then
      c.chunk = c.chunk + 1
    else
      c.other = c.other + 1
      c.other_list = c.other_list or {}
      c.other_list[#c.other_list + 1] = key
    end
  end
  return c
end

local function fresh(seed)
  reset_all()
  ygg_dispatch("event.start", seed)
end

-- 计时中位数助手（selftest 面 string 可用；先定义后用——check 体立即执行）。
local function sort_asc(list)
  for i = 1, #list do
    local min = i
    for j = i + 1, #list do
      if list[j] < list[min] then
        min = j
      end
    end
    if min ~= i then
      list[i], list[min] = list[min], list[i]
    end
  end
  return list
end

local function median_of(list)
  sort_asc(list)
  local n = #list
  if n == 0 then
    return 0
  end
  if n % 2 == 1 then
    return list[(n + 1) // 2]
  end
  return (list[n // 2] + list[n // 2 + 1]) / 2
end

-- g4/g5/g6 的跨用例传递（check 闭包共享同一 chunk 上值）。
local MAIN = nil -- g4 主局（闭环 A）报告 + 键账 + 终态导出
local THR = nil -- g5 吞吐读数

-- ============================================================================
-- g1 决策流 KAT：同 seed 逐值相等 + 加权比例钉值 + 跨 seed 否定
-- ============================================================================

-- KAT 钉值（decide 纯抽取 @ N=2000，seed="s26-v3-golden"；确定性 ⇒ 恰等）。
-- 变更决策表/权重/流名任一项即本表失配——重测后就地更新并记设计依据。
local KAT_N = 2000
local KAT = { extend = 709, demolish = 493, place = 600, savepoint = 198 }

check("g1 决策流 KAT：同 seed 逐值相等 + 比例钉值 + 跨 seed 否定", function()
  local counts = { extend = 0, demolish = 0, place = 0, savepoint = 0 }
  for t = 1, KAT_N do
    local a1 = bot.decide(SEED_A, t)
    counts[a1] = counts[a1] + 1
    assert_eq(bot.decide(SEED_A, t), a1, "同拍重抽逐值相等 @" .. t)
  end
  assert_eq(counts.extend, KAT.extend, "extend 抽数")
  assert_eq(counts.demolish, KAT.demolish, "demolish 抽数")
  assert_eq(counts.place, KAT.place, "place 抽数")
  assert_eq(counts.savepoint, KAT.savepoint, "savepoint 抽数")
  -- 比例带宽锚（35/25/30/10 ±3pt；钉值失配时先看这四条定位）
  local function band(count, target)
    local d = count / KAT_N - target
    if d < 0 then
      d = -d
    end
    assert_true(d < 0.03, "比例带宽 " .. target)
  end
  band(counts.extend, 0.35)
  band(counts.demolish, 0.25)
  band(counts.place, 0.30)
  band(counts.savepoint, 0.10)
  -- 跨 seed 否定（方法：两 seed 逐拍比对，存在分拍即证流隔离）
  local diff = false
  for t = 1, KAT_N do
    if bot.decide(SEED_A, t) ~= bot.decide(SEED_B, t) then
      diff = true
      break
    end
  end
  assert_true(diff, "跨 seed 决策序列应有差异")
end)

-- ============================================================================
-- g2 拆建循环残留键（S23 口径）：墓碑恰 +N、其他键族零增量、失效带读数
-- ============================================================================

check("g2 拆建循环残留键：KV 键数恰 +spawn、墓碑恰 +destroy、attr 余键 0", function()
  fresh(SEED_A)
  -- 预热 1 枚 spawn+destroy：pkg.entity.count/seq 计数键先落位，
  -- 否则首 spawn 的计数键会混进「恰 +N」口径。
  local warm = entity.spawn("sz.belt", { x = -1, y = -1, rot = 0 }, { belt = true })
  entity.destroy(warm)
  local c0 = census()
  local live0 = entity.count()
  local ch = bot.churn(S.churn, gc_kb)
  local c1 = census()
  local live1 = entity.count()
  assert_eq(ch.spawned, S.churn, "churn spawn 数")
  assert_eq(c1.rec - c0.rec, S.churn, "rec 键恰 +churn（每 spawn 恰 1 键）")
  assert_eq(c1.all - c0.all, S.churn, "KV 键总数恰 +churn（其他键族零增量 = attr 余键 0）")
  assert_eq(live1, live0, "churn 自生自灭（活体零净增）")
  print("INFO shapez.v3.g2_tombstone_bytes=" .. (c1.rec_bytes - c0.rec_bytes)
    .. " path_dead=" .. c1.path_dead .. " churn_mem_kb=" .. ch.mem_before .. "->" .. ch.mem_after)
end)

-- ============================================================================
-- g3 同 seed 双局长局重放（非闭环、无存档；零件流压测的主场）
-- ============================================================================

check("g3 回放哈希：同 seed 双局锚·探针·终态逐值相等；跨 seed 否定", function()
  fresh(SEED_A)
  local ra = bot.run({ seed = SEED_A, ticks = S.g3_ticks, caps = S.caps, clock = now_ms, gc = gc_kb })
  fresh(SEED_A)
  local rb = bot.run({ seed = SEED_A, ticks = S.g3_ticks, caps = S.caps, clock = now_ms, gc = gc_kb })
  assert_true(#ra.anchors > 0, "锚非空（重放对账有对象）")
  assert_eq(#ra.anchors, #rb.anchors, "双局锚数")
  for i = 1, #ra.anchors do
    assert_eq(rb.anchors[i].tick, ra.anchors[i].tick, "锚拍 @" .. i)
    assert_eq(rb.anchors[i].hash, ra.anchors[i].hash, "锚哈希 @" .. ra.anchors[i].tick)
  end
  assert_true(#ra.marks > 0, "存档点探针非空")
  assert_eq(#ra.marks, #rb.marks, "双局探针数")
  for i = 1, #ra.marks do
    assert_eq(rb.marks[i].hash, ra.marks[i].hash, "探针哈希 @" .. ra.marks[i].tick)
  end
  assert_eq(rb.final_hash, ra.final_hash, "终态快照哈希")
  assert_eq(rb.actions.extend_hit, ra.actions.extend_hit, "执行面 extend")
  assert_eq(rb.actions.demolish_hit, ra.actions.demolish_hit, "执行面 demolish")
  assert_eq(rb.actions.place_hit, ra.actions.place_hit, "执行面 place")
  print("INFO shapez.v3.g3_executed extend=" .. ra.actions.extend_hit
    .. " demolish=" .. ra.actions.demolish_hit .. " place=" .. ra.actions.place_hit
    .. " noop=" .. ra.actions.noop .. " marks=" .. #ra.marks)
  -- 跨 seed 否定（方法：换 seed 同脚本重跑，终态哈希必异）
  fresh(SEED_B)
  local rc = bot.run({ seed = SEED_B, ticks = 200, caps = S.caps, clock = now_ms, gc = gc_kb })
  assert_true(rc.final_hash ~= ra.final_hash, "跨 seed 终态应不同")
end)

-- ============================================================================
-- g4 周期存读档闭环（长局版三局闭环）：B 续跑局 ≡ A 于 [k..N] 全锚 + 终态逐字节
-- ============================================================================

check("g4 周期存读档闭环：B ≡ A 于 [k..N] 全锚 + 探针 + 终态导出逐字节", function()
  fresh(SEED_A)
  local cen0 = census()
  local live0 = entity.count()
  local a = bot.run({
    seed = SEED_A, ticks = S.ticks, churn = 0, closed = true,
    save_every = S.save_every, save_path = CL_PATH, reset = reset_all,
    caps = S.caps, clock = now_ms, gc = gc_kb, profile = true,
  })
  local cen1 = census()
  local live1 = entity.count()
  -- 闭环结构不变量
  assert_eq(a.saves.count, S.ticks // S.save_every - 1, "周期存读档次数（末拍不存）")
  assert_true(#a.anchors == S.ticks // ANCHOR_EVERY, "锚数 = 拍数/锚距")
  assert_true(a.actions.extend_hit > 0, "闭环带面有执行（种子落格）")
  -- KV 键账（仪表盘①：主局口径 = 决策面 spawn/destroy 的账本；sz.chunk.* 是
  -- mapgen 懒生成缓存（§2.5 既定不进档），closed 轨无零件 ⇒ path 键零增量）
  local spawned = a.actions.extend_hit + a.actions.place_hit
  local delta_rec = cen1.rec - cen0.rec
  local delta_all = cen1.all - cen0.all
  local delta_chunk = cen1.chunk - cen0.chunk
  local delta_path = cen1.path - cen0.path
  assert_eq(delta_rec, spawned, "rec 键恰 +spawn 数")
  assert_eq(delta_path, 0, "闭环轨 path 键零增量（零件不上带）")
  -- 结构键增量（首拍/首 spawn 才落位的账面键，非实体残茶）：sz.tick /
  -- sz.layout_v / pkg.entity.count / pkg.entity.seq。
  local STRUCT = { ["sz.tick"] = true, ["sz.layout_v"] = true,
    ["pkg.entity.count"] = true, ["pkg.entity.seq"] = true }
  local seen = {}
  for i = 1, #(cen1.other_list or {}) do
    seen[cen1.other_list[i]] = true
  end
  for i = 1, #(cen0.other_list or {}) do
    seen[cen0.other_list[i]] = nil
  end
  local delta_anchor = cen1.anchor - cen0.anchor
  local delta_struct = 0
  local unknown_new = nil
  for key in pairs(seen) do
    if STRUCT[key] then
      delta_struct = delta_struct + 1
    else
      unknown_new = key
    end
  end
  assert_true(unknown_new == nil, "未知新键族现身：" .. tostring(unknown_new))
  assert_eq(delta_all - delta_chunk - delta_path - delta_anchor - delta_struct, delta_rec,
    "非 rec·非缓存·非锚·非结构键族零增量（attr 余键 0）")
  assert_eq(delta_rec - (live1 - live0), a.actions.demolish_hit, "destroy 恰留 1 墓碑/次")
  MAIN = {
    rep = a,
    cen = { rec = delta_rec, all = delta_all,
      rec_bytes = cen1.rec_bytes - cen0.rec_bytes, path_dead = cen1.path_dead,
      chunk = cen1.chunk, anchor = cen1.anchor },
  }
  MAIN.a_export = codec.export({ entities = true, ids = "index", keys = savegame.named_keys(), mod = "shapez" })
  MAIN.a_final = procgen.hash(simtick.snapshot())
  print("INFO shapez.v3.g4_a saves=" .. a.saves.count
    .. " save_ms=" .. string.format("%.1f", a.saves.save_ms)
    .. " load_ms=" .. string.format("%.1f", a.saves.load_ms)
    .. " spawn=" .. spawned .. " demolish=" .. a.actions.demolish_hit)
  -- ── B 局：load A@k 首存档，续跑到 N（同节奏存读档，同扰动）─────────────
  local k = S.save_every
  reset_all()
  local receipt = savegame.load(CL_PATH .. "." .. k)
  assert_true(receipt.bytes_equal, "B 载入三重对账")
  assert_true(receipt.snap_checked, "锚拍存档快照硬校")
  local b = bot.run({
    seed = SEED_A, ticks = S.ticks - k, churn = 0, closed = true,
    save_every = S.save_every, save_path = CL_PATH, reset = reset_all,
    caps = S.caps, clock = now_ms, gc = gc_kb,
  })
  -- B ≡ A 于 [k..N]：锚与探针逐值
  local a_tail, a_marks = {}, {}
  for i = 1, #a.anchors do
    if a.anchors[i].tick > k then
      a_tail[#a_tail + 1] = a.anchors[i]
    end
  end
  for i = 1, #a.marks do
    if a.marks[i].tick > k then
      a_marks[#a_marks + 1] = a.marks[i]
    end
  end
  assert_eq(#b.anchors, #a_tail, "B 续跑段锚数 == A [k..N] 锚数")
  for i = 1, #a_tail do
    assert_eq(b.anchors[i].tick, a_tail[i].tick, "锚拍对齐 @" .. i)
    assert_eq(b.anchors[i].hash, a_tail[i].hash, "续跑锚哈希 @" .. a_tail[i].tick)
  end
  assert_eq(#b.marks, #a_marks, "B 探针数 == A [k..N] 探针数")
  for i = 1, #a_marks do
    assert_eq(b.marks[i].hash, a_marks[i].hash, "续跑探针 @" .. a_marks[i].tick)
  end
  assert_eq(b.final_hash, MAIN.a_final, "终态快照哈希相等")
  local b_export = codec.export({ entities = true, ids = "index", keys = savegame.named_keys(), mod = "shapez" })
  assert_eq(codec.encode(b_export), codec.encode(MAIN.a_export), "终态导出逐字节")
end)

-- ============================================================================
-- g5 吞吐台（仪表盘②）+ 高水位决策/tick 读数 + 存读档泄漏画像（full）
-- ============================================================================

check("g5 吞吐台：export/read+import 计时读数 + 往返逐字节 + id 保真", function()
  fresh(SEED_A)
  -- 直生 THR_N 实体（密集网格；不经事件面——压测台口径）
  for i = 0, S.thr_n - 1 do
    entity.spawn("sz.belt", {
      x = 2000 + i % 100,
      y = 2000 + (i // 100) % 100,
      rot = i % 4,
    }, { belt = true })
  end
  local thr_ticks = FULL and 3 or 1
  -- export median（重复导出计时；同一份世界）
  local export_ms = {}
  local bundle
  for _ = 1, thr_ticks do
    local t0 = now_ms()
    bundle = codec.export({ entities = true, ids = "index", keys = savegame.named_keys(), mod = "shapez" })
    export_ms[#export_ms + 1] = now_ms() - t0
  end
  local bytes = #codec.encode(bundle)
  local written = codec.write(THR_PATH, bundle)
  -- read+import median（每次入 fresh KV；schema 先行注册——savegame.load 同款）
  local import_ms = {}
  for _ = 1, thr_ticks do
    reset_all()
    simtick.register_schemas()
    local t0 = now_ms()
    local back = codec.read(THR_PATH)
    codec.import(back, { on_conflict = "overwrite" })
    import_ms[#import_ms + 1] = now_ms() - t0
  end
  local export_eps = S.thr_n / (median_of(export_ms) / 1000)
  local import_eps = S.thr_n / (median_of(import_ms) / 1000)
  THR = { export_eps = export_eps, import_eps = import_eps, bytes = bytes }
  -- 往返逐字节（write 回执 sha == read 重编码；bundle 补 checksum 域后同编）
  bundle.checksum = written.sha256
  local back = codec.read(THR_PATH)
  assert_eq(codec.encode(back), codec.encode(bundle), "write→read 逐字节")
  assert_eq(entity.count(), S.thr_n, "import id 保真（致密重建计数）")
  -- 高水位读数：吞吐世界上的决策+tick 采样（fast 100 / full 30 拍；路径序
  -- O(P²) 支配单拍成本，10⁴ 世界采样宜短——读数入巡检记录，不阈断）
  local big = bot.run({ seed = SEED_B, ticks = FULL and 30 or 100,
    caps = { belts = 16384, procs = 4096, miners = 1024 },
    clock = now_ms, gc = gc_kb, profile = true })
  local bp50 = bot.pctl(big.tick_ms, 0.50)
  local bp99 = bot.pctl(big.tick_ms, 0.99)
  print("INFO shapez.v3.g5_bundle_bytes=" .. bytes
    .. " export_ms=" .. string.format("%.1f", median_of(export_ms))
    .. " import_ms=" .. string.format("%.1f", median_of(import_ms)))
  print("INFO shapez.v3.g5_bigworld tick_p50_us=" .. string.format("%.0f", bp50 * 1000)
    .. " p99_us=" .. string.format("%.0f", bp99 * 1000)
    .. " exec extend=" .. big.actions.extend_hit)
  -- 存读档泄漏画像（full：100 轮 @2×10³ 实体中档；泄漏判据看增长不看绝对量）
  if S.leak_rounds > 0 then
    -- 降档到 2×10³：重建中档世界（吞吐世界保持原样不再动）
    reset_all()
    fresh(SEED_A)
    for i = 0, 1999 do
      entity.spawn("sz.belt", { x = 3000 + i % 50, y = 3000 + (i // 50) % 40, rot = i % 4 }, { belt = true })
    end
    local m0 = gc_kb()
    local round_ms = {}
    for _ = 1, S.leak_rounds do
      local t0 = now_ms()
      savegame.save(LEAK_PATH)
      reset_all()
      savegame.load(LEAK_PATH)
      round_ms[#round_ms + 1] = now_ms() - t0
    end
    local m1 = gc_kb()
    assert_eq(entity.count(), 2000, "泄漏画像轮后计数不变")
    print("INFO shapez.v3.g5_leak rounds=" .. S.leak_rounds
      .. " round_ms_med=" .. string.format("%.1f", median_of(round_ms))
      .. " mem_kb=" .. string.format("%.0f", m0) .. "->" .. string.format("%.0f", m1)
      .. " delta_kb=" .. string.format("%.1f", m1 - m0))
  end
end)


-- ============================================================================
-- g6 仪表盘落点：PERF 八行固定键 + 分相/桶 INFO（读数不阈断）
-- ============================================================================

check("g6 仪表盘读数落点：PERF 八行 + 内存分相 + tick 桶均值（主局账面闭合）", function()
  assert_true(MAIN ~= nil, "g4 主局报告在场")
  assert_true(THR ~= nil, "g5 吞吐读数在场")
  local rep = MAIN.rep
  -- PERF 八行固定键（五指标：①残留键×2 ②吞吐×2 ③内存×1 ④tick×2 ⑤回放×1）
  print("PERF shapez.v3.kv_residual_keys=" .. MAIN.cen.rec)
  print("PERF shapez.v3.tombstone_bytes=" .. MAIN.cen.rec_bytes)
  print(string.format("PERF shapez.v3.export_eps=%.0f", THR.export_eps))
  print(string.format("PERF shapez.v3.import_eps=%.0f", THR.import_eps))
  print(string.format("PERF shapez.v3.mem_final_kb=%.0f", rep.mem.final))
  print(string.format("PERF shapez.v3.tick_p50_us=%.0f", bot.pctl(rep.tick_ms, 0.50) * 1000))
  print(string.format("PERF shapez.v3.tick_p99_us=%.0f", bot.pctl(rep.tick_ms, 0.99) * 1000))
  print("PERF shapez.v3.replay_final_hash=" .. rep.final_hash)
  -- 分相与桶（INFO；泄漏口径读数：终值 − build 基线，扣墓碑见巡检记录表）
  local per = S.ticks // S.buckets
  local buckets = {}
  for b = 1, S.buckets do
    local sum = 0
    for i = (b - 1) * per + 1, b * per do
      sum = sum + rep.tick_ms[i]
    end
    buckets[b] = sum / per
  end
  local line = "INFO shapez.v3.tick_buckets_ms="
  for b = 1, S.buckets do
    line = line .. string.format("%.3f", buckets[b]) .. (b < S.buckets and "," or "")
  end
  print(line)
  print("INFO shapez.v3.mem_phases_kb build=" .. string.format("%.0f", rep.mem.build)
    .. " run=" .. string.format("%.0f", rep.mem.run)
    .. " final=" .. string.format("%.0f", rep.mem.final)
    .. " delta=" .. string.format("%.1f", rep.mem.final - rep.mem.build))
  print("INFO shapez.v3.kv_shape path=" .. (MAIN.cen.rec >= 0 and "" or "") .. "path_dead="
    .. MAIN.cen.path_dead .. " chunk_cache=" .. MAIN.cen.chunk .. " anchors=" .. MAIN.cen.anchor)
  -- 主局账面闭合（不变量，非性能）
  assert_eq(rep.ticks, S.ticks, "主局拍数")
  assert_true(#rep.marks > 0, "存档点探针非空")
end)
