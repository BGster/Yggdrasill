-- `src/inv.lua` —— TLA 不变量注册表（`Inv` → inv.check 的映射落点）。
--
-- 契约：openwiki/contracts/mod/tla-models.md §映射表。四个模型的 `TypeOK` / `Inv` /
-- `KlotskiGoal` / `TerminationDetection` 全部注册在这里，`inv.check(model)`
-- 返回违例清单（空 = 绿）——TLA 的「不变量在每状态断言」翻译成「每步后可查」。
--
-- 模块顶层纯（多文件纯度契约）：只有 local / 常量 / 函数定义；注册表是模块
-- local，经 `M.register` 在模型 `install()` 时填充（幂等：同名同 fn 覆盖）。
-- 违例清单按注册名排序输出（无 table.sort，沙箱方言手写选择排序），保证
-- 双局对账与红例断言的清单序确定。

local M = { VERSION = "0.1.0" }

local REGISTRY = {}

--- 注册一条不变量：`inv.register("gol", "TypeOK", fn)`。
--- `fn()` 返回违例字符串数组（空数组 = 绿）；fn 内部错误也计为违例
--- （fail-closed：断言崩溃不能伪装成绿）。
function M.register(model, name, fn)
  if type(model) ~= "string" or type(name) ~= "string" or type(fn) ~= "function" then
    error("YGG-MOD-TLA-INV: register 需要非空 model/name 字符串与函数", 0)
  end
  local bucket = REGISTRY[model]
  if bucket == nil then
    bucket = {}
    REGISTRY[model] = bucket
  end
  bucket[name] = fn
  return true
end

--- 已注册的不变量名（升序；诊断面）。
function M.names(model)
  local bucket = REGISTRY[model]
  if bucket == nil then
    return {}
  end
  local out = {}
  for name in pairs(bucket) do
    out[#out + 1] = name
  end
  for i = 1, #out do
    local min = i
    for j = i + 1, #out do
      if out[j] < out[min] then
        min = j
      end
    end
    if min ~= i then
      out[i], out[min] = out[min], out[i]
    end
  end
  return out
end

--- 查一个模型的全部不变量：返回违例清单（`"Name: detail"` 字符串数组，
--- 升序；空数组 = 绿）。未知模型 = 空数组（注册面在 install 期完成，
--- 查未注册模型是调用方契约错误，但不静默绿也不炸——返回空清单并在
--- `M.known(model)` 留判别面）。
function M.check(model)
  local bucket = REGISTRY[model]
  local out = {}
  if bucket == nil then
    return out
  end
  for _, name in ipairs(M.names(model)) do
    local ok, verdict = pcall(bucket[name])
    if not ok then
      out[#out + 1] = name .. ": <抛错> " .. tostring(verdict)
    elseif type(verdict) == "table" then
      for i = 1, #verdict do
        out[#out + 1] = name .. ": " .. tostring(verdict[i])
      end
    elseif type(verdict) == "string" and #verdict > 0 then
      out[#out + 1] = name .. ": " .. verdict
    end
  end
  return out
end

--- 模型是否已有注册不变量（`check` 空清单的判别面）。
function M.known(model)
  return REGISTRY[model] ~= nil
end

return M
