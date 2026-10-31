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

-- 注册面由入口调用：模块只导出（顶层不碰任何有副作用的 host API）。
-- 派发序因此只由入口的调用顺序决定，与 require 顺序无关。
function util.register()
  ygg.on("probe.order", function()
    ygg.narrate("util")
  end)
end

return util
