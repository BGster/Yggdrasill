-- `pkg.entity.record` —— 存活判定、计数器与实体生命周期（spawn…destroy 17 API）。
--
-- 契约：`docs/design/packages/entity.md` §4。批十一拆分（纯搬移）：spec / memo 经
-- `pkg.entity.schema` 取用（value_ok / spec_error / memo / memo_sync 单源）；词典
-- util 与宿主句柄经 `pkg.entity.base` 取用。存活真源 = 内核存在性（契约 I5）。

local base = require("pkg.entity.base")
local schema = require("pkg.entity.schema")

local fail = base.fail
local E_ATTR = base.E_ATTR
local K = base.K
local host = base.host
local sort_names = base.sort_names
local is_integer = base.is_integer
local value_ok = schema.value_ok
local spec_error = schema.spec_error
local memo = schema.memo
local memo_sync = schema.memo_sync

local M = {}

-- ── 存活判定与计数器（§2 的 index 行：内核存在性 + 计数 + 水位）──────────────
--
-- 存活真源 = **内核存在性**（`ygg.entity.exists`，S9 起）：真删之后记录整条不在，
-- 「死」与「不存在」在内核里就是同一件事，包不再镜像任何位。所以不会出现
-- 「内核已删、包说活」的两套可见性（契约 I5）。`count` / `seq` 仍是派生标量：
-- 计数换 O(1) 的 `count()`，水位当 `ids` 的扫描上界。

--- 存活判定：一次内核存在性读（BTreeMap 查），零 KV 读、绝不扫键（契约 I5）。
---
--- 非数字 / 非整数 id 一律「不在世」（保持 S9 前的探询口径：`exists` 是 mod 的
--- 问路面，`nil` / 字符串不该让宿主面的 u64 参数转换抛错）。
local function alive(h, id)
  if type(id) ~= "number" or id % 1 ~= 0 then
    return false
  end
  return h.entity.exists(id)
end

--- 计数器增减（缺失 = 0；负数收敛到 0，不做断言——计数是派生值，不是真源）。
local function shift(h, key, delta)
  local current = h.kv.get(key)
  if type(current) ~= "number" then
    current = 0
  end
  local next_value = current + delta
  if next_value < 0 then
    next_value = 0
  end
  h.kv.set(key, next_value)
  return next_value
end

--- 写路径取 spec：未注册即 `SCHEMA`（读路径用 `M.schema` 拿 nil）。
local function require_spec(h, schema_name)
  memo_sync(h)
  local spec = memo.specs[schema_name]
  if spec == nil then
    spec_error("未注册的 schema `" .. tostring(schema_name) .. "`")
  end
  return spec
end

