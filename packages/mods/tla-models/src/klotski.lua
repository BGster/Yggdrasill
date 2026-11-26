-- `src/klotski.lua` —— SlidingPuzzles（Klotski 4×5；组合谜题 + CHOOSE 唯一化）。
--
-- 契约：openwiki/contracts/mod/tla-models.md §映射表。源：tlaplus/Examples
-- `specifications/SlidingPuzzles/SlidingPuzzles.tla`（源仓 manifest 无全量
-- 状态数，runtime 00:00:05；81 步最优系源注，本移植只承诺格步语义）。
--
-- 映射：`board`（10 件集）→ 10 个 `tla.piece` 实体 schema `{kind, ox, oy}`，
-- cells 由 kind 偏移展开（v2/h2/sq/u）；`Init == board = Klotski` → 钉源
-- 十件布局；`Next`（\E e \in empty + update）→ `step("e=x,y|d=dx,dy")`
-- ——空格选择与方向显式化为动作输入（\E 向量化入轨）；`ChooseOne` 唯一化
-- → 目标格命中件数 ≠ 1 必报 `YGG-MOD-TLA-CHOOSE`（不静默取第一个）；
-- `TypeOK` → inv 注册表 + **运行期加强：件两两不交**（TLA 的集合嵌套表达
-- 不出重叠，实体形状能，红例①的检出点，契约 §语义子集声明）；`KlotskiGoal`
-- → 「2×2 方块不在底中四格」逐字移植（源以不变量反用驱动 TLC 找 81 步解）。
--
-- move 语义逐字对齐源：`m = {q - d : q ∈ pc}`（件向空格平移 −d），
-- free 过滤 = m ∩ ⋃(board\{pc}) = {} ∧ m ⊆ Pos；显式动作输入命中被过滤
-- 候选时报 `YGG-MOD-TLA-BLOCKED`（fail-closed，不静默跳过）。

local entity = require("pkg.entity")
local inv = require("src.inv")

local M = { VERSION = "0.1.0", name = "klotski" }

local SCHEMA = "tla.piece"
local W, H = 4, 5
M.W, M.H = W, H

--- reset 期刷新的实体 id 缓存（同 gol/ewd998：实体集合只在 reset 变化，
--- 热路径免 O(seq) KV 扫）。
local ids_cache = nil

--- kind → cells 偏移（源 Klotski 集合的件形归纳）。
local KINDS = {
  v2 = { { 0, 0 }, { 0, 1 } },
  h2 = { { 0, 0 }, { 1, 0 } },
  sq = { { 0, 0 }, { 1, 0 }, { 0, 1 }, { 1, 1 } },
  u = { { 0, 0 } },
}

--- 源 Init 十件布局（kind, ox, oy）。
local INIT = {
  { "v2", 0, 0 }, { "sq", 1, 0 }, { "v2", 3, 0 }, { "v2", 0, 2 },
  { "h2", 1, 2 }, { "v2", 3, 2 },
  { "u", 1, 3 }, { "u", 2, 3 }, { "u", 0, 4 }, { "u", 3, 4 },
}

--- KlotskiGoal 的目标四格（源 {<<1,3>>,<<1,4>>,<<2,3>>,<<2,4>>}）。
local GOAL = { { 1, 3 }, { 1, 4 }, { 2, 3 }, { 2, 4 } }

--- 方向集（源 dir：四邻，序固定）。
local DIRS = { { 1, 0 }, { 0, 1 }, { -1, 0 }, { 0, -1 } }

-- ── 基元（沙箱方言：无 table 库）────────────────────────────────────────────

local function in_pos(x, y)
  return x >= 0 and x < W and y >= 0 and y < H
end

--- 全部件 id（升序；热路径走 reset 期缓存）。
local function piece_ids()
  if ids_cache ~= nil then
    return ids_cache
  end
  ids_cache = entity.ids(nil, SCHEMA)
  return ids_cache
end

