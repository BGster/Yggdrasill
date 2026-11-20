-- `pkg.dialogue.handlers` —— handler 注册面（`on` / `off` / 读回）与函数侧注册表。
--
-- # 函数不进 KV（D3）
--
-- `dlg.h.*` 是**持久面**，而 Lua 函数不可序列化（`codec.md` I7 亦拒 `function`）。
-- 于是 `on(type, spec)` 拆两处：
--
--   * KV 面：`{ kind, name, opts }`（`opts` 只放数据：memory / window / tool /
--     credential_key …）；
--   * 模块局部面：`REGISTRY[type .. ":" .. name] = spec.fn`（不落盘）。
--
-- 由此得到一个可断言的重载语义：重载后 KV 有 spec 而 `REGISTRY` 无函数 = **未绑定**
-- ——`emit` 走 I2 的回退链，未处理时发 `dialogue.failed`（`reason = "unbound"`）。
-- 「重启后忘了注册」于是变成一条会红的用例（d20），而不是静默不响应。
--
-- # 注册序 = 执行序（I7）
--
-- 名表 `dlg.h.<type>.names` **不排序**：它的字节序就是注册序，也就是执行序。
-- 名表里没有的 handler 不执行（`off` 之后即使墓碑键还在，也读不到）。

local S = require("pkg.dialogue.store")

local H = {}

--- 函数侧注册表（模块局部，不落盘）。键 = `type .. ":" .. name`。
--- `type` 的字符集不含 `:`（`^[a-z][a-z0-9_-]*$`），所以这个拼法无歧义。
local REGISTRY = {}

local KINDS = { builtin = true, agent = true }
local MEMORIES = { pair = true, entity = true, none = true }

--- handler 的复合键（REGISTRY 用）。
local function compound_key(type_name, name)
  return type_name .. ":" .. name
end

--- credential_key 的字符集：`^[A-Z][A-Z0-9_]*$`（.env 的键名形态，I6）。
local function credential_key_ok(value)
  if type(value) ~= "string" or #value == 0 then
    return false
  end
  local first = value:byte(1)
  if first < 65 or first > 90 then
    return false
  end
  for i = 2, #value do
    local b = value:byte(i)
    local ok = (b >= 65 and b <= 90) or (b >= 48 and b <= 57) or b == 95
    if not ok then
      return false
    end
  end
  return true
end

--- 正整数判定（window 用）。
local function positive_int(value)
  return type(value) == "number" and value % 1 == 0 and value >= 1
end

