-- `src/bot.lua` —— v3 自动局压测台（长局 / 高频创销 / 决策流 KAT）。
--
-- # 契约定位（docs/design/mod/shapez-game.md §2.7）
--
-- 计划即规格（r1 rev 1，/tmp/s26c-plan.md §1）：决策源 = 逐拍独立 procgen 流
-- `bot.<tick>`（流隔离 = 无跨拍 rng 状态；流种子 = `sz.seed`）⇒
-- 决策 = f(seed, tick, 世界态)，存读档与重放天然安全，禁缺省熵（R6' 同款）。
--
-- 决策表（加权伪随机序，比例钉 KAT，`rng:weighted` 数组声明序即池序）：
--   ① 带链延伸（活带端点直行/转弯）35 ｜ ② 拆带（从尾端拆）25 ｜
--   ③ 放处理器（miner 斑内 / cutter·stacker·rotater·trash 轮转）30 ｜
--   ④ 存档点标记（纯读快照哈希探针，不落 KV）10。
--
-- 状态纪律（§5 三要素）：bot **无 KV 状态键**——决策序不落日志、不落档，
-- 同 seed 双局重推同决策，逐锚 + 终态快照比对即重放闭环。
--
-- 长局剧本 `run(cfg)`：决策 tick 段（可带周期存读档）→ 拆建循环段（纯
-- spawn/destroy 高水位，压 S23）。闭环模式（`closed`）= 零销毁轨：demolish
-- 全程空转、首存拍（含）前 extend 也空转 ⇒ 每个存拍世界都零带零洞（id 致密），
-- save→load 续跑局的致密重排与原局逐位相等（importing「保 id」语义 = 致密
-- 重排，洞位 id 不在存档契约内——test-f「k 拍前零传送带」约束的 bot 版）。
-- 拆建高频创销压力由非闭环轨（churn 段 + 无存档双局重放）承担。
--
-- 探针纪律：生产沙箱 deny `os`/`collectgarbage`——本模块**零引用**，计时与
-- 内存读数由调用面经 `cfg.clock` / `cfg.gc` 注入（selftest 注 os.clock 与
-- collectgarbage；缺省空探针，生产装载不受影响）。
--
-- 沙箱方言：模块顶层纯；ygg 经 host() 单点取。

local procgen = require("pkg.procgen")
local entity = require("pkg.entity")
local belts = require("src.belts")
local processors = require("src.processors")
local simtick = require("src.simtick")
local mapgen = require("src.mapgen")
local savegame = require("src.savegame")

local M = { VERSION = "1.0.0" }

local function host()
  return ygg
end

-- ============================================================================
-- 决策表（纯抽取面：决策 = 第 1 抽，目标 = 同流后续抽）
-- ============================================================================

--- 决策池：数组声明序即池序（R3'：pairs 无序禁用）。权重和 100，便于 KAT 读数。
local ACTIONS = { "extend", "demolish", "place", "savepoint" }
local WEIGHTS = { 35, 25, 30, 10 }

--- 处理器轮转序（place 决策的子抽取面）。
local PLACE_KINDS = { "miner", "cutter", "stacker", "rotater", "trash" }

--- `decide(seed, tick)` → 动作名。纯函数：逐拍独立流 `bot.<tick>` 第 1 抽。
--- 同 (seed, tick) 跨进程逐值相等——决策序 KAT 的直接对象。
function M.decide(seed, tick)
  local rng = procgen.rng(seed, "bot." .. tick)
  return ACTIONS[rng:weighted(WEIGHTS)]
end

-- ============================================================================
-- 世界目标面（同流后续抽 + 世界态派生；全部升序扫描，确定性）
-- ============================================================================

--- 升序 id 数组（entity.ids 本就升序；schema nil = 全体）。
local function ids_of(schema)
  return entity.ids(nil, schema)
end

