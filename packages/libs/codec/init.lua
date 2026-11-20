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
local E_CONFLICT = "YGG-PKG-CODEC-CONFLICT"
local E_CHECKSUM = "YGG-PKG-CODEC-CHECKSUM"
local E_MISSING = "YGG-PKG-CODEC-MISSING"
local E_VERSION = "YGG-PKG-CODEC-VERSION"
local E_HOST = "YGG-PKG-CODEC-HOST"

-- 本实现写出的 `codec_version`。跨版本不做迁移（契约 §1：只拒，不猜）。
local CODEC_VERSION = "1"

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

--- 取本次调用的宿主句柄（`deps.host` 优先，其次 `ygg` 全局）。
local function host()
  local current = M.deps and M.deps.host or nil
  if current == nil then
    current = ygg
  end
  if current == nil then
    fail(E_HOST, "ygg 不可达：沙箱外调用需 `install({ host = ... })`")
  end
  return current
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

-- ── decode：递归下降 + 位置化报错 ───────────────────────────────────────────────
--
-- 报错必须带 `at byte N`：坏档的排查现场只有一份文件，而「某个字段是 nil」把
-- 定位工作全推给人；字节偏移能把一个十几万字节的存档缩到一行。

--- JSON `null` 的内部哨兵：对象里的 null 值**丢弃**（Lua 无 null，“不存在”就是
--- 这个意思，与 encode 侧对偶）；数组元素位置的 null 直接报错（丢不得）。
local NULL = {}

local function parse_error(at, detail)
  fail(E_TYPE, detail .. " at byte " .. tostring(at))
end

local function skip_space(text, pos, length)
  while pos <= length do
    local byte = text:byte(pos)
    if byte == 32 or byte == 9 or byte == 10 or byte == 13 then
      pos = pos + 1
    else
      break
    end
  end
  return pos
end

--- Unicode 码位 → UTF-8 字节（`utf8` 全局在沙箱不可达，手写四档）。
local function utf8_char(code)
  if code < 0x80 then
    return ("%c"):format(code)
  end
  if code < 0x800 then
    return ("%c%c"):format(0xC0 | (code >> 6), 0x80 | (code & 0x3F))
  end
  if code < 0x10000 then
    return ("%c%c%c"):format(
      0xE0 | (code >> 12),
      0x80 | ((code >> 6) & 0x3F),
      0x80 | (code & 0x3F)
    )
  end
  return ("%c%c%c%c"):format(
    0xF0 | (code >> 18),
    0x80 | ((code >> 12) & 0x3F),
    0x80 | ((code >> 6) & 0x3F),
    0x80 | (code & 0x3F)
  )
end

local parse_value

-- 单字节转义：`\b \f \n \r \t` → 码位。提到模块级是因为 decode 在吞吐路径上，
-- 每次转义都现建一张表纯属浪费。
local SIMPLE_ESCAPES = { [98] = 8, [102] = 12, [110] = 10, [114] = 13, [116] = 9 }

