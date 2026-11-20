-- `packages/libs/role-skill/window.lua` —— 近期流的滑动窗口（有界 ring + 计数读面）。
--
-- 契约：`docs/design/packages/role-skill.md` §2（`role.window.<id>` 键族）/
-- §3 I10（窗口与计数器分工）/ §4（`window_push` / `window`）。
--
-- # 为什么是独立存储（D12）
--
-- `counters` 只增（I5），「最近 n 次」在总量里已经被销毁：从它派生要么得保存无限
-- 历史（破有界存储），要么只是近似（静默错）。所以窗口是一条独立的 ring，按**插入
-- 序**成立，不碰任何时间面（I6）。它也绝不并进 actor 记录——窗口每次练习都写，混进
-- 记录会让每次 `charge` 重写整条 ring。
--
-- # 有界
--
-- ring 容量 = 安装期 `window_cap`（缺省 100）；超了就挤掉最旧的一条。`window` 的 `n`
-- 超过容量直接报 `-DEF`：静默截断会产出「确定性但错」的权重，比报错难归因得多。
--
-- # 标签是标签，不是引用
--
-- `label` 不要求已定义 / 已授予——窗口是统计流，不是引用面（契约 §4）；要求建引用
-- 会逼调用方为一个纯统计去注册一堆技能。

local store = require("pkg.role_skill.store")

local fail = store.fail
local E = store.E

local M = {}

--- 读 ring：无键 ⇒ `nil`；载荷不是表 ⇒ `-KV`（本包只写数组，坏形状说明不是我们写的）。
local function read_ring(id)
  local ring = store.read(store.window_key(id), nil)
  if ring == nil then
    return nil
  end
  if type(ring) ~= "table" then
    fail(E.KV, "键 " .. store.window_key(id) .. " 的载荷必须是字符串数组，得到 " .. type(ring))
  end
  return ring
end

--- 标签校验：非空字符串（空串会让「没推过」与「推过空标签」在计数面同形）。
local function check_label(label, where)
  if type(label) ~= "string" or label == "" then
    fail(E.DEF, where .. " 的 label 必须是非空字符串，得到 " .. tostring(label))
  end
  return label
end

--- `role.window_push(id, label)` → 推入后的 ring 长度（`window` 的容量判据看得到它）。
function M.push(id, label)
  check_label(label, "window_push")
  local cap = store.window_cap()
  local ring = read_ring(id) or {}
  ring[#ring + 1] = label
  if #ring > cap then
    local trimmed = {}
    local shift = #ring - cap
    for i = 1, cap do
      trimmed[i] = ring[i + shift]
    end
    ring = trimmed
  end
  store.write(store.window_key(id), ring)
  return #ring
end

--- `role.window(id, label, n)` → 最近 `n` 条里等于 `label` 的**条数**。
---
--- `n = 0` ⇒ 0；无键 ⇒ 0；标签不在 ⇒ 0；`n` 超史但 ≤ 容量 ⇒ 统计现有条目（合法）。
function M.count(id, label, n)
  check_label(label, "window")
  if type(n) ~= "number" or n ~= (n // 1) or n < 0 then
    fail(E.DEF, "window 的 n 必须是非负整数，得到 " .. tostring(n))
  end
  n = n | 0
  local cap = store.window_cap()
  if n > cap then
    fail(E.DEF, "window 的 n 不得超过 window_cap(" .. tostring(cap) .. ")，得到 " .. tostring(n))
  end
  if n == 0 then
    return 0
  end
  local ring = read_ring(id)
  if ring == nil then
    return 0
  end
  local total = #ring
  local from = total - n + 1
  if from < 1 then
    from = 1
  end
  local count = 0
  for i = from, total do
    if ring[i] == label then
      count = count + 1
    end
  end
  return count
end

return M
