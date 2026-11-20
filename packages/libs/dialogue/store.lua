-- `pkg.dialogue.store` —— 对话包的 KV 管道（键表、标量纪律、名字表、模块局部配置）。
--
-- # 键的物理形状（契约 §2；回写 D10）
--
-- 宿主 `kv.set` 只收四个标量型（`crates/ygg-sandbox/src/host_api.rs:568`），所以
-- 契约里「表值」的 `handlers` / `threads` 全部落成扁平标量键 + 换行名表：
--
--   dlg.seq                          number  轮次单写口（对齐 state-surface §2.8.1 范例）
--   dlg.types                        string  有 handler 的 type 名表（升序）
--   dlg.h.<type>.names               string  handler 名表（**注册序**，不排序：I7）
--   dlg.h.<type>.n.<name>.kind       string  builtin|agent
--   dlg.h.<type>.n.<name>.opts       string  opts 键名表（升序）
--   dlg.h.<type>.n.<name>.opt.<key>  标量   opts 的值（memory/window/tool/credential_key…）
--   dlg.keys                         string  线程键名表（升序）——D2 的 `dlg.index`
--   dlg.thread.<key>.turns           number  已存轮数
--   dlg.thread.<key>.t.<i>.{who,text,seq}
--
-- # 为什么 handler 用「名表 + 按名寻址」而不是数字下标
--
-- `off` 之后若按下标重排，旧下标会变成**读不到但还在**的墓碑（KV 没有删除面，
-- `crates/ygg-core/src/kv.rs`）——「条目消失」就成了「条目看不见」。按名寻址时
-- `off` 只改名表一行，被删 handler 的键是**显式登记的墓碑**（同名重注册会覆盖它们，
-- 于是墓碑不会无限长）。注册序存在名表里（不排序），执行序 = 名表序 = I7。
--
-- 前缀单写者：本包只写 `dlg.` 前缀（`docs/design/mod/mod-package.md` §3.2.3）。

local S = {}

S.K = {
  seq = "dlg.seq",
  types = "dlg.types",
  handler_prefix = "dlg.h.",
  threads = "dlg.keys",
  thread_prefix = "dlg.thread.",
}

-- 错误码家族 `YGG-PKG-DIALOGUE-<COND>`（契约 §4；回写 D1' 逐条列全）。
S.E = {
  INSTALL = "YGG-PKG-DIALOGUE-INSTALL",
  TYPE = "YGG-PKG-DIALOGUE-TYPE",
  DUP = "YGG-PKG-DIALOGUE-DUP",
  UNBOUND = "YGG-PKG-DIALOGUE-UNBOUND",
  WINDOW = "YGG-PKG-DIALOGUE-WINDOW",
  DEPTHCFG = "YGG-PKG-DIALOGUE-DEPTHCFG",
  HOST = "YGG-PKG-DIALOGUE-HOST",
}

function S.fail(code, detail)
  error(code .. ": " .. detail, 0)
end

-- ── 宿主句柄与模块局部配置 ─────────────────────────────────────────────────

local installed = nil

--- 模块局部配置：`install(deps)` 可改，**不落 KV**（它是行为参数，不是存档状态）。
S.CONFIG = { window = 12, max_depth = 4 }

--- 装配：记 host 与配置。返回是否首次装配（桥只装一次，重复 install 不重复注册）。
function S.install(deps, config)
  if deps ~= nil and type(deps) ~= "table" then
    S.fail(S.E.INSTALL, "install 只收 table 或 nil，得到 " .. type(deps))
  end
  installed = { host = (type(deps) == "table" and deps.host) or nil }
  if config ~= nil then
    if config.window ~= nil then
      if type(config.window) ~= "number" or config.window % 1 ~= 0 or config.window < 1 then
        S.fail(S.E.WINDOW, "window 必须是正整数，得到 " .. tostring(config.window))
      end
      S.CONFIG.window = config.window
    end
    if config.max_depth ~= nil then
      if type(config.max_depth) ~= "number" or config.max_depth % 1 ~= 0 or config.max_depth < 1 then
        S.fail(S.E.DEPTHCFG, "max_depth 必须是正整数，得到 " .. tostring(config.max_depth))
      end
      S.CONFIG.max_depth = config.max_depth
    end
  end
  return S
end

