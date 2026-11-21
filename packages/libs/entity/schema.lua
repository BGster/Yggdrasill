-- `pkg.entity.schema` —— spec 的编译、比较、落盘与 memo（`define` 的语义核心）。
--
-- 契约：`docs/design/packages/entity.md` §2 / I3。批十一拆分（纯搬移）：内部规范形
-- 只由本模块读写，对外形每次重建（`M.schema` 返回深副本）；`memo` 与 `memo_sync`
-- 住这里（单源），record / query 经 require 取用。`value_ok` 是写路径值域判据的
-- 唯一出处（spawn / set / query 的类型门都从这里取）。

local base = require("pkg.entity.base")

local fail = base.fail
local E_SCHEMA = base.E_SCHEMA
local TYPES = base.TYPES
local K = base.K
local name_ok = base.name_ok
local finite = base.finite
local is_integer = base.is_integer
local split_lines = base.split_lines
local join_lines = base.join_lines
local sort_names = base.sort_names
local copy_array = base.copy_array
local host = base.host

local S = {}

-- ── schema spec：编译 / 比较 / 落盘 / memo ────────────────────────────────────
--
-- spec 的内部规范形（编译产物）与对外形（`M.schema` 的返回）是同一份数据的两份
-- 拷贝：内部形只由本文件读写，对外形每次重建，调用方改坏自己的副本不影响注册表。
--
-- 规范形的两个要点：
--   * `attrs` / `tags` 是**名字升序的数组**（不是 map）——顺序即字节序，落盘时
--     `join_lines` 直接得到确定的名表，读回时不必再排一次。
--   * `attr_index` / `tag_index` 是名字 → 下标的反查表：`spawn` / `set` 的键校验
--     是 O(1) 表取，不扫数组（否则每次写属性都是 O(属性数) 的线性查找）。

local function spec_error(detail)
  fail(E_SCHEMA, detail)
end

--- 值域判定（按声明的类型）：`ref` 只收正整数 id，`number` 只收有限数。
local function value_ok(kind, v)
  if kind == "number" then
    return type(v) == "number" and finite(v)
  elseif kind == "string" then
    return type(v) == "string"
  elseif kind == "boolean" then
    return type(v) == "boolean"
  elseif kind == "ref" then
    return type(v) == "number" and is_integer(v) and v >= 1
  end
  return false
end

--- 编译一条属性声明（`{ type, required?, default? }`）。
local function compile_attr(schema_name, attr_name, def)
  if not name_ok(attr_name) then
    spec_error("schema `" .. schema_name .. "` 的属性名非法: " .. tostring(attr_name))
  end
  if type(def) ~= "table" then
    spec_error("schema `" .. schema_name .. "` 的属性 `" .. attr_name .. "` 声明必须是表")
  end
  if not TYPES[def.type] then
    spec_error(
      "schema `" .. schema_name .. "` 的属性 `" .. attr_name .. "` 类型非法: " .. tostring(def.type)
    )
  end
  local required = def.required
  if required == nil then
    required = false
  end
  if type(required) ~= "boolean" then
    spec_error("schema `" .. schema_name .. "` 的属性 `" .. attr_name .. "` 的 required 必须是布尔")
  end
  -- `has_default` 是**存在性位**：`false` / `0` / `""` 都是合法缺省值，不能用
  -- 「键缺失」表达（nil 才是「没有缺省」）。
  local has_default = def.default ~= nil
  if has_default and not value_ok(def.type, def.default) then
    spec_error(
      "schema `" .. schema_name .. "` 的属性 `" .. attr_name .. "` 的缺省值不在值域内"
    )
  end
  return {
    name = attr_name,
    type = def.type,
    required = required,
    has_default = has_default,
    default = def.default,
  }
end