local function parse_string(text, pos, length)
  pos = pos + 1 -- 跳过开引号
  local out = {}
  while true do
    if pos > length then
      parse_error(pos, "字符串未闭合")
    end
    local byte = text:byte(pos)
    if byte == 34 then
      return join(out), pos + 1
    end
    if byte == 92 then
      local escape = text:byte(pos + 1)
      if escape == nil then
        parse_error(pos, "转义序列被截断")
      end
      if escape == 34 then
        out[#out + 1] = '"'
        pos = pos + 2
      elseif escape == 92 then
        out[#out + 1] = "\\"
        pos = pos + 2
      elseif escape == 47 then
        out[#out + 1] = "/"
        pos = pos + 2
      elseif escape == 98 or escape == 102 or escape == 110 or escape == 114 or escape == 116 then
        out[#out + 1] = ("%c"):format(SIMPLE_ESCAPES[escape])
        pos = pos + 2
      elseif escape == 117 then
        local hex = text:sub(pos + 2, pos + 5)
        local code = #hex == 4 and tonumber(hex, 16) or nil
        if code == nil then
          parse_error(pos, "\\u 转义需要 4 位十六进制")
        end
        pos = pos + 6
        if code >= 0xD800 and code <= 0xDBFF then
          if text:sub(pos, pos + 1) ~= "\\u" then
            parse_error(pos, "高位代理后缺少低位代理")
          end
          local low = tonumber(text:sub(pos + 2, pos + 5), 16)
          if low == nil or low < 0xDC00 or low > 0xDFFF then
            parse_error(pos, "低位代理非法")
          end
          pos = pos + 6
          code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00)
        end
        out[#out + 1] = utf8_char(code)
      else
        parse_error(pos, "非法转义 \\" .. tostring(escape))
      end
    else
      if byte < 32 then
        parse_error(pos, "字符串里有未转义的控制字符")
      end
      out[#out + 1] = text:sub(pos, pos)
      pos = pos + 1
    end
  end
end

local function parse_number(text, pos, length)
  local start = pos
  if text:byte(pos) == 45 then
    pos = pos + 1
  end
  local digits = 0
  while pos <= length do
    local byte = text:byte(pos)
    if byte >= 48 and byte <= 57 then
      pos = pos + 1
      digits = digits + 1
    else
      break
    end
  end
  if digits == 0 then
    parse_error(start, "数字缺整数部分")
  end
  if pos <= length and text:byte(pos) == 46 then
    pos = pos + 1
    local fraction = 0
    while pos <= length do
      local byte = text:byte(pos)
      if byte >= 48 and byte <= 57 then
        pos = pos + 1
        fraction = fraction + 1
      else
        break
      end
    end
    if fraction == 0 then
      parse_error(start, "小数点后无数字")
    end
  end
  if pos <= length then
    local byte = text:byte(pos)
    if byte == 101 or byte == 69 then
      pos = pos + 1
      local sign = text:byte(pos)
      if sign == 43 or sign == 45 then
        pos = pos + 1
      end
      local exponent = 0
      while pos <= length do
        local digit = text:byte(pos)
        if digit >= 48 and digit <= 57 then
          pos = pos + 1
          exponent = exponent + 1
        else
          break
        end
      end
      if exponent == 0 then
        parse_error(start, "指数无数字")
      end
    end
  end
  local lexeme = text:sub(start, pos - 1)
  local number = tonumber(lexeme)
  if number == nil then
    parse_error(start, "数字无法解析：" .. lexeme)
  end
  -- `tonumber` 保形：`1` → integer，`1.0` / `1e2` → float（与 encode 侧对偶）
  return number, pos
end

parse_value = function(text, pos, length)
  pos = skip_space(text, pos, length)
  if pos > length then
    parse_error(pos, "意外的输入结束")
  end
  local byte = text:byte(pos)

  if byte == 34 then
    return parse_string(text, pos, length)
  end

  if byte == 123 then -- {
    local object = {}
    pos = skip_space(text, pos + 1, length)
    if pos <= length and text:byte(pos) == 125 then
      return object, pos + 1
    end
    while true do
      pos = skip_space(text, pos, length)
      if pos > length or text:byte(pos) ~= 34 then
        parse_error(pos, "对象的键必须是字符串")
      end
      local key
      key, pos = parse_string(text, pos, length)
      pos = skip_space(text, pos, length)
      if pos > length or text:byte(pos) ~= 58 then
        parse_error(pos, "键后缺冒号")
      end
      local value
      value, pos = parse_value(text, pos + 1, length)
      if value ~= NULL then
        object[key] = value
      end
      pos = skip_space(text, pos, length)
      if pos > length then
        parse_error(pos, "对象未闭合")
      end
      local next_byte = text:byte(pos)
      if next_byte == 125 then
        return object, pos + 1
      end
      if next_byte ~= 44 then
        parse_error(pos, "对象成员之间缺逗号")
      end
      pos = pos + 1
    end
  end

  if byte == 91 then -- [
    local array = {}
    local count = 0
    pos = skip_space(text, pos + 1, length)
    if pos <= length and text:byte(pos) == 93 then
      return array, pos + 1
    end
    while true do
      local value
      value, pos = parse_value(text, pos, length)
      if value == NULL then
        parse_error(pos, "数组元素不得为 null（Lua 无 null 槽位）")
      end
      count = count + 1
      array[count] = value
      pos = skip_space(text, pos, length)
      if pos > length then
        parse_error(pos, "数组未闭合")
      end
      local next_byte = text:byte(pos)
      if next_byte == 93 then
        return array, pos + 1
      end
      if next_byte ~= 44 then
        parse_error(pos, "数组元素之间缺逗号")
      end
      pos = pos + 1
    end
  end

  if text:sub(pos, pos + 3) == "true" then
    return true, pos + 4
  end
  if text:sub(pos, pos + 4) == "false" then
    return false, pos + 5
  end
  if text:sub(pos, pos + 3) == "null" then
    return NULL, pos + 4
  end
  if byte == 45 or (byte >= 48 and byte <= 57) then
    return parse_number(text, pos, length)
  end
  parse_error(pos, "无法识别的 token")
end

--- `codec.decode(text)`：确定性 JSON → Lua 值。
---
--- 对偶规则：对象的 `null` 值 = 键不存在（丢弃）；数组里的 `null` 报错（丢不得，
--- 丢了就是下标平移）；顶层 `null` 返回 `nil`（与「不存在」同义）。
function M.decode(text)
  if type(text) ~= "string" then
    fail(E_TYPE, "decode 只收字符串，得到 " .. type(text))
  end
  local length = #text
  local value, pos = parse_value(text, 1, length)
  pos = skip_space(text, pos, length)
  if pos <= length then
    parse_error(pos, "顶层值之后还有残留字节")
  end
  if value == NULL then
    return nil
  end
  return value
end

-- ── checksum：规范化字节的 sha256 ─────────────────────────────────────────────

--- `codec.checksum(bundle)`：把 `checksum` 字段置空后再编码，取 sha256。
---
--- 为何置空而不是删掉：置空是**确定的**（键仍在，值固定为空串），于是「同一份
--- 内容 + 任意 checksum 字段」总是算出同一个值；而「删掉」会让编码后的键集合随
--- 调用方是否先填过 checksum 而变，出现两个不同的规范形。
---
--- 不修改入参：`checksum(bundle)` 是纯读面（契约 §4），换个顶层表就算隔离
--- （值全是标量/表引用，浅拷贝已足够——深层内容不会被本函数改写）。
function M.checksum(bundle)
  if type(bundle) ~= "table" then
    fail(E_TYPE, "checksum 只收 bundle 表，得到 " .. type(bundle))
  end
  local canonical = {}
  for key, value in pairs(bundle) do
    canonical[key] = value
  end
  canonical.checksum = ""
  return M.sha256(M.encode(canonical))
end

-- ── export / write / read / roundtrip ────────────────────────────────────────

--- 顶层浅拷（`checksum` / `write` 都不改入参：契约 §4 都是纯读/写盘面）。
local function copy_top(map)
  local out = {}
  for key, value in pairs(map) do
    out[key] = value
  end
  return out
end

--- 数值数组升序插入排序（记录 id 必须严格升序：导入侧的「保 id」判定依赖它）。
local function sort_numbers(items)
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

--- `codec.export(namespace)`：bundle = 实体记录 + 点名 KV 键。
---
--- `namespace`：`{ entities = true|false, ids = "index"|{…}, keys = {…}|nil, mod = "<名>" }`。
---
--- 三条形状纪律：
---   * 记录从 **entity 的 API** 取（`ids` / `schema_of` / `attrs` / `tags`），**不**读
---     `pkg.entity.*` 任何键——布局单源在 entity，泄漏一个键名就把两个包焊死了；
---   * `ids = "index"` 走 `entity.ids()`（升序）；给数组则只导那些 id（仍排序）；
---   * `keys` 是**显式列表**，绝不遍历 KV（宿主没有遍历面，D9）；没点名的键不进包。
function M.export(ns)
  if type(ns) ~= "table" then
    fail(E_TYPE, "export 需要 namespace 表，得到 " .. type(ns))
  end
  local current = host()

  local bundle = {
    codec_version = CODEC_VERSION,
    exported_from = type(ns.mod) == "string" and ns.mod or "",
    entities = {},
    kv = {},
    checksum = "",
  }

  if ns.entities ~= false then
    local ids
    if ns.ids == nil or ns.ids == "index" then
      ids = entity.ids()
    elseif type(ns.ids) == "table" then
      ids = {}
      for i = 1, #ns.ids do
        ids[i] = ns.ids[i]
      end
      sort_numbers(ids)
    else
      fail(E_TYPE, 'ns.ids 只能是 "index" 或 id 数组，得到 ' .. type(ns.ids))
    end
    for i = 1, #ids do
      local id = ids[i]
      bundle.entities[i] = {
        id = id,
        schema = entity.schema_of(id),
        attrs = entity.attrs(id),
        tags = entity.tags(id),
      }
    end
  end

  if ns.keys ~= nil then
    if type(ns.keys) ~= "table" then
      fail(E_TYPE, "ns.keys 必须是键数组（不支持前缀/遍历）")
    end
    for i = 1, #ns.keys do
      local key = ns.keys[i]
      if type(key) ~= "string" then
        fail(E_TYPE, "ns.keys 的元素必须是字符串键，得到 " .. type(key))
      end
      local value = current.kv.get(key)
      if value ~= nil then
        bundle.kv[key] = value
      end
    end
  end

  return bundle
end

--- `codec.write(path, bundle)`：算 checksum → 编码 → 落盘 → 与宿主回值**互校**。
---
--- 返回 `{ bytes, sha256 }`，sha256 **是我方算法算的**（它是「写进 bundle 的那个
--- 值」）。宿主的 sha256 从 P0 的「来源」变成「互校」：一致则无事，不一致报
--- `YGG-PKG-CODEC-CHECKSUM`——两套实现算出不同字节就说明有一边错了，该报警而不是
--- 选一个信。不修改入参（写进文件的是副本）。
function M.write(path, bundle)
  if type(path) ~= "string" or #path == 0 then
    fail(E_TYPE, "write 需要一个非空路径字符串")
  end
  if type(bundle) ~= "table" then
    fail(E_TYPE, "write 只收 bundle 表，得到 " .. type(bundle))
  end
  local current = host()
  local digest = M.checksum(bundle)
  local payload = copy_top(bundle)
  payload.checksum = digest
  local text = M.encode(payload)

  local written = current.fs.write(path, text)
  if type(written) ~= "table" then
    fail(E_TYPE, "ygg.fs.write 未返回结果表")
  end
  local host_digest = written.sha256
  if type(host_digest) == "string" and #host_digest > 0 and host_digest:lower() ~= digest then
    fail(
      E_CHECKSUM,
      "宿主 sha256 与包内自算不一致（宿主 " .. host_digest .. " / 包内 " .. digest .. "）"
    )
  end
  return { bytes = #text, sha256 = digest }
end

--- `codec.read(path)`：读盘 → 解码 → **先校 checksum** 再交出去。
---
--- 缺失文件与坏档是两个不同的码：`MISSING`（路径/挂载问题）与 `CHECKSUM`
--- （内容被改过）指向不同的处置，混成一个码就把排查方向抹掉了。
function M.read(path)
  local current = host()
  local text = current.fs.read(path)
  if text == nil then
    fail(E_MISSING, "读不到存档：" .. tostring(path))
  end
  if type(text) ~= "string" then
    fail(E_TYPE, "ygg.fs.read 未返回字符串")
  end
  local bundle = M.decode(text)
  if type(bundle) ~= "table" then
    fail(E_TYPE, "存档顶层必须是 JSON 对象")
  end
  if type(bundle.checksum) ~= "string" then
    fail(E_CHECKSUM, "存档缺 checksum 字段（不能证明未被改过）")
  end
  local actual = M.checksum(bundle)
  if actual ~= bundle.checksum then
    fail(
      E_CHECKSUM,
      "checksum 不符（声明 " .. bundle.checksum .. " / 实算 " .. actual .. "）"
    )
  end
  return bundle
end

--- `codec.roundtrip(path)`：导 → 写 → 读 → 再编，**比字节**。
---
--- `equal` 比的是两份**编码后字节**而非长度：长度相等而字节不同正是「键序漂了」
--- 或「数字形式漂了」的典型症状，只比长度会漏掉它。
function M.roundtrip(path)
  local bundle = M.export({ entities = true, ids = "index", mod = "roundtrip" })
  local written = M.write(path, bundle)
  local loaded = M.read(path)
  -- `write` 不改入参，所以这里重建「写进文件的那份」：同一个 checksum + 同一个编码。
  local mirror = copy_top(bundle)
  mirror.checksum = written.sha256
  local text_in = M.encode(mirror)
  local text_out = M.encode(loaded)
  return { bytes_in = #text_in, bytes_out = #text_out, equal = text_in == text_out }
end

-- ── import：两阶段（validate → apply）三种冲突策略 ─────────────────────────

local CONFLICT_POLICIES = { skip = true, overwrite = true, rename = true }

--- 属性值域判定（4 行）：与 entity 各自实现，**不**抽第三个包——抽出去就多
--- 一条依赖边，收益小于一条边（契约 §1.4）。
local function value_ok(kind, value)
  if kind == "number" then
    return type(value) == "number" and value == value and value ~= math.huge and value ~= -math.huge
  elseif kind == "string" then
    return type(value) == "string"
  elseif kind == "boolean" then
    return type(value) == "boolean"
  elseif kind == "ref" then
    return type(value) == "number" and math.type(value) == "integer" and value >= 1
  end
  return false
end

--- 校验一条记录（**纯读**，零写）并返回规范形。
---
--- 这一层是「全或无」的兑现处：所有能检查的都在这里检查完，于是 apply 阶段
--- 的失败（如果真的发生）只剩宿主层异常，而那是 driver 事务回滚的事。
local function compile_record(record, index)
  local at = "第 " .. index .. " 条记录"
  if type(record) ~= "table" then
    fail(E_TYPE, at .. " 不是表")
  end
  local id = record.id
  if type(id) ~= "number" or math.type(id) ~= "integer" or id < 1 then
    fail(E_TYPE, at .. " 的 id 必须是正整数")
  end
  if type(record.schema) ~= "string" then
    fail(E_TYPE, at .. " 缺 schema 名")
  end
  local spec = entity.schema(record.schema)
  if spec == nil then
    fail(E_TYPE, at .. " 的 schema `" .. record.schema .. "` 未在本局注册（先 define 再 import）")
  end
  if record.attrs ~= nil and type(record.attrs) ~= "table" then
    fail(E_TYPE, at .. " 的 attrs 必须是表")
  end
  if record.tags ~= nil and type(record.tags) ~= "table" then
    fail(E_TYPE, at .. " 的 tags 必须是数组")
  end

  local attrs = {}
  if record.attrs ~= nil then
    for name, value in pairs(record.attrs) do
      local slot = spec.attr_index[name]
      if slot == nil then
        fail(E_TYPE, at .. " 的属性 `" .. tostring(name) .. "` 未在 schema 声明")
      end
      if not value_ok(spec.attrs[slot].type, value) then
        fail(E_TYPE, at .. " 的属性 `" .. tostring(name) .. "` 不在值域内")
      end
      attrs[name] = value
    end
  end
  for i = 1, #spec.attrs do
    local decl = spec.attrs[i]
    if decl.required and not decl.has_default and attrs[decl.name] == nil then
      fail(E_TYPE, at .. " 缺必填属性 `" .. decl.name .. "`（且无缺省值）")
    end
  end

  local tags = {}
  if record.tags ~= nil then
    local seen = {}
    for i = 1, #record.tags do
      local tag = record.tags[i]
      if type(tag) ~= "string" or spec.tag_index[tag] == nil then
        fail(E_TYPE, at .. " 的 tag `" .. tostring(tag) .. "` 未在 schema 声明")
      end
      if not seen[tag] then
        seen[tag] = true
        tags[#tags + 1] = tag
      end
    end
    sort_names(tags)
  end

  return { id = id, schema = record.schema, attrs = attrs, tags = tags }
end

--- `overwrite` 的落点：属性逐键 `set`（同值自然零写）+ tag 按差分补齐/清除。
local function apply_overwrite(record)
  for name, value in pairs(record.attrs) do
    entity.set(record.id, name, value)
  end
  local spec = entity.schema(record.schema)
  local wanted = {}
  for i = 1, #record.tags do
    wanted[record.tags[i]] = true
  end
  for i = 1, #spec.tags do
    local tag = spec.tags[i].name
    local present = entity.has(record.id, tag)
    if wanted[tag] and not present then
      entity.tag(record.id, tag)
    elseif not wanted[tag] and present then
      entity.untag(record.id, tag)
    end
  end
end

--- 算「指向被重命名 id 的 ref 属性数」（契约 I6：只报数，**不**重写引用）。
local function count_dangling_refs(renamed, records)
  local count = 0
  if next(renamed) == nil then
    return 0
  end
  for i = 1, #records do
    local record = records[i]
    local spec = entity.schema(record.schema)
    if spec ~= nil then
      for j = 1, #spec.attrs do
        local decl = spec.attrs[j]
        if decl.type == "ref" then
          local value = record.attrs[decl.name]
          if type(value) == "number" and renamed[value] ~= nil then
            count = count + 1
          end
        end
      end
    end
  end
  return count
end

--- `codec.import(bundle, opts)`：两阶段两回。
---
--- **阶段 ① validate（纯读、零写）**：版本 → checksum → 策略 → 逐记录（schema 已注册、
--- 属性键在声明集内、值域、必填、tag 声明、id 严格升序）。任何一条不过就一条不写。
--- **阶段 ② apply**：按策略落地。宿主层异常由 driver 的事务回滚兼底（本包不自己
--- 实现事务——那是第二套存储）。
---
--- 「保 id」不需要特技：内核自 1 起致密单调分配，所以空局按升序导入全量时
--- `new_id == old_id` **自然**成立；不成立的那些 id（子集导入、已在库的 id）
--- 记进 `renamed`。引用**不重写**（I6）：ref 属性原值原样写入，只在返回体报个数。
---
--- `kv` 段**不写回**（D9）：单写者纪律归 mod，所以包把 `bundle.kv` 原样带回。
function M.import(bundle, opts)
  -- ① validate
  if type(bundle) ~= "table" then
    fail(E_TYPE, "import 需要 bundle 表，得到 " .. type(bundle))
  end
  if bundle.codec_version ~= CODEC_VERSION then
    fail(
      E_VERSION,
      "未知的 codec_version `"
        .. tostring(bundle.codec_version)
        .. "`（本实现只认 "
        .. CODEC_VERSION
        .. "，且不做迁移）"
    )
  end
  if type(bundle.checksum) ~= "string" then
    fail(E_CHECKSUM, "bundle 缺 checksum 字段（不能证明未被改过）")
  end
  -- 空串 = **未封签**（`export` 刚产出的内存态），不是「已经校验通过」。
  -- 为何不在 export 里封签：封签要把整份 bundle 再哈希一遍（纯 Lua sha256 是
  -- 导出侧最贵的一步），而导出常常只为了看一眼或转存；而越过信任边界的档
  -- 一律经 `read`（从盘上读，必封且必校）。于是这里是**显式**两态而不是隐式：
  -- 有声明就必校，没声明就标 `verified = false` 交回调用方自己决定。
  local verified = false
  if #bundle.checksum > 0 then
    local actual = M.checksum(bundle)
    if actual ~= bundle.checksum then
      fail(E_CHECKSUM, "checksum 不符（声明 " .. bundle.checksum .. " / 实算 " .. actual .. "）")
    end
    verified = true
  end
  if type(opts) ~= "table" then
    fail(E_CONFLICT, 'import 需要 opts.on_conflict（"skip" / "overwrite" / "rename"）')
  end
  local policy = opts.on_conflict
  if policy == nil or not CONFLICT_POLICIES[policy] then
    fail(
      E_CONFLICT,
      'on_conflict 必选且只能取 "skip" / "overwrite" / "rename"，得到 '
        .. tostring(policy)
    )
  end
  if type(bundle.entities) ~= "table" then
    fail(E_TYPE, "bundle.entities 必须是数组")
  end
  local records = {}
  local previous = 0
  for i = 1, #bundle.entities do
    local record = compile_record(bundle.entities[i], i)
    if record.id <= previous then
      fail(E_TYPE, "记录 id 必须严格升序（第 " .. i .. " 条：" .. record.id .. "）")
    end
    previous = record.id
    records[i] = record
  end

  -- ② apply
  local report = {
    created = 0,
    skipped = 0,
    overwritten = 0,
    renamed = {},
    refs = 0,
    verified = verified,
    kv = {},
  }
  for i = 1, #records do
    local record = records[i]
    if entity.exists(record.id) then
      if policy == "skip" then
        report.skipped = report.skipped + 1
      elseif policy == "overwrite" then
        apply_overwrite(record)
        report.overwritten = report.overwritten + 1
      else
        local fresh_id = entity.spawn(record.schema, record.attrs, record.tags)
        report.renamed[record.id] = fresh_id
        report.created = report.created + 1
      end
    else
      local new_id = entity.spawn(record.schema, record.attrs, record.tags)
      if new_id ~= record.id then
        report.renamed[record.id] = new_id
      end
      report.created = report.created + 1
    end
  end
  report.refs = count_dangling_refs(report.renamed, records)
  if type(bundle.kv) == "table" then
    report.kv = copy_top(bundle.kv)
  end
  return report
end

return M