--- 把 `tags` 参数（序列形或集合形）规范化成**升序去重**的名字数组。
---
--- 两种形都收：序列形 `{ "enemy", "boss" }` 更好写，集合形 `{ enemy = true }` 是
--- 契约 §2 声明的形状。不存在歧义——名字字符集里没有纯数字，所以 `#tags > 0`
--- 只可能是序列形。
local function tag_list_of(spec, tags)
  local out = {}
  local seen = {}
  if tags == nil then
    return out
  end
  local function accept(tag_name)
    if type(tag_name) ~= "string" then
      spec_error("tag 必须是字符串，得到 " .. type(tag_name))
    end
    if spec.tag_index[tag_name] == nil then
      spec_error("schema `" .. spec.name .. "` 未声明 tag `" .. tag_name .. "`")
    end
    if not seen[tag_name] then
      seen[tag_name] = true
      out[#out + 1] = tag_name
    end
  end
  if #tags > 0 then
    for i = 1, #tags do
      accept(tags[i])
    end
  else
    for key, value in pairs(tags) do
      if value ~= true then
        spec_error("集合形 tags 的值必须是 true（得到 " .. tostring(value) .. "）")
      end
      accept(key)
    end
  end
  sort_names(out)
  return out
end

-- ── 构造 / 读 / 写 ──────────────────────────────────────────────────────────

--- `entity.spawn(name, attrs?, tags?)`：按 schema 分配 id 并写入属性袋。
---
--- **所有校验在任何写之前完成**：未知 schema / 未声明键 / 值域外 / 必填缺失都会
--- 在 `ygg.entity.spawn()` 之前报错，于是失败调用是**零写零事件**的（用例 7 用
--- mock 的写计数钉这一条）。
function M.spawn(schema_name, attrs, tags)
  local h = host()
  local spec = require_spec(h, schema_name)
  if attrs ~= nil and type(attrs) ~= "table" then
    fail(E_ATTR, "attrs 必须是表，得到 " .. type(attrs))
  end

  -- 最终属性集 = 缺省值 ← 调用方给的值（后者覆盖前者）。
  local final = {}
  for i = 1, #spec.attrs do
    local attr = spec.attrs[i]
    if attr.has_default then
      final[attr.name] = attr.default
    end
  end
  if attrs ~= nil then
    for key, value in pairs(attrs) do
      local index = spec.attr_index[key]
      if index == nil then
        fail(E_ATTR, "schema `" .. schema_name .. "` 未声明属性 `" .. tostring(key) .. "`")
      end
      local decl = spec.attrs[index]
      if not value_ok(decl.type, value) then
        fail(
          E_ATTR,
          "属性 `" .. tostring(key) .. "` 的值不在 `" .. decl.type .. "` 值域内（得到 " .. type(value) .. "）"
        )
      end
      final[key] = value
    end
  end
  for i = 1, #spec.attrs do
    local attr = spec.attrs[i]
    if attr.required and final[attr.name] == nil then
      spec_error("schema `" .. schema_name .. "` 的必填属性 `" .. attr.name .. "` 缺失")
    end
  end

  local tag_list = tag_list_of(spec, tags)

  -- 校验全部通过，从这一行起才碰宿主。
  local id = h.entity.spawn()
  h.kv.set(K.rec .. id .. ".schema", schema_name)
  for i = 1, #spec.attrs do
    local attr = spec.attrs[i]
    local value = final[attr.name]
    if value ~= nil then
      h.entity.set_attribute(id, attr.name, value)
    end
  end
  for i = 1, #tag_list do
    h.entity.add_tag(id, tag_list[i])
  end
  -- 存活位不再落 KV（S9）：存活 = 内核存在性，spawn 出来就在世。
  shift(h, K.count, 1)
  local seq = h.kv.get(K.seq)
  if type(seq) ~= "number" or seq < id then
    h.kv.set(K.seq, id)
  end
  h.emit("entity.created", id)
  return id
end

--- `entity.get(id, key)`：存活门 + 一次内核属性读（未声明键读不到值，天然 nil）。
function M.get(id, key)
  local h = host()
  if not alive(h, id) then
    return nil
  end
  if type(key) ~= "string" then
    return nil
  end
  return h.entity.get_attribute(id, key)
end

--- `entity.attrs(id)`：该实体**实际有值**的全部属性（浅副本；值域是标量，浅即完整）。
---
--- 遍历 schema 声明的属性名表而不是内核的属性迭代面：内核没给「列属性」的 Lua
--- 面（只有逐个 `get_attribute`），而属性名集合 = schema 声明集这条不变量已由
--- `spawn` / `set` 的键校验保证。
function M.attrs(id)
  local h = host()
  local out = {}
  if not alive(h, id) then
    return out
  end
  memo_sync(h)
  local spec = memo.specs[h.kv.get(K.rec .. id .. ".schema")]
  if spec == nil then
    return out
  end
  for i = 1, #spec.attrs do
    local attr_name = spec.attrs[i].name
    local value = h.entity.get_attribute(id, attr_name)
    if value ~= nil then
      out[attr_name] = value
    end
  end
  return out
end

--- `entity.set(id, key, value)`：写一个属性；返回「值是否真的变了」。
---
--- 同值写入返回 `false` 且**不发事件**（契约 I6）——“有没有变”由本包判定而不交给
--- 内核：内核 `set_attribute` 对同值写只做到「不计 revision」，而事件的恰好一次
--- 是本包的语义。整/浮身份参与比较：`1` 与 `1.0` 在存诸面是两个值。
function M.set(id, key, value)
  local h = host()
  if not alive(h, id) then
    fail(E_ATTR, "实体 " .. tostring(id) .. " 不存活，不可写属性")
  end
  local spec = require_spec(h, h.kv.get(K.rec .. id .. ".schema"))
  local index = spec.attr_index[key]
  if index == nil then
    fail(E_ATTR, "实体 " .. tostring(id) .. " 的 schema 未声明属性 `" .. tostring(key) .. "`")
  end
  local decl = spec.attrs[index]
  if not value_ok(decl.type, value) then
    fail(
      E_ATTR,
      "属性 `" .. tostring(key) .. "` 的值不在 `" .. decl.type .. "` 值域内（得到 " .. type(value) .. "）"
    )
  end
  local current = h.entity.get_attribute(id, key)
  if current ~= nil and current == value and is_integer(current) == is_integer(value) then
    return false
  end
  h.entity.set_attribute(id, key, value)
  h.emit("entity.attr_changed", tostring(id) .. "|" .. key .. "|" .. tostring(value))
  return true
end

-- ── 标签 / 存活集合 / 生命周期 ───────────────────────────────────────────────
--
-- 标签用**内核标签面**（`add_tag` / `remove_tag` / `has_tag`）：它们随存档往返、
-- 返回「是否真的变了」、且不用本包自己维护第二份集合。代价是「有哪些 tag」不可
-- 枚举（内核没有列 tag 面）——所以 tag 也必须是 schema 声明的（D8）：未声明的 tag
-- 无法出现在 `destroy` 的清理名单里，会变成永远擦不掉的残留。

--- `entity.tags(id)`：已声明的、当前存在的 tag 名（升序数组）。
---
--- 契约 §4 的 API 表原本只有 `has`；导出侧（codec）需要「把一条记录的 tag 全取
--- 出来」而 KV 不可遍历，故补这一面（已回写 entity.md §4）。
function M.tags(id)
  local h = host()
  local out = {}
  if not alive(h, id) then
    return out
  end
  memo_sync(h)
  local spec = memo.specs[h.kv.get(K.rec .. id .. ".schema")]
  if spec == nil then
    return out
  end
  for i = 1, #spec.tags do
    local tag_name = spec.tags[i].name
    if h.entity.has_tag(id, tag_name) then
      out[#out + 1] = tag_name
    end
  end
  return out
end

--- `entity.has(id, tag)`：纯读；死亡实体与未声明 tag 都是 `false`。
function M.has(id, tag_name)
  local h = host()
  if not alive(h, id) then
    return false
  end
  if type(tag_name) ~= "string" then
    return false
  end
  return h.entity.has_tag(id, tag_name)
end

--- `entity.tag` / `entity.untag` 的共同体：声明校验 → 存活门 → 内核变更 → 事件。
local function tag_write(id, tag_name, adding)
  local h = host()
  if not alive(h, id) then
    fail(E_ATTR, "实体 " .. tostring(id) .. " 不存活，不可改标签")
  end
  local spec = require_spec(h, h.kv.get(K.rec .. id .. ".schema"))
  if spec.tag_index[tag_name] == nil then
    spec_error("schema `" .. spec.name .. "` 未声明 tag `" .. tostring(tag_name) .. "`")
  end
  local changed
  if adding then
    changed = h.entity.add_tag(id, tag_name)
  else
    changed = h.entity.remove_tag(id, tag_name)
  end
  if changed then
    h.emit(adding and "entity.tagged" or "entity.untagged", tostring(id) .. "|" .. tag_name)
  end
  return changed
end

--- `entity.tag(id, tag)`：加标签；返回「是否真的变了」（已存在 = `false`）。
function M.tag(id, tag_name)
  return tag_write(id, tag_name, true)
end

--- `entity.untag(id, tag)`：去标签；返回「是否真的变了」（原本没有 = `false`）。
function M.untag(id, tag_name)
  return tag_write(id, tag_name, false)
end

--- `entity.exists(id)`：一次内核存在性读，**零 KV 读、绝不扫键**（契约 I5）。
function M.exists(id)
  return alive(host(), id)
end

--- `entity.schema_of(id)`：实体的 schema 名；死亡或不存在 = `nil`。
function M.schema_of(id)
  local h = host()
  if not alive(h, id) then
    return nil
  end
  return h.kv.get(K.rec .. id .. ".schema")
end

--- `entity.count()`：存活实体数（KV 计数器，O(1)）。
function M.count()
  local current = host().kv.get(K.count)
  if type(current) ~= "number" then
    return 0
  end
  return current
end

--- `entity.ids(tag?, schema?)`：存活 id 升序数组；可过 tag / schema。
---
--- 扫 `1..seq` 而不是扫 KV 键（宿主没有遍历面）：死亡位与「内核直生 id」自动
--- 跳过，未到达的水位不会多扫。复杂度 O(seq)，契约已将 `ids` / `count` 登记为 O(n)。
function M.ids(tag_filter, schema_filter)
  local h = host()
  local out = {}
  local seq = h.kv.get(K.seq)
  if type(seq) ~= "number" then
    return out
  end
  for id = 1, seq do
    if alive(h, id) then
      local keep = true
      if schema_filter ~= nil then
        keep = h.kv.get(K.rec .. id .. ".schema") == schema_filter
      end
      if keep and tag_filter ~= nil then
        keep = h.entity.has_tag(id, tag_filter)
      end
      if keep then
        out[#out + 1] = id
      end
    end
  end
  return out
end

--- `entity.destroy(id)`：**内核真删**（属性 + tag + 记录一并回收），**不**遍历、**不清**引用。
---
--- 三次契约修订（S9）确定了这个形状：
---   * 内核有 `destroy` 面了——所以销毁 = `EntityStore::remove`，不再靠存活位遮蔽；
---   * KV 仍无删除——所以 `rec.<id>.schema` 留着（每次 destroy 恰好留 1 枚键，见 I5）；
---   * 引用的清理归 mod 语义（「召唤物死亡，主人失去它」是规则），包不代决（契约 I4）。
function M.destroy(id)
  local h = host()
  if not alive(h, id) then
    return false
  end
  -- 真删（属性 / tag / 记录一起走）。返回值的 false 分支不必处理：存活门刚证过它在世；
  -- 就算内核侧已先一步删掉（绕过包的直呼），这里的 false 也只是「已经不在了」。
  h.entity.destroy(id)
  shift(h, K.count, -1)
  h.emit("entity.destroyed", id)
  return true
end

return M

