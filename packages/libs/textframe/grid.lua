-- `packages/libs/textframe/grid.lua` —— 帧缓冲的全部实现：建帧 / 单格写 / 分层 / 行装配 / 块拼接。
--
-- 契约：`docs/design/packages/textframe.md` §2 / §3 / §4。
--
-- # 形状
--
--   frame = { w, h, cells = {…} }，元素按**行主序 1 基**取：`cells[(y - 1) * w + x]`。
--   单元格 = 恰 1 字节的字符串（可见字符），或 `nil` / `""`（空白且**透明**）。
--   `" "` 是实字空格，不是空白——`blit` 会覆盖它（T3 的三态，正反都钉在自测里）。
--
-- # 为什么写侧严格、读侧宽容
--
-- 写侧（`put`）只收「恰 1 字节」：多字节、多字符、空串一律 `-CELL`。空串尤其要拒——
-- 它会让「这一格有多宽」变成谎报，而宽度正是与 mod 侧 golden 帧的唯一接缝（I6）。
-- 读侧（`blit` / `to_lines`）容忍 `nil` 与 `""` 当空白：三处复现的事实输入里，
-- 未写过的格子就是不存在的键（CDDA 的稀疏 `layer`、Saltline 的 `cells[...] or " "`、
-- DCSS 的 `if ch == "" then ch = unseen`）。
--
-- # 拼接器（本包自带，不借 `pkg.assetgen.join`）
--
-- 逐段 `out = out .. chunk` 是 O(n²)；两两合并是 O(n·log n)，且只在最后一轮触碰整
-- 串。这不只是性能：一帧 30 行 × 80 列，naive 版每行把 80 个片段重抄一遍。十几行
-- 的实现换一条 `textframe → assetgen` 依赖边不划算，而「每包各自持有这几行」已是
-- codec / quest / entity / dialogue / assetgen 的既定先例（assetgen.md §8 裁过同一题）。

local M = {}

--- 错误码面（包内唯一一份码字面量；`init.lua` 对外做显式副本，不做 `pairs` 拷贝）。
M.E = {
  PARAM = "YGG-PKG-TEXTFRAME-PARAM",
  RANGE = "YGG-PKG-TEXTFRAME-RANGE",
  CELL = "YGG-PKG-TEXTFRAME-CELL",
  SHAPE = "YGG-PKG-TEXTFRAME-SHAPE",
}

local E = M.E

--- 报错：`<码>: <细节>`（level 0 = 不夹带调用点；用例按码做子串断言）。
local function fail(code, detail)
  error(code .. ": " .. detail, 0)
end

local I64_MAX = 0x7FFFFFFFFFFFFFFF

