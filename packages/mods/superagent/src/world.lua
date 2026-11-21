-- `src/world.lua` —— 开局铺场：种子 → procgen 派生 → 批帧进 sim + 实体注册。
--
-- # 铺场三步（每局恒定序，同输入同字节）
--
--   1. 世界种子 = `pkg.procgen.hash(seedText)`（FNV-1a 64 over codec JSON，
--      16 位十六进制）——世界的内容参数（道具摆放）由它派生，同数据同种子；
--   2. 世界规模进 Tier 1：`simframe.spawn_entities(#npcs + #clerks + #items)`
--      → `ygg.sim.spawn_batch`（裸实体批；一个逐实体 sim getter 都没有——批式
--      铁律，R5）。世界规模因此进 sim 状态哈希，重放断言不是常数；
--   3. 人物/道具登记进 KV 实体面：`pkg.entity` schema 化 spawn（属性袋）+
--      `pkg.role_skill` 角色/技能装配。任务实体归 quests.lua（C4），本文件不碰。
--
-- # 降级诚实账
--
-- 裸 KernelFacade（playtest / selftest）没有 sim：spawn_batch 抛
-- `YGG-SIM-UNMOUNTED`。铺场不许静默——叙述 `[SIM-UNMOUNTED]` 一行，KV 面
-- 照常落（游戏仍可玩），返回表里 `sim_spawned = 0` 明示。
--
-- # 纪律
--
-- 模块顶层纯；KV 键前缀 `sg.`；文案全部来自 `ygg.data("ygg:superagent/world")`
-- 装载期快照（本文件不写字面文案）。

local pkg_entity = require("pkg.entity")
local pkg_role = require("pkg.role_skill")
local pkg_procgen = require("pkg.procgen")

local M = { VERSION = "1.0.0" }

local WORLD_ID = "ygg:superagent/world"

-- KV 键（本文件单写者；前缀 sg.）
local K = {
  seeded = "sg.seeded",
  seed = "sg.seed",
  npc = "sg.npc.",       -- sg.npc.<key> = 实体 id
  item = "sg.item.",     -- sg.item.<key> = 实体 id
  npc_keys = "sg.npcs",  -- 换行名表
  item_keys = "sg.items",
  patience = "sg.patience.", -- sg.patience.<npc key> = 剩余耐心
}

local E_SEEDED = "YGG-MOD-WORLD-SEEDED"

local function host()
  return ygg
end

--- 手写排序（沙箱无 table.sort；升序，选择排序，n 小）。
local function sort(list)
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

