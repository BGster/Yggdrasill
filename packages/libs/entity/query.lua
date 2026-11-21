-- `pkg.entity.query` —— 通用查询的包面与两道门（schema 面；S-24）。
--
-- 契约：`docs/design/packages/entity.md` §9.8。批十一拆分（纯搬移）：schema 门与
-- 未声明键门在**任何查询之前**完成；类型错在这里变成错误，内核面把类型错变成假。
-- spec / memo 经 `pkg.entity.schema` 取用（单源）。

local base = require("pkg.entity.base")
local schema = require("pkg.entity.schema")

local fail = base.fail
local E_ATTR = base.E_ATTR
local E_SCHEMA = base.E_SCHEMA
local K = base.K
local host = base.host
local value_ok = schema.value_ok
local memo = schema.memo
local memo_sync = schema.memo_sync

local M = {}

-- ── query 的两道门（schema 面；S-24）─────────────────────────────────────────

--- 属性名必须在 spec 里声明（`id` 是内核保留伪属性，不在 schema 里）。
local function check_declared(spec, schema_name, attr)
  if attr == "id" then
    return
  end
  if type(attr) ~= "string" or spec.attr_index[attr] == nil then
    fail(E_ATTR, "schema `" .. schema_name .. "` 未声明属性 `" .. tostring(attr) .. "`")
  end
end

--- 结构态操作数的类型门（复用 `value_ok`）；字符串态的类型由内核判——跨族即假，
--- 包不二次解析谓词文本（那会造出第二份文法真源）。
local function check_operand(spec, schema_name, attr, op_kind, value)
  if attr == "id" then
    return
  end
  local index = spec.attr_index[attr]
  if index == nil then
    return
  end
  local kind = spec.attrs[index].type
  local items = value
  if op_kind == "in" then
    if type(value) ~= "table" then
      return
    end
    items = value
  else
    items = { value }
  end
  for i = 1, #items do
    local item = items[i]
    if type(item) == "number" or type(item) == "string" or type(item) == "boolean" then
      if not value_ok(kind, item) then
        fail(
          E_ATTR,
          "schema `"
            .. schema_name
            .. "` 的属性 `"
            .. attr
            .. "` 声明为 "
            .. tostring(kind)
            .. "，操作数 "
            .. tostring(item)
            .. " 不相容"
        )
      end
    end
  end
end

--- `entity.query(selector)`：通用查询（S-24 的 `ygg.entity.query` 的包面）。
---
--- **只加内核加不上的两样**：`schema` 门与未声明键门。内核不认识 schema——它是本包
--- 在 KV 里的状态（`rec.<id>.schema`），放进内核面就是第二个 schema 注册表（违 I3）。
--- 所以 schema 过滤是**包面后过滤**，不是内核过滤。
---
--- 三档（契约 §9.8）：
---   * 无 `schema`：去掉 `schema` 键后**逐字透传**，1 次穿越，出表原样回；
---   * 有 `schema` + 无 `select`：内核回 id 形态 → 按 `rec.<id>.schema` 滤 → `{count, ids}`；
---   * 有 `schema` + 有 `select`：内核回行形态（行上已带 id）→ 原地滤行 → `{count, rows}`
---     （**不二次穿越**：再走一次 `attr_fetch` 只会多一次材质化，语义一样）。
---
--- 代价的诚实边界：schema-only 全量查询仍是 R 次 KV 读（与 `ids(nil, schema)` 今天同款，
--- 无回退也无进账）；根治要把 schema 镜像成保留 tag，那要动 I3。
---
--- 门与值的分工（一句话）：**包面把类型错变成错误，内核面把类型错变成假**。
function M.query(selector)
  local h = host()
  if type(selector) ~= "table" then
    -- 形状错交内核（FRAME）：包不复述文法，否则就是第二份真源
    return h.entity.query(selector)
  end
  local schema_filter = selector.schema
  local forwarded = {}
  for key, value in pairs(selector) do
    if key ~= "schema" then
      forwarded[key] = value
    end
  end
  if schema_filter == nil then
    return h.entity.query(forwarded)
  end
  if type(schema_filter) ~= "string" then
    fail(E_SCHEMA, "query 的 `schema` 必须是字符串，得到 " .. type(schema_filter))
  end
  memo_sync(h)
  local spec = memo.specs[schema_filter]
  if spec == nil then
    fail(E_SCHEMA, "schema `" .. schema_filter .. "` 未注册")
  end
  -- 未声明键门 + 结构态的类型门（都在**任何查询之前**完成）
  local where = selector.where
  if type(where) == "table" then
    for attr, predicate in pairs(where) do
      if type(attr) ~= "string" then
        fail(E_ATTR, "query 的 `where` 键必须是属性名")
      end
      check_declared(spec, schema_filter, attr)
      if type(predicate) == "table" and type(predicate.op) == "string" then
        check_operand(spec, schema_filter, attr, predicate.op, predicate.value)
      end
    end
  end
  local projection = selector.select
  if type(projection) == "table" then
    for i = 1, #projection do
      check_declared(spec, schema_filter, projection[i])
    end
  end

  local result = h.entity.query(forwarded)
  if type(projection) ~= "table" or #projection == 0 then
    local ids = {}
    local count = 0
    local source = result.ids or {}
    for i = 1, #source do
      local id = source[i]
      if h.kv.get(K.rec .. id .. ".schema") == schema_filter then
        count = count + 1
        ids[count] = id
      end
    end
    return { count = count, ids = ids }
  end
  local rows = {}
  local count = 0
  local source = result.rows or {}
  for i = 1, #source do
    local row = source[i]
    if h.kv.get(K.rec .. row.id .. ".schema") == schema_filter then
      count = count + 1
      rows[count] = row
    end
  end
  return { count = count, rows = rows }
end

return M

