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

-- # 文件切分（批十一；纯搬移，装配范式同 quest 三件套）
--
--   base.lua    KV 键唯一出处 / 错误码 / 词典 util / `installed` 状态座与 host()
--   schema.lua  spec 的编译、比较、落盘、memo（`define` 的语义核心）
--   record.lua  存活判定 / 计数器 / 实体生命周期（spawn…destroy 17 API）
--   query.lua   通用查询的包面与两道门（schema 面；S-24）
--
-- 依赖向无环：base ← schema ← record ← query。本文件只做装配与转口：一个 API
-- 一行的转发层，导出表面逐键不变（18 键，entity.md §4）。

local base = require("pkg.entity.base")
local schema = require("pkg.entity.schema")
local record = require("pkg.entity.record")
local query = require("pkg.entity.query")

local M = { VERSION = "0.1.0" }

--- 装配：记下可选依赖。缺省 `{ host = ygg }`，selftest 用 `{ host = mock }` 注入。

function M.install(deps)
  base.install(deps)
  return M
end

--- `entity.define(name, spec)`：注册（或幂等确认）一个 schema（主体在 schema）。
function M.define(name, raw_spec)
  return schema.define(name, raw_spec)
end

--- `entity.schema(name)`：读一份 spec 的深副本；未注册返回 `nil`。
function M.schema(name)
  return schema.schema(name)
end

--- `entity.spawn(name, attrs?, tags?)`：按 schema 分配 id 并写入属性袋。
function M.spawn(schema_name, attrs, tags)
  return record.spawn(schema_name, attrs, tags)
end

--- `entity.get(id, key)`：存活门 + 一次内核属性读。
function M.get(id, key)
  return record.get(id, key)
end

--- `entity.attrs(id)`：该实体实际有值的全部属性（浅副本）。
function M.attrs(id)
  return record.attrs(id)
end

--- `entity.set(id, key, value)`：写一个属性；返回「值是否真的变了」。
function M.set(id, key, value)
  return record.set(id, key, value)
end

--- `entity.tags(id)`：已声明的、当前存在的 tag 名（升序数组）。
function M.tags(id)
  return record.tags(id)
end

--- `entity.has(id, tag)`：纯读；死亡实体与未声明 tag 都是 `false`。
function M.has(id, tag_name)
  return record.has(id, tag_name)
end

--- `entity.tag(id, tag)`：加标签；返回「是否真的变了」。
function M.tag(id, tag_name)
  return record.tag(id, tag_name)
end

--- `entity.untag(id, tag)`：去标签；返回「是否真的变了」。
function M.untag(id, tag_name)
  return record.untag(id, tag_name)
end

--- `entity.exists(id)`：一次内核存在性读，零 KV 读、绝不扫键。
function M.exists(id)
  return record.exists(id)
end

--- `entity.schema_of(id)`：实体的 schema 名；死亡或不存在 = `nil`。
function M.schema_of(id)
  return record.schema_of(id)
end

--- `entity.count()`：存活实体数（KV 计数器，O(1)）。
function M.count()
  return record.count()
end

--- `entity.ids(tag?, schema?)`：存活 id 升序数组；可过 tag / schema。
function M.ids(tag_filter, schema_filter)
  return record.ids(tag_filter, schema_filter)
end

--- `entity.query(selector)`：通用查询（schema 门 + 未声明键门；主体在 query）。
function M.query(selector)
  return query.query(selector)
end

--- `entity.destroy(id)`：内核真删（属性 + tag + 记录一并回收）。
function M.destroy(id)
  return record.destroy(id)
end

return M
