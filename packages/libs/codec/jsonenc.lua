-- `pkg.codec.jsonenc` —— encode：确定性 JSON 编码（键序 = 字节序，无多余空白）。
--
-- 契约：`openwiki/contracts/packages/codec.md` §4.1。批十一拆分（纯搬移）：原生面委托与
-- 回退随本文件走——面在场 ⇒ `pcall` 调原生，`YGG-CODEC-UNSUPPORTED` ⇒ 落回 Lua
-- 路径产出规范字节与规范文案。骨架工具（join / sort_names / is_integer）住这里，
-- 是兄弟模块的唯一取用点（禁止抄第二份）。

local runtime = require("pkg.codec.runtime")

local fail = runtime.fail
local E_TYPE = runtime.E_TYPE
local E_HOST = runtime.E_HOST
local native_face = runtime.native_face

local M = {}

-- ── 无标准库的骨架工具（与 entity 各自 6 行，不抽第三个包）──────────────────

--- 数组 → 字符串。
---
--- **不能写成 `out = out .. parts[i]`**：Lua 字符串不可变，逐段追加是 O(n²)，
--- 1.6 MB 的 bundle 会跑到几十秒（实测：43 s）。改成两两合并（对数轮），每轮
--- 把元素数减半，总拷贝量为 O(总字节 × log n)，降到百毫秒级。
local function join(parts)
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

--- 原地归并排序（字节序升序）。键序 = 字节序是**契约 I1 的定义**，不是偏好：
--- 两张插入序不同的同值表必须产出同一份字节，而 `pairs` 序依赖哈希种子。
---
--- 为什么不是插入排序：它是 O(n²)，而 bundle 的 `kv` 段就是一条 2 000 键的宽表。
--- 实测（普通 Lua，逆序插入逼最坏路径）——插入排序时 `encode` 的耗时是
--- 250 键 2.5 ms / 1 000 键 17.7 ms / 2 000 键 51.4 ms / 4 000 键 174.8 ms
--- （键数 ×16 ⇒ 耗时 ×68.8，纯二次行为），其中 2 000 键那次有 ~51 ms 只是排序。
--- 归并排序把这段降到 O(n log n)，且交换律上更省：每轮只做一遍拷贝。
---
--- 写法约束：沙箱没有 `table.sort`（也没有 `string` 表），所以归并手写；
--- 原地语义必须保住——调用方 `sort_names(keys)` 不接返回值。
---
--- **缓冲复用**（不是随手写）：`scratch` 若每次调用新建，一次 `codec.encode` 会为
--- 每张对象表留一份未回收的垃圾——1 000 张 6 键表 ⇒ Lua 堆涨 **+270 KiB**（实测
--- 998 KiB vs 728 KiB），正好把 `pkg.role-skill` 的「1k actor 载荷 ≤ 1 MiB」结构
--- 断言顶红。复用是安全的：一次排序跑完才返回，排序期间不会发生第二次排序
---（递归发生在排序之后），所以不存在重入。
local SORT_SCRATCH = {}

local function sort_names(items)
  local count = #items
  if count < 2 then
    return items
  end
  local source, target = items, SORT_SCRATCH
  local width = 1
  while width < count do
    local left = 1
    while left <= count do
      local middle = left + width - 1
      if middle > count then
        middle = count
      end
      local right = middle + width
      if right > count then
        right = count
      end
      local i, j, write = left, middle + 1, left
      while i <= middle and j <= right do
        if source[i] <= source[j] then
          target[write] = source[i]
          i = i + 1
        else
          target[write] = source[j]
          j = j + 1
        end
        write = write + 1
      end
      while i <= middle do
        target[write] = source[i]
        i = i + 1
        write = write + 1
      end
      while j <= right do
        target[write] = source[j]
        j = j + 1
        write = write + 1
      end
      left = left + width * 2
    end
    source, target = target, source
    width = width * 2
  end
  if source ~= items then
    for index = 1, count do
      items[index] = source[index]
    end
  end
  return items
end

