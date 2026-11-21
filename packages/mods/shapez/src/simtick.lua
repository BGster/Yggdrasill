-- `src/simtick.lua` —— tick 信封：一拍 = 矿机/处理器/传送带 + sim.step + 锚定。
--
-- # 契约定位（docs/design/mod/shapez-game.md §6）
--
-- 源 game_root tick 循环 + dynamic_tickrate 的 v1 等价面：
-- 一个 tick = 1/60s（世界 tuning 的 ticksPerSecond）。推进序（确定性铁律）：
--   ① 矿机/处理器（出料 → 投产/推进）
--   ② 传送带（路径推进 + 末端弹射到落点）
-- 落点裁决（resolve）按格查：belt → 邻路收件口；hub → 交付；processor/miner
-- → 入料槽；trash → 吞。同一格多实体时取 id 最小者（确定性）。
-- 每 N tick 落一枚锚：`sz.anchor.<tick> = procgen.hash(snapshot())`（I3 重放对账）。
--
-- 沙箱方言：模块顶层纯；ygg 经 host() 单点取。

local procgen = require("pkg.procgen")
local entity = require("pkg.entity")
local belts = require("src.belts")
local processors = require("src.processors")
local hub = require("src.hub")
local mapgen = require("src.mapgen")

local M = { VERSION = "1.0.0" }

local TICK_KEY = "sz.tick"

local function host()
  return ygg
end

-- ============================================================================
-- 装配（事件入口调；重复调用幂等）
-- ============================================================================

local SCHEMAS = {
  ["sz.belt"] = {
    attrs = {
      x = { type = "number", required = true },
      y = { type = "number", required = true },
      rot = { type = "number", required = true },
    },
    tags = { belt = {} },
  },
  ["sz.processor"] = {
    attrs = {
      x = { type = "number", required = true },
      y = { type = "number", required = true },
      rot = { type = "number", required = true },
      kind = { type = "string", required = true },
      in1 = { type = "string", default = "" },
      in2 = { type = "string", default = "" },
      progress = { type = "number", default = 0 },
      out1 = { type = "string", default = "" },
      out2 = { type = "string", default = "" },
    },
    tags = { processor = {}, cutter = {}, stacker = {}, rotater = {}, trash = {} },
  },
  ["sz.hub"] = {
    attrs = {
      x = { type = "number", required = true },
      y = { type = "number", required = true },
      rot = { type = "number", required = true },
    },
    tags = { hub = {} },
  },
  ["sz.miner"] = {
    attrs = {
      x = { type = "number", required = true },
      y = { type = "number", required = true },
      rot = { type = "number", required = true },
      shape = { type = "string", required = true },
      progress = { type = "number", default = 0 },
      out1 = { type = "string", default = "" },
    },
    tags = { miner = {} },
  },
}

--- `register_schemas()`：schema 注册段（load 复用面：只 define，不落
--- `sz.setup`——它由存档回写）。`entity.define` 同 spec 幂等零写。
function M.register_schemas()
  for name, spec in pairs(SCHEMAS) do
    entity.define(name, spec)
  end
end

--- `mount(world)`：运行时常量挂载（setup 与 savegame.load 共用）。
function M.mount(world)
  M.C = belts.constants(world.tuning)
  M.WORLD = world
  M.CYCLE = world.tuning.processorCycle
  M.C.cycle_of = function(kind)
    return M.CYCLE[kind] or 0
  end
end

--- `setup(world, seed?)`：schema 注册 + 种子守护 + 常量 + hub 装配。幂等。
--- `seed` = event.start 载荷串（首局落 `sz.seed`；first-wins，读档局以档为准）。
function M.setup(world, seed)
  local h = host()
  if h.kv.get("sz.setup") == nil then
    M.register_schemas()
    h.kv.set("sz.setup", true)
  end
  mapgen.register()
  mapgen.ensure_seed(seed)
  M.mount(world)
  hub.setup(world)
