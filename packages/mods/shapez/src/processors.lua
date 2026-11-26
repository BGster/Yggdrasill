-- `src/processors.lua` —— 处理器：miner / cutter / stacker / rotater / trash。
--
-- # 契约定位（openwiki/contracts/mod/shapez-game.md §4）
--
-- 源 item_processor.js（642 行）+ miner/hub 系统 + ejector 系统的 v1 等价面。
-- 每台处理器 = `sz.processor` 实体（attrs：kind/in1/in2/progress/out1/out2）；
-- 矿机 = `sz.miner`（shape/progress/out1）。进度与输出槽全部定点整数 + shortkey 串。
--
-- # v1 简化（逐条登记，契约 §4）
--   1. 单一前向出料口：out1→out2 串行出料（源 = 每槽独立方向同时出）。
--   2. 完成时输出槽须空闲，否则 progress 钉在满值等待（源 ejector 槽语义的收敛版）。
--   3. trash 即时（cycle=0），不占进度。
--   4. 每 tick 每台至多出一件（源同 tick 可双槽齐发）。
--
-- 方向约定沿用 belts.DIRX/DIRY；入料口 = 背面格（pos - DIR[rot]），
-- 出料口 = 正面格（pos + DIR[rot]）。
--
-- 沙箱方言：模块顶层纯；实体面走 pkg.entity（宿主 14 键面不直碰）。

local entity = require("pkg.entity")
local belts = require("src.belts")
local shapes = require("src.shapes")

local M = { VERSION = "1.0.0" }

local Q32 = 4294967296

local function fail(code, detail)
  error("YGG-MOD-PROC-" .. code .. ": " .. detail, 0)
end

-- ============================================================================
-- 放置 / 查询
-- ============================================================================

local KINDS = { cutter = true, stacker = true, rotater = true, trash = true }

--- `place(kind, x, y, rot)` → id。
function M.place(kind, x, y, rot)
  if not KINDS[kind] then
    fail("KIND", "未知处理器类型：" .. tostring(kind))
  end
  local id = entity.spawn("sz.processor", {
    x = x, y = y, rot = rot,
    kind = kind,
    in1 = "", in2 = "",
    progress = 0,
    out1 = "", out2 = "",
  }, { processor = true, [kind] = true })
  belts.bump_layout()
  return id
end

--- `place_miner(x, y, rot)` → id（矿机；shape 由资源斑查得）。
function M.place_miner(x, y, rot, patch)
  if patch == nil then
    fail("PATCH", "矿机必须落在资源斑上")
  end
  local id = entity.spawn("sz.miner", {
    x = x, y = y, rot = rot,
    shape = shapes.miner_shape(patch.subshape, patch.color),
    progress = 0,
    out1 = "",
  }, { miner = true })
  belts.bump_layout()
  return id
end

local function scan(schema)
  return entity.ids(nil, schema)
end

--- `at(schema, x, y)` → id | nil（该格上的该类实体）。
function M.at(schema, x, y)
  local ids = scan(schema)
  for i = 1, #ids do
    local id = ids[i]
    if entity.get(id, "x") == x and entity.get(id, "y") == y then
      return id
    end
  end
  return nil
end

-- ============================================================================
-- 入料面（传送带末端弹射的落点）
-- ============================================================================

--- `input_free(id)`：入料槽是否有空位。
function M.input_free(id)
  local kind = entity.get(id, "kind")
  local in1 = entity.get(id, "in1")
  if kind == "stacker" then
    return in1 == "" or entity.get(id, "in2") == ""
  end
  return in1 == ""
end

--- `accept_input(id, key)`：入一件（stacker 先 in1 后 in2）。
function M.accept_input(id, key)
  if not M.input_free(id) then
    return false
  end
  if entity.get(id, "in1") == "" then
    entity.set(id, "in1", key)
    return true
  end
  entity.set(id, "in2", key)
  return true
end

-- ============================================================================
-- 产物计算（纯函数，形状库出口）
-- ============================================================================

local function outputs_of(kind, in1, in2)
  if kind == "cutter" then
    local halves = shapes.cut_half(in1)
    return { halves.right, halves.left }
  elseif kind == "rotater" then
    return { shapes.rotate_cw(in1) }
  elseif kind == "stacker" then
    return { shapes.stack(in1, in2) }
  elseif kind == "trash" then
    return {}
  end
  fail("KIND", "未知类型：" .. tostring(kind))
end

local function inputs_needed(kind)
  if kind == "stacker" then
    return 2
  end
  return 1
end

-- ============================================================================
-- 推进
-- ============================================================================