--- `opts` 的形状校验：只收标量值 + 四个已知键各自的词表。
local function check_opts(type_name, name, opts)
  if opts == nil then
    return {}
  end
  if type(opts) ~= "table" then
    S.fail(S.E.TYPE, type_name .. "/" .. name .. " 的 opts 必须是表或 nil")
  end
  local clean = {}
  for key, value in pairs(opts) do
    if not S.key_name_ok(key) then
      S.fail(S.E.TYPE, "opts 的键 " .. tostring(key) .. " 不在合法字符集内")
    end
    S.scalar(value, S.E.TYPE, "opts[" .. key .. "]")
    clean[key] = value
  end
  if clean.memory ~= nil then
    if type(clean.memory) ~= "string" or not MEMORIES[clean.memory] then
      S.fail(S.E.TYPE, "opts.memory 只能是 pair|entity|none，得到 " .. tostring(clean.memory))
    end
  end
  if clean.window ~= nil and not positive_int(clean.window) then
    S.fail(S.E.WINDOW, "opts.window 必须是正整数，得到 " .. tostring(clean.window))
  end
  if clean.credential_key ~= nil and not credential_key_ok(clean.credential_key) then
    S.fail(S.E.TYPE, "opts.credential_key 必须匹配 ^[A-Z][A-Z0-9_]*$，得到 " .. tostring(clean.credential_key))
  end
  if clean.tool ~= nil and (type(clean.tool) ~= "string" or #clean.tool == 0) then
    S.fail(S.E.TYPE, "opts.tool 必须是非空字符串（工具 id）")
  end
  return clean
end

-- ── 读面 ───────────────────────────────────────────────────────────────────

--- 一个 type 的 handler 名表（注册序）。
function H.names(h, type_name)
  return S.split(S.get(h, S.handler_key(type_name, "names")))
end

--- 一个 type 的 handler 列表（注册序）→ `{ {kind, name, opts}, … }`。
function H.list(h, type_name)
  local out = {}
  local names = H.names(h, type_name)
  for i = 1, #names do
    local name = names[i]
    local opts = {}
    local opt_names = S.split(S.get(h, S.handler_key(type_name, "n." .. name .. ".opts")))
    for j = 1, #opt_names do
      opts[opt_names[j]] = S.get(h, S.handler_key(type_name, "n." .. name .. ".opt." .. opt_names[j]))
    end
    out[i] = {
      kind = S.get(h, S.handler_key(type_name, "n." .. name .. ".kind")),
      name = name,
      opts = opts,
    }
  end
  return out
end

--- 有 handler 的 type 名表（升序）。
function H.types(h)
  return S.split(S.get(h, S.K.types))
end

--- 函数侧查表（未绑定 = `nil`）。
function H.fn_of(type_name, name)
  return REGISTRY[compound_key(type_name, name)]
end

-- ── 写面 ───────────────────────────────────────────────────────────────────

--- `on(type, spec)`：KV 写 spec，函数进 REGISTRY。同名重复注册 = `-DUP`。
function H.on(h, type_name, spec)
  if not S.type_name_ok(type_name) then
    S.fail(S.E.TYPE, "type 必须匹配 ^[a-z][a-z0-9_-]*$，得到 " .. tostring(type_name))
  end
  if type(spec) ~= "table" then
    S.fail(S.E.TYPE, "spec 必须是表，得到 " .. type(spec))
  end
  local kind = spec.kind
  if type(kind) ~= "string" or not KINDS[kind] then
    S.fail(S.E.TYPE, "spec.kind 只能是 builtin|agent，得到 " .. tostring(kind))
  end
  local name = spec.name
  if not S.key_name_ok(name) then
    S.fail(S.E.TYPE, "spec.name 不在合法字符集内：" .. tostring(name))
  end
  local opts = check_opts(type_name, name, spec.opts)

  local names = H.names(h, type_name)
  for i = 1, #names do
    if names[i] == name then
      S.fail(S.E.DUP, type_name .. " 下已有同名 handler：" .. name)
    end
  end
  names[#names + 1] = name
  S.set(h, S.handler_key(type_name, "names"), S.join(names))

  local prefix = "n." .. name .. "."
  S.set(h, S.handler_key(type_name, prefix .. "kind"), kind)
  local opt_names = {}
  for key, value in pairs(opts) do
    opt_names[#opt_names + 1] = key
    S.set(h, S.handler_key(type_name, prefix .. "opt." .. key), value)
  end
  S.sort(opt_names)
  S.set(h, S.handler_key(type_name, prefix .. "opts"), S.join(opt_names))

  local types = H.types(h)
  local known = false
  for i = 1, #types do
    if types[i] == type_name then
      known = true
    end
  end
  if not known then
    types[#types + 1] = type_name
    S.sort(types)
    S.set(h, S.K.types, S.join(types))
  end

  REGISTRY[compound_key(type_name, name)] = spec.fn
  return true
end

--- `off(type, name)`：从名表里摘掉（墓碑键留在原处，KV 没有删除面）。
function H.off(h, type_name, name)
  local names = H.names(h, type_name)
  local kept = {}
  local found = false
  for i = 1, #names do
    if names[i] == name then
      found = true
    else
      kept[#kept + 1] = names[i]
    end
  end
  if not found then
    return false
  end
  S.set(h, S.handler_key(type_name, "names"), S.join(kept))
  REGISTRY[compound_key(type_name, name)] = nil
  if #kept == 0 then
    local types = H.types(h)
    local remaining = {}
    for i = 1, #types do
      if types[i] ~= type_name then
        remaining[#remaining + 1] = types[i]
      end
    end
    S.set(h, S.K.types, S.join(remaining))
  end
  return true
end

--- 清空函数侧注册表（重载模拟用：KV 不动，函数全丢）。
function H.forget_all()
  REGISTRY = {}
end

return H
