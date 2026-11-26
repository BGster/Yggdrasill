-- `pkg.codec.jsondec` —— decode：递归下降 + 位置化报错（`at byte N`）。
--
-- 契约：`openwiki/contracts/packages/codec.md` §4.1。批十一拆分（纯搬移）：原生面委托与
-- 回退随本文件走，对偶规则（对象 null 丢弃 / 数组 null 报错 / 顶层 null = nil）
-- 见 `M.decode` 头注。解析器骨架（NULL / parse_error / skip_space / utf8_char /
-- parse_string / parse_number / parse_value）全部住本文件。

local runtime = require("pkg.codec.runtime")
local jsonenc = require("pkg.codec.jsonenc")

local fail = runtime.fail
local E_TYPE = runtime.E_TYPE
local E_HOST = runtime.E_HOST
local native_face = runtime.native_face
local join = jsonenc.join

local M = {}

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
  local face = native_face()
  if face ~= nil then
    local ok, value = pcall(face.decode, text)
    if ok then
      return value
    end
    -- 同 `encode`：先 `tostring` 再找码（宿主错误值未必是字符串）。
    if not tostring(value):find("YGG-CODEC-UNSUPPORTED", 1, true) then
      fail(E_HOST, "原生面内部失败：" .. tostring(value))
    end
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

return M