function S.host()
  local h = installed and installed.host or nil
  if h == nil then
    h = ygg
  end
  if h == nil then
    S.fail(S.E.HOST, "ygg 不可达：沙箱外调用需 `install({ host = ... })`")
  end
  return h
end

-- ── 无标准库的骨架工具（同 quest/store.lua；两包各自几行，不抽第三个包）──────

local function concat(parts, sep)
  local n = #parts
  if n == 0 then
    return ""
  end
  local buf = {}
  for i = 1, n do
    buf[i] = parts[i]
  end
  local width = 1
  while width < n do
    local index = 1
    while index <= n do
      local right = buf[index + width]
      if right ~= nil then
        buf[index] = buf[index] .. sep .. right
      end
      index = index + width * 2
    end
    width = width * 2
  end
  return buf[1]
end

function S.join(lines)
  return concat(lines, "\n")
end

function S.split(text)
  local out = {}
  if type(text) ~= "string" or #text == 0 then
    return out
  end
  local start = 1
  while true do
    local pos = text:find("\n", start, true)
    if pos == nil then
      out[#out + 1] = text:sub(start)
      return out
    end
    out[#out + 1] = text:sub(start, pos - 1)
    start = pos + 1
  end
end

function S.sort(items)
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

local function is_alpha(b)
  return (b >= 65 and b <= 90) or (b >= 97 and b <= 122)
end

--- 名字/键名是否合法（首字符字母或下划线，其余 `[A-Za-z0-9_.:-]`）。
function S.key_name_ok(name)
  if type(name) ~= "string" or #name == 0 then
    return false
  end
  local first = name:byte(1)
  if not (is_alpha(first) or first == 95) then
    return false
  end
  for i = 2, #name do
    local b = name:byte(i)
    local ok = is_alpha(b) or (b >= 48 and b <= 57) or b == 95 or b == 46 or b == 58 or b == 45
    if not ok then
      return false
    end
  end
  return true
end

--- 线程键是否合法：非空、无换行。
---
--- 与 handler 名不同，线程键**以数字开头**（`"<source>:<target>"`），所以不能套
--- `key_name_ok` 的首字符规则；它进的是 KV 键中段与名表项，唯一能劈开名表的是换行。
function S.thread_key_ok(thread)
  if type(thread) ~= "string" or #thread == 0 then
    return false
  end
  return thread:find("\n", 1, true) == nil
end

function S.type_name_ok(name)
  if type(name) ~= "string" or #name == 0 then
    return false
  end
  local first = name:byte(1)
  if first < 97 or first > 122 then
    return false
  end
  for i = 2, #name do
    local b = name:byte(i)
    local ok = (b >= 97 and b <= 122) or (b >= 48 and b <= 57) or b == 95 or b == 45
    if not ok then
      return false
    end
  end
  return true
end

local function finite(v)
  if v ~= v then
    return false
  end
  return v <= 1.0 / 0.0 and v >= -1.0 / 0.0
end

--- 标量纪律：宿主 KV / 事件 / 工具实参都只收四个标量型。
function S.scalar(value, code, label)
  local kind = type(value)
  if kind == "string" or kind == "boolean" then
    return value
  end
  if kind == "number" then
    if not finite(value) then
      S.fail(code, label .. " 必须是有限数")
    end
    return value
  end
  S.fail(code, label .. " 只收标量（number/string/boolean），得到 " .. kind)
end

--- 实体 ref 校验：`0` 是合法的「系统」占位（线程键里不用 nil：表里 nil = 缺键）。
function S.ref_ok(value)
  if type(value) ~= "number" or not finite(value) then
    return false
  end
  if value < 0 or value % 1 ~= 0 then
    return false
  end
  return true
end

-- ── KV 读写与键构造 ────────────────────────────────────────────────────────

function S.get(h, key)
  return h.kv.get(key)
end

function S.set(h, key, value)
  S.scalar(value, S.E.TYPE, "KV 值 " .. key)
  h.kv.set(key, value)
end

function S.handler_key(type_name, suffix)
  return S.K.handler_prefix .. type_name .. "." .. suffix
end

function S.thread_key(thread, suffix)
  return S.K.thread_prefix .. thread .. "." .. suffix
end

--- 已分配的最大轮次号（缺键 = 0）。
function S.seq(h)
  local value = S.get(h, S.K.seq)
  if value == nil then
    return 0
  end
  return value
end

return S