--- 单台前向出料：out1 优先；仅空闲（progress == 0）时出（产物在加工期占槽不可射）。
--- out2 缺席（矿机单槽）时跳过晋升。返回是否移出。
local function try_eject_one(id, resolve)
  local progress = entity.get(id, "progress") or 0
  if progress ~= 0 then
    return false
  end
  local out1 = entity.get(id, "out1")
  if out1 == nil or out1 == "" then
    local out2 = entity.get(id, "out2")
    if out2 ~= nil and out2 ~= "" then
      entity.set(id, "out1", out2)
      entity.set(id, "out2", "")
    end
    return false
  end
  local rot = entity.get(id, "rot")
  local fx = entity.get(id, "x") + belts.DIRX[rot]
  local fy = entity.get(id, "y") + belts.DIRY[rot]
  local acceptor = resolve(fx, fy)
  if acceptor ~= nil and acceptor(out1) then
    entity.set(id, "out1", "")
    return true
  end
  return false
end

--- 矿机出料：不受 progress 门（那是产量累加器，非忙态）；out1 有料即射。
local function try_eject_miner(id, resolve)
  local out1 = entity.get(id, "out1")
  if out1 == nil or out1 == "" then
    return false
  end
  local rot = entity.get(id, "rot")
  local fx = entity.get(id, "x") + belts.DIRX[rot]
  local fy = entity.get(id, "y") + belts.DIRY[rot]
  local acceptor = resolve(fx, fy)
  if acceptor ~= nil and acceptor(out1) then
    entity.set(id, "out1", "")
    return true
  end
  return false
end

--- `tick(c, resolve)`：全部处理器 + 矿机各推进一步（id 升序）。
--- resolve(fx, fy) → fn(item_key) → boolean | nil（simtick 装配的落点裁决）。
function M.tick(c, resolve)
  -- ① 处理器：出料 → 投产/推进
  local ids = scan("sz.processor")
  for i = 1, #ids do
    local id = ids[i]
    local kind = entity.get(id, "kind")
    if kind == "trash" then
      -- 即时：入口有料即吞
      entity.set(id, "in1", "")
      entity.set(id, "in2", "")
    else
      try_eject_one(id, resolve)
      local in1 = entity.get(id, "in1")
      local in2 = entity.get(id, "in2")
      local needed = inputs_needed(kind)
      local ready = in1 ~= "" and (needed == 1 or in2 ~= "")
      local progress = entity.get(id, "progress") or 0
      local out1 = entity.get(id, "out1")
      local out2 = entity.get(id, "out2") or ""
      if ready and progress == 0 and (out1 == nil or out1 == "") and out2 == "" then
        -- 开工即算产物并占槽（源：完成时输出进 ejector 槽；v1 收敛为开工占用）
        local outs = outputs_of(kind, in1, in2)
        entity.set(id, "in1", "")
        if needed == 2 then
          entity.set(id, "in2", "")
        end
        entity.set(id, "out1", outs[1] or "")
        entity.set(id, "out2", outs[2] or "")
        progress = c.processor_step_q(c.cycle_of(kind))
        if progress > Q32 then
          progress = Q32
        end
        entity.set(id, "progress", progress)
      elseif progress > 0 then
        -- 加工中：推进；到满即空闲（产物已在槽上）
        progress = progress + c.processor_step_q(c.cycle_of(kind))
        if progress >= Q32 then
          progress = 0
        end
        entity.set(id, "progress", progress)
      end
    end
  end
  -- ② 矿机：产出 → 出料
  local mids = scan("sz.miner")
  for i = 1, #mids do
    local id = mids[i]
    if entity.get(id, "out1") == "" then
      local progress = entity.get(id, "progress") + c.miner_step_q
      if progress >= Q32 then
        entity.set(id, "out1", entity.get(id, "shape"))
        progress = progress - Q32
      end
      entity.set(id, "progress", progress)
    end
    try_eject_miner(id, resolve)
  end
end

--- `snapshot()` → 处理器+矿机状态行（重放哈希输入；id 升序）。
function M.snapshot()
  local out = {}
  local ids = entity.ids(nil, "sz.processor")
  for i = 1, #ids do
    local id = ids[i]
    out[#out + 1] = {
      id = id, kind = entity.get(id, "kind"),
      in1 = entity.get(id, "in1"), in2 = entity.get(id, "in2"),
      progress = entity.get(id, "progress"),
      out1 = entity.get(id, "out1"), out2 = entity.get(id, "out2"),
    }
  end
  local mids = entity.ids(nil, "sz.miner")
  for i = 1, #mids do
    local id = mids[i]
    out[#out + 1] = {
      id = id, kind = "miner",
      shape = entity.get(id, "shape"),
      progress = entity.get(id, "progress"),
      out1 = entity.get(id, "out1"),
    }
  end
  -- 按 id 升序（两类混排后重排，确定性）
  for i = 1, #out do
    local min = i
    for j = i + 1, #out do
      if out[j].id < out[min].id then
        min = j
      end
    end
    if min ~= i then
      out[i], out[min] = out[min], out[i]
    end
  end
  return out
end

return M