--- 数是不是 Lua 5.4 的 **integer**。
---
--- 为什么必须自己判：沙箱不保证注入面那个整/浮类型函数（`math` 只由驱动注入，包不得
--- 依赖它，见文件头第 1 条），而整数与浮点在算术、比较、`//`、`%`、位运算下完全不可区分
--- （`1 == 1.0`、`1 // 1 == 1.0 // 1`、`("%d"):format(1) == ("%d"):format(1.0)`）。能
--- 区分身份的只剩文本形态：Lua 5.4 整数按 `%d` 打印（无 `.`、无 `e`），浮点按 `%.14g`
--- 打印且「整数形」补 `.0`（`tostring(1)="1"` / `tostring(1.0)="1.0"` /
--- `tostring(1e15)="1e+15"`）。
---
--- 为什么不能写 `v % 1 == 0`：`2.0 % 1 == 0.0`，整值浮点会被吞成整型。而本包的 KV/JSON
--- 面里 `1` 与 `1.0` 是**两个字节串**，吞掉一次就是字节稳定当场破。
---
--- 只用 `:byte` 不引入 pattern：`string` 全局在沙箱缺席，但字符串方法经
--- `getmetatable("").__index` 可达（与 `entity` 的 `name_ok` 同一风格）。
---
--- 残留依赖（如实声明）：判据依赖 `tostring` 的 `%.14g`——与 `number_text` 里
--- `NUMBER_FORMATS` 的 `%g` 是**同一依赖面**，不是新增风险类别。
local function is_integer(v)
  if type(v) ~= "number" or v - v ~= 0 then -- 非数 / ±inf / NaN 一律 false
    return false
  end
  local text = tostring(v)
  for i = 1, #text do
    local b = text:byte(i)
    if b == 46 or b == 101 then -- '.' / 'e'
      return false
    end
  end
  return true
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
---
--- 扫描方式：用 pattern 一次跳到**下一个需要转义的字节**（C 层扫描），把中间的
--- 整段原字一次性取出——逐字节 `sub(i, i)` 会为 1.6 MB 产出百万级片段，拼接与
--- 内存都会掌不住（实测：这种写法让 encode 掉到 0.04 MB/s）。
local function string_text(text)
  local out = { '"' }
  local start = 1
  local length = #text
  while true do
    local at = text:find('["\\%c]', start)
    if at == nil then
      break
    end
    if at > start then
      out[#out + 1] = text:sub(start, at - 1)
    end
    local byte = text:byte(at)
    local escape = ESCAPES[byte]
    if escape ~= nil then
      out[#out + 1] = escape
    else
      out[#out + 1] = ("\\u%04x"):format(byte)
    end
    start = at + 1
  end
  if start <= length then
    out[#out + 1] = text:sub(start)
  end
  out[#out + 1] = '"'
  return join(out)
end

-- 浮点的精度试探格式串：提到模块级，避开每次尝试都新建一个格式串。
-- （导出侧每个数字都要试 1..17 次，10k × 8 属性就是几十万次。）
local NUMBER_FORMATS = {}
for precision = 1, 17 do
  NUMBER_FORMATS[precision] = "%." .. precision .. "g"
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
  if value - value ~= 0 then
    fail(E_TYPE, "±inf 不是可编码的值")
  end
  if is_integer(value) then
    return tostring(value)
  end
  if value == 0.0 then
    return "0.0" -- 覆盖 -0.0：零只有一个存诸形
  end
  local magnitude = value < 0 and -value or value
  local allow_exponent = not (magnitude >= 1e-4 and magnitude < 1e15)
  local text = nil
  for precision = 1, 17 do
    local candidate = NUMBER_FORMATS[precision]:format(value)
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
      if not is_integer(key) or key < 1 then
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
  local face = native_face()
  if face ~= nil then
    local ok, text = pcall(face.encode, value)
    if ok then
      return text
    end
    -- `UNSUPPORTED`（值域拒绝 / 带元方法的表）⇒ 落到下面，用 Lua 路径产出规范
    -- 字节与规范文案；其余错误是原生实现的内部失败，必须响（不能静默降级）。
    --
    -- 判据必须先 `tostring`：宿主的 runtime error 过 mlua 边界后可能是字符串、
    -- 也可能是带 `__tostring` 的错误对象（mlua 的两种路径都出现过），直接对
    -- 错误值调字符串方法会在「错误对象」那一支上炸成另一个错。
    if not tostring(text):find("YGG-CODEC-UNSUPPORTED", 1, true) then
      fail(E_HOST, "原生面内部失败：" .. tostring(text))
    end
  end
  local out = {}
  encode_into(value, out, 0, {}, "$")
  return join(out)
end

-- ── 导出面（jsondec 取 join；importing 取 is_integer / sort_names）────────────

M.join = join
M.sort_names = sort_names
M.is_integer = is_integer

return M