--- 名表（换行连接）→ 数组。
function M.name_table(text)
  local out = {}
  if type(text) ~= "string" or #text == 0 then
    return out
  end
  for key in (text .. "\n"):gmatch("([^\n]*)\n") do
    if #key > 0 then
      out[#out + 1] = key
    end
  end
  return out
end

--- 连接（手写 join：沙箱无 `table.concat`）。
function M.join(keys, sep)
  local out = ""
  for i = 1, #keys do
    if i > 1 then
      out = out .. sep
    end
    out = out .. tostring(keys[i])
  end
  return out
end

--- 数组 → 换行名表（升序——名表字节序即遍历序，确定性要求升序写入）。
function M.join_names(keys)
  local copy = {}
  for i = 1, #keys do
    copy[i] = keys[i]
  end
  sort(copy)
  return M.join(copy, "\n")
end

-- ── schema 装配（幂等：同 spec 重定义是包侧零写）──────────────────────────

local SCHEMA_NPC = {
  attrs = {
    name = { type = "string" },
    title = { type = "string" },
    kind = { type = "string" },
    briefing = { type = "string" },
  },
}

local SCHEMA_ITEM = {
  attrs = {
    name = { type = "string" },
    category = { type = "string" },
    weight = { type = "number" },
    holder = { type = "ref" },
  },
}

--- `define_all(world)`：schema / 角色 / 技能三张声明表（幂等；全数据驱动——
--- 角色与技能清单来自 world.json 的 `roles` / `skills` 段，本文件不抄写业务名单）。
--- 注意载体：人物/道具的 schema 化实体是 **pkg.entity**（计划 §4 裁决的包面），
--- 不是宿主 ygg.entity（那 14 键无 define/schema）。
function M.define_all(world)
  pkg_entity.define("bureau.npc", SCHEMA_NPC)
  pkg_entity.define("bureau.item", SCHEMA_ITEM)
  for i = 1, #world.roles do
    pkg_role.define(world.roles[i].name, world.roles[i].spec)
  end
  for i = 1, #world.skills do
    pkg_role.skill_define(world.skills[i].name, world.skills[i].spec)
  end
  return true
end

-- ── 铺场主入口 ──────────────────────────────────────────────────────────────

--- `seed()` → 报告表；已铺过（sg.seeded）→ 幂等零写返回既有报告。
---
--- 报告键：`seed / sim_spawned / sim_next_id / npcs / items / degraded`。
function M.seed()
  local h = host()
  if h.kv.get(K.seeded) then
    return {
      seed = h.kv.get(K.seed),
      sim_spawned = tonumber(h.kv.get("sg.sim_spawned") or "0"),
      npcs = M.name_table(h.kv.get(K.npc_keys)),
      items = M.name_table(h.kv.get(K.item_keys)),
      degraded = h.kv.get("sg.sim_degraded") == true,
      replay = true,
    }
  end
  local world = h.data(WORLD_ID)
  M.define_all(world)

  -- ① 种子：内容参数派生（procgen 摘要，16 位十六进制）。
  local seed = pkg_procgen.hash(world.seedText)
  h.kv.set(K.seed, seed)

  -- ② sim 世界归钟单点所有（clock.advance 首拍重建「名册 + 在办」规模）；
  -- 这里只记名册数供开局叙述。降级探测照旧（诚实账）。
  local roster_count = #world.npcs + #world.clerks + #world.items
  local degraded = false
  local sim_ok = pcall(function()
    h.sim.status()
  end)
  if not sim_ok then
    degraded = true
    h.kv.set("sg.sim_degraded", true)
  end

  -- ③ 人物：NPC（委托人 + 文员合一 roster）spawn + 角色授予。
  local npc_keys = {}
  for i = 1, #world.npcs do
    local npc = world.npcs[i]
    local id = pkg_entity.spawn("bureau.npc", {
      name = npc.name,
      title = npc.title,
      kind = npc.kind,
      briefing = npc.briefing,
    })
    h.kv.set(K.npc .. npc.key, id)
    h.kv.set(K.patience .. npc.key, npc.patience)
    pkg_role.grant(id, "bureau.client")
    npc_keys[#npc_keys + 1] = npc.key
  end
  for i = 1, #world.clerks do
    local clerk = world.clerks[i]
    local id = pkg_entity.spawn("bureau.npc", {
      name = clerk.name,
      title = clerk.title,
      kind = clerk.kind,
      briefing = "",
    })
    h.kv.set(K.npc .. clerk.key, id)
    pkg_role.grant(id, "bureau.clerk")
    npc_keys[#npc_keys + 1] = clerk.key
  end
  h.kv.set(K.npc_keys, M.join_names(npc_keys))

  -- ④ 道具：holder 用 ref 指到实体 id；摆放序由种子流派生（Fisher-Yates，
  --    只走包公开面 next32——`M.shuffle` 住 rng 子模块、init 面未导出，
  --    绕子模块会破包 API 面纪律）。
  local rng = pkg_procgen.rng(seed, "items")
  local order = {}
  for i = 1, #world.items do
    order[i] = i
  end
  for i = #order, 2, -1 do
    local j = (pkg_procgen.next32(rng) % i) + 1
    order[i], order[j] = order[j], order[i]
  end
  local item_keys = {}
  for row = 1, #order do
    local item = world.items[order[row]]
    local holder_key = item.holder
    local holder_id = h.kv.get(K.npc .. holder_key)
    local id = pkg_entity.spawn("bureau.item", {
      name = item.name,
      category = item.category,
      weight = item.weight,
      holder = holder_id,
    })
    h.kv.set(K.item .. item.key, id)
    item_keys[#item_keys + 1] = item.key
  end
  h.kv.set(K.item_keys, M.join_names(item_keys))

  h.kv.set(K.seeded, true)
  return {
    seed = seed,
    roster = roster_count,
    npcs = npc_keys,
    items = item_keys,
    degraded = degraded,
  }
end

--- `npc_id(key)` / `item_id(key)` → 实体 id | nil。
function M.npc_id(key)
  return host().kv.get(K.npc .. key)
end

function M.item_id(key)
  return host().kv.get(K.item .. key)
end

--- `npc_keys()` → 升序 key 数组（读名表）。
function M.npc_keys()
  return M.name_table(host().kv.get(K.npc_keys))
end

function M.item_keys()
  return M.name_table(host().kv.get(K.item_keys))
end

--- `patience(key)` → 剩余耐心（number）。
function M.patience(key)
  return host().kv.get(K.patience .. key)
end

--- `spend_patience(key, cost)` → 花完后的剩余值（向下不越 0）。
function M.spend_patience(key, cost)
  local h = host()
  local left = h.kv.get(K.patience .. key) or 0
  if left > cost then
    left = left - cost
  else
    left = 0
  end
  h.kv.set(K.patience .. key, left)
  return left
end

--- `world()` → 资源表（装载期快照的运行期读点）。
function M.world()
  return host().data(WORLD_ID)
end

--- `assert_not_seeded()`：防重复铺场的调用点断言（mod.lua 开局专用）。
function M.assert_not_seeded()
  if host().kv.get(K.seeded) then
    error(E_SEEDED .. ": 世界已铺过（sg.seeded 在场），开局事件不得重入", 0)
  end
  return true
end

return M
