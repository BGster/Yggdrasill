-- `src/belts.lua` —— 传送带路径与定点推进（v1 核心循环的热路径）。
--
-- # 契约定位（docs/design/mod/shapez-game.md §3）
--
-- 源 BeltPath/belt 系统（belt_path.js 1,661 行 + systems/belt.js）的 v1 等价面：
-- 路径 = 传送带链；物品 = shortkey 串；推进 = Q32.32 整数定点（Lua 64 位整数）。
-- 状态真源 = KV `sz.path.<startid>`（codec.encode 规范 JSON）；
-- 内存缓存按布局版本（`sz.layout_v`）失效重建。
--
-- # 定点口径（R1；黄金值自建，不逐字节复刻源浮点）
--
-- 单位 = Q32.32（raw = 值 × 2^32，全程整数 `//`）：
--   SPACING_Q   = itemSpacingMilli × 2^32 // (1000×1)   —— 0.63 tile 的定点近似
--   STEP_Q      = beltSpeedMilli × 2^32 // (1000 × ticksPerSecond)
-- 路径长 = 带数 × 2^32（v1 无曲线段：源曲线 0.78 记 1.0，语义差已登记）。
--
-- # v1 简化（逐条登记，契约 §3）
--   1. 链判定放宽：后带不逆风即接（源要求静息朝向严格对齐 + 曲线变体）。
--   2. 无压缩堆叠分支（numCompressedItems*）：v1 间距下限 0.63 恒成立。
--   3. 布局重建时路径状态不迁移（按 startid 命中才保留）。
--
-- 沙箱方言：模块顶层纯；ygg 只经 host() 单点取；无标准库表/math。

local codec = require("pkg.codec")

local M = { VERSION = "1.0.0" }

-- 方向枚举（契约 §2）：rot 0=+x 1=+y 2=-x 3=-y（y 向下，屏幕系）。
M.DIRX = { [0] = 1, [1] = 0, [2] = -1, [3] = 0 }
M.DIRY = { [0] = 0, [1] = 1, [2] = 0, [3] = -1 }

local Q32 = 4294967296 -- 2^32

local function host()
  return ygg
end

local function fail(code, detail)
  error("YGG-MOD-BELT-" .. code .. ": " .. detail, 0)
end

-- ============================================================================
-- 定点常量（纯整数推导；selftest 有 KAT 钉值）
-- ============================================================================

--- `constants(tuning)` → 定点常量表。全整数运算，跨平台逐字节一致。
function M.constants(tuning)
  local tps = tuning.ticksPerSecond
  if type(tps) ~= "number" or tps < 1 then
    fail("TUNING", "ticksPerSecond 必须正整数")
  end
  return {
    q32 = Q32,
    spacing_q = (tuning.itemSpacingMilli * Q32) // (1000 * 1),
    belt_step_q = (tuning.beltSpeedMilli * Q32) // (1000 * tps),
    miner_step_q = (tuning.minerSpeedMilli * Q32) // (1000 * tps),
    processor_step_q = function(cycle)
      if cycle == 0 then
        return 0 -- trash：即时
      end
      return (tuning.beltSpeedMilli * Q32) // (1000 * tps * cycle)
    end,
  }
end

-- ============================================================================
-- 路径重建
-- ============================================================================

local paths_cache = nil -- { [startid] = path }
local layout_version_seen = nil
local belt_fingerprint_seen = nil

local function entity_ids(schema)
  return host().entity.ids(schema)
end

local function get_attr(id, key)
  return host().entity.get(id, key)
end

local function belt_of(id)
  return {
    id = id,
    x = get_attr(id, "x"),
    y = get_attr(id, "y"),
    rot = get_attr(id, "rot"),
  }
end

local function tile_key(x, y)
  return x .. "," .. y
end

--- 前向目标带：位于 forward(cur) 且不逆风（rot ~= opposite(cur.rot)）。
local function successor_of(cur, tiles)
  local nx = cur.x + M.DIRX[cur.rot]
  local ny = cur.y + M.DIRY[cur.rot]
  local nid = tiles[tile_key(nx, ny)]
  if nid == nil then
    return nil
  end
  local nrot = get_attr(nid, "rot")
  local opposite = (cur.rot + 2) % 4
  if nrot == opposite then
    return nil
  end
  return nid
