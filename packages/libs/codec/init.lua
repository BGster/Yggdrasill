-- `pkg.codec` —— 确定性 JSON 编解码与存档往返。契约：`openwiki/contracts/packages/codec.md`。
--
-- # 这个包做什么
--
-- 两个正交问题，两套函数：**形状**（谁是字段、谁是引用）归 `pkg.entity`，
-- **字节**（同一输入能否产出同一输出）归本包。混在一起时，任何「让 JSON 好看
-- 一点」的键序调整都会变成实体契约的变更——这是契约 §8 的反方案一。
--
-- # 实现纪律（沙箱方言；契约 §8 记着为什么）
--
-- 1. 沙箱没有 `string` / `table` / `utf8` / `math` 全局（`sandbox.rs` 白名单测试
--    钉死）：字符串只能走方法语法，表工具与整/浮判据手写。后四个在**裸**沙箱缺席，
--    `math` 只在驱动路径由 `inject::install` 注入（`inject.rs`）——包不得依赖那条
--    注入，否则任何人用 `Sandbox::new()` 直接装包都会在运行期炸。点式调用标准库表
--    即会在真实跑局里崩，而 selftest 看不出来——交付门禁带静态 grep
--    （`tests/packages_sandbox.rs` 用例 ③）。
-- 2. 模块顶层纯：只 local / 常量 / 函数定义。`ygg` 访问一律发生在调用期，经
--    `host()` 单点取句柄（契约 I8：`install` 不预读、不预写）。
-- 3. **键序 = 字节序升序**（契约 I1）：不排序就没有确定性，没有确定性就没有
--    「同 seed 同产物」。
-- 4. **不读 `pkg.entity.*` 任何键**：记录只经 `pkg.entity` 的 API 取（布局单源在
--    entity；出现 `pkg.entity.rec` 这类键名即视为布局泄漏）。
--

-- # 文件切分（批十一；纯搬移，装配范式同 sha256.lua / quest 三件套）
--
--   runtime.lua   错误码 / `fail` / `CODEC_VERSION` / deps 记位与宿主句柄（叶子座）
--   jsonenc.lua   encode：确定性 JSON 编码（原生面委托 + 回退）+ 骨架工具单源
--   jsondec.lua   decode：递归下降解析（`at byte N` 位置化报错）
--   bundle.lua    checksum / export / write / read / roundtrip（存档往返）
--   importing.lua import：两阶段（validate → apply）三种冲突策略
--   sha256.lua    纯 Lua SHA-256（本批之前的先例模块，不动）
--
-- 依赖向无环：runtime ← {jsonenc, jsondec} ← bundle ← importing；`bundle` /
-- `importing` → `pkg.entity` 是包间边（同原单文件 top-level require）。本文件只做
-- 装配与转口：一个 API 一行的转发层，导出表面逐键不变（12 键，codec.md §4）。
--
-- 依赖序单向：codec → entity（`packages/README.md` §3）。

local runtime = require("pkg.codec.runtime")
local jsonenc = require("pkg.codec.jsonenc")
local jsondec = require("pkg.codec.jsondec")
local bundle_mod = require("pkg.codec.bundle")
local importing = require("pkg.codec.importing")

local M = { VERSION = "0.1.0" }

-- 依赖记位（`install` 写入；`host()` 读）。`M.deps` 是**公开字段**而不是私有
-- local：装进去的东西应该看得见（用例靠它断言装的是哪份 mock；调度处也靠它
-- 排查「到底装了没」）。本字段是 runtime 座的**别名**（`M.install` 同步）。
M.deps = nil

--- 装配：记下可选依赖（`{ host = ... }`；缺省走 `ygg` 全局）。
---
--- `M.deps` 是**公开字段**而不是私有 local：装进去的东西应该看得见（用例靠它
--- 断言装的是哪份 mock；调度处也靠它排查「到底装了没」）。

function M.install(deps)
  M.deps = runtime.install(deps)
  return M
end

--- `codec.sha256(bytes)`：类型门 + 委托 `pkg.codec.sha256`（包装体在 runtime）。
function M.sha256(bytes)
  return runtime.sha256(bytes)
end

--- `codec.encode(value)`：确定性 JSON 字节（键序 = 字节序；主体在 jsonenc）。
function M.encode(value)
  return jsonenc.encode(value)
end

--- `codec.decode(text)`：确定性 JSON → Lua 值（主体在 jsondec）。
function M.decode(text)
  return jsondec.decode(text)
end

--- `codec.checksum(bundle)`：规范化字节的 sha256（主体在 bundle）。
function M.checksum(bundle)
  return bundle_mod.checksum(bundle)
end

--- `codec.export(namespace)`：bundle = 实体记录 + 点名 KV 键（主体在 bundle）。
function M.export(ns)
  return bundle_mod.export(ns)
end

--- `codec.write(path, bundle)`：算 checksum → 编码 → 落盘 → 校回执字节数。
function M.write(path, bundle)
  return bundle_mod.write(path, bundle)
end

--- `codec.read(path)`：读盘 → 解码 → 先校 checksum 再交出去。
function M.read(path)
  return bundle_mod.read(path)
end

--- `codec.roundtrip(path)`：导 → 写 → 读 → 再编，比字节。
function M.roundtrip(path)
  return bundle_mod.roundtrip(path)
end

--- `codec.import(bundle, opts)`：两阶段两回（主体在 importing）。
function M.import(bundle, opts)
  return importing.import(bundle, opts)
end

return M
