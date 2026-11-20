-- `packages/libs/role-skill/init.lua` —— 包根模块 `pkg.role_skill`：**纯导出面**。
--
-- 契约：`docs/design/packages/role-skill.md`。顶层只有 `local` / `require` / 函数
-- 定义（I8）；定义、注册与订阅一律在 `M.install(deps)` 或调用期。
--
-- 目录形状与命名（`packages/README.md` §5）：目录名 `role-skill`，模块名里的连字符
-- 折叠成下划线（`pkg.role_skill`）——模块名段字符集只有 `[A-Za-z0-9_]`，折叠是唯一
-- 可行映射。**用例名前缀保留连字符**（`pkg.role-skill.*`），与模块名是两个命名空间。
--
-- # 分工
--
--   `pkg.role_skill.store`   四个 KV 键的唯一读写口 + 依赖解析 + emit
--   `pkg.role_skill.actors`  角色/技能定义表、实体记录、授予/撤销/查询
--   `pkg.role_skill.skills`  技能槽位（rank / charges）
--   `pkg.role_skill.compose` `effective` 的合成规则（只读，从不写回）
--   `pkg.role_skill.counters` 事件计数（订阅面 + 显式清零）
--
-- # 沙箱纪律
--
-- 宿主沙箱只暴露白名单 globals，`math` / `string` / `table` 三张表都不可达
-- （`crates/ygg-sandbox/src/sandbox.rs` 的 `BASE_WHITELIST`）。本包因此不用它们。

local store = require("pkg.role_skill.store")
local actors = require("pkg.role_skill.actors")
local skills = require("pkg.role_skill.skills")
local compose = require("pkg.role_skill.compose")
local counters = require("pkg.role_skill.counters")

local M = { VERSION = "0.1.0" }

--- 派生事件面（恰好四个名字；订阅方只读这张表，不猜名字）。
M.EVENTS = {
  "role.granted",
  "role.revoked",
  "role.skill_gained",
  "role.skill_lost",
}

--- 错误码面（包内唯一一份住在 `store.E`，这里只做对外副本）。
M.ERRORS = {}
for key, code in pairs(store.E) do
  M.ERRORS[key] = code
end

--- KV 键名（四个键的单源；契约 §2）。
M.KEY = store.KEY

--- 装配：`{ host?, ygg?, entity?, codec?, events? }`。
---
--- 只记 deps、装事件订阅面（`events` 非空时）；不预读定义、不写 KV（I8）。
function M.install(deps)
  store.install(deps)
  if type(deps) == "table" and deps.events ~= nil then
    counters.subscribe(deps.events)
  end
  return M
end

--- `role.define(name, spec)` → `true`（同 spec 幂等，异 spec 报 `-DEF`）。
function M.define(name, spec)
  return actors.define(name, spec)
end

--- `role.skill_define(name, spec)` → `true`（同样的幂等口径）。
function M.skill_define(name, spec)
  return actors.skill_define(name, spec)
end

--- `role.grant(id, role_name)` → `boolean`（生效后是否持有；+ 事件 `role.granted`）。
function M.grant(id, role_name)
  return actors.grant(id, role_name)
end

--- `role.revoke(id, role_name)` → `boolean`（原本是否持有；+ 事件 `role.revoked`）。
function M.revoke(id, role_name)
  return actors.revoke(id, role_name)
end

--- `role.has(id, role_name)` → `boolean`（纯读）。
function M.has(id, role_name)
  return actors.has(id, role_name)
end

--- `role.grant_skill(id, name, rank?)` → `boolean`（+ 事件 `role.skill_gained`）。
function M.grant_skill(id, name, rank)
  return skills.grant_skill(id, name, rank)
end

--- `role.revoke_skill(id, name)` → `boolean`（+ 事件 `role.skill_lost`）。
function M.revoke_skill(id, name)
  return skills.revoke_skill(id, name)
end

--- `role.charge(id, name)` → 剩余格数（PP 语义；零不减负）。
function M.charge(id, name)
  return skills.charge(id, name)
end

--- `role.recharge(id, name, n?)` → 恢复后的格数（上限由声明的 `charges` 决定）。
function M.recharge(id, name, n)
  return skills.recharge(id, name, n)
end

--- `role.effective(id, attr)` → `number`（基础值 + 合成加成；**纯读，从不写回**）。
function M.effective(id, attr)
  return compose.effective(id, attr)
end

--- `role.compose(fn)` → `true`（替换合成规则 `(base, bonuses) -> number`）。
function M.compose(fn)
  return compose.set(fn)
end

--- `role.count(id, event)` → 次数（纯读；冷却靠它表达，不靠时钟）。
function M.count(id, event)
  return counters.count(id, event)
end

--- `role.reset_counters(id, event?)` → `true`（显式清零；**何时清是 mod 的语义**）。
function M.reset_counters(id, event)
  return counters.reset(id, event)
end

return M