--- 一件的 cells（kind 偏移 + (ox,oy) 展开），数组 {{x,y},...}。
local function piece_cells(id)
  local kind = entity.get(id, "kind")
  local ox = entity.get(id, "ox")
  local oy = entity.get(id, "oy")
  local offsets = KINDS[kind]
  local cells = {}
  for i = 1, #offsets do
    cells[#cells + 1] = { offsets[i][1] + ox, offsets[i][2] + oy }
  end
  return cells
end

--- 命中格 s 的件（ChooseOne 的候选收集）：0 或 ≥2 件即报
--- `YGG-MOD-TLA-CHOOSE`（唯一化语义，红例③的检出点）。
local function choose_piece(sx, sy)
  local hit = nil
  local count = 0
  local ids = piece_ids()
  for i = 1, #ids do
    local cells = piece_cells(ids[i])
    for j = 1, #cells do
      if cells[j][1] == sx and cells[j][2] == sy then
        hit = ids[i]
        count = count + 1
        break
      end
    end
  end
  if count ~= 1 then
    error("YGG-MOD-TLA-CHOOSE: 格 " .. sx .. "," .. sy .. " 命中 " .. count
      .. " 件（ChooseOne 要求恰一）", 0)
  end
  return hit
end

--- 件集 → 占格键集（"x,y"）。
local function occupy_map(exclude_id)
  local occ = {}
  local ids = piece_ids()
  for i = 1, #ids do
    if ids[i] ~= exclude_id then
      local cells = piece_cells(ids[i])
      for j = 1, #cells do
        occ[cells[j][1] .. "," .. cells[j][2]] = true
      end
    end
  end
  return occ
end

-- ── 不变量 ───────────────────────────────────────────────────────────────────

--- TypeOK：10 件；kind 合法；ox/oy 整数；cells ⊆ Pos；
--- **加强**：件 cells 两两不交（运行期检出重叠，契约 §语义子集）。
local function check_type_ok()
  local violations = {}
  local ids = piece_ids()
  if #ids ~= #INIT then
    violations[#violations + 1] = "件数 " .. #ids .. " ≠ " .. #INIT
  end
  local seen = {}
  for i = 1, #ids do
    local id = ids[i]
    local kind = entity.get(id, "kind")
    local ox = entity.get(id, "ox")
    local oy = entity.get(id, "oy")
    if KINDS[kind] == nil then
      violations[#violations + 1] = "@" .. id .. " 未知 kind：" .. tostring(kind)
    end
    if type(ox) ~= "number" or ox ~= (ox // 1) or type(oy) ~= "number" or oy ~= (oy // 1) then
      violations[#violations + 1] = "@" .. id .. " ox/oy 非整：" .. tostring(ox) .. "," .. tostring(oy)
    end
    local cells = piece_cells(id)
    for j = 1, #cells do
      local cx, cy = cells[j][1], cells[j][2]
      if not in_pos(cx, cy) then
        violations[#violations + 1] = "@" .. id .. " 出界格：" .. cx .. "," .. cy
      end
      local key = cx .. "," .. cy
      if seen[key] then
        violations[#violations + 1] = "重叠格：" .. key .. "（@" .. seen[key] .. " 与 @" .. id .. "）"
      end
      seen[key] = id
    end
  end
  return violations
end

--- KlotskiGoal：目标四格集 ∉ board（逐字：任何件的 cell 集 ≠ 目标四格集）。
local function check_goal()
  local ids = piece_ids()
  for i = 1, #ids do
    local cells = piece_cells(ids[i])
    if #cells == #GOAL then
      local match = true
      for j = 1, #GOAL do
        local found = false
        for k = 1, #cells do
          if cells[k][1] == GOAL[j][1] and cells[k][2] == GOAL[j][2] then
            found = true
            break
          end
        end
        if not found then
          match = false
          break
        end
      end
      if match then
        return { "2×2 方块抵达底中四格（目标态）" }
      end
    end
  end
  return {}
end

-- ── 装配 ─────────────────────────────────────────────────────────────────────

function M.install()
  entity.define(SCHEMA, {
    attrs = {
      kind = { type = "string", required = true },
      ox = { type = "number", required = true },
      oy = { type = "number", required = true },
    },
  })
  inv.register(M.name, "KlotskiGoal", check_goal)
  inv.register(M.name, "TypeOK", check_type_ok)
  return true
end

--- `reset()`：钉源 Init 十件布局（旧件全拆重建）。
function M.reset()
  ids_cache = nil
  local ids = piece_ids()
  for i = 1, #ids do
    entity.destroy(ids[i])
  end
  ids_cache = nil
  for i = 1, #INIT do
    entity.spawn(SCHEMA, { kind = INIT[i][1], ox = INIT[i][2], oy = INIT[i][3] })
  end
end

--- 当前空格键集（Pos \ ⋃board）。
function M.empty_cells()
  local occ = occupy_map(nil)
  local out = {}
  for y = 0, H - 1 do
    for x = 0, W - 1 do
      if not occ[x .. "," .. y] then
        out[#out + 1] = { x, y }
      end
    end
  end
  return out
end

--- `step(action)`：`"e=x,y|d=dx,dy"`——空格 e 沿方向 d 收件（源 update 语义）。
function M.step(action)
  if action == nil or action == "stutter" then
    return
  end
  local ex, ey, dx, dy = tostring(action):match("^e=(-?%d+),(-?%d+)|d=(-?%d+),(-?%d+)$")
  if ex == nil then
    error("YGG-MOD-TLA-ACTION: klotski 动作应为 \"e=x,y|d=dx,dy\"：" .. tostring(action), 0)
  end
  ex, ey, dx, dy = tonumber(ex), tonumber(ey), tonumber(dx), tonumber(dy)
  local occ = occupy_map(nil)
  if occ[ex .. "," .. ey] then
    error("YGG-MOD-TLA-GUARD: e 不是空格：" .. ex .. "," .. ey, 0)
  end
  -- dirs 过滤：目标格须在 Pos 且被占（源 dir(e, es) 两条件逐字）。
  local sx, sy = ex + dx, ey + dy
  if not in_pos(sx, sy) or not occ[sx .. "," .. sy] then
    error("YGG-MOD-TLA-GUARD: 方向不使能（目标须在界内且被占）："
      .. sx .. "," .. sy, 0)
  end
  local id = choose_piece(sx, sy)
  -- m = {q - d : q ∈ pc}；free 过滤：不撞他件 ∧ ⊆ Pos（源 update 逐字）。
  local cells = piece_cells(id)
  local others = occupy_map(id)
  local moved = {}
  for i = 1, #cells do
    local mx, my = cells[i][1] - dx, cells[i][2] - dy
    if not in_pos(mx, my) then
      error("YGG-MOD-TLA-BLOCKED: 平移出界：" .. mx .. "," .. my, 0)
    end
    if others[mx .. "," .. my] then
      error("YGG-MOD-TLA-BLOCKED: 平移撞件：" .. mx .. "," .. my, 0)
    end
    moved[#moved + 1] = { mx, my }
  end
  -- 件恒等保持（同一实体 id，只改 ox/oy）。
  entity.set(id, "ox", entity.get(id, "ox") - dx)
  entity.set(id, "oy", entity.get(id, "oy") - dy)
  return moved
end

--- 启用动作枚举：每空格 × 每（界内且被占的）方向，且过 **free 全过滤**
--- （源 update 逐字：m = cells−d 不出界 ∧ 不撞他件——缺此过滤会把
--- 被挡候选列进选择域，auto 掷中即 BLOCKED，S26-4 测试期实捉）。
--- stutter 恒在；输出升序（字符串序 = auto_tick 的确定选择域）。
function M.auto_actions()
  local ids = piece_ids()
  local cells_of = {}
  local occ = {}
  for i = 1, #ids do
    local cs = piece_cells(ids[i])
    cells_of[ids[i]] = cs
    for j = 1, #cs do
      occ[cs[j][1] .. "," .. cs[j][2]] = ids[i]
    end
  end
  local out = { "stutter" }
  for y = 0, H - 1 do
    for x = 0, W - 1 do
      if not occ[x .. "," .. y] then
        for i = 1, #DIRS do
          local dx, dy = DIRS[i][1], DIRS[i][2]
          local sx, sy = x + dx, y + dy
          local pid = nil
          if in_pos(sx, sy) then
            pid = occ[sx .. "," .. sy]
          end
          if pid ~= nil then
            local free = true
            local cs = cells_of[pid]
            for k = 1, #cs do
              local mx, my = cs[k][1] - dx, cs[k][2] - dy
              if not in_pos(mx, my) then
                free = false
                break
              end
              local holder = occ[mx .. "," .. my]
              if holder ~= nil and holder ~= pid then
                free = false
                break
              end
            end
            if free then
              out[#out + 1] = "e=" .. x .. "," .. y .. "|d=" .. dx .. "," .. dy
            end
          end
        end
      end
    end
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

--- `snapshot()`：规范快照（pieces 按 ox,oy,kind 升序的数组）。
function M.snapshot()
  local ids = piece_ids()
  local list = {}
  for i = 1, #ids do
    local id = ids[i]
    list[#list + 1] = {
      kind = entity.get(id, "kind"),
      ox = entity.get(id, "ox"),
      oy = entity.get(id, "oy"),
      __id = id,
    }
  end
  for i = 1, #list do
    local min = i
    for j = i + 1, #list do
      local b, m = list[j], list[min]
      if b.ox < m.ox or (b.ox == m.ox and (b.oy < m.oy or (b.oy == m.oy and b.kind < m.kind))) then
        min = j
      end
    end
    if min ~= i then
      list[i], list[min] = list[min], list[i]
    end
  end
  local pieces = {}
  for i = 1, #list do
    pieces[#pieces + 1] = { kind = list[i].kind, ox = list[i].ox, oy = list[i].oy }
  end
  return { pieces = pieces }
end

--- 件恒等读（重放前后同 id 断言用）。
function M.piece_id_at(kind, ox, oy)
  local ids = piece_ids()
  for i = 1, #ids do
    local id = ids[i]
    if entity.get(id, "kind") == kind and entity.get(id, "ox") == ox and entity.get(id, "oy") == oy then
      return id
    end
  end
  return nil
end

return M
