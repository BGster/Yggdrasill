-- `pkg.codec.runtime` —— codec 的共享座：错误码、`fail`、`CODEC_VERSION`、deps 记位
-- 与宿主句柄（host 三件）。
--
-- 契约：`docs/design/packages/codec.md`。批十一拆分的叶子模块：`fail` / `E_*` /
-- `CODEC_VERSION` / deps 存放只住这一处（共享 local 禁止抄第二份），其余模块经
-- `require("pkg.codec.runtime")` 取用。装配范式同 `sha256.lua`；沙箱方言纪律与包
-- 整体设计见 `init.lua` 头注与契约 §8。
--
-- 与原单文件的唯一差异（计划 §2 授权项）：原 `M.install` 的 deps 记位改为本模块
-- 的 `installed` 座（init.lua 的 `M.install` 调它并把回值同步为公开字段 `M.deps`
-- 别名），`host_or_nil` 相应改读 `installed`——观察面语义不变。

local sha256 = require("pkg.codec.sha256")

local M = {}

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

-- 依赖记位（`install` 写入；`host_or_nil` / `host` / `native_face` 读）。
local installed = nil

function M.install(deps)
  installed = { host = (type(deps) == "table" and deps.host) or nil }
  return installed
end

--- 取本次调用的宿主句柄；不存在时返回 nil（不报错）。
---
--- 拆成「取」与「要求」两半，是因为 `encode` / `decode` 的语义里**缺宿主不是错**：
--- 没有原生面就走本包的 Lua 实现，产出字节相同（契约 §4.1 的下沉边界）。
local function host_or_nil()
  local current = installed and installed.host or nil
  if current == nil then
    current = ygg
  end
  return current
end

--- 取本次调用的宿主句柄（`deps.host` 优先，其次 `ygg` 全局）；缺了就报 `HOST`。
local function host()
  local current = host_or_nil()
  if current == nil then
    fail(E_HOST, "ygg 不可达：沙箱外调用需 `install({ host = ... })`")
  end
  return current
end

--- 原生加速面（可选；契约 §4.1）：无状态、两键齐备才算数，否则返回 nil。
---
--- 两键齐备才用，是为了让「半个面」退化成「没有面」——一个只有 `encode` 的实现
--- 会让 `decode` 静默走另一条路，而两条路的字节相等正是本包唯一要保的性质。
local function native_face()
  local current = host_or_nil()
  if current == nil then
    return nil
  end
  local face = current.codec
  if type(face) ~= "table" then
    return nil
  end
  if type(face.encode) ~= "function" or type(face.decode) ~= "function" then
    return nil
  end
  return face
end

--- `codec.sha256(bytes)`：纯 Lua SHA-256（64 位小写 hex）。
---
--- 为何自己写而不取宿主 `fs.write` 的回值：那一值只能**落盘之后**得到，而
--- `checksum(bundle)` 必须在落盘前就能算（导出侧要把校验和写进 bundle 本体），
--- 且「导入前先校验」要求纯读路径能复核——依赖宿主回值就证明不了「盘上没被改
--- 过」，而那正是 checksum 存在的理由（D11 的退役依据）。详见 `sha256.lua` 头注。
---
--- 输入域在包边界收窄：非字符串（表 / 函数 / nil）在进哈希前就报
--- `YGG-PKG-CODEC-TYPE`，而不是变成 `tostring(table)` 这种带地址的字节。
function M.sha256(bytes)
  if type(bytes) ~= "string" then
    fail(E_TYPE, "sha256 只收字符串，得到 " .. type(bytes))
  end
  return sha256.sha256(bytes)
end

-- ── 导出面（供兄弟模块与 init.lua 取用）──────────────────────────────────────

M.E_TYPE = E_TYPE
M.E_CONFLICT = E_CONFLICT
M.E_CHECKSUM = E_CHECKSUM
M.E_MISSING = E_MISSING
M.E_VERSION = E_VERSION
M.E_HOST = E_HOST
M.CODEC_VERSION = CODEC_VERSION
M.fail = fail
M.host_or_nil = host_or_nil
M.host = host
M.native_face = native_face

return M

