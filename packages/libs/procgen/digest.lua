-- `packages/libs/procgen/digest.lua` —— 稳定十六进制摘要面（`procgen.hash`）。
--
-- 契约：`openwiki/contracts/packages/procgen.md` §4 / I6。
--
-- # 为什么借 codec 的编码而不自己写一份
--
-- 仓里已经有唯一一份确定性 JSON 编码口径（`codec.md` I1：同一值两次 `encode`
-- 逐字节相等）。为摘要再写第二份编码器，就是契约 §8 反复禁止的「同一套东西各写
-- 一遍」——两份口径一秒就能分叉，而分叉的症状是「同 seed 同产物」静默失效。
-- 所以 procgen 对 codec 的依赖**只在这一面**（契约 §1 首条边界）。
--
-- # 这不是加密摘要
--
-- FNV-1a 64 是**变更检测**：跨机跨 run 稳定、16 位十六进制、实现只有十几行。
-- 它不抗碰撞，也不该被用来验签。字节一致的真证据是「两次独立进程的 canon 行
-- 逐字节相等 + 宿主算 sha256」（契约 §7），`hash` 只是便捷面。

local rng = require("pkg.procgen.rng")

local fail = rng.fail
local E = rng.E

local M = {}

--- 十六进制数字面：不用 `string` 表的 `format`（宿主沙箱里那张表不可达）。
local HEX = { "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "a", "b", "c", "d", "e", "f" }

local encoder = nil

--- 装配：记 `deps.codec`（缺省 `require("pkg.codec")`）。
function M.install(deps)
  if deps ~= nil and type(deps) ~= "table" then
    fail(E.PARAM, "install 只收 table 或 nil，得到 " .. type(deps))
  end
  encoder = type(deps) == "table" and deps.codec or nil
  return M
end

--- 取编码面（`install` 注入优先，其次 require）。签名不对即 `-DEPS`。
local function codec()
  local module = encoder
  if module == nil then
    local ok, loaded = pcall(require, "pkg.codec")
    if not ok then
      fail(E.DEPS, "procgen.hash 需要 pkg.codec（install({ codec = ... }) 或可 require）")
    end
    module = loaded
  end
  if type(module) ~= "table" or type(module.encode) ~= "function" then
    fail(E.DEPS, "codec 面必须导出 encode(value) -> string")
  end
  encoder = module
  return module
end

--- 64 位整数 → 16 位小写十六进制（`>>` 是逻辑右移，负值也按无符号展开）。
function M.hex64(value)
  local out = ""
  for _ = 1, 16 do
    out = HEX[(value & 0xF) + 1] .. out
    value = value >> 4
  end
  return out
end

--- `procgen.hash(value)` → 16 位十六进制字符串。
---
--- 口径 = `fnv1a64(codec.encode(value))`。产物自带 `algo` / `v`（`run` 统一附着），
--- 所以换算法或换版本必然换 hash——不需要把版本塞进 hash 的入参（I6）。
function M.hash(value)
  return M.hex64(rng.fnv64(codec().encode(value)))
end

return M
