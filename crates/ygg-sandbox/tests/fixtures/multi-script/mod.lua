-- 入口：三层依赖 entry → src.handlers → src.util（`src.shared` 由入口与
-- `src.handlers` 两处 require，`src.silent` 无返回值）。
--
-- 本 fixture 是模块装载面的**真跑对象**（不是空壳）：每个模块在装载期留痕
-- （`load_trace` / `*_execs` / `*_runs`），入口把观测值写进 `ygg.kv`，测试读
-- `probe.<topic>` 逐字比对。
--
-- 为什么观测写在这里而不在模块里：模块顶层受 MOD_EFFECT 约束（不得写状态），
-- 入口 chunk 豁免。装配与观测因此只有一处来源，顺序就是本文件的行序。

local handlers = require("src.handlers") -- 第二跳：→ src.util / src.shared
local util = require("src.util") -- 与 handlers 里那次是同一份
local shared = require("src.shared") -- 同上（第三处 require 同一模块）
local silent = require("src.silent") -- 无返回值 → 缓存 true
local silent_again = require("src.silent") -- 重复 require 不得再执行

local TEXTS = ygg.data("ygg:scene/texts")

-- 次数与身份观测面。`probe.stats` 必须记在下面两次 `bump` 之前：它的口径是
-- "装载完成、还没有人调用模块函数"时的计数。
ygg.kv.set(
  "probe.stats",
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
ygg.kv.set("probe.trace", "trace=" .. load_trace)
ygg.kv.set(
  "probe.identity",
  "util_same="
    .. tostring(util == handlers.util())
    .. ";shared_same="
    .. tostring(shared == handlers.shared())
    .. ";util_execs="
    .. tostring(util_execs)
    .. ";shared_runs="
    .. tostring(shared_runs)
)
ygg.kv.set(
  "probe.nil",
  "silent="
    .. tostring(silent)
    .. ";again="
    .. tostring(silent_again)
    .. ";same="
    .. tostring(silent == silent_again)
    .. ";runs="
    .. tostring(silent_runs)
)

-- 装配序 = 这几行的调用序（不是上面的 require 序）：模块纯导出，装配集中在入口。
-- 把 require 顺序调换不会改变装配序——测试据这条断言"拆文件不改行为"。
local order = ""
local function note(name)
  order = (order == "") and name or (order .. "\n" .. name)
end
util.register(note)
shared.register(note)
handlers.register_order(note)
note("entry")
ygg.kv.set("probe.order", order)

-- 模块函数被调用时真的写动了模块内状态（不是把常量抄进来）。
ygg.kv.set("probe.table", "handler_from_table=" .. util.label .. ";bumped=" .. util.bump(2))
ygg.kv.set("probe.bump", "util_count=" .. tostring(util.bump(3)))

-- 存读档观测面：世界状态（KV）随存档往返，模块内状态（util.count 等）不随
-- —— 后者属于「代码的加载产物」，读档必须重建（cache 归零、只执行一次）。
ygg.kv.set("stage", "begin")
ygg.kv.set("probe.kv", "stage=" .. tostring(ygg.kv.get("stage")))

-- 资源面：`ygg.data` 是纯读，模块顶层也允许；这里再证一次入口读得到。
ygg.kv.set("probe.texts", "scenes=" .. tostring(TEXTS.scenes ~= nil))
