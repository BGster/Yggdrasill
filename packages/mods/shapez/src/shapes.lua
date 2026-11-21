-- `src/shapes.lua` —— 形状规范形：shortkey 编解码 + 切割/堆叠/旋转纯函数。
--
-- # 契约定位（docs/design/mod/shapez-game.md §2）
--
-- shortkey 字符串**就是**规范形（源 `serialize() == getHash()` 的同位语义）：
-- 层自下而上以 `:` 相接，每层 8 字符 = 4 象限 ×（子形 + 颜色），
-- 象限序 0=tr 1=br 2=bl 3=tl，空象限 `--`。切割/堆叠/旋转 = 串↔表↔串的纯变换。
--
-- # 语义来源（shapez.io @8a79de5，GPL-3 内部测试；等价复现非逐行抄写）
--
--   shape_definition.js        fromShortKey / getHash / cloneRotate* /
--                              cloneFilteredByQuadrants / cloneAndStackWith
--   shape_definition_manager.js shapeActionCutHalf（right=[2,3] 的源序保留）
-- 语义差（R1/R2/R4，逐条有 selftest）：
--   - 象限表内保留 0..3 下标（配对常量，不碰 `#`）；层是 1 基数组。
--   - 截断一律整数 `//`；层拼接顺序钉死在 KAT 里。
--   - 空项 = 空串 `""`（源 empty definition 的 getHash），不是合法 shortkey，
--     只作传输占位；is_valid("") == false。
--
-- 沙箱方言：模块顶层纯；无 `string`/`table` 标准库表（方法语法可用）；
-- 无 `math`。排序/拼接手写。

local M = { VERSION = "1.0.0" }

-- 子形/颜色 shortcode 字符集（源 enumSubShapeToShortcode / enumColorToShortcode）。
local SUBCHARS = "RCSW"
local COLORCHARS = "rgbypwu"
local MAX_LAYERS = 4

local function host()
  return ygg
end

local function fail(code, detail)
  error("YGG-MOD-SHAPE-" .. code .. ": " .. detail, 0)
end

--- 字符是否在集合串里（无 string.find 的纯扫法；集合串 ≤ 7 字符）。
local function char_in(ch, set)
  if type(ch) ~= "string" or #ch ~= 1 then
    return false
  end
  for i = 1, #set do
    if set:sub(i, i) == ch then
      return true
    end
  end
  return false
end

