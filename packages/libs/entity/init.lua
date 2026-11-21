-- `pkg.entity` —— 实体与属性袋（机制层包）。契约：`docs/design/packages/entity.md`。
--
-- # 这个包做什么
--
-- 三件事：按 schema 分配 id、把属性装进内核属性面、在状态真的变化时发一条事件。
-- 类型由 schema 表达（数据），所以「新增一种存在」= 改数据，不改本文件。
--
-- # 实现纪律（沙箱方言；契约 §8「设计依据」记着为什么）
--
-- 1. **没有标准库表**：沙箱 globals 里 `string` / `table` / `utf8` / `coroutine` /
--    `math` 全为 nil（`crates/ygg-sandbox/src/sandbox.rs` 的白名单测试钉死；`math`
--    在 driver 路径由 `inject::install` 注入，裸沙箱缺席——包不得依赖那条注入）。
--    字符串操作只能走方法语法（`("x"):sub(1, 1)`），表工具（排序 / 拼接）与
--    整/浮判据手写在下面。本文件里若出现标准库表的点式调用，就会在真实跑局里
--    nil-index 崩，而 selftest（普通 Lua）**看不出来**——所以交付门禁带一条静态
--    grep（`tests/packages_sandbox.rs` 用例 ③）。
-- 2. **模块顶层纯**：只有 local / 常量 / 函数定义。任何 `ygg` 访问都发生在被调用
--    的函数体里，且经 `host()` 单点取句柄。
-- 3. **名字字符集** `^[A-Za-z_][A-Za-z0-9_.:-]*$`：名表是换行连接的字符串（KV 存
--    不了数组），所以空白/换行必须被拒——否则名表会被一个名字劈成两半。
-- 4. **KV 只收标量**：没有表、没有删除、没有遍历（`crates/ygg-core/src/kv.rs`）。
--    一切「集合」都落成「逐项标量键 + 一个名字枚举键」，见 §2 的物理落点表。
--
-- 前缀单写者：本包只写 `pkg.entity.*`（`docs/design/mod/mod-package.md` §3.2.3）。

local M = { VERSION = "0.1.0" }

-- ── KV 键（§2 物理落点表的**唯一**出处；任何函数不得内联拼这些前缀）──────────────
local K = {
  schemas = "pkg.entity.schemas",
  schemas_rev = "pkg.entity.schemas_rev",
  schema = "pkg.entity.schema.",
  rec = "pkg.entity.rec.",
  count = "pkg.entity.count",
  seq = "pkg.entity.seq",
}

-- 错误码家族 `YGG-PKG-ENTITY-<COND>`（契约只固定 SCHEMA / ATTR；HOST 是环境面）。
local E_SCHEMA = "YGG-PKG-ENTITY-SCHEMA"
local E_ATTR = "YGG-PKG-ENTITY-ATTR"
local E_HOST = "YGG-PKG-ENTITY-HOST"

-- schema spec 的合法属性类型（`ref` = 正整数 id，与 number 分开声明）。
local TYPES = { number = true, string = true, boolean = true, ref = true }

--- 报错：`<码>: <细节>`（level 0 = 不夹带调用点，测试按码做子串断言）。
local function fail(code, detail)
  error(code .. ": " .. detail, 0)
end

-- ── 无标准库的骨架工具 ────────────────────────────────────────────────────────

--- 字节是否 ASCII 字母。
local function is_alpha(b)
  return (b >= 65 and b <= 90) or (b >= 97 and b <= 122)
end

--- 名字是否合法：首字符 `[A-Za-z_]`，其余 `[A-Za-z0-9_.:-]`。
---
--- 手写而不引入 pattern 库：沙箱里 `string` 表不可达，而 `:find` 的 pattern 语法
--- 也是标准库的一部分——只用 `:byte` 让这条校验在**任何**载体上同一份字节。
local function name_ok(name)
  if type(name) ~= "string" or #name == 0 then
    return false
  end
  local first = name:byte(1)
  if not (is_alpha(first) or first == 95) then
    return false
  end
  for i = 2, #name do
    local b = name:byte(i)
    if not (is_alpha(b) or (b >= 48 and b <= 57) or b == 95 or b == 46 or b == 58 or b == 45) then
      return false
    end
  end
  return true