--- 活带路径的升序 startid 数组（belts.ensure 的键序不定向，显式排序）。
local function path_starts()
  local paths = belts.ensure()
  local starts = {}
  for startid in pairs(paths) do
    starts[#starts + 1] = startid
  end
  for i = 1, #starts do
    local min = i
    for j = i + 1, #starts do
      if starts[j] < starts[min] then
        min = j
      end
    end
    if min ~= i then
      starts[i], starts[min] = starts[min], starts[i]
    end
  end
  return starts, paths
end

local function cell_free(x, y)
  if entity.count() == 0 then
    return true
  end
  for _, schema in ipairs({ "sz.belt", "sz.processor", "sz.miner", "sz.hub" }) do
    local list = ids_of(schema)
    for i = 1, #list do
      local id = list[i]
      if entity.get(id, "x") == x and entity.get(id, "y") == y then
        return false
      end
    end
  end
  return true
end

--- 首带种子格（无带世界的确定性落格；远离原点探针区，预闭段不涉足）。
local SEED_CELLS = { { 50, 50 }, { 51, 50 }, { 52, 50 }, { 53, 50 } }

--- 带链延伸：无带时落首带种子；有带时抽一条活路径 → 端点直行/转弯落一格。
--- 返回 id | nil。
local function do_extend(rng, cap)
  if entity.count() >= cap then
    return nil
  end
  local starts, paths = path_starts()
  if #starts == 0 then
    for i = 1, #SEED_CELLS do
      local cell = SEED_CELLS[i]
      if cell_free(cell[1], cell[2]) then
        local rot = rng:int(0, 3)
        local id = entity.spawn("sz.belt", { x = cell[1], y = cell[2], rot = rot }, { belt = true })
        belts.bump_layout()
        return id
      end
    end
    return nil
  end
  local startid = starts[rng:int(1, #starts)]
  local path = paths[startid]
  local n = #path.belts
  local tail = path.belts[n]
  local rot = entity.get(tail, "rot")
  local tx = entity.get(tail, "x")
  local ty = entity.get(tail, "y")
  -- 直行 0 / 左转 1 / 右转 3（rot 0=+x 1=+y 2=-x 3=-y）
  local turn = rng:int(0, 2)
  local nrot = (rot + ({ [0] = 0, [1] = 3, [2] = 1 })[turn]) % 4
  local nx = tx + belts.DIRX[nrot]
  local ny = ty + belts.DIRY[nrot]
  if not cell_free(nx, ny) then
    return nil
  end
  local id = entity.spawn("sz.belt", { x = nx, y = ny, rot = nrot }, { belt = true })
  belts.bump_layout()
  return id
end

--- 拆带：抽一条活路径 → 尾端带拆除（entity.destroy + 布局版本 bump）。
--- 返回 id | nil。失效带（`sz.path.<startid>` 余键）是 S23 巡检对象。
local function do_demolish(rng)
  local starts, paths = path_starts()
  if #starts == 0 then
    return nil
  end
  local startid = starts[rng:int(1, #starts)]
  local path = paths[startid]
  local tail = path.belts[#path.belts]
  if not entity.destroy(tail) then
    return nil
  end
  belts.bump_layout()
  return tail
end

--- 放处理器：kind 轮转抽 → 活带邻格（miner 须斑内）；空世界抽原点邻格。
--- 返回 id | nil。
local function do_place(rng, cap)
  if entity.count() >= cap then
    return nil
  end
  local kind = PLACE_KINDS[rng:int(1, #PLACE_KINDS)]
  -- 锚格：活带端点 / 无带时原点
  local ax, ay = 0, 0
  local starts, paths = path_starts()
  if #starts > 0 then
    local path = paths[starts[rng:int(1, #starts)]]
    local tail = path.belts[#path.belts]
    ax = entity.get(tail, "x")
    ay = entity.get(tail, "y")
  end
  local dir = rng:int(0, 3)
  local px = ax + belts.DIRX[dir]
  local py = ay + belts.DIRY[dir]
  if not cell_free(px, py) then
    return nil
  end
  if kind == "miner" then
    local patch = mapgen.patch_at(px, py)
    if patch == nil then
      return nil
    end
    return processors.place_miner(px, py, dir, patch)
  end
  return processors.place(kind, px, py, dir)
end

-- ============================================================================
-- 拆建循环段（纯 spawn/destroy 高水位；不 tick——tick 热路径的水位代价
-- 由主段实测见巡检记录，此处压 S23 的 KV 墓碑与实体账面）
-- ============================================================================

local CHURN_BASE = 1000 -- 远离 bot 世界区（决策目标都在锚格邻域）

--- `churn(rounds, gc)` → { rounds, spawned }。格序 = 100×10 网格行扫，确定性。
function M.churn(rounds, gc)
  local probe = gc or function()
    return -1
  end
  local before = probe()
  local spawned = 0
  for r = 0, rounds - 1 do
    local x = CHURN_BASE + (r % 100)
    local y = CHURN_BASE + ((r // 100) % 10)
    local id = entity.spawn("sz.belt", { x = x, y = y, rot = r % 4 }, { belt = true })
    spawned = spawned + 1
    entity.destroy(id)
  end
  return { rounds = rounds, spawned = spawned, mem_before = before, mem_after = probe() }
end

-- ============================================================================
-- 长局剧本
-- ============================================================================

local function asc_rank(n, p)
  local r = (n * p) // 1
  if r < 1 then
    r = 1
  end
  if r > n then
    r = n
  end
  return r
end

--- 百分位（升序副本 + 线性选择下标；`p` ∈ (0,1]）。空表返回 nil。
function M.pctl(list, p)
  if type(list) ~= "table" or #list == 0 then
    return nil
  end
  local sorted = {}
  for i = 1, #list do
    sorted[i] = list[i]
  end
  for i = 1, #sorted do
    local min = i
    for j = i + 1, #sorted do
      if sorted[j] < sorted[min] then
        min = j
      end
    end
    if min ~= i then
      sorted[i], sorted[min] = sorted[min], sorted[i]
    end
  end
  return sorted[asc_rank(#sorted, p)]
end

--- `run(cfg)` → 长局报告。
---
--- cfg 字段：
---   seed        种子串（必填；= event.start 载荷口径）
---   ticks       决策 tick 段拍数（默认 0）
---   churn       拆建循环段轮数（默认 0）
---   closed      闭环模式：首 save 拍前 extend/demolish 空转（默认 false）
---   save_every  每 N 拍 save→reset→load→续跑（0 = 关；需 reset/save_path）
---   save_path   存档路径前缀（save_every > 0 时必填；实存 = `<prefix>.<世界拍>`
---               ——逐拍可寻址，续跑局按拍回取）
---   reset       重置回调（`reset_all()` 同款：清 KV/实体，留 handler 与文件）
---   caps        { belts, procs, miners } 稳态上限（防 tick 热路径稳态膨胀）
---   clock       毫秒探针（缺省空）
---   gc          KB 探针（缺省空；探针内部自collect，见 selftest 注入）
---   profile     true = 逐拍耗时采样（毫秒数组进报告）
---
--- 报告：actions（抽到/执行到双计）· anchors · marks（存档点探针哈希）·
--- saves（次数与毫秒）· churn · mem 分相 · tick 毫秒分布 · final_hash。
function M.run(cfg)
  local h = host()
  local clock = cfg.clock or function()
    return 0
  end
  local gc = cfg.gc or function()
    return -1
  end
  local caps = cfg.caps or { belts = 12, procs = 6, miners = 2 }
  local seed = assert(cfg.seed, "bot.run 需要 seed")
  local ticks = cfg.ticks or 0
  local save_every = cfg.save_every or 0
  if save_every > 0 then
    assert(type(cfg.reset) == "function", "bot.run 周期存读档需要 cfg.reset")
    assert(type(cfg.save_path) == "string", "bot.run 周期存读档需要 cfg.save_path")
  end

  local report = {
    seed = seed,
    ticks = ticks,
    actions = { extend = 0, demolish = 0, place = 0, savepoint = 0,
      noop = 0, extend_hit = 0, demolish_hit = 0, place_hit = 0 },
    anchors = {}, -- { {tick, hash} }（500 拍锚，simtick 落 KV 同款）
    marks = {},   -- { {tick, hash} }（存档点探针，纯读不落 KV）
    saves = { count = 0, save_ms = 0, load_ms = 0 },
    mem = {},
    tick_ms = cfg.profile and {} or nil,
  }
  local world = h.data("ygg:shapez/world")
  simtick.setup(world, seed)
  -- 流名与周期存档节拍一律用**世界拍**（KV sz.tick）：续跑局载入后从 k+1
  -- 继续，与原局同流同决策（闭环重放的序钟）。
  local t0 = h.kv.get("sz.tick") or 0
  report.mem.build = gc()
  local exec_cap = caps.belts + caps.procs + caps.miners + 8

  for t = 1, ticks do
    local wt = t0 + t
    local rng = procgen.rng(seed, "bot." .. wt)
    local action = ACTIONS[rng:weighted(WEIGHTS)]
    report.actions[action] = report.actions[action] + 1
    local pre_closed = cfg.closed and save_every > 0 and wt <= save_every
    local hit = nil
    if action == "extend" then
      if not pre_closed and entity.count() < exec_cap then
        hit = do_extend(rng, exec_cap)
        if hit then
          report.actions.extend_hit = report.actions.extend_hit + 1
        end
      end
    elseif action == "demolish" then
      if not cfg.closed then -- 闭环轨零销毁：洞位 id 不在存读档契约内（保 id=致密重排）
        hit = do_demolish(rng)
        if hit then
          report.actions.demolish_hit = report.actions.demolish_hit + 1
        end
      end
    elseif action == "place" then
      if entity.count() < exec_cap then
        hit = do_place(rng, exec_cap)
        if hit then
          report.actions.place_hit = report.actions.place_hit + 1
        end
      end
    else -- savepoint：纯读快照探针（无 KV 写）
      report.marks[#report.marks + 1] = { tick = wt, hash = procgen.hash(simtick.snapshot()) }
    end
    if hit == nil and action ~= "savepoint" then
      report.actions.noop = report.actions.noop + 1
    end
    local t0c = clock()
    local tr = simtick.tick()
    local t1c = clock()
    if tr.anchor ~= nil then
      report.anchors[#report.anchors + 1] = { tick = tr.tick, hash = tr.anchor }
    end
    if cfg.profile then
      report.tick_ms[#report.tick_ms + 1] = t1c - t0c
    end
    -- 周期存读档（save→reset→load→续跑；末拍不存——拍后无续跑可言）。
    -- 进程内缓存连续性注记见契约 §2.7：path 态不进档是 v2 既定登记，
    -- 闭环等价的硬判据在首存拍前的零带零洞剧本（closed）。
    if save_every > 0 and wt % save_every == 0 and wt < t0 + ticks then
      local s0 = clock()
      savegame.save(cfg.save_path .. "." .. wt)
      local s1 = clock()
      cfg.reset()
      savegame.load(cfg.save_path .. "." .. wt)
      local s2 = clock()
      report.saves.count = report.saves.count + 1
      report.saves.save_ms = report.saves.save_ms + (s1 - s0)
      report.saves.load_ms = report.saves.load_ms + (s2 - s1)
      report.mem[#report.mem + 1] = { at = wt, phase = "saveload", kb = gc() }
    end
  end

  report.mem.run = gc()
  if cfg.churn and cfg.churn > 0 then
    report.churn = M.churn(cfg.churn, gc)
  end
  report.mem.final = gc()
  report.final_hash = procgen.hash(simtick.snapshot())
  return report
end

return M
