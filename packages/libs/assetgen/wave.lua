-- `packages/libs/assetgen/wave.lua` —— 波形核：Q15 相位 → [-32768, 32767]。
--
-- 契约：`docs/design/packages/assetgen.md` §2（c）/ §3 I1。
--
-- # 为什么相位是 1/65536 转的单位
--
-- 相位累加器要能做完一整圈而不失真：`65536` 正好是一个 16 位周期的容量，累加后
-- `& 0xFFFF` 就是取模，且 `f * 65536 // sr` 的中间量最大只有 `48000 × 65536 ≈ 3e9`
-- （i64 里九牛一毛）。换成「弧度」就要写 2π 的定点近似，多一层舍入。
--
-- # 峭度与占空
--
-- 四种波形都是**半开区间**定义（`p < 32768` 而不是 `<=`）：闭区间会让相邻周期在
-- 边界上出现一帧双值，而那是听不出来、但会让字节比较在改采样率时漂移的东西。

local mathx = require("pkg.assetgen.mathx")

local M = {}

M.KINDS = { saw = true, sine = true, square = true, triangle = true }

--- `wave.value(kind, phase)` → 该相位上的波形取值。
function M.value(kind, phase)
  local p = phase & 0xFFFF
  if kind == "sine" then
    return mathx.sin_q15(p)
  end
  if kind == "square" then
    if p < 32768 then
      return 32767
    end
    return -32768
  end
  if kind == "saw" then
    return p * 2 - 32768
  end
  -- triangle：上升 0..32766、下降 32766..-32768、回升 -32768..32766。
  if p < 16384 then
    return p * 2
  end
  if p < 49152 then
    return 32766 - (p - 16384) * 2
  end
  return (p - 49152) * 2 - 32768
end

return M
