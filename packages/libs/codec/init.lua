-- `pkg.codec` —— 确定性 JSON 编解码与存档往返。契约：`docs/design/packages/codec.md`。
--
-- # 这个包做什么
--
-- 两个正交问题，两套函数：**形状**（谁是字段、谁是引用）归 `pkg.entity`，
-- **字节**（同一输入能否产出同一输出）归本包。混在一起时，任何「让 JSON 好看
-- 一点」的键序调整都会变成实体契约的变更——这是契约 §8 的反方案一。
--
-- # 实现纪律（沙箱方言；契约 §8 记着为什么）
--
-- 1. 沙箱没有 `string` / `table` / `utf8` 全局（`sandbox.rs` 白名单测试钉死）：
--    字符串只能走方法语法，表工具手写。出现标准库表的点式调用即会在真实
--    跑局里崩，而 selftest 看不出来——交付门禁带静态 grep。
-- 2. 模块顶层纯：只 local / 常量 / 函数定义。`ygg` 访问一律发生在调用期，经
--    `host()` 单点取句柄（契约 I8：`install` 不预读、不预写）。
-- 3. **键序 = 字节序升序**（契约 I1）：不排序就没有确定性，没有确定性就没有
--    「同 seed 同产物」。
-- 4. **不读 `pkg.entity.*` 任何键**：记录只经 `pkg.entity` 的 API 取（布局单源在
--    entity；出现 `pkg.entity.rec` 这类键名即视为布局泄漏）。
--
-- 依赖序单向：codec → entity（`packages/README.md` §3）。

local entity = require("pkg.entity")
local sha256 = require("pkg.codec.sha256")

local M = { VERSION = "0.1.0" }

-- 依赖记位（`install` 写入；`host()` 读）。
M.deps = nil

-- 错误码家族 `YGG-PKG-CODEC-<COND>`（契约固定 TYPE / CONFLICT / CHECKSUM；
-- MISSING / VERSION / HOST 是本实现补齐的家族成员，见 codec.md §8）。
local E_TYPE = "YGG-PKG-CODEC-TYPE"

--- 报错：`<码>: <细节>`（level 0 = 不夹带调用点，测试按码做子串断言）。
local function fail(code, detail)
  error(code .. ": " .. detail, 0)
end

-- ── 宿主句柄（install 只记 deps；契约 I8 的「不预读不预写」）────────────────────

--- 装配：记下可选依赖（`{ host = ... }`；缺省走 `ygg` 全局）。
---
--- `M.deps` 是**公开字段**而不是私有 local：装进去的东西应该看得见（用例靠它
--- 断言装的是哪份 mock；调度处也靠它排查「到底装了没」）。
function M.install(deps)
  M.deps = { host = (type(deps) == "table" and deps.host) or nil }
  return M
end

--- `codec.sha256(bytes)`：纯 Lua SHA-256（64 位小写 hex）。
---
--- 为何自己写而不取宿主 `fs.write` 的回值：那一值只能在**落盘之后**得到，而
--- `checksum(bundle)` 必须在不落盘时就能算（导出侧要把校验和写进 bundle 本体），
--- 且「导入前先校验」要求能在纯读路径上复核。详见 `sha256.lua` 的头注。
---
--- 输入域先在这里收窄：非字符串（表 / 函数 / nil）在进哈希前就报
--- `YGG-PKG-CODEC-TYPE`，而不是变成 `tostring(table)` 这种带地址的字节。
function M.sha256(bytes)
  if type(bytes) ~= "string" then
    fail(E_TYPE, "sha256 只收字符串，得到 " .. type(bytes))
  end
  return sha256.sha256(bytes)
end

return M
