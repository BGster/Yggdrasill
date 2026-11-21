-- `pkg.codec.bundle` —— checksum / export / write / read / roundtrip（存档往返）。
--
-- 契约：`docs/design/packages/codec.md` §4。批十一拆分（纯搬移）：`checksum` 的
-- sha256 与 encode 分别取自 runtime / jsonenc（单源，不抄第二份）；`export` 的
-- 记录取自 `pkg.entity` 的 API（包间依赖边，同原单文件 top-level require）。
-- 字节纪律（键序 / checksum 置空 / 只校回执字节数）见各函数头注与契约 I4。

local runtime = require("pkg.codec.runtime")
local jsonenc = require("pkg.codec.jsonenc")
local jsondec = require("pkg.codec.jsondec")
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
local E_CHECKSUM = runtime.E_CHECKSUM
local E_MISSING = runtime.E_MISSING
local CODEC_VERSION = runtime.CODEC_VERSION
local host = runtime.host

local M = {}

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
  return runtime.sha256(jsonenc.encode(canonical))
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

--- `codec.write(path, bundle)`：算 checksum → 编码 → 落盘 → 校**回执字节数**。
---
--- 返回 `{ bytes, sha256 }`，sha256 **是我方算法算的**（它是「写进 bundle 的那个
--- 值」）。宿主回的 `sha256` 不是互校对象：它算写出文本，而 `checksum(bundle)` 算
--- `checksum=""` 的规范化编码，两份字节永不相同（S16）；内容面由 `read` 的强制
--- checksum 兜底，写侧只比字节数（截断/半写 → `YGG-PKG-CODEC-CHECKSUM`）。
--- 不修改入参（写进文件的是副本）。
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
  local text = jsonenc.encode(payload)

  local written = current.fs.write(path, text)
  if type(written) ~= "table" then
    fail(E_TYPE, "ygg.fs.write 未返回结果表")
  end
  -- 写入完整性只校**字节数**（截断 / 半写）。内容面不在这里重哈希：宿主 `fs.write`
  -- 自己算 `sha256(content)` 并挂在 VFS 条目上（`ygg.fs.stat` 可查），而本包的纯 Lua
  -- sha256 是 ~0.8 MB/s（S19 才下沉）——每写一遍就再哈希整份文本等于把存档耗时翻倍
  -- （实测 66 KiB=90 ms、上限 256 KiB=285 ms）。「档没被改过」的证据仍由 `read` 的
  -- 强制 checksum（I4）在越信任边界时复核；S16 前的互校对象是两份不同字节，永假。
  if type(written.bytes) == "number" and written.bytes ~= #text then
    fail(E_CHECKSUM, "写入字节数不符（宿主 " .. written.bytes .. " / 期望 " .. #text .. "）")
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
  local bundle = jsondec.decode(text)
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
  local text_in = jsonenc.encode(mirror)
  local text_out = jsonenc.encode(loaded)
  return { bytes_in = #text_in, bytes_out = #text_out, equal = text_in == text_out }
end

-- ── 导出面（importing 取 checksum / copy_top）────────────────────────────────

M.copy_top = copy_top

return M