--- 整值判定：整值浮点（`3.0`）也算整数，带小数的、超 i64 的都不算。
---
--- 不用 `math.type`：沙箱里 `math` 是不可达的 `nil`（`crates/ygg-sandbox/src/sandbox.rs`
--- 的逐名断言），包代码一碰就崩。`v == v // 1` 是同一件事的算术写法。
local function is_int(value)
  return type(value) == "number" and value == (value // 1) and value <= I64_MAX and value >= -I64_MAX
end

--- 帧缓冲的形状校验；`cells` 缺省时补成空表（§2：缺省 `{}`）。
--- 返回 `w, h, cells`。**不校验 `#cells`**：数组在前缀处可断（未写过的格子本来就不存在键）。
local function check_frame(buf, who)
  if type(buf) ~= "table" then
    fail(E.SHAPE, who .. ": 不是帧缓冲（" .. type(buf) .. "）")
  end
  local w, h = buf.w, buf.h
  if not is_int(w) or w < 1 then
    fail(E.PARAM, who .. ": w 必须是 ≥1 的整数")
  end
  if not is_int(h) or h < 1 then
    fail(E.PARAM, who .. ": h 必须是 ≥1 的整数")
  end
  local cells = buf.cells
  if cells == nil then
    cells = {}
    buf.cells = cells
  end
  if type(cells) ~= "table" then
    fail(E.SHAPE, who .. ": cells 必须是表（" .. type(cells) .. "）")
  end
  return w, h, cells
end

--- 两两合并（原地覆写入参：调用方交出 parts 的所有权）。
local function concat(parts)
  local n = #parts
  if n == 0 then
    return ""
  end
  local width = 1
  while width < n do
    local index = 1
    while index <= n do
      local right = parts[index + width]
      if right ~= nil then
        parts[index] = parts[index] .. right
      end
      index = index + width * 2
    end
    width = width * 2
  end
  return parts[1]
end

--- 建帧：`{ w, h, cells = {} }`。空白态即「键不存在」。
function M.frame(w, h)
  if not is_int(w) or w < 1 then
    fail(E.PARAM, "frame: w 必须是 ≥1 的整数")
  end
  if not is_int(h) or h < 1 then
    fail(E.PARAM, "frame: h 必须是 ≥1 的整数")
  end
  return { w = w, h = h, cells = {} }
end

--- 单格写：1 基坐标；`ch = nil` 清格（回空白）。越界是调用方 bug ⇒ `-RANGE`。
function M.put(buf, x, y, ch)
  local w, h, cells = check_frame(buf, "put")
  if not is_int(x) or not is_int(y) then
    fail(E.PARAM, "put: x / y 必须是整数")
  end
  if x < 1 or x > w or y < 1 or y > h then
    fail(E.RANGE, ("put: (%s, %s) 越出 %sx%s"):format(tostring(x), tostring(y), tostring(w), tostring(h)))
  end
  if ch ~= nil and (type(ch) ~= "string" or #ch ~= 1) then
    fail(E.CELL, "put: 单元格必须是恰 1 字节的字符串或 nil（得到 " .. tostring(ch) .. "）")
  end
  cells[(y - 1) * w + x] = ch
  return buf
end

--- 分层叠加：`src` 的清格跳过，其余覆盖 `dst`（后写胜）；越界部分整块丢弃（**永不抛**）。
---
--- 坐标是 `src` 左上角在 `dst` 里的位置（可为负 = 左侧/顶部被裁掉）。
--- 读侧宽容：不逐格校验宽度（那是 `put` 的职责），`src` 里若混进多字节字符，
--- 会在 `to_lines` 的 I6 断言处现形，而不是在这里。
function M.blit(dst, src, ox, oy)
  local dw, dh, dcells = check_frame(dst, "blit(dst)")
  local sw, sh, scells = check_frame(src, "blit(src)")
  if not is_int(ox) or not is_int(oy) then
    fail(E.PARAM, "blit: ox / oy 必须是整数")
  end
  for y = 1, sh do
    local ty = oy + y - 1
    if ty >= 1 and ty <= dh then
      local sbase = (y - 1) * sw
      local tbase = (ty - 1) * dw
      for x = 1, sw do
        local tx = ox + x - 1
        if tx >= 1 and tx <= dw then
          local ch = scells[sbase + x]
          if ch ~= nil and ch ~= "" then
            dcells[tbase + tx] = ch
          end
        end
      end
    end
  end
  return dst
end

--- 行装配：`h` 行、每行恰 `w` 个字符、**不 trim 尾空格**（I6——改这里等于改帧摘要）。
---
--- # 为什么宽度断言在**行**上做，不在**格**上做
---
--- 逐格 `type(ch) ~= "string" or #ch ~= 1` 是 I5 的读侧副本，但它要跑 `w·h` 次
--- （80×24 = 1920 次/帧，实测吃掉约 45% 的合成时间）；而 I6 真正要守的是**行宽**——
--- 一格多字节必然让 `#line > w`，一格数字会被 `..` 悄悄拼接成多个字节，同样是 `#line` 变长。
--- 所以把断言放在拼完之后：每行一次 `#`，24 次/帧，成本可忽略，守的还是同一条不变式。
function M.to_lines(buf)
  local w, h, cells = check_frame(buf, "to_lines")
  local lines = {}
  for y = 1, h do
    local base = (y - 1) * w
    local parts = {}
    for x = 1, w do
      local ch = cells[base + x]
      if ch == nil or ch == "" then
        parts[x] = " "
      else
        parts[x] = ch
      end
    end
    local line = concat(parts)
    if #line ~= w then
      fail(E.CELL, ("to_lines: 第 %d 行宽 %d ≠ w=%d（单元格不是单字节？）"):format(y, #line, w))
    end
    lines[y] = line
  end
  return lines
end

--- 块拼接：块序 = 行序，`nil` 块跳过（HUD / 日志这类可选块因此不必先判空）。
function M.stack(blocks)
  if type(blocks) ~= "table" then
    fail(E.SHAPE, "stack: blocks 必须是表（" .. type(blocks) .. "）")
  end
  local out = {}
  for i = 1, #blocks do
    local block = blocks[i]
    if block ~= nil then
      if type(block) ~= "table" then
        fail(E.SHAPE, ("stack: blocks[%d] 不是行数组（%s）"):format(i, type(block)))
      end
      for j = 1, #block do
        local line = block[j]
        if type(line) ~= "string" then
          fail(E.SHAPE, ("stack: blocks[%d][%d] 不是字符串（%s）"):format(i, j, type(line)))
        end
        out[#out + 1] = line
      end
    end
  end
  return out
end

return M
