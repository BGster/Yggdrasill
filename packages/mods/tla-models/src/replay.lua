-- `src/replay.lua` —— 轨道驱动：哈希链 / KV 锚 / 双局对账（S26-4 黄金轨道面）。
--
-- 契约：docs/design/mod/tla-models.md §轨道驱动。轨道驱动 =
-- `reset(model)` 建规范初态 → `tick(model, action)` 执行一次 TLA 动作
-- （\E 非确定性显式化为动作选择输入；stutter = 无动作的显式步型）→
-- 每拍 `procgen.hash(snapshot())` 落哈希链、每 N 拍落 KV 锚
-- `tla.anchor.<model>.<tick>`（shapez v1 同法 I3）。
--
-- 哈希口径（KAT 钉死）：`hash(tick) = procgen.hash(snapshot())`——纯状态指纹，
-- 不掺 tick/前值（故 stutter 步哈希不变，[][Next]_v 的 v'=v 分支可对账）。
-- 「链」= 逐拍指纹序列；前向掺链会让 stutter 锚漂移，弃。
--
-- KV 无删除面（ygg-core 纪律）：reset 只重置 `tla.tick.<m>` 并覆盖写，更短
-- 轨道的旧 tick 键留在 KV 但**不入对账读面**（verify 只读 tick ≤ 当前）。
--
-- 双局对账（I3）：同轨 reset 后跑两遍，逐锚相等 + 终态快照逐字节相等
-- （快照表深相等 + codec.encode 字节相等双断言）。

local procgen = require("pkg.procgen")
local codec = require("pkg.codec")
local inv = require("src.inv")

local M = { VERSION = "0.1.0" }

--- 锚间隔：每 4 拍落一个 KV 锚（shapez v1 同量级）。
M.ANCHOR_EVERY = 4

--- 沙箱方言：模块顶层纯；ygg 经 host() 单点取（shapez simtick 同款）。
local function host()
  return ygg
end

-- ── 内部工具（沙箱方言：无 table 库，手写）──────────────────────────────────

--- 快照表深相等（数组与映射同规：键集相等且逐值相等；值限标量/表）。
local function deep_eq(a, b)
  if a == b then
    return true
  end
  if type(a) ~= "table" or type(b) ~= "table" then
    return false
  end
  for key, value in pairs(a) do
    if not deep_eq(value, b[key]) then
      return false
    end
  end
  for key in pairs(b) do
    if a[key] == nil then
      return false
    end
  end
  return true
end

--- 快照表 → 规范字节（codec.encode：键序 = 字节序，I1 确定性）。
local function canon(snap)
  return codec.encode(snap)
end

-- ── 装配：模型注册表 ─────────────────────────────────────────────────────────
--
-- 每个模型模块暴露统一面（契约 §API）：
--   name       模型名（KV 前缀 / 事件载荷词）
--   install()  schema 注册 + 不变量注册（幂等）
--   reset()    钉规范初态（清实体重建 / KV 覆写）
--   step(action)  执行一个 TLA 动作（守卫不满足报 `YGG-MOD-TLA-GUARD`）
--   snapshot() 规范快照表（纯状态变量，键名固定）
--   auto_actions() 当前状态下启用的动作串数组（升序；stutter 恒在）

local MODELS = {}

--- 注册一个模型模块（mod.lua 装配期调用；同名覆盖）。
function M.register(mod)
  if type(mod) ~= "table" or type(mod.name) ~= "string" then
    error("YGG-MOD-TLA-REPLAY: register 需要带 name 的模型模块表", 0)
  end
  MODELS[mod.name] = mod
  return true
end

