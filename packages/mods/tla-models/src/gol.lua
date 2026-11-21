-- `src/gol.lua` —— GameOfLife N=4（确定性 CA；TLA 惯用法：唯一后继）。
--
-- 契约：docs/design/mod/tla-models.md §映射表。源：tlaplus/Examples
-- `specifications/GameOfLife/GameOfLife.tla`（TLC 实测 65536/131072/1，
-- 引自源仓 manifest.json）。
--
-- 映射：`grid \in [Pos -> BOOLEAN]`（Pos = 1..N × 1..N）→ 16 个 `tla.cell`
-- 实体 schema `{x, y, alive}`（每身份态一实体）；`TypeOK` → inv 注册表
-- （格恰 N²、坐标在界、alive 布尔、无重复坐标）；`Init`（全布尔域非确定）
-- → 钉规范初态 = 全灭（pattern 由 reset opts 显式注入，契约 §语义子集）；
-- `Next`（唯一后继）→ `step("step")`，stutter = `step("stutter")`。
--
-- 规则逐字对齐源：`score(p)` = 8 邻域内 `sc` 之和，出界或死格计 0
-- （sc 的 CASE 四分支）；活格 score∈{2,3} 存活，死格 score=3 复活。
-- KAT 黄金值 = Python 独立推导 + 手算双推导一致（blinker 往返 / block
-- 稳定 / toad 单步，见 test-a-gol.lua 头注）。

local entity = require("pkg.entity")
local inv = require("src.inv")

local M = { VERSION = "0.1.0", name = "gol" }

local N = 4
M.N = N

local SCHEMA = "tla.cell"

--- 坐标 → 键（`x,y`），实体复用与查重的单一口径。
local function coord_key(x, y)
  return x .. "," .. y
end

--- 全部活实体 id（升序）。
local function cell_ids()
  return entity.ids(nil, SCHEMA)
end

-- ── 不变量（TypeOK 的运行期移植）────────────────────────────────────────────

--- TypeOK：`grid \in [Pos -> BOOLEAN]`。实体面检查四条：
--- ①格数恰 N²；②每个 (x,y) 是 1..N 的整数；③alive 是布尔；
--- ④坐标无重复（同格双实体 = 非法态红例的检出点）。
local function check_type_ok()
  local violations = {}
  local ids = cell_ids()
  local seen = {}
  local n = #ids
  if n ~= N * N then
    violations[#violations + 1] = "格数 " .. n .. " ≠ " .. N * N
  end
  for i = 1, n do
    local id = ids[i]
    local x = entity.get(id, "x")
    local y = entity.get(id, "y")
    local alive = entity.get(id, "alive")
    local where = "@" .. tostring(id)
    if type(x) ~= "number" or x ~= (x // 1) or x < 1 or x > N then
      violations[#violations + 1] = where .. " x 越界/非整：" .. tostring(x)
    end
    if type(y) ~= "number" or y ~= (y // 1) or y < 1 or y > N then
      violations[#violations + 1] = where .. " y 越界/非整：" .. tostring(y)
    end
    if type(alive) ~= "boolean" then
      violations[#violations + 1] = where .. " alive 非布尔：" .. tostring(alive)
    end
    if type(x) == "number" and type(y) == "number" then
      local key = coord_key(x, y)
      if seen[key] then
        violations[#violations + 1] = where .. " 坐标重复：" .. key
      end
      seen[key] = true
    end
  end
  return violations
end

-- ── 装配 ─────────────────────────────────────────────────────────────────────

--- schema 注册 + 不变量注册（幂等）。
function M.install()
  entity.define(SCHEMA, {
    fields = {
      x = { type = "number", required = true },
      y = { type = "number", required = true },
      alive = { type = "boolean", required = true },
    },
  })
  inv.register(M.name, "TypeOK", check_type_ok)
  return true
end

--- 单格读（setup 后的正确性依赖 TypeOK 绿；越界读返回 nil）。
function M.alive_at(x, y)
  local ids = cell_ids()
  for i = 1, #ids do
    local id = ids[i]
    if entity.get(id, "x") == x and entity.get(id, "y") == y then
      return entity.get(id, "alive") == true
    end
  end
  return nil
end

--- `reset(opts?)`：钉规范初态。opts.alive = {"x,y", ...} 为开局的活格
--- （缺省全灭）。旧实体全拆重建（实体集合换血，与 shapez reset 同法）。
function M.reset(opts)
  local ids = cell_ids()
  for i = 1, #ids do
    entity.destroy(ids[i])
  end
  local alive = {}
  if opts ~= nil and type(opts.alive) == "table" then
    for i = 1, #opts.alive do
      alive[opts.alive[i]] = true
    end
  end
  for y = 1, N do
    for x = 1, N do
      entity.spawn(SCHEMA, { x = x, y = y, alive = alive[coord_key(x, y)] == true })
    end
  end
end

-- ── 规则（源 Next 逐字对齐）────────────────────────────────────────────────

--- `sc(p)`：出界或死格 = 0，否则 1（源 CASE 四分支合流）。
local function sc(x, y)
  if x < 1 or y < 1 or x > N or y > N then
    return 0
  end
  return M.alive_at(x, y) and 1 or 0
end

--- `score(p)`：8 邻域 sc 之和（无 math/table 库，直排 8 项）。
local function score(x, y)
  return sc(x - 1, y - 1) + sc(x, y - 1) + sc(x + 1, y - 1)
    + sc(x - 1, y) + sc(x + 1, y)
    + sc(x - 1, y + 1) + sc(x, y + 1) + sc(x + 1, y + 1)
end

--- 下一代活格键集（先整表求值再落盘——原 Next 的 grid' 是单步全量替换，
--- 逐格原地写会串代）。
local function next_generation()
  local alive_next = {}
  for y = 1, N do
    for x = 1, N do
      local s = score(x, y)
      if (M.alive_at(x, y) and (s == 2 or s == 3)) or (not M.alive_at(x, y) and s == 3) then
        alive_next[coord_key(x, y)] = true
      end
    end
  end
  return alive_next
end

--- `step(action)`：`"step"` = 一个 generation（唯一后继）；`"stutter"` =
--- `[][Next]_grid` 的不变分支。其余动作串 = 契约错误（fail-closed）。
function M.step(action)
  if action == nil or action == "stutter" then
    return
  end
  if action ~= "step" then
    error("YGG-MOD-TLA-ACTION: gol 未知动作（step|stutter）：" .. tostring(action), 0)
  end
  local alive_next = next_generation()
  local ids = cell_ids()
  for i = 1, #ids do
    local id = ids[i]
    local key = coord_key(entity.get(id, "x"), entity.get(id, "y"))
    entity.set(id, "alive", alive_next[key] == true)
  end
end

--- 当前状态下启用的动作（确定性模型：step 恒可用 + stutter 恒可用）。
function M.auto_actions()
  return { "stutter", "step" }
end

--- `snapshot()`：规范快照（cells 按 y,x 升序的数组；键名固定 = 哈希 KAT 面）。
function M.snapshot()
  local ids = cell_ids()
  local by_coord = {}
  for i = 1, #ids do
    local id = ids[i]
    by_coord[coord_key(entity.get(id, "y"), entity.get(id, "x"))] = {
      x = entity.get(id, "x"),
      y = entity.get(id, "y"),
      alive = entity.get(id, "alive") == true,
    }
  end
  local cells = {}
  for y = 1, N do
    for x = 1, N do
      cells[#cells + 1] = by_coord[coord_key(y, x)]
    end
  end
  return { n = N, cells = cells }
end

return M