end

--- 前驱集合：所有 forward(p) == 本带位置且本带不逆风的带。
local function predecessors_of(bid, belts, tiles)
  local b = belts[bid]
  local out = {}
  local opposite = (b.rot + 2) % 4
  for delta = 0, 3 do
    if delta ~= opposite then
      -- p 指向本带：p 的 forward == 本带位置 → p 在本带反方向邻居
      local px = b.x - M.DIRX[delta]
      local py = b.y - M.DIRY[delta]
      local pid = tiles[tile_key(px, py)]
      if pid ~= nil and get_attr(pid, "rot") == delta then
        out[#out + 1] = pid
      end
    end
  end
  -- 升序（遍历 delta 0..3 不保证 id 序；路径构建对 id 序敏感）
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

local function load_path_state(startid)
  local raw = host().kv.get("sz.path." .. startid)
  if type(raw) ~= "string" or #raw == 0 then
    return { spacing_q = 0, items = {} }
  end
  local state = codec.decode(raw)
  return { spacing_q = state.s, items = state.i }
end

local function save_path_state(startid, path)
  host().kv.set("sz.path." .. startid, codec.encode({ s = path.spacing_q, i = path.items }))
end

--- `dirty()`：布局变了（放/拆带后调）；下一 tick 重建路径。
function M.dirty()
  paths_cache = nil
end

--- `ensure()`：按需重建路径（KV 真源 + 内存缓存）。返回路径表（只读用途）。
function M.ensure()
  local h = host()
  local version = h.kv.get("sz.layout_v") or 0
  local ids = entity_ids("sz.belt")
  -- 指纹 = 带数 + 最大 id：KV 被整体重置（selftest reset）时版本号回 0，
  -- 但实体集合已换血——指纹失配同样强制重建。
  local fingerprint = #ids .. ":" .. (ids[#ids] or 0)
  if paths_cache ~= nil and layout_version_seen == version and belt_fingerprint_seen == fingerprint then
    return paths_cache
  end
  local belts = {}
  local tiles = {}
  for i = 1, #ids do
    local b = belt_of(ids[i])
    belts[b.id] = b
    tiles[tile_key(b.x, b.y)] = b.id
  end

  local claimed = {}
  local paths = {}
  for i = 1, #ids do
    local startid = ids[i]
    if claimed[startid] == nil and #predecessors_of(startid, belts, tiles) == 0 then
      -- 新路径：从 startid 沿前向走，逐带认领
      local belt_ids = {}
      local cur = startid
      while cur ~= nil and claimed[cur] == nil do
        claimed[cur] = true
        belt_ids[#belt_ids + 1] = cur
        cur = successor_of(belts[cur], tiles)
      end
      local last = belts[belt_ids[#belt_ids]]
      local path = {
        start = startid,
        belts = belt_ids,
        total_q = #belt_ids * Q32,
        spacing_q = 0,
        items = {},
        front_x = last.x + M.DIRX[last.rot],
        front_y = last.y + M.DIRY[last.rot],
      }
      local state = load_path_state(startid)
      path.spacing_q = state.spacing_q
      path.items = state.items
      paths[startid] = path
    end
  end
  -- 有前驱但未被任何链认领的带（成环等）：自成单带路径（确定性兜底）
  for i = 1, #ids do
    local bid = ids[i]
    if claimed[bid] == nil then
      local b = belts[bid]
      claimed[bid] = true
      local path = {
        start = bid,
        belts = { bid },
        total_q = Q32,
        spacing_q = 0,
        items = {},
        front_x = b.x + M.DIRX[b.rot],
        front_y = b.y + M.DIRY[b.rot],
      }
      local state = load_path_state(bid)
      path.spacing_q = state.spacing_q
      path.items = state.items
      paths[bid] = path
    end
  end

  paths_cache = paths
  layout_version_seen = version
  belt_fingerprint_seen = fingerprint
  return paths
end

-- ============================================================================
-- 接收面（处理器 / 邻路移交的入带口）
-- ============================================================================

--- `can_accept(path)`：入带口空隙 ≥ 间距。
function M.can_accept(path)
  return path.spacing_q >= M.constants_cache.spacing_q
end

--- `try_accept(path, item_key)`：源 tryAcceptItem 的定点直译。
function M.try_accept(path, item_key)
  if type(item_key) ~= "string" then
    fail("ITEM", "物品必须是 shortkey 串")
  end
  if path.spacing_q < M.constants_cache.spacing_q then
    return false
  end
  local max_prog = path.spacing_q - M.constants_cache.spacing_q
  if max_prog < 0 then
    max_prog = 0
  end
  local initial = max_prog
  if M.constants_cache.belt_step_q < initial then
    initial = M.constants_cache.belt_step_q
  end
  -- 新物品插到队首（items[1] = 距入带口最近的物品）
  local new_items = { { d = path.spacing_q - initial, k = item_key } }
  for i = 1, #path.items do
    new_items[i + 1] = path.items[i]
  end
  path.items = new_items
  path.spacing_q = initial
  return true
end

-- ============================================================================
-- 推进
-- ============================================================================

--- `tick(constants, resolve_acceptor)`：全部路径各推进一步。
--- resolve_acceptor(front_x, front_y) → fn(item_key) → boolean | nil。
--- 返回是否发生了任何 KV 写（观察面用）。
function M.tick(c, resolve_acceptor)
  M.constants_cache = c
  local paths = M.ensure()
  local wrote = false
  -- 路径序 = startid 升序（确定性）
  local order = {}
  for startid in pairs(paths) do
    order[#order + 1] = startid
  end
  for i = 1, #order do
    local min = i
    for j = i + 1, #order do
      if order[j] < order[min] then
        min = j
      end
    end
    if min ~= i then
      order[i], order[min] = order[min], order[i]
    end
  end

  for k = 1, #order do
    local path = paths[order[k]]
    if #path.items > 0 then
      local remaining = c.belt_step_q
      local first = true
      for idx = #path.items, 1, -1 do
        local item = path.items[idx]
        local min_sp = 0
        if idx ~= #path.items then
          min_sp = c.spacing_q
        end
        local clamped = item.d - min_sp
        if remaining < clamped then
          clamped = remaining
        end
        if clamped < 0 then
          clamped = 0
        end
        remaining = remaining - clamped
        item.d = item.d - clamped
        path.spacing_q = path.spacing_q + clamped
        if first and item.d <= 0 then
          local acceptor = resolve_acceptor(path.front_x, path.front_y)
          if acceptor ~= nil and acceptor(item.k) then
            path.items[idx] = nil
          end
        end
        first = false
        if remaining <= 0 then
          break
        end
      end
      -- 循环后补射（源 post-update：末位 d==0 再试一次）
      local n = #path.items
      if n > 0 and path.items[n].d <= 0 then
        local acceptor = resolve_acceptor(path.front_x, path.front_y)
        if acceptor ~= nil and acceptor(path.items[n].k) then
          path.items[n] = nil
        end
      end
      -- 压实 items（弹出后出现空洞）
      local compact = {}
      for idx = 1, #path.items do
        if path.items[idx] ~= nil then
          compact[#compact + 1] = path.items[idx]
        end
      end
      path.items = compact
      save_path_state(path.start, path)
      wrote = true
    end
  end
  return wrote
end

--- `snapshot()` → 全路径状态表（重放哈希输入；纯读）。
function M.snapshot()
  local paths = M.ensure()
  local out = {}
  local order = {}
  for startid in pairs(paths) do
    order[#order + 1] = startid
  end
  for i = 1, #order do
    local min = i
    for j = i + 1, #order do
      if order[j] < order[min] then
        min = j
      end
    end
    if min ~= i then
      order[i], order[min] = order[min], order[i]
    end
  end
  for k = 1, #order do
    local path = paths[order[k]]
    local items = {}
    for idx = 1, #path.items do
      items[idx] = { d = path.items[idx].d, k = path.items[idx].k }
    end
    out[k] = { start = path.start, s = path.spacing_q, i = items }
  end
  return out
end

--- `bump_layout()`：布局版本 +1（放/拆带后由装配面调）。
function M.bump_layout()
  local h = host()
  h.kv.set("sz.layout_v", (h.kv.get("sz.layout_v") or 0) + 1)
  M.dirty()
end

return M
