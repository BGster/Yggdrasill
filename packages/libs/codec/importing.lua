-- `pkg.codec.importing` —— import：两阶段（validate → apply）三种冲突策略。
--
-- 契约：`docs/design/packages/codec.md` §4。批十一拆分（纯搬移）：validate 的
-- 记录校验取 spec 经 `pkg.entity` 的 API（包间依赖边，同原单文件）；checksum 与
-- copy_top 取自 `pkg.codec.bundle`（单源）；is_integer / sort_names 取自 jsonenc。
-- 「保 id」、引用不重写（I6）、kv 不写回（D9）见 `M.import` 头注。

local runtime = require("pkg.codec.runtime")
local jsonenc = require("pkg.codec.jsonenc")
local bundle_mod = require("pkg.codec.bundle")
-- `entity` 取**当前** `pkg.entity` 实例（转发代理，装配层器件）：单文件时代
-- capture 发生在 init.lua 自身（消费夹具 fresh() 清 `pkg.codec` 即重捕获）；拆分后
-- capture 点在本文件，而夹具只清 `pkg.codec` / `pkg.entity` 两键——代理让每次访问
-- 解析 require 缓存里的当前实例，保住这条消费者可见契约。沙箱白名单有
-- `setmetatable`（sandbox.rs）。
local entity = setmetatable({}, {
  __index = function(_, key)
    return require("pkg.entity")[key]
  end,
})

local fail = runtime.fail
local E_TYPE = runtime.E_TYPE
local E_CONFLICT = runtime.E_CONFLICT
local E_CHECKSUM = runtime.E_CHECKSUM
local E_VERSION = runtime.E_VERSION
local CODEC_VERSION = runtime.CODEC_VERSION
local is_integer = jsonenc.is_integer
local sort_names = jsonenc.sort_names

local M = {}

-- ── import：两阶段（validate → apply）三种冲突策略 ─────────────────────────

local CONFLICT_POLICIES = { skip = true, overwrite = true, rename = true }

--- 属性值域判定（4 行）：与 entity 各自实现，**不**抽第三个包——抽出去就多
--- 一条依赖边，收益小于一条边（契约 §1.4）。
local function value_ok(kind, value)
  if kind == "number" then
    return type(value) == "number" and value - value == 0
  elseif kind == "string" then
    return type(value) == "string"
  elseif kind == "boolean" then
    return type(value) == "boolean"
  elseif kind == "ref" then
    return type(value) == "number" and is_integer(value) and value >= 1
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
  if type(id) ~= "number" or not is_integer(id) or id < 1 then
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
    local actual = bundle_mod.checksum(bundle)
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
    report.kv = bundle_mod.copy_top(bundle.kv)
  end
  return report
end

return M

