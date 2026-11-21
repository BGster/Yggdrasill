-- `tests/test-f-save.lua` —— v2 存档信封：codec 往返 + A/B/C 三局联调闭环。
--
-- 覆盖：write/read 往返逐字节（§4⑤ + codec.roundtrip 双保险）；误载脏 KV
-- 守卫；load-continue（B 局载入续跑，判据 §4①–⑤）；幂等导入 created=0
-- （codec I2）；篡改一字节拒 YGG-PKG-CODEC-CHECKSUM（方法=翻转正文字节）；
-- mapgen_v 不符拒；派生态（sz.path.*/sz.chunk.*）不进档；PERF save_ms/load_ms。
--
-- 剧本（§4 联调层）：黄金种子开局 → hub/miner@Cu 圆心直喂过 1→2 → cutter 线
-- （矿机臂格喂割刀、割刀正向交割进 hub）过 2→3 → k=1000 拍 save（此拍前
-- **零传送带**——路径态是唯一不可重建的易变态，k 前不放带则 §4①–④ 判据
-- 与「sz.path.* 不进档」同时成立）→ 主机 B（fresh KV）load → 双机同日志
-- 放带链（Ru 斑圆心起南行再西行入 hub）+ miner@Ru → 续跑至 N=4000。
-- 对照局 C 不中断跑满 0..N。判据：A≡C 全锚、B≡C 于 [k..N] 全锚、B@k 哈希
-- == A@k、终态快照逐字节、传输零损（文件字节面）。

local mapgen = require("src.mapgen")
local savegame = require("src.savegame")
local simtick = require("src.simtick")
local codec = require("pkg.codec")
local procgen = require("pkg.procgen")

local WORLD = ygg.data("ygg:shapez/world")

local GOLDEN_SEED = "s26-v2-golden"
local K = 1000 -- 存档拍（恰为锚拍）
local N = 4000 -- 续跑终点
local SAVE_PATH = "s26-v2-save.json"

--- Cu 斑（chunk -1,0）白盒格：circles[1] 圆心 + 东臂（圆心四臂恒命中）。
--- 布局：miner@(C) rot1 → belt@(C+0,+1) rot1 → hub@(C+0,+2)；
---       miner2@(C+1,0) rot1 → cutter@(C+1,+1) rot2（出料正向交割进 hub）。
---       k 前零传送带之外的全部实体；带链在 save 之后再放（双机同日志）。
local function layout_cells()
  local c = mapgen.ensure_chunk(-1, 0)[1].circles[1]
  local cx = -16 + c[1]
  local cy = c[2]
  return { miner1 = { cx, cy }, belt = { cx, cy + 1 }, hub = { cx, cy + 2 },
           cutter = { cx + 1, cy + 1 }, miner2 = { cx + 1, cy } }
end

--- Ru 斑（chunk 0,-1）圆心（带链起点矿机；save 后才放置）。
local function ru_cell()
  local c = mapgen.ensure_chunk(0, -1)[1].circles[1]
  return c[1], -16 + c[2]
end

--- 事件日志：三局共用的动作序列（确定性回放面）。
--- 几何在 scratch 局内白盒派生（种子已设才能生成 chunk），再由各用例 reset 后回放。
local function geometry()
  reset_all()
  ygg_dispatch("event.start", GOLDEN_SEED)
  return layout_cells(), ru_cell()
end

