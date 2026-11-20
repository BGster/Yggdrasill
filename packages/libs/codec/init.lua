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
--    字符串只能走方法语法，表工具手写。出现 `string.` 或 `table.sort` 即会在真实
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

local M = { VERSION = "0.1.0" }

-- 错误码家族 `YGG-PKG-CODEC-<COND>`（契约固定 TYPE / CONFLICT / CHECKSUM；
-- MISSING / VERSION / HOST 是本实现补齐的家族成员，见 codec.md §8）。
local E_TYPE = "YGG-PKG-CODEC-TYPE"
local E_CONFLICT = "YGG-PKG-CODEC-CONFLICT"
local E_CHECKSUM = "YGG-PKG-CODEC-CHECKSUM"
local E_MISSING = "YGG-PKG-CODEC-MISSING"
local E_VERSION = "YGG-PKG-CODEC-VERSION"
local E_HOST = "YGG-PKG-CODEC-HOST"

-- 本实现写出的 `codec_version`。跨版本不做迁移（契约 §1：只拒，不猜）。
local CODEC_VERSION = "1"

--- 报错：`<码>: <细节>`（level 0 = 不夹带调用点，测试按码做子串断言）。
local function fail(code, detail)
  error(code .. ": " .. detail, 0)
end

-- ── 宿主句柄（install 只记 deps；契约 I8 的「不预读不预写」）────────────────────

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
