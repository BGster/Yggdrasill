-- `pkg.codec` —— 确定性 JSON 编解码与存档往返。契约：`docs/design/packages/codec.md`。
--
-- # 这个包做什么
--
-- 两个正交问题，两套函数：**形状**（谁是字段、谁是引用）归 `pkg.entity`，
-- **字节**（同一输入能否产出同一输出）归本包。混在一起时，任何「让 JSON 好看
-- 一点」的键序调整都会变成实体契约的变更——这是契约 §8 的反方案一。
--
-- # 实现纪律（沙箱方言；契约 §8 记着为什么）
--
-- 1. 沙箱没有 `string` / `table` / `utf8` 全局（`sandbox.rs` 白名单测试钉死）：
--    字符串只能走方法语法，表工具手写。出现标准库表的点式调用即会在真实
--    跑局里崩，而 selftest 看不出来——交付门禁带静态 grep。
-- 2. 模块顶层纯：只 local / 常量 / 函数定义。`ygg` 访问一律发生在调用期，经
--    `host()` 单点取句柄（契约 I8：`install` 不预读、不预写）。
-- 3. **键序 = 字节序升序**（契约 I1）：不排序就没有确定性，没有确定性就没有
--    「同 seed 同产物」。
-- 4. **不读 `pkg.entity.*` 任何键**：记录只经 `pkg.entity` 的 API 取（布局单源在
--    entity；出现 `pkg.entity.rec` 这类键名即视为布局泄漏）。
--
-- 依赖序单向：codec → entity（`packages/README.md` §3）。

local entity = require("pkg.entity")
local sha256 = require("pkg.codec.sha256")

local M = { VERSION = "0.1.0" }

-- 依赖记位（`install` 写入；`host()` 读）。
M.deps = nil

-- 错误码家族 `YGG-PKG-CODEC-<COND>`（契约固定 TYPE / CONFLICT / CHECKSUM；
-- MISSING / VERSION / HOST 是本实现补齐的家族成员，见 codec.md §8）。
local E_TYPE = "YGG-PKG-CODEC-TYPE"

--- 报错：`<码>: <细节>`（level 0 = 不夹带调用点，测试按码做子串断言）。
local function fail(code, detail)
  error(code .. ": " .. detail, 0)
end

-- ── 宿主句柄（install 只记 deps；契约 I8 的「不预读不预写」）────────────────────

--- 装配：记下可选依赖（`{ host = ... }`；缺省走 `ygg` 全局）。
---
--- `M.deps` 是**公开字段**而不是私有 local：装进去的东西应该看得见（用例靠它
--- 断言装的是哪份 mock；调度处也靠它排查「到底装了没」）。
function M.install(deps)
  M.deps = { host = (type(deps) == "table" and deps.host) or nil }
  return M
end

--- `codec.sha256(bytes)`：纯 Lua SHA-256（64 位小写 hex）。
---
--- 为何自己写而不取宿主 `fs.write` 的回值：那一值只能**落盘之后**得到，而
--- `checksum(bundle)` 必须在落盘前就能算（导出侧要把校验和写进 bundle 本体），
--- 且「导入前先校验」要求纯读路径能复核——依赖宿主回值就证明不了「盘上没被改
--- 过」，而那正是 checksum 存在的理由（D11 的退役依据）。详见 `sha256.lua` 头注。
---
--- 输入域在包边界收窄：非字符串（表 / 函数 / nil）在进哈希前就报
--- `YGG-PKG-CODEC-TYPE`，而不是变成 `tostring(table)` 这种带地址的字节。
function M.sha256(bytes)
  if type(bytes) ~= "string" then
    fail(E_TYPE, "sha256 只收字符串，得到 " .. type(bytes))
  end
  return sha256.sha256(bytes)
end

-- ── 无标准库的骨架工具（与 entity 各自 6 行，不抽第三个包）──────────────────

--- 数组 → 字符串（沙箱里没有拼接库，只能手写）。
local function join(parts)
  local out = ""
  for i = 1, #parts do
    out = out .. parts[i]
  end
  return out
end

--- 原地插入排序（字节序升序）。键序 = 字节序是**契约 I1 的定义**，不是偏好：
--- 两张插入序不同的同值表必须产出同一份字节，而 `pairs` 序依赖哈希种子。
local function sort_names(items)
  for i = 2, #items do
    local value = items[i]
    local j = i - 1
    while j >= 1 and items[j] > value do
      items[j + 1] = items[j]
      j = j - 1
    end
    items[j + 1] = value
  end
  return items
end

-- ── encode：确定性 JSON ────────────────────────────────────────────────────

-- 字符串转义表（`\u00XX` 之外的控制字符与引号/反斜杠）。
local ESCAPES = {
  [34] = '\\"',
  [92] = "\\\\",
  [10] = "\\n",
  [13] = "\\r",
  [9] = "\\t",
  [8] = "\\b",
  [12] = "\\f",
}

