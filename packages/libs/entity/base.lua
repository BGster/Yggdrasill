-- `pkg.entity.base` —— entity 的共享座：KV 键唯一出处、错误码、词典 util、
-- `installed` 状态座与 `host()`。
--
-- 契约：`docs/design/packages/entity.md`。批十一拆分的叶子模块：`fail` / `E_*` /
-- `K` / 词典 util 只住这一处（共享 local 禁止抄第二份），schema / record / query
-- 经 `require("pkg.entity.base")` 取用。与原单文件的唯一差异（计划 §2 授权项）：
-- `M.install` 的记位函数落座本模块（init.lua 的 `M.install` 转发），返回表由包表
-- 换为本模块表——观察面语义不变。

local B = {}

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
function B.install(deps)
  installed = { host = (type(deps) == "table" and deps.host) or nil }
  return B
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

-- ── 导出面（schema / record / query / init.lua 取用）─────────────────────────

B.K = K
B.E_SCHEMA = E_SCHEMA
B.E_ATTR = E_ATTR
B.E_HOST = E_HOST
B.TYPES = TYPES
B.fail = fail
B.name_ok = name_ok
B.finite = finite
B.is_integer = is_integer
B.split_lines = split_lines
B.join_lines = join_lines
B.sort_names = sort_names
B.copy_array = copy_array
B.host = host

return B