end

-- ============================================================================
-- 落点裁决（belt 末端 / 处理器出料共用）
-- ============================================================================

--- `resolve(fx, fy)` → fn(item_key) → boolean | nil。
--- 同格多实体取 id 最小者；空项只进 trash / 邻带（hub 不收空项）。
function M.resolve(fx, fy)
  -- belt：邻路收件口
  local belt_ids = entity.ids(nil, "sz.belt")
  local belt_id = nil
  for i = 1, #belt_ids do
    local id = belt_ids[i]
    if entity.get(id, "x") == fx and entity.get(id, "y") == fy then
      if belt_id == nil or id < belt_id then
        belt_id = id
      end
    end
  end

  -- hub
  local hid = hub.hub_id()
  local hub_here = hid ~= nil and entity.get(hid, "x") == fx and entity.get(hid, "y") == fy

  -- processor / miner / trash（入料口）
  local proc_ids = entity.ids(nil, "sz.processor")
  local proc_id = nil
  for i = 1, #proc_ids do
    local id = proc_ids[i]
    if entity.get(id, "x") == fx and entity.get(id, "y") == fy then
      if proc_id == nil or id < proc_id then
        proc_id = id
      end
    end
  end
  local miner_ids = entity.ids(nil, "sz.miner")
  local miner_id = nil
  for i = 1, #miner_ids do
    local id = miner_ids[i]
    if entity.get(id, "x") == fx and entity.get(id, "y") == fy then
      if miner_id == nil or id < miner_id then
        miner_id = id
      end
    end
  end

  return function(item_key)
    if belt_id ~= nil then
      local paths = belts.ensure()
      -- 该带所属路径的收件口 = 路径起点（路径首带 = start）
      for startid in pairs(paths) do
        local path = paths[startid]
        for k = 1, #path.belts do
          if path.belts[k] == belt_id then
            return belts.try_accept(path, item_key)
          end
        end
      end
      return false
    end
    if hub_here then
      return hub.deliver(M.WORLD, item_key).accepted
    end
    if proc_id ~= nil and processors.input_free(proc_id) then
      return processors.accept_input(proc_id, item_key)
    end
    if miner_id ~= nil and entity.get(miner_id, "out1") == "" then
      -- 矿机不收外料（源同口径）；占位拒绝
      return false
    end
    return false
  end
end

-- ============================================================================
-- 一拍
-- ============================================================================

--- `tick()` → { tick, hash, anchors }：走一拍。降级诚实账沿 superagent clock。
function M.tick()
  local h = host()
  processors.tick(M.C, M.resolve)
  belts.tick(M.C, M.resolve)
  local tick = (h.kv.get(TICK_KEY) or 0) + 1
  h.kv.set(TICK_KEY, tick)
  local ok, err = pcall(function()
    h.sim.step(1)
  end)
  local degraded = not ok
  local anchors = nil
  local every = M.WORLD.tuning.anchorEveryTicks
  if tick % every == 0 then
    anchors = M.anchor(tick)
  end
  if degraded and tick == 1 then
    h.narrate("[SIM-UNMOUNTED] shapez tick 降级：本步未进模拟内核（宿主未挂 sim），KV 计拍照走；宿主面细节：" .. tostring(err))
  end
  return { tick = tick, degraded = degraded, anchor = anchors }
end

--- `anchor(tick)`：落一枚重放锚（`sz.anchor.<tick>` = 状态哈希）。返回哈希。
function M.anchor(tick)
  local h = host()
  local digest = procgen.hash(M.snapshot())
  h.kv.set("sz.anchor." .. tick, digest)
  return digest
end

--- `snapshot()` → 全局状态表（重放哈希输入；纯读、确定性序）。
function M.snapshot()
  return {
    tick = host().kv.get(TICK_KEY) or 0,
    hub = hub.snapshot(),
    belts = belts.snapshot(),
    procs = processors.snapshot(),
  }
end

return M
