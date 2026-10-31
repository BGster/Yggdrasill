-- 被入口与 src.handlers 分别 require 的模块：只执行一次，返回同一 table。
shared_runs = (shared_runs or 0) + 1
load_trace = (load_trace or "") .. ">shared"

local shared = { tag = "shared", count = 0 }

function shared.bump(step)
  shared.count = shared.count + (step or 1)
  return shared.count
end

-- 注册面由入口调用（顶层不碰有副作用的 host API）。
function shared.register()
  ygg.on("probe.order", function()
    ygg.narrate("shared")
  end)
end

return shared
