-- `packages/libs/procgen/registry.lua` —— defs / algo 注册表 + `run`。
--
-- 契约：`openwiki/contracts/packages/procgen.md`（I4 生成器纯函数化 / I5 参数化而非分支 /
-- I6 版本进入哈希 / I7 顶层纯度）。
--
-- # defs 不进 KV（契约 §2 的回写结论）
--
-- 宿主 KV 只收四个标量（`crates/ygg-core/src/entity.rs` 的 `Value`），
-- `{ algo, params }` 这种嵌套表写不进去。所以 defs 分两条路进来：安装期
-- `install({ defs = ... })`（mod 从自己的 pack 数据构造）与运行期
-- `procgen.define`（本文件的内存表）。代价写在契约里：**运行期 define 的 def 不跨
-- 存档**，要跨存档的生成器必须来自 pack 数据。
--
-- # 表都是内存表，不是状态
--
-- `defs` / `algos` 是模块级注册表（与 codec 的 `package.loaded` 同级），不是跨
-- dispatch 的游戏状态：它们不参与回放，也不写 KV（I4）。

local rng = require("pkg.procgen.rng")

local fail = rng.fail
local E = rng.E

local M = {}

--- 生成器契约版本：产物一律自带 `v`（I6）。
---
--- 单一常量而**不是**每个 algo 一份：改任一生成器的产物形状就 +1，让所有 hash 一起
--- 变。这正是「换算法必然换哈希」要的效果——分开记反而给「只改了一个 algo 却漏改
--- 它的版本号」留了位置。
local CONTRACT_VERSION = 1

--- 自带算法所在的模块（`M.ALGOS` = 名字 → 实现）。注册在 `install` 期完成，不改 KV。
local BUILTIN_MODULES = {
  "pkg.procgen.algos.layout",
  "pkg.procgen.algos.terrain",
}

local defs = {}
local algos = {}

--- 注册一个 algo 实现（同名覆盖；install 幂等依赖这条）。
local function algo_register(name, fn)
  if type(name) ~= "string" or name == "" then
    fail(E.PARAM, "algo 名必须是非空字符串，得到 " .. type(name))
  end
  if type(fn) ~= "function" then
    fail(E.PARAM, "algo " .. name .. " 的实现必须是函数，得到 " .. type(fn))
  end
  algos[name] = fn
end

-- ── 值工具（深拷贝 / 深相等）──────────────────────────────────────────────────
--
-- defs 的 params 必须**存副本**：调用方在 define 之后改自己的表，不该改变已注册
-- 生成器的产物（那是最难查的一类不确定性）。同 spec 判定用同一个递归，不用
-- `codec.encode` 比字符串——procgen 对 codec 的依赖限定在摘要面（契约 §1）。

local function copy_value(value)
  if type(value) ~= "table" then
    return value
  end
  local out = {}
  for key, item in pairs(value) do
    out[key] = copy_value(item)
  end
  return out
end

local function same_value(left, right)
  if type(left) ~= type(right) then
    return false
  end
  if type(left) ~= "table" then
    return left == right
  end
  local count = 0
  for key, item in pairs(left) do
    count = count + 1
    if not same_value(item, right[key]) then
      return false
    end
  end
  for _ in pairs(right) do
    count = count - 1
  end
  return count == 0
end

-- ── 装配 ─────────────────────────────────────────────────────────────────────

--- 装配：注册自带算法、注入 defs（`{ codec?, defs? }`；`host` 本包不需要——不碰
--- 宿主面）。重复 `install` 幂等：算法按同名覆盖同一个实现，defs 走同 spec 幂等。
function M.install(deps)
  if deps ~= nil and type(deps) ~= "table" then
    fail(E.PARAM, "install 只收 table 或 nil，得到 " .. type(deps))
  end
  M.register_builtins()
  if type(deps) == "table" and deps.defs ~= nil then
    if type(deps.defs) ~= "table" then
      fail(E.PARAM, "deps.defs 必须是 { [名] = { algo, params } } 表")
    end
    for name, spec in pairs(deps.defs) do
      M.define(name, spec)
    end
  end
  return M
end

--- 注册自带算法（`pkg.procgen.algos.*` 的 `ALGOS` 表）。缺模块即 `-DEPS`。
function M.register_builtins()
  for i = 1, #BUILTIN_MODULES do
    local module_name = BUILTIN_MODULES[i]
    local ok, module = pcall(require, module_name)
    if not ok or type(module) ~= "table" or type(module.ALGOS) ~= "table" then
      fail(E.DEPS, "自带算法模块装载失败：" .. module_name)
    end
    for name, fn in pairs(module.ALGOS) do
      algo_register(name, fn)
    end
  end
  return M
end

--- `procgen.algo(name, fn)` → `true`；同名再次注册 = 覆盖（install 幂等需要）。
function M.algo(name, fn)
  algo_register(name, fn)
  return true
end

function M.define(name, spec)
  if type(name) ~= "string" or name == "" then
    fail(E.PARAM, "def 名必须是非空字符串，得到 " .. type(name))
  end
  if type(spec) ~= "table" then
    fail(E.PARAM, "def " .. name .. " 的 spec 必须是表，得到 " .. type(spec))
  end
  if type(spec.algo) ~= "string" or spec.algo == "" then
    fail(E.PARAM, "def " .. name .. " 的 algo 必须是非空字符串")
  end
  if spec.params ~= nil and type(spec.params) ~= "table" then
    fail(E.PARAM, "def " .. name .. " 的 params 必须是表，得到 " .. type(spec.params))
  end
  local normalized = { algo = spec.algo, params = copy_value(spec.params or {}) }
  local existing = defs[name]
  if existing ~= nil then
    if same_value(existing, normalized) then
      return true
    end
    fail(E.DEF, "def " .. name .. " 已用另一份 spec 定义过")
  end
  defs[name] = normalized
  return true
end

--- `procgen.run(name, seed, stream?)` → 产物表（纯：不写 KV、不发事件）。
---
--- `stream` 缺省 = 生成器名（契约 §4 的回写结论）：不同的生成器天然落在不同的流上，
--- 于是「生成地图」不会扰动「生成怪物」——两者只要不显式共用流名就互不影响。
function M.run(name, seed, stream)
  local def = defs[name]
  if def == nil then
    fail(E.DEF, "def " .. tostring(name) .. " 未定义")
  end
  local fn = algos[def.algo]
  if fn == nil then
    fail(E.ALGO, "algo " .. def.algo .. " 未注册")
  end
  local handle = rng.new(seed, stream or name)
  local product = fn(def.params, handle)
  if type(product) ~= "table" then
    fail(E.PARAM, "algo " .. def.algo .. " 必须返回表，得到 " .. type(product))
  end
  if product.algo ~= nil or product.v ~= nil then
    fail(E.PARAM, "algo " .. def.algo .. " 的产物不得自带 algo / v（由 run 统一附着，I6）")
  end
  local out = { algo = def.algo, v = CONTRACT_VERSION }
  for key, value in pairs(product) do
    out[key] = value
  end
  return out
end

return M
