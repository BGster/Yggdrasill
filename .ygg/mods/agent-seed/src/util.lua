-- src/util.lua —— 纯函数工具（沙箱无 table 库）：fill / split / cut / trim / join / contains / to_bool / spaces。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 资源由入口 install(deps) 注入。本模块唯一写的 KV 前缀：无（纯函数）。

local M = {}


local function fill(template, vars)
  if type(template) ~= "string" then
    return ""
  end
  return (template:gsub("{{([%w_]+)}}", function(key)
    local value = vars[key]
    if value == nil then
      return "{{" .. key .. "}}"
    end
    return tostring(value)
  end))
end

local function split(text, sep)
  local out = {}
  if type(text) ~= "string" or text == "" then
    return out
  end
  local start = 1
  while true do
    local index, last = text:find(sep, start, true)
    if index == nil then
      out[#out + 1] = text:sub(start)
      return out
    end
    out[#out + 1] = text:sub(start, index - 1)
    start = last + 1
  end
end

local function cut(text, sep)
  local index, last = text:find(sep, 1, true)
  if index == nil then
    return text, nil
  end
  return text:sub(1, index - 1), text:sub(last + 1)
end

local function trim(text)
  if type(text) ~= "string" then
    return ""
  end
  return (text:gsub("^%s+", ""):gsub("%s+$", ""))
end

local function join(items, sep)
  local out = ""
  for i = 1, #items do
    if i > 1 then
      out = out .. sep
    end
    out = out .. tostring(items[i])
  end
  return out
end

local function contains(list, value)
  for i = 1, #list do
    if list[i] == value then
      return true
    end
  end
  return false
end

local function to_bool(text)
  if text == "true" then
    return true
  end
  if text == "false" then
    return false
  end
  return nil
end

local function spaces(count)
  local out = ""
  for _ = 1, count do
    out = out .. " "
  end
  return out
end

M.contains = contains
M.cut = cut
M.fill = fill
M.join = join
M.spaces = spaces
M.split = split
M.to_bool = to_bool
M.trim = trim

return M