--- 字符串 → JSON 字面量。
---
--- 非 ASCII 原样输出（UTF-8 字节直通）：多一层 `\uXXXX` 就只能多一个出错面，而
--- 确定性判据不需要它。控制字符用 `\u00XX`（JSON 强制），其余保留原字。
local function string_text(text)
  local out = { '"' }
  for i = 1, #text do
    local byte = text:byte(i)
    local escape = ESCAPES[byte]
    if escape ~= nil then
      out[#out + 1] = escape
    elseif byte < 32 then
      out[#out + 1] = ("\\u%04x"):format(byte)
    else
      out[#out + 1] = text:sub(i, i)
    end
  end
  out[#out + 1] = '"'
  return join(out)
end

--- 数字 → 最短往返字节。
---
--- 三条规则缺一不可：① 整数与浮点**分开写**（`1` vs `1.0`，否则 `1.0` 解回整数，
--- 破字节稳定）；② 浮点取**第一个能往返的精度**（`%.*g` 从 1 试到 17）——固定 17
--- 位会把 `0.1` 写成 `0.10000000000000001`，白白多 15 个字节；③ `-0.0` 归一到 `0.0`
--- （与内核 `normalize_f64` 同口径：存诸面只有一个零）。
---
--- 指数形式的取舍是一条**确定规则**而不是启发：量级在 `[1e-4, 1e15)` 内时优先十进制
--- （`100.0` 写成 `100.0` 而不是 `1e02`），超出这个区间才允许 `%g` 的指数输出。
local function number_text(value)
  if value ~= value then
    fail(E_TYPE, "NaN 不是可编码的值")
  end
  if value == math.huge or value == -math.huge then
    fail(E_TYPE, "±inf 不是可编码的值")
  end
  if math.type(value) == "integer" then
    return tostring(value)
  end
  if value == 0.0 then
    return "0.0" -- 覆盖 -0.0：零只有一个存诸形
  end
  local magnitude = value < 0 and -value or value
  local allow_exponent = not (magnitude >= 1e-4 and magnitude < 1e15)
  local text = nil
  for precision = 1, 17 do
    local candidate = ("%." .. precision .. "g"):format(value)
    if tonumber(candidate) == value then
      local uses_exponent = candidate:find("[eE]") ~= nil
      if allow_exponent or not uses_exponent then
        text = candidate
        break
      end
    end
  end
  text = text or ("%.17g"):format(value)
  text = text:gsub("E", "e"):gsub("%+", "")
  if not text:find("%.") and not text:find("e") then
    text = text .. ".0" -- 没有小数点也没有指数 ⇒ 加 `.0` 保住「浮点」身份
  end
  return text
end

--- 表的分类：`"array"`（正整数键 1..n 无洞） / `"object"`（全字符串键）。
---
--- 混合与洞都是错（契约 I7）：混合无法用 JSON 表达，洞一旦静默变 `null` 就读不回
--- 原形状——「拒绝」是唯一不让错误往后跑的选择。
local function table_kind(value, path)
  local array_count = 0
  local object_count = 0
  local max_index = 0
  for key in pairs(value) do
    if type(key) == "number" then
      if math.type(key) ~= "integer" or key < 1 then
        fail(E_TYPE, "表的键必须是正整数或字符串（在 " .. path .. " 遇到 " .. tostring(key) .. "）")
      end
      array_count = array_count + 1
      if key > max_index then
        max_index = key
      end
    elseif type(key) == "string" then
      object_count = object_count + 1
    else
      fail(E_TYPE, "表的键必须是正整数或字符串（在 " .. path .. " 遇到 " .. type(key) .. " 键）")
    end
  end
  if array_count > 0 and object_count > 0 then
    fail(E_TYPE, "同一张表不得混用数组键与字符串键（在 " .. path .. "）")
  end
  if array_count > 0 and max_index ~= array_count then
    fail(
      E_TYPE,
      "数组有洞（在 " .. path .. "：最大下标 " .. max_index .. "，条目数 " .. array_count .. "）"
    )
  end
  if array_count > 0 then
    return "array", max_index
  end
  return "object", 0
end

local MAX_DEPTH = 64

local function encode_into(value, out, depth, stack, path)
  local kind = type(value)
  if value == nil then
    fail(E_TYPE, "nil 不是可编码的值（在 " .. path .. "；对象键用「不存在」表达）")
  end
  if kind == "boolean" then
    out[#out + 1] = value and "true" or "false"
    return
  end
  if kind == "number" then
    out[#out + 1] = number_text(value)
    return
  end
  if kind == "string" then
    out[#out + 1] = string_text(value)
    return
  end
  if kind ~= "table" then
    fail(E_TYPE, "类型 " .. kind .. " 不可编码（在 " .. path .. "）")
  end
  if stack[value] then
    fail(E_TYPE, "表循环引用（在 " .. path .. "）")
  end
  if depth > MAX_DEPTH then
    fail(E_TYPE, "嵌套超过 " .. MAX_DEPTH .. " 层（在 " .. path .. "）")
  end
  stack[value] = true

  local shape, length = table_kind(value, path)
  if shape == "array" then
    out[#out + 1] = "["
    for i = 1, length do
      if i > 1 then
        out[#out + 1] = ","
      end
      encode_into(value[i], out, depth + 1, stack, path .. "[" .. i .. "]")
    end
    out[#out + 1] = "]"
  else
    local keys = {}
    for key in pairs(value) do
      keys[#keys + 1] = key
    end
    sort_names(keys)
    out[#out + 1] = "{"
    for i = 1, #keys do
      local key = keys[i]
      if i > 1 then
        out[#out + 1] = ","
      end
      out[#out + 1] = string_text(key)
      out[#out + 1] = ":"
      encode_into(value[key], out, depth + 1, stack, path .. "." .. key)
    end
    out[#out + 1] = "}"
  end

  stack[value] = nil
end

--- `codec.encode(value)`：确定性 JSON 字节（键序 = 字节序，无多余空白）。
---
--- 同一个值的两次 `encode` 必须**逐字节**相等（契约 I1）；这条不是「好看」，是
--- 「同 seed 同产物」在序列化面的同一条纪律。
function M.encode(value)
  local out = {}
  encode_into(value, out, 0, {}, "$")
  return join(out)
end

return M
