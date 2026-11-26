-- 中间层（三层依赖的第二跳）：自己 require 两个叶子模块，并把「装配」这件事
-- 做成模块函数交给入口调用——装配参数由入口给（`note`），模块自己不写状态。

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

-- 由入口在装载期调用：装配序由入口的调用序决定，模块只把名字交回给调用方。
function handlers.register_order(note)
  if note then
    note("handlers")
  end
end

return handlers
