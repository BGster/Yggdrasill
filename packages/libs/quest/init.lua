-- `pkg.quest` —— 任务类型、实例与委托关系（机制层包）。契约：`openwiki/contracts/packages/quest.md`。
--
-- # 这个包做什么
--
-- 一类流程（模板）+ 多个在跑的具体实例 + 谁给的谁接的。步骤推进只发五个派生事件
-- 名，不发 `on_enter`、不做 UI、不判「谁能接」——判定是 mod 的规则（契约 §1）。
--
-- # 实现纪律（沙箱方言）
--
-- 1. **没有标准库表**：沙箱 globals 里 `string` / `table` / `math` 全为 nil
--    （`crates/ygg-sandbox/src/sandbox.rs` 的白名单测试钉死）。拼接 / 切分 /
--    排序手写在 `store.lua`，只用方法语法（`("x"):find(…)`）。
-- 2. **模块顶层纯**：只有 local / 常量 / 函数定义。所有 `ygg` 访问都在被调用的
--    函数体里，经 `store.host()` 单点取句柄（契约 I8）。
-- 3. **KV 只收标量**：没有表、没有删除、没有遍历（`crates/ygg-core/src/kv.rs`），
--    所以一切集合都落成「逐项标量键 + 一个换行名表」，物理形状见 `store.lua` 头注。
--
-- # 文件切分
--
--   store.lua  键名唯一出处 / 标量纪律 / id 索引 / 无标准库骨架
--   types.lua  流程模板的编译、落盘、比较（`define` 的语义核心）
--   flow.lua   实例原语（迁移表查表）与读数（`step` / `open_ids` / `keys`）
--
-- 本文件只做装配与转发：一个 API 一行的转发层，逻辑都在上面三处。

local store = require("pkg.quest.store")
local types = require("pkg.quest.types")
local flow = require("pkg.quest.flow")

local M = { VERSION = "0.1.0" }

--- 派生事件面（恰好五个名字；契约 I6）。dialogue 的桥只读这张表，不猜名字。
M.EVENTS = {
  "quest.opened",
  "quest.advanced",
  "quest.completed",
  "quest.abandoned",
  "quest.jumped",
}

--- 错误码面（契约 §4 的表在同一次提交里逐条列全，见回写 D1）。
M.ERRORS = {}
for key, code in pairs(store.E) do
  M.ERRORS[key] = code
end

--- 迁移表（`state × 原语 → { 迁移目标, 事件名 }`；终态缺席 = 一律被拒）。
M.TRANSITIONS = flow.TRANSITIONS

--- 装配：只记 deps（不预读模板、不写 KV；契约 I8）。
function M.install(deps)
  store.install(deps)
  return M
end

--- `define(name, template)` → `true`（同 spec 幂等零写；异 spec 报 `-TYPE`）。
function M.define(name, template)
  return types.define(store.host(), name, template)
end

--- `type(name)` → 模板 | `nil`（每次重建副本，调用方改不坏注册表）。
function M.type(name)
  return types.get(store.host(), name)
end

--- `open(type_name, opts?)` → 新 id（`+ 事件 quest.opened`）。
function M.open(type_name, opts)
  return flow.open(store.host(), type_name, opts)
end

--- `get(id)` → 实例浅副本 | `nil`。
function M.get(id)
  return flow.get(store.host(), id)
end

--- `assign(id, giver, taker)` → `true`（只写 ref，不判权限、不发事件）。
function M.assign(id, giver, taker)
  return flow.assign(store.host(), id, giver, taker)
end

--- `advance(id)` → 推进后的 `state`（末步转 `done`）。
function M.advance(id)
  return flow.advance(store.host(), id)
end

--- `jump(id, step_id)` → `true`（显式跳步，留 `quest.jumped`）。
function M.jump(id, step_id)
  return flow.jump(store.host(), id, step_id)
end

--- `abandon(id, reason?)` → `true`。
function M.abandon(id, reason)
  return flow.abandon(store.host(), id, reason)
end

--- `open_ids(owner_ref?)` → `{id, …}`（升序；只含 `state == "open"`）。
function M.open_ids(owner_ref)
  return flow.open_ids(store.host(), owner_ref)
end

--- `step(id)` → `{ index, id, guide, on_enter }` | `nil`。
function M.step(id)
  return flow.step(store.host(), id)
end

--- `keys()` → 本包拥有的 KV 键（升序；`codec.export({ keys = quest.keys() })` 的点名面）。
function M.keys()
  return flow.keys(store.host())
end

return M
