-- 入口：三层依赖 entry → src.handlers → src.util（`src.shared` 由入口与
-- `src.handlers` 两处 require，`src.silent` 无返回值）。
--
-- 本 fixture 是 r4 测试面的**真跑对象**（不是空壳）：每个模块在装载期留痕
-- （`load_trace` / `*_execs` / `*_runs`）并注册 `probe.order` handler，测试据此
-- 断言执行顺序、执行次数、对象身份与注册序；`probe.*` 事件是给测试的观测面，
-- `event.start` / `ygg.choice_selected` 是给 playtest 的推进面（走到【结局）。

local handlers = require("src.handlers") -- 第二跳：→ src.util / src.shared
local util = require("src.util") -- 与 handlers 里那次是同一份
local shared = require("src.shared") -- 同上（第三处 require 同一模块）
local silent = require("src.silent") -- 无返回值 → 缓存 true
local silent_again = require("src.silent") -- 重复 require 不得再执行

local TEXTS = ygg.data("ygg:scene/texts")

-- 正向用例 1 的落点：模块返回的 table 里的函数被入口用来注册 handler。
handlers.register()

-- 派发序 = 这两行的调用序（不是上面的 require 序）：模块纯导出，装配集中在入口。
-- 把 require 顺序调换不会改变 probe.order 的派发顺序——测试据这条断言"拆文件不改行为"。
util.register()
shared.register()
handlers.register_order()

-- 执行顺序（模块体进入序）与身份/次数观测面。
ygg.on("probe.trace", function()
  ygg.narrate("trace=" .. load_trace)
end)
ygg.on("probe.identity", function()
  ygg.narrate(
    "util_same="
      .. tostring(util == handlers.util())
      .. ";shared_same="
      .. tostring(shared == handlers.shared())
      .. ";util_execs="
      .. tostring(util_execs)
      .. ";shared_runs="
      .. tostring(shared_runs)
  )
end)
ygg.on("probe.nil", function()
  ygg.narrate(
    "silent="
      .. tostring(silent)
      .. ";again="
      .. tostring(silent_again)
      .. ";same="
      .. tostring(silent == silent_again)
      .. ";runs="
      .. tostring(silent_runs)
  )
end)

-- 存读档观测面：世界状态（KV）随存档往返，模块内状态（util.count 等）不随
-- —— 后者属于「代码的加载产物」，读档必须重建（cache 归零、只执行一次）。
ygg.on("probe.kv", function()
  ygg.narrate("stage=" .. tostring(ygg.kv.get("stage")))
end)

-- 确定性观测面：注册序 = handler 派发序（同一事件下逐项比对）。
ygg.on("probe.order", function()
  ygg.narrate("entry")
end)

ygg.narrative.node("node:begin", function()
  ygg.narrate(TEXTS.scenes.begin .. handlers.suffix())
  ygg.kv.set("stage", "begin")
  ygg.choice(TEXTS.prompts.door, { TEXTS.options.enter, TEXTS.options.leave })
end)

ygg.on("ygg.choice_selected", function(index)
  if index == 1 then
    ygg.narrate(TEXTS.endings.brave)
  else
    ygg.narrate(TEXTS.endings.cautious)
  end
end)

ygg.on("event.start", function()
  ygg.narrative["goto"]("node:begin")
end)