--- 已注册模型名（升序）。
function M.model_names()
  local out = {}
  for name in pairs(MODELS) do
    out[#out + 1] = name
  end
  for i = 1, #out do
    local min = i
    for j = i + 1, #out do
      if out[j] < out[min] then
        min = j
      end
    end
    if min ~= i then
      out[i], out[min] = out[min], out[i]
    end
  end
  return out
end

--- 取模型模块（未注册返回 nil）。
function M.model(name)
  return MODELS[name]
end

-- ── 轨道原语 ─────────────────────────────────────────────────────────────────

--- 当前 tick 读（未开轨 = 0）。
local function tick_of(name)
  local value = host().kv.get("tla.tick." .. name)
  if value == nil then
    return 0
  end
  return value
end

--- `reset(model, opts?)`：清 tick / 重写实体与 KV 初态；opts 透传模型
--- （gol 的 pattern 参数走这里）。返回当前快照。
function M.reset(mod, opts)
  mod.install()
  host().kv.set("tla.tick." .. mod.name, 0)
  mod.reset(opts)
  return mod.snapshot()
end

--- `tick(model, action)`：执行一步并落哈希链/锚。action = 动作串
--- （nil = stutter：显式步型，状态不变）。返回 `{ tick, hash }`。
function M.tick(mod, action)
  mod.step(action)
  local tick = tick_of(mod.name) + 1
  local h = host()
  h.kv.set("tla.tick." .. mod.name, tick)
  local hash = procgen.hash(mod.snapshot())
  h.kv.set("tla.hash." .. mod.name .. "." .. tick, hash)
  if tick % M.ANCHOR_EVERY == 0 then
    h.kv.set("tla.anchor." .. mod.name .. "." .. tick, hash)
  end
  return { tick = tick, hash = hash }
end

--- `run_track(model, actions)`：reset 后逐动作推进（action=nil 的洞 =
--- stutter）。返回 `{ ticks, hashes, final }`。
function M.run_track(mod, actions)
  M.reset(mod)
  local ticks = {}
  local hashes = {}
  for i = 1, #actions do
    local result = M.tick(mod, actions[i])
    ticks[#ticks + 1] = result.tick
    hashes[#hashes + 1] = result.hash
  end
  return { ticks = ticks, hashes = hashes, final = mod.snapshot() }
end

--- `auto_tick(model, seed)`：确定性自选步——动作 = f(seed, tick, 状态)。
--- rng = procgen.rng(seed, name.."#"..tick)（命名流派生，同种子同轨同择），
--- 从 `auto_actions()`（升序、stutter 恒在尾）按 `next32 % n` 取一。
--- 非确定性显式化的对偶面：双局同 seed 逐锚必等。
function M.auto_tick(mod, seed)
  local tick = tick_of(mod.name)
  local enabled = mod.auto_actions()
  local index = 1
  if #enabled > 1 then
    local handle = procgen.rng(tostring(seed), mod.name .. "#" .. tick)
    index = (procgen.next32(handle) % #enabled) + 1
  end
  return M.tick(mod, enabled[index])
end

--- `run_auto(model, seed, n)`：连走 n 拍自选步。返回同 run_track。
function M.run_auto(mod, seed, n)
  M.reset(mod)
  local ticks = {}
  local hashes = {}
  for _ = 1, n do
    local result = M.auto_tick(mod, seed)
    ticks[#ticks + 1] = result.tick
    hashes[#hashes + 1] = result.hash
  end
  return { ticks = ticks, hashes = hashes, final = mod.snapshot() }
end

-- ── 对账 ─────────────────────────────────────────────────────────────────────

--- `verify_track(model, actions, golden)`：双局重放对账 + 黄金值对照。
--- golden = `{ hashes?, anchors?, final? }`（final 为快照表或 nil）。
--- 返回违例明细字符串数组（空 = 绿）；任何一锚一哈希一字节不等都点名。
--- 本函数同时是红例②的执行面：golden 被篡改一字节即非空。
function M.verify_track(mod, actions, golden)
  local violations = {}
  local run_a = M.run_track(mod, actions)
  local run_b = M.run_track(mod, actions)
  if not deep_eq(run_a, run_b) then
    violations[#violations + 1] = "dual-run: 两局 ticks/hashes/final 不等"
  end
  for i = 1, #run_a.hashes do
    if golden.hashes ~= nil then
      local expect = golden.hashes[i]
      if expect == nil then
        violations[#violations + 1] = "golden.hashes[" .. i .. "]: 缺项"
      elseif expect ~= run_a.hashes[i] then
        violations[#violations + 1] = "hashes[" .. i
          .. "]: 实得 " .. run_a.hashes[i] .. " 期望 " .. expect
      end
    end
    if run_a.ticks[i] % M.ANCHOR_EVERY == 0 then
      local anchor = host().kv.get("tla.anchor." .. mod.name .. "." .. run_a.ticks[i])
      if anchor ~= run_a.hashes[i] then
        violations[#violations + 1] = "anchor@" .. run_a.ticks[i] .. ": KV 与链不等"
      end
    end
  end
  if golden.anchors ~= nil then
    for tick, expect in pairs(golden.anchors) do
      local actual = host().kv.get("tla.anchor." .. mod.name .. "." .. tick)
      if actual ~= expect then
        violations[#violations + 1] = "golden.anchor@" .. tick
          .. ": 实得 " .. tostring(actual) .. " 期望 " .. tostring(expect)
      end
    end
  end
  if golden.final ~= nil and not deep_eq(run_a.final, golden.final) then
    violations[#violations + 1] = "final: 终态快照与黄金表不等"
  end
  -- 每局收尾各查一遍不变量（回放全程绿由各模型测试逐拍断言）。
  local findings = inv.check(mod.name)
  for i = 1, #findings do
    violations[#violations + 1] = "inv@" .. mod.name .. ": " .. findings[i]
  end
  return violations
end

--- `tamper_test(model, actions, golden, mutate)`：红例②执行面——对 golden
--- 深拷贝后施加 mutate（`"hash1"` 翻第一拍哈希一字节 / `"final"` 破终态 /
--- `"anchor"` 破锚），断言 verify_track 必报非空。返回违例明细（必非空）。
function M.tamper(mod, actions, golden, mutate)
  local dirty = {}
  for key, value in pairs(golden) do
    dirty[key] = value
  end
  if mutate == "hash1" and golden.hashes ~= nil and golden.hashes[1] ~= nil then
    local flipped = golden.hashes[1]
    local head = flipped:sub(1, 1)
    flipped = (head == "0" and "1" or "0") .. flipped:sub(2)
    dirty.hashes = {}
    for i = 1, #golden.hashes do
      dirty.hashes[i] = (i == 1) and flipped or golden.hashes[i]
    end
  elseif mutate == "final" and golden.final ~= nil then
    local forged = {}
    for key, value in pairs(golden.final) do
      forged[key] = value
    end
    forged["__tampered"] = true
    dirty.final = forged
  elseif mutate == "anchor" and golden.anchors ~= nil then
    dirty.anchors = {}
    for tick, value in pairs(golden.anchors) do
      local flipped = tostring(value)
      local head = flipped:sub(1, 1)
      dirty.anchors[tick] = (head == "a" and "b" or "a") .. flipped:sub(2)
    end
  end
  return M.verify_track(mod, actions, dirty)
end

--- 快照深相等暴露给测试（终态逐字节断言的双半之一）。
function M.snap_eq(a, b)
  return deep_eq(a, b)
end

--- 快照规范字节暴露给测试（双半之二：encode 字节相等）。
function M.snap_bytes(snap)
  return canon(snap)
end

return M
