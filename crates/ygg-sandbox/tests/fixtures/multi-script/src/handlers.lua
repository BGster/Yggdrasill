-- 中间层（三层依赖的第二跳）：自己 require 两个叶子模块，并把「注册 handler」
-- 这件事做成模块函数交给入口调用——正向用例 1/3 的落点。
load_trace = (load_trace or "") .. ">handlers"

local util = require("src.util")
local shared = require("src.shared")

local handlers = {}

function handlers.suffix()
  return util.suffix()
end

function handlers.util()
  return util
end

function handlers.shared()
  return shared
end

-- 由入口在装载期调用：这里注册的 handler 用的是模块返回 table 里的函数。
function handlers.register()
  ygg.on("probe.table", function()
    ygg.narrate("handler_from_table=" .. util.label .. ";bumped=" .. util.bump(2))
  end)
  ygg.on("probe.bump", function()
    ygg.narrate("util_count=" .. tostring(util.bump(3)))
  end)
  ygg.on("probe.stats", function()
    ygg.narrate(
      "util_execs="
        .. tostring(util_execs)
        .. ";shared_runs="
        .. tostring(shared_runs)
        .. ";silent_runs="
        .. tostring(silent_runs)
        .. ";util_count="
        .. tostring(util.count)
        .. ";shared_count="
        .. tostring(shared.count)
    )
  end)
end

ygg.on("probe.order", function()
  ygg.narrate("handlers")
end)

return handlers