local function event_script(L, ru_x, ru_y)
  local pre = {
    { "event.start", GOLDEN_SEED },
    { "sz.place.hub", L.hub[1] .. "," .. L.hub[2] .. ",0" },
    { "sz.place.miner", L.miner1[1] .. "," .. L.miner1[2] .. ",1" },
    { "sz.place.proc", "cutter|" .. L.cutter[1] .. "," .. L.cutter[2] .. ",2" },
    { "sz.place.miner", L.miner2[1] .. "," .. L.miner2[2] .. ",1" },
  }
  -- k 前推进到 K（1→2→3 过级在直喂/交割线下完成）
  pre[#pre + 1] = { "sz.tick", tostring(K) }
  -- post：Ru 线（带链：Ru 圆心矿机南行到 hub 行高再西行入 hub）+ 推进到 N
  local south = L.hub[2] - ru_y - 1 -- 南行带数（从 ru_y+1 到 hub_y-1）
  local west = ru_x - L.hub[1] - 1 -- 西行带数（从 hub_x+1 到 ru_x-1）
  local post = {}
  if south > 0 and west > 0 then
    post[#post + 1] = { "sz.place.miner", ru_x .. "," .. ru_y .. ",1" }
    for i = 1, south do
      post[#post + 1] = { "sz.place.belt", ru_x .. "," .. (ru_y + i) .. ",1" }
    end
    for i = 1, west do
      post[#post + 1] = { "sz.place.belt", (ru_x - i) .. "," .. L.hub[2] .. ",2" }
    end
  end
  post[#post + 1] = { "sz.tick", tostring(N - K) }
  return { pre = pre, post = post, south = south, west = west }
end


--- 回放一段事件日志。
local function play(log)
  for i = 1, #log do
    ygg_dispatch(log[i][1], log[i][2])
  end
end

--- 收集当前 KV 里全部锚（t 升序）。
local function collect_anchors()
  local all = ygg_kv_all()
  local out = {}
  for key, value in pairs(all) do
    local t = key:match("^sz%.anchor%.(%d+)$")
    if t ~= nil then
      out[#out + 1] = { t = tonumber(t), hash = value }
    end
  end
  table.sort(out, function(a, b)
    return a.t < b.t
  end)
  return out
end

--- 各用例入口：scratch 局派生几何 → 干净局回放 pre 段。
local function start_pre()
  local L, ru_x, ru_y = geometry()
  local script = event_script(L, ru_x, ru_y)
  reset_all()
  play(script.pre)
  return script
end

check("save.roundtrip-bytes", function()
  local script = start_pre()
  local receipt = savegame.save(SAVE_PATH)
  assert_true(receipt.bytes ~= nil and receipt.bytes > 0, "落盘字节数回执")
  -- ⑤ 传输零损：文件字节面 == write 回执口径
  local text = ygg.fs.read(SAVE_PATH)
  assert_eq(#text, receipt.bytes, "fs 字节 == 回执字节")
  -- 读回重编码 == 再读再编码（读回稳定）
  local bundle = codec.read(SAVE_PATH)
  assert_eq(codec.encode(codec.read(SAVE_PATH)), codec.encode(bundle), "读回稳定")
  -- codec.roundtrip 双保险（id 保真 → fresh KV 导入不改 id，字节可等）
  local rt = codec.roundtrip(SAVE_PATH)
  assert_true(rt.equal, "codec.roundtrip 字节相等")
end)

check("save.guard-dirty-kv", function()
  local script = start_pre()
  savegame.save(SAVE_PATH)
  -- 活局上 load 必拒（误载脏 KV 防护）
  assert_error("YGG-MOD-SAVE-STATE", function()
    savegame.load(SAVE_PATH)
  end, "活局拒绝覆盖载入")
  -- 未开局存档必拒
  reset_all()
  assert_error("YGG-MOD-SAVE-STATE", function()
    savegame.save(SAVE_PATH)
  end, "未开局拒绝存档")
end)

check("save.derived-keys-excluded", function()
  local script = start_pre()
  mapgen.ensure_chunk(3, 3) -- chunk 缓存在场（派生态）
  savegame.save(SAVE_PATH)
  local text = ygg.fs.read(SAVE_PATH)
  assert_true(text:find("sz%.path%.", 1, false) == nil, "sz.path.* 不进档（ensure 可重建）")
  assert_true(text:find("sz%.chunk%.", 1, false) == nil, "sz.chunk.* 不进档（seed 唯真源）")
  assert_true(text:find("quest%.types", 1, false) ~= nil, "quest 自报键在档（点名族③）")
  assert_true(text:find("sz%.anchor%." .. K, 1, false) ~= nil, "锚键在档（点名族②）")
  local bundle = codec.read(SAVE_PATH)
  assert_eq(bundle.kv["sz.seed"], GOLDEN_SEED, "sz.seed 在档")
  assert_eq(bundle.kv["sz.mapgen_v"], mapgen.MAPGEN_V, "sz.mapgen_v 在档")
end)

check("save.load-continue-abc-closed-loop", function()
  local script = start_pre()
  assert_true(script.south > 0 and script.west > 0, "带链几何成立（south=" ..
    script.south .. " west=" .. script.west .. "）")
  -- ── 局 A：0..k（start_pre 已回放），存档，续跑 k..N ──────────────────────
  local anchors_a_pre = collect_anchors()
  local hash_a_at_k = procgen.hash(simtick.snapshot())
  savegame.save(SAVE_PATH)
  local text_a = ygg.fs.read(SAVE_PATH)
  play(script.post)
  local anchors_a = collect_anchors()
  local final_a = codec.encode(simtick.snapshot())
  -- ── 局 B：fresh KV → load → 同日志续跑 k..N ─────────────────────────────
  reset_all()
  local text_b = ygg.fs.read(SAVE_PATH)
  assert_eq(text_b, text_a, "⑤ 传输零损：B 读到的文件字节与 A 写下的一致")
  local receipt = savegame.load(SAVE_PATH)
  assert_true(receipt.bytes_equal, "复导出逐字节对账")
  assert_true(receipt.snap_checked, "锚拍存档走了快照哈希硬校")
  assert_eq(procgen.hash(simtick.snapshot()), hash_a_at_k, "① B@k 快照哈希 == A@k")
  -- 锚键回写：档内锚（≤k）在 B 的 KV 逐值还原
  local anchors_b_loaded = collect_anchors()
  assert_eq(#anchors_b_loaded, #anchors_a_pre, "载入后锚键数 == 存档时锚键数")
  for i = 1, #anchors_a_pre do
    assert_eq(anchors_b_loaded[i].t, anchors_a_pre[i].t, "锚 t 还原")
    assert_eq(anchors_b_loaded[i].hash, anchors_a_pre[i].hash, "锚哈希还原")
  end
  play(script.post)
  local anchors_b = collect_anchors()
  local final_b = codec.encode(simtick.snapshot())
  -- ── 局 C：fresh KV 不中断 0..N ──────────────────────────────────────────
  reset_all()
  local whole = {}
  for i = 1, #script.pre do
    whole[#whole + 1] = script.pre[i]
  end
  for i = 1, #script.post do
    whole[#whole + 1] = script.post[i]
  end
  play(whole)
  local anchors_c = collect_anchors()
  local final_c = codec.encode(simtick.snapshot())
  -- ── 判据 ②③④ ────────────────────────────────────────────────────────────
  assert_eq(final_b, final_a, "③ B 终态快照与 A 逐字节相等")
  assert_eq(final_c, final_a, "④ C 终态快照与 A 逐字节相等")
  local function at(anchors, t)
    for i = 1, #anchors do
      if anchors[i].t == t then
        return anchors[i].hash
      end
    end
    return nil
  end
  for i = 1, #anchors_c do
    local t = anchors_c[i].t
    assert_eq(at(anchors_a, t), anchors_c[i].hash, "④ A≡C 全锚 t=" .. t)
    if t >= K then
      assert_eq(at(anchors_b, t), anchors_c[i].hash, "④ B≡C 于 [k..N] 锚 t=" .. t)
    end
  end
  assert_true(#anchors_c >= 8, "锚量前提（实得 " .. #anchors_c .. "）")
  -- ② B 续跑锚与 A 同拍锚逐值（collect 已含；等价于 ④ 的 [k..N] 段，这里钉 A-B 直比）
  for i = 1, #anchors_b do
    if anchors_b[i].t > K then
      assert_eq(at(anchors_a, anchors_b[i].t), anchors_b[i].hash,
        "② B/A 同拍锚 t=" .. anchors_b[i].t)
    end
  end
end)

check("save.idempotent-import-created-zero", function()
  local script = start_pre()
  savegame.save(SAVE_PATH)
  reset_all()
  local receipt = savegame.load(SAVE_PATH)
  local bundle = codec.read(SAVE_PATH)
  -- 同档再导：全量命中 overwrite，零新建（codec I2 幂等）
  local again = codec.import(bundle, { on_conflict = "overwrite" })
  assert_eq(again.created, 0, "幂等导入 created=0")
  assert_eq(again.overwritten, #bundle.entities, "逐条 overwrite")
  assert_eq(#bundle.entities, receipt.imported.created, "首导 created == 实体数")
end)

check("save.tamper-byte-rejected", function()
  local script = start_pre()
  savegame.save(SAVE_PATH)
  -- 方法：翻转正文一字节（中部字符）
  local text = ygg.fs.read(SAVE_PATH)
  assert_true(#text > 4, "档非空")
  local pos = (#text) // 2
  local flipped
  if text:sub(pos, pos) == "x" then
    flipped = text:sub(1, pos - 1) .. "y" .. text:sub(pos + 1)
  else
    flipped = text:sub(1, pos - 1) .. "x" .. text:sub(pos + 1)
  end
  ygg.fs.write(SAVE_PATH, flipped)
  reset_all()
  assert_error("YGG-PKG-CODEC-CHECKSUM", function()
    savegame.load(SAVE_PATH)
  end, "篡改一字节 → checksum 拒")
end)

check("save.mapgenv-mismatch-rejected", function()
  local script = start_pre()
  savegame.save(SAVE_PATH)
  -- 方法：读档改 sz.mapgen_v 后重新封签（绕过 checksum 面，专测版本闸）
  local bundle = codec.read(SAVE_PATH)
  bundle.kv["sz.mapgen_v"] = mapgen.MAPGEN_V + 1
  bundle.checksum = codec.checksum(bundle)
  ygg.fs.write(SAVE_PATH, codec.encode(bundle))
  reset_all()
  assert_error("YGG-MOD-SAVE-MAPGENV", function()
    savegame.load(SAVE_PATH)
  end, "生成器版本不符即拒（R5'）")
end)

check("save.perf-save-load", function()
  local script = start_pre()
  local clock_ok, clock = pcall(function()
    return os.clock
  end)
  if not clock_ok then
    return "os 不可用，跳过 PERF（selftest 普通 Lua 必有）"
  end
  -- 三次取中位（小档基线，续 S19/S20 表）
  local saves, loads = {}, {}
  for _ = 1, 3 do
    local t0 = clock()
    savegame.save(SAVE_PATH)
    saves[#saves + 1] = (clock() - t0) * 1000
    reset_all()
    local t1 = clock()
    savegame.load(SAVE_PATH)
    loads[#loads + 1] = (clock() - t1) * 1000
  end
  table.sort(saves)
  table.sort(loads)
  print(string.format("PERF shapez.save_ms=%.1f load_ms=%.1f （小档 save+load 预算 ≤50ms）",
    saves[2], loads[2]))
  assert_true(saves[2] + loads[2] < 50, "小档 save+load ≤50ms（§1.7 预算）")
end)