--- 单层 8 字符 → `{ [0..3] = {s,c}|nil }`；非法即抛 `-KEY`。
local function decode_layer(text, key)
  if #text ~= 8 then
    fail("KEY", "层长必须 8 字符，得到 " .. #text .. "（key=" .. key .. "）")
  end
  local quads = {}
  local any = false
  for quad = 0, 3 do
    local sc = text:sub(quad * 2 + 1, quad * 2 + 1)
    local cc = text:sub(quad * 2 + 2, quad * 2 + 2)
    if sc == "-" then
      if cc ~= "-" then
        fail("KEY", "空象限颜色位必须 `-`（key=" .. key .. "）")
      end
      quads[quad] = nil
    elseif char_in(sc, SUBCHARS) then
      if not char_in(cc, COLORCHARS) then
        fail("KEY", "颜色位非法（key=" .. key .. "）")
      end
      quads[quad] = { s = sc, c = cc }
      any = true
    else
      fail("KEY", "子形位非法（key=" .. key .. "）")
    end
  end
  if not any then
    fail("KEY", "全空层（key=" .. key .. "）")
  end
  return quads
end

--- 层数组 → 规范 shortkey（丢全空层；层间 `:`）。
local function encode_layers(layers)
  local parts = {}
  for i = 1, #layers do
    local quads = layers[i]
    local text = ""
    local any = false
    for quad = 0, 3 do
      local item = quads[quad]
      if item == nil then
        text = text .. "--"
      else
        text = text .. item.s .. item.c
        any = true
      end
    end
    if any then
      parts[#parts + 1] = text
    end
  end
  local out = ""
  for i = 1, #parts do
    if i > 1 then
      out = out .. ":"
    end
    out = out .. parts[i]
  end
  return out
end

--- 拆层文本（按 `:`）；返回文本数组。
local function split_layers(key)
  local parts = {}
  local cur = ""
  for i = 1, #key do
    local ch = key:sub(i, i)
    if ch == ":" then
      parts[#parts + 1] = cur
      cur = ""
    else
      cur = cur .. ch
    end
  end
  parts[#parts + 1] = cur
  return parts
end

-- ============================================================================
-- 规范面
-- ============================================================================

--- `is_valid(key)` → boolean：1..4 层、每层 8 字符、字符集合法、每层非空。
--- 空串与任何非法输入一律 `false`（不抛）。
function M.is_valid(key)
  if type(key) ~= "string" or #key == 0 or #key > 4 * 9 - 1 then
    return false
  end
  local parts = split_layers(key)
  if #parts < 1 or #parts > MAX_LAYERS then
    return false
  end
  for i = 1, #parts do
    local text = parts[i]
    if #text ~= 8 then
      return false
    end
    local any = false
    for quad = 0, 3 do
      local sc = text:sub(quad * 2 + 1, quad * 2 + 1)
      local cc = text:sub(quad * 2 + 2, quad * 2 + 2)
      if sc == "-" then
        if cc ~= "-" then
          return false
        end
      elseif char_in(sc, SUBCHARS) and char_in(cc, COLORCHARS) then
        any = true
      else
        return false
      end
    end
    if not any then
      return false
    end
  end
  return true
end

--- `decode(key)` → 层数组（新表，可安全改）；非法 shortkey 抛 `-KEY`。
function M.decode(key)
  if type(key) ~= "string" or #key == 0 then
    fail("KEY", "shortkey 必须非空字符串，得到 " .. type(key))
  end
  local parts = split_layers(key)
  if #parts > MAX_LAYERS then
    fail("KEY", "层数上限 " .. MAX_LAYERS .. "（得到 " .. #parts .. "）")
  end
  local layers = {}
  for i = 1, #parts do
    layers[i] = decode_layer(parts[i], key)
  end
  return layers
end

--- `encode(layers)` → 规范 shortkey（丢全空层）。
function M.encode(layers)
  if type(layers) ~= "table" then
    fail("KEY", "layers 必须是表")
  end
  if #layers > MAX_LAYERS then
    fail("KEY", "层数上限 " .. MAX_LAYERS .. "（得到 " .. #layers .. "）")
  end
  return encode_layers(layers)
end

--- `canonical(key)` → 规范形（decode+encode）；空项返回 `""`。
function M.canonical(key)
  if key == "" then
    return ""
  end
  return M.encode(M.decode(key))
end

--- `is_empty(key)`：空项或零有效层。
function M.is_empty(key)
  return key == ""
end

-- ============================================================================
-- 变换面（入参出参都是 shortkey 串）
-- ============================================================================

--- 象限过滤：保留 include 里的象限，丢全空层。
local function filter_quads(layers, include)
  local out = {}
  for i = 1, #layers do
    local quads = layers[i]
    local new_quads = {}
    local any = false
    for quad = 0, 3 do
      if include[quad] and quads[quad] ~= nil then
        new_quads[quad] = { s = quads[quad].s, c = quads[quad].c }
        any = true
      end
    end
    if any then
      out[#out + 1] = new_quads
    end
  end
  return out
end

--- `cut_half(key)` → `{ right, left }`。
--- 源序保留：right = 象限 [2,3]（bl+tl），left = 象限 [0,1]（tr+br）——
--- 源 shapeActionCutHalf 的命名/顺序照抄（物理左右与下标的关系见源码注释）。
function M.cut_half(key)
  local layers = M.decode(key)
  local right = encode_layers(filter_quads(layers, { [2] = true, [3] = true }))
  local left = encode_layers(filter_quads(layers, { [0] = true, [1] = true }))
  return { right = right, left = left }
end

--- `cut_quad(key)` → `{ [1..4] = key }`（象限 0..3 各自成项；空象限 = `""`）。
function M.cut_quad(key)
  local layers = M.decode(key)
  local out = {}
  for quad = 0, 3 do
    out[quad + 1] = encode_layers(filter_quads(layers, { [quad] = true }))
  end
  return out
end

--- 旋转：源 cloneRotateCW/CCW/180 的移位语义（新表，不改入参层）。
--- CW：new[q] = old[(q - 1) % 4]；CCW：new[q] = old[(q + 1) % 4]；
--- 180：new[q] = old[(q + 2) % 4]。
local function rotate(key, offset)
  local layers = M.decode(key)
  for i = 1, #layers do
    local old = layers[i]
    local new_quads = {}
    for quad = 0, 3 do
      local src = (quad + offset) % 4
      local item = old[src]
      if item ~= nil then
        new_quads[quad] = { s = item.s, c = item.c }
      end
    end
    layers[i] = new_quads
  end
  return encode_layers(layers)
end

function M.rotate_cw(key)
  return rotate(key, 3) -- new[0] = old[3]
end

function M.rotate_ccw(key)
  return rotate(key, 1) -- new[0] = old[1]
end

function M.rotate_180(key)
  return rotate(key, 2)
end

--- `stack(lower, upper)`：源 cloneAndStackWith 的层合并算法。
--- 逐象限找底层最高层 / 顶层最低层，gap = topLowest - bottomHighest，
--- mergeAt = max(1 - min(gap), 0)，顶层各层落到 mergeAt + layer，逐象限 OR，
--- 层数上限 4。全空入参抛 `-STACK`（源 assert 同口径）。
function M.stack(lower, upper)
  if lower == "" or upper == "" then
    fail("STACK", "空项不可堆叠")
  end
  local bottom = M.decode(lower)
  local top = M.decode(upper)

  local bottom_highest = { [0] = -1, -1, -1, -1 }
  for layer = #bottom, 1, -1 do
    for quad = 0, 3 do
      if bottom[layer][quad] ~= nil and bottom_highest[quad] < layer - 1 then
        bottom_highest[quad] = layer - 1
      end
    end
  end
  local top_lowest = { [0] = MAX_LAYERS, MAX_LAYERS, MAX_LAYERS, MAX_LAYERS }
  for layer = 1, #top do
    for quad = 0, 3 do
      if top[layer][quad] ~= nil and top_lowest[quad] > layer - 1 then
        top_lowest[quad] = layer - 1
      end
    end
  end

  local min_gap = MAX_LAYERS * 2
  for quad = 0, 3 do
    local gap = top_lowest[quad] - bottom_highest[quad]
    if gap < min_gap then
      min_gap = gap
    end
  end
  local merge_at = 1 - min_gap
  if merge_at < 0 then
    merge_at = 0
  end

  local merged = {}
  for layer = 1, #bottom do
    local quads = {}
    for quad = 0, 3 do
      local item = bottom[layer][quad]
      if item ~= nil then
        quads[quad] = { s = item.s, c = item.c }
      end
    end
    merged[layer] = quads
  end
  local needed = merge_at + #top
  for layer = #merged + 1, needed do
    merged[layer] = {}
  end
  for layer = 1, #top do
    local target = merged[merge_at + layer]
    for quad = 0, 3 do
      local bottom_item = target[quad]
      local top_item = top[layer][quad]
      if bottom_item ~= nil and top_item ~= nil then
        fail("STACK", "象限重叠（子形会丢失）")
      end
      if bottom_item == nil and top_item ~= nil then
        target[quad] = { s = top_item.s, c = top_item.c }
      end
    end
  end
  while #merged > MAX_LAYERS do
    merged[#merged] = nil
  end
  return encode_layers(merged)
end

--- `miner_shape(subshape, color)`：资源斑形状 = 4 同象限（源 MapResource cachedShape）。
function M.miner_shape(subshape, color)
  if not char_in(subshape, SUBCHARS) or not char_in(color, COLORCHARS) then
    fail("PATCH", "资源斑 subshape/color 非法：" .. tostring(subshape) .. tostring(color))
  end
  local quad = subshape .. color
  return quad .. quad .. quad .. quad
end

--- `hub_snapshot(layers_or_key)`：观察面摘要（层数 × 满象限数），供 narrate/meta。
function M.stat(key)
  if M.is_empty(key) then
    return { layers = 0, quads = 0 }
  end
  local layers = M.decode(key)
  local quads = 0
  for i = 1, #layers do
    for quad = 0, 3 do
      if layers[i][quad] ~= nil then
        quads = quads + 1
      end
    end
  end
  return { layers = #layers, quads = quads }
end

-- 依赖注入一致性：本模块不触宿主面（host 预留校验，selftest 无 ygg 也可用）。
function M.selfcheck()
  return host() ~= nil or true
end

return M
