-- `pkg.quest.store` —— 任务包的 KV 管道（键表、标量纪律、名字表、id 索引）。
--
-- # 为什么单独一个文件
--
-- 三件事只在这里写一次：① 键名的**唯一**出处（`K`）——任何函数内联拼 `"quest."`
-- 就是第二个写者，前缀单写者纪律当场破；② 沙箱方言的骨架工具（无 `string` /
-- `table` / `math` 表，只能方法语法 + 手写排序）；③ id 索引面（`quest.index.<id>`
-- 与 `quest.seq`）。
--
-- # 键的物理形状（契约 §2；回写 D10）
--
-- 宿主 `kv.set` 只收四个标量型（`crates/ygg-sandbox/src/host_api.rs:568`
-- `value_from_lua`，表写入即 `unsupported payload type: table`），所以契约里
-- 「表值」的 `types` / `inst` / `index` 全部落成**扁平标量键 + 一个换行名表**：
--
--   quest.seq                        number  已分配的最大 id（单写口 = `open`）
--   quest.index.<id>                 true    id 索引真源（逐 id 一枚标记）
--   quest.types                      string  类型名表（升序、换行连接）
--   quest.t.<name>.steps             number  步骤数
--   quest.t.<name>.s.<i>.{id,guide,on_enter}
--   quest.t.<name>.m.names           string  meta 键名表
--   quest.t.<name>.m.<key>           string  meta 类型词（number|string|boolean|ref）
--   quest.inst.<id>.{type,step,state,giver,taker}
--   quest.inst.<id>.m.<key>          标量   meta 值（只写调用方给的键）
--
-- `quest.index.<id>` 不省的理由：它就是 §5「泄漏检测」要数的那个口径（每轮
-- reset() 后条目数恒为 1），也是 `open_ids` 的成员判据——ids 连续不等于「还在
-- 索引里」，两件事分开存，断言才有意义。
--
-- 前缀单写者：本包只写 `quest.` 前缀（`docs/design/mod/mod-package.md` §3.2.3）。

local S = {}

-- ── KV 键（唯一出处）────────────────────────────────────────────────────────
S.K = {
  seq = "quest.seq",
  index = "quest.index.",
  types = "quest.types",
  type_prefix = "quest.t.",
  inst_prefix = "quest.inst.",
}

-- 错误码家族 `YGG-PKG-QUEST-<COND>`（契约 §4；回写 D1 逐条列全）。
S.E = {
  NAME = "YGG-PKG-QUEST-NAME",
  TEMPLATE = "YGG-PKG-QUEST-TEMPLATE",
  TYPE = "YGG-PKG-QUEST-TYPE",
  META = "YGG-PKG-QUEST-META",
  REF = "YGG-PKG-QUEST-REF",
  ID = "YGG-PKG-QUEST-ID",
  STEP = "YGG-PKG-QUEST-STEP",
  TERMINAL = "YGG-PKG-QUEST-TERMINAL",
  INSTALL = "YGG-PKG-QUEST-INSTALL",
  HOST = "YGG-PKG-QUEST-HOST",
}

--- 报错：`<码>: <细节>`（level 0 = 不夹带调用点；用例按码做子串断言）。
function S.fail(code, detail)
  error(code .. ": " .. detail, 0)
end

-- ── 宿主句柄（install 只记 deps；契约 I8）────────────────────────────────────

local installed = nil

--- 装配：记下可选依赖。缺省 `ygg` 全局，selftest 用 `{ host = mock }` 注入。
function S.install(deps)
  if deps ~= nil and type(deps) ~= "table" then
    S.fail(S.E.INSTALL, "install 只收 table 或 nil，得到 " .. type(deps))
  end
  installed = { host = (type(deps) == "table" and deps.host) or nil }
  return S
end

--- 取本次调用的宿主句柄（`deps.host` 优先，其次 `ygg` 全局）。
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

-- ── 无标准库的骨架工具（与 entity 各自几行，不抽第三个包）───────────────────

--- 字符串拼接（沙箱里 `table.concat` 不可达；O(n) 两两合并，理由同 codec 的 `join`）。
local function concat(parts, sep)
  local n = #parts
  if n == 0 then
    return ""
  end
  local width = 1
  while width < n do
    local index = 1
    while index <= n do
      local right = parts[index + width]
      if right ~= nil then
        parts[index] = parts[index] .. sep .. right
      end
      index = index + width * 2
    end
    width = width * 2
  end
  return parts[1]
end

--- 数组 → 换行名表（空数组 = 空串）。
function S.join(lines)
  return concat(lines, "\n")
end

--- 换行名表 → 数组（空串 = 空数组，不产生一个空名）。
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

--- 原地插入排序（字节序升序）。名字表要确定性字节，`pairs` 序做不到。
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

--- 字节是否 ASCII 字母（手写：沙箱无 `string` 表，只用 `:byte`）。
local function is_alpha(b)
  return (b >= 65 and b <= 90) or (b >= 97 and b <= 122)
end

--- 名字是否合法：首字符 `[A-Za-z_]`，其余 `[A-Za-z0-9_.:-]`。
---
--- 换行必须被拒：名表是换行连接的字符串，一个带换行的名字会把表劈成两半。
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

--- 类型名是否合法：`^[a-z][a-z0-9_-]*$`（契约 §2 的 id 集）。
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

-- ── 标量纪律 ────────────────────────────────────────────────────────────────

--- 是否有限数（`nan ~= nan`；`±inf` 与自身相等但被 `> 1` 判出）。
local function finite(v)
  if v ~= v then
    return false
  end
  return v <= 1.0 / 0.0 and v >= -1.0 / 0.0
end

--- 值必须是宿主 KV / 事件能收的标量（表、函数在宿主导线即被拒，提前给可读码）。
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

--- ref 校验：`nil` 或正整数 id（`0` 表示「无」由调用方自己用 0 表示）。
function S.ref_ok(value)
  if value == nil then
    return true
  end
  if type(value) ~= "number" or not finite(value) then
    return false
  end
  if value < 1 then
    return false
  end
  return value % 1 == 0
end

-- ── KV 读写（两个方法而已；宿主没有遍历也没有删除）──────────────────────────

function S.get(h, key)
  return h.kv.get(key)
end

function S.set(h, key, value)
  S.scalar(value, S.E.TEMPLATE, "KV 值 " .. key)
  h.kv.set(key, value)
end

-- ── id 与索引 ───────────────────────────────────────────────────────────────

--- 已分配的最大 id（缺键 = 0；本包全部 id 从 1 起连续）。
function S.seq(h)
  local value = S.get(h, S.K.seq)
  if value == nil then
    return 0
  end
  return value
end

function S.index_key(id)
  return S.K.index .. tostring(id)
end

--- 索引成员判据（`quest.index.<id>` 为真）。
function S.indexed(h, id)
  return S.get(h, S.index_key(id)) == true
end

--- 索引里的 id 数组（升序；扫描上界 = `quest.seq`）。
function S.ids(h)
  local out = {}
  for id = 1, S.seq(h) do
    if S.indexed(h, id) then
      out[#out + 1] = id
    end
  end
  return out
end

function S.inst_key(id, field)
  return S.K.inst_prefix .. tostring(id) .. "." .. field
end

function S.type_key(name, suffix)
  return S.K.type_prefix .. name .. "." .. suffix
end

return S
