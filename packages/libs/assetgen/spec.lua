-- `packages/libs/assetgen/spec.lua` —— 错误码 / `fail` / 参数三段值域 / spec 归一。
--
-- 契约：`docs/design/packages/assetgen.md` §3 I8（值域三段）/ §4（`ERRORS` 五键）。
--
-- # 为什么单开这一个叶子
--
-- 错误码字符串与「低于下界 = PARAM / 高于上界 = CAP」这条判定被**三个** algo 模块
-- （texture / mesh / sound）共用。让它们各自复制一份 = 仓里出现多份错误码字面量，
-- 改名时漏掉一处就静默失效——与「第三份哈希」同一类分叉。所以放进唯一被所有实现
-- 依赖的叶子。
--
-- # 值域三段（I8）
--
--   非整数 / 低于下界 → PARAM（形状错，调用方写错了）
--   高于上界         → CAP  （资源帽，尺寸超出本包承诺的范围）
--   未知算法         → ALGO
--
-- 越界**不截断**：截断会把半个素材交给调用方，而调用方无从发现。

local M = {}

M.E = {
  PARAM = "YGG-PKG-ASSETGEN-PARAM",
  ALGO = "YGG-PKG-ASSETGEN-ALGO",
  CAP = "YGG-PKG-ASSETGEN-CAP",
  KIND = "YGG-PKG-ASSETGEN-KIND",
  DEPS = "YGG-PKG-ASSETGEN-DEPS",
}

--- 报错：`<码>: <细节>`（level 0 = 不夹带调用点；用例按码做子串断言）。
function M.fail(code, detail)
  error(code .. ": " .. detail, 0)
end

local I64_MAX = 0x7FFFFFFFFFFFFFFF

--- 整值判定：整值浮点（`3.0`）也算整数，带小数的、超 i64 的都不算。
---
--- 不用 `math.type`：沙箱里 `math` 是不可达的 `nil`（`crates/ygg-sandbox/src/sandbox.rs`
--- 的逐名断言），包代码一碰就崩。`v == v // 1` 是同一件事的算术写法。
function M.is_int(value)
  return type(value) == "number" and value == (value // 1) and value <= I64_MAX and value >= -I64_MAX
end

--- 整型参数：缺省取 `default`，按三段判定，返回归一化整数。
function M.int(params, key, default, low, high)
  local value = params[key]
  if value == nil then
    value = default
  end
  if not M.is_int(value) then
    M.fail(M.E.PARAM, "params." .. key .. " 必须是整数，得到 " .. type(value))
  end
  if value < low then
    M.fail(M.E.PARAM, "params." .. key .. " 必须 ≥ " .. low .. "，得到 " .. value)
  end
  if value > high then
    M.fail(M.E.CAP, "params." .. key .. " 必须 ≤ " .. high .. "，得到 " .. value)
  end
  return value | 0
end

--- 枚举参数：值不在 `allowed` 里 = `-PARAM`（未知**参数值**不是未知算法）。
function M.one_of(params, key, default, allowed)
  local value = params[key]
  if value == nil then
    value = default
  end
  if type(value) ~= "string" or allowed[value] == nil then
    local listed = ""
    for name in pairs(allowed) do
      if listed ~= "" then
        listed = listed .. " / "
      end
      listed = listed .. tostring(name)
    end
    M.fail(M.E.PARAM, "params." .. key .. " 必须是 " .. listed .. " 之一，得到 " .. tostring(value))
  end
  return value
end

--- 颜色参数：`0xRRGGBB`（`0..0xFFFFFF`）；越界按三段判定（负 = PARAM，超 = CAP）。
function M.rgb(params, key, default)
  return M.int(params, key, default, 0, 0xFFFFFF)
end

--- 稠密数组参数：缺省取 `default`；非数组 / 有空洞 = `-PARAM`。
function M.array(params, key, default)
  local value = params[key]
  if value == nil then
    return default
  end
  if type(value) ~= "table" then
    M.fail(M.E.PARAM, "params." .. key .. " 必须是数组，得到 " .. type(value))
  end
  local n = #value
  for i = 1, n do
    if value[i] == nil then
      M.fail(M.E.PARAM, "params." .. key .. " 必须是稠密数组（下标 " .. i .. " 为空）")
    end
  end
  return value
end

--- spec 归一：`{ algo = "<kind>.<name>", params = {...} }` → `name, params`。
---
--- 前缀必须与 kind 一致：`assetgen.texture({ algo = "mesh.box" })` 是调用方写错了
--- 形状（`-PARAM`），不是未知算法（`-ALGO`）——两者的修法不同，码必须分开。
function M.algo_of(kind, spec)
  if type(spec) ~= "table" then
    M.fail(M.E.PARAM, "spec 必须是表，得到 " .. type(spec))
  end
  local algo = spec.algo
  if type(algo) ~= "string" then
    M.fail(M.E.PARAM, "spec.algo 必须是字符串，得到 " .. type(algo))
  end
  local prefix = kind .. "."
  if algo:sub(1, #prefix) ~= prefix then
    M.fail(M.E.PARAM, "spec.algo 必须以 " .. prefix .. " 开头，得到 " .. algo)
  end
  local params = spec.params
  if params == nil then
    params = {}
  elseif type(params) ~= "table" then
    M.fail(M.E.PARAM, "spec.params 必须是表，得到 " .. type(params))
  end
  return algo:sub(#prefix + 1), params
end

--- 缺省流名（I2）：`assetgen/<kind>/<algo>`。
function M.default_stream(kind, name)
  return "assetgen/" .. kind .. "/" .. name
end

return M
