-- `packages/libs/assetgen/mathx.lua` —— 整数数学核（产物路径「零浮点」的支点）。
--
-- 契约：`openwiki/contracts/packages/assetgen.md` §3 I1 / §8。
--
-- 沙箱 globals 里 `math` 表**不可达**（`crates/ygg-sandbox/src/sandbox.rs` 的逐名
-- 断言钉死），所以三角与开方只能自备。这不是障碍而是收获：整数域让「同 seed 同
-- 字节」变成**构造性**成立，不需要论证 libm 尾位差、FMA 收缩或舍入模式。
--
-- # 三条语言约束
--
-- 1. `//` 是唯一的整数除法；`/` 是浮点除法，产物路径禁用（静态用例 X3 扫源码）。
-- 2. `>>` 是**逻辑**右移（`(-1) >> 1` = 2^63-1）；象限拆分与状态推进都靠它。
-- 3. 溢出**回绕不报错**：所有中间量都落在 i64 内（最大是正弦近似的 1.06e15）。

local M = {}

local Q15 = 32768 -- 1.0 的定点表示
local QUARTER = 16384 -- 一象限的相位宽度（全转 = 65536 = 2^16）
local DEG_QUARTER = 90000 -- 一象限 = 90 度，以**毫度**为单位
local BHASKARA_DEN = 40500000000 -- 40500 × 10^6：把毫度带进 Bhaskara 的分母

function M.abs(v)
  if v < 0 then
    return -v
  end
  return v
end

function M.min(a, b)
  if a < b then
    return a
  end
  return b
end

function M.max(a, b)
  if a > b then
    return a
  end
  return b
end

function M.clamp(v, lo, hi)
  if v < lo then
    return lo
  end
  if v > hi then
    return hi
  end
  return v
end

--- `(a * b) // c`，不溢出：把 `b` 拆成 `b = q*c + r`，`floor(a*b/c) = a*q + floor(a*r/c)`。
---
--- 拆半的正确性对负数同样成立（`a*q` 是整数，floor 只作用在 `a*r/c` 上）。用它的地方
--- 都是「分子会到 2^60 以上」的缩放，直接相乘会静默回绕 —— 回绕后的产物依然确定，
--- 但确定地**错**，这是最难归因的一类缺陷。
function M.mul_div(a, b, c)
  local q = b // c
  local r = b % c
  return a * q + (a * r) // c
end

--- 整数牛顿开方：`isqrt(n)` = `floor(sqrt(n))`（`n < 0` 返回 0）。
function M.isqrt(n)
  if n <= 0 then
    return 0
  end
  local x = n
  local y = (x + 1) // 2
  while y < x do
    x = y
    y = (x + n // x) // 2
  end
  return x
end

--- smoothstep（Q15 入出）：`3t² − 2t³`，端点 0/1 且一阶导为 0。
---
--- 用它而不是线性插值，是为了让格点噪声不出现「网格线」：线性插值在格点处导数不连续，
--- 64×64 的贴图上肉眼可见。整数版本的截断误差 ≤ 1/32768，不影响任何断言。
function M.smoothstep_q15(t)
  local square = (t * t) // Q15
  return (square * (3 * Q15 - 2 * t)) // Q15
end

--- 第一象限的正弦（Q15），`d ∈ [0, 16384]` → `0..32767`。
---
--- Bhaskara I 的有理近似：`sin(θ°) ≈ 4θ(180−θ) / (40500 − θ(180−θ))`。最大绝对误差
--- 约 1.7e-3（θ=0 与 θ=90 处精确为 0 与 1），在 8–48 kHz 的音效上不可听，且**完全
--- 整数**。选它而不是 256 项字面量表：表要吃掉源码预算，而确定性收益为零。
local function quarter_sin(d)
  local deg = (d * DEG_QUARTER) // QUARTER -- 毫度，0..90000
  local rest = 180000 - deg
  local num = 4 * deg * rest
  return (num * 32767) // (BHASKARA_DEN - deg * rest)
end

--- `sin_q15(phase)` → `[-32767, 32767]`；`phase` 以 1/65536 转为单位（负数由 `&` 归一）。
---
--- 象限折返：`q=1` 与 `q=3` 用 `QUARTER - r` 反射到第一象限，`q≥2` 取负。折返而不是
--- 「算到 2π 再取模」的理由：折返每一步都是小整数，不会把 2^32 量级的相位带进乘法。
function M.sin_q15(phase)
  local p = phase & 0xFFFF
  local quadrant = p >> 14
  local r = p & (QUARTER - 1)
  if quadrant == 0 then
    return quarter_sin(r)
  end
  if quadrant == 1 then
    return quarter_sin(QUARTER - r)
  end
  if quadrant == 2 then
    return -quarter_sin(r)
  end
  return -quarter_sin(QUARTER - r)
end

--- `n / 32768`，四舍五入到最近整数。
---
--- 用「加半再向下取整」而不是裸 `//`：Q15 的 1.0 是 32767，裸除会让每一次旋转都
--- 系统性缩小 0.03‰（一个 1000mm 的盒子转一次变成 999mm，`mesh.compose` 的包围盒
--- 断言会以「差 1」的形式红掉）。四舍五入把偏差压到 ±0.5，且对负数同样确定。
function M.q15_div(n)
  return (n + QUARTER) // Q15
end

--- `cos_q15(phase)` = `sin_q15(phase + 一象限)`：正弦移相 **90°**（不是 180°——半转移的
--- 是符号，会把余弦变成零，而零值构造出来的圆柱/环面/球会全部塌到一条轴上）。
function M.cos_q15(phase)
  return M.sin_q15(phase + QUARTER)
end

return M
