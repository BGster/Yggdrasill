-- 叶子模块：返回 table（入口与 src.handlers 都经它注册 / 调用）。
-- `util_execs` 是执行次数观测面：任何重复装载都会立刻显形。
util_execs = (util_execs or 0) + 1
load_trace = (load_trace or "") .. ">util"

local util = { count = 0, label = "util" }

function util.bump(step)
  util.count = util.count + (step or 1)
  return util.count
end

function util.suffix()
  return "（多文件 Mod）"
end

-- 装载期注册：注册序 = 模块执行序（确定性断言的观测面之一）。
ygg.on("probe.order", function()
  ygg.narrate("util")
end)

return util
