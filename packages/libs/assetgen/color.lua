-- `packages/libs/assetgen/color.lua` —— `0xRRGGBB` 整数的通道运算（调色）。
--
-- 契约：`docs/design/packages/assetgen.md` §2（纹理 `ch` 三值的来源）。
--
-- 颜色在产物路径上是**一个整数**而不是三元素数组：数组会让每个像素多一次表访问
-- （64×64×4 就是 16k 次），而整数解包只是两次位运算。千分比权重（`1000` = 1.0）
-- 是本包全境的缩放口径——与 `mathx` 的 Q15 分工明确：Q15 给三角，千分比给人读参数。

local M = {}

--- `0xRRGGBB` → `r, g, b`（各 0..255）。
function M.unpack(rgb)
  return (rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF
end

--- `r, g, b` → `0xRRGGBB`（超出 8 位的部分截断，不报错——调用方已经过值域校验）。
function M.pack(r, g, b)
  return ((r & 0xFF) << 16) | ((g & 0xFF) << 8) | (b & 0xFF)
end

--- 千分比权重的逐通道线性插值：`t = 0` 给 `a`，`t = 1000` 给 `b`。
---
--- 用 `//` 而不是 `/`：负数中间量（`b < a`）下 `//` 是**向下取整**，结果仍然确定；
--- 换成浮点除法就会把舍入模式带进产物。
function M.lerp(a, b, t)
  local ar, ag, ab = M.unpack(a)
  local br, bg, bb = M.unpack(b)
  local function mix(x, y)
    return x + ((y - x) * t) // 1000
  end
  return M.pack(mix(ar, br), mix(ag, bg), mix(ab, bb))
end

--- 亮度调制：`permille = 1000` 恒等，`0` 全黑，`2000` 双倍（通道各自截断到 255）。
function M.scale(rgb, permille)
  local r, g, b = M.unpack(rgb)
  local function fade(x)
    local v = (x * permille) // 1000
    if v > 255 then
      return 255
    end
    return v
  end
  return M.pack(fade(r), fade(g), fade(b))
end

--- 亮度（0..255）：`0.299R + 0.587G + 0.114B`，用千分比整数权重。
function M.luma(rgb)
  local r, g, b = M.unpack(rgb)
  return (r * 299 + g * 587 + b * 114) // 1000
end

return M