end

--- 有限数（NaN / ±∞ 不是属性值：内核 `EntityStore::set_attribute` 同样拒）。
---
--- `v - v == 0` 同时排除两类：`inf - inf = nan ≠ 0`、`nan - nan = nan ≠ 0`；
--- 有限数恒 `0`。调用点全在 `type(v) == "number"` 之后（`value_ok` 先判类型）。
local function finite(v)
  return v - v == 0
end

--- 数是不是 Lua 5.4 的 **integer**（与 `pkg.codec` 的同名谓词各自一份）。
---
--- 沙箱不保证注入面那个整/浮类型函数（`math` 只由驱动注入，包不得依赖它，见文件头
--- 第 1 条），而整数与浮点在算术、比较、`//`、`%` 下完全不可区分（`1 == 1.0`）。能
--- 区分身份的只剩文本形态：整数按 `%d` 打印（无 `.`、无 `e`），浮点按 `%.14g` 打印
--- 且「整数形」补 `.0`（`tostring(1)="1"` / `tostring(1.0)="1.0"`）。
---
--- 为什么不能写 `v % 1 == 0`：`2.0 % 1 == 0.0`，整值浮点会被吞成整型。而 `ref` 值域与
--- `set` 的同值判定都以**身份**为判据（KV 写 `tostring(id)`，`2` 与 `2.0` 是两个键）。
---
--- 只用 `:byte` 不引入 pattern：与上面 `name_ok` 同一条纪律（`string` 全局缺席，
--- 字符串方法经元表可达）。
---
--- 不抽第三个包：与 `codec` 各自一份，是「抽出去就多一条依赖边」的同一笔账
--- （契约 §1.4）。
local function is_integer(v)
  if type(v) ~= "number" or v - v ~= 0 then
    return false
  end
  local text = tostring(v)
  for i = 1, #text do
    local b = text:byte(i)
    if b == 46 or b == 101 then -- '.' / 'e'
      return false
    end
  end
  return true
end

--- 换行分隔的名表 → 数组（空串 → 空数组）。
--- KV 存不了数组，所以「已定义的 schema 名」「某 schema 的属性名」都是这个形状。
local function split_lines(text)
  local out = {}
  if type(text) ~= "string" or #text == 0 then
    return out
  end
  local start = 1
  while true do
    local at = text:find("\n", start, true)
    if at == nil then
      out[#out + 1] = text:sub(start)
      return out
    end
    out[#out + 1] = text:sub(start, at - 1)
    start = at + 1
  end
end

--- 数组 → 换行分隔的名表（`split_lines` 的逆；元素必须已过 `name_ok`）。
local function join_lines(items)
  local parts = {}
  for i = 1, #items do
    parts[i] = items[i]
  end
  local out = ""
  for i = 1, #parts do
    out = (i == 1) and parts[1] or (out .. "\n" .. parts[i])
  end
  return out
end

--- 原地插入排序（字节序升序）。没有标准库排序，而名表的序必须是确定的。
---
--- 比较用 Lua 的 `<`（C locale 下即字节序），名字字符集全是 ASCII
--- （`name_ok` 已保证），因此与 JSON 编码器的键序口径一致。
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

local function copy_array(items)
  local out = {}
  for i = 1, #items do
    out[i] = items[i]
  end
  return out
end

-- ── 宿主句柄（install 只记 deps，不预读不预写；契约 I7）───────────────────────

local installed = nil

--- 装配：记下可选依赖。缺省 `{ host = ygg }`，selftest 用 `{ host = mock }` 注入。
function M.install(deps)
  installed = { host = (type(deps) == "table" and deps.host) or nil }
  return M
end

--- 取本次调用的宿主句柄（`deps.host` 优先，其次 `ygg` 全局）。
local function host()
  local h = installed and installed.host or nil
  if h == nil then
    h = ygg
  end
  if h == nil then
    fail(E_HOST, "ygg 不可达：沙箱外调用需 `install({ host = ... })`")
  end
  return h
end

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
function M.define(name, raw_spec)
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
function M.schema(name)
  local h = host()
  memo_sync(h)
  local spec = memo.specs[name]
  if spec == nil then
    return nil
  end
  return copy_spec(spec)
end

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