--- 编译整个 spec（名字校验 + 排序 + 反查表）。所有校验在**任何写之前**完成。
local function compile_spec(schema_name, raw)
  if type(raw) ~= "table" then
    spec_error("schema `" .. schema_name .. "` 的 spec 必须是表")
  end
  if raw.attrs ~= nil and type(raw.attrs) ~= "table" then
    spec_error("schema `" .. schema_name .. "` 的 attrs 必须是表")
  end
  if raw.tags ~= nil and type(raw.tags) ~= "table" then
    spec_error("schema `" .. schema_name .. "` 的 tags 必须是表")
  end

  local attr_raw_names = {}
  if raw.attrs ~= nil then
    for key in pairs(raw.attrs) do
      attr_raw_names[#attr_raw_names + 1] = key
    end
  end
  sort_names(attr_raw_names)

  local attrs = {}
  local attr_index = {}
  for i = 1, #attr_raw_names do
    local attr = compile_attr(schema_name, attr_raw_names[i], raw.attrs[attr_raw_names[i]])
    attrs[i] = attr
    attr_index[attr.name] = i
  end

  local tag_raw_names = {}
  if raw.tags ~= nil then
    for key in pairs(raw.tags) do
      tag_raw_names[#tag_raw_names + 1] = key
    end
  end
  sort_names(tag_raw_names)

  local tags = {}
  local tag_index = {}
  for i = 1, #tag_raw_names do
    local tag_name = tag_raw_names[i]
    if not name_ok(tag_name) then
      spec_error("schema `" .. schema_name .. "` 的 tag 名非法: " .. tostring(tag_name))
    end
    local def = raw.tags[tag_name]
    if type(def) ~= "table" then
      spec_error("schema `" .. schema_name .. "` 的 tag `" .. tag_name .. "` 声明必须是表")
    end
    local required = def.required
    if required == nil then
      required = false
    end
    if type(required) ~= "boolean" then
      spec_error("schema `" .. schema_name .. "` 的 tag `" .. tag_name .. "` 的 required 必须是布尔")
    end
    tags[i] = { name = tag_name, required = required }
    tag_index[tag_name] = i
  end

  return {
    name = schema_name,
    attrs = attrs,
    tags = tags,
    attr_index = attr_index,
    tag_index = tag_index,
  }
end

--- 两份 spec 是否逐字段相同（缺省值的整/浮身份也比：`1` 与 `1.0` 落盘即两样）。
local function spec_equal(a, b)
  if #a.attrs ~= #b.attrs or #a.tags ~= #b.tags then
    return false
  end
  for i = 1, #a.attrs do
    local x, y = a.attrs[i], b.attrs[i]
    if
      x.name ~= y.name
      or x.type ~= y.type
      or x.required ~= y.required
      or x.has_default ~= y.has_default
    then
      return false
    end
    if x.has_default then
      if x.default ~= y.default or is_integer(x.default) ~= is_integer(y.default) then
        return false
      end
    end
  end
  for i = 1, #a.tags do
    if a.tags[i].name ~= b.tags[i].name or a.tags[i].required ~= b.tags[i].required then
      return false
    end
  end
  return true
end

--- spec 的深拷贝（对外返回用；`attr_index` / `tag_index` 一并重建）。
local function copy_spec(spec)
  local attrs = {}
  local attr_index = {}
  for i = 1, #spec.attrs do
    local a = spec.attrs[i]
    attrs[i] = {
      name = a.name,
      type = a.type,
      required = a.required,
      has_default = a.has_default,
      default = a.default,
    }
    attr_index[a.name] = i
  end
  local tags = {}
  local tag_index = {}
  for i = 1, #spec.tags do
    tags[i] = { name = spec.tags[i].name, required = spec.tags[i].required }
    tag_index[tags[i].name] = i
  end
  return { name = spec.name, attrs = attrs, tags = tags, attr_index = attr_index, tag_index = tag_index }
end

--- 从 KV 读一份 spec（`<n>.attrs` 缺失 = 该 schema 不存在）。
local function read_spec(h, schema_name)
  local base = K.schema .. schema_name
  local attrs_text = h.kv.get(base .. ".attrs")
  if type(attrs_text) ~= "string" then
    return nil
  end
  local attr_names = split_lines(attrs_text)
  local tag_names = split_lines(h.kv.get(base .. ".tags"))
  local attrs = {}
  local attr_index = {}
  for i = 1, #attr_names do
    local attr_name = attr_names[i]
    local prefix = base .. ".a." .. attr_name
    local has_default = h.kv.get(prefix .. ".has_default") == true
    attrs[i] = {
      name = attr_name,
      type = h.kv.get(prefix .. ".type"),
      required = h.kv.get(prefix .. ".required") == true,
      has_default = has_default,
      default = has_default and h.kv.get(prefix .. ".default") or nil,
    }
    attr_index[attr_name] = i
  end
  local tags = {}
  local tag_index = {}
  for i = 1, #tag_names do
    tags[i] = {
      name = tag_names[i],
      required = h.kv.get(base .. ".t." .. tag_names[i] .. ".required") == true,
    }
    tag_index[tags[i].name] = i
  end
  return {
    name = schema_name,
    attrs = attrs,
    tags = tags,
    attr_index = attr_index,
    tag_index = tag_index,
  }
end

-- 进程内注册表镜像：`schemas_rev` 是失效锚（define 每次 +1）。没有第二个真源——
-- 命中缓存的唯一条件是 KV 里的 rev 与本镜像记的一致，所以「另一个写者改了注册表」
-- 也会被下一次调用发现（只要它同时推进了 rev，而单写者纪律保证只有本包写）。
local memo = { rev = nil, names = nil, specs = nil }

local function memo_sync(h)
  local rev = h.kv.get(K.schemas_rev)
  if type(rev) ~= "number" then
    rev = 0
  end
  if memo.rev == rev and memo.specs ~= nil then
    return
  end
  local names = split_lines(h.kv.get(K.schemas))
  local specs = {}
  for i = 1, #names do
    specs[names[i]] = read_spec(h, names[i])
  end
  memo.rev, memo.names, memo.specs = rev, names, specs
end

--- 把 spec 落到扁平键上（§2 的物理落点表：一族标量键 + 两张名表）。
local function write_spec(h, spec)
  local names = copy_array(memo.names or {})
  local seen = false
  for i = 1, #names do
    if names[i] == spec.name then
      seen = true
    end
  end
  if not seen then
    names[#names + 1] = spec.name
    sort_names(names)
  end

  local base = K.schema .. spec.name
  local attr_names = {}
  for i = 1, #spec.attrs do
    attr_names[i] = spec.attrs[i].name
  end
  local tag_names = {}
  for i = 1, #spec.tags do
    tag_names[i] = spec.tags[i].name
  end

  h.kv.set(base .. ".attrs", join_lines(attr_names))
  h.kv.set(base .. ".tags", join_lines(tag_names))
  for i = 1, #spec.attrs do
    local attr = spec.attrs[i]
    local prefix = base .. ".a." .. attr.name
    h.kv.set(prefix .. ".type", attr.type)
    h.kv.set(prefix .. ".required", attr.required)
    h.kv.set(prefix .. ".has_default", attr.has_default)
    if attr.has_default then
      h.kv.set(prefix .. ".default", attr.default)
    end
  end
  for i = 1, #spec.tags do
    h.kv.set(base .. ".t." .. spec.tags[i].name .. ".required", spec.tags[i].required)
  end
  h.kv.set(K.schemas, join_lines(names))

  local rev = h.kv.get(K.schemas_rev)
  if type(rev) ~= "number" then
    rev = 0
  end
  h.kv.set(K.schemas_rev, rev + 1)
  memo.rev = nil -- 失效：下一次调用重建镜像（写后立即读也走同一条路）
  return true
end

--- `entity.define(name, spec)`：注册（或幂等确认）一个 schema。
---
--- 同 spec 重复定义 = `true` 且**零写**（KV 的 `schemas_rev` 只 +1 一次）；异 spec
--- 重复定义 = `YGG-PKG-ENTITY-SCHEMA`。不覆盖是刻意的：覆盖会让已存在的实体的属性
--- 集合在无人察觉时改变，而「新增属性 = 改数据里的 spec 并换个名字」是契约 I3 的出口。
function S.define(name, raw_spec)
  if not name_ok(name) then
    spec_error("schema 名非法: " .. tostring(name))
  end
  local spec = compile_spec(name, raw_spec)
  local h = host()
  memo_sync(h)
  local existing = memo.specs[name]
  if existing ~= nil then
    if spec_equal(existing, spec) then
      return true
    end
    spec_error("schema `" .. name .. "` 已定义，且与本次声明的形状不同（不覆盖）")
  end
  return write_spec(h, spec)
end

--- `entity.schema(name)`：读一份 spec 的深副本；未注册返回 `nil`。
function S.schema(name)
  local h = host()
  memo_sync(h)
  local spec = memo.specs[name]
  if spec == nil then
    return nil
  end
  return copy_spec(spec)
end

-- ── 导出面（record / query 取 value_ok / spec_error / memo / memo_sync）──────

S.value_ok = value_ok
S.spec_error = spec_error
S.memo = memo
S.memo_sync = memo_sync

return S

