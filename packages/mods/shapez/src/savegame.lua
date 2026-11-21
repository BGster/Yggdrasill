-- `src/savegame.lua` —— 存档信封（P4 superagent 信封先例）。
--
-- # 契约定位（docs/design/mod/shapez-game.md §2.6）
--
-- 信封 = `codec.export(ns)`，`ns={entities=true, ids="index", keys=点名键集, mod="shapez"}`。
-- 点名键三族（D9 显式点名；ygg.kv 无遍历面 → 动态键全部**构造性枚举**）：
--   ① 静态键 sz.tick/level/got/layout_v/setup/hub_id/level_quest/seed/mapgen_v；
--   ② 锚键 `sz.anchor.<t>`，t = anchorEveryTicks·j ≤ sz.tick（算术枚举）；
--   ③ `quest.keys()` 自报清单。
-- `sz.path.*`（belts.ensure 可重建）、`sz.chunk.*`（seed 唯真源，惰性重生成）
-- **不进档**——存档即源 BaseMap `{seed}` 同构的最小面。
--
-- 协议：`save(path)` = codec.write（封签）；`load(path)` = codec.read（强校）
-- → `codec.import(bundle,{on_conflict="overwrite"})`（保 id：空局升序导入
-- 致密分配自然相等）→ **kv 段由 mod 逐键回写**（codec D9：包不写回；quest.*
-- 键回写 = 逐字节还原 quest 包的 KV 投影，对账兜底）→ 三重对账：复导出
-- 逐字节 + 实体计数 + 锚点快照哈希（锚拍存档时硬校）。
-- 误载脏 KV 防护：load 前见 `sz.setup` 已在场即拒。
--
-- 沙箱方言：模块顶层纯；ygg 经 host() 单点取。

local codec = require("pkg.codec")
local procgen = require("pkg.procgen")
local entity = require("pkg.entity")
local quest = require("pkg.quest")
local simtick = require("src.simtick")
local mapgen = require("src.mapgen")

local M = { VERSION = "1.0.0" }

--- 信封来源署名（exported_from；读回强校）。
M.MOD = "shapez"

local DATA_ID = "ygg:shapez/world"

--- ① 静态键（点名；nil 键由 codec.export 跳过）。
local STATIC_KEYS = {
  "sz.tick", "sz.level", "sz.got", "sz.layout_v", "sz.setup",
  "sz.hub_id", "sz.level_quest", "sz.seed", "sz.mapgen_v",
}

local function host()
  return ygg
end

local function fail(code, detail)
  error("YGG-MOD-SAVE-" .. code .. ": " .. detail, 0)
end

--- 点名键集三族的构造性枚举（升序去重由调用方 codec.export 的 encode 规范化兜底）。
function M.named_keys()
  local h = host()
  local keys = {}
  local seen = {}
  local function add(key)
    if type(key) == "string" and seen[key] == nil then
      seen[key] = true
      keys[#keys + 1] = key
    end
  end
  for i = 1, #STATIC_KEYS do
    add(STATIC_KEYS[i])
  end
  -- ② 锚键算术枚举：t = every·j ≤ tick（simtick.anchor 同一落点集）。
  local world = simtick.WORLD
  local tuning = world ~= nil and world.tuning or nil
  local every = tuning ~= nil and tuning.anchorEveryTicks or nil
  local tick = h.kv.get("sz.tick") or 0
  if type(every) == "number" and every > 0 then
    for t = every, tick, every do
      add("sz.anchor." .. t)
    end
  end
  -- ③ quest 自报清单（quest 包唯一权威枚举面）。
  local quest_keys = quest.keys()
  for i = 1, #quest_keys do
    add(quest_keys[i])
  end
  return keys
end

local function namespace()
  return { entities = true, ids = "index", keys = M.named_keys(), mod = M.MOD }
end

--- `save(path)` → `{bytes, sha256}`（codec.write 封签落盘）。未开局拒绝。
function M.save(path)
  if host().kv.get("sz.setup") == nil then
    fail("STATE", "未开局（sz.setup 缺席）——先 event.start 再存档")
  end
  return codec.write(path, codec.export(namespace()))
end

--- `load(path)` → 对账回执。强校读回 + import（overwrite，保 id）+ kv 逐键
--- 回写 + 三重对账。要求当前 KV 是空局（`sz.setup` 缺席）。
function M.load(path)
  local h = host()
  if h.kv.get("sz.setup") ~= nil then
    fail("STATE", "当前 KV 已开局（sz.setup 在场）——拒绝覆盖载入（防误载脏 KV）")
  end
  local bundle = codec.read(path)
  if bundle.exported_from ~= M.MOD then
    fail("MOD", "存档来源不符：" .. tostring(bundle.exported_from))
  end
  local kv = bundle.kv
  if type(kv) ~= "table" then
    fail("SHAPE", "存档缺 kv 段")
  end
  if kv["sz.mapgen_v"] ~= mapgen.MAPGEN_V then
    fail("MAPGENV",
      "生成器版本不符：档 " .. tostring(kv["sz.mapgen_v"]) .. " / 现 " .. tostring(mapgen.MAPGEN_V))
  end
  if kv["sz.seed"] == nil then
    fail("SEED", "存档缺 sz.seed（R6 禁缺省熵）")
  end
  -- schema 先行注册（import 校验需要 spec；不落 sz.setup——它由档内回写）。
  simtick.register_schemas()
  local report = codec.import(bundle, { on_conflict = "overwrite" })
  for key, value in pairs(kv) do
    h.kv.set(key, value)
  end
  -- 运行时常量重挂（snapshot/常量面；hub.setup 幂等早退）。
  local world = h.data(DATA_ID)
  simtick.mount(world)
  -- ── 三重对账（superagent items reconcile 同款）───────────────────────────
  -- checksum 域归一：read 回的 bundle 带 digest，新 export 为空串——归一后逐字节比
  local again = codec.export(namespace())
  again.checksum = bundle.checksum
  if codec.encode(again) ~= codec.encode(bundle) then
    fail("RECONCILE", "复导出与档 codec.encode 逐字节不等")
  end
  local entity_count = entity.count()
  if entity_count ~= #bundle.entities then
    fail("RECONCILE", "实体计数不符：" .. tostring(entity_count) .. " / 档 " .. tostring(#bundle.entities))
  end
  local receipt = {
    imported = report,
    bytes_equal = true,
    snap_checked = false,
    snap_hash = nil,
  }
  local tick = h.kv.get("sz.tick") or 0
  local every = world.tuning.anchorEveryTicks
  if type(every) == "number" and every > 0 and tick > 0 and tick % every == 0 then
    -- 锚拍存档：读回态必须与落档锚逐值相等（simtick.anchor 同一 hash 面）。
    local anchor = h.kv.get("sz.anchor." .. tick)
    if anchor == nil then
      fail("RECONCILE", "锚拍存档缺锚键 sz.anchor." .. tostring(tick))
    end
    local digest = procgen.hash(simtick.snapshot())
    receipt.snap_checked = true
    receipt.snap_hash = digest
    if anchor ~= digest then
      fail("RECONCILE", "锚点快照哈希不符：锚 " .. tostring(anchor) .. " / 现 " .. tostring(digest))
    end
  end
  return receipt
end

return M
