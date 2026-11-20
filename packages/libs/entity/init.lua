-- `pkg.entity` —— 实体与属性袋（机制层包）。契约：`docs/design/packages/entity.md`。
--
-- # 这个包做什么
--
-- 三件事：按 schema 分配 id、把属性装进内核属性面、在状态真的变化时发一条事件。
-- 类型由 schema 表达（数据），所以「新增一种存在」= 改数据，不改本文件。
--
-- # 实现纪律（沙箱方言；契约 §8「设计依据」记着为什么）
--
-- 1. **没有标准库表**：沙箱 globals 里 `string` / `table` / `utf8` / `coroutine`
--    全为 nil（`crates/ygg-sandbox/src/sandbox.rs` 的白名单测试钉死）。字符串操作
--    只能走方法语法（`("x"):sub(1, 1)`），表工具（排序 / 拼接）手写在下面。
--    本文件里出现 `string.` 或 `table.sort` 即会在真实跑局里 nil-index 崩，
--    而 selftest（普通 Lua）**看不出来**——所以交付门禁带一条静态 grep。
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
  alive = "pkg.entity.alive.",
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
local function finite(v)
  return v == v and v ~= math.huge and v ~= -math.huge
end

--- 属性值域：boolean / 有限 number / string（契约 I2）。
local function scalar(v)
  local t = type(v)
  if t == "number" then
    return finite(v)
  end
  return t == "string" or t == "boolean"
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

--- 原地插入排序（字节序升序）。没有 `table.sort`，而名表的序必须是确定的。
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

return M
